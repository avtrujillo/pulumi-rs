# pulumi-rs

Native Rust SDK for [Pulumi](https://www.pulumi.com/) — define cloud infrastructure as strongly-typed Rust code.

## What is Pulumi?

Pulumi is an infrastructure-as-code (IaC) platform. Instead of writing YAML or HCL configuration files, you write a program in a real programming language. Your program declares the cloud resources you want (S3 buckets, VMs, databases, DNS records, etc.), and the Pulumi engine figures out how to create, update, or delete them to match.

When you run `pulumi up`, the Pulumi CLI:

1. Starts your program as a subprocess
2. Sets environment variables telling it how to connect back to the engine via gRPC
3. Listens for resource registration calls from your program
4. Diffs the desired state against the last-known state
5. Calls the appropriate cloud provider APIs to converge

This SDK is the Rust side of that conversation. It speaks gRPC to the Pulumi engine, handling serialization, dependency tracking, and the resource lifecycle.

## Requirements

- **Rust nightly** — required for the `impl_trait_in_assoc_type` feature (see [why nightly?](#why-nightly))
- **Pulumi CLI** — install from [pulumi.com/docs/install](https://www.pulumi.com/docs/install/)
- **A cloud provider plugin** — e.g. `pulumi plugin install resource aws`

## Quick start

Add the dependency:

```toml
[package]
edition = "2024"

[dependencies]
pulumi = { path = "." }  # or from a registry once published
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

Write your program:

```rust
#![feature(impl_trait_in_assoc_type)]

use pulumi::{Context, Resource, ResourceBuilder, Result};
use serde::{Deserialize, Serialize};

// 1. Define a resource type
struct S3Bucket;

#[derive(Serialize)]
struct S3BucketArgs {
    bucket: String,
}

#[derive(Deserialize, Clone)]
struct S3BucketOutputs {
    arn: String,
    bucket: String,
    // ... any other fields the provider returns
}

impl Resource for S3Bucket {
    const TYPE_TOKEN: &'static str = "aws:s3/bucket:Bucket";
    type Inputs = S3BucketArgs;
    type Outputs = S3BucketOutputs;
}

// 2. Write your stack function
async fn my_stack(ctx: Context) -> Result<serde_json::Value> {
    let bucket = ResourceBuilder::<S3Bucket>::new(&ctx, "my-bucket", S3BucketArgs {
        bucket: "my-globally-unique-name".into(),
    }).await?;

    Ok(serde_json::json!({
        "bucketArn": bucket.outputs.arn,
    }))
}

// 3. Run it
#[tokio::main]
async fn main() {
    pulumi::run(my_stack).await.expect("pulumi program failed");
}
```

Then deploy:

```bash
pulumi up
```

## Core concepts

### Resources

A **resource** is anything managed by a cloud provider: a VM, a database, a DNS record, a Kubernetes deployment. In this SDK, every resource is described by a Rust type that implements a trait. You never pass raw strings around — the type system ensures you use the right inputs and get the right outputs.

There are three kinds of resources:

- **Custom resource** (`Resource` / `ResourceBuilder<R>`) — A real cloud object managed by a provider plugin (S3 bucket, EC2 instance, etc.)
- **Component resource** (`ComponentResource` / `ComponentBuilder<C>`) — A logical grouping you define to organize child resources (no cloud API call)
- **Remote component** (`RemoteComponent` / `RemoteComponentBuilder<R>`) — A higher-level component implemented inside a provider plugin

### The `Resource` trait

This is the most common trait. It maps a Rust type to a specific cloud resource:

```rust
pub trait Resource: Sized + Send + 'static {
    /// The Pulumi type token — identifies this resource to the engine.
    /// Format: "provider:module/name:Name", e.g. "aws:s3/bucket:Bucket"
    const TYPE_TOKEN: &'static str;

    /// Provider plugin version (optional, defaults to "")
    const VERSION: &'static str = "";

    /// Plugin download URL (optional, defaults to "")
    const PLUGIN_DOWNLOAD_URL: &'static str = "";

    /// What you pass in when creating the resource.
    /// Must implement serde::Serialize — it gets sent as JSON over gRPC.
    type Inputs: Serialize + Send + 'static;

    /// What the provider gives back after creating/updating the resource.
    /// Must implement serde::Deserialize — it's parsed from the gRPC response.
    type Outputs: DeserializeOwned + Clone + Send + Sync + 'static;
}
```

**Where do type tokens come from?** Every Pulumi provider has a schema that lists its resources and their type tokens. For example, the AWS provider's S3 bucket is `"aws:s3/bucket:Bucket"`, and its Lambda function is `"aws:lambda/function:Function"`. You can find these in the [Pulumi Registry](https://www.pulumi.com/registry/).

**How do I know what fields go in Inputs and Outputs?** The provider schema also describes the input and output properties. The input struct fields should match the provider's input property names (use `#[serde(rename = "...")]` if the Rust name differs). Output fields are populated by the provider after the resource is created.

### Registering resources with builders

Every resource type has a corresponding builder that implements `IntoFuture`, so you can `.await` it directly:

```rust
// Minimal — just provide context, a logical name, and inputs:
let bucket = ResourceBuilder::<S3Bucket>::new(&ctx, "my-bucket", args).await?;

// With options — the builder has chainable methods:
let bucket = ResourceBuilder::<S3Bucket>::new(&ctx, "my-bucket", args)
    .parent(vpc_urn)           // set a parent resource
    .provider(aws_provider)    // use a specific provider instance
    .depends_on(other_urn)     // explicit dependency
    .protect()                 // prevent accidental deletion
    .delete_before_replace()   // delete old resource before creating replacement
    .ignore_changes(vec!["tags".into()])  // don't diff these properties
    .retain_on_delete()        // keep cloud resource when removed from code
    .import_id("existing-id")  // import an existing cloud resource
    .await?;
```

The result is a `RegisteredResource<R>` with typed outputs:

```rust
pub struct RegisteredResource<R: Resource> {
    pub urn: String,                              // engine-assigned identifier
    pub id: String,                               // cloud provider ID
    pub outputs: R::Outputs,                      // your typed output struct
    pub property_deps: HashMap<String, Vec<String>>, // per-property dependencies
}
```

### Reading existing resources

To import an existing cloud resource (one not originally created by Pulumi) into your state:

```rust
let existing = ReadBuilder::<S3Bucket>::new(&ctx, "imported-bucket", "my-actual-bucket-id")
    .await?;

println!("ARN of existing bucket: {}", existing.outputs.arn);
```

### Component resources

Components are logical groupings. They don't create cloud resources themselves — they just provide a parent URN that child resources can reference for organizational hierarchy:

```rust
struct MyVpcSetup;

impl ComponentResource for MyVpcSetup {
    const TYPE_TOKEN: &'static str = "my:network:VpcSetup";
}

async fn create_vpc(ctx: &Context) -> Result<()> {
    let component = ComponentBuilder::<MyVpcSetup>::new(ctx, "vpc-setup").await?;

    // Create child resources under this component:
    let subnet = ResourceBuilder::<Subnet>::new(ctx, "public-subnet", subnet_args)
        .parent(component.urn())
        .await?;

    // Signal that all children are registered:
    component.register_outputs(ctx, serde_json::json!({
        "subnetId": subnet.id,
    })).await?;

    Ok(())
}
```

### Remote components

Remote components are like custom resources but are implemented as multi-resource components inside a provider plugin. They take typed inputs and return typed outputs:

```rust
struct EksCluster;

impl RemoteComponent for EksCluster {
    const TYPE_TOKEN: &'static str = "eks:index:Cluster";
    const VERSION: &'static str = "2.0.0";
    type Inputs = EksClusterArgs;
    type Outputs = EksClusterOutputs;
}

let cluster = RemoteComponentBuilder::<EksCluster>::new(&ctx, "my-cluster", args).await?;
```

## Provider functions

Providers also expose read-only functions (sometimes called "data sources" in other IaC tools). For example, looking up an AMI ID or getting the current caller identity. These are modeled with the `ProviderFunction` trait:

```rust
pub trait ProviderFunction: Sized + Send + 'static {
    const TOKEN: &'static str;             // e.g. "aws:ec2/getAmi:getAmi"
    const VERSION: &'static str = "";
    const PLUGIN_DOWNLOAD_URL: &'static str = "";
    type Args: Serialize + Send + 'static;
    type Returns: DeserializeOwned + Send + 'static;
}
```

Usage:

```rust
struct GetCallerIdentity;

#[derive(Serialize)]
struct GetCallerIdentityArgs {}

#[derive(Deserialize)]
struct GetCallerIdentityResult {
    account_id: String,
    arn: String,
    user_id: String,
}

impl ProviderFunction for GetCallerIdentity {
    const TOKEN: &'static str = "aws:index/getCallerIdentity:getCallerIdentity";
    type Args = GetCallerIdentityArgs;
    type Returns = GetCallerIdentityResult;
}

let identity = InvokeBuilder::<GetCallerIdentity>::new(&ctx, GetCallerIdentityArgs {})
    .await?;

println!("AWS Account: {}", identity.account_id);
```

### Component methods

Some remote components expose methods that can be called after the resource is created. These use the `ComponentMethod` trait and `CallBuilder`:

```rust
struct StaticPageDeploy;

impl ComponentMethod for StaticPageDeploy {
    const TOKEN: &'static str = "my:cdn:StaticSite/deploy";
    type Args = DeployArgs;
    type Returns = DeployResult;
}

let result = CallBuilder::<StaticPageDeploy>::new(&ctx, deploy_args)
    .arg_dep("siteUrn", vec![site.urn.clone()])  // track input dependencies
    .await?;

// result.result   — typed DeployResult
// result.return_deps — per-property dependency URNs on the return
```

## Output\<T\> — deferred values

`Output<T>` represents a value that might not be known yet. During `pulumi preview`, resource IDs and computed properties are unknown — they only become real values during `pulumi up`. The `Output` type handles this cleanly.

```rust
// Create a resolved output:
let name = Output::new("my-bucket".to_string());

// Transform with map (like Iterator::map, but async + deferred):
let url = name.map(|n| format!("https://{n}.s3.amazonaws.com"));

// Chain outputs that depend on each other:
let derived = name.flat_map(|n| {
    Output::new(format!("{n}-backup"))
});

// Wait for the value:
let value: String = url.await?;
// or non-consuming:
let value: String = url.get().await?;

// Combine multiple outputs:
let pair = pulumi::all2(&output_a, &output_b);  // Output<(A, B)>
let triple = pulumi::all3(&a, &b, &c);          // Output<(A, B, C)>
let vec = pulumi::all(vec![o1, o2, o3]);         // Output<Vec<T>>

// Create from an async computation:
let out = pulumi::from_future(async { expensive_calc().await });

// Manual resolve/reject (advanced):
let (output, resolver) = Output::<String>::unresolved();
resolver.resolve("hello".into());
// or: resolver.reject(Error::Custom("failed".into()));

// Secret outputs (engine encrypts the value at rest):
let secret = Output::secret("my-api-key".to_string());
```

`Output<T>` is `Clone` and `Send + Sync`, so it can be shared across tasks freely.

## Stack outputs (exports)

Your program function returns a `serde_json::Value` representing the stack's exports. These become visible in `pulumi stack output` and can be consumed by other stacks via stack references:

```rust
async fn my_stack(ctx: Context) -> Result<serde_json::Value> {
    let bucket = ResourceBuilder::<S3Bucket>::new(&ctx, "site", args).await?;
    let func = ResourceBuilder::<LambdaFunction>::new(&ctx, "handler", fn_args).await?;

    // These values are your stack outputs:
    Ok(serde_json::json!({
        "bucketArn": bucket.outputs.arn,
        "functionName": func.outputs.function_name,
        "endpoint": format!("https://{}", func.outputs.invoke_url),
    }))
}
```

## Configuration

The Pulumi CLI passes config values (set via `pulumi config set`) to your program. Access them through the `Context`:

```rust
async fn my_stack(ctx: Context) -> Result<serde_json::Value> {
    // Optional config:
    let region = ctx.get_config("aws:region");  // Option<&str>

    // Required config (returns Error if missing):
    let domain = ctx.require_config("myapp:domain")?;

    // Other context info:
    let project = ctx.project();        // project name
    let stack = ctx.stack();            // stack name (e.g. "dev", "prod")
    let is_preview = ctx.is_dry_run();  // true during `pulumi preview`

    Ok(serde_json::json!({}))
}
```

## Logging

Send log messages to the Pulumi CLI output:

```rust
pulumi::log::info(&ctx, "deploying bucket").await?;
pulumi::log::warn(&ctx, "deprecated API used").await?;
pulumi::log::error(&ctx, "something went wrong").await?;
pulumi::log::debug(&ctx, "detailed trace info").await?;
```

## Error handling

All operations return `pulumi::Result<T>`, which uses the `pulumi::Error` enum:

| Variant | When it happens |
|---------|-----------------|
| `Transport(tonic::transport::Error)` | gRPC connection failure |
| `Rpc(tonic::Status)` | Engine/monitor returned an error |
| `MissingEnv(&'static str)` | Required `PULUMI_*` env var not set |
| `Serde(serde_json::Error)` | Serialization/deserialization failed |
| `ResourceFailed { urn }` | Provider reported resource creation failure |
| `InvokeFailure { token, failures }` | Provider function returned check failures |
| `Custom(String)` | Anything else |

## How the type system works (design notes)

The SDK uses Rust's type system to eliminate an entire class of errors that plague untyped IaC:

**Trait-per-entity pattern.** Each cloud resource, provider function, or component method is a Rust type (usually a zero-sized struct) with a trait impl. The trait's associated types (`Inputs`, `Outputs`, `Args`, `Returns`) are your serde structs. This means:
- You can't pass Lambda inputs to an S3 Bucket registration — it won't compile
- You get autocomplete on output fields instead of stringly-typed property access
- Refactoring a field name is a compiler error, not a runtime surprise

**`IntoFuture` builders.** Every builder (`ResourceBuilder`, `ReadBuilder`, `InvokeBuilder`, `ComponentBuilder`, `RemoteComponentBuilder`, `CallBuilder`) implements `IntoFuture`. This means you can `.await` the builder directly — no `.build().execute()` ceremony.

**Constants for metadata.** Type tokens, versions, and download URLs are `const` — they're baked into the binary at compile time and automatically threaded through the builders. You declare them once on the trait impl and never think about them again.

### Why nightly?

This SDK requires the nightly-only feature `impl_trait_in_assoc_type` ([tracking issue](https://github.com/rust-lang/rust/issues/63063)). The reason comes down to a gap in stable Rust's type system around async and associated types.

We want builders you can `.await` directly:

```rust
let bucket = ResourceBuilder::<S3Bucket>::new(&ctx, "my-bucket", args).await?;
```

The `.await` syntax on non-futures works via the `IntoFuture` trait, which requires you to declare the future as an associated type:

```rust
impl<R: Resource> IntoFuture for ResourceBuilder<R> {
    type Output = Result<RegisteredResource<R>>;
    type IntoFuture = /* what goes here? */;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            // serialize inputs, make gRPC call, deserialize outputs...
        }
    }
}
```

The `into_future` body is an `async` block. The compiler generates an anonymous, unnameable type for it — so you can't write it in the `type IntoFuture = ...` position. On stable Rust, you have two workarounds:

**Option A: Box the future.** Erase the type behind `Pin<Box<dyn Future>>`:

```rust
type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;
```

This compiles, but now every builder carries a vtable pointer and dynamic dispatch where a direct call would do. It's not about heap allocation — tokio already heap-allocates the top-level task — it's that you've made the future opaque to the compiler. It can't inline the async state machine into the parent future, so you end up with an extra indirection per `.await` for no semantic reason.

**Option B: Don't use `IntoFuture`.** Add an explicit `.execute()` method that returns `impl Future` (which *is* allowed in return position on stable). This avoids the associated type problem, but now every call site is `builder.execute().await` instead of `builder.await` — noisy, and surprising to anyone who expects `.await` to just work on an async-flavored builder.

The `impl_trait_in_assoc_type` feature lets us write:

```rust
type IntoFuture = impl Future<Output = Self::Output> + Send;
```

The compiler fills in the concrete async block type. No boxing, no extra method — just `.await` a builder and it works. The SDK uses this in seven `IntoFuture` impls (`Output<T>`, `ResourceBuilder`, `ReadBuilder`, `ComponentBuilder`, `RemoteComponentBuilder`, `InvokeBuilder`, `CallBuilder`).

## Project structure

```
src/
├── lib.rs          Entry point: run(), re-exports
├── context.rs      Context (gRPC connections, config access)
├── resource.rs     Resource/ComponentResource/RemoteComponent traits + builders
├── invoke.rs       ProviderFunction/ComponentMethod traits + builders
├── output.rs       Output<T> — deferred value type with combinators
├── error.rs        Error enum
├── log.rs          Logging to Pulumi engine
├── stack.rs        Root stack registration + export_outputs
├── serde.rs        JSON ↔ Protobuf Struct conversion (internal)
proto/
└── pulumi/         Protobuf definitions for Pulumi gRPC protocol
build.rs            Compiles .proto files at build time via tonic-build
Cargo.toml          Rust nightly, edition 2024
```

## Building

```bash
cargo build          # includes protobuf compilation via build.rs
cargo test           # run all tests (async tests using #[tokio::test])
cargo clippy         # lint
```

## License

Apache-2.0
