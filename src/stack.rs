use crate::context::Context;
use crate::error::Result;
use crate::resource::{register_resource, register_resource_outputs, ResourceOptions};

/// Registers the stack resource itself.
///
/// This is called automatically by [`crate::run`] to create the root stack resource.
/// The returned URN is set as the root resource in the engine.
pub(crate) async fn register_stack(ctx: &Context) -> Result<String> {
    let stack_type = "pulumi:pulumi:Stack";
    let stack_name = format!("{}-{}", ctx.project(), ctx.stack());

    let result = register_resource(
        ctx,
        stack_type,
        &stack_name,
        serde_json::Value::Object(Default::default()),
        &ResourceOptions::default(),
    )
    .await?;

    // Set the root resource URN in the engine.
    let mut engine = ctx.engine().await;
    engine
        .set_root_resource(crate::proto::pulumirpc::SetRootResourceRequest {
            urn: result.urn.clone(),
        })
        .await?;

    Ok(result.urn)
}

/// Registers stack outputs (exports).
///
/// Stack outputs are values that are exported from the Pulumi program and
/// can be referenced by other stacks or viewed in the Pulumi console.
///
/// # Example
///
/// ```ignore
/// pulumi::stack::export_outputs(&ctx, &stack_urn, serde_json::json!({
///     "bucketName": bucket_name,
///     "endpoint": endpoint,
/// })).await?;
/// ```
pub async fn export_outputs(
    ctx: &Context,
    stack_urn: &str,
    outputs: serde_json::Value,
) -> Result<()> {
    register_resource_outputs(ctx, stack_urn, outputs).await
}
