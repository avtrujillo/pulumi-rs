use pulumi::{Context, Result};

#[tokio::main]
async fn main() {
    pulumi::run(stack).await.expect("program failed");
}

async fn stack(ctx: Context) -> Result<serde_json::Value> {
    // Config keys are namespaced by project name: "config:<key>".
    let message = ctx.require_config("config:message")?;

    Ok(serde_json::json!({
        "message": message,
    }))
}
