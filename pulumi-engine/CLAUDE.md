# pulumi-engine

Rust-native Pulumi engine. Implements the ResourceMonitor and Engine gRPC *servers* that a Pulumi program (built with `pulumi-core`) connects to as a client. This replaces the Go-based `pulumi` CLI engine for the resource registration lifecycle.

## Build

```bash
cargo build -p pulumi-engine    # Requires protoc on PATH (needed by pulumi-core)
cargo test -p pulumi-engine     # 4 diff tests
```

Uses protobuf types and gRPC server traits from `pulumi_core::proto::pulumirpc` (the proto module is public). No separate proto compilation — `pulumi-core` generates both client and server stubs (`build_server(true)`).

Doctests are disabled (`doctest = false`).

## How it works

### `up` / `preview`

```
PulumiEngine::up()
  |
  +-- Load prior checkpoint from <work_dir>/.pulumi-rs/<stack>.json
  +-- Start ResourceMonitor + Engine gRPC servers on ephemeral ports
  +-- Spawn user program with PULUMI_* env vars
  +-- Handle gRPC calls: diff each RegisterResource against prior state
  +-- Detect deleted resources (in prior but not re-registered)
  +-- Save checkpoint to disk (skipped for preview/dry-run)
  +-- Return UpResult with stack outputs
```

### `destroy`

```
PulumiEngine::destroy()
  |
  +-- Load checkpoint
  +-- Walk resources in reverse order, log each deletion
  +-- Save empty checkpoint
  +-- Return DestroyResult
```

### `refresh`

```
PulumiEngine::refresh()
  |
  +-- Load checkpoint
  +-- Log each resource (TODO: call provider.Read to sync actual state)
  +-- Re-save checkpoint
```

## Module Guide

| Module | Purpose |
|--------|---------|
| `orchestrator.rs` | `PulumiEngine` with `up()`, `preview()`, `destroy()`, `refresh()`. `EngineOptions` configures project/stack/program/checkpoint path. |
| `engine_service.rs` | Implements the `Engine` gRPC service: `Log` (prints to stderr), `GetRootResource`, `SetRootResource`, `StartDebugging` (no-op), `RequirePulumiVersion` (accepts any). |
| `monitor_service.rs` | Implements the `ResourceMonitor` gRPC service. `RegisterResource` diffs against prior state to determine create/update/same. `RegisterResourceOutputs` captures stack outputs. `Invoke`/`Call` delegate to real provider plugins via `ProviderManager`. |
| `diff.rs` | `diff_resource()` compares new inputs against prior `ResourceState`. Returns `ResourceAction` (Create/Update/Same). Supports `ignore_changes`. 4 unit tests. |
| `state.rs` | `EngineState` — shared state behind `Arc<Mutex<>>`. Tracks current + prior resources, root URN, stack outputs. `Checkpoint` for JSON serialization to disk. |
| `provider.rs` | `Provider` trait for abstracting over provider implementations (RPITIT). `GrpcProvider` launches real provider plugins as subprocesses and communicates via gRPC. `ProviderManager<P>` caches provider instances by package name. |
| `error.rs` | `Error` enum: `Transport`, `ProviderStatus`, `ProgramFailed`, `Spawn`, `Custom`. |

## State persistence

State is saved as a JSON `Checkpoint` to `<work_dir>/.pulumi-rs/<stack>.json` (configurable via `EngineOptions::checkpoint_path`). The checkpoint contains:
- All registered resources (URN, ID, type, inputs, outputs, dependencies)
- Stack outputs
- Project/stack metadata

On subsequent `up()` calls, prior state is loaded and used for diffing. On `destroy()`, the checkpoint is cleared. Preview does not persist state.

## Current limitations (TODOs)

- **No secret encryption**: Secrets are not encrypted in the checkpoint.
- **No transforms**: `RegisterStackTransform` and `RegisterStackInvokeTransform` are accepted but ignored.
- **Partial refresh**: `refresh()` calls `provider.read()` for each custom resource, but state syncing is best-effort.

## Integration with pulumi-automation

Enable the `native-engine` feature on `pulumi-automation` to get `NativeStack`, which uses this engine instead of the CLI:

```rust
let ws = LocalWorkspace::new("./my-project");
let stack = ws.native_stack("dev", vec!["./target/release/my-program".into()]);
let result = stack.up().await?;      // create/update with checkpoint
stack.destroy().await?;               // tear down + clear checkpoint
```

## Dependencies

- **pulumi-core** — proto types and gRPC server traits (`pulumi_core::proto::pulumirpc`)
- **tokio** — async runtime + subprocess spawning
- **tonic** — gRPC server framework
- **prost-types** — protobuf well-known types (Struct, Value)
- **serde / serde_json** — JSON serialization for checkpoint and state
- **tokio-stream** — `TcpListenerStream` for tonic's `serve_with_incoming`
