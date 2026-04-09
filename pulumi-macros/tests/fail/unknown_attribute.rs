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
#[pulumi(type_token = "pkg:mod:Res")]
#[pulumi(inputs = Args)]
#[pulumi(outputs = Outputs)]
#[pulumi(unknown_attr = "foo")]
struct MyResource;

fn main() {}
