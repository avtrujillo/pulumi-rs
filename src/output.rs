use std::future::Future;
use std::sync::Arc;

use tokio::sync::oneshot;
use tokio::sync::Mutex;

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
    Pending(Vec<oneshot::Sender<T>>),
    /// The value has been resolved.
    Resolved(T),
}

impl<T: Clone + Send + Sync + 'static> Output<T> {
    /// Creates a new output that is already resolved with the given value.
    pub fn new(value: T) -> Self {
        Output {
            inner: Arc::new(OutputInner {
                data: Mutex::new(OutputState::Resolved(value)),
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
            data: Mutex::new(OutputState::Resolved(value)),
            deps: Mutex::new(Vec::new()),
            known: Mutex::new(true),
            secret: Mutex::new(true),
        });
        Output { inner }
    }

    /// Waits for this output to resolve and returns its value.
    pub async fn get(&self) -> T {
        // Fast path: check if already resolved.
        {
            let data = self.inner.data.lock().await;
            if let OutputState::Resolved(ref val) = *data {
                return val.clone();
            }
        }

        // Slow path: register a waiter.
        let rx = {
            let mut data = self.inner.data.lock().await;
            match *data {
                OutputState::Resolved(ref val) => return val.clone(),
                OutputState::Pending(ref mut waiters) => {
                    let (tx, rx) = oneshot::channel();
                    waiters.push(tx);
                    rx
                }
            }
        };

        rx.await.expect("output resolver dropped without resolving")
    }

    /// Returns the URN dependencies of this output.
    pub async fn dependencies(&self) -> Vec<String> {
        self.inner.deps.lock().await.clone()
    }

    /// Returns whether this output's value is known.
    pub async fn is_known(&self) -> bool {
        *self.inner.known.lock().await
    }

    /// Returns whether this output is a secret.
    pub async fn is_secret(&self) -> bool {
        *self.inner.secret.lock().await
    }

    /// Transforms the output value by applying `f` to it once resolved.
    ///
    /// This is the Pulumi equivalent of `apply` — it lets you derive new values
    /// from resource outputs while preserving dependency tracking.
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
            // Copy deps.
            {
                let source_deps = source_inner.deps.lock().await;
                let mut out_deps = out_inner.deps.lock().await;
                *out_deps = source_deps.clone();
            }
            // Copy known/secret.
            {
                let known = *source_inner.known.lock().await;
                *out_inner.known.lock().await = known;
            }
            {
                let secret = *source_inner.secret.lock().await;
                *out_inner.secret.lock().await = secret;
            }

            let val = source.get().await;
            let mapped = f(val);
            resolver.resolve(mapped).await;
        });

        out
    }

    /// Like [`map`](Output::map), but the function returns an `Output<U>`,
    /// which is then flattened.
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
                let source_deps = source_inner.deps.lock().await;
                let mut out_deps = out_inner.deps.lock().await;
                *out_deps = source_deps.clone();
            }
            {
                let known = *source_inner.known.lock().await;
                *out_inner.known.lock().await = known;
            }
            {
                let secret = *source_inner.secret.lock().await;
                *out_inner.secret.lock().await = secret;
            }

            let val = source.get().await;
            let inner_output = f(val);

            // Also merge deps from the inner output.
            {
                let inner_deps = inner_output.inner.deps.lock().await;
                let mut out_deps = out_inner.deps.lock().await;
                out_deps.extend(inner_deps.iter().cloned());
            }

            let inner_val = inner_output.get().await;
            resolver.resolve(inner_val).await;
        });

        out
    }
}

impl<T: Clone + Send + Sync + 'static> std::fmt::Debug for Output<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output").finish_non_exhaustive()
    }
}

/// Resolves a pending [`Output`] with a value.
pub struct OutputResolver<T: Clone + Send + Sync + 'static> {
    inner: Arc<OutputInner<T>>,
}

impl<T: Clone + Send + Sync + 'static> OutputResolver<T> {
    /// Resolves the output with the given value, waking all waiters.
    pub async fn resolve(self, value: T) {
        let mut data = self.inner.data.lock().await;
        let waiters = match std::mem::replace(&mut *data, OutputState::Resolved(value.clone())) {
            OutputState::Pending(waiters) => waiters,
            OutputState::Resolved(_) => panic!("output resolved twice"),
        };
        for tx in waiters {
            let _ = tx.send(value.clone());
        }
    }

    /// Marks the output as unknown and resolves it with a default value.
    pub async fn resolve_unknown(self, default: T) {
        *self.inner.known.lock().await = false;
        self.resolve(default).await;
    }

    /// Adds dependency URNs to the output.
    pub async fn add_deps(&self, deps: impl IntoIterator<Item = String>) {
        let mut current = self.inner.deps.lock().await;
        current.extend(deps);
    }

    /// Marks the output as containing a secret.
    pub async fn set_secret(&self, secret: bool) {
        *self.inner.secret.lock().await = secret;
    }
}

/// Combines two outputs into one that resolves when both are ready.
pub fn all2<A, B>(a: &Output<A>, b: &Output<B>) -> Output<(A, B)>
where
    A: Clone + Send + Sync + 'static,
    B: Clone + Send + Sync + 'static,
{
    let a = a.clone();
    let b = b.clone();
    let (out, resolver) = Output::<(A, B)>::unresolved();

    tokio::spawn(async move {
        let (va, vb) = tokio::join!(a.get(), b.get());
        resolver.resolve((va, vb)).await;
    });

    out
}

/// Combines three outputs into one.
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
        let (va, vb, vc) = tokio::join!(a.get(), b.get(), c.get());
        resolver.resolve((va, vb, vc)).await;
    });

    out
}

/// Combines a vector of outputs into a single output of a vector.
pub fn all<T>(outputs: Vec<Output<T>>) -> Output<Vec<T>>
where
    T: Clone + Send + Sync + 'static,
{
    let (out, resolver) = Output::<Vec<T>>::unresolved();

    tokio::spawn(async move {
        let mut results = Vec::with_capacity(outputs.len());
        for o in &outputs {
            results.push(o.get().await);
        }
        resolver.resolve(results).await;
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
        resolver.resolve(val).await;
    });

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_output_new() {
        let out = Output::new(42);
        assert_eq!(out.get().await, 42);
        assert!(out.is_known().await);
        assert!(!out.is_secret().await);
    }

    #[tokio::test]
    async fn test_output_unresolved() {
        let (out, resolver) = Output::<i32>::unresolved();
        tokio::spawn(async move {
            resolver.resolve(99).await;
        });
        assert_eq!(out.get().await, 99);
    }

    #[tokio::test]
    async fn test_output_map() {
        let out = Output::new(10);
        let doubled = out.map(|v| v * 2);
        assert_eq!(doubled.get().await, 20);
    }

    #[tokio::test]
    async fn test_output_flat_map() {
        let out = Output::new(5);
        let result = out.flat_map(|v| Output::new(v + 100));
        assert_eq!(result.get().await, 105);
    }

    #[tokio::test]
    async fn test_all() {
        let a = Output::new(1);
        let b = Output::new(2);
        let c = Output::new(3);
        let combined = all(vec![a, b, c]);
        assert_eq!(combined.get().await, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_all2() {
        let a = Output::new("hello".to_string());
        let b = Output::new(42);
        let combined = all2(&a, &b);
        assert_eq!(combined.get().await, ("hello".to_string(), 42));
    }

    #[tokio::test]
    async fn test_secret_output() {
        let out = Output::secret(42);
        assert!(out.is_secret().await);
        assert_eq!(out.get().await, 42);
    }

    #[tokio::test]
    async fn test_from_future() {
        let out = from_future(async { 42 });
        assert_eq!(out.get().await, 42);
    }
}
