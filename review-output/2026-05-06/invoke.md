# SDK Review: `invoke`

**Date:** 2026-05-06  
**Rust file:** `pulumi-core/src/invoke.rs`  

---

## Summary

The Rust `invoke` module covers the basic happy-path skeleton — a typed `InvokeBuilder`, a `CallBuilder`, and the raw gRPC plumbing — but is missing an entire dimension of functionality that every upstream SDK treats as first-class: Output-form invocation, unknown/secret propagation, preview-safe short-circuiting, invoke transforms, and parent-based provider resolution. The behavioral gaps are not cosmetic; several will cause silent correctness failures at runtime and during previews.

---

## Missing Features

### 1. Output-form invocation (`invoke_output` / `invokeOutput`)

All three upstream SDKs provide a variant that returns an `Output<T>` rather than a bare result. This is the *preferred* form for modern Pulumi programs.

| SDK | Symbol |
|-----|--------|
| Node.js | `invokeOutput<T>(tok, props, opts): Output<T>` |
| Python | `invoke_output(tok, props, opts): Output[Any]` |
| Rust | **absent** |

The difference is not cosmetic. The Output form:
- Propagates secrets from inputs and response onto the returned value
- Marks the result unknown when any input dependency is unresolved (preview safety)
- Tracks resource dependencies from `dependsOn` and from serialized inputs

### 2. `invokeSingle` / `callSingle` variants

Both Node.js and Python export `invokeSingle`, `invokeSingleOutput`, `callSingle` that unwrap a single-keyed result map into a scalar value. Pulumi codegen emits these for functions whose schema declares a single return property.

| SDK | Symbols |
|-----|---------|
| Node.js | `invokeSingle`, `invokeSingleOutput`, `callSingle` |
| Python | `invoke_single`, `invoke_output_single`, `call_single` |
| Rust | **absent** |

### 3. Invoke transforms (`InvokeTransform`, `InvokeTransformArgs`, `InvokeTransformResult`)

Node.js and Python define a full transform pipeline for invokes, analogous to resource transforms:

```typescript
// Node.js
export type InvokeTransform = (args: InvokeTransformArgs) =>
    Promise<InvokeTransformResult | undefined> | InvokeTransformResult | undefined;
```

```python
# Python
InvokeTransform = Callable[
    [InvokeTransformArgs],
    Optional[Union[Awaitable[Optional[InvokeTransformResult]], InvokeTransformResult]],
]
```

The Rust implementation has no `InvokeTransform`, `InvokeTransformArgs`, or `InvokeTransformResult` types and no mechanism to register or apply them before the gRPC call.

### 4. `depends_on` support for Output-form invokes

`InvokeOutputOptions` in Node.js and Python extends base options with an explicit `depends_on` field:

```typescript
// Node.js
export interface InvokeOutputOptions extends InvokeOptions {
    dependsOn?: Input<Input<Resource>[]> | Input<Resource>;
}
```

```python
# Python
class InvokeOutputOptions(InvokeOptions):
    depends_on: Optional[Input[Union[Sequence[Input[Resource]], Resource]]]
```

Rust's `InvokeOptions` has no `depends_on` field. This means callers cannot declare explicit ordering constraints on invokes.

### 5. `parent` field and parent-based provider resolution

Node.js and Python both resolve the provider from `opts.parent` when `opts.provider` is `None`:

```javascript
// Node.js
function getProvider(tok: string, opts: InvokeOptions) {
    return opts.provider ? opts.provider : opts.parent ? opts.parent.getProvider(tok) : undefined;
}
```

```python
# Python
if opts.parent is not None and opts.provider is None:
    opts.provider = opts.parent.get_provider(tok)
```

Rust's `InvokeOptions` has no `parent` field. Provider can only be set as a raw string; there is no fallback resolution chain.

### 6. Package reference (`packageRef`) threading

All upstream SDKs accept a `packageRef` parameter that, when resolved, overrides `version` and `pluginDownloadURL` and is forwarded to the engine. Rust hardcodes it as an empty string:

```rust
// Rust — hardcoded, not configurable
package_ref: String::new(),
```

Neither `InvokeBuilder` nor `CallBuilder` exposes a `.package_ref()` builder method.

### 7. `InvokeOptions.merge()` method

Python provides a `merge()` static/instance method on both `InvokeOptions` and `InvokeOutputOptions` that merges two option bags with well-defined precedence rules (including collection-aware merging of `depends_on`). Rust has no equivalent.

### 8. `ResourcePackage` / `ResourceModule` registry (Go)

Go exposes a versioned registry and `RegisterResourcePackage` / `RegisterResourceModule` functions so that provider SDKs can hook into resource reference deserialization. Rust has no equivalent registry or `ConstructProvider` / `Construct` callbacks.

---

## Behavioral Divergences

### 1. No unknown-value short-circuit during previews (critical)

Every upstream SDK checks for unknown values in serialized inputs before firing the gRPC request and returns an unknown result immediately if any are found:

```javascript
// Node.js
if (containsUnknownValues(serialized)) {
    return { result: {}, isKnown: false, containsSecrets: false, dependencies: [] };
}
```

```python
# Python
if rpc.struct_contains_unknowns(inputs):
    return (InvokeResult(None, is_secret=False, is_known=False), None)
```

The Rust `invoke_inner` fires the RPC unconditionally. During a `pulumi preview`, provider functions receive unknown sentinel UUIDs (`04da6b54-80e4-46f7-96ec-b56ff0331ba9`) as real arguments, which will produce garbage or errors from the provider plugin instead of a properly-marked unknown output. This is arguably the most correctness-critical gap.

### 2. No secret unwrapping before RPC / no secret propagation on result

Both Node.js and Python unwrap Pulumi secret sentinels from inputs before sending to the provider, track whether any input was secret, and mark the output secret if either the input or response was secret:

```javascript
// Node.js
const [plainInputs, inputsContainSecrets] = unwrapSecretValues(serialized);
// ...
containsSecrets: deserialized.containsSecrets || inputsContainSecrets,
```

```python
# Python
plain_inputs, inputs_contain_secrets = rpc._unwrap_rpc_secret_struct_properties(inputs)
invoke_output_secret = is_secret or inputs_contain_secrets
```

Rust sends the raw JSON — including the magic secret key `4dabf18193072939515e22adb298388d` — to the provider as a literal property value. The provider plugin will receive malformed inputs; secrets are neither stripped nor propagated.

### 3. `call()` returns `Result<CallResult<CM>>` instead of `Output<CM::Returns>`

In every upstream SDK, `call()` always returns an `Output` (never a plain value), because component methods inherently operate on potentially-unresolved values:

```typescript
// Node.js
export function call<T>(tok, props, res?, packageRef?): Output<T>
```

```python
# Python
def call(tok, props, res=None, typ=None, package_ref=None) -> "Output[Any]"
```

Rust's `CallBuilder` awaits to `Result<CallResult<CM>>` — a fully resolved Rust `Result`. This means:
- No Output dependency tracking on the return value
- No secretness propagation
- No unknownness propagation
- Callers cannot compose call results with `Output::map` / `Output::flat_map`

### 4. `keep_output_values` not set for `call_inner`

Both Node.js and Python set `keepOutputValues: true` / `keep_output_values=True` when serializing inputs for `call`, to preserve output wire values for the component provider. They also set `excludeResourceRefsFromDependencies: true`:

```javascript
// Node.js
await serializePropertiesReturnDeps(`call:${tok}`, props, {
    keepOutputValues: true,
    excludeResourceReferencesFromDependencies: true,
});
```

Rust serializes call args with `serde_json::to_value(&args)`, which performs a plain JSON serialization — no output preservation, no resource-reference exclusion from property dependencies.

### 5. `accept_resources` not ENV-gated

Node.js and Python respect the `PULUMI_DISABLE_RESOURCE_REFERENCES` environment variable:

```javascript
req.setAcceptresources(!utils.disableResourceReferences);
```

```python
accept_resources = os.getenv("PULUMI_DISABLE_RESOURCE_REFERENCES", "").upper() not in {"TRUE", "1"}
```

Rust hardcodes `accept_resources: true`, ignoring the environment variable entirely. This will cause resource-reference deserialization to fail in environments where the variable is set (e.g., older engines).

### 6. No CustomResource ID-knowness guard for Output-form invokes

Node.js and Python `invokeOutput` / `invoke_output` both expand dependencies of inputs and check whether all `CustomResource` IDs are known before proceeding, returning `isKnown: false` if any ID is unknown:

```javascript
// Node.js
for (const dep of expandedDeps.values()) {
    if (CustomResource.isInstance(dep) && dep.id) {
        const known = await dep.id.isKnown;
        if (!known) { return { result: {}, isKnown: false, ... }; }
    }
}
```

Rust has no equivalent guard.

### 7. Provider is a raw string, not a typed resource

Rust `InvokeOptions.provider` is `Option<String>`. Upstream SDKs accept `ProviderResource` typed objects and perform the URN+ID serialization internally. This couples Rust callers to a manual URN-construction ceremony and makes typed provider passing impossible.

---

## API Surface Gaps

### Types missing from public API

| Type | Present in | Absent from Rust |
|------|-----------|------------------|
| `InvokeOutputOptions` | Node.js, Python | ✗ |
| `InvokeTransform` | Node.js, Python | ✗ |
| `InvokeTransformArgs` | Node.js, Python | ✗ |
| `InvokeTransformResult` | Node.js, Python | ✗ |
| `InvokeResult` (awaitable wrapper) | Python | ✗ |
| `ResourcePackage` trait + registry | Go | ✗ |
| `ResourceModule` trait + registry | Go | ✗ |

### Methods/functions missing from public API

| Symbol | Present in | Absent from Rust |
|--------|-----------|------------------|
| `invoke_output()` / `invokeOutput()` | Node.js, Python | ✗ |
| `invoke_single()` / `invokeSingle()` | Node.js, Python | ✗ |
| `invoke_output_single()` / `invokeSingleOutput()` | Node.js, Python | ✗ |
| `call_single()` / `callSingle()` | Node.js, Python | ✗ |
| `InvokeOptions::merge()` | Python | ✗ |
| `InvokeOptions.parent` field | Node.js, Python | ✗ |
| `InvokeOptions.depends_on` field | Node.js, Python | ✗ |
| `.package_ref()` builder method | Node.js, Python | ✗ |
| `register_invoke_transform()` | Node.js, Python | ✗ |

### Builder methods missing from `InvokeBuilder` / `CallBuilder`

- `.version()` — `version` is sourced from the const but cannot be overridden at call-site in a discoverable way (the `opts.version` field is on the struct but there is no dedicated builder method)
- `.plugin_download_url()` — same problem
- `.package_ref()` — completely absent
- `.depends_on()` — absent

---

## Recommendations

**Priority 1 — Preview correctness (blocking for production use)**

1. **Unknown-value short-circuit in `invoke_inner`**: Scan the serialized `args` protobuf Struct for the sentinel UUID `04da6b54-80e4-46f7-96ec-b56ff0331ba9` before calling the monitor. Return `Err(Error::UnknownValue)` or (for output form) resolve the Output as unknown. Without this, `pulumi preview` will produce incorrect and potentially crashing behavior.

2. **Secret unwrapping before RPC**: Strip the Pulumi secret sentinel (`4dabf18193072939515e22adb298388d`) from the serialized args struct before sending to the provider, and track whether any input was secret. This can live alongside the existing `serde.rs` logic.

**Priority 2 — Output-form invoke (required for idiomatic Pulumi programs)**

3. **`invoke_output()` on `InvokeBuilder`**: Add a `.output()` terminator (or a separate `InvokeOutputBuilder`) that returns `Output<F::Returns>` instead of `Result<F::Returns>`. Wire in: secret propagation, unknown short-circuit, dependency tracking from `depends_on`.

4. **Add `depends_on` to `InvokeOptions` / separate `InvokeOutputOptions`**: Accept `Vec<Output<String>>` (URNs) or a proper resource slice. This unlocks explicit ordering and is required for `invoke_output` to behave correctly.

**Priority 3 — API completeness (required before codegen can emit correct Rust)**

5. **`invoke_single` / `invoke_output_single` / `call_single`**: These are the forms codegen will emit for single-return functions. Add a `.single()` terminator on both builders that calls `_extract_single_value` on the result.

6. **`parent` field in `InvokeOptions`**: Add a `parent: Option<Arc<dyn Resource>>` field and implement the fallback chain `provider ?? parent.get_provider(tok)`. This is required for inheritance-based provider selection to work.

7. **`keep_output_values` for `call_inner`**: When building the call request, the args should be serialized with output-value preservation. Until `Output<T>` can be serialized, at minimum document that args to `CallBuilder` must be pre-resolved and file an issue.

**Priority 4 — Robustness and interop**

8. **`accept_resources` ENV gate**: Read `PULUMI_DISABLE_RESOURCE_REFERENCES` from the environment (in `Settings` or inline) and thread it through `invoke_inner`. Hardcoding `true` breaks compatibility with older engines.

9. **`package_ref` threading**: Add `.package_ref(impl Future<Output=Option<String>>)` to both builders and forward it to the RPC request, clearing `version`/`plugin_download_url` when it resolves to `Some`.

10. **Invoke transforms**: Add `InvokeTransform`, `InvokeTransformArgs`, `InvokeTransformResult` types and a `register_invoke_transform()` function. The transform pipeline should run before `invoke_inner`. This is a significant undertaking but is required for policy-as-code and SDK-level interception.

---

## Verdict

**MAJOR GAPS**

The Rust implementation has the correct structural skeleton and the gRPC wiring compiles and routes correctly, but it is missing the entire Output-form invocation path (the preferred form in all modern Pulumi programs), has critical behavioral divergences that will cause silent correctness failures during previews (no unknown short-circuit, no secret propagation), and is missing the invoke transform system entirely. It is not suitable for production use or as a codegen target without addressing at minimum Priorities 1–3 above.