use pulumi_macros::Resource;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct InstanceArgs {
    ami: String,
}

#[derive(Deserialize, Clone)]
struct InstanceOutputs {
    id: String,
}

#[derive(Resource)]
#[pulumi(type_token = "aws:ec2/instance:Instance")]
#[pulumi(inputs = InstanceArgs)]
#[pulumi(outputs = InstanceOutputs)]
#[pulumi(version = "6.0.0")]
#[pulumi(plugin_download_url = "https://example.com")]
struct Instance;

fn main() {}
