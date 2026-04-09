use pulumi_macros::ProviderFunction;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Args {
    x: i32,
}
#[derive(Deserialize)]
struct Returns {
    y: i32,
}

#[derive(ProviderFunction)]
#[pulumi(args = Args)]
#[pulumi(returns = Returns)]
struct MyFunc;

fn main() {}
