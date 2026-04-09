# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Native Rust SDK for [Pulumi](https://www.pulumi.com/) infrastructure-as-code. Communicates with the Pulumi engine via gRPC to register cloud resources, invoke provider functions, and manage stack outputs. Programs built with this SDK are executed by the Pulumi CLI, which sets required environment variables and runs gRPC services.

## Repository Structure

This is a Cargo workspace. Crates:

- **`pulumi-core`** — Core SDK: gRPC client, `Output<T>`, resource/invoke builders, serde, logging.
- **`pulumi-macros`** — Proc-macro crate: `#[derive(Resource)]`, `#[derive(ComponentResource)]`, `#[derive(ProviderFunction)]`.
- **`pulumi`** — Public-facing crate that re-exports `pulumi-core`. Enable `macros` feature to get derive macros.
- **`pulumi-automation`** — Automation API: drive stack operations (`up`, `preview`, `destroy`, `refresh`) programmatically by wrapping the Pulumi CLI. Enable the `native-engine` feature to use the Rust-native engine instead.
- **`pulumi-engine`** — Rust-native Pulumi engine: implements the ResourceMonitor and Engine gRPC servers. Used as an alternative to the Go-based `pulumi` CLI engine.
- **`pulumi-cli`** — Rust-native CLI for Pulumi stack operations, built on `pulumi-automation`.

## Build Commands

```bash
cargo build          # Build the workspace (includes protobuf compilation via build.rs)
cargo test           # Run all tests (async tests using #[tokio::test])
cargo clippy         # Lint
cargo test <name>    # Run a single test by name
```

Protobuf client and server stubs are generated at build time by `build.rs` using `tonic-build` (`build_server(true)`). Requires `protoc` on the system PATH. Generated types are accessed via `crate::proto::pulumirpc`. Server stubs are used by `pulumi-engine` to implement the ResourceMonitor and Engine gRPC services.

## Architecture

The SDK entry point is `pulumi_core::run(program)` in `pulumi-core/src/lib.rs`, which:
1. Reads engine connection settings from `PULUMI_*` environment variables (`context.rs`)
2. Establishes gRPC connections to the Pulumi engine and resource monitor
3. Registers the root stack resource (`stack.rs`)
4. Executes the user's async program function
5. Exports stack outputs

### Key modules (in `pulumi-core/src/`)

- **`connection.rs`** — Trait abstractions (`MonitorConnection`, `EngineConnection`) over the gRPC connections. Uses RPITIT (return-position `impl Trait` in traits) for zero-cost async dispatch. Provides `GrpcMonitor`/`GrpcEngine` (real gRPC) and `MockMonitor`/`MockEngine` (for testing).
- **`output.rs`** — `Output<T>`, the core Pulumi type representing potentially-unknown, potentially-secret async values. Wraps `Shared<BoxFuture<T>>` with dependency/secret/known metadata. Supports `map`, `flat_map`, `all`, `all2`, `all3` combinators. `OutputResolver` resolves or rejects pending outputs (auto-rejects on drop).
- **`context.rs`** — `Context<M, E>` is generic over `MonitorConnection` and `EngineConnection`. `Settings` parses all `PULUMI_*` env vars. Provides config access via `get_config()`/`require_config()`.
- **`resource.rs`** — `Resource` trait and `ResourceBuilder` for registering resources. Also exposes `ComponentBuilder`, `RemoteComponentBuilder`, `ReadBuilder`. All builders implement `IntoFuture` for `.await`.
- **`invoke.rs`** — `ProviderFunction` trait with `InvokeBuilder`; `ComponentMethod` trait with `CallBuilder`. Both implement `IntoFuture`.
- **`serde.rs`** — Bidirectional JSON ↔ Protobuf Struct conversion. Handles Pulumi wire format for secrets (magic key `4dabf18193072939515e22adb298388d`) and unknowns (sentinel UUID `04da6b54-80e4-46f7-96ec-b56ff0331ba9`).
- **`error.rs`** — `Error` enum with variants for transport, RPC, missing env, serde, resource failure, invoke failure, and custom errors.
- **`log.rs`** — `debug()`, `info()`, `warn()`, `error()`, `status()` send log messages to the Pulumi engine.
- **`stack.rs`** — Registers root stack resource (`pulumi:pulumi:Stack`) and exports outputs.
- **`stack_reference.rs`** — `StackReference` / `StackReferenceBuilder` for cross-stack references.
- **`transform.rs`** — `register_stack_transform()` for global resource transforms. Lazily starts a callback gRPC server.

### Proto definitions

Located in `pulumi-core/proto/pulumi/`. Key services: `ResourceMonitor` (resource.proto) for resource registration/invocation, `Engine` (engine.proto) for logging and root resource management.

### Key patterns

- **`ResourceBuilder` pattern**: Fluent API — `ResourceBuilder::<R>::new(&ctx, name, inputs)` → `.options()` → `.parent()` → `.provider()` → `.depends_on()` → `.await` returns `RegisteredResource<R>`.
- **`Output<T>` resolution**: Create with `Output::new()` which returns `(Output<T>, OutputResolver<T>)`. The resolver must be used to complete the output; dropping it without resolving triggers auto-rejection.
- **Connection traits**: `MonitorConnection` and `EngineConnection` abstract over gRPC clients, enabling mock implementations for testing without a running engine.
- **Tests**: Unit tests live in `#[cfg(test)] mod tests` within `output.rs` (15 async tests), `context.rs` (10 tests), and `serde.rs` (2 tests). All async tests use `#[tokio::test]`.

## Conventions

- Rust edition 2024; all public APIs are async (Tokio runtime)
- `Output<T>` implements `IntoFuture` so it can be `.await`ed directly
- Thread safety via `Arc<Mutex<>>` on shared state
- Doctests are disabled (`doctest = false` in Cargo.toml)
- Connection traits use RPITIT for zero-cost async dispatch (no boxing, no vtables)
- Nightly toolchain pinned in `rust-toolchain.toml` (`nightly-2026-03-03`)
