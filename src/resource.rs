use std::collections::HashMap;
use std::future::{Future, IntoFuture};

use ::serde::de::DeserializeOwned;
use ::serde::Serialize;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::Output;
use crate::proto::pulumirpc;
use crate::serde::{json_to_struct, struct_to_json};

/// Options for registering a resource.
#[derive(Debug, Clone, Default)]
pub struct ResourceOptions {
    /// An optional parent resource URN.
    pub parent: Option<String>,
    /// An optional explicit provider reference.
    pub provider: Option<String>,
    /// URNs of resources this resource depends on.
    pub depends_on: Vec<String>,
    /// Whether to protect this resource from deletion.
    pub protect: bool,
    /// Properties to ignore during updates.
    pub ignore_changes: Vec<String>,
    /// The version of the provider to use.
    pub version: String,
    /// The download URL for the provider plugin.
    pub plugin_download_url: String,
    /// Properties that should be treated as secrets.
    pub additional_secret_outputs: Vec<String>,
    /// Whether to delete before replacing.
    pub delete_before_replace: bool,
    /// Properties that trigger replacement when changed.
    pub replace_on_changes: Vec<String>,
    /// Whether to retain the resource on delete.
    pub retain_on_delete: bool,
    /// An existing resource ID to import.
    pub import_id: Option<String>,
}

/// The result of registering a resource.
#[derive(Debug, Clone)]
pub struct ResourceResult {
    /// The URN assigned by the engine.
    pub urn: String,
    /// The provider-assigned ID (empty for component resources).
    pub id: String,
    /// The resolved output properties as JSON.
    pub outputs: serde_json::Value,
    /// Per-property dependency information.
    pub property_deps: HashMap<String, Vec<String>>,
}

// ---------------------------------------------------------------------------
// Type-driven resource traits (leveraging impl_trait_in_assoc_type)
// ---------------------------------------------------------------------------

/// Describes a Pulumi custom resource at the type level.
///
/// Implementors declare the resource's type token, provider metadata, and
/// strongly-typed input/output property types. Registration is handled
/// generically by [`ResourceBuilder`], which implements [`IntoFuture`] —
/// so creating a resource is as simple as:
///
/// ```ignore
/// let bucket = ResourceBuilder::<S3Bucket>::new(&ctx, "my-bucket", S3BucketArgs {
///     bucket: "my-unique-name".into(),
/// }).await?;
///
/// println!("ARN: {}", bucket.outputs.arn);
/// ```
pub trait Resource: Sized + Send + 'static {
    /// The Pulumi type token (e.g. `"aws:s3/bucket:Bucket"`).
    const TYPE_TOKEN: &'static str;

    /// The provider plugin version to use when registering this resource.
    const VERSION: &'static str = "";

    /// The download URL for the provider plugin.
    const PLUGIN_DOWNLOAD_URL: &'static str = "";

    /// The input properties type. Must be serializable to JSON so that
    /// the SDK can send it over gRPC.
    type Inputs: Serialize + Send + 'static;

    /// The output properties type returned by the provider after the
    /// resource has been created or updated.
    type Outputs: DeserializeOwned + Clone + Send + Sync + 'static;
}

/// A registered resource with typed output properties.
///
/// This is what you get back when you `.await` a [`ResourceBuilder`].
#[derive(Debug, Clone)]
pub struct RegisteredResource<R: Resource> {
    /// The URN assigned by the Pulumi engine.
    pub urn: String,
    /// The provider-assigned ID (empty during preview).
    pub id: String,
    /// The typed output properties deserialized from the provider response.
    pub outputs: R::Outputs,
    /// Per-property dependency URNs.
    pub property_deps: HashMap<String, Vec<String>>,
}

/// A builder for registering a strongly-typed resource.
///
/// Implements [`IntoFuture`] using `impl_trait_in_assoc_type`, so it can be
/// `.await`ed directly without boxing the future. The builder captures all
/// registration parameters and performs the gRPC call when awaited.
///
/// # Example
///
/// ```ignore
/// let bucket = ResourceBuilder::<S3Bucket>::new(&ctx, "my-bucket", args)
///     .parent(parent_urn)
///     .protect()
///     .await?;
/// ```
pub struct ResourceBuilder<R: Resource> {
    ctx: Context,
    name: String,
    inputs: R::Inputs,
    opts: ResourceOptions,
    _marker: std::marker::PhantomData<R>,
}

impl<R: Resource> ResourceBuilder<R> {
    /// Creates a new builder for a resource of type `R`.
    pub fn new(ctx: &Context, name: impl Into<String>, inputs: R::Inputs) -> Self {
        ResourceBuilder {
            ctx: ctx.clone(),
            name: name.into(),
            inputs,
            opts: ResourceOptions {
                version: R::VERSION.to_string(),
                plugin_download_url: R::PLUGIN_DOWNLOAD_URL.to_string(),
                ..Default::default()
            },
            _marker: std::marker::PhantomData,
        }
    }

    /// Overrides all resource options at once.
    pub fn options(mut self, opts: ResourceOptions) -> Self {
        self.opts = opts;
        self
    }

    /// Sets the parent resource URN.
    pub fn parent(mut self, urn: impl Into<String>) -> Self {
        self.opts.parent = Some(urn.into());
        self
    }

    /// Sets the explicit provider reference.
    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.opts.provider = Some(provider.into());
        self
    }

    /// Adds a dependency on another resource.
    pub fn depends_on(mut self, urn: impl Into<String>) -> Self {
        self.opts.depends_on.push(urn.into());
        self
    }

    /// Marks the resource as protected from deletion.
    pub fn protect(mut self) -> Self {
        self.opts.protect = true;
        self
    }

    /// Marks properties that should be treated as secrets.
    pub fn additional_secret_outputs(mut self, props: Vec<String>) -> Self {
        self.opts.additional_secret_outputs = props;
        self
    }

    /// Enables delete-before-replace behavior.
    pub fn delete_before_replace(mut self) -> Self {
        self.opts.delete_before_replace = true;
        self
    }

    /// Sets an existing cloud resource ID to import.
    pub fn import_id(mut self, id: impl Into<String>) -> Self {
        self.opts.import_id = Some(id.into());
        self
    }

    /// Sets properties to ignore during updates.
    pub fn ignore_changes(mut self, props: Vec<String>) -> Self {
        self.opts.ignore_changes = props;
        self
    }

    /// Sets properties that trigger replacement when changed.
    pub fn replace_on_changes(mut self, props: Vec<String>) -> Self {
        self.opts.replace_on_changes = props;
        self
    }

    /// Retains the resource in the cloud when it is deleted from the program.
    pub fn retain_on_delete(mut self) -> Self {
        self.opts.retain_on_delete = true;
        self
    }
}

/// `ResourceBuilder<R>` can be `.await`ed directly. The future type is an
/// opaque `impl Future` — no heap allocation for the future itself.
impl<R: Resource> IntoFuture for ResourceBuilder<R> {
    type Output = Result<RegisteredResource<R>>;
    type IntoFuture = impl Future<Output = Self::Output> + Send;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let inputs_json = serde_json::to_value(&self.inputs)?;
            let result = register_resource_inner(
                &self.ctx,
                R::TYPE_TOKEN,
                &self.name,
                inputs_json,
                &self.opts,
                true,
                false,
            )
            .await?;
            let outputs: R::Outputs = serde_json::from_value(result.outputs)?;
            Ok(RegisteredResource {
                urn: result.urn,
                id: result.id,
                outputs,
                property_deps: result.property_deps,
            })
        }
    }
}

/// Describes a Pulumi component resource at the type level.
///
/// Component resources are logical groupings — they don't correspond to a
/// physical cloud resource but serve as parents for other resources.
///
/// Use [`ComponentBuilder`] to register a component and get back a
/// [`RegisteredComponent`].
pub trait ComponentResource: Sized + Send + 'static {
    /// The Pulumi type token (e.g. `"my:module:MyComponent"`).
    const TYPE_TOKEN: &'static str;
}

/// A registered component resource.
#[derive(Debug, Clone)]
pub struct RegisteredComponent<C: ComponentResource> {
    /// The URN assigned by the Pulumi engine.
    pub urn: String,
    _marker: std::marker::PhantomData<C>,
}

impl<C: ComponentResource> RegisteredComponent<C> {
    /// Returns the URN of this component.
    pub fn urn(&self) -> &str {
        &self.urn
    }

    /// Registers the final outputs for this component resource.
    pub async fn register_outputs(
        &self,
        ctx: &Context,
        outputs: serde_json::Value,
    ) -> Result<()> {
        register_resource_outputs(ctx, &self.urn, outputs).await
    }
}

/// A builder for registering a component resource.
pub struct ComponentBuilder<C: ComponentResource> {
    ctx: Context,
    name: String,
    opts: ResourceOptions,
    _marker: std::marker::PhantomData<C>,
}

impl<C: ComponentResource> ComponentBuilder<C> {
    /// Creates a new component builder.
    pub fn new(ctx: &Context, name: impl Into<String>) -> Self {
        ComponentBuilder {
            ctx: ctx.clone(),
            name: name.into(),
            opts: ResourceOptions::default(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Overrides all resource options at once.
    pub fn options(mut self, opts: ResourceOptions) -> Self {
        self.opts = opts;
        self
    }

    /// Sets the parent resource URN.
    pub fn parent(mut self, urn: impl Into<String>) -> Self {
        self.opts.parent = Some(urn.into());
        self
    }

    /// Marks the component as protected from deletion.
    pub fn protect(mut self) -> Self {
        self.opts.protect = true;
        self
    }
}

/// `ComponentBuilder<C>` can be `.await`ed directly.
impl<C: ComponentResource> IntoFuture for ComponentBuilder<C> {
    type Output = Result<RegisteredComponent<C>>;
    type IntoFuture = impl Future<Output = Self::Output> + Send;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let result = register_resource_inner(
                &self.ctx,
                C::TYPE_TOKEN,
                &self.name,
                serde_json::Value::Object(Default::default()),
                &self.opts,
                false,
                false,
            )
            .await?;
            Ok(RegisteredComponent {
                urn: result.urn,
                _marker: std::marker::PhantomData,
            })
        }
    }
}

/// Describes a remote component resource at the type level.
///
/// Remote components are implemented by a provider plugin. They accept
/// typed inputs and produce typed outputs, like [`Resource`], but are
/// registered as non-custom remote resources.
pub trait RemoteComponent: Sized + Send + 'static {
    /// The Pulumi type token.
    const TYPE_TOKEN: &'static str;

    /// The provider plugin version.
    const VERSION: &'static str = "";

    /// The plugin download URL.
    const PLUGIN_DOWNLOAD_URL: &'static str = "";

    /// The input properties type.
    type Inputs: Serialize + Send + 'static;

    /// The output properties type.
    type Outputs: DeserializeOwned + Clone + Send + Sync + 'static;
}

/// A registered remote component with typed outputs.
#[derive(Debug, Clone)]
pub struct RegisteredRemoteComponent<R: RemoteComponent> {
    /// The URN assigned by the Pulumi engine.
    pub urn: String,
    /// The typed output properties.
    pub outputs: R::Outputs,
    /// Per-property dependency URNs.
    pub property_deps: HashMap<String, Vec<String>>,
}

/// A builder for registering a remote component resource.
pub struct RemoteComponentBuilder<R: RemoteComponent> {
    ctx: Context,
    name: String,
    inputs: R::Inputs,
    opts: ResourceOptions,
    _marker: std::marker::PhantomData<R>,
}

impl<R: RemoteComponent> RemoteComponentBuilder<R> {
    /// Creates a new remote component builder.
    pub fn new(ctx: &Context, name: impl Into<String>, inputs: R::Inputs) -> Self {
        RemoteComponentBuilder {
            ctx: ctx.clone(),
            name: name.into(),
            inputs,
            opts: ResourceOptions {
                version: R::VERSION.to_string(),
                plugin_download_url: R::PLUGIN_DOWNLOAD_URL.to_string(),
                ..Default::default()
            },
            _marker: std::marker::PhantomData,
        }
    }

    /// Overrides all resource options at once.
    pub fn options(mut self, opts: ResourceOptions) -> Self {
        self.opts = opts;
        self
    }

    /// Sets the parent resource URN.
    pub fn parent(mut self, urn: impl Into<String>) -> Self {
        self.opts.parent = Some(urn.into());
        self
    }

    /// Sets the explicit provider reference.
    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.opts.provider = Some(provider.into());
        self
    }

    /// Adds a dependency on another resource.
    pub fn depends_on(mut self, urn: impl Into<String>) -> Self {
        self.opts.depends_on.push(urn.into());
        self
    }
}

/// `RemoteComponentBuilder<R>` can be `.await`ed directly.
impl<R: RemoteComponent> IntoFuture for RemoteComponentBuilder<R> {
    type Output = Result<RegisteredRemoteComponent<R>>;
    type IntoFuture = impl Future<Output = Self::Output> + Send;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let inputs_json = serde_json::to_value(&self.inputs)?;
            let result = register_resource_inner(
                &self.ctx,
                R::TYPE_TOKEN,
                &self.name,
                inputs_json,
                &self.opts,
                false,
                true,
            )
            .await?;
            let outputs: R::Outputs = serde_json::from_value(result.outputs)?;
            Ok(RegisteredRemoteComponent {
                urn: result.urn,
                outputs,
                property_deps: result.property_deps,
            })
        }
    }
}

/// Registers a custom resource with the Pulumi engine.
///
/// Custom resources are managed by a cloud provider plugin (e.g. AWS, GCP).
///
/// # Arguments
///
/// * `ctx` - The Pulumi context.
/// * `resource_type` - The type token (e.g. `"aws:s3/bucket:Bucket"`).
/// * `name` - The logical name of the resource.
/// * `inputs` - The input properties as a JSON value.
/// * `opts` - Resource options.
///
/// # Returns
///
/// A [`ResourceResult`] containing the URN, ID, and output properties.
pub async fn register_resource(
    ctx: &Context,
    resource_type: &str,
    name: &str,
    inputs: serde_json::Value,
    opts: &ResourceOptions,
) -> Result<ResourceResult> {
    register_resource_inner(ctx, resource_type, name, inputs, opts, true, false).await
}

/// Registers a component resource with the Pulumi engine.
///
/// Component resources are logical groupings that don't directly correspond to
/// a cloud resource. They serve as parents for other resources.
pub async fn register_component_resource(
    ctx: &Context,
    resource_type: &str,
    name: &str,
    opts: &ResourceOptions,
) -> Result<ResourceResult> {
    register_resource_inner(
        ctx,
        resource_type,
        name,
        serde_json::Value::Object(Default::default()),
        opts,
        false,
        false,
    )
    .await
}

/// Registers a remote component resource.
///
/// Remote components are implemented by a provider plugin rather than in the
/// current program.
pub async fn register_remote_component(
    ctx: &Context,
    resource_type: &str,
    name: &str,
    inputs: serde_json::Value,
    opts: &ResourceOptions,
) -> Result<ResourceResult> {
    register_resource_inner(ctx, resource_type, name, inputs, opts, false, true).await
}

async fn register_resource_inner(
    ctx: &Context,
    resource_type: &str,
    name: &str,
    inputs: serde_json::Value,
    opts: &ResourceOptions,
    custom: bool,
    remote: bool,
) -> Result<ResourceResult> {
    let object = json_to_struct(&inputs);

    let parent = opts
        .parent
        .clone()
        .or_else(|| {
            // If no parent is specified, we won't set one here;
            // the engine will use the root stack.
            None
        })
        .unwrap_or_default();

    let req = pulumirpc::RegisterResourceRequest {
        r#type: resource_type.to_string(),
        name: name.to_string(),
        parent,
        custom,
        object: Some(object),
        protect: Some(opts.protect),
        dependencies: opts.depends_on.clone(),
        provider: opts.provider.clone().unwrap_or_default(),
        property_dependencies: HashMap::new(),
        delete_before_replace: opts.delete_before_replace,
        version: opts.version.clone(),
        ignore_changes: opts.ignore_changes.clone(),
        accept_secrets: true,
        additional_secret_outputs: opts.additional_secret_outputs.clone(),
        alias_ur_ns: Vec::new(),
        import_id: opts.import_id.clone().unwrap_or_default(),
        custom_timeouts: None,
        delete_before_replace_defined: opts.delete_before_replace,
        supports_partial_values: true,
        remote,
        accept_resources: true,
        providers: HashMap::new(),
        replace_on_changes: opts.replace_on_changes.clone(),
        plugin_download_url: opts.plugin_download_url.clone(),
        plugin_checksums: HashMap::new(),
        retain_on_delete: Some(opts.retain_on_delete),
        aliases: Vec::new(),
        deleted_with: String::new(),
        alias_specs: true,
        source_position: None,
        transforms: Vec::new(),
        supports_result_reporting: true,
        package_ref: String::new(),
        hooks: None,
        hide_diffs: Vec::new(),
        stack_trace: None,
        parent_stack_trace_handle: String::new(),
        replace_with: Vec::new(),
        replacement_trigger: None,
        env_var_mappings: HashMap::new(),
    };

    let mut monitor = ctx.monitor().await;
    let resp = monitor.register_resource(req).await?;
    let inner = resp.into_inner();

    // Check if the registration was reported as failed.
    if inner.result == pulumirpc::Result::Fail as i32 {
        return Err(Error::ResourceFailed {
            urn: inner.urn.clone(),
        });
    }

    let outputs = inner
        .object
        .as_ref()
        .map(struct_to_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let property_deps = inner
        .property_dependencies
        .iter()
        .map(|(k, v)| (k.clone(), v.urns.clone()))
        .collect();

    Ok(ResourceResult {
        urn: inner.urn,
        id: inner.id,
        outputs,
        property_deps,
    })
}

/// Registers the outputs of a component resource.
///
/// This should be called after all child resources of a component have been
/// registered, to signal that the component's outputs are ready.
pub async fn register_resource_outputs(
    ctx: &Context,
    urn: &str,
    outputs: serde_json::Value,
) -> Result<()> {
    let outputs_struct = json_to_struct(&outputs);

    let req = pulumirpc::RegisterResourceOutputsRequest {
        urn: urn.to_string(),
        outputs: Some(outputs_struct),
    };

    let mut monitor = ctx.monitor().await;
    monitor.register_resource_outputs(req).await?;

    Ok(())
}

/// Reads an existing resource from the cloud provider.
///
/// This is used to import existing cloud resources into Pulumi's state.
pub async fn read_resource(
    ctx: &Context,
    resource_type: &str,
    name: &str,
    id: &str,
    props: serde_json::Value,
    opts: &ResourceOptions,
) -> Result<ResourceResult> {
    let properties = json_to_struct(&props);

    let parent = opts.parent.clone().unwrap_or_default();

    let req = pulumirpc::ReadResourceRequest {
        id: id.to_string(),
        r#type: resource_type.to_string(),
        name: name.to_string(),
        parent,
        properties: Some(properties),
        dependencies: opts.depends_on.clone(),
        provider: opts.provider.clone().unwrap_or_default(),
        version: opts.version.clone(),
        accept_secrets: true,
        additional_secret_outputs: opts.additional_secret_outputs.clone(),
        accept_resources: true,
        plugin_download_url: opts.plugin_download_url.clone(),
        plugin_checksums: HashMap::new(),
        source_position: None,
        package_ref: String::new(),
        stack_trace: None,
        parent_stack_trace_handle: String::new(),
    };

    let mut monitor = ctx.monitor().await;
    let resp = monitor.read_resource(req).await?;
    let inner = resp.into_inner();

    let outputs = inner
        .properties
        .as_ref()
        .map(struct_to_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    Ok(ResourceResult {
        urn: inner.urn,
        id: id.to_string(),
        outputs,
        property_deps: HashMap::new(),
    })
}

/// A builder for custom resources that wraps outputs ergonomically.
///
/// This provides a higher-level API where inputs can be `Output<T>` values,
/// and results are returned as `Output<T>` as well.
pub struct CustomResource {
    resource_type: String,
    name: String,
    inputs: serde_json::Value,
    opts: ResourceOptions,
}

impl CustomResource {
    /// Creates a new custom resource builder.
    pub fn new(resource_type: &str, name: &str) -> Self {
        CustomResource {
            resource_type: resource_type.to_string(),
            name: name.to_string(),
            inputs: serde_json::Value::Object(Default::default()),
            opts: ResourceOptions::default(),
        }
    }

    /// Sets the input properties.
    pub fn inputs(mut self, inputs: serde_json::Value) -> Self {
        self.inputs = inputs;
        self
    }

    /// Sets resource options.
    pub fn options(mut self, opts: ResourceOptions) -> Self {
        self.opts = opts;
        self
    }

    /// Sets the parent resource.
    pub fn parent(mut self, urn: impl Into<String>) -> Self {
        self.opts.parent = Some(urn.into());
        self
    }

    /// Sets the provider.
    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.opts.provider = Some(provider.into());
        self
    }

    /// Adds a dependency.
    pub fn depends_on(mut self, urn: impl Into<String>) -> Self {
        self.opts.depends_on.push(urn.into());
        self
    }

    /// Registers the resource and returns outputs.
    ///
    /// Returns `(urn, id, outputs)` as `Output` values.
    pub async fn register(
        self,
        ctx: &Context,
    ) -> Result<(Output<String>, Output<String>, Output<serde_json::Value>)> {
        let result = register_resource(ctx, &self.resource_type, &self.name, self.inputs, &self.opts).await?;

        let urn = Output::new(result.urn);
        let id = Output::new(result.id);
        let outputs = Output::new(result.outputs);

        Ok((urn, id, outputs))
    }
}

/// Helper to extract a string property from a JSON output.
pub fn get_output_string(
    outputs: &Output<serde_json::Value>,
    key: &str,
) -> Output<Option<String>> {
    let key = key.to_string();
    outputs.map(move |v: serde_json::Value| {
        v.as_object()
            .and_then(|m| m.get(&key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })
}
