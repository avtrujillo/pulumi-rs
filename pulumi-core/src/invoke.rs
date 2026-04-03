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
