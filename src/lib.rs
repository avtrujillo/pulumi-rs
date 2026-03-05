#![feature(impl_trait_in_assoc_type)]

//! # Pulumi SDK for Rust
//!
//! This crate provides a native Rust SDK for [Pulumi](https://www.pulumi.com),
//! the infrastructure-as-code platform. It allows you to define cloud infrastructure
//! using Rust, communicating with the Pulumi engine over gRPC.
//!
//! ## Quick Start
//!
//! A Pulumi Rust program is an async function that receives a [`Context`] and uses
//! it to register resources. The [`run`] function handles all the engine communication
//! setup.
//!
//! ```ignore
//! use pulumi::{Context, Resource, ResourceBuilder, Result};
//! use serde::{Deserialize, Serialize};
//!
//! // Define a resource at the type level.
//! struct S3Bucket;
//!
//! impl Resource for S3Bucket {
//!     const TYPE_TOKEN: &'static str = "aws:s3/bucket:Bucket";
//!     type Inputs = S3BucketArgs;
//!     type Outputs = S3BucketOutputs;
//! }
//!
//! #[derive(Serialize)]
//! struct S3BucketArgs { bucket: String }
//!
//! #[derive(Deserialize, Clone)]
//! struct S3BucketOutputs { arn: String }
//!
//! #[tokio::main]
//! async fn main() {
//!     pulumi::run(my_stack).await.expect("pulumi program failed");
//! }
//!
//! async fn my_stack(ctx: Context) -> pulumi::Result<serde_json::Value> {
//!     let bucket = ResourceBuilder::<S3Bucket>::new(&ctx, "my-bucket", S3BucketArgs {
//!         bucket: "my-unique-bucket-name".into(),
//!     }).await?;
//!
//!     Ok(serde_json::json!({
//!         "bucketArn": bucket.outputs.arn,
//!     }))
//! }
//! ```
//!
//! ## Architecture
//!
//! When the Pulumi CLI runs your program, it sets environment variables
//! (`PULUMI_MONITOR`, `PULUMI_ENGINE`, etc.) that this SDK uses to connect
//! to the engine via gRPC. The SDK then:
//!
//! 1. Registers a root stack resource
//! 2. Calls your program function
//! 3. Registers stack outputs (exports)
//!
//! ## Core Types
//!
//! - [`Resource`] — Trait that describes a cloud resource at the type level
//! - [`ResourceBuilder`] — Builder to register a [`Resource`]; implements [`IntoFuture`](std::future::IntoFuture)
//! - [`Output<T>`] — A value that may not be known yet (the fundamental Pulumi type)
//! - [`Context`] — The Pulumi program context (gRPC connections + config)
//! - [`ProviderFunction`] — Trait that describes a provider function at the type level
//! - [`InvokeBuilder`] — Builder to invoke a [`ProviderFunction`]; implements [`IntoFuture`](std::future::IntoFuture)
//! - [`Error`] / [`Result`] — Error handling

pub mod context;
pub mod error;
pub mod invoke;
pub mod log;
pub mod output;
pub mod resource;
pub mod stack;

pub(crate) mod serde;

/// Generated protobuf types for the Pulumi gRPC protocol.
#[allow(warnings)]
pub(crate) mod proto {
    pub mod pulumirpc {
        tonic::include_proto!("pulumirpc");
    }
}

// Re-export core types at the crate root.
pub use context::Context;
pub use error::{Error, Result};
pub use invoke::{CallBuilder, CallResult, ComponentMethod, InvokeBuilder, InvokeOptions, ProviderFunction};
pub use output::{all, all2, all3, from_future, Output};
pub use resource::{
    ComponentBuilder, ComponentResource, ReadBuilder, RegisteredComponent,
    RegisteredRemoteComponent, RegisteredResource, RemoteComponent, RemoteComponentBuilder,
    Resource, ResourceBuilder, ResourceOptions,
};

/// Runs a Pulumi program.
///
/// This is the main entry point for a Pulumi Rust program. It:
///
/// 1. Reads engine connection settings from environment variables
/// 2. Connects to the Pulumi engine and resource monitor via gRPC
/// 3. Registers the root stack resource
/// 4. Calls your program function
/// 5. Registers stack outputs
///
/// The program function receives a [`Context`] and should return a JSON value
/// containing the stack's exports (outputs), or an empty object if there are none.
///
/// # Example
///
/// ```ignore
/// #[tokio::main]
/// async fn main() {
///     pulumi::run(|ctx| async move {
///         // Register resources...
///         Ok(serde_json::json!({}))
///     }).await.expect("pulumi program failed");
/// }
/// ```
pub async fn run<F, Fut>(program: F) -> Result<()>
where
    F: FnOnce(Context) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value>>,
{
    let settings = context::Settings::from_env()?;
    let ctx = Context::new(settings).await?;

    // Register the root stack resource.
    let stack_urn = stack::register_stack(&ctx).await?;

    // Run the user's program.
    let exports = program(ctx.clone()).await?;

    // Register stack outputs.
    stack::export_outputs(&ctx, &stack_urn, exports).await?;

    Ok(())
}
