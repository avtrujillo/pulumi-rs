# SDK Review: `stack_reference`

**Date:** 2026-05-06  
**Rust file:** `pulumi-core/src/stack_reference.rs`  

---

## Summary

The Rust `stack_reference` module provides a functional scaffolding for cross-stack references but contains at least one critical behavioral correctness bug (wrong gRPC RPC), completely abandons the `Output<T>` laziness model present in every upstream SDK, and is missing the entire secret-detection surface area that Go and Node.js both expose. For programs that rely on secrets in stack outputs or that run `pulumi preview` (dry runs), the Rust implementation will either silently corrupt data or produce wrong results.

---

## Missing Features

### 1. `ReadResource` RPC path (critical)

**Go:** `ctx.ReadResource("pulumi:pulumi:StackReference", name, id, args, &ref, opts...)`
**Node.js:** `super("pulumi:pulumi:StackReference", name, {...}, { ...opts, id: stackReferenceName })` — passing `id` to the `CustomResource` constructor triggers `readResource`, not `registerResource`.
**Rust:** calls `register_resource_inner(..., custom=true, remote=false)` — this sends a `RegisterResource` RPC.

Stack references are *read* operations against existing state, not resource *creations*. `RegisterResource` tells the engine to create/update a resource; `ReadResource` tells it to read existing state from the provider. The `ReadBuilder` abstraction already exists in `resource.rs` but is not used here.

### 2. `StackReferenceOutputDetails` and `get_output_details()`

**Go:**
```go
type StackReferenceOutputDetails struct {
    Value      any
    SecretValue any
}
func (s *StackReference) GetOutputDetails(name string) (*StackReferenceOutputDetails, error)
```
**Node.js:**
```ts
interface StackReferenceOutputDetails { value?: any; secretValue?: any; }
async getOutputDetails(name: string): Promise<StackReferenceOutputDetails>
```
**Rust:** no `StackReferenceOutputDetails` type, no `get_output_details()` method. There is no mechanism to distinguish a plaintext output from a secret output.

### 3. `secretOutputNames` field

**Node.js:**
```ts
public readonly secretOutputNames!: Output<string[]>;
```
Used internally by `isSecretOutputName()` to tag individual `getOutput()` results with the correct secret bit. This is also how the engine communicates the set of secret outputs to the SDK without requiring the engine to unwrap the secret wrapper in the wire format.

**Rust:** absent entirely. Even if `StackReferenceOutputDetails` were added, there is no per-key secret index to consult.

### 4. Lazy `Output<T>`-returning `get_output()` / `require_output()`

**Go:** `GetOutput(name StringInput) AnyOutput` — returns an `AnyOutput` whose resolution is deferred until the program graph settles.
**Node.js:** `getOutput(name: Input<string>): Output<any>` — the returned `Output` propagates the secret bit from `secretOutputNames` and composes with the rest of the output graph.
**Rust:** `get_output(&self, key: &str) -> Option<&serde_json::Value>` — synchronous and eager.

The Rust `Output<T>` type already exists (`output.rs`) and supports `map`, `flat_map`, `all`, etc. Stack reference outputs should be wrapped in `Output<T>` so downstream resources that depend on them form a proper dependency edge and so unknown-ness survives `pulumi preview`.

### 5. `requireOutput` / `require_output` semantics as an `Output`-returning method

**Node.js:**
```ts
requireOutput(name: Input<string>): Output<any>
// Throws inside the Output's resolution if the key is absent:
// "Required output 'x' does not exist on stack 'y'."
```
**Rust:** `require_output<T: DeserializeOwned>(&self, key: &str) -> Result<T>` — fails *eagerly* at call site, not lazily inside an Output chain. Any resource that is supposed to receive this value will not form a dependency edge.

### 6. Dry-run / preview unknown output handling

**Go:**
```go
if !ok {
    if s.ctx.DryRun() {
        return UnsafeUnknownOutput([]Resource{s}), nil
    }
    return nil, nil
}
```
During `pulumi preview` an output key may not yet exist (if the referenced stack has never been deployed). Go returns a well-typed unknown output that preserves the dependency on the StackReference resource. Rust returns `None`, which will cause the calling program to treat a preview-time missing key as a definitive absence and potentially panic or produce incorrect plans.

### 7. `name` and `outputs` fields as `Output<T>`

**Go:** `Name StringOutput`, `Outputs MapOutput`
**Node.js:** `name: Output<string>`, `outputs: Output<{[name:string]:any}>`

The Rust `StackReference` struct exposes neither. Users cannot, e.g., pass `stack_ref.name` as an `Input` to another resource, or attach a dependency on the full outputs map.

### 8. `get_output_value()` / `require_output_value()` (Node.js)

```ts
async getOutputValue(name: string): Promise<any>    // throws if secret
async requireOutputValue(name: string): Promise<any> // throws if missing OR secret
```
These are escape hatches for promptly reading a non-secret value into normal Rust/async code (as opposed to using it within an `Output` chain). No equivalent exists in Rust.

---

## Behavioral Divergences

### 1. `RegisterResource` vs. `ReadResource` — incorrect RPC (critical)

This is the most severe divergence. Sending `RegisterResource` for a stack reference will cause the Pulumi engine to attempt to create/manage a new resource rather than reading existing cross-stack state. With some backends this silently "works" but stores incorrect state; with others it results in errors or double-counting of resource counts against quotas. The correct RPC is `ReadResource`, which maps to `ReadBuilder` in `resource.rs`.

### 2. Eager materialization strips `Output<T>` semantics

The builder immediately calls `.get("outputs").cloned()` and stores a `serde_json::Value`. Every upstream SDK keeps outputs as `Output<T>` through the lifetime of the resource. By materializing eagerly, the Rust SDK:
- Loses unknown propagation (any downstream resource that depends on a stack output will not be marked unknown during preview)
- Loses the dependency graph edge (the resource that uses this value will not list the StackReference as a dependency in the state file unless the user manually calls `.depends_on()`)
- Cannot represent a stack output that becomes known only after an `up`

### 3. Secret sentinel keys stripped silently

The Rust serde layer (`serde.rs`) handles the Pulumi secret wire format (magic key `4dabf18193072939515e22adb298388d`) and unknown sentinel (`04da6b54-80e4-46f7-96ec-b56ff0331ba9`). But after calling `register_resource_inner`, the result's `.outputs` field has already been deserialized. When `stack_outputs` is extracted with `result.outputs.get("outputs").cloned()`, any secret-wrapped values will have been unwrapped by the serde layer with no record that they were secrets. The secret bit is permanently lost — the value appears as plaintext.

This means:
```rust
let token: String = sr.require_output("githubToken")?; // secret value leaked as plaintext
```

### 4. Missing key behavior diverges on dry runs

**Go:** returns `UnsafeUnknownOutput` (correct for preview).
**Node.js:** returns `undefined` within an `Output` (the Output itself is still tracked).
**Rust:** returns `None` synchronously — the caller has no way to distinguish "this key doesn't exist" from "this key doesn't exist *yet* because we're in preview".

### 5. Resource ID not set

**Go:**
```go
id := args.Name.ToStringOutput().ApplyT(func(s string) ID { return ID(s) }).(IDOutput)
ctx.ReadResource("pulumi:pulumi:StackReference", name, id, ...)
```
**Node.js:** `{ ...opts, id: stackReferenceName }`

The resource ID for a StackReference must equal the stack name (e.g., `"org/project/stack"`). Rust never passes an `id` to `register_resource_inner`. The engine uses this ID to locate the referenced stack; without it, behavior is backend-dependent and likely incorrect.

---

## API Surface Gaps

| Upstream API | Rust equivalent | Status |
|---|---|---|
| `StackReferenceOutputDetails { Value, SecretValue }` | — | **Missing** |
| `GetOutputDetails(name string) (*Details, error)` | — | **Missing** |
| `GetOutput(name Input<string>) Output<Any>` (lazy) | `get_output(&str) -> Option<&Value>` (eager) | **Diverges** |
| `RequireOutput(name Input<string>) Output<Any>` (lazy) | `require_output<T>(&str) -> Result<T>` (eager) | **Diverges** |
| `GetStringOutput(name) StringOutput` | `require_output::<String>()` (partial) | **Missing type-safe Output wrapper** |
| `GetFloat64Output(name) Float64Output` | — | **Missing** |
| `GetIntOutput(name) IntOutput` | — | **Missing** |
| `GetIDOutput(name) IDOutput` | — | **Missing** |
| `getOutputValue(name) Promise<any>` (throws on secret) | — | **Missing** |
| `requireOutputValue(name) Promise<any>` (throws on secret) | — | **Missing** |
| `name: Output<string>` field | — | **Missing** |
| `outputs: Output<Map>` field | — | **Missing** |
| `secretOutputNames: Output<string[]>` field | — | **Missing** |

---

## Recommendations

### Priority 1 — Fix the `ReadResource` vs `RegisterResource` bug
**Rationale:** This is a correctness defect that will produce wrong state in every deployment. Switch `StackReferenceBuilder::into_future` to use `ReadBuilder` (which already exists in `resource.rs`), passing the stack name as the resource ID. This is a blocking issue for any real-world usage.

```rust
// Sketch of fix
let result = ReadBuilder::<StackRef>::new(&self.ctx, &self.name, &self.stack_name, inputs)
    .options(self.opts)
    .await?;
```

### Priority 2 — Add `StackReferenceOutputDetails` and `get_output_details()`
**Rationale:** Secret leakage is a security defect. Users calling `require_output()` on a secret stack output will receive the plaintext value with no warning. `get_output_details()` is the minimal API to distinguish secret from non-secret values and is present in all upstream SDKs.

```rust
pub struct StackReferenceOutputDetails {
    pub value: Option<serde_json::Value>,
    pub secret_value: Option<serde_json::Value>,
}

pub fn get_output_details(&self, key: &str) -> StackReferenceOutputDetails
```

### Priority 3 — Wrap outputs in `Output<T>` for lazy access
**Rationale:** Eagerly materializing outputs breaks `pulumi preview` (unknowns) and breaks the dependency graph (downstream resources won't list StackReference as a dep). Change `get_output()` to return `Output<Option<serde_json::Value>>` using the resolver pattern already established in `output.rs`. The struct's `outputs` field should be `Output<HashMap<String, serde_json::Value>>`.

### Priority 4 — Track `secret_output_names` and propagate secret bits per-key
**Rationale:** Even with `StackReferenceOutputDetails`, without knowing *which* keys are secrets, the per-key `get_output()` path cannot correctly tag its result as secret. Mirror Node.js's `secretOutputNames: Output<Vec<String>>` field and use it inside `get_output()` to set the secret flag on the returned `Output<T>`.

### Priority 5 — Add dry-run / unknown output handling in `get_output()`
**Rationale:** Needed for `pulumi preview` correctness. When a key is not found and `context.is_dry_run()` is true, return an `Output` in the unknown state (analogous to Go's `UnsafeUnknownOutput`) rather than `None`.

### Priority 6 — Add typed convenience accessors
**Rationale:** The generic `require_output<T>` partially covers Go's `GetStringOutput` / `GetFloat64Output` / `GetIntOutput`, but returning `Result<T>` instead of `Output<T>` means these don't compose with the output graph. Once Priority 3 is done, add `get_string_output()`, `get_int_output()`, etc. as thin wrappers that call the lazy `get_output()` and apply a type-checked deserialization map.

---

## Verdict

**SIGNIFICANT GAPS**

The `ReadResource` vs `RegisterResource` mismatch is a blocking correctness bug. The complete absence of secret tracking means the module is unsafe for use with stacks that export secrets (a common pattern). The eager output materialization eliminates `Output<T>` composability, breaking preview and dependency tracking. These are not minor API surface omissions — they affect fundamental runtime correctness.