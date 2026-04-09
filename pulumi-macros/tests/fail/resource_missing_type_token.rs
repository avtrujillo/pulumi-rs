use pulumi_macros::Resource;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Args {
    name: String,
}
#[derive(Deserialize, Clone)]
struct Outputs {
    id: String,
}

#[derive(Resource)]
#[pulumi(inputs = Args)]
#[pulumi(outputs = Outputs)]
struct MyResource;

fn main() {}
