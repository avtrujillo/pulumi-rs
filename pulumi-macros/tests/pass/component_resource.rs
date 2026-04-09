use pulumi_macros::ComponentResource;

#[derive(ComponentResource)]
#[pulumi(type_token = "my:module:WebApp")]
struct WebApp;

fn main() {}
