use std::collections::HashMap;
use std::future::{Future, IntoFuture};

use ::serde::Serialize;
use ::serde::de::DeserializeOwned;

use crate::connection::{EngineConnection, GrpcEngine, GrpcMonitor, MonitorConnection};
use crate::context::Context;
use crate::error::{Error, Result};
use crate::proto::pulumirpc;
use crate::serde::{json_to_struct, struct_to_json};

// ---------------------------------------------------------------------------
// Type-driven invocation traits (leveraging impl_trait_in_assoc_type)
// ---------------------------------------------------------------------------

/// Describes a Pulumi provider function at the type level.
///
/// Implementors declare the function token, provider metadata, and
/// strongly-typed argument/return types. Invocation is handled
/// generically by [`InvokeBuilder`], which implements [`IntoFuture`]:
///
/// ```ignore
/// struct GetAmi;
///
/// impl ProviderFunction for GetAmi {
///     const TOKEN: &'static str = "aws:index/getAmi:getAmi";
///     type Args = GetAmiArgs;
///     type Returns = GetAmiResult;
/// }
///
/// let ami = InvokeBuilder::<GetAmi>::new(&ctx, GetAmiArgs { ... }).await?;
/// println!("AMI ID: {}", ami.id);
/// ```
pub trait ProviderFunction: Sized + Send + 'static {
    /// The function token (e.g. `"aws:index/getAmi:getAmi"`).
    const TOKEN: &'static str;

    /// The provider plugin version.
    const VERSION: &'static str = "";

    /// The plugin download URL.
    const PLUGIN_DOWNLOAD_URL: &'static str = "";

    /// The argument type. Must be serializable to JSON.
    type Args: Serialize + Send + 'static;

    /// The return type. Must be deserializable from JSON.
    type Returns: DeserializeOwned + Send + 'static;
}

/// Options for invoking a provider function.
#[derive(Debug, Clone, Default)]
pub struct InvokeOptions {
    /// An optional provider reference.
    pub provider: Option<String>,
    /// The version of the provider to use.
    pub version: String,
    /// The plugin download URL.
    pub plugin_download_url: String,
}

/// A builder for invoking a strongly-typed provider function.
///
/// Implements [`IntoFuture`] using `impl_trait_in_assoc_type`, so it can be
/// `.await`ed directly without boxing the future.
pub struct InvokeBuilder<
    F: ProviderFunction,
    M: MonitorConnection = GrpcMonitor,
    E: EngineConnection = GrpcEngine,
> {
    ctx: Context<M, E>,
    args: F::Args,
    opts: InvokeOptions,
    _marker: std::marker::PhantomData<F>,
}

impl<F: ProviderFunction, M: MonitorConnection, E: EngineConnection> InvokeBuilder<F, M, E> {
    /// Creates a new invoke builder for function `F`.
    pub fn new(ctx: &Context<M, E>, args: F::Args) -> Self {
        InvokeBuilder {
            ctx: ctx.clone(),
            args,
            opts: InvokeOptions {
                version: F::VERSION.to_string(),
                plugin_download_url: F::PLUGIN_DOWNLOAD_URL.to_string(),
                ..Default::default()
            },
            _marker: std::marker::PhantomData,
        }
    }

    /// Overrides all invoke options at once.
    pub fn options(mut self, opts: InvokeOptions) -> Self {
        self.opts = opts;
        self
    }

    /// Sets the explicit provider reference.
    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.opts.provider = Some(provider.into());
        self
    }
}

/// `InvokeBuilder<F, M, E>` can be `.await`ed directly.
impl<F: ProviderFunction, M: MonitorConnection, E: EngineConnection> IntoFuture
    for InvokeBuilder<F, M, E>
{
    type Output = Result<F::Returns>;
    type IntoFuture = impl Future<Output = Self::Output> + Send;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let args_json = serde_json::to_value(&self.args)?;
            let result = invoke_inner(&self.ctx, F::TOKEN, args_json, &self.opts).await?;
            Ok(serde_json::from_value(result)?)
        }
    }
}

/// Describes a method on a remote component resource at the type level.
///
/// Component methods are like [`ProviderFunction`]s but support dependency
/// tracking on both inputs and outputs.
///
/// ```ignore
/// struct MyComponentDoThing;
///
/// impl ComponentMethod for MyComponentDoThing {
///     const TOKEN: &'static str = "my:module:MyComponent/doThing";
///     type Args = DoThingArgs;
///     type Returns = DoThingResult;
/// }
///
/// let result = CallBuilder::<MyComponentDoThing>::new(&ctx, args).await?;
/// ```
pub trait ComponentMethod: Sized + Send + 'static {
    /// The method token (e.g. `"my:module:MyComponent/doThing"`).
    const TOKEN: &'static str;

    /// The provider plugin version.
    const VERSION: &'static str = "";

    /// The plugin download URL.
    const PLUGIN_DOWNLOAD_URL: &'static str = "";

    /// The argument type.
    type Args: Serialize + Send + 'static;

    /// The return type.
    type Returns: DeserializeOwned + Send + 'static;
}

/// The result of calling a component method, including return dependencies.
#[derive(Debug)]
pub struct CallResult<CM: ComponentMethod> {
    /// The typed return value.
    pub result: CM::Returns,
    /// Per-property dependency URNs on the return value.
    pub return_deps: HashMap<String, Vec<String>>,
}

/// A builder for calling a component method with dependency tracking.
///
/// Implements [`IntoFuture`] using `impl_trait_in_assoc_type`.
pub struct CallBuilder<
    CM: ComponentMethod,
    M: MonitorConnection = GrpcMonitor,
    E: EngineConnection = GrpcEngine,
> {
    ctx: Context<M, E>,
    args: CM::Args,
    arg_deps: HashMap<String, Vec<String>>,
    opts: InvokeOptions,
    _marker: std::marker::PhantomData<CM>,
}

impl<CM: ComponentMethod, M: MonitorConnection, E: EngineConnection> CallBuilder<CM, M, E> {
    /// Creates a new call builder for method `CM`.
    pub fn new(ctx: &Context<M, E>, args: CM::Args) -> Self {
        CallBuilder {
            ctx: ctx.clone(),
            args,
            arg_deps: HashMap::new(),
            opts: InvokeOptions {
                version: CM::VERSION.to_string(),
                plugin_download_url: CM::PLUGIN_DOWNLOAD_URL.to_string(),
                ..Default::default()
            },
            _marker: std::marker::PhantomData,
        }
    }

    /// Sets dependency URNs for a specific argument property.
    pub fn arg_dep(mut self, property: impl Into<String>, deps: Vec<String>) -> Self {
        self.arg_deps.insert(property.into(), deps);
        self
    }

    /// Sets all argument dependencies at once.
    pub fn arg_deps(mut self, deps: HashMap<String, Vec<String>>) -> Self {
        self.arg_deps = deps;
        self
    }

    /// Sets the explicit provider reference.
    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.opts.provider = Some(provider.into());
        self
    }
}

/// `CallBuilder<CM, M, E>` can be `.await`ed directly.
impl<CM: ComponentMethod, M: MonitorConnection, E: EngineConnection> IntoFuture
    for CallBuilder<CM, M, E>
{
    type Output = Result<CallResult<CM>>;
    type IntoFuture = impl Future<Output = Self::Output> + Send;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let (json_result, return_deps) =
                call_inner(&self.ctx, CM::TOKEN, self.args, self.arg_deps, &self.opts).await?;
            let result: CM::Returns = serde_json::from_value(json_result)?;
            Ok(CallResult {
                result,
                return_deps,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Internal gRPC functions (used by builders)
// ---------------------------------------------------------------------------

async fn invoke_inner<M: MonitorConnection, E: EngineConnection>(
    ctx: &Context<M, E>,
    token: &str,
    args: serde_json::Value,
    opts: &InvokeOptions,
) -> Result<serde_json::Value> {
    let args_struct = json_to_struct(&args);

    let req = pulumirpc::ResourceInvokeRequest {
        tok: token.to_string(),
        args: Some(args_struct),
        provider: opts.provider.clone().unwrap_or_default(),
        version: opts.version.clone(),
        accept_resources: true,
        plugin_download_url: opts.plugin_download_url.clone(),
        plugin_checksums: HashMap::new(),
        source_position: None,
        package_ref: String::new(),
        stack_trace: None,
        parent_stack_trace_handle: String::new(),
    };

    let resp = ctx.monitor().invoke(req).await?;

    // Check for failures.
    if !resp.failures.is_empty() {
        let failures: Vec<(String, String)> = resp
            .failures
            .iter()
            .map(|f| (f.property.clone(), f.reason.clone()))
            .collect();
        return Err(Error::InvokeFailure {
            token: token.to_string(),
            failures,
        });
    }

    let result = resp
        .r#return
        .as_ref()
        .map(struct_to_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    Ok(result)
}

async fn call_inner<A: Serialize + Send, M: MonitorConnection, E: EngineConnection>(
    ctx: &Context<M, E>,
    token: &str,
    args: A,
    arg_deps: HashMap<String, Vec<String>>,
    opts: &InvokeOptions,
) -> Result<(serde_json::Value, HashMap<String, Vec<String>>)> {
    let args_json = serde_json::to_value(&args)?;
    let args_struct = json_to_struct(&args_json);

    let arg_dependencies = arg_deps
        .into_iter()
        .map(|(k, urns)| {
            (
                k,
                pulumirpc::resource_call_request::ArgumentDependencies { urns },
            )
        })
        .collect();

    let req = pulumirpc::ResourceCallRequest {
        tok: token.to_string(),
        args: Some(args_struct),
        arg_dependencies,
        provider: opts.provider.clone().unwrap_or_default(),
        version: opts.version.clone(),
        plugin_download_url: opts.plugin_download_url.clone(),
        plugin_checksums: HashMap::new(),
        source_position: None,
        package_ref: String::new(),
        stack_trace: None,
        parent_stack_trace_handle: String::new(),
    };

    let resp = ctx.monitor().call(req).await?;

    if !resp.failures.is_empty() {
        let failures: Vec<(String, String)> = resp
            .failures
            .iter()
            .map(|f| (f.property.clone(), f.reason.clone()))
            .collect();
        return Err(Error::InvokeFailure {
            token: token.to_string(),
            failures,
        });
    }

    let result = resp
        .r#return
        .as_ref()
        .map(struct_to_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let return_deps = resp
        .return_dependencies
        .into_iter()
        .map(|(k, v)| (k, v.urns))
        .collect();

    Ok((result, return_deps))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::connection::{MockEngine, MonitorConnection};
    use crate::context::{Context, Settings};
    use crate::error::{Error, Result};
    use crate::proto::pulumirpc;
    use crate::test_support::TestContextBuilder;

    // --- Test fixture types ---

    struct TestFn;

    #[derive(Serialize)]
    struct TestFnArgs {
        input: String,
    }

    #[derive(Debug, Deserialize)]
    struct TestFnReturns {
        output: Option<String>,
    }

    impl ProviderFunction for TestFn {
        const TOKEN: &'static str = "test:fn:doThing";
        type Args = TestFnArgs;
        type Returns = TestFnReturns;
    }

    #[derive(Debug)]
    struct TestMethod;

    #[derive(Serialize)]
    struct TestMethodArgs {
        val: String,
    }

    #[derive(Debug, Deserialize)]
    struct TestMethodReturns {
        result: Option<String>,
    }

    impl ComponentMethod for TestMethod {
        const TOKEN: &'static str = "test:comp:MyComp/doThing";
        type Args = TestMethodArgs;
        type Returns = TestMethodReturns;
    }

    // --- Custom monitor that returns failures ---

    #[derive(Clone)]
    struct FailingMonitor;

    impl MonitorConnection for FailingMonitor {
        async fn register_resource(
            &self,
            req: pulumirpc::RegisterResourceRequest,
        ) -> Result<pulumirpc::RegisterResourceResponse> {
            Ok(pulumirpc::RegisterResourceResponse {
                urn: format!("urn::{}", req.name),
                id: String::new(),
                object: req.object,
                stable: true,
                stables: vec![],
                property_dependencies: Default::default(),
                result: 0,
            })
        }

        async fn register_resource_outputs(
            &self,
            _req: pulumirpc::RegisterResourceOutputsRequest,
        ) -> Result<()> {
            Ok(())
        }

        async fn read_resource(
            &self,
            req: pulumirpc::ReadResourceRequest,
        ) -> Result<pulumirpc::ReadResourceResponse> {
            Ok(pulumirpc::ReadResourceResponse {
                urn: String::new(),
                properties: req.properties,
            })
        }

        async fn invoke(
            &self,
            _req: pulumirpc::ResourceInvokeRequest,
        ) -> Result<pulumirpc::InvokeResponse> {
            Ok(pulumirpc::InvokeResponse {
                r#return: None,
                failures: vec![pulumirpc::CheckFailure {
                    property: "input".into(),
                    reason: "bad value".into(),
                }],
            })
        }

        async fn call(
            &self,
            _req: pulumirpc::ResourceCallRequest,
        ) -> Result<pulumirpc::CallResponse> {
            Ok(pulumirpc::CallResponse {
                r#return: None,
                failures: vec![pulumirpc::CheckFailure {
                    property: "val".into(),
                    reason: "invalid arg".into(),
                }],
                return_dependencies: Default::default(),
            })
        }

        async fn register_stack_transform(&self, _req: pulumirpc::Callback) -> Result<()> {
            Ok(())
        }

        async fn supports_feature(&self, _feature: &str) -> Result<bool> {
            Ok(true)
        }
    }

    fn failing_ctx() -> Context<FailingMonitor, MockEngine> {
        let settings = Settings {
            monitor_addr: String::new(),
            engine_addr: String::new(),
            project: "test".into(),
            stack: "dev".into(),
            dry_run: false,
            parallel: -1,
            organization: String::new(),
            config: HashMap::new(),
            config_secret_keys: Vec::new(),
        };
        Context::for_testing(FailingMonitor, MockEngine::new(), settings)
    }

    // --- InvokeBuilder ---

    #[tokio::test]
    async fn test_invoke_basic_returns_ok() {
        let tc = TestContextBuilder::new().build();
        // Fix F=TestFn; M and E are inferred from tc.context().
        let result = InvokeBuilder::<TestFn, _, _>::new(tc.context(), TestFnArgs { input: "x".into() })
            .await
            .unwrap();
        // MockMonitor returns empty struct → output deserializes as None
        assert!(result.output.is_none());
    }

    #[tokio::test]
    async fn test_invoke_with_provider_option() {
        let tc = TestContextBuilder::new().build();
        let result = InvokeBuilder::<TestFn, _, _>::new(tc.context(), TestFnArgs { input: "x".into() })
            .provider("urn:provider::explicit")
            .await
            .unwrap();
        assert!(result.output.is_none());
    }

    #[tokio::test]
    async fn test_invoke_failure_returns_error() {
        let ctx = failing_ctx();
        let err = InvokeBuilder::<TestFn, FailingMonitor, MockEngine>::new(
            &ctx,
            TestFnArgs { input: "bad".into() },
        )
        .await
        .unwrap_err();
        let Error::InvokeFailure { token, failures } = err else {
            panic!("expected InvokeFailure, got {err:?}");
        };
        assert_eq!(token, "test:fn:doThing");
        assert_eq!(failures[0], ("input".into(), "bad value".into()));
    }

    // --- CallBuilder ---

    #[tokio::test]
    async fn test_call_basic_returns_ok() {
        let tc = TestContextBuilder::new().build();
        let result = CallBuilder::<TestMethod, _, _>::new(tc.context(), TestMethodArgs { val: "v".into() })
            .await
            .unwrap();
        assert!(result.result.result.is_none());
        assert!(result.return_deps.is_empty());
    }

    #[tokio::test]
    async fn test_call_with_arg_deps() {
        let tc = TestContextBuilder::new().build();
        let result = CallBuilder::<TestMethod, _, _>::new(tc.context(), TestMethodArgs { val: "v".into() })
            .arg_dep("val", vec!["urn:dep".into()])
            .await
            .unwrap();
        // MockMonitor ignores arg_deps but call still succeeds
        assert!(result.result.result.is_none());
    }

    #[tokio::test]
    async fn test_call_failure_returns_error() {
        let ctx = failing_ctx();
        let err = CallBuilder::<TestMethod, FailingMonitor, MockEngine>::new(
            &ctx,
            TestMethodArgs { val: "bad".into() },
        )
        .await
        .unwrap_err();
        let Error::InvokeFailure { token, failures } = err else {
            panic!("expected InvokeFailure, got {err:?}");
        };
        assert_eq!(token, "test:comp:MyComp/doThing");
        assert_eq!(failures[0].0, "val");
    }
}
