# SDK Review: `transform`

**Date:** 2026-05-06  
**Rust file:** `pulumi-core/src/transform.rs`  

---

## Summary

The Rust `transform.rs` implements only the narrowest slice of what the upstream SDKs provide: a synchronous-only stack-level resource transform with several incomplete field mappings. It is missing the entire invoke-transform surface, both hook systems (resource and error), async transform support, the pass-through semantic shared by all other SDKs, and key `ResourceOptions` fields. This is the most under-specified module in the SDK.

---

## Missing Features

### 1. Invoke Transforms — Completely Absent
All three upstream SDKs provide a parallel transform pipeline for provider function invocations.

| SDK | Types | Registration Function |
|---|---|---|
| Go | `InvokeTransformArgs`, `InvokeTransformResult`, `InvokeTransform` | (via resource options) |
| Node.js | `InvokeTransformArgs`, `InvokeTransform` | `registerStackInvokeTransform()`, `registerStackInvokeTransformAsync()` |
| Python | `InvokeTransformArgs`, `InvokeTransformResult`, `InvokeTransform` | `register_invoke_transform()`, `do_register_invoke_transform()` |
| **Rust** | *(none)* | *(none)* |

The proto side (`TransformInvokeRequest` / `TransformInvokeResponse` / `TransformInvokeOptions`) already exists in the Pulumi protobuf definitions. The gRPC call `registerStackInvokeTransform` on the monitor is also already present. This is a complete functional gap.

### 2. Resource Hooks — Completely Absent
Node.js exposes `registerResourceHook(hook: ResourceHook)` and Python exposes `register_resource_hook(hook: ResourceHook)`. These fire callbacks after resource state is known (with `urn`, `id`, `newInputs`, `oldInputs`, `newOutputs`, `oldOutputs`). The `RegisterResourceHook` RPC and `ResourceHookRequest` / `ResourceHookResponse` proto messages are both absent from the Rust implementation.

### 3. Error Hooks — Completely Absent
Node.js exposes `registerErrorHook(hook: ErrorHook)` and Python exposes `register_error_hook(hook: ErrorHook)`. These fire on resource operation failures and can signal retry (`bool` return). No equivalent `ErrorHook`, `ErrorHookArgs`, or `register_error_hook` exists in Rust.

### 4. Async Transforms Not Supported
```rust
// Rust (current) — synchronous only
pub type TransformFn = Arc<dyn Fn(TransformArgs) -> TransformResult + Send + Sync + 'static>;
```
Node.js treats all transform callbacks as `Promise`-returning. Python uses `isinstance(maybeAwaitable, Awaitable)` to support both sync and async. The Go SDK is synchronous but uses `context.Context`. The Rust type has no async provision; a user cannot perform async I/O (e.g., fetching a secret) inside a transform.

### 5. Per-Resource (Non-Stack) Transform Registration
Node.js exposes `registerTransform(callback: ResourceTransform): Promise<Callback>` as a lower-level primitive that returns the `Callback` proto for use in per-resource options. Python exposes `register_transform(transform)`. Rust only exposes `register_stack_transform`; there is no way to attach a transform to a single resource's options.

### 6. `awaitStackRegistrations()` Equivalent
Node.js tracks `_pendingRegistrations` and exposes `awaitStackRegistrations(): Promise<void>` so the engine can wait until all async stack-transform registrations complete before processing resources. Without this, resources registered early in the program might escape transforms that are registered concurrently. Rust has no equivalent mechanism.

---

## Behavioral Divergences

### 1. No Pass-Through Semantic (Critical)
Every upstream SDK allows a transform to opt out of modifying a resource by returning `nil` (Go), `undefined` (Node.js), or `None` (Python). In that case the original properties and options are preserved verbatim. Rust's `TransformFn` returns `TransformResult` unconditionally — there is no `Option<TransformResult>`. A Rust transform *must* return something, and whatever it returns will overwrite the original. If a transform author returns `TransformResult { props: args.props, opts: args.opts }` naively, they still go through the serialization/deserialization round-trip unnecessarily, and any proto fields not covered by `ResourceOptions` (see API Surface Gaps) will be silently dropped.

```go
// Go — nil means "do not transform"
type ResourceTransform func(context.Context, *ResourceTransformArgs) *ResourceTransformResult

// Rust — no way to opt out
pub type TransformFn = Arc<dyn Fn(TransformArgs) -> TransformResult + Send + Sync + 'static>;
```

**Fix:** Change the return type to `Option<TransformResult>` and short-circuit with the original request when `None` is returned.

### 2. Silent Data Loss on Round-Trip for Unhandled Proto Fields
`resource_options_to_proto_opts` hard-codes several fields to empty/`None`:

```rust
hooks: None,
replacement_trigger: None,
replace_with: Vec::new(),
hide_diff: Vec::new(),
plugin_checksums: HashMap::new(),
providers: HashMap::new(),
```

When a transform receives a `TransformRequest` from the engine for a resource that *already has* `replace_with`, `hooks`, or `replacement_trigger` set, `proto_opts_to_resource_options` parses the request but `ResourceOptions` has no fields for these values. They are read from the proto and immediately discarded. The transform then calls `resource_options_to_proto_opts` and emits empty values, **silently overwriting** data the engine had set. This is not a minor gap — it is data corruption.

Node.js and Python carefully preserve or re-encode every field including `replaceWith`, `hooks`, `replacementTrigger`, `hideDiff`, `pluginChecksums`, and `providers`.

### 3. Global Static Server Breaks Test Isolation
```rust
static CALLBACK_SERVER: Mutex<Option<ServerHandle>> = Mutex::const_new(None);
```
The server is a process-wide singleton. Node.js and Python scope the callback server to a monitor/context instance. In Rust, two `Context` instances (common in integration tests) share the same server and transform registry, causing token namespace collisions and cross-test contamination. Python's `_CallbackServicer._servicers` is also a class-level list but at least each servicer owns its own callbacks map.

### 4. Server-Ready Race Condition
```rust
let (started_tx, started_rx) = oneshot::channel::<()>();
tokio::spawn(async move {
    let _ = started_tx.send(());  // fires before serve_with_incoming even starts
    tonic::transport::Server::builder()
        .add_service(...)
        .serve_with_incoming(incoming)
        .await
        ...
});
let _ = started_rx.await;  // returns immediately, server may not be listening yet
```
The oneshot fires *before* `serve_with_incoming` begins, creating a race window where the callback address is advertised to the engine before the server is accepting connections. Node.js explicitly polls the server by calling `invoke` on itself until it gets a known-good error before resolving. Rust should bind the port *before* spawning and wait for actual readiness.

### 5. No Cleanup on Registration Failure
When `ctx.monitor().register_stack_transform(callback).await` fails, the transform closure remains in `state.transforms` forever, leaking memory and a spurious callback endpoint. Both Node.js and Python remove the transform from their registries on RPC failure:

```python
# Python
except:
    self._transforms.pop(transform)
    self._callbacks.pop(callback.token)
    raise
```

### 6. Deduplication Absent
Python deduplicates transforms by function identity (`self._transforms.get(transform)`) and returns the existing callback if the same function is registered twice. Rust assigns a new token on every call.

---

## API Surface Gaps

### Types Missing

| Upstream Name | Go | Node.js | Python | Rust |
|---|---|---|---|---|
| `InvokeTransformArgs` | ✅ | ✅ | ✅ | ❌ |
| `InvokeTransformResult` | ✅ | ✅ | ✅ | ❌ |
| `InvokeTransform` (fn type) | ✅ | ✅ | ✅ | ❌ |
| `ResourceHook` | — | ✅ | ✅ | ❌ |
| `ResourceHookArgs` | — | ✅ | ✅ | ❌ |
| `ErrorHook` | — | ✅ | ✅ | ❌ |
| `ErrorHookArgs` | — | ✅ | ✅ | ❌ |

`TransformFn` / `TransformArgs` / `TransformResult` drop the `Resource` prefix used by every other SDK (`ResourceTransform`, `ResourceTransformArgs`, `ResourceTransformResult`). This is a minor naming divergence but will cause friction when SDK users cross-reference documentation.

### Functions / Methods Missing

| Function | Node.js | Python | Rust |
|---|---|---|---|
| `register_stack_invoke_transform` | ✅ | ✅ | ❌ |
| `register_transform` (per-resource) | ✅ | ✅ | ❌ |
| `register_resource_hook` | ✅ | ✅ | ❌ |
| `register_error_hook` | ✅ | ✅ | ❌ |
| `awaitStackRegistrations` / drain | ✅ | — | ❌ |
| `shutdown` (callback server) | ✅ | ✅ | ❌ |

### `ResourceOptions` Fields Missing from Both Struct and Conversions

| Proto field | `TransformResourceOptions` present | In `ResourceOptions` struct | Handled in `proto_opts_to_resource_options` | Handled in `resource_options_to_proto_opts` |
|---|---|---|---|---|
| `replace_with` | ✅ | ❌ | ❌ | Hardcoded `Vec::new()` |
| `hooks` | ✅ | ❌ | ❌ | Hardcoded `None` |
| `replacement_trigger` | ✅ | ❌ | ❌ | Hardcoded `None` |
| `hide_diff` | ✅ | ❌ | ❌ | Hardcoded `Vec::new()` |
| `plugin_checksums` | ✅ | ❌ | ❌ | Hardcoded `HashMap::new()` |
| `providers` (component) | ✅ | ❌ | ❌ | Hardcoded `HashMap::new()` |

---

## Recommendations

**Priority 1 — Fix the data-corruption bug (P0, blocks correctness)**
Add `replace_with`, `hooks`, `replacement_trigger`, `hide_diff`, `plugin_checksums`, and `providers` to `ResourceOptions`. Update both conversion functions to round-trip these fields. The silent-drop behavior means existing users of other Pulumi features (e.g., `replace_with`) will have those options stripped when a transform runs.

**Priority 2 — Add `Option<TransformResult>` pass-through (P0, semantic correctness)**
Change `TransformFn` to `Arc<dyn Fn(TransformArgs) -> Option<TransformResult> + Send + Sync + 'static>` and short-circuit to the original request bytes when `None` is returned. This also eliminates the data-corruption risk for transforms that do not intend to modify anything.

**Priority 3 — Implement invoke transforms (P1, feature parity)**
Add `InvokeTransformArgs`, `InvokeTransformResult`, `InvokeTransformFn`, and `register_stack_invoke_transform`. The gRPC plumbing (`TransformInvokeRequest` / `TransformInvokeResponse`) already exists in the proto. The invoke callback handler follows exactly the same pattern as the resource one.

**Priority 4 — Async transform support (P1, usability)**
Change the callback type to return a `BoxFuture<'static, Option<TransformResult>>`. The `invoke` method on `CallbackService` is already async, so the only change is allowing the user-supplied function to be async. This aligns with Node.js (always async) and Python (detects awaitable).

**Priority 5 — Fix the server-ready race condition (P1, reliability)**
Move `started_tx.send(())` to *after* the server is listening, or do a self-probe as Node.js does.

**Priority 6 — Scope the callback server per-Context (P1, test isolation)**
Remove the `static CALLBACK_SERVER` and embed the server handle in `Context`. This matches Go/Node.js/Python behavior and fixes cross-test contamination.

**Priority 7 — Implement resource and error hooks (P2, feature parity)**
Add `ResourceHook`, `ResourceHookArgs`, `ErrorHook`, `ErrorHookArgs`, `register_resource_hook`, and `register_error_hook`. These are used by frameworks that need post-apply callbacks and retry semantics.

**Priority 8 — Cleanup on registration failure (P2, correctness)**
Remove the transform from `state.transforms` if the monitor RPC returns an error.

**Priority 9 — `awaitStackRegistrations` (P2, engine compatibility)**
Implement a draining mechanism so the SDK can wait for all in-flight `register_stack_transform` calls before allowing resource registration to proceed.

**Priority 10 — Rename types (P3, ergonomics)**
Rename `TransformArgs` → `ResourceTransformArgs`, `TransformResult` → `ResourceTransformResult`, `TransformFn` → `ResourceTransform` to match every other SDK and avoid ambiguity once invoke transforms share the namespace.

---

## Verdict

**MAJOR GAPS**

The module is missing two complete feature systems (invoke transforms, hooks), has a data-corruption bug for any resource options field not yet mapped through `ResourceOptions`, lacks the universal pass-through semantic, has a server-ready race condition, and uses a global static that breaks test isolation. The existing tests cover only the proto conversion helpers and do not exercise the gRPC server path at all.