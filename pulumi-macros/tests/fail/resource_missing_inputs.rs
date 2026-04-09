use pulumi_macros::Resource;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone)]
struct Outputs {
    id: String,
}

#[derive(Resource)]
#[pulumi(type_token = "pkg:mod:Res")]
#[pulumi(outputs = Outputs)]
struct MyResource;

fn main() {}
