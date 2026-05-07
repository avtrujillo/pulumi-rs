# SDK Review: `log`

**Date:** 2026-05-06  
**Rust file:** `pulumi-core/src/log.rs`  

---

## Summary

The Rust `log` module covers the basic four severity levels and correctly maps them to the protobuf wire format, but has concrete, API-breaking omissions relative to every upstream SDK. The most critical gaps are the missing `stream_id` parameter (silently zeroed out), the absence of `ephemeral` as a first-class flag on each function (it's only reachable via the `status()` shortcut at `Info` severity), and the substitution of a raw `&str` URN for an actual `Resource` object — which forces callers to break the ergonomic async chain. These are not minor style differences; they prevent parity with programs ported from Go or Python.

---

## Missing Features

### 1. `stream_id` parameter — hardcoded to `0`

Both Go (`LogArgs.StreamID int32`) and Python (`stream_id: Optional[int]`) expose stream IDs so that the Pulumi console can stitch related log lines into a single collapsible stream. The Rust implementation hardcodes `stream_id: 0` in `log_message()` and has no public parameter to override it:

```rust
// current — stream_id is invisible to callers
let req = pulumirpc::LogRequest {
    stream_id: 0,   // ← always zero
    ...
};
```

Any Rust program that ports logic which used stream grouping (e.g. a progress log stream in Python) will silently lose that association without error.

---

### 2. `ephemeral` not a first-class parameter

Go exposes `ephemeral` as a field on `LogArgs` for every call:

```go
ctx.Log.Warn("retrying…", &LogArgs{Ephemeral: true})
```

Python exposes it on every function:

```python
warn("retrying…", ephemeral=True)
```

Rust exposes it only through the special-cased `status()` function, which is hardwired to `Severity::Info`. It is **impossible** in the current Rust API to send an ephemeral `Warn` or ephemeral `Error` message — a real use-case (e.g. live-updating error counters during a preview) that works fine in all upstream SDKs.

---

### 3. `Resource` parameter replaced by raw URN string

Go and Python both accept a live `Resource` object and resolve its URN asynchronously inside the log function:

```go
// Go — URN resolved internally via awaitURN()
ctx.Log.Info("bucket created", &LogArgs{Resource: bucket})
```

```python
# Python — resolved via asyncio.ensure_future if resource != None
info("bucket created", resource=bucket)
```

The Rust API takes `urn: Option<&str>`, which means the caller must already know the final URN:

```rust
// Rust — caller must pre-resolve the URN
info(ctx, "bucket created", Some(&bucket.urn)).await?;
```

This breaks the ergonomic contract: a `Resource`'s URN is itself an `Output<String>`, so callers must add their own `.await` on the URN before they can log against a resource. Go resolves this inside `_log()` via `args.Resource.URN().awaitURN(log.ctx)`.

---

### 4. No `Log` trait / interface abstraction

Go defines a `Log` interface that decouples callers from the concrete `logState` implementation:

```go
type Log interface {
    Debug(msg string, args *LogArgs) error
    Info(msg string, args *LogArgs) error
    Warn(msg string, args *LogArgs) error
    Error(msg string, args *LogArgs) error
}
```

The Rust module has no equivalent trait. This makes it impossible to inject a mock logging backend in tests without using the full `MockEngine` apparatus, and prevents third-party crates from implementing alternative log sinks (e.g. a buffering adapter).

---

### 5. No `LogArgs` / options struct

Both Go (`LogArgs`) and (implicitly) Python (named keyword arguments) bundle optional parameters in a forward-compatible way. Adding a new optional field to `LogArgs` is non-breaking in Go. The Rust functions have flat positional signatures:

```rust
pub async fn info<M, E>(ctx: &Context<M, E>, message: &str, urn: Option<&str>) -> Result<()>
```

Adding `stream_id` or `ephemeral` to these signatures in a future PR will be **a breaking change** to all existing callers.

---

### 6. No engine-unavailable fallback

Python's `info()`, `warn()`, and `error()` degrade gracefully to `stderr` when no engine connection is available:

```python
engine = get_engine()
if engine is not None:
    _log(engine, ...)
else:
    print("info: " + msg, file=sys.stderr)  # offline / test fallback
```

The Rust implementation propagates the error back to the caller with no fallback. During unit tests or offline scenarios, callers must handle the error explicitly rather than getting the intuitive `stderr` output.

---

### 7. No in-flight operation tracking

The Go `logState._log()` integrates with a `workGroup` to ensure all log RPCs complete before the Pulumi program exits:

```go
if log.join != nil {
    log.join.Add(1)
    defer log.join.Done()
}
```

The Rust module has no equivalent. A fire-and-forget log call at the end of a program could be silently dropped if the runtime shuts down before the gRPC call completes.

---

## Behavioral Divergences

| Behavior | Go | Python | Rust |
|---|---|---|---|
| `stream_id` | Caller-supplied via `LogArgs` | Caller-supplied parameter | Always `0` |
| `ephemeral` | Per-call on any severity | Per-call on any severity | Only `Info` via `status()` |
| Resource attachment | `Resource` object, URN resolved internally | `Resource` object, URN resolved internally | Raw `&str` URN, pre-resolved by caller |
| UTF-8 sanitization | `strings.ToValidUTF8(msg, "")` | No explicit sanitization | No explicit sanitization (N/A for `&str`, but relevant for `String` sourced from FFI) |
| Missing engine | N/A (always injected) | Falls back to `stderr` | Propagates `Error` |
| `status()` function | Not present; use `&LogArgs{Ephemeral: true}` | Not present; use `ephemeral=True` | Dedicated function — `Info` + `ephemeral=true` only |

---

## API Surface Gaps

```rust
// MISSING: LogArgs / options struct for forward-compatible parameters
pub struct LogArgs<'a> {
    pub urn: Option<&'a str>,  // or: pub resource: Option<&'a dyn Resource>
    pub stream_id: i32,
    pub ephemeral: bool,
}

// MISSING: `stream_id` parameter on every public function
pub async fn debug<M, E>(ctx: &Context<M, E>, message: &str, args: &LogArgs<'_>) -> Result<()>

// MISSING: `ephemeral` independently reachable on non-Info severities
pub async fn warn<M, E>(ctx: &Context<M, E>, message: &str, args: &LogArgs<'_>) -> Result<()>

// MISSING: Log trait (testability + extensibility)
pub trait Log {
    async fn debug(&self, msg: &str, args: &LogArgs<'_>) -> Result<()>;
    async fn info(&self, msg: &str, args: &LogArgs<'_>) -> Result<()>;
    async fn warn(&self, msg: &str, args: &LogArgs<'_>) -> Result<()>;
    async fn error(&self, msg: &str, args: &LogArgs<'_>) -> Result<()>;
}

// MISSING: Resource-typed URN parameter support
// MISSING: Severity::from_proto() / into_proto() round-trip (useful in engine crate)
```

---

## Recommendations

**P0 — Fix before any public release:**

1. **Add `stream_id` to all public log functions.** This is a protocol-level field being silently dropped; it will cause subtle, hard-to-debug console grouping failures for migrated programs. Introduce `LogArgs` now to avoid a later breaking signature change.

2. **Add `ephemeral` as a parameter on every severity function.** The `status()` shortcut can stay, but `warn(ctx, msg, &LogArgs { ephemeral: true, .. })` must be possible. The current design makes ephemeral warnings and errors structurally unreachable.

**P1 — Implement before beta:**

3. **Accept a `Resource` type (or `Output<String>`) for the URN, resolving internally.** The raw `&str` API breaks the ergonomic contract of the `Output`-chain model. At minimum, provide an overload or a `LogArgs::from_resource()` that awaits the URN internally.

4. **Add a `Log` trait.** This enables mock injection in tests (`impl Log for VecLog { … }`), prevents coupling between the `log` module and `Context`, and unblocks the `pulumi-engine` crate from implementing its own log sink.

**P2 — Quality-of-life improvements:**

5. **Add stderr fallback for engine-unavailable cases.** Match Python's behavior so unit tests that do not spin up a mock engine still produce readable output instead of silent `Error` returns.

6. **Add in-flight log tracking.** Integrate with `Context`'s shutdown barrier so no log RPCs can be dropped during program exit — matching the `workGroup` pattern from Go.

---

## Verdict

**SIGNIFICANT GAPS**

The module is functional for the simplest case (fire a plain-text message at a given severity), but three concrete features — `stream_id`, independent `ephemeral` control, and `Resource`-based URN resolution — are missing or structurally inaccessible, and the flat function signatures guarantee a future breaking-change churn when any of them are added. These are not cosmetic omissions; they affect observable runtime behavior and cross-language portability.