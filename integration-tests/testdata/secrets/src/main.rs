use pulumi::{Context, Resource, ResourceBuilder, Result};
use serde::{Deserialize, Serialize};

struct RandomPassword;

impl Resource for RandomPassword {
    const TYPE_TOKEN: &'static str = "random:index/randomPassword:RandomPassword";
    type Inputs = RandomPasswordInputs;
    type Outputs = RandomPasswordOutputs;
}

#[derive(Serialize)]
struct RandomPasswordInputs {
    length: i32,
    special: bool,
}

// The `result` field is marked secret by the random provider.
#[derive(Deserialize, Clone)]
struct RandomPasswordOutputs {
    result: String,
    length: i32,
}

#[tokio::main]
async fn main() {
    pulumi::run(stack).await.expect("program failed");
}

async fn stack(ctx: Context) -> Result<serde_json::Value> {
    let r = ResourceBuilder::<RandomPassword>::new(
        &ctx,
        "test-password",
        RandomPasswordInputs {
            length: 20,
            special: false,
        },
    )
    .await?;

    // Export `result` (secret) and `length` (plain).
    // The Pulumi engine propagates the secret flag from the provider response,
    // so `result` will be marked secret in the stack state.
    Ok(serde_json::json!({
        "result": r.outputs.result,
        "length": r.outputs.length,
    }))
}
