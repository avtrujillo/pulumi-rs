use pulumi_macros::ComponentMethod;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct DoThingArgs {
    input: String,
}

#[derive(Deserialize)]
struct DoThingResult {
    output: String,
}

#[derive(ComponentMethod)]
#[pulumi(token = "my:module:MyComponent/doThing")]
#[pulumi(args = DoThingArgs)]
#[pulumi(returns = DoThingResult)]
struct MyComponentDoThing;

fn main() {}
