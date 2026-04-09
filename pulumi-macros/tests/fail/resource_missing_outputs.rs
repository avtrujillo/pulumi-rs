use pulumi_macros::Resource;
use serde::Serialize;

#[derive(Serialize)]
struct Args {
    name: String,
}

#[derive(Resource)]
#[pulumi(type_token = "pkg:mod:Res")]
#[pulumi(inputs = Args)]
struct MyResource;

fn main() {}
