//! # Pulumi Engine (Rust-native)
//!
//! A Rust reimplementation of the Pulumi engine. Runs the ResourceMonitor and
//! Engine gRPC services that a Pulumi program (built with `pulumi-core`)
//! connects to.
//!
//! ## Architecture
//!
//! The engine:
//! 1. Starts ResourceMonitor and Engine gRPC servers on ephemeral ports
//! 2. Spawns the user's Pulumi program as a subprocess, passing connection
//!    info via `PULUMI_*` environment variables
//! 3. Handles resource registrations, invocations, and logging
//! 4. Collects stack outputs when the program completes
//!
//! ## Usage
//!
//! ```ignore
//! use pulumi_engine::{PulumiEngine, EngineOptions};
//!
//! let engine = PulumiEngine::new(EngineOptions {
//!     project: "my-project".into(),
//!     stack: "dev".into(),
//!     work_dir: "./my-pulumi-project".into(),
//!     program: vec!["./target/release/my-program".into()],
//!     dry_run: false,
//!     ..Default::default()
//! });
//!
//! let result = engine.up().await?;
//! ```

pub mod engine_service;
pub mod error;
pub mod monitor_service;
pub mod orchestrator;
pub mod state;

pub(crate) use pulumi_core::proto::pulumirpc;

pub use error::{Error, Result};
pub use orchestrator::{EngineOptions, PulumiEngine, UpResult};
