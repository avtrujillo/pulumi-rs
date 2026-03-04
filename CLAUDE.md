# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Native Rust SDK for [Pulumi](https://www.pulumi.com/) infrastructure-as-code. Communicates with the Pulumi engine via gRPC to register cloud resources, invoke provider functions, and manage stack outputs. Programs built with this SDK are executed by the Pulumi CLI, which sets required environment variables and runs gRPC services.

## Build Commands

```bash
cargo build          # Build the project (includes protobuf compilation via build.rs)
cargo test           # Run all tests (async tests using #[tokio::test])
cargo clippy         # Lint
cargo test <name>    # Run a single test by name
```

Protobuf client stubs are generated at build time by `build.rs` using `tonic-build`. No server code is generated.

## Architecture

The SDK entry point is `pulumi::run(program)` in `lib.rs`, which:
1. Reads engine connection settings from `PULUMI_*` environment variables (`context.rs`)
2. Establishes gRPC connections to the Pulumi engine and resource monitor
3. Registers the root stack resource (`stack.rs`)
4. Executes the user's async program function
5. Exports stack outputs

### Key modules

- **`output.rs`** — `Output<T>`, the core Pulumi type representing potentially-unknown, potentially-secret async values. Wraps `Shared<BoxFuture<T>>` with dependency/secret/known metadata. Supports `map`, `flat_map`, `all`, `all2`, `all3` combinators. `OutputResolver` resolves or rejects pending outputs (auto-rejects on drop).
- **`context.rs`** — `Context` holds gRPC clients (`ResourceMonitorClient`, `EngineClient`) behind `Arc<Mutex<>>`. `Settings` parses all `PULUMI_*` env vars. Provides config access via `get_config()`/`require_config()`.
- **`resource.rs`** — `CustomResource` builder for registering resources. Also exposes `register_component_resource()`, `register_remote_component()`, `read_resource()`, `register_resource_outputs()`. Registration returns `(Output<urn>, Output<id>, Output<outputs>)`.
- **`invoke.rs`** — `invoke()` calls read-only provider functions; `call()` invokes component methods with dependency tracking.
- **`serde.rs`** — Bidirectional JSON ↔ Protobuf Struct conversion. Handles Pulumi wire format for secrets (magic key `4dabf18193072939515e22adb298388d`) and unknowns (sentinel UUID `04da6b54-80e4-46f7-96ec-b56ff0331ba9`).
- **`error.rs`** — `Error` enum with variants for transport, RPC, missing env, serde, resource failure, invoke failure, and custom errors.
- **`log.rs`** — `debug()`, `info()`, `warn()`, `error()`, `status()` send log messages to the Pulumi engine.
- **`stack.rs`** — Registers root stack resource (`pulumi:pulumi:Stack`) and exports outputs.

### Proto definitions

Located in `proto/pulumi/`. Key services: `ResourceMonitor` (resource.proto) for resource registration/invocation, `Engine` (engine.proto) for logging and root resource management.

## Conventions

- Rust edition 2024; all public APIs are async (Tokio runtime)
- `Output<T>` implements `IntoFuture` so it can be `.await`ed directly
- Thread safety via `Arc<Mutex<>>` on shared state
- Doctests are disabled (`doctest = false` in Cargo.toml)
