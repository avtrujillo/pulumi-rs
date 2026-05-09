# SDK Review: `automation_events`

**Date:** 2026-05-09  
**Rust file:** `pulumi-automation/src/event.rs`  

---

## Summary

The Rust `automation_events` module (`pulumi-automation/src/event.rs`) implements a thin, partial subset of the upstream event model: only 4 of 11 discriminated-union event variants are present, 6 of those 4 structs are missing fields, one field carries an entirely fabricated name/semantics not found in any upstream, and key enumerations (`OpType`, `DiffKind`) are absent. The module is insufficient for production use in any scenario involving resource operation callbacks, policy enforcement, or debugging.

---

## Missing Features

### 1. Six Missing Event Variants on `EngineEvent`

Both Node.js (`events.ts`) and Python (`events.py`) define 11 discriminated-union variants on `EngineEvent`. Rust provides only 4:

| Variant | Node.js field | Python field | Rust |
|---|---|---|---|
| `CancelEvent` | `cancelEvent` | `cancel_event` | ❌ Missing |
| `StdoutEngineEvent` | `stdoutEvent` | `stdout_event` | ❌ Missing |
| `DiagnosticEvent` | `diagnosticEvent` | `diagnostic_event` | ✅ (partial) |
| `PreludeEvent` | `preludeEvent` | `prelude_event` | ✅ (partial) |
| `SummaryEvent` | `summaryEvent` | `summary_event` | ✅ (partial) |
| `ResourcePreEvent` | `resourcePreEvent` | `resource_pre_event` | ✅ (partial) |
| `ResOutputsEvent` | `resOutputsEvent` | `res_outputs_event` | ❌ Missing |
| `ResOpFailedEvent` | `resOpFailedEvent` | `res_op_failed_event` | ❌ Missing |
| `PolicyEvent` | `policyEvent` | `policy_event` | ❌ Missing |
| `StartDebuggingEvent` | `startDebuggingEvent` | `start_debugging_event` | ❌ Missing |

`CancelEvent` is a zero-field struct used as a terminal signal. `ResOutputsEvent` and `ResOpFailedEvent` are critical for success/failure status of resource operations. `PolicyEvent` is required for CrossGuard/policy-as-code users. `StartDebuggingEvent` carries DAP debugger attachment config.

---

### 2. Missing `CancelEvent` Type

Node.js: `export type CancelEvent = {};`
Python: `class CancelEvent(BaseEvent): ...`

No corresponding Rust type exists.

---

### 3. Missing `StdoutEngineEvent` Type

Node.js: `{ message: string; color: string }`
Python: `StdoutEngineEvent(message, color)`

No corresponding Rust type exists.

---

### 4. Missing `ResOutputsEvent` Type

Node.js: `{ metadata: StepEventMetadata; planning?: boolean }`
Python: `ResOutputsEvent(metadata, planning?)`

No corresponding Rust type exists. This is the primary event signaling that a resource was **successfully provisioned**.

---

### 5. Missing `ResOpFailedEvent` Type

Node.js: `{ metadata: StepEventMetadata; status: number; steps: number }`
Python: `ResOpFailedEvent(metadata, status, steps)`

No corresponding Rust type exists. This is the primary event signaling a resource operation **failed**.

---

### 6. Missing `PolicyEvent` Type

Node.js:
```typescript
export interface PolicyEvent {
    resourceUrn?: string; message: string; color: string;
    policyName: string; policyPackName: string; policyPackVersion: string;
    policyPackVersionTag: string; enforcementLevel: "warning" | "mandatory";
}
```
Python: equivalent `PolicyEvent` dataclass.

No corresponding Rust type exists.

---

### 7. Missing `StartDebuggingEvent` Type

Node.js: `{ config: Record<string, any> }`
Python: `StartDebuggingEvent(config: Mapping[str, Any])`

No corresponding Rust type exists.

---

### 8. Missing `OpType` Enum

Python defines `OpType(str, Enum)` with 15 named variants:
`SAME`, `CREATE`, `UPDATE`, `DELETE`, `REPLACE`, `CREATE_REPLACEMENT`, `DELETE_REPLACED`, `READ`, `READ_REPLACEMENT`, `REFRESH`, `DISCARD`, `DISCARD_REPLACED`, `REMOVE_PENDING_REPLACE`, `IMPORT`, `IMPORT_REPLACEMENT`

Node.js imports `OpType` from `stack.ts` for use as a typed key in `OpMap`.

Rust `StepEventMetadata::op` is a raw `String`, discarding all compile-time safety, exhaustiveness checking, and display formatting for operation types.

---

### 9. Missing `DiffKind` Enum and `PropertyDiff` Struct

Node.js:
```typescript
export enum DiffKind { add, addReplace, delete, deleteReplace, update, updateReplace }
export interface PropertyDiff { diffKind: DiffKind; inputDiff: boolean; }
```
Python: `DiffKind(str, Enum)` with 6 variants; `PropertyDiff(diff_kind, input_diff)`.

No `DiffKind` or `PropertyDiff` type exists in Rust. `StepEventMetadata.detailed_diff` (which maps property paths to `PropertyDiff`) is entirely absent.

---

## Behavioral Divergences

### 1. `SummaryEvent::may_update` Is a Fabricated Field

**This is the most serious divergence.** The upstream field is:

- Node.js: `maybeCorrupt: boolean` — "True if one or more of the resources are in an invalid state"
- Python: `maybe_corrupt: bool`

The Rust struct has:
```rust
pub struct SummaryEvent {
    #[serde(default)]
    pub may_update: bool,
    ...
}
```

The field `mayUpdate` **does not exist in the Pulumi JSON event wire format**. The correct camelCase key is `maybeCorrupt`. Since `#[serde(rename_all = "camelCase")]` is active, Rust will attempt to deserialize a JSON key `"mayUpdate"` (which never appears) and silently default to `false`. The actual `maybeCorrupt` field from the engine output is **silently dropped**. Any code checking `summary_event.may_update` to detect corruption is completely broken.

---

### 2. `EngineEvent` Missing `timestamp` Field

Node.js: `timestamp: number` (Unix seconds, required)
Python: `timestamp: int` (from `data.get("timestamp", 0)`)

The Rust `EngineEvent` has no `timestamp` field. Any consumer needing to order events by wall-clock time or compute operation duration from events cannot do so.

---

### 3. `DiagnosticEvent::urn` Is `String`, Not `Option<String>`

Node.js: `urn?: string`
Python: `urn: Optional[str] = None`

Rust:
```rust
pub struct DiagnosticEvent {
    #[serde(default)]
    pub urn: String,
    ...
}
```

When `urn` is absent from JSON, Rust silently defaults to an empty string `""`. Callers cannot distinguish "this diagnostic has no associated resource" (absent) from "the URN is the empty string" (present but empty). The upstream Python correctly uses `None` to represent absence.

---

### 4. `PreludeEvent::config` Is Untyped `serde_json::Value`

Node.js: `config: Record<string, string>` (map of string→string)
Python: `config: Mapping[str, str]`

Rust:
```rust
pub struct PreludeEvent {
    pub config: serde_json::Value,
}
```

This loses the typed contract. Should be `HashMap<String, String>` (or `IndexMap` for insertion-order preservation), consistent with both upstreams.

---

### 5. `SummaryEvent::resource_changes` Is Untyped `serde_json::Value`

Node.js: `resourceChanges: OpMap` — i.e., `Record<OpType, number>`
Python: `resource_changes: OpMap` — i.e., `MutableMapping[OpType, int]`

Rust:
```rust
pub resource_changes: serde_json::Value,
```

Should be `HashMap<String, i32>` at minimum, or `HashMap<OpType, i32>` once `OpType` is defined.

---

### 6. `SummaryEvent` Missing `policy_packs` Field

Node.js: `policyPacks: Record<string, string>`
Python: `policy_packs: Mapping[str, str]` (deserialized from `"PolicyPacks"` PascalCase key — a known upstream quirk)

No `policy_packs` field exists in the Rust `SummaryEvent`. Any caller checking which policy packs ran cannot access this data.

---

### 7. `ResourcePreEvent` Missing `planning` Field

Node.js: `planning?: boolean`
Python: `planning: Optional[bool] = None`

Rust `ResourcePreEvent` has no `planning` field, so preview-vs-deploy context is lost.

---

### 8. `StepEventMetadata` Missing Five Fields

| Field | Node.js | Python | Rust |
|---|---|---|---|
| `provider` | `provider: string` (required) | `provider: str` (required) | ❌ Missing |
| `keys` | `keys?: string[]` | `keys: Optional[List[str]]` | ❌ Missing |
| `diffs` | `diffs?: string[]` | `diffs: Optional[List[str]]` | ❌ Missing |
| `detailedDiff` | `detailedDiff?: Record<string, PropertyDiff>` | `detailed_diff: Optional[Mapping[str, PropertyDiff]]` | ❌ Missing |
| `logical` | `logical?: boolean` | `logical: Optional[bool]` | ❌ Missing |

`provider` is a **required** field in Node.js. Its absence means users cannot determine which provider performed an operation.

---

### 9. `StepEventStateMetadata` Missing Nine Fields

| Field | Node.js | Python | Rust |
|---|---|---|---|
| `id` | `id: string` (required) | `id: str` (required) | ❌ Missing |
| `parent` | `parent: string` (required) | `parent: str` (required) | ❌ Missing |
| `provider` | `provider: string` (required) | `provider: str` (required) | ❌ Missing |
| `custom` | `custom?: boolean` | `custom: Optional[bool]` | ❌ Missing |
| `delete` | `delete?: boolean` | `delete: Optional[bool]` | ❌ Missing |
| `protect` | `protect?: boolean` | `protect: Optional[bool]` | ❌ Missing |
| `retainOnDelete` | `retainOnDelete?: boolean` | `retain_on_delete: Optional[bool]` | ❌ Missing |
| `initErrors` | `initErrors?: string[]` | `init_errors: Optional[List[str]]` | ❌ Missing |
| `taint` | `taint?: boolean` | `taint: Optional[bool]` | ❌ Missing |

Three of these (`id`, `parent`, `provider`) are **required** in Node.js. `initErrors` is critical for diagnosing partially-initialized resources.

---

### 10. `DiagnosticEvent` Missing Three Fields

| Field | Node.js | Python | Rust |
|---|---|---|---|
| `color` | `color: string` (required) | `color: str` (required) | ❌ Missing |
| `prefix` | `prefix?: string` | `prefix: Optional[str]` | ❌ Missing |
| `streamId` | `streamId?: number` | `stream_id: Optional[int]` | ❌ Missing |
| `ephemeral` | `ephemeral?: boolean` | `ephemeral: Optional[bool]` | ❌ Missing |

`color` is required in both upstreams. `ephemeral` controls whether messages should be rendered transiently in the progress display.

---

## API Surface Gaps

The following public types and fields are present in upstream SDKs but absent from the Rust crate:

**Missing types:**
- `CancelEvent`
- `StdoutEngineEvent`
- `ResOutputsEvent`
- `ResOpFailedEvent`
- `PolicyEvent`
- `StartDebuggingEvent`
- `OpType` (enum)
- `DiffKind` (enum)
- `PropertyDiff`

**Missing fields on `EngineEvent`:**
- `timestamp: i64`
- `cancel_event: Option<CancelEvent>`
- `stdout_event: Option<StdoutEngineEvent>`
- `res_outputs_event: Option<ResOutputsEvent>`
- `res_op_failed_event: Option<ResOpFailedEvent>`
- `policy_event: Option<PolicyEvent>`
- `start_debugging_event: Option<StartDebuggingEvent>`

**Missing fields on `SummaryEvent`:**
- `maybe_corrupt: bool` (currently misnamed `may_update` with broken semantics)
- `policy_packs: HashMap<String, String>`

**Missing fields on `ResourcePreEvent`:**
- `planning: Option<bool>`

**Missing fields on `StepEventMetadata`:**
- `provider: String`
- `keys: Option<Vec<String>>`
- `diffs: Option<Vec<String>>`
- `detailed_diff: Option<HashMap<String, PropertyDiff>>`
- `logical: Option<bool>`

**Missing fields on `StepEventStateMetadata`:**
- `id: String`
- `parent: String`
- `provider: String`
- `custom: Option<bool>`
- `delete: Option<bool>`
- `protect: Option<bool>`
- `retain_on_delete: Option<bool>`
- `init_errors: Option<Vec<String>>`
- `taint: Option<bool>`

**Missing fields on `DiagnosticEvent`:**
- `color: String`
- `prefix: Option<String>`
- `stream_id: Option<i32>`
- `ephemeral: Option<bool>`

---

## Recommendations

**P0 — Fix before any release:**

1. **Rename `SummaryEvent::may_update` → `maybe_corrupt: bool`** and add `#[serde(default)]`. This is a silent data corruption bug — events from the engine are being deserialized with the wrong key, causing the corruption indicator to always read `false`.

2. **Add `timestamp: i64` to `EngineEvent`** with `#[serde(default)]`. Required for event ordering and is present in all upstream SDKs.

3. **Add `ResOutputsEvent` and `ResOpFailedEvent`** to both the struct variants and `EngineEvent`. These are the primary success/failure signals for resource operations; their absence makes it impossible to implement correct `up` completion tracking.

**P1 — Required for feature parity:**

4. **Add `OpType` as a typed enum** (15 variants from Python). Change `StepEventMetadata::op` from `String` to `OpType`. This enables pattern-matching and prevents accepting garbage op strings.

5. **Add `CancelEvent`, `StdoutEngineEvent`, `PolicyEvent`** structs and corresponding `EngineEvent` fields. `PolicyEvent` is required for CrossGuard users; `CancelEvent` is required for detecting operation termination.

6. **Add the 5 missing fields to `StepEventMetadata`**: `provider` (required), `keys`, `diffs`, `detailed_diff`, `logical`. The `provider` field is non-optional in both upstreams.

7. **Add the 9 missing fields to `StepEventStateMetadata`**: `id`, `parent`, `provider` (all required), plus the 6 optional flag fields. Without `id`, resource identity is unavailable; without `init_errors`, error diagnosis is impossible.

8. **Fix `DiagnosticEvent::urn` to `Option<String>`** and add `color`, `prefix`, `stream_id`, `ephemeral`. The `Option` fix is a semantic correctness issue; `ephemeral` controls UI rendering behavior.

**P2 — Complete the model:**

9. **Add `DiffKind` enum and `PropertyDiff` struct**, then populate `StepEventMetadata::detailed_diff`. Required for diff-aware automation tooling.

10. **Add `StartDebuggingEvent`** struct and `EngineEvent` field. Needed for DAP debugger integration.

11. **Strengthen `PreludeEvent::config` to `HashMap<String, String>`** and `SummaryEvent::resource_changes` to `HashMap<String, i32>`. Both are currently `serde_json::Value`, which loses the typed contract enforced by both upstreams.

12. **Add `planning: Option<bool>` to `ResourcePreEvent`** and `ResOutputsEvent`. Required to distinguish preview from apply events.

13. **Add `policy_packs: HashMap<String, String>` to `SummaryEvent`** with `#[serde(rename = "PolicyPacks", default)]` (note PascalCase — this is a known upstream quirk documented in both Go source and Python's `from_json`).

---

## Verdict

**MAJOR GAPS**

6 of 11 event types are completely absent, 1 field carries fabricated semantics that silently misreads the wire format, 3 required fields are missing from `StepEventStateMetadata` and `StepEventMetadata`, and the `timestamp` field needed for event ordering is missing from the top-level `EngineEvent`. The module is not suitable for production automation use.