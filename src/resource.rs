use std::collections::HashMap;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::ResourceOutput;
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
/// This provides a higher-level API where results are returned as
/// `ResourceOutput<T>` values.
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
    /// Returns `(urn, id, outputs)` as `ResourceOutput` values.
    pub async fn register(
        self,
        ctx: &Context,
    ) -> Result<(ResourceOutput<String>, ResourceOutput<String>, ResourceOutput<serde_json::Value>)> {
        let result = register_resource(ctx, &self.resource_type, &self.name, self.inputs, &self.opts).await?;

        let urn = ResourceOutput::new(result.urn);
        let id = ResourceOutput::new(result.id);
        let outputs = ResourceOutput::new(result.outputs);

        Ok((urn, id, outputs))
    }
}

/// Helper to extract a string property from a JSON output.
pub fn get_output_string(
    outputs: &ResourceOutput<serde_json::Value>,
    key: &str,
) -> ResourceOutput<Option<String>> {
    let key = key.to_string();
    outputs.map(move |v| {
        v.as_object()
            .and_then(|m| m.get(&key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })
}
