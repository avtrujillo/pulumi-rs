# pulumi-engine

Rust-native Pulumi engine. Implements the ResourceMonitor and Engine gRPC *servers* that a Pulumi program (built with `pulumi-core`) connects to as a client. This replaces the Go-based `pulumi` CLI engine for the resource registration lifecycle.

## Build

```bash
cargo build -p pulumi-engine    # Requires protoc on PATH (needed by pulumi-core)
cargo test -p pulumi-engine     # 85 unit tests, ~20s in a debug build (see Tests below)
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
  +-- For each custom resource, call provider.Read to get live state
  +-- Diff stored outputs vs live outputs (detect drift + deletions)
  +-- Remove deleted resources, update drifted state
  +-- Save checkpoint
  +-- Return RefreshResult with per-resource RefreshDiff details
```

## Module Guide

| Module | Purpose |
|--------|---------|
| `orchestrator.rs` | `PulumiEngine` with `up()`, `preview()`, `destroy()`, `refresh()`. `EngineOptions` configures project/stack/program/checkpoint path. |
| `engine_service.rs` | Implements the `Engine` gRPC service: `Log` (prints to stderr and emits a `Diagnostic` event), `GetRootResource`, `SetRootResource`, `StartDebugging` (no-op), `RequirePulumiVersion` (accepts any). Split into handlers and shims (see below). |
| `monitor_service.rs` | Implements the `ResourceMonitor` gRPC service. `RegisterResource` runs stack transforms, then diffs against prior state to determine create/update/same. `RegisterResourceOutputs` captures stack outputs. `Invoke`/`Call`/`ReadResource` delegate to real provider plugins via `ProviderManager`. Split into handlers and shims (see below). |
| `events.rs` | `EngineEvent` (`Prelude`, `ResourceStep`, `Diagnostic`, `Summary`) and the shared `EventCollector` (`Arc<Mutex<Vec<EngineEvent>>>`) that the orchestrator and both gRPC services write to. `pulumi-automation`'s `NativeStack` converts these into its own event type. |
| `diff.rs` | `diff_resource()` compares new inputs against prior `ResourceState`. Returns `ResourceAction` (Create/Update/Same). Supports `ignore_changes`. `diff_refresh()` compares stored outputs against live provider outputs; returns `RefreshAction` (Same/Updated/Deleted) with changed keys. 10 unit tests. |
| `state.rs` | `EngineState` — shared state behind `Arc<Mutex<>>`. Tracks current + prior resources, root URN, stack outputs. `Checkpoint` for JSON serialization to disk. `ResourceState` has custom `Debug` that redacts secret properties. 7 tests. |
| `secrets.rs` | `SecretsManager` trait (RPITIT) and `PassphraseSecretsManager`. AES-256-GCM encryption with PBKDF2-HMAC-SHA256 key derivation. Recursive JSON tree walkers for encrypting/decrypting secret-wrapped values. 12 tests. |
| `provider.rs` | `Provider` trait for abstracting over provider implementations (RPITIT). `GrpcProvider` launches real provider plugins as subprocesses and communicates via gRPC. `ProviderManager<P>` caches provider instances by package name. |
| `error.rs` | `Error` enum: `Transport`, `ProviderStatus`, `ProgramFailed`, `Spawn`, `Secrets`, `Custom`. `ProviderStatus` holds `Box<tonic::Status>` (a bare `Status` is 176 bytes); `From<tonic::Status>` boxes it, so `?` works unchanged. |
| `test_utils.rs` | Test-only (`#[cfg(test)]`). `MockProvider` (records deletes, canned CRUD responses), `TestEngine` (both gRPC servers on ephemeral ports with tonic clients, for transport-level tests), and `state_with_prior()` (an `EngineState` pre-populated with prior-run resources). |

## Handlers and shims

In `monitor_service.rs` and `engine_service.rs`, each RPC is two functions:

- A **handler** (`pub(crate) async fn handle_*`) on the inherent impl. It takes plain prost types and returns `Result<T, Status>`, and holds all the logic.
- A **tonic trait shim**: a few lines that unwrap `Request<T>`, call the handler, and wrap the result in `Response<T>`.

Put new logic in handlers and test them by calling `handle_*` directly. Use `TestEngine` only when the test is about transport: wire encoding, status-code propagation, or shim wiring.

Handlers return `tonic::Status` directly because tonic's service traits require it. `Status` is over clippy's `result_large_err` limit, so the workspace `clippy.toml` exempts that one type; do not box `Status` in handler signatures and do not add a crate-wide `allow`.

## Tests

85 unit tests: `monitor_service.rs` (26), `orchestrator.rs` (18), `secrets.rs` (12), `diff.rs` (10), `engine_service.rs` (10), `state.rs` (7), `provider.rs` (2).

Three orchestrator tests spawn `/bin/true` or `/bin/false` as the "program" to exercise the full `up` / `preview` lifecycle, and four tests go through real gRPC via `TestEngine`. The suite takes about 20 seconds in a debug build.

## State persistence

State is saved as a JSON `Checkpoint` to `<work_dir>/.pulumi-rs/<stack>.json` (configurable via `EngineOptions::checkpoint_path`). The checkpoint contains:
- All registered resources (URN, ID, type, inputs, outputs, dependencies)
- Stack outputs
- Project/stack metadata

On subsequent `up()` calls, prior state is loaded and used for diffing. On `destroy()`, the checkpoint is cleared. Preview does not persist state.

## Secret encryption

When `PULUMI_CONFIG_PASSPHRASE` is set (or `EngineOptions::secrets_manager` is provided), secret values in the checkpoint are encrypted at rest using AES-256-GCM with PBKDF2-HMAC-SHA256 key derivation (1M iterations). The ciphertext format (`v1:` + base64(nonce || ciphertext || tag)) is compatible with the Go Pulumi SDK.

The `SecretsManager` trait uses RPITIT, matching the `Provider` and `MonitorConnection` patterns. `PassphraseSecretsManager` implements it. Salt is persisted in the checkpoint's `secrets_provider` field so the same key is derived across runs.

Secret property names are tracked per-resource in `ResourceState::secret_properties` and `ResourceState`'s `Debug` impl redacts their values.

### Security audit notes

The following behaviors were reviewed and confirmed to match the Go Pulumi engine:

| Concern | Risk | Rationale |
|---------|------|-----------|
| `engine_service.rs` prints user log messages via `eprintln!` | Negligible | By design — user controls log content, same as Go engine |
| `Error::ProgramFailed` includes child stderr | Very low | SDK logs go through gRPC, not child stderr; only direct user `eprintln!` would appear |
| Provider operations receive secret-wrapped values | None | By design — providers need actual values to create cloud resources; secrets stay wrapped in the magic-key wire format with `accept_secrets: true` |
| `UpResult.stdout`/`stderr` returned to caller | Very low | Returned to user's own automation code; SDK never writes secrets to stdio |
| gRPC is HTTP (no TLS) on localhost | Very low | Bound to `127.0.0.1` only; attacker needs local root, at which point machine is already compromised |

## Current limitations (TODOs)

- **Invoke transforms**: `RegisterStackInvokeTransform` is accepted but not yet executed (resource transforms are done; invoke-level transforms are a follow-up).

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
- **aes-gcm** — AES-256-GCM authenticated encryption for secrets
- **pbkdf2 / sha2 / hmac** — PBKDF2-HMAC-SHA256 key derivation
- **base64** — ciphertext and salt encoding
- **rand** — nonce and salt generation
