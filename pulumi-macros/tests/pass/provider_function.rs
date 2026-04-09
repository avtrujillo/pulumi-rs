use pulumi_macros::ProviderFunction;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct GetAmiArgs {
    most_recent: bool,
}

#[derive(Deserialize)]
struct GetAmiResult {
    id: String,
}

#[derive(ProviderFunction)]
#[pulumi(token = "aws:index/getAmi:getAmi")]
#[pulumi(args = GetAmiArgs)]
#[pulumi(returns = GetAmiResult)]
struct GetAmi;

fn main() {}
