//! # Pulumi SDK for Rust
//!
//! This crate is the main entry point for building Pulumi programs in Rust.
//! It re-exports the contents of [`pulumi_core`] for convenience.
//!
//! ## Features
//!
//! - **`macros`** — Enables derive macros ([`Resource`](derive@Resource),
//!   [`ComponentResource`](derive@ComponentResource),
//!   [`ProviderFunction`](derive@ProviderFunction)) for reducing boilerplate.
//!
//! See [`pulumi_core`] for full API documentation.

pub use pulumi_core::*;

/// Derive the [`Resource`](pulumi_core::resource::Resource) trait for a custom cloud resource.
///
/// See [`pulumi_macros::Resource`] for usage and examples.
#[cfg(feature = "macros")]
pub use pulumi_macros::Resource;

/// Derive the [`ComponentResource`](pulumi_core::resource::ComponentResource) trait.
///
/// See [`pulumi_macros::ComponentResource`] for usage and examples.
#[cfg(feature = "macros")]
pub use pulumi_macros::ComponentResource;

/// Derive the [`ProviderFunction`](pulumi_core::invoke::ProviderFunction) trait.
///
/// See [`pulumi_macros::ProviderFunction`] for usage and examples.
#[cfg(feature = "macros")]
pub use pulumi_macros::ProviderFunction;
