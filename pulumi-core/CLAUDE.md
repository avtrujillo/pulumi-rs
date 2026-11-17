# pulumi-core

Core SDK crate for the Pulumi Rust SDK. Contains the gRPC client, `Output<T>` type, resource/invoke builders, serialization, and logging.

## Build

```bash
cargo build -p pulumi-core   # Requires protoc on PATH
cargo test -p pulumi-core     # 160 unit tests (see Tests below)
cargo clippy -p pulumi-core
```

Protobuf client and server stubs are generated at build time by `build.rs` using `tonic-build` (`build_server(true)`) from 4 proto files (`resource.proto`, `engine.proto`, `provider.proto`, `callback.proto`). Server stubs are used by `pulumi-engine`. Generated types are accessed via `crate::proto::pulumirpc`.

## Key Dependencies

- **tokio** — async runtime (multi-thread, macros, sync, time)
- **tonic / prost** — gRPC client and protobuf serialization
- **serde / serde_json** — JSON handling
- **futures** — future combinators

## Module Guide

| Module | Purpose |
|--------|---------|
| `lib.rs` | Entry point `run(program)`: connects to engine, registers stack, executes program, exports outputs. Uses `#![feature(impl_trait_in_assoc_type)]`. |
| `connection.rs` | Trait abstractions over gRPC connections. `MonitorConnection` (register_resource, invoke, call, etc.) and `EngineConnection` (log, get/set_root_resource, etc.). Concrete impls: `GrpcMonitor`/`GrpcEngine` for real gRPC, `MockMonitor`/`MockEngine` for testing. Uses RPITIT for zero-cost async dispatch. |
| `output.rs` | `Output<T>` — async value wrapping a `Shared<BoxFuture>` that resolves to the value *together with* its metadata (known, secret, dependency URNs), so combinators merge metadata as they produce the value. `is_known()`, `is_secret()` and `dependencies()` are async and wait for resolution. `Output::unknown()` has no value: combinators skip their callback and stay unknown; awaiting it is an error. `OutputResolver` completes pending outputs (auto-rejects on drop); its `add_deps`/`set_secret` take `&mut self` and apply at resolution. Combinators: `map`, `flat_map`, `all`, `all2`, `all3`, `from_future`. |
| `context.rs` | `Context<M, E>` — generic over `MonitorConnection` and `EngineConnection`. `Settings` parses `PULUMI_*` env vars. Config accessors: `get_config()`, `require_config()`, typed variants for bool/int/float/object. |
| `resource.rs` | Traits: `Resource`, `ComponentResource`, `RemoteComponent`. Builders: `ResourceBuilder`, `ComponentBuilder`, `RemoteComponentBuilder`, `ReadBuilder` — all implement `IntoFuture` for `.await`. `ResourceOptions` for parent, provider, depends_on, protect, aliases, custom timeouts, etc. |
| `invoke.rs` | Traits: `ProviderFunction`, `ComponentMethod`. Builders: `InvokeBuilder`, `CallBuilder` — implement `IntoFuture`. |
| `serde.rs` | JSON <-> Protobuf Struct conversion (pub(crate)). Handles Pulumi wire format for secrets (magic key `4dabf18193072939515e22adb298388d`) and unknowns (sentinel UUID `04da6b54-80e4-46f7-96ec-b56ff0331ba9`). |
| `error.rs` | `Error` enum: `Transport`, `Rpc`, `MissingEnv`, `Serde`, `ResourceFailed`, `InvokeFailure`, `Custom`. `Rpc` holds `Box<tonic::Status>` (a bare `Status` is 176 bytes); `From<tonic::Status>` boxes it, so `?` works unchanged. |
| `asset.rs` | `Asset` (`Text`, `Uri`, `Path`) and `Archive` (`Assets`, `Uri`, `Path`) enums, serialized untagged. Re-exported at the crate root. |
| `test_support.rs` | `TestContextBuilder` / `TestContext` for unit-testing programs without an engine: canned responses and injected errors for resources, reads, invokes and calls; config and preview mode; accessors for everything that was registered, read, invoked, called or logged. |
| `log.rs` | `debug()`, `info()`, `warn()`, `error()`, `status()` — async logging to the Pulumi engine via gRPC. |
| `stack.rs` | `register_stack()` creates the root `pulumi:pulumi:Stack` resource. `export_outputs()` registers stack outputs. |
| `stack_reference.rs` | `StackReference` / `StackReferenceBuilder` for cross-stack references with typed output access. |
| `transform.rs` | `register_stack_transform()` for global resource transforms. Lazily starts a callback gRPC server on first registration. |

## Proto Files

Located in `proto/pulumi/`: `resource.proto`, `engine.proto`, `provider.proto`, `callback.proto`, `alias.proto`, `plugin.proto`, `source.proto`.

## Tests

160 unit tests in inline `#[cfg(test)]` modules:

| Module | Tests | Covers |
|--------|-------|--------|
| `output.rs` | 40 | resolution, combinators, rejection, and secret / unknown / dependency propagation (including outputs derived before the source resolved) |
| `connection.rs` | 37 | `MockMonitor` recording, canned responses, error injection; `MockEngine` |
| `test_support.rs` | 19 | `TestContextBuilder` responses, errors and accessors |
| `resource.rs` | 11 | builders, alias conversion |
| `stack_reference.rs` | 11 | output access, typed access, builder |
| `transform.rs` | 11 | proto ↔ `ResourceOptions` / alias conversion |
| `context.rs` | 10 | config parsing, typed accessors, secret detection |
| `log.rs` | 9 | severity mapping, log functions |
| `invoke.rs` | 6 | `InvokeBuilder`, `CallBuilder` |
| `serde.rs` | 6 | roundtrip, secret wrapping, secret / unknown detection |

All async tests use `#[tokio::test]`.

## Conventions

- Rust edition 2024; requires nightly for `impl_trait_in_assoc_type` (pinned to `nightly-2026-11-14` in the workspace `rust-toolchain.toml`)
- `Output<T>` implements `IntoFuture` so it can be `.await`ed directly
- Builder types also implement `IntoFuture` for ergonomic registration
- Thread safety via `Arc<Mutex<>>` on shared state (mocks, transforms); `Output<T>` itself holds no locks
- Never read an output's known/secret/deps from side state — they only exist once the output resolves
- No crate-wide `#![allow(clippy::result_large_err)]`: keep `Error` small by boxing large payloads instead
- Doctests disabled (`doctest = false`)
