use std::collections::HashMap;
use std::future::{Future, IntoFuture};

use ::serde::Serialize;
use ::serde::de::DeserializeOwned;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::proto::pulumirpc;
use crate::serde::{json_to_struct, struct_to_json};

/// An alias for a resource, used to migrate resources without replacement.
///
/// Aliases tell the Pulumi engine that a resource may have previously existed
/// under a different URN. This allows renaming, re-parenting, or re-typing
/// resources without triggering a delete+create.
#[derive(Debug, Clone)]
pub enum Alias {
    /// A fully-specified previous URN.
    Urn(String),
    /// A structured alias specification that can override individual components
    /// of the URN (name, type, stack, project, parent).
    Spec(AliasSpec),
}

/// A structured alias specification.
///
/// Each field overrides the corresponding component of the resource's URN.
/// Fields left as `None` default to the current resource's values.
#[derive(Debug, Clone, Default)]
pub struct AliasSpec {
    /// The previous name of the resource.
    pub name: Option<String>,
    /// The previous type of the resource.
    pub r#type: Option<String>,
    /// The previous stack.
    pub stack: Option<String>,
    /// The previous project.
    pub project: Option<String>,
    /// The previous parent.
    pub parent: Option<AliasParent>,
}

/// Specifies the previous parent of a resource in an alias.
#[derive(Debug, Clone)]
pub enum AliasParent {
    /// The URN of the previous parent.
    Urn(String),
    /// The resource previously had no parent (was a root-level resource).
    NoParent,
}

/// Custom timeout values for resource CRUD operations.
///
/// Each timeout is a duration string (e.g. `"5m"`, `"1h"`, `"30s"`).
#[derive(Debug, Clone, Default)]
pub struct CustomTimeouts {
    /// Timeout for create operations.
    pub create: Option<String>,
    /// Timeout for update operations.
    pub update: Option<String>,
    /// Timeout for delete operations.
    pub delete: Option<String>,
}

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
    /// Aliases for this resource, allowing renames and re-parenting
    /// without replacement.
    pub aliases: Vec<Alias>,
    /// Custom timeouts for create, update, and delete operations.
    pub custom_timeouts: Option<CustomTimeouts>,
    /// If set, this resource will be deleted when the specified resource
    /// URN is deleted, without requiring an explicit delete call.
    pub deleted_with: Option<String>,
}

/// Internal result of registering a resource (untyped JSON).
#[derive(Debug, Clone)]
pub(crate) struct ResourceResult {
    pub urn: String,
    pub id: String,
    pub outputs: serde_json::Value,
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
/// struct S3Bucket;
///
/// impl Resource for S3Bucket {
///     const TYPE_TOKEN: &'static str = "aws:s3/bucket:Bucket";
///     type Inputs = S3BucketArgs;
///     type Outputs = S3BucketOutputs;
/// }
///
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
/// This is what you get back when you `.await` a [`ResourceBuilder`] or
/// [`ReadBuilder`].
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

    /// Adds an alias for this resource.
    pub fn alias(mut self, alias: Alias) -> Self {
        self.opts.aliases.push(alias);
        self
    }

    /// Adds a URN alias for this resource.
    pub fn alias_urn(mut self, urn: impl Into<String>) -> Self {
        self.opts.aliases.push(Alias::Urn(urn.into()));
        self
    }

    /// Sets custom timeouts for create, update, and delete operations.
    pub fn custom_timeouts(mut self, timeouts: CustomTimeouts) -> Self {
        self.opts.custom_timeouts = Some(timeouts);
        self
    }

    /// Sets the resource that, when deleted, also deletes this resource.
    pub fn deleted_with(mut self, urn: impl Into<String>) -> Self {
        self.opts.deleted_with = Some(urn.into());
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

/// A builder for reading an existing cloud resource into Pulumi state.
///
/// Like [`ResourceBuilder`], implements [`IntoFuture`] with an unboxed future.
///
/// # Example
///
/// ```ignore
/// let existing = ReadBuilder::<S3Bucket>::new(&ctx, "imported-bucket", "bucket-id-123")
///     .await?;
/// ```
pub struct ReadBuilder<R: Resource> {
    ctx: Context,
    name: String,
    id: String,
    props: serde_json::Value,
    opts: ResourceOptions,
    _marker: std::marker::PhantomData<R>,
}

impl<R: Resource> ReadBuilder<R> {
    /// Creates a new read builder for a resource of type `R`.
    ///
    /// `id` is the existing cloud provider ID of the resource to import.
    pub fn new(ctx: &Context, name: impl Into<String>, id: impl Into<String>) -> Self {
        ReadBuilder {
            ctx: ctx.clone(),
            name: name.into(),
            id: id.into(),
            props: serde_json::Value::Object(Default::default()),
            opts: ResourceOptions {
                version: R::VERSION.to_string(),
                plugin_download_url: R::PLUGIN_DOWNLOAD_URL.to_string(),
                ..Default::default()
            },
            _marker: std::marker::PhantomData,
        }
    }

    /// Sets known properties of the existing resource.
    pub fn props(mut self, props: serde_json::Value) -> Self {
        self.props = props;
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

/// `ReadBuilder<R>` can be `.await`ed directly.
impl<R: Resource> IntoFuture for ReadBuilder<R> {
    type Output = Result<RegisteredResource<R>>;
    type IntoFuture = impl Future<Output = Self::Output> + Send;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let result = read_resource_inner(
                &self.ctx,
                R::TYPE_TOKEN,
                &self.name,
                &self.id,
                self.props,
                &self.opts,
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
    pub async fn register_outputs(&self, ctx: &Context, outputs: serde_json::Value) -> Result<()> {
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

// ---------------------------------------------------------------------------
// Internal registration functions (used by builders and stack.rs)
// ---------------------------------------------------------------------------

pub(crate) fn alias_to_proto(alias: &Alias) -> pulumirpc::Alias {
    match alias {
        Alias::Urn(urn) => pulumirpc::Alias {
            alias: Some(pulumirpc::alias::Alias::Urn(urn.clone())),
        },
        Alias::Spec(spec) => pulumirpc::Alias {
            alias: Some(pulumirpc::alias::Alias::Spec(pulumirpc::alias::Spec {
                name: spec.name.clone().unwrap_or_default(),
                r#type: spec.r#type.clone().unwrap_or_default(),
                stack: spec.stack.clone().unwrap_or_default(),
                project: spec.project.clone().unwrap_or_default(),
                parent: spec.parent.as_ref().map(|p| match p {
                    AliasParent::Urn(urn) => pulumirpc::alias::spec::Parent::ParentUrn(urn.clone()),
                    AliasParent::NoParent => pulumirpc::alias::spec::Parent::NoParent(true),
                }),
            })),
        },
    }
}

pub(crate) async fn register_resource_inner(
    ctx: &Context,
    resource_type: &str,
    name: &str,
    inputs: serde_json::Value,
    opts: &ResourceOptions,
    custom: bool,
    remote: bool,
) -> Result<ResourceResult> {
    let object = json_to_struct(&inputs);

    let parent = opts.parent.clone().unwrap_or_default();

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
        custom_timeouts: opts.custom_timeouts.as_ref().map(|t| {
            pulumirpc::register_resource_request::CustomTimeouts {
                create: t.create.clone().unwrap_or_default(),
                update: t.update.clone().unwrap_or_default(),
                delete: t.delete.clone().unwrap_or_default(),
            }
        }),
        delete_before_replace_defined: opts.delete_before_replace,
        supports_partial_values: true,
        remote,
        accept_resources: true,
        providers: HashMap::new(),
        replace_on_changes: opts.replace_on_changes.clone(),
        plugin_download_url: opts.plugin_download_url.clone(),
        plugin_checksums: HashMap::new(),
        retain_on_delete: Some(opts.retain_on_delete),
        aliases: opts.aliases.iter().map(alias_to_proto).collect(),
        deleted_with: opts.deleted_with.clone().unwrap_or_default(),
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

    let resp = ctx.monitor().register_resource(req).await?;

    // Check if the registration was reported as failed.
    if resp.result == pulumirpc::Result::Fail as i32 {
        return Err(Error::ResourceFailed {
            urn: resp.urn.clone(),
        });
    }

    let outputs = resp
        .object
        .as_ref()
        .map(struct_to_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let property_deps = resp
        .property_dependencies
        .iter()
        .map(|(k, v)| (k.clone(), v.urns.clone()))
        .collect();

    Ok(ResourceResult {
        urn: resp.urn,
        id: resp.id,
        outputs,
        property_deps,
    })
}

pub(crate) async fn register_resource_outputs(
    ctx: &Context,
    urn: &str,
    outputs: serde_json::Value,
) -> Result<()> {
    let outputs_struct = json_to_struct(&outputs);

    let req = pulumirpc::RegisterResourceOutputsRequest {
        urn: urn.to_string(),
        outputs: Some(outputs_struct),
    };

    ctx.monitor().register_resource_outputs(req).await?;

    Ok(())
}

async fn read_resource_inner(
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

    let resp = ctx.monitor().read_resource(req).await?;

    let outputs = resp
        .properties
        .as_ref()
        .map(struct_to_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    Ok(ResourceResult {
        urn: resp.urn,
        id: id.to_string(),
        outputs,
        property_deps: HashMap::new(),
    })
}
