# SDK Review: `error`

**Date:** 2026-05-09  
**Rust file:** `pulumi-core/src/error.rs`  

---

## Summary

The Rust `error.rs` is a well-structured foundation for internal SDK error handling, but it is missing the entire family of **user-facing, semantically-rich error types** that all three upstream SDKs uniformly expose: `RunError`, `InputPropertyError`/`InputPropertyErrorDetails`, and `InputPropertiesError`. The `InvokeFailure` variant also uses untyped tuples where upstream SDKs use named, structured types. The Rust module covers transport/RPC/serde concerns well, but gaps in the public error API surface would block provider authors from emitting structured validation errors.

---

## Missing Features

### 1. `RunError` / Clean-Exit Error — All three SDKs
All upstream SDKs expose a special "clean exit" error class/type that signals intentional program termination without emitting verbose stack traces or source text. The Pulumi engine treats this path differently.

- **Python:** `class RunError(Exception)` — bare subclass, no fields
- **Node.js:** `class RunError extends Error` — with `__pulumiRunError` RTTI sentinel and `isInstance()` static method for SxS safety
- **Go:** Implicit via the `errors` package conventions; the clean-exit semantics are documented across the engine boundary

Rust has **no equivalent**. A user program wanting a clean abort must use `Error::Custom(...)`, which the engine would treat as a hard crash, not a clean exit.

---

### 2. `InputPropertyErrorDetails` — All three SDKs
A small named struct/interface/TypedDict encoding a single property-path + reason pair, used as the building block for the two validation error types.

| SDK | Name | Fields |
|---|---|---|
| Go | `InputPropertyErrorDetails` | `PropertyPath string`, `Reason string` (+ `String() string` method) |
| Node.js | `InputPropertyErrorDetails` | `propertyPath: string`, `reason: string` (interface) |
| Python | `InputPropertyErrorDetails` | `property_path: str`, `reason: str` (TypedDict) |

Rust has **no equivalent**. The `InvokeFailure` variant uses `Vec<(String, String)>` — anonymous, positional, and undocumented as to which element is the path vs. reason.

---

### 3. `InputPropertyError` — All three SDKs
A single-property validation error. Used by providers to signal that one specific input field failed validation.

- **Go:** `NewInputPropertyError(propertyPath, reason)`, `InputPropertyErrorf(propertyPath, format, ...args)` constructors
- **Node.js:** `class InputPropertyError extends Error` with `propertyPath`, `reason`, and `isInstance()`
- **Python:** `class InputPropertyError(Exception)` with `property_path`, `reason`

Rust has **no equivalent**.

---

### 4. `InputPropertiesError` — All three SDKs
A multi-property validation error, carrying a top-level message and a list of `InputPropertyErrorDetails`.

- **Go:** `InputPropertiesError` with `WithDetails(...InputPropertyErrorDetails)` builder method; `NewInputPropertiesError(message, ...details)`, `InputPropertiesErrorf(format, ...args)` constructors; formats output with `\n ` separators
- **Node.js:** `class InputPropertiesError extends Error` with `message`, `errors: InputPropertyErrorDetails[]`, RTTI sentinel, and `isInstance()`
- **Python:** `class InputPropertiesError(Exception)` with `message`, optional `errors: list[InputPropertyErrorDetails]`

Rust has **no equivalent**. The `InvokeFailure` variant is the closest analog but is structurally different (it is an enum variant, not an independent type; it requires `token`; and its failure list is untyped).

---

### 5. `ResourceError` — Node.js only
`class ResourceError extends Error` carries a `resource: Resource | undefined` reference and a `hideStack?: boolean` flag. The engine uses this to associate error messages with a specific resource URN in output and to optionally suppress stack traces.

Rust's `ResourceFailed` only stores a `urn: String`, has no `hideStack` equivalent, and cannot be constructed by user programs (it is only emitted internally on registration failure).

---

### 6. `isGrpcError()` utility — Node.js
```typescript
export function isGrpcError(err: Error): boolean {
    const code = (<any>err).code;
    return code === grpc.status.UNAVAILABLE || code === grpc.status.CANCELLED;
}
```
Distinguishes recoverable/expected gRPC termination from real errors. Useful in retry logic and graceful shutdown. Rust has no equivalent helper; callers must manually `match` on `Error::Rpc(status)` and inspect `status.code()`.

---

## Behavioral Divergences

### 1. `InvokeFailure` uses untyped `Vec<(String, String)>` instead of named structs
```rust
// Rust (current)
InvokeFailure {
    token: String,
    failures: Vec<(String, String)>,  // which is property_path, which is reason?
}
```
Go, Node.js, and Python all use explicitly named structs (`InputPropertyErrorDetails` with `property_path`/`PropertyPath` and `reason`/`Reason`). The tuple API is ambiguous, non-self-documenting, and can't be extended without a breaking change.

### 2. `ResourceFailed` is SDK-internal; upstream `ResourceError` is user-constructible
In all upstream SDKs, users can construct a `ResourceError` / raise it themselves to associate an arbitrary failure with a resource. The Rust `ResourceFailed` variant is generated only by the SDK internals (resource registration RPC), not constructible by end users or provider authors.

### 3. No "clean exit" semantic distinction
In Go/Node.js/Python, the Pulumi engine runtime catches `RunError`/`RunError` specifically and exits without emitting a stack trace. The Rust `run()` function has no equivalent detection path: any `Error` returned propagates as a hard failure regardless of semantics.

### 4. No RTTI / cross-crate identity markers (Node.js pattern)
Node.js uses `__pulumiRunError`/`__pulumiInputPropertyError` sentinel fields to allow `isInstance()` checks that survive multiple SDK copies in one process. This is a JS-specific concern, but Rust has no analogous cross-crate downcasting helpers (via `Any`/`is::<T>()`) on the public `Error` type either.

### 5. `InvokeFailure` conflates input-property failures with the invoke error concept
Upstream SDKs keep `InputPropertyError`/`InputPropertiesError` as standalone, reusable types that appear in multiple contexts (resource `check`, `diff`, `configure`, as well as `invoke`). The Rust SDK bundles the failure list directly into `InvokeFailure`, making it impossible to reuse the structured validation types in future resource lifecycle hooks.

---

## API Surface Gaps

| Element | Go | Node.js | Python | Rust |
|---|---|---|---|---|
| `RunError` type | ✅ | ✅ | ✅ | ❌ |
| `InputPropertyErrorDetails` struct | ✅ | ✅ | ✅ | ❌ |
| `InputPropertyError` type | ✅ | ✅ | ✅ | ❌ |
| `InputPropertiesError` type | ✅ | ✅ | ✅ | ❌ |
| `ResourceError` (user-constructible, resource-linked) | ➖ | ✅ | ➖ | ❌ |
| `isGrpcError()` / gRPC status classifier | ➖ | ✅ | ➖ | ❌ |
| `InputPropertiesError::WithDetails()` / builder | ✅ | ➖ | ➖ | ❌ |
| Named failures on invoke errors | ✅ | ✅ | ✅ | ❌ (tuple) |
| `is_run_error()` / `isInstance()` helpers | ➖ | ✅ | ➖ | ❌ |
| `source()` / error chaining | ➖ | ➖ | ➖ | ✅ (Rust-only, good) |
| `From<tonic::*>` conversions | ➖ | ➖ | ➖ | ✅ (Rust-only, good) |
| `Result<T>` type alias | ➖ | ➖ | ➖ | ✅ (Rust-only, good) |

---

## Recommendations

**P0 — Breaking API fix:**

1. **Introduce `InputPropertyErrorDetails` struct** and replace `Vec<(String, String)>` in `InvokeFailure.failures`. This is a breaking change that gets worse the longer it waits. Named fields (`property_path: String`, `reason: String`) match all three upstream SDKs and are self-documenting.
   ```rust
   pub struct InputPropertyErrorDetails {
       pub property_path: String,
       pub reason: String,
   }
   // Then:
   InvokeFailure { token: String, failures: Vec<InputPropertyErrorDetails> }
   ```

**P1 — Missing public API required for provider authors:**

2. **Add `InputPropertyError` and `InputPropertiesError` as public types** (not just enum variants). Provider authors writing resource `check`/`diff` logic need to construct and return these. Mirror Go's constructor functions (`new_input_property_error`, `new_input_properties_error`) and Python's `InputPropertiesError(message, errors)`.

3. **Add a `RunError` variant or type** and wire it into the `run()` function's error handling path so the engine receives a clean-exit signal rather than a crash. This is required for user programs to abort intentionally without noise.

**P2 — Ergonomics:**

4. **Add `is_grpc_error(err: &Error) -> bool`** that checks for `Error::Rpc(status)` where `status.code()` is `Unavailable` or `Cancelled`. This is needed for retry/shutdown logic and directly mirrors the Node.js utility.

5. **Add `InputPropertiesError::with_details()` builder** method (mirrors Go's `WithDetails()`), so callers can accumulate property errors fluently before returning.

**P3 — Completeness:**

6. **Make `ResourceFailed` user-constructible** or add a separate `ResourceError { urn: String, message: String, hide_stack: bool }` variant for user programs to raise resource-scoped errors, mirroring Node.js `ResourceError`.

---

## Verdict

**SIGNIFICANT GAPS**

The three types present in every upstream SDK — `RunError`, `InputPropertyError`/`InputPropertyErrorDetails`, and `InputPropertiesError` — are entirely absent from the Rust implementation. The `InvokeFailure` variant's use of anonymous tuples is a concrete breaking-API debt. These are not obscure edge cases; they are the primary error types that provider authors and user programs interact with. The internal transport/RPC/serde error handling is solid, but the public-facing error API is materially incomplete relative to all upstream SDKs.