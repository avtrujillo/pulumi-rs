# SDK Review: `connection`

**Date:** 2026-05-09  
**Rust file:** `pulumi-core/src/connection.rs`  

---

## Summary

The Rust `connection.rs` module delivers a correct, well-tested foundation for the core resource-lifecycle RPCs (`register_resource`, `read_resource`, `invoke`, `call`) and a comprehensive `MockMonitor`/`MockEngine` with recording and error-injection capabilities. However, three gRPC methods introduced alongside newer engine features are entirely absent from the trait surface (`register_package`, `signal_and_wait_for_shutdown`, `register_invoke_transform`), the default tonic message-size limit is 96× smaller than every other SDK (a production correctness bug), and the mock cannot simulate different feature-support levels — making backwards-compatibility testing impossible.

---

## Missing Features

### 1. `MonitorConnection::register_package` — parameterized providers
Go's `Context.RegisterPackage` / `GetOrRegisterPackageRef` calls `monitor.RegisterPackage(ctx, in)`. The entire `RegisterPackage` RPC is absent from the Rust trait. Any code that uses parameterized providers (i.e., dynamically-typed provider SDKs) cannot work.

Go equivalent:
```go
func (ctx *Context) RegisterPackage(in *pulumirpc.RegisterPackageRequest) (*pulumirpc.RegisterPackageResponse, error) {
    if !ctx.state.supportsParameterization { … }
    return ctx.state.monitor.RegisterPackage(ctx.ctx, in)
}
```
Rust has no `register_package` method in `MonitorConnection` or `GrpcMonitor`.

### 2. `MonitorConnection::signal_and_wait_for_shutdown` — delete hooks
Node.js calls `monitorRef.signalAndWaitForShutdown(new emptyproto.Empty(), …)` inside `signalAndWaitForShutdown()` before the process exits when resource hooks are enabled. Without this RPC, delete-phase resource hooks never fire. The Rust trait has no equivalent.

### 3. `MonitorConnection::register_invoke_transform` — invoke transforms
Go has a distinct `supportsInvokeTransforms` feature flag and `registerInvokeTransform()` method (line ~230 of context.go). Rust's `register_stack_transform` covers only resource transforms. The two are separate RPCs in the proto and guarded by separate feature flags (`"transforms"` vs. `"invokeTransforms"`). Invoke transforms cannot be registered at all from Rust.

### 4. `EngineConnection::require_pulumi_version` — version gating
Node.js exposes `requirePulumiVersion(range: string)`, which sends `RequirePulumiVersionRequest` to the engine to enforce minimum CLI version. Generated provider SDKs use this to fail-fast on incompatible CLI versions. Absent from `EngineConnection` and `GrpcEngine`.

### 5. gRPC channel options — 400 MB message size and server timeout
Node.js:
```typescript
export const grpcChannelOptions: grpc.ChannelOptions = {
    "grpc.max_receive_message_length": 1024 * 1024 * 400,    // 400 MB
    "grpc.server_max_unrequested_time_in_server": 30 * 60,   // 1800 s
};
```
Python:
```python
_MAX_RPC_MESSAGE_SIZE = 1024 * 1024 * 400
_SERVER_MAX_UNREQUESTED_TIME_IN_SERVER = 30 * 60
```
`GrpcMonitor` and `GrpcEngine` use tonic's default `4 MB` decode limit. Large Kubernetes manifests, large Terraform state blobs, or provider schemas routinely exceed 4 MB, causing silent decode failures. This is the most critical correctness gap. The fix is to call `.max_decoding_message_size(400 * 1024 * 1024)` on the tonic client builders.

### 6. OpenTelemetry trace context propagation
Node.js injects W3C `traceparent`/`tracestate` headers into every outgoing gRPC call via `createTraceContextInterceptor()`. Rust has no tracing interceptor on the channel. Distributed tracing across the engine/SDK boundary is invisible.

### 7. Feature flag caching at connection startup
Go calls all `supportsFeature` queries eagerly during `NewContext` and stores results as boolean fields on `contextState`. Node.js's `awaitFeatureSupport()` does the same. Rust exposes a raw `supports_feature(&str)` call but has no caching layer — every SDK operation that needs to check a feature flag pays an extra gRPC round-trip.

### 8. `MonitorConnection::stream_invoke` — streaming invokes
The `ResourceMonitor` proto includes `StreamInvoke`. Absent from the Rust trait.

---

## Behavioral Divergences

### 1. Mock resource ID scheme differs from Go
Rust generates `"mock-id-{name}"`. Go's `mockMonitor.NewResource` returns `t + "::" + name` as the default ID (e.g., `"aws:s3/bucket:Bucket::my-bucket"`). Programs that snapshot-test URNs or IDs against Go-generated values will mismatch.

### 2. `protect` field loses three-state optionality
In the proto, `protect` is `optional bool`. Go preserves this as `*bool`. Rust's `ResourceRegistration.protect: bool` collapses `Some(false)` and `None` into the same `false`, making it impossible for tests to distinguish "explicitly not protected" from "protection not specified."

### 3. Alias collection silently drops `Alias::Spec_` variants
In `MockMonitor::register_resource`, alias collection:
```rust
.filter_map(|a| match a.alias.as_ref() {
    Some(pulumirpc::alias::Alias::Urn(urn)) => Some(urn.clone()),
    _ => None,                          // ← Spec_ aliases discarded
})
```
Go's `makeResourceState` handles both `Alias_Urn` and `Alias_Spec_` variants. Any alias created with the structured spec form (name/type/parent) is silently dropped from `ResourceRegistration.aliases`.

### 4. `MockMonitor::supports_feature` cannot be configured
`supports_feature` unconditionally returns `true`. Go's `contextState` stores per-flag booleans derived from real or mock feature queries. Rust has no way to make the mock return `false` for `"aliasSpecs"`, `"transforms"`, etc., to exercise backwards-compatibility code paths.

### 5. Preview semantics are narrower in scope
Rust's `preview: bool` in `MockMonitorOptions` only affects `register_resource` response bodies (returns empty `Struct`). In Go, `DryRun()` is propagated through the entire context and affects output resolution (`keepUnknowns` in `resourceState.resolve`), ID computation, and `InvokeOutput` dependency handling. Rust's mock has no `DryRun` signal that flows into `Output<T>` resolution.

### 6. RPC error wrapping loses call-site context
Go wraps: `fmt.Errorf("connecting to resource monitor over RPC: %w", err)`. Rust propagates `tonic::Status` directly into `Error::Transport` via `?`. A tonic `UNAVAILABLE` from `register_resource` and from `invoke` are indistinguishable in the error value.

### 7. No RPC drain / connection close
Go's `ctx.wait()` drains outstanding RPCs before shutdown using an `rpcs` counter + condition variable. Node.js has `waitForRPCs()` + `disconnectSync()`. Rust has no drain mechanism; calling code cannot wait for in-flight RPCs before dropping the connection, risking partial writes to the engine.

---

## API Surface Gaps

### `MonitorConnection` trait
| Missing method | Used in |
|---|---|
| `register_package` | Go `RegisterPackage`, parameterized providers |
| `signal_and_wait_for_shutdown` | Node.js `signalAndWaitForShutdown`, delete hooks |
| `register_invoke_transform` | Go `registerInvokeTransform`, invoke transforms |
| `stream_invoke` | proto `StreamInvoke` |

### `EngineConnection` trait
| Missing method | Used in |
|---|---|
| `require_pulumi_version` | Node.js `requirePulumiVersion` |

### `ResourceRegistration` struct — missing captured fields
```rust
// Fields present in RegisterResourceRequest proto but absent from ResourceRegistration:
hide_diffs: Vec<String>,
deleted_with: String,
replace_with: String,
hooks: Option<...>,
env_var_mappings: HashMap<String, String>,
replacement_trigger: Option<serde_json::Value>,
source_position: Option<...>,
stack_trace: Option<...>,
package_ref: String,
transforms: Vec<...>,     // per-resource transforms
protect: Option<bool>,    // should be optional, not bool
```

### `MockMonitorOptions` struct — missing configuration fields
```rust
// Needed:
features: HashSet<String>,   // controls supports_feature() return value
// Dynamic response hooks:
on_register_resource: Option<Box<dyn Fn(&RegisterResourceRequest) -> Result<serde_json::Value> + Send + Sync>>,
on_invoke:            Option<Box<dyn Fn(&ResourceInvokeRequest)   -> Result<serde_json::Value> + Send + Sync>>,
```

### Connection lifecycle
- `GrpcMonitor` / `GrpcEngine`: no `close()` method, no `Drop` impl — channel resources leak.
- No `rpc_keep_alive()` / drain primitive equivalent to Go's `beginRPC`/`endRPC`.

### Detectability of mock execution
- No `running_with_mocks() -> bool` equivalent (Go: `ctx.RunningWithMocks()`).

---

## Recommendations

**Priority 1 — Correctness bugs (fix before any production use)**

1. **Set 400 MB gRPC message decode limit on `GrpcMonitor` and `GrpcEngine`.** Call `.max_decoding_message_size(400 * 1024 * 1024)` on the tonic client builder in `GrpcMonitor::new` and `GrpcEngine::new`. This is a one-liner with immediate production impact.

2. **Add `features: HashSet<String>` to `MockMonitorOptions`** and make `MockMonitor::supports_feature` check it (falling back to `true` when the set is empty for backwards compat). Without this, every backwards-compatibility code path is untestable.

**Priority 2 — Missing RPCs blocking important features**

3. **Add `register_package` to `MonitorConnection` + `GrpcMonitor`.** Required for parameterized provider support. Should be paired with a `GetOrRegisterPackageRef`-style caching helper in `context.rs` mirroring Go's `gsync.Map` + `sync.Once` pattern.

4. **Add `signal_and_wait_for_shutdown` to `MonitorConnection` + `GrpcMonitor`.** Wire it into the shutdown path in `pulumi_core::run`. Without it, resource delete hooks are broken for any program using them.

5. **Add `register_invoke_transform` to `MonitorConnection` + `GrpcMonitor`.** Currently `register_stack_transform` is used for both, which is incorrect at the proto level and will fail against engines that check the feature flag.

**Priority 3 — API surface completeness**

6. **Add `require_pulumi_version` to `EngineConnection` + `GrpcEngine`.** Required for generated SDKs to perform version gating.

7. **Fix `protect: Option<bool>` in `ResourceRegistration`** — preserve three-state proto semantics. Low-effort, required for accurate test assertions.

8. **Fix alias collection in `MockMonitor::register_resource`** — handle `Alias::Spec_` variants instead of silently discarding them.

9. **Add missing fields to `ResourceRegistration`** — at minimum `hide_diffs`, `deleted_with`, `replace_with`, `package_ref`, and `hooks`. These are testable options that users will want to assert on.

10. **Add `close()` to both traits (or implement `Drop` on the `Grpc*` structs).** Prevents channel leaks. Can be a default no-op in the trait for mocks.

---

## Verdict

**SIGNIFICANT GAPS**

The core CRUD operations and mock infrastructure are solid. However, three missing monitor RPCs (`register_package`, `signal_and_wait_for_shutdown`, `register_invoke_transform`) block real engine features; the 4 MB gRPC message size is a production correctness defect; and the mock cannot simulate different engine capability levels, making backwards-compatibility testing structurally impossible.