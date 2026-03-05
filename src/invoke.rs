use std::collections::HashMap;
use std::future::{Future, IntoFuture};

use ::serde::de::DeserializeOwned;
use ::serde::Serialize;

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
pub struct InvokeBuilder<F: ProviderFunction> {
    ctx: Context,
    args: F::Args,
    opts: InvokeOptions,
    _marker: std::marker::PhantomData<F>,
}

impl<F: ProviderFunction> InvokeBuilder<F> {
    /// Creates a new invoke builder for function `F`.
    pub fn new(ctx: &Context, args: F::Args) -> Self {
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

/// `InvokeBuilder<F>` can be `.await`ed directly.
impl<F: ProviderFunction> IntoFuture for InvokeBuilder<F> {
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
pub struct CallResult<M: ComponentMethod> {
    /// The typed return value.
    pub result: M::Returns,
    /// Per-property dependency URNs on the return value.
    pub return_deps: HashMap<String, Vec<String>>,
}

/// A builder for calling a component method with dependency tracking.
///
/// Implements [`IntoFuture`] using `impl_trait_in_assoc_type`.
pub struct CallBuilder<M: ComponentMethod> {
    ctx: Context,
    args: M::Args,
    arg_deps: HashMap<String, Vec<String>>,
    opts: InvokeOptions,
    _marker: std::marker::PhantomData<M>,
}

impl<M: ComponentMethod> CallBuilder<M> {
    /// Creates a new call builder for method `M`.
    pub fn new(ctx: &Context, args: M::Args) -> Self {
        CallBuilder {
            ctx: ctx.clone(),
            args,
            arg_deps: HashMap::new(),
            opts: InvokeOptions {
                version: M::VERSION.to_string(),
                plugin_download_url: M::PLUGIN_DOWNLOAD_URL.to_string(),
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

/// `CallBuilder<M>` can be `.await`ed directly.
impl<M: ComponentMethod> IntoFuture for CallBuilder<M> {
    type Output = Result<CallResult<M>>;
    type IntoFuture = impl Future<Output = Self::Output> + Send;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let (json_result, return_deps) =
                call_inner(&self.ctx, M::TOKEN, self.args, self.arg_deps, &self.opts).await?;
            let result: M::Returns = serde_json::from_value(json_result)?;
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

async fn invoke_inner(
    ctx: &Context,
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

    let mut monitor = ctx.monitor().await;
    let resp = monitor.invoke(req).await?;
    let inner = resp.into_inner();

    // Check for failures.
    if !inner.failures.is_empty() {
        let failures: Vec<(String, String)> = inner
            .failures
            .iter()
            .map(|f| (f.property.clone(), f.reason.clone()))
            .collect();
        return Err(Error::InvokeFailure {
            token: token.to_string(),
            failures,
        });
    }

    let result = inner
        .r#return
        .as_ref()
        .map(struct_to_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    Ok(result)
}

async fn call_inner<A: Serialize + Send>(
    ctx: &Context,
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

    let mut monitor = ctx.monitor().await;
    let resp = monitor.call(req).await?;
    let inner = resp.into_inner();

    if !inner.failures.is_empty() {
        let failures: Vec<(String, String)> = inner
            .failures
            .iter()
            .map(|f| (f.property.clone(), f.reason.clone()))
            .collect();
        return Err(Error::InvokeFailure {
            token: token.to_string(),
            failures,
        });
    }

    let result = inner
        .r#return
        .as_ref()
        .map(struct_to_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let return_deps = inner
        .return_dependencies
        .into_iter()
        .map(|(k, v)| (k, v.urns))
        .collect();

    Ok((result, return_deps))
}
