# pulumi-engine

Rust-native Pulumi engine. Implements the ResourceMonitor and Engine gRPC *servers* that a Pulumi program (built with `pulumi-core`) connects to as a client. This replaces the Go-based `pulumi` CLI engine for the resource registration lifecycle.

## Build

```bash
cargo build -p pulumi-engine    # Requires protoc on PATH
```

Protobuf server stubs are generated at build time by `build.rs` using `tonic-build` (client stubs are disabled). Proto files are symlinked from `pulumi-core/proto/`.

Doctests are disabled (`doctest = false`).

## How it works

```
PulumiEngine::up()
  |
  +-- Start ResourceMonitor gRPC server on 127.0.0.1:<ephemeral>
  +-- Start Engine gRPC server on 127.0.0.1:<ephemeral>
  |
  +-- Spawn user program as subprocess with PULUMI_* env vars:
  |     PULUMI_MONITOR=127.0.0.1:<port>
  |     PULUMI_ENGINE=127.0.0.1:<port>
  |     PULUMI_PROJECT=<project>
  |     PULUMI_STACK=<stack>
  |     PULUMI_DRY_RUN=true|false
  |
  +-- Handle gRPC calls from the program (resource registration, logging, etc.)
  +-- Collect stack outputs from RegisterResourceOutputs
  +-- Return UpResult when the program exits
```

## Module Guide

| Module | Purpose |
|--------|---------|
| `orchestrator.rs` | `PulumiEngine` — starts gRPC servers, spawns user program, collects results. `EngineOptions` configures project/stack/program. |
| `engine_service.rs` | Implements the `Engine` gRPC service: `Log` (prints to stderr), `GetRootResource`, `SetRootResource`, `StartDebugging` (no-op), `RequirePulumiVersion` (accepts any). |
| `monitor_service.rs` | Implements the `ResourceMonitor` gRPC service: `RegisterResource` (assigns URN, synthetic ID), `RegisterResourceOutputs` (captures stack outputs), `SupportsFeature`, `Invoke`/`Call` (stubs), `ReadResource`, `RegisterPackage`, `SignalAndWaitForShutdown`. |
| `state.rs` | `EngineState` — in-memory state behind `Arc<Mutex<>>`. Tracks registered resources, root URN, stack outputs. Generates URNs. |
| `error.rs` | `Error` enum: `Transport`, `ProgramFailed`, `Spawn`, `Custom`. |

## Current limitations (TODOs)

- **No provider plugins**: `RegisterResource` assigns synthetic IDs instead of calling real providers. `Invoke` and `Call` return empty results.
- **No state persistence**: All state is in-memory; no checkpoint/snapshot files.
- **No diff/update planning**: Every run is a fresh creation, no update or delete logic.
- **No secret encryption**: Secrets are not encrypted in state.
- **No transforms**: `RegisterStackTransform` is accepted but ignored.
- **No destroy**: Only `up` and `preview` are implemented.

## Integration with pulumi-automation

Enable the `native-engine` feature on `pulumi-automation` to get `NativeStack`, which uses this engine instead of the CLI:

```rust
let ws = LocalWorkspace::new("./my-project");
let stack = ws.native_stack("dev", vec!["./target/release/my-program".into()]);
let result = stack.up().await?;
```

## Dependencies

- **tokio** — async runtime + subprocess spawning
- **tonic** — gRPC server framework
- **prost / prost-types** — protobuf types
- **serde / serde_json** — JSON handling for state
- **tokio-stream** — `TcpListenerStream` for tonic's `serve_with_incoming`
