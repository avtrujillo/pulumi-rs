use crate::context::Context;
use crate::error::Result;
use crate::resource::{ResourceOptions, register_resource_inner, register_resource_outputs};

/// Registers the stack resource itself.
///
/// This is called automatically by [`crate::run`] to create the root stack resource.
/// The returned URN is set as the root resource in the engine.
pub(crate) async fn register_stack(ctx: &Context) -> Result<String> {
    let stack_name = format!("{}-{}", ctx.project(), ctx.stack());

    let result = register_resource_inner(
        ctx,
        "pulumi:pulumi:Stack",
        &stack_name,
        serde_json::Value::Object(Default::default()),
        &ResourceOptions::default(),
        true,
        false,
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
pub async fn export_outputs(
    ctx: &Context,
    stack_urn: &str,
    outputs: serde_json::Value,
) -> Result<()> {
    register_resource_outputs(ctx, stack_urn, outputs).await
}
