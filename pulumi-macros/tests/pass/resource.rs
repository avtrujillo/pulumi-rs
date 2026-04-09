use pulumi_macros::Resource;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct BucketArgs {
    bucket: String,
}

#[derive(Deserialize, Clone)]
struct BucketOutputs {
    arn: String,
}

#[derive(Resource)]
#[pulumi(type_token = "aws:s3/bucket:Bucket")]
#[pulumi(inputs = BucketArgs)]
#[pulumi(outputs = BucketOutputs)]
struct Bucket;

fn main() {}
