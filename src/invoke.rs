use std::collections::HashMap;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::proto::pulumirpc;
use crate::serde::{json_to_struct, struct_to_json};

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

/// Invokes a provider function and returns the result.
///
/// Provider functions are things like `aws:index/getAmi:getAmi` that query
/// data from the cloud without creating resources.
///
/// # Arguments
///
/// * `ctx` - The Pulumi context.
/// * `token` - The function token (e.g. `"aws:index/getAmi:getAmi"`).
/// * `args` - The function arguments as a JSON value.
/// * `opts` - Invoke options.
///
/// # Returns
///
/// The function's return value as JSON.
pub async fn invoke(
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

/// Calls a provider method (for component resources that expose methods).
///
/// This is a higher-level operation than `invoke` — it supports dependency
/// tracking on both inputs and outputs.
pub async fn call(
    ctx: &Context,
    token: &str,
    args: serde_json::Value,
    arg_deps: HashMap<String, Vec<String>>,
    opts: &InvokeOptions,
) -> Result<(serde_json::Value, HashMap<String, Vec<String>>)> {
    let args_struct = json_to_struct(&args);

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
