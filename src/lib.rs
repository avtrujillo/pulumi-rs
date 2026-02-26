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
//! use pulumi::{Context, Output, Result};
//!
//! #[tokio::main]
//! async fn main() {
//!     pulumi::run(my_stack).await.expect("pulumi program failed");
//! }
//!
//! async fn my_stack(ctx: Context) -> pulumi::Result<serde_json::Value> {
//!     // Register resources here using ctx
//!     let (urn, id, outputs) = pulumi::CustomResource::new("aws:s3/bucket:Bucket", "my-bucket")
//!         .inputs(serde_json::json!({ "bucket": "my-unique-bucket-name" }))
//!         .register(&ctx)
//!         .await?;
//!
//!     // Return stack exports
//!     Ok(serde_json::json!({
//!         "bucketUrn": urn.get().await,
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
//! - [`Output<T>`] — A value that may not be known yet (the fundamental Pulumi type)
//! - [`Context`] — The Pulumi program context (gRPC connections + config)
//! - [`resource::ResourceOptions`] — Options for resource registration
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
pub use output::{all, all2, all3, from_future, Output};
pub use resource::{CustomResource, ResourceOptions};

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
