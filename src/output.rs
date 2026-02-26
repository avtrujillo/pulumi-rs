use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

use crate::error::Error;

/// Represents a value that may not yet be known.
///
/// `Output<T>` is the core Pulumi type. Resource properties are always wrapped in
/// `Output` because their values may depend on other resources that haven't been
/// created yet, or may be unknown during `pulumi preview`.
///
/// Outputs support combinators like [`Output::map`] and [`Output::flat_map`] to
/// transform values while preserving dependency tracking.
#[derive(Clone)]
pub struct Output<T: Clone + Send + Sync + 'static> {
    inner: Arc<OutputInner<T>>,
}

struct OutputInner<T: Clone + Send + Sync + 'static> {
    data: Mutex<OutputState<T>>,
    /// URNs of resources this output depends on.
    deps: Mutex<Vec<String>>,
    /// Whether this output's value is known (false during preview for new resources).
    known: Mutex<bool>,
    /// Whether this output contains a secret value.
    secret: Mutex<bool>,
}

enum OutputState<T> {
    /// The value is pending resolution.
    Pending(Vec<oneshot::Sender<Result<T, Arc<Error>>>>),
    /// The value has been resolved successfully or with an error.
    Resolved(Result<T, Arc<Error>>),
}

impl<T: Clone + Send + Sync + 'static> Output<T> {
    /// Creates a new output that is already resolved with the given value.
    pub fn new(value: T) -> Self {
        Output {
            inner: Arc::new(OutputInner {
                data: Mutex::new(OutputState::Resolved(Ok(value))),
                deps: Mutex::new(Vec::new()),
                known: Mutex::new(true),
                secret: Mutex::new(false),
            }),
        }
    }

    /// Creates an output pair: an unresolved output and a resolver to set its value.
    pub fn unresolved() -> (Self, OutputResolver<T>) {
        let inner = Arc::new(OutputInner {
            data: Mutex::new(OutputState::Pending(Vec::new())),
            deps: Mutex::new(Vec::new()),
            known: Mutex::new(true),
            secret: Mutex::new(false),
        });
        let output = Output {
            inner: inner.clone(),
        };
        let resolver = OutputResolver { inner };
        (output, resolver)
    }

    /// Creates an output whose value is unknown (used during preview).
    pub fn unknown() -> Self {
        let inner = Arc::new(OutputInner {
            data: Mutex::new(OutputState::Pending(Vec::new())),
            deps: Mutex::new(Vec::new()),
            known: Mutex::new(false),
            secret: Mutex::new(false),
        });
        Output { inner }
    }

    /// Creates a secret output with the given value.
    pub fn secret(value: T) -> Self {
        let inner = Arc::new(OutputInner {
            data: Mutex::new(OutputState::Resolved(Ok(value))),
            deps: Mutex::new(Vec::new()),
            known: Mutex::new(true),
            secret: Mutex::new(true),
        });
        Output { inner }
    }

    /// Waits for this output to resolve and returns its value, or an error
    /// if the output was rejected or the resolver was dropped.
    pub async fn get(&self) -> crate::error::Result<T> {
        // Fast path: check if already resolved.
        {
            let data = self.inner.data.lock().unwrap();
            if let OutputState::Resolved(ref result) = *data {
                return result
                    .clone()
                    .map_err(|arc| Error::Custom(arc.to_string()));
            }
        }

        // Slow path: register a waiter.
        let rx = {
            let mut data = self.inner.data.lock().unwrap();
            match *data {
                OutputState::Resolved(ref result) => {
                    return result
                        .clone()
                        .map_err(|arc| Error::Custom(arc.to_string()));
                }
                OutputState::Pending(ref mut waiters) => {
                    let (tx, rx) = oneshot::channel();
                    waiters.push(tx);
                    rx
                }
            }
        };

        rx.await
            .map_err(|_| Error::Custom("output resolver dropped without resolving".into()))?
            .map_err(|arc| Error::Custom(arc.to_string()))
    }

    /// Returns the URN dependencies of this output.
    pub fn dependencies(&self) -> Vec<String> {
        self.inner.deps.lock().unwrap().clone()
    }

    /// Returns whether this output's value is known.
    pub fn is_known(&self) -> bool {
        *self.inner.known.lock().unwrap()
    }

    /// Returns whether this output is a secret.
    pub fn is_secret(&self) -> bool {
        *self.inner.secret.lock().unwrap()
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
        let source = self.clone();
        let (out, resolver) = Output::<U>::unresolved();

        // Propagate metadata.
        let out_inner = out.inner.clone();
        let source_inner = self.inner.clone();

        tokio::spawn(async move {
            // Copy deps/known/secret.
            {
                let source_deps = source_inner.deps.lock().unwrap();
                let mut out_deps = out_inner.deps.lock().unwrap();
                *out_deps = source_deps.clone();
            }
            {
                let known = *source_inner.known.lock().unwrap();
                *out_inner.known.lock().unwrap() = known;
            }
            {
                let secret = *source_inner.secret.lock().unwrap();
                *out_inner.secret.lock().unwrap() = secret;
            }

            match source.get().await {
                Ok(val) => resolver.resolve(f(val)),
                Err(e) => resolver.reject(e),
            }
        });

        out
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
        let source = self.clone();
        let (out, resolver) = Output::<U>::unresolved();

        let out_inner = out.inner.clone();
        let source_inner = self.inner.clone();

        tokio::spawn(async move {
            // Copy deps from source.
            {
                let source_deps = source_inner.deps.lock().unwrap();
                let mut out_deps = out_inner.deps.lock().unwrap();
                *out_deps = source_deps.clone();
            }
            {
                let known = *source_inner.known.lock().unwrap();
                *out_inner.known.lock().unwrap() = known;
            }
            {
                let secret = *source_inner.secret.lock().unwrap();
                *out_inner.secret.lock().unwrap() = secret;
            }

            match source.get().await {
                Ok(val) => {
                    let inner_output = f(val);

                    // Also merge deps from the inner output.
                    {
                        let inner_deps = inner_output.inner.deps.lock().unwrap();
                        let mut out_deps = out_inner.deps.lock().unwrap();
                        out_deps.extend(inner_deps.iter().cloned());
                    }

                    match inner_output.get().await {
                        Ok(inner_val) => resolver.resolve(inner_val),
                        Err(e) => resolver.reject(e),
                    }
                }
                Err(e) => resolver.reject(e),
            }
        });

        out
    }
}

impl<T: Clone + Send + Sync + 'static> std::fmt::Debug for Output<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output").finish_non_exhaustive()
    }
}

/// Resolves or rejects a pending [`Output`].
///
/// If dropped without calling [`resolve`](OutputResolver::resolve) or
/// [`reject`](OutputResolver::reject), the output is automatically rejected
/// with an error indicating the resolver was dropped.
pub struct OutputResolver<T: Clone + Send + Sync + 'static> {
    inner: Arc<OutputInner<T>>,
}

impl<T: Clone + Send + Sync + 'static> OutputResolver<T> {
    /// Resolves the output with the given value, waking all waiters.
    pub fn resolve(self, value: T) {
        self.complete(Ok(value));
    }

    /// Rejects the output with an error, waking all waiters.
    pub fn reject(self, error: Error) {
        self.complete(Err(error));
    }

    fn complete(self, result: Result<T, Error>) {
        // Use ManuallyDrop to prevent Drop from running after we complete.
        let me = std::mem::ManuallyDrop::new(self);
        Self::complete_inner(&me.inner, result);
    }

    fn complete_inner(inner: &Arc<OutputInner<T>>, result: Result<T, Error>) {
        let arc_result = result.map_err(Arc::new);
        let mut data = inner.data.lock().unwrap();
        let waiters = match std::mem::replace(&mut *data, OutputState::Resolved(arc_result.clone()))
        {
            OutputState::Pending(waiters) => waiters,
            OutputState::Resolved(_) => return,
        };
        for tx in waiters {
            let _ = tx.send(arc_result.clone());
        }
    }

    /// Marks the output as unknown and resolves it with a default value.
    pub fn resolve_unknown(self, default: T) {
        *self.inner.known.lock().unwrap() = false;
        self.resolve(default);
    }

    /// Adds dependency URNs to the output.
    pub fn add_deps(&self, deps: impl IntoIterator<Item = String>) {
        let mut current = self.inner.deps.lock().unwrap();
        current.extend(deps);
    }

    /// Marks the output as containing a secret.
    pub fn set_secret(&self, secret: bool) {
        *self.inner.secret.lock().unwrap() = secret;
    }
}

impl<T: Clone + Send + Sync + 'static> Drop for OutputResolver<T> {
    fn drop(&mut self) {
        // If the resolver is dropped without resolving, reject with an error.
        let data = self.inner.data.lock().unwrap();
        if matches!(*data, OutputState::Pending(_)) {
            drop(data);
            Self::complete_inner(
                &self.inner,
                Err(Error::Custom(
                    "output resolver dropped without resolving".into(),
                )),
            );
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
    let a = a.clone();
    let b = b.clone();
    let (out, resolver) = Output::<(A, B)>::unresolved();

    tokio::spawn(async move {
        let (ra, rb) = tokio::join!(a.get(), b.get());
        match (ra, rb) {
            (Ok(va), Ok(vb)) => resolver.resolve((va, vb)),
            (Err(e), _) | (_, Err(e)) => resolver.reject(e),
        }
    });

    out
}

/// Combines three outputs into one.
/// If any output is rejected, the combined output is rejected with that error.
pub fn all3<A, B, C>(a: &Output<A>, b: &Output<B>, c: &Output<C>) -> Output<(A, B, C)>
where
    A: Clone + Send + Sync + 'static,
    B: Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
{
    let a = a.clone();
    let b = b.clone();
    let c = c.clone();
    let (out, resolver) = Output::<(A, B, C)>::unresolved();

    tokio::spawn(async move {
        let (ra, rb, rc) = tokio::join!(a.get(), b.get(), c.get());
        match (ra, rb, rc) {
            (Ok(va), Ok(vb), Ok(vc)) => resolver.resolve((va, vb, vc)),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => resolver.reject(e),
        }
    });

    out
}

/// Combines a vector of outputs into a single output of a vector.
/// If any output is rejected, the combined output is rejected with that error.
pub fn all<T>(outputs: Vec<Output<T>>) -> Output<Vec<T>>
where
    T: Clone + Send + Sync + 'static,
{
    let (out, resolver) = Output::<Vec<T>>::unresolved();

    tokio::spawn(async move {
        let mut results = Vec::with_capacity(outputs.len());
        for o in &outputs {
            match o.get().await {
                Ok(val) => results.push(val),
                Err(e) => {
                    resolver.reject(e);
                    return;
                }
            }
        }
        resolver.resolve(results);
    });

    out
}

/// Lifts an async function into an output.
pub fn from_future<T, F>(fut: F) -> Output<T>
where
    T: Clone + Send + Sync + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let (out, resolver) = Output::<T>::unresolved();

    tokio::spawn(async move {
        let val = fut.await;
        resolver.resolve(val);
    });

    out
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
}
