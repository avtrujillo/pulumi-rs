use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::{FutureExt, Shared};
use tokio::sync::Mutex;
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

/// Metadata about an output's dependency tracking, secretness, and knowability.
///
/// `known` and `secret` are simple booleans that only ever transition in one
/// direction (known→unknown, non-secret→secret), so `AtomicBool` gives us
/// lock-free synchronous reads without any unwrap. `deps` can accumulate new
/// entries asynchronously as combinators resolve, so it lives behind a
/// `tokio::sync::Mutex` (no poisoning; `.lock().await` is infallible).
struct OutputMeta {
    deps: Mutex<Vec<String>>,
    known: AtomicBool,
    secret: AtomicBool,
}

impl OutputMeta {
    fn new() -> Arc<Self> {
        Arc::new(OutputMeta {
            deps: Mutex::new(Vec::new()),
            known: AtomicBool::new(true),
            secret: AtomicBool::new(false),
        })
    }

    fn new_unknown() -> Arc<Self> {
        Arc::new(OutputMeta {
            deps: Mutex::new(Vec::new()),
            known: AtomicBool::new(false),
            secret: AtomicBool::new(false),
        })
    }

    fn new_secret() -> Arc<Self> {
        Arc::new(OutputMeta {
            deps: Mutex::new(Vec::new()),
            known: AtomicBool::new(true),
            secret: AtomicBool::new(true),
        })
    }

    fn is_known(&self) -> bool {
        self.known.load(Ordering::Acquire)
    }

    fn is_secret(&self) -> bool {
        self.secret.load(Ordering::Acquire)
    }

    fn set_known(&self, v: bool) {
        self.known.store(v, Ordering::Release);
    }

    fn set_secret(&self, v: bool) {
        self.secret.store(v, Ordering::Release);
    }

    async fn get_deps(&self) -> Vec<String> {
        self.deps.lock().await.clone()
    }

    async fn extend_deps(&self, it: impl IntoIterator<Item = String>) {
        self.deps.lock().await.extend(it);
    }

    /// Merge `other`'s metadata into `self`: deps union, known AND, secret OR.
    async fn merge_from(&self, other: &OutputMeta) {
        self.extend_deps(other.get_deps().await).await;
        if !other.is_known() {
            self.set_known(false);
        }
        if other.is_secret() {
            self.set_secret(true);
        }
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
    pub fn unknown() -> Self {
        // Unknown outputs never resolve — they represent values that
        // can't be determined during preview.
        let (_tx, rx) = oneshot::channel::<Result<T, Arc<Error>>>();
        let future = async move {
            rx.await
                .unwrap_or_else(|_| Err(Arc::new(Error::Custom("output value is unknown".into()))))
        }
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
    pub async fn dependencies(&self) -> Vec<String> {
        self.meta.get_deps().await
    }

    /// Returns whether this output's value is known.
    pub fn is_known(&self) -> bool {
        self.meta.is_known()
    }

    /// Returns whether this output is a secret.
    pub fn is_secret(&self) -> bool {
        self.meta.is_secret()
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
        let source_meta = self.meta.clone();

        let meta = Arc::new(OutputMeta {
            deps: Mutex::new(Vec::new()),
            known: AtomicBool::new(self.meta.is_known()),
            secret: AtomicBool::new(self.meta.is_secret()),
        });
        let out_meta = meta.clone();

        let future = async move {
            out_meta.extend_deps(source_meta.get_deps().await).await;
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
        let source_meta = self.meta.clone();

        let meta = Arc::new(OutputMeta {
            deps: Mutex::new(Vec::new()),
            known: AtomicBool::new(self.meta.is_known()),
            secret: AtomicBool::new(self.meta.is_secret()),
        });
        let out_meta = meta.clone();

        let future = async move {
            out_meta.extend_deps(source_meta.get_deps().await).await;
            match source.await {
                Ok(val) => {
                    let inner = f(val);
                    out_meta.merge_from(&inner.meta).await;
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
        self.meta.set_known(false);
        self.resolve(default);
    }

    /// Adds dependency URNs to the output.
    pub async fn add_deps(&self, deps: impl IntoIterator<Item = String>) {
        self.meta.extend_deps(deps).await;
    }

    /// Marks the output as containing a secret.
    pub fn set_secret(&self, secret: bool) {
        self.meta.set_secret(secret);
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
///
/// Metadata is merged: deps are unioned, the result is secret if either input
/// is secret, and the result is unknown if either input is unknown.
/// If either output is rejected, the combined output is rejected with that error.
pub fn all2<A, B>(a: &Output<A>, b: &Output<B>) -> Output<(A, B)>
where
    A: Clone + Send + Sync + 'static,
    B: Clone + Send + Sync + 'static,
{
    let fa = a.future.clone();
    let fb = b.future.clone();
    let a_meta = a.meta.clone();
    let b_meta = b.meta.clone();

    let meta = Arc::new(OutputMeta {
        deps: Mutex::new(Vec::new()),
        known: AtomicBool::new(a.meta.is_known() && b.meta.is_known()),
        secret: AtomicBool::new(a.meta.is_secret() || b.meta.is_secret()),
    });
    let out_meta = meta.clone();

    let future = async move {
        out_meta.extend_deps(a_meta.get_deps().await).await;
        out_meta.extend_deps(b_meta.get_deps().await).await;

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
///
/// Metadata is merged: deps are unioned, the result is secret if any input
/// is secret, and the result is unknown if any input is unknown.
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
    let a_meta = a.meta.clone();
    let b_meta = b.meta.clone();
    let c_meta = c.meta.clone();

    let meta = Arc::new(OutputMeta {
        deps: Mutex::new(Vec::new()),
        known: AtomicBool::new(a.meta.is_known() && b.meta.is_known() && c.meta.is_known()),
        secret: AtomicBool::new(
            a.meta.is_secret() || b.meta.is_secret() || c.meta.is_secret(),
        ),
    });
    let out_meta = meta.clone();

    let future = async move {
        out_meta.extend_deps(a_meta.get_deps().await).await;
        out_meta.extend_deps(b_meta.get_deps().await).await;
        out_meta.extend_deps(c_meta.get_deps().await).await;

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
///
/// Metadata is merged: deps are unioned, the result is secret if any input
/// is secret, and the result is unknown if any input is unknown.
/// If any output is rejected, the combined output is rejected with that error.
pub fn all<T>(outputs: Vec<Output<T>>) -> Output<Vec<T>>
where
    T: Clone + Send + Sync + 'static,
{
    let known = outputs.iter().all(|o| o.meta.is_known());
    let secret = outputs.iter().any(|o| o.meta.is_secret());

    let metas: Vec<Arc<OutputMeta>> = outputs.iter().map(|o| o.meta.clone()).collect();
    let futures: Vec<_> = outputs.iter().map(|o| o.future.clone()).collect();

    let meta = Arc::new(OutputMeta {
        deps: Mutex::new(Vec::new()),
        known: AtomicBool::new(known),
        secret: AtomicBool::new(secret),
    });
    let out_meta = meta.clone();

    let future = async move {
        for m in &metas {
            out_meta.extend_deps(m.get_deps().await).await;
        }
        futures::future::try_join_all(futures).await
    }
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

    /// A resolved output starts known, non-secret, and its value is immediately available.
    #[tokio::test]
    async fn test_output_new() {
        let out = Output::new(42);
        assert_eq!(out.get().await.unwrap(), 42);
        assert!(out.is_known());
        assert!(!out.is_secret());
    }

    /// An unresolved output blocks until the resolver sends a value.
    #[tokio::test]
    async fn test_output_unresolved() {
        let (out, resolver) = Output::<i32>::unresolved();
        tokio::spawn(async move {
            resolver.resolve(99);
        });
        assert_eq!(out.get().await.unwrap(), 99);
    }

    /// `Output` implements `IntoFuture` so it can be awaited directly.
    #[tokio::test]
    async fn test_output_await() {
        let out = Output::new(42);
        assert_eq!(out.await.unwrap(), 42);
    }

    /// `map` transforms the value while preserving metadata.
    #[tokio::test]
    async fn test_output_map() {
        let out = Output::new(10);
        let doubled = out.map(|v| v * 2);
        assert_eq!(doubled.get().await.unwrap(), 20);
    }

    /// `flat_map` flattens a nested output correctly.
    #[tokio::test]
    async fn test_output_flat_map() {
        let out = Output::new(5);
        let result = out.flat_map(|v| Output::new(v + 100));
        assert_eq!(result.get().await.unwrap(), 105);
    }

    /// `all` combines a vec of outputs into one.
    #[tokio::test]
    async fn test_all() {
        let a = Output::new(1);
        let b = Output::new(2);
        let c = Output::new(3);
        let combined = all(vec![a, b, c]);
        assert_eq!(combined.get().await.unwrap(), vec![1, 2, 3]);
    }

    /// `all2` combines two outputs of different types into a tuple.
    #[tokio::test]
    async fn test_all2() {
        let a = Output::new("hello".to_string());
        let b = Output::new(42);
        let combined = all2(&a, &b);
        assert_eq!(combined.get().await.unwrap(), ("hello".to_string(), 42));
    }

    /// A secret output reports `is_secret() == true` and its value is still accessible.
    #[tokio::test]
    async fn test_secret_output() {
        let out = Output::secret(42);
        assert!(out.is_secret());
        assert_eq!(out.get().await.unwrap(), 42);
    }

    /// `from_future` wraps an async computation in a non-secret, known output.
    #[tokio::test]
    async fn test_from_future() {
        let out = from_future(async { 42 });
        assert_eq!(out.get().await.unwrap(), 42);
    }

    /// `reject` propagates the error to all waiters.
    #[tokio::test]
    async fn test_reject() {
        let (out, resolver) = Output::<i32>::unresolved();
        resolver.reject(Error::Custom("something went wrong".into()));
        let result = out.get().await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("something went wrong")
        );
    }

    /// An error from an upstream output propagates through `map`.
    #[tokio::test]
    async fn test_reject_propagates_through_map() {
        let (out, resolver) = Output::<i32>::unresolved();
        let mapped = out.map(|v| v * 2);
        resolver.reject(Error::Custom("upstream failed".into()));
        let result = mapped.get().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("upstream failed"));
    }

    /// An error from one element of `all` propagates to the combined output.
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

    /// Dropping the resolver without resolving yields an error.
    #[tokio::test]
    async fn test_resolver_dropped_returns_error() {
        let (out, resolver) = Output::<i32>::unresolved();
        drop(resolver);
        let result = out.get().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("resolver dropped"));
    }

    /// Calling `get` multiple times on the same output returns the same result.
    #[tokio::test]
    async fn test_get_multiple_times() {
        let out = Output::new(42);
        assert_eq!(out.get().await.unwrap(), 42);
        assert_eq!(out.get().await.unwrap(), 42);
    }

    /// Cloning an output and awaiting both clones returns the same value.
    #[tokio::test]
    async fn test_clone_and_await() {
        let out = Output::new(42);
        let out2 = out.clone();
        assert_eq!(out.await.unwrap(), 42);
        assert_eq!(out2.await.unwrap(), 42);
    }

    // --- Secret propagation through combinators ---

    /// `all2` is secret when either input is secret.
    #[tokio::test]
    async fn test_all2_propagates_secret_from_first() {
        let secret = Output::secret(1);
        let plain = Output::new(2);
        let combined = all2(&secret, &plain);
        assert!(combined.is_secret());
    }

    /// `all2` is secret when the second input is secret.
    #[tokio::test]
    async fn test_all2_propagates_secret_from_second() {
        let plain = Output::new(1);
        let secret = Output::secret(2);
        let combined = all2(&plain, &secret);
        assert!(combined.is_secret());
    }

    /// `all2` is not secret when neither input is secret.
    #[tokio::test]
    async fn test_all2_no_false_positive_secret() {
        let a = Output::new(1);
        let b = Output::new(2);
        let combined = all2(&a, &b);
        assert!(!combined.is_secret());
    }

    /// `all3` is secret when any input is secret.
    #[tokio::test]
    async fn test_all3_propagates_secret_from_middle() {
        let a = Output::new(1);
        let b = Output::secret(2);
        let c = Output::new(3);
        let combined = all3(&a, &b, &c);
        assert!(combined.is_secret());
    }

    /// `all` (vec) is secret when any element is secret.
    #[tokio::test]
    async fn test_all_vec_propagates_secret() {
        let outputs = vec![Output::new(1), Output::secret(2), Output::new(3)];
        let combined = all(outputs);
        assert!(combined.is_secret());
    }

    /// `all` (vec) is not secret when no element is secret.
    #[tokio::test]
    async fn test_all_vec_no_false_positive_secret() {
        let outputs = vec![Output::new(1), Output::new(2)];
        let combined = all(outputs);
        assert!(!combined.is_secret());
    }

    /// `flat_map` propagates secret from the inner output.
    #[tokio::test]
    async fn test_flat_map_propagates_secret_from_inner() {
        let plain = Output::new(42);
        let result = plain.flat_map(|_| Output::secret(99));
        assert!(result.is_secret());
        assert_eq!(result.get().await.unwrap(), 99);
    }

    /// `flat_map` propagates secret from the outer (source) output.
    #[tokio::test]
    async fn test_flat_map_propagates_secret_from_outer() {
        let secret = Output::secret(42);
        let result = secret.flat_map(|v| Output::new(v + 1));
        assert!(result.is_secret());
    }

    /// `flat_map` is not secret when neither outer nor inner is secret.
    #[tokio::test]
    async fn test_flat_map_no_false_positive_secret() {
        let plain = Output::new(1);
        let result = plain.flat_map(|v| Output::new(v * 2));
        assert!(!result.is_secret());
    }

    // --- Known/unknown propagation ---

    /// `all2` is unknown when either input is unknown.
    #[tokio::test]
    async fn test_all2_propagates_unknown() {
        let unknown = Output::<i32>::unknown();
        let known = Output::new(2);
        let combined = all2(&unknown, &known);
        assert!(!combined.is_known());
    }

    /// `flat_map` propagates unknown from the inner output.
    #[tokio::test]
    async fn test_flat_map_propagates_unknown_from_inner() {
        let known = Output::new(1);
        let result = known.flat_map(|_| Output::<i32>::unknown());
        assert!(!result.is_known());
    }
}