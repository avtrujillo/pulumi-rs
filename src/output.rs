use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures::future::{FutureExt, Shared};
use tokio::sync::oneshot;

use crate::error::Error;

type BoxFuture<T> = Pin<Box<dyn Future<Output = Result<T, Arc<Error>>> + Send>>;

/// Represents a value that may not yet be known.
///
/// `Output<T>` is the core Pulumi type. Resource properties are always wrapped in
/// `Output` because their values may depend on other resources that haven't been
/// created yet, or may be unknown during `pulumi preview`.
///
/// Outputs support combinators like [`Output::map`] and [`Output::flat_map`] to
/// transform values while preserving dependency tracking.
///
/// `Output<T>` implements [`IntoFuture`], so you can `.await` it directly:
///
/// ```ignore
/// let value: String = my_output.await?;
/// ```
///
/// Since `Output<T>` is [`Clone`], awaiting it does not prevent further use —
/// just clone first if you need the output again.
#[derive(Clone)]
pub struct Output<T: Clone + Send + Sync + 'static> {
    future: Shared<BoxFuture<T>>,
    meta: Arc<OutputMeta>,
}

struct OutputMeta {
    deps: Mutex<Vec<String>>,
    known: Mutex<bool>,
    secret: Mutex<bool>,
}

impl OutputMeta {
    fn new() -> Arc<Self> {
        Arc::new(OutputMeta {
            deps: Mutex::new(Vec::new()),
            known: Mutex::new(true),
            secret: Mutex::new(false),
        })
    }

    fn new_unknown() -> Arc<Self> {
        Arc::new(OutputMeta {
            deps: Mutex::new(Vec::new()),
            known: Mutex::new(false),
            secret: Mutex::new(false),
        })
    }

    fn new_secret() -> Arc<Self> {
        Arc::new(OutputMeta {
            deps: Mutex::new(Vec::new()),
            known: Mutex::new(true),
            secret: Mutex::new(true),
        })
    }

    fn copy_from(other: &OutputMeta) -> Arc<Self> {
        Arc::new(OutputMeta {
            deps: Mutex::new(other.deps.lock().unwrap().clone()),
            known: Mutex::new(*other.known.lock().unwrap()),
            secret: Mutex::new(*other.secret.lock().unwrap()),
        })
    }

    /// Merges metadata from another output into this one.
    /// Sets secret if either is secret, unknown if either is unknown, and unions deps.
    fn merge_from(&self, other: &OutputMeta) {
        let other_deps = other.deps.lock().unwrap();
        self.deps.lock().unwrap().extend(other_deps.iter().cloned());

        if *other.secret.lock().unwrap() {
            *self.secret.lock().unwrap() = true;
        }
        if !*other.known.lock().unwrap() {
            *self.known.lock().unwrap() = false;
        }
    }

    /// Creates metadata by merging two sources.
    fn merged(a: &OutputMeta, b: &OutputMeta) -> Arc<Self> {
        let meta = OutputMeta::copy_from(a);
        meta.merge_from(b);
        meta
    }
}

impl<T: Clone + Send + Sync + 'static> Output<T> {
    /// Creates a new output that is already resolved with the given value.
    pub fn new(value: T) -> Self {
        Output {
            future: futures::future::ready(Ok(value)).boxed().shared(),
            meta: OutputMeta::new(),
        }
    }

    /// Creates an output pair: an unresolved output and a resolver to set its value.
    pub fn unresolved() -> (Self, OutputResolver<T>) {
        let (tx, rx) = oneshot::channel();
        let future = async move {
            rx.await.unwrap_or_else(|_| {
                Err(Arc::new(Error::Custom(
                    "output resolver dropped without resolving".into(),
                )))
            })
        }
        .boxed()
        .shared();

        let meta = OutputMeta::new();
        let output = Output {
            future,
            meta: meta.clone(),
        };
        let resolver = OutputResolver {
            sender: Some(tx),
            meta,
        };
        (output, resolver)
    }

    /// Creates an output whose value is unknown (used during preview).
    ///
    /// Awaiting an unknown output returns an error, since the value cannot
    /// be determined during preview. Use [`Output::is_known`] to check
    /// before awaiting.
    pub fn unknown() -> Self {
        let future = futures::future::ready(Err(Arc::new(Error::Custom(
            "output value is unknown".into(),
        ))))
        .boxed()
        .shared();

        Output {
            future,
            meta: OutputMeta::new_unknown(),
        }
    }

    /// Creates a secret output with the given value.
    pub fn secret(value: T) -> Self {
        Output {
            future: futures::future::ready(Ok(value)).boxed().shared(),
            meta: OutputMeta::new_secret(),
        }
    }

    /// Waits for this output to resolve and returns its value, or an error
    /// if the output was rejected or the resolver was dropped.
    ///
    /// This is equivalent to `.await` but works on `&self` without consuming
    /// the output.
    pub async fn get(&self) -> crate::error::Result<T> {
        self.future
            .clone()
            .await
            .map_err(|arc| Error::Custom(arc.to_string()))
    }

    /// Returns the URN dependencies of this output.
    pub fn dependencies(&self) -> Vec<String> {
        self.meta.deps.lock().unwrap().clone()
    }

    /// Returns whether this output's value is known.
    pub fn is_known(&self) -> bool {
        *self.meta.known.lock().unwrap()
    }

    /// Returns whether this output is a secret.
    pub fn is_secret(&self) -> bool {
        *self.meta.secret.lock().unwrap()
    }

    /// Transforms the output value by applying `f` to it once resolved.
    ///
    /// This is the Pulumi equivalent of `apply` — it lets you derive new values
    /// from resource outputs while preserving dependency tracking.
    ///
    /// If the source output was rejected with an error, the error propagates
    /// and `f` is never called.
    ///
    /// ```ignore
    /// let name: Output<String> = bucket.name();
    /// let url: Output<String> = name.map(|n| format!("https://{n}.s3.amazonaws.com"));
    /// ```
    pub fn map<U, F>(&self, f: F) -> Output<U>
    where
        U: Clone + Send + Sync + 'static,
        F: FnOnce(T) -> U + Send + 'static,
    {
        let source = self.future.clone();
        let meta = OutputMeta::copy_from(&self.meta);

        let future = async move {
            match source.await {
                Ok(val) => Ok(f(val)),
                Err(e) => Err(e),
            }
        }
        .boxed()
        .shared();

        Output { future, meta }
    }

    /// Like [`map`](Output::map), but the function returns an `Output<U>`,
    /// which is then flattened.
    ///
    /// If the source output was rejected with an error, the error propagates
    /// and `f` is never called.
    pub fn flat_map<U, F>(&self, f: F) -> Output<U>
    where
        U: Clone + Send + Sync + 'static,
        F: FnOnce(T) -> Output<U> + Send + 'static,
    {
        let source = self.future.clone();
        let meta = OutputMeta::copy_from(&self.meta);
        let out_meta = meta.clone();

        let future = async move {
            match source.await {
                Ok(val) => {
                    let inner = f(val);

                    // Merge all metadata from the inner output.
                    out_meta.merge_from(&inner.meta);

                    inner.future.await
                }
                Err(e) => Err(e),
            }
        }
        .boxed()
        .shared();

        Output { future, meta }
    }
}

impl<T: Clone + Send + Sync + 'static> std::fmt::Debug for Output<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output").finish_non_exhaustive()
    }
}

impl<T: Clone + Send + Sync + 'static> IntoFuture for Output<T> {
    type Output = crate::error::Result<T>;
    type IntoFuture = impl Future<Output = Self::Output> + Send;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            self.future
                .await
                .map_err(|arc| Error::Custom(arc.to_string()))
        }
    }
}

/// Resolves or rejects a pending [`Output`].
///
/// If dropped without calling [`resolve`](OutputResolver::resolve) or
/// [`reject`](OutputResolver::reject), the output is automatically rejected
/// with an error indicating the resolver was dropped.
pub struct OutputResolver<T: Clone + Send + Sync + 'static> {
    sender: Option<oneshot::Sender<Result<T, Arc<Error>>>>,
    meta: Arc<OutputMeta>,
}

impl<T: Clone + Send + Sync + 'static> OutputResolver<T> {
    /// Resolves the output with the given value, waking all waiters.
    pub fn resolve(mut self, value: T) {
        if let Some(tx) = self.sender.take() {
            let _ = tx.send(Ok(value));
        }
    }

    /// Rejects the output with an error, waking all waiters.
    pub fn reject(mut self, error: Error) {
        if let Some(tx) = self.sender.take() {
            let _ = tx.send(Err(Arc::new(error)));
        }
    }

    /// Marks the output as unknown and resolves it with a default value.
    pub fn resolve_unknown(self, default: T) {
        *self.meta.known.lock().unwrap() = false;
        self.resolve(default);
    }

    /// Adds dependency URNs to the output.
    pub fn add_deps(&self, deps: impl IntoIterator<Item = String>) {
        self.meta.deps.lock().unwrap().extend(deps);
    }

    /// Marks the output as containing a secret.
    pub fn set_secret(&self, secret: bool) {
        *self.meta.secret.lock().unwrap() = secret;
    }
}

impl<T: Clone + Send + Sync + 'static> Drop for OutputResolver<T> {
    fn drop(&mut self) {
        if let Some(tx) = self.sender.take() {
            let _ = tx.send(Err(Arc::new(Error::Custom(
                "output resolver dropped without resolving".into(),
            ))));
        }
    }
}

/// Combines two outputs into one that resolves when both are ready.
/// If either output is rejected, the combined output is rejected with that error.
pub fn all2<A, B>(a: &Output<A>, b: &Output<B>) -> Output<(A, B)>
where
    A: Clone + Send + Sync + 'static,
    B: Clone + Send + Sync + 'static,
{
    let fa = a.future.clone();
    let fb = b.future.clone();
    let meta = OutputMeta::merged(&a.meta, &b.meta);

    let future = async move {
        let (ra, rb) = tokio::join!(fa, fb);
        match (ra, rb) {
            (Ok(va), Ok(vb)) => Ok((va, vb)),
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }
    .boxed()
    .shared();

    Output { future, meta }
}

/// Combines three outputs into one.
/// If any output is rejected, the combined output is rejected with that error.
pub fn all3<A, B, C>(a: &Output<A>, b: &Output<B>, c: &Output<C>) -> Output<(A, B, C)>
where
    A: Clone + Send + Sync + 'static,
    B: Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
{
    let fa = a.future.clone();
    let fb = b.future.clone();
    let fc = c.future.clone();
    let meta = OutputMeta::merged(&a.meta, &b.meta);
    meta.merge_from(&c.meta);

    let future = async move {
        let (ra, rb, rc) = tokio::join!(fa, fb, fc);
        match (ra, rb, rc) {
            (Ok(va), Ok(vb), Ok(vc)) => Ok((va, vb, vc)),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
        }
    }
    .boxed()
    .shared();

    Output { future, meta }
}

/// Combines a vector of outputs into a single output of a vector.
/// If any output is rejected, the combined output is rejected with that error.
pub fn all<T>(outputs: Vec<Output<T>>) -> Output<Vec<T>>
where
    T: Clone + Send + Sync + 'static,
{
    let futures: Vec<_> = outputs.iter().map(|o| o.future.clone()).collect();
    let meta = OutputMeta::new();
    for o in &outputs {
        meta.merge_from(&o.meta);
    }

    let future = async move { futures::future::try_join_all(futures).await }
        .boxed()
        .shared();

    Output { future, meta }
}

/// Lifts an async function into an output.
pub fn from_future<T, F>(fut: F) -> Output<T>
where
    T: Clone + Send + Sync + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let future = async move { Ok(fut.await) }.boxed().shared();

    Output {
        future,
        meta: OutputMeta::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_output_new() {
        let out = Output::new(42);
        assert_eq!(out.get().await.unwrap(), 42);
        assert!(out.is_known());
        assert!(!out.is_secret());
    }

    #[tokio::test]
    async fn test_output_unresolved() {
        let (out, resolver) = Output::<i32>::unresolved();
        tokio::spawn(async move {
            resolver.resolve(99);
        });
        assert_eq!(out.get().await.unwrap(), 99);
    }

    #[tokio::test]
    async fn test_output_await() {
        let out = Output::new(42);
        assert_eq!(out.await.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_output_map() {
        let out = Output::new(10);
        let doubled = out.map(|v| v * 2);
        assert_eq!(doubled.get().await.unwrap(), 20);
    }

    #[tokio::test]
    async fn test_output_flat_map() {
        let out = Output::new(5);
        let result = out.flat_map(|v| Output::new(v + 100));
        assert_eq!(result.get().await.unwrap(), 105);
    }

    #[tokio::test]
    async fn test_all() {
        let a = Output::new(1);
        let b = Output::new(2);
        let c = Output::new(3);
        let combined = all(vec![a, b, c]);
        assert_eq!(combined.get().await.unwrap(), vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_all2() {
        let a = Output::new("hello".to_string());
        let b = Output::new(42);
        let combined = all2(&a, &b);
        assert_eq!(combined.get().await.unwrap(), ("hello".to_string(), 42));
    }

    #[tokio::test]
    async fn test_secret_output() {
        let out = Output::secret(42);
        assert!(out.is_secret());
        assert_eq!(out.get().await.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_from_future() {
        let out = from_future(async { 42 });
        assert_eq!(out.get().await.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_reject() {
        let (out, resolver) = Output::<i32>::unresolved();
        resolver.reject(Error::Custom("something went wrong".into()));
        let result = out.get().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("something went wrong"));
    }

    #[tokio::test]
    async fn test_reject_propagates_through_map() {
        let (out, resolver) = Output::<i32>::unresolved();
        let mapped = out.map(|v| v * 2);
        resolver.reject(Error::Custom("upstream failed".into()));
        let result = mapped.get().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("upstream failed"));
    }

    #[tokio::test]
    async fn test_reject_propagates_through_all() {
        let a = Output::new(1);
        let (b, resolver) = Output::<i32>::unresolved();
        let combined = all(vec![a, b]);
        resolver.reject(Error::Custom("b failed".into()));
        let result = combined.get().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("b failed"));
    }

    #[tokio::test]
    async fn test_resolver_dropped_returns_error() {
        let (out, resolver) = Output::<i32>::unresolved();
        drop(resolver);
        let result = out.get().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("resolver dropped"));
    }

    #[tokio::test]
    async fn test_get_multiple_times() {
        let out = Output::new(42);
        assert_eq!(out.get().await.unwrap(), 42);
        assert_eq!(out.get().await.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_clone_and_await() {
        let out = Output::new(42);
        let out2 = out.clone();
        assert_eq!(out.await.unwrap(), 42);
        assert_eq!(out2.await.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_unknown_output() {
        let out = Output::<i32>::unknown();
        assert!(!out.is_known());
        assert!(out.get().await.is_err());
    }

    #[tokio::test]
    async fn test_resolve_unknown() {
        let (out, resolver) = Output::<i32>::unresolved();
        resolver.resolve_unknown(0);
        assert!(!out.is_known());
        assert_eq!(out.get().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_all3() {
        let a = Output::new(1);
        let b = Output::new(2);
        let c = Output::new(3);
        let combined = all3(&a, &b, &c);
        assert_eq!(combined.get().await.unwrap(), (1, 2, 3));
    }

    #[tokio::test]
    async fn test_map_preserves_secret() {
        let out = Output::secret(42);
        let mapped = out.map(|v| v * 2);
        assert!(mapped.is_secret());
        assert_eq!(mapped.get().await.unwrap(), 84);
    }

    #[tokio::test]
    async fn test_map_preserves_deps() {
        let (out, resolver) = Output::<i32>::unresolved();
        resolver.add_deps(vec!["urn:a".into()]);
        resolver.resolve(1);
        let mapped = out.map(|v| v + 1);
        assert_eq!(mapped.dependencies(), vec!["urn:a".to_string()]);
    }

    #[tokio::test]
    async fn test_flat_map_merges_secret() {
        let out = Output::new(1);
        let result = out.flat_map(|_| Output::secret(42));
        // Metadata from the inner output is merged when the future runs.
        assert_eq!(result.get().await.unwrap(), 42);
        assert!(result.is_secret());
    }

    #[tokio::test]
    async fn test_flat_map_merges_unknown() {
        let out = Output::new(1);
        let result = out.flat_map(|v| {
            let (o, resolver) = Output::<i32>::unresolved();
            resolver.resolve_unknown(v);
            o
        });
        // Metadata from the inner output is merged when the future runs.
        let _ = result.get().await;
        assert!(!result.is_known());
    }

    #[tokio::test]
    async fn test_all2_preserves_secret() {
        let a = Output::new(1);
        let b = Output::secret(2);
        let combined = all2(&a, &b);
        assert!(combined.is_secret());
        assert_eq!(combined.get().await.unwrap(), (1, 2));
    }

    #[tokio::test]
    async fn test_all2_merges_deps() {
        let (a, ra) = Output::<i32>::unresolved();
        let (b, rb) = Output::<i32>::unresolved();
        ra.add_deps(vec!["urn:a".into()]);
        rb.add_deps(vec!["urn:b".into()]);
        ra.resolve(1);
        rb.resolve(2);
        let combined = all2(&a, &b);
        let deps = combined.dependencies();
        assert!(deps.contains(&"urn:a".to_string()));
        assert!(deps.contains(&"urn:b".to_string()));
    }

    #[tokio::test]
    async fn test_all_preserves_secret() {
        let a = Output::new(1);
        let b = Output::secret(2);
        let combined = all(vec![a, b]);
        assert!(combined.is_secret());
    }
}
