use pulumi::{Context, Resource, ResourceBuilder, Result};
use serde::{Deserialize, Serialize};

struct RandomString;

impl Resource for RandomString {
    const TYPE_TOKEN: &'static str = "random:index/randomString:RandomString";
    type Inputs = RandomStringInputs;
    type Outputs = RandomStringOutputs;
}

#[derive(Serialize)]
struct RandomStringInputs {
    length: i32,
    special: bool,
}

#[derive(Deserialize, Clone)]
struct RandomStringOutputs {
    result: String,
    length: i32,
}

#[tokio::main]
async fn main() {
    pulumi::run(stack).await.expect("program failed");
}

async fn stack(ctx: Context) -> Result<serde_json::Value> {
    let r = ResourceBuilder::<RandomString>::new(
        &ctx,
        "test-string",
        RandomStringInputs {
            length: 16,
            special: false,
        },
    )
    .await?;

    Ok(serde_json::json!({
        "result": r.outputs.result,
        "length": r.outputs.length,
    }))
}
