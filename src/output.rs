use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;

use futures::future::{FutureExt, Shared};
use tokio::sync::oneshot;

use crate::error::Error;

/// The resolved data from a [`ResourceOutput`], including both the value
/// and Pulumi metadata.
#[derive(Clone, Debug)]
pub struct OutputData<T> {
    /// The resolved value.
    pub value: T,
    /// URNs of resources this value depends on.
    pub deps: Vec<String>,
    /// Whether the value is known (false during preview for new resources).
    pub known: bool,
    /// Whether the value contains sensitive data.
    pub secret: bool,
}

impl<T> OutputData<T> {
    /// Transforms the value, preserving metadata.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> OutputData<U> {
        OutputData {
            value: f(self.value),
            deps: self.deps,
            known: self.known,
            secret: self.secret,
        }
    }

    /// Merges metadata from another `OutputData`, combining deps and
    /// propagating known/secret flags.
    pub fn merge_meta<U>(&mut self, other: &OutputData<U>) {
        self.deps.extend(other.deps.iter().cloned());
        self.known = self.known && other.known;
        self.secret = self.secret || other.secret;
    }
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = Result<OutputData<T>, Arc<Error>>> + Send>>;

/// Represents a resource property that may not yet be known.
///
/// `ResourceOutput<T>` is the core Pulumi type. Resource properties are always
/// wrapped in `ResourceOutput` because their values may depend on other
/// resources that haven't been created yet, or may be unknown during
/// `pulumi preview`.
///
/// Beyond being a future that resolves to `Result<T>`, a `ResourceOutput<T>`
/// carries metadata that propagates through combinators:
///
/// - **Dependencies** — which resource URNs this value depends on
/// - **Known status** — whether the value is known (false during preview)
/// - **Secret status** — whether the value contains sensitive data
///
/// `ResourceOutput<T>` implements [`IntoFuture`], so you can `.await` it:
///
/// ```ignore
/// let value: String = my_output.await?;
/// ```
///
/// Use [`data()`](ResourceOutput::data) to access both the value and metadata.
#[derive(Clone)]
pub struct ResourceOutput<T: Clone + Send + Sync + 'static> {
    future: Shared<BoxFuture<T>>,
}

impl<T: Clone + Send + Sync + 'static> ResourceOutput<T> {
    /// Creates a resolved output with the given value.
    pub fn new(value: T) -> Self {
        ResourceOutput {
            future: futures::future::ready(Ok(OutputData {
                value,
                deps: Vec::new(),
                known: true,
                secret: false,
            }))
            .boxed()
            .shared(),
        }
    }

    /// Creates an unresolved output and a resolver to complete it.
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

        let output = ResourceOutput { future };
        let resolver = OutputResolver { sender: Some(tx) };
        (output, resolver)
    }

    /// Creates an output whose value is unknown (used during preview).
    pub fn unknown() -> Self {
        let (_tx, rx) = oneshot::channel::<Result<OutputData<T>, Arc<Error>>>();
        let future = async move {
            rx.await.unwrap_or_else(|_| {
                Err(Arc::new(Error::Custom("output value is unknown".into())))
            })
        }
        .boxed()
        .shared();

        ResourceOutput { future }
    }

    /// Creates a secret output with the given value.
    pub fn secret(value: T) -> Self {
        ResourceOutput {
            future: futures::future::ready(Ok(OutputData {
                value,
                deps: Vec::new(),
                known: true,
                secret: true,
            }))
            .boxed()
            .shared(),
        }
    }

    /// Waits for this output to resolve and returns its value.
    ///
    /// This is equivalent to `.await` but works on `&self` without consuming
    /// the output.
    pub async fn get(&self) -> crate::error::Result<T> {
        self.data().await.map(|d| d.value)
    }

    /// Waits for this output to resolve and returns the full [`OutputData`],
    /// including metadata (deps, known, secret).
    pub async fn data(&self) -> crate::error::Result<OutputData<T>> {
        self.future
            .clone()
            .await
            .map_err(|arc| Error::Custom(arc.to_string()))
    }

    /// Transforms the output value by applying `f` once resolved.
    ///
    /// Metadata (dependencies, known, secret) propagates automatically.
    /// If the source was rejected, the error propagates and `f` is not called.
    ///
    /// ```ignore
    /// let url = bucket_name.map(|n| format!("https://{n}.s3.amazonaws.com"));
    /// ```
    pub fn map<U, F>(&self, f: F) -> ResourceOutput<U>
    where
        U: Clone + Send + Sync + 'static,
        F: FnOnce(T) -> U + Send + 'static,
    {
        let source = self.future.clone();

        let future = async move { source.await.map(|data| data.map(f)) }
            .boxed()
            .shared();

        ResourceOutput { future }
    }

    /// Like [`map`](ResourceOutput::map), but `f` returns a `ResourceOutput<U>`
    /// which is flattened. Metadata from both outputs is merged.
    pub fn flat_map<U, F>(&self, f: F) -> ResourceOutput<U>
    where
        U: Clone + Send + Sync + 'static,
        F: FnOnce(T) -> ResourceOutput<U> + Send + 'static,
    {
        let source = self.future.clone();

        let future = async move {
            let source_data = source.await?;
            // Extract metadata before consuming value.
            let source_meta = OutputData {
                value: (),
                deps: source_data.deps,
                known: source_data.known,
                secret: source_data.secret,
            };
            let inner = f(source_data.value);
            let mut inner_data = inner.future.await?;
            inner_data.merge_meta(&source_meta);
            Ok(inner_data)
        }
        .boxed()
        .shared();

        ResourceOutput { future }
    }
}

impl<T: Clone + Send + Sync + 'static> std::fmt::Debug for ResourceOutput<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceOutput").finish_non_exhaustive()
    }
}

impl<T: Clone + Send + Sync + 'static> IntoFuture for ResourceOutput<T> {
    type Output = crate::error::Result<T>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            self.future
                .await
                .map(|d| d.value)
                .map_err(|arc| Error::Custom(arc.to_string()))
        })
    }
}

/// Resolves or rejects a pending [`ResourceOutput`].
///
/// If dropped without calling [`resolve`](OutputResolver::resolve) or
/// [`reject`](OutputResolver::reject), the output is automatically rejected
/// with an error.
pub struct OutputResolver<T: Clone + Send + Sync + 'static> {
    sender: Option<oneshot::Sender<Result<OutputData<T>, Arc<Error>>>>,
}

impl<T: Clone + Send + Sync + 'static> OutputResolver<T> {
    /// Resolves the output with a value and default metadata (known, not secret, no deps).
    pub fn resolve(self, value: T) {
        self.resolve_with(OutputData {
            value,
            deps: Vec::new(),
            known: true,
            secret: false,
        });
    }

    /// Resolves the output with a value and full metadata.
    pub fn resolve_with(mut self, data: OutputData<T>) {
        if let Some(tx) = self.sender.take() {
            let _ = tx.send(Ok(data));
        }
    }

    /// Rejects the output with an error.
    pub fn reject(mut self, error: Error) {
        if let Some(tx) = self.sender.take() {
            let _ = tx.send(Err(Arc::new(error)));
        }
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

/// Combines two outputs into one. Metadata is merged.
pub fn all2<A, B>(a: &ResourceOutput<A>, b: &ResourceOutput<B>) -> ResourceOutput<(A, B)>
where
    A: Clone + Send + Sync + 'static,
    B: Clone + Send + Sync + 'static,
{
    let fa = a.future.clone();
    let fb = b.future.clone();

    let future = async move {
        let (ra, rb) = tokio::join!(fa, fb);
        let (da, db) = (ra?, rb?);
        Ok(OutputData {
            value: (da.value, db.value),
            deps: [da.deps, db.deps].concat(),
            known: da.known && db.known,
            secret: da.secret || db.secret,
        })
    }
    .boxed()
    .shared();

    ResourceOutput { future }
}

/// Combines three outputs into one. Metadata is merged.
pub fn all3<A, B, C>(
    a: &ResourceOutput<A>,
    b: &ResourceOutput<B>,
    c: &ResourceOutput<C>,
) -> ResourceOutput<(A, B, C)>
where
    A: Clone + Send + Sync + 'static,
    B: Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
{
    let fa = a.future.clone();
    let fb = b.future.clone();
    let fc = c.future.clone();

    let future = async move {
        let (ra, rb, rc) = tokio::join!(fa, fb, fc);
        let (da, db, dc) = (ra?, rb?, rc?);
        Ok(OutputData {
            value: (da.value, db.value, dc.value),
            deps: [da.deps, db.deps, dc.deps].concat(),
            known: da.known && db.known && dc.known,
            secret: da.secret || db.secret || dc.secret,
        })
    }
    .boxed()
    .shared();

    ResourceOutput { future }
}

/// Combines a vector of outputs into a single output. Metadata is merged.
pub fn all<T>(outputs: Vec<ResourceOutput<T>>) -> ResourceOutput<Vec<T>>
where
    T: Clone + Send + Sync + 'static,
{
    let futures: Vec<_> = outputs.iter().map(|o| o.future.clone()).collect();

    let future = async move {
        let results = futures::future::try_join_all(futures).await?;
        let mut deps = Vec::new();
        let mut known = true;
        let mut secret = false;
        let mut values = Vec::with_capacity(results.len());
        for data in results {
            values.push(data.value);
            deps.extend(data.deps);
            known = known && data.known;
            secret = secret || data.secret;
        }
        Ok(OutputData {
            value: values,
            deps,
            known,
            secret,
        })
    }
    .boxed()
    .shared();

    ResourceOutput { future }
}

/// Lifts an async function into an output.
pub fn from_future<T, F>(fut: F) -> ResourceOutput<T>
where
    T: Clone + Send + Sync + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let future = async move {
        Ok(OutputData {
            value: fut.await,
            deps: Vec::new(),
            known: true,
            secret: false,
        })
    }
    .boxed()
    .shared();

    ResourceOutput { future }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_output_new() {
        let out = ResourceOutput::new(42);
        let data = out.data().await.unwrap();
        assert_eq!(data.value, 42);
        assert!(data.known);
        assert!(!data.secret);
    }

    #[tokio::test]
    async fn test_output_unresolved() {
        let (out, resolver) = ResourceOutput::<i32>::unresolved();
        tokio::spawn(async move {
            resolver.resolve(99);
        });
        assert_eq!(out.get().await.unwrap(), 99);
    }

    #[tokio::test]
    async fn test_output_await() {
        let out = ResourceOutput::new(42);
        assert_eq!(out.await.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_output_map() {
        let out = ResourceOutput::new(10);
        let doubled = out.map(|v| v * 2);
        assert_eq!(doubled.get().await.unwrap(), 20);
    }

    #[tokio::test]
    async fn test_output_flat_map() {
        let out = ResourceOutput::new(5);
        let result = out.flat_map(|v| ResourceOutput::new(v + 100));
        assert_eq!(result.get().await.unwrap(), 105);
    }

    #[tokio::test]
    async fn test_all() {
        let a = ResourceOutput::new(1);
        let b = ResourceOutput::new(2);
        let c = ResourceOutput::new(3);
        let combined = all(vec![a, b, c]);
        assert_eq!(combined.get().await.unwrap(), vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_all2() {
        let a = ResourceOutput::new("hello".to_string());
        let b = ResourceOutput::new(42);
        let combined = all2(&a, &b);
        assert_eq!(
            combined.get().await.unwrap(),
            ("hello".to_string(), 42)
        );
    }

    #[tokio::test]
    async fn test_secret_output() {
        let out = ResourceOutput::secret(42);
        let data = out.data().await.unwrap();
        assert!(data.secret);
        assert_eq!(data.value, 42);
    }

    #[tokio::test]
    async fn test_from_future() {
        let out = from_future(async { 42 });
        assert_eq!(out.get().await.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_reject() {
        let (out, resolver) = ResourceOutput::<i32>::unresolved();
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
        let (out, resolver) = ResourceOutput::<i32>::unresolved();
        let mapped = out.map(|v| v * 2);
        resolver.reject(Error::Custom("upstream failed".into()));
        let result = mapped.get().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("upstream failed"));
    }

    #[tokio::test]
    async fn test_reject_propagates_through_all() {
        let a = ResourceOutput::new(1);
        let (b, resolver) = ResourceOutput::<i32>::unresolved();
        let combined = all(vec![a, b]);
        resolver.reject(Error::Custom("b failed".into()));
        let result = combined.get().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("b failed"));
    }

    #[tokio::test]
    async fn test_resolver_dropped_returns_error() {
        let (out, resolver) = ResourceOutput::<i32>::unresolved();
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
        let out = ResourceOutput::new(42);
        assert_eq!(out.get().await.unwrap(), 42);
        assert_eq!(out.get().await.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_clone_and_await() {
        let out = ResourceOutput::new(42);
        let out2 = out.clone();
        assert_eq!(out.await.unwrap(), 42);
        assert_eq!(out2.await.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_metadata_propagates_through_map() {
        let (out, resolver) = ResourceOutput::<i32>::unresolved();
        let mapped = out.map(|v| v * 2);
        resolver.resolve_with(OutputData {
            value: 21,
            deps: vec!["urn:some:resource".into()],
            known: true,
            secret: true,
        });
        let data = mapped.data().await.unwrap();
        assert_eq!(data.value, 42);
        assert_eq!(data.deps, vec!["urn:some:resource"]);
        assert!(data.secret);
    }

    #[tokio::test]
    async fn test_metadata_merges_through_flat_map() {
        let (out, resolver) = ResourceOutput::<i32>::unresolved();
        let result = out.flat_map(|v| {
            let (inner, inner_resolver) = ResourceOutput::<i32>::unresolved();
            inner_resolver.resolve_with(OutputData {
                value: v + 1,
                deps: vec!["urn:inner".into()],
                known: true,
                secret: false,
            });
            inner
        });
        resolver.resolve_with(OutputData {
            value: 10,
            deps: vec!["urn:outer".into()],
            known: true,
            secret: true,
        });
        let data = result.data().await.unwrap();
        assert_eq!(data.value, 11);
        assert!(data.deps.contains(&"urn:outer".to_string()));
        assert!(data.deps.contains(&"urn:inner".to_string()));
        assert!(data.secret); // outer was secret
    }

    #[tokio::test]
    async fn test_all2_merges_metadata() {
        let a = ResourceOutput::secret(1);
        let b = ResourceOutput::new(2);
        let combined = all2(&a, &b);
        let data = combined.data().await.unwrap();
        assert!(data.secret); // a was secret
        assert_eq!(data.value, (1, 2));
    }
}
