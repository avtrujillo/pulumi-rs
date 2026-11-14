use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;

use futures::future::{FutureExt, Shared};
use tokio::sync::oneshot;

use crate::error::Error;

type Resolved<T> = Result<OutputData<T>, Arc<Error>>;
type BoxFuture<T> = Pin<Box<dyn Future<Output = Resolved<T>> + Send>>;

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
///
/// # Metadata
///
/// Whether an output is known, whether it is secret, and which resources it
/// depends on are all facts about the *resolved* output: a
/// [`flat_map`](Output::flat_map) callback can return a secret or unknown
/// output, and a resolver can mark its output secret right up until it
/// resolves. [`is_known`](Output::is_known), [`is_secret`](Output::is_secret)
/// and [`dependencies`](Output::dependencies) are therefore async and wait for
/// the output to resolve, as they do in the other Pulumi SDKs.
#[derive(Clone)]
pub struct Output<T: Clone + Send + Sync + 'static> {
    future: Shared<BoxFuture<T>>,
}

/// What an output resolves to: the value together with its metadata.
///
/// Carrying the metadata alongside the value (rather than in shared mutable
/// state next to the future) means combinators merge it at the same moment
/// they produce the value, so it can never be observed half-propagated.
#[derive(Clone)]
struct OutputData<T> {
    /// `None` when there is no value at all (see [`Output::unknown`]).
    /// Invariant: `value.is_none()` implies `!known`.
    value: Option<T>,
    known: bool,
    secret: bool,
    /// URNs of the resources this output depends on, deduplicated, in the
    /// order they were first seen.
    deps: Vec<String>,
}

impl<T> OutputData<T> {
    fn known(value: T) -> Self {
        OutputData {
            value: Some(value),
            known: true,
            secret: false,
            deps: Vec::new(),
        }
    }

    fn unknown() -> Self {
        OutputData {
            value: None,
            known: false,
            secret: false,
            deps: Vec::new(),
        }
    }

    /// Applies `f` to the value, if there is one, keeping the metadata.
    fn map<U>(self, f: impl FnOnce(T) -> U) -> OutputData<U> {
        OutputData {
            value: self.value.map(f),
            known: self.known,
            secret: self.secret,
            deps: self.deps,
        }
    }

    /// Replaces the value with `inner`'s and merges the metadata: known AND,
    /// secret OR, deps union.
    fn and_then<U>(self, inner: OutputData<U>) -> OutputData<U> {
        OutputData {
            value: inner.value,
            known: self.known && inner.known,
            secret: self.secret || inner.secret,
            deps: merge_deps(self.deps, inner.deps),
        }
    }

    /// Pairs two values and merges the metadata: known AND, secret OR, deps
    /// union.
    fn zip<U>(self, other: OutputData<U>) -> OutputData<(T, U)> {
        OutputData {
            value: self.value.zip(other.value),
            known: self.known && other.known,
            secret: self.secret || other.secret,
            deps: merge_deps(self.deps, other.deps),
        }
    }
}

/// Appends the entries of `extra` that `deps` does not already contain.
fn merge_deps(mut deps: Vec<String>, extra: impl IntoIterator<Item = String>) -> Vec<String> {
    for dep in extra {
        if !deps.contains(&dep) {
            deps.push(dep);
        }
    }
    deps
}

fn resolver_dropped_error() -> Arc<Error> {
    Arc::new(Error::Custom(
        "output resolver dropped without resolving".into(),
    ))
}

/// Converts what an output resolved to into what awaiting it returns.
fn into_value<T>(resolved: Resolved<T>) -> crate::error::Result<T> {
    match resolved {
        Ok(data) => data
            .value
            .ok_or_else(|| Error::Custom("output value is unknown".into())),
        Err(arc) => Err(Error::Custom(arc.to_string())),
    }
}

impl<T: Clone + Send + Sync + 'static> Output<T> {
    fn from_data(data: OutputData<T>) -> Self {
        Output {
            future: futures::future::ready(Ok(data)).boxed().shared(),
        }
    }

    /// Creates a new output that is already resolved with the given value.
    pub fn new(value: T) -> Self {
        Self::from_data(OutputData::known(value))
    }

    /// Creates an output pair: an unresolved output and a resolver to set its value.
    pub fn unresolved() -> (Self, OutputResolver<T>) {
        let (tx, rx) = oneshot::channel();
        let future = async move { rx.await.unwrap_or_else(|_| Err(resolver_dropped_error())) }
            .boxed()
            .shared();

        let resolver = OutputResolver {
            sender: Some(tx),
            secret: false,
            deps: Vec::new(),
        };
        (Output { future }, resolver)
    }

    /// Creates an output whose value is unknown (used during preview).
    ///
    /// The output resolves immediately but has no value: awaiting it returns
    /// an error, [`is_known`](Output::is_known) reports `false`, and
    /// combinators propagate the unknown without calling their callbacks.
    pub fn unknown() -> Self {
        Self::from_data(OutputData::unknown())
    }

    /// Creates a secret output with the given value.
    pub fn secret(value: T) -> Self {
        Self::from_data(OutputData {
            secret: true,
            ..OutputData::known(value)
        })
    }

    /// Waits for this output to resolve and returns its value, or an error
    /// if the output was rejected, the resolver was dropped, or the output
    /// has no value because it is unknown.
    ///
    /// This is equivalent to `.await` but works on `&self` without consuming
    /// the output.
    pub async fn get(&self) -> crate::error::Result<T> {
        into_value(self.future.clone().await)
    }

    /// Waits for this output to resolve and returns the URNs of the resources
    /// it depends on.
    ///
    /// Returns an empty list if the output was rejected.
    pub async fn dependencies(&self) -> Vec<String> {
        self.future
            .clone()
            .await
            .map(|data| data.deps)
            .unwrap_or_default()
    }

    /// Waits for this output to resolve and returns whether its value is known.
    ///
    /// Returns `false` if the output was rejected.
    pub async fn is_known(&self) -> bool {
        self.future.clone().await.is_ok_and(|data| data.known)
    }

    /// Waits for this output to resolve and returns whether it is a secret.
    ///
    /// Returns `false` if the output was rejected: a rejected output has no
    /// value to protect, and the error surfaces from [`get`](Output::get).
    pub async fn is_secret(&self) -> bool {
        self.future.clone().await.is_ok_and(|data| data.secret)
    }

    /// Transforms the output value by applying `f` to it once resolved.
    ///
    /// This is the Pulumi equivalent of `apply` — it lets you derive new values
    /// from resource outputs while preserving dependency tracking, secretness
    /// and knownness.
    ///
    /// If the source output was rejected with an error, the error propagates
    /// and `f` is never called. `f` is also not called if the source has no
    /// value (see [`Output::unknown`]); the result is then unknown too.
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
        let future = async move { Ok::<_, Arc<Error>>(source.await?.map(f)) }
            .boxed()
            .shared();
        Output { future }
    }

    /// Like [`map`](Output::map), but the function returns an `Output<U>`,
    /// which is then flattened.
    ///
    /// The result is secret if either this output or the one returned by `f`
    /// is secret, known only if both are known, and depends on the resources
    /// of both.
    ///
    /// If the source output was rejected with an error, the error propagates
    /// and `f` is never called. `f` is also not called if the source has no
    /// value (see [`Output::unknown`]); the result is then unknown too.
    pub fn flat_map<U, F>(&self, f: F) -> Output<U>
    where
        U: Clone + Send + Sync + 'static,
        F: FnOnce(T) -> Output<U> + Send + 'static,
    {
        let source = self.future.clone();
        let future = async move {
            let mut outer = source.await?;
            let inner = match outer.value.take() {
                Some(value) => f(value).future.await?,
                None => OutputData::unknown(),
            };
            Ok::<_, Arc<Error>>(outer.and_then(inner))
        }
        .boxed()
        .shared();
        Output { future }
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
        async move { into_value(self.future.await) }
    }
}

/// Resolves or rejects a pending [`Output`].
///
/// Dependencies and secretness set on the resolver take effect when the
/// output resolves, so every output derived from it sees them no matter
/// when the derived output was created.
///
/// If dropped without calling [`resolve`](OutputResolver::resolve) or
/// [`reject`](OutputResolver::reject), the output is automatically rejected
/// with an error indicating the resolver was dropped.
pub struct OutputResolver<T: Clone + Send + Sync + 'static> {
    sender: Option<oneshot::Sender<Resolved<T>>>,
    secret: bool,
    deps: Vec<String>,
}

impl<T: Clone + Send + Sync + 'static> OutputResolver<T> {
    fn send(&mut self, value: T, known: bool) {
        if let Some(tx) = self.sender.take() {
            let _ = tx.send(Ok(OutputData {
                value: Some(value),
                known,
                secret: self.secret,
                deps: std::mem::take(&mut self.deps),
            }));
        }
    }

    /// Resolves the output with the given value, waking all waiters.
    pub fn resolve(mut self, value: T) {
        self.send(value, true);
    }

    /// Rejects the output with an error, waking all waiters.
    pub fn reject(mut self, error: Error) {
        if let Some(tx) = self.sender.take() {
            let _ = tx.send(Err(Arc::new(error)));
        }
    }

    /// Marks the output as unknown and resolves it with a default value.
    pub fn resolve_unknown(mut self, default: T) {
        self.send(default, false);
    }

    /// Adds dependency URNs to the output.
    pub fn add_deps(&mut self, deps: impl IntoIterator<Item = String>) {
        self.deps = merge_deps(std::mem::take(&mut self.deps), deps);
    }

    /// Marks the output as containing a secret.
    pub fn set_secret(&mut self, secret: bool) {
        self.secret = secret;
    }
}

impl<T: Clone + Send + Sync + 'static> Drop for OutputResolver<T> {
    fn drop(&mut self) {
        if let Some(tx) = self.sender.take() {
            let _ = tx.send(Err(resolver_dropped_error()));
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

    let future = async move {
        let (ra, rb) = tokio::join!(fa, fb);
        Ok::<_, Arc<Error>>(ra?.zip(rb?))
    }
    .boxed()
    .shared();

    Output { future }
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

    let future = async move {
        let (ra, rb, rc) = tokio::join!(fa, fb, fc);
        Ok::<_, Arc<Error>>(ra?.zip(rb?).zip(rc?).map(|((a, b), c)| (a, b, c)))
    }
    .boxed()
    .shared();

    Output { future }
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
    let futures: Vec<_> = outputs.into_iter().map(|o| o.future).collect();

    let future = async move {
        let resolved = futures::future::try_join_all(futures).await?;
        let mut combined = OutputData::known(Vec::with_capacity(resolved.len()));
        for data in resolved {
            combined = combined.zip(data).map(|(mut values, value)| {
                values.push(value);
                values
            });
        }
        Ok::<_, Arc<Error>>(combined)
    }
    .boxed()
    .shared();

    Output { future }
}

/// Lifts an async function into an output.
pub fn from_future<T, F>(fut: F) -> Output<T>
where
    T: Clone + Send + Sync + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let future = async move { Ok(OutputData::known(fut.await)) }
        .boxed()
        .shared();

    Output { future }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resolved output starts known, non-secret, and its value is immediately available.
    #[tokio::test]
    async fn test_output_new() {
        let out = Output::new(42);
        assert_eq!(out.get().await.unwrap(), 42);
        assert!(out.is_known().await);
        assert!(!out.is_secret().await);
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

    /// `all3` combines three outputs of different types into a tuple.
    #[tokio::test]
    async fn test_all3() {
        let a = Output::new("hello".to_string());
        let b = Output::new(42);
        let c = Output::new(true);
        let combined = all3(&a, &b, &c);
        assert_eq!(
            combined.get().await.unwrap(),
            ("hello".to_string(), 42, true)
        );
    }

    /// A secret output reports `is_secret() == true` and its value is still accessible.
    #[tokio::test]
    async fn test_secret_output() {
        let out = Output::secret(42);
        assert!(out.is_secret().await);
        assert_eq!(out.get().await.unwrap(), 42);
    }

    /// `from_future` wraps an async computation in a non-secret, known output.
    #[tokio::test]
    async fn test_from_future() {
        let out = from_future(async { 42 });
        assert_eq!(out.get().await.unwrap(), 42);
        assert!(out.is_known().await);
        assert!(!out.is_secret().await);
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

    /// A rejected output reports not known, not secret, and no dependencies
    /// rather than hanging or panicking.
    #[tokio::test]
    async fn test_rejected_output_metadata() {
        let (out, mut resolver) = Output::<i32>::unresolved();
        resolver.set_secret(true);
        resolver.add_deps(["urn:a".to_string()]);
        resolver.reject(Error::Custom("nope".into()));
        assert!(!out.is_known().await);
        assert!(!out.is_secret().await);
        assert!(out.dependencies().await.is_empty());
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

    /// `map` keeps the source's secretness.
    #[tokio::test]
    async fn test_map_propagates_secret() {
        let secret = Output::secret(1);
        let mapped = secret.map(|v| v + 1);
        assert!(mapped.is_secret().await);
        assert_eq!(mapped.get().await.unwrap(), 2);
    }

    /// `all2` is secret when either input is secret.
    #[tokio::test]
    async fn test_all2_propagates_secret_from_first() {
        let secret = Output::secret(1);
        let plain = Output::new(2);
        let combined = all2(&secret, &plain);
        assert!(combined.is_secret().await);
    }

    /// `all2` is secret when the second input is secret.
    #[tokio::test]
    async fn test_all2_propagates_secret_from_second() {
        let plain = Output::new(1);
        let secret = Output::secret(2);
        let combined = all2(&plain, &secret);
        assert!(combined.is_secret().await);
    }

    /// `all2` is not secret when neither input is secret.
    #[tokio::test]
    async fn test_all2_no_false_positive_secret() {
        let a = Output::new(1);
        let b = Output::new(2);
        let combined = all2(&a, &b);
        assert!(!combined.is_secret().await);
    }

    /// `all3` is secret when any input is secret.
    #[tokio::test]
    async fn test_all3_propagates_secret_from_middle() {
        let a = Output::new(1);
        let b = Output::secret(2);
        let c = Output::new(3);
        let combined = all3(&a, &b, &c);
        assert!(combined.is_secret().await);
    }

    /// `all` (vec) is secret when any element is secret.
    #[tokio::test]
    async fn test_all_vec_propagates_secret() {
        let outputs = vec![Output::new(1), Output::secret(2), Output::new(3)];
        let combined = all(outputs);
        assert!(combined.is_secret().await);
    }

    /// `all` (vec) is not secret when no element is secret.
    #[tokio::test]
    async fn test_all_vec_no_false_positive_secret() {
        let outputs = vec![Output::new(1), Output::new(2)];
        let combined = all(outputs);
        assert!(!combined.is_secret().await);
    }

    /// `flat_map` propagates secret from the inner output.
    #[tokio::test]
    async fn test_flat_map_propagates_secret_from_inner() {
        let plain = Output::new(42);
        let result = plain.flat_map(|_| Output::secret(99));
        assert!(result.is_secret().await);
        assert_eq!(result.get().await.unwrap(), 99);
    }

    /// `flat_map` propagates secret from the inner output even when the
    /// source has not resolved yet at the time `flat_map` is called.
    #[tokio::test]
    async fn test_flat_map_propagates_secret_from_inner_when_pending() {
        let (plain, resolver) = Output::<i32>::unresolved();
        let result = plain.flat_map(|v| Output::secret(v + 1));
        resolver.resolve(1);
        assert!(result.is_secret().await);
        assert_eq!(result.get().await.unwrap(), 2);
    }

    /// `flat_map` propagates secret from the outer (source) output.
    #[tokio::test]
    async fn test_flat_map_propagates_secret_from_outer() {
        let secret = Output::secret(42);
        let result = secret.flat_map(|v| Output::new(v + 1));
        assert!(result.is_secret().await);
    }

    /// `flat_map` is not secret when neither outer nor inner is secret.
    #[tokio::test]
    async fn test_flat_map_no_false_positive_secret() {
        let plain = Output::new(1);
        let result = plain.flat_map(|v| Output::new(v * 2));
        assert!(!result.is_secret().await);
    }

    /// Marking the resolver secret after a derived output was created still
    /// makes the derived output secret.
    #[tokio::test]
    async fn test_resolver_secret_set_after_map_propagates() {
        let (out, mut resolver) = Output::<i32>::unresolved();
        let mapped = out.map(|v| v + 1);
        let combined = all2(&out, &Output::new(0));
        resolver.set_secret(true);
        resolver.resolve(1);
        assert!(out.is_secret().await);
        assert!(mapped.is_secret().await);
        assert!(combined.is_secret().await);
    }

    // --- Known/unknown propagation ---

    /// An unknown output has no value and reports `is_known() == false`.
    #[tokio::test]
    async fn test_unknown_output() {
        let out = Output::<i32>::unknown();
        assert!(!out.is_known().await);
        assert!(!out.is_secret().await);
        let err = out.get().await.unwrap_err();
        assert!(err.to_string().contains("output value is unknown"));
    }

    /// `map` on an unknown output stays unknown and never calls the callback.
    #[tokio::test]
    async fn test_map_propagates_unknown_without_calling_callback() {
        let unknown = Output::<i32>::unknown();
        let mapped = unknown.map(|_| -> i32 { panic!("callback must not run") });
        assert!(!mapped.is_known().await);
        assert!(mapped.get().await.is_err());
    }

    /// `all2` is unknown when either input is unknown.
    #[tokio::test]
    async fn test_all2_propagates_unknown() {
        let unknown = Output::<i32>::unknown();
        let known = Output::new(2);
        let combined = all2(&unknown, &known);
        assert!(!combined.is_known().await);
    }

    /// `all` (vec) is unknown when any element is unknown.
    #[tokio::test]
    async fn test_all_vec_propagates_unknown() {
        let combined = all(vec![Output::new(1), Output::unknown()]);
        assert!(!combined.is_known().await);
    }

    /// `flat_map` propagates unknown from the inner output.
    #[tokio::test]
    async fn test_flat_map_propagates_unknown_from_inner() {
        let known = Output::new(1);
        let result = known.flat_map(|_| Output::<i32>::unknown());
        assert!(!result.is_known().await);
    }

    /// `flat_map` on an unknown output stays unknown, never calls the
    /// callback, and keeps the outer output's secretness.
    #[tokio::test]
    async fn test_flat_map_propagates_unknown_from_outer() {
        let (outer, mut resolver) = Output::<i32>::unresolved();
        let unknown_secret = outer.flat_map(|_| Output::<i32>::unknown());
        let result =
            unknown_secret.flat_map(|_| -> Output<i32> { panic!("callback must not run") });
        resolver.set_secret(true);
        resolver.resolve(1);
        assert!(!result.is_known().await);
        assert!(result.is_secret().await);
    }

    /// `resolve_unknown` marks the output unknown for outputs derived before
    /// it resolved, while the default value still flows through.
    #[tokio::test]
    async fn test_resolve_unknown_propagates_to_derived() {
        let (out, resolver) = Output::<i32>::unresolved();
        let mapped = out.map(|v| v + 1);
        resolver.resolve_unknown(0);
        assert!(!out.is_known().await);
        assert!(!mapped.is_known().await);
        assert_eq!(mapped.get().await.unwrap(), 1);
    }

    // --- Dependency propagation ---

    fn with_deps(value: i32, deps: &[&str]) -> Output<i32> {
        let (out, mut resolver) = Output::unresolved();
        resolver.add_deps(deps.iter().map(|d| d.to_string()));
        resolver.resolve(value);
        out
    }

    /// Dependencies are available without awaiting the value first.
    #[tokio::test]
    async fn test_map_propagates_deps() {
        let mapped = with_deps(1, &["urn:a"]).map(|v| v + 1);
        assert_eq!(mapped.dependencies().await, vec!["urn:a"]);
    }

    /// `flat_map` unions the outer and inner dependencies.
    #[tokio::test]
    async fn test_flat_map_merges_deps() {
        let outer = with_deps(1, &["urn:a"]);
        let result = outer.flat_map(|v| with_deps(v, &["urn:b"]));
        assert_eq!(result.dependencies().await, vec!["urn:a", "urn:b"]);
    }

    /// `all` unions dependencies without duplicates, in first-seen order.
    #[tokio::test]
    async fn test_all_merges_and_dedups_deps() {
        let a = with_deps(1, &["urn:a", "urn:b"]);
        let b = with_deps(2, &["urn:b", "urn:c"]);
        let combined = all(vec![a.clone(), b, a]);
        assert_eq!(
            combined.dependencies().await,
            vec!["urn:a", "urn:b", "urn:c"]
        );
    }

    /// `all3` unions the dependencies of all three inputs.
    #[tokio::test]
    async fn test_all3_merges_deps() {
        let a = with_deps(1, &["urn:a"]);
        let b = with_deps(2, &["urn:b"]);
        let c = with_deps(3, &["urn:c"]);
        let combined = all3(&a, &b, &c);
        assert_eq!(
            combined.dependencies().await,
            vec!["urn:a", "urn:b", "urn:c"]
        );
    }
}
