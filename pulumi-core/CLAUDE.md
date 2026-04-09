# pulumi-core

Core SDK crate for the Pulumi Rust SDK. Contains the gRPC client, `Output<T>` type, resource/invoke builders, serialization, and logging.

## Build

```bash
cargo build -p pulumi-core   # Requires protoc on PATH
cargo test -p pulumi-core     # ~27 tests (15 output, 10 context, 2 serde)
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
| `output.rs` | `Output<T>` — async value wrapping `Shared<BoxFuture<T>>` with dependency/secret/known metadata. `OutputResolver` completes pending outputs (auto-rejects on drop). Combinators: `map`, `flat_map`, `all`, `all2`, `all3`, `from_future`. |
| `context.rs` | `Context<M, E>` — generic over `MonitorConnection` and `EngineConnection`. `Settings` parses `PULUMI_*` env vars. Config accessors: `get_config()`, `require_config()`, typed variants for bool/int/float/object. |
| `resource.rs` | Traits: `Resource`, `ComponentResource`, `RemoteComponent`. Builders: `ResourceBuilder`, `ComponentBuilder`, `RemoteComponentBuilder`, `ReadBuilder` — all implement `IntoFuture` for `.await`. `ResourceOptions` for parent, provider, depends_on, protect, aliases, custom timeouts, etc. |
| `invoke.rs` | Traits: `ProviderFunction`, `ComponentMethod`. Builders: `InvokeBuilder`, `CallBuilder` — implement `IntoFuture`. |
| `serde.rs` | JSON <-> Protobuf Struct conversion (pub(crate)). Handles Pulumi wire format for secrets (magic key `4dabf18193072939515e22adb298388d`) and unknowns (sentinel UUID `04da6b54-80e4-46f7-96ec-b56ff0331ba9`). |
| `error.rs` | `Error` enum: `Transport`, `Rpc`, `MissingEnv`, `Serde`, `ResourceFailed`, `InvokeFailure`, `Custom`. |
| `log.rs` | `debug()`, `info()`, `warn()`, `error()`, `status()` — async logging to the Pulumi engine via gRPC. |
| `stack.rs` | `register_stack()` creates the root `pulumi:pulumi:Stack` resource. `export_outputs()` registers stack outputs. |
| `stack_reference.rs` | `StackReference` / `StackReferenceBuilder` for cross-stack references with typed output access. |
| `transform.rs` | `register_stack_transform()` for global resource transforms. Lazily starts a callback gRPC server on first registration. |

## Proto Files

Located in `proto/pulumi/`: `resource.proto`, `engine.proto`, `provider.proto`, `callback.proto`, `alias.proto`, `plugin.proto`, `source.proto`.

## Tests

Inline `#[cfg(test)]` modules:
- `output.rs` — 15 tests (resolution, mapping, combining, secrets, rejection, clone)
- `context.rs` — 10 tests (config parsing, typed accessors, secret detection)
- `serde.rs` — 2 tests (roundtrip, secret wrapping)

All async tests use `#[tokio::test]`.

## Conventions

- Rust edition 2024; requires nightly for `impl_trait_in_assoc_type`
- `Output<T>` implements `IntoFuture` so it can be `.await`ed directly
- Builder types also implement `IntoFuture` for ergonomic registration
- Thread safety via `Arc<Mutex<>>` on shared state
- Doctests disabled (`doctest = false`)
