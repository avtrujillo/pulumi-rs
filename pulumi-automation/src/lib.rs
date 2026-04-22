//! # Pulumi Automation API for Rust
//!
//! Programmatic interface for driving Pulumi stack operations without shelling
//! out to the CLI manually. This crate wraps the `pulumi` CLI, parsing its
//! structured JSON output into typed Rust values.
//!
//! ## Quick Start
//!
//! ```ignore
//! use pulumi_automation::LocalWorkspace;
//!
//! #[tokio::main]
//! async fn main() -> pulumi_automation::Result<()> {
//!     let ws = LocalWorkspace::new("./my-pulumi-project");
//!     let stack = ws.create_or_select_stack("dev").await?;
//!
//!     // Set config
//!     stack.set_config("aws:region", &"us-west-2".into()).await?;
//!
//!     // Deploy
//!     let result = stack.up().await?;
//!     println!("update stdout: {}", result.stdout);
//!
//!     // Read outputs
//!     let outputs = stack.outputs().await?;
//!     println!("outputs: {outputs:?}");
//!
//!     Ok(())
//! }
//! ```

pub mod cmd;
pub mod config;
pub mod error;
pub mod event;
#[cfg(feature = "native-engine")]
pub mod native;
pub mod stack;
pub mod workspace;

pub use config::ConfigValue;
pub use error::{Error, Result};
pub use event::EngineEvent;
#[cfg(feature = "native-engine")]
pub use native::NativeStack;
pub use stack::{DestroyResult, OutputValue, PreviewResult, RefreshResult, Stack, UpResult};
pub use workspace::{LocalWorkspace, PluginInfo, StackSummary, WhoAmIResult};
