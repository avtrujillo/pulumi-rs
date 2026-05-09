# SDK Review: `serde`

**Date:** 2026-05-09  
**Rust file:** `pulumi-core/src/serde.rs`  

---

## Summary

The Rust `serde.rs` is a thin structural JSON↔protobuf Struct converter with only rudimentary and partially incorrect Pulumi wire-format awareness. It implements roughly 10–15% of the semantic surface of its Go, Node.js, and Python counterparts. Every upstream SDK uses this module as the beating heart of the resource lifecycle (serialize inputs → register → deserialize outputs → resolve futures); the Rust module cannot yet fulfil that role.

---

## Missing Features

### 1. Signature constants — incomplete and misnamed

All three upstream SDKs export all six wire-format signatures as named constants. Rust exports only two, and one is misnamed:

| Upstream name | Value | Rust |
|---|---|---|
| `specialSigKey` / `_special_sig_key` | `4dabf18193072939515e22adb298388d` | `SECRET_SIG` ← **wrong name** |
| `specialSecretSig` / `_special_secret_sig` | `1b47061264138c4ac30d75fd1eb44270` | ❌ missing |
| `specialAssetSig` / `_special_asset_sig` | `c44067f5952c0a294b673a41bacd8c17` | ❌ missing |
| `specialArchiveSig` / `_special_archive_sig` | `0def7320c3a5731c473e5ecbe6d01bc7` | ❌ missing |
| `specialResourceSig` / `_special_resource_sig` | `5cf8f73096256a8f31e491e813e4eb8e` | ❌ missing |
| `specialOutputValueSig` / `_special_output_value_sig` | `d0e6a833031e9bbcd3f4e8bde6ca49a4` | ❌ missing |
| `unknownValue` / `UNKNOWN` | `04da6b54-80e4-46f7-96ec-b56ff0331ba9` | `UNKNOWN_SIG` ✓ |

`SECRET_SIG` stores `4dabf18193072939515e22adb298388d`, which is the **map key name** (`specialSigKey`), not the secret signature value. This causes downstream confusion in `wrap_secret`, which hardcodes both inline.

---

### 2. Asset and Archive serialization/deserialization — entirely absent

Go `marshalInputOptionsImpl`, Node `serializeProperty`, and Python `serialize_property` all detect Pulumi asset/archive types and emit the signature-bearing wire objects:

```typescript
// Node.js — rpc.ts
{ [specialSigKey]: specialAssetSig, path: "..." }
{ [specialSigKey]: specialArchiveSig, assets: {...} }
```

`deserializeProperty` (all three SDKs) reconstructs `FileAsset`, `StringAsset`, `RemoteAsset`, `FileArchive`, `AssetArchive`, `RemoteArchive` from those objects. Rust has neither direction for any of these types.

---

### 3. Resource reference serialization/deserialization — entirely absent

When `supportsResourceReferences` is negotiated with the monitor, all SDKs emit:

```typescript
// Node.js — rpc.ts
{ [specialSigKey]: specialResourceSig, urn: "...", id: "..." }
```

and deserialize back to live `Resource` / `ProviderResource` objects via `unmarshalResourceReference` (Go) / `deserialize_resource` (Python). Rust has nothing for either direction.

---

### 4. Output-value wire format — entirely absent

When `supportsOutputValues` is negotiated, all SDKs emit:

```typescript
// Node.js — rpc.ts  serializing Output<T>
{ [specialSigKey]: specialOutputValueSig, value: ..., secret: true, dependencies: [...] }
```

and deserialize back to `Output<T>` with proper known/secret/dependency state (Node `deserializeProperty` case `specialOutputValueSig`; Python `deserialize_output_value`). Rust has no equivalent.

---

### 5. `serialize_property` / `serialize_properties` — entirely absent

The core serialization entry points that handle `Output<T>` awaiting, dependency tracking per-property, feature-flag gating (`keepOutputValues`, `excludeResourceReferencesFromDeps`), and recursive descent through arrays/maps/structs are completely missing. Rust's `json_to_proto_value` accepts only already-resolved `serde_json::Value`; it cannot process live `Output<T>` values.

Specifically missing:
- `serialize_property(value, deps, opts)` — Go `marshalInput`, Node `serializeProperty`, Python `serialize_property`
- `serialize_properties(inputs, property_deps, opts)` — Go `marshalInputs`, Node `serializeResourceProperties`, Python `serialize_properties`
- `serialize_resource_properties(label, props, opts)` — Node only; filters out `id`/`urn`

---

### 6. `deserialize_property` / `deserialize_properties` — entirely absent

The mirror of serialization: takes protobuf Struct/Value and produces rich SDK types (with secret propagation, unknown handling, asset/archive/resource reconstruction). Rust's `proto_value_to_json` / `struct_to_json` do structural conversion only; they produce `serde_json::Value` without any Pulumi semantic interpretation.

Specifically missing:
- `deserialize_property(value, keep_unknowns)` — all three SDKs
- `deserialize_properties(props_struct, keep_unknowns)` — all three SDKs
- `deserialize_properties_unwrap_secrets` — Python only
- `struct_contains_unknowns(props)` — Python only

---

### 7. Resource package/module registry — entirely absent

All three SDKs implement a versioned registry used to reconstruct strongly-typed resource objects from URNs during deserialization:

```go
// Go — rpc.go
type ResourcePackage interface { Versioned; ConstructProvider(...) }
type ResourceModule  interface { Versioned; Construct(...) }
func RegisterResourcePackage(pkg string, rp ResourcePackage)
func RegisterResourceModule(pkg, mod string, rm ResourceModule)
```

```python
# Python — rpc.py
class ResourcePackage(ABC):
    def construct_provider(self, name, typ, urn) -> ProviderResource: ...
class ResourceModule(ABC):
    def construct(self, name, typ, urn) -> Resource: ...
def register_resource_package(pkg, package)
def get_resource_package(pkg, version)
def register_resource_module(pkg, mod, module)
def get_resource_module(pkg, mod, version)
```

Rust has none of this. Without it, deserialized resource references always degrade to bare URN strings/dependency stubs; typed resource reconstruction is impossible.

---

### 8. Property transfer and resolution lifecycle — entirely absent

The three-phase lifecycle (before RPC, after RPC success, after RPC failure) is fully absent:

- **`transfer_properties`** (Go/Node/Python): creates `Output<T>` promises for all properties before the gRPC call, returns resolver callbacks.
- **`resolve_properties`** / **`resolve_outputs`** (Go/Node/Python): after the engine responds, resolves all pending outputs with actual values and dependency sets.
- **`resolve_outputs_due_to_exception`** (Python): resolves all outputs exceptionally on resource failure.
- **`resolveProperties`** (Node): handles unknown/secret unwrapping per-property during resolution.

---

### 9. `contains_unknowns` / `containsUnknownValues` — absent

Recursive unknown detection used during output serialization to decide whether a value should be emitted as `UNKNOWN_SIG`:

```python
# Python — rpc.py
def contains_unknowns(val: Any) -> bool:
    ...  # cycle-safe recursive walk
```

```typescript
// Node — rpc.ts
export function containsUnknownValues(value: unknown): boolean { ... }
```

Rust's `is_unknown` operates on `&Struct` (wrong level — see Behavioral Divergences), not on arbitrary values.

---

### 10. `unwrap_secret_values` / `unwrap_rpc_secret_struct_properties` — absent

Recursive secret unwrapping from nested values, with secretness-propagation detection:

```typescript
// Node — rpc.ts
export function unwrapSecretValues(value: unknown): [unknown, boolean] { ... }
```

```python
# Python — rpc.py
def _unwrap_rpc_secret_struct_properties(value: Any) -> tuple[Any, bool]: ...
```

Rust's `unwrap_secret` returns only `Option<&Value>` from a flat Struct and does no recursive propagation.

---

### 11. `translate_output_properties` — absent (Python)

Python has a function that recursively translates property names (`camelCase` → `snake_case`) and performs type coercions (float→int, string→Enum, dict→output type). Rust has no equivalent and will need it once codegen emits typed output structs.

---

### 12. `SerializationOptions` / feature-flag options — absent

Node.js exposes:
```typescript
export interface SerializationOptions {
    keepOutputValues?: boolean;
    excludeResourceReferencesFromDependencies?: boolean;
}
```

Go has `marshalOptions` with the same fields. These gate whether the output-value and resource-reference wire formats are used, based on monitor capability negotiation. Rust has no equivalent option type.

---

## Behavioral Divergences

### 1. `is_secret` is incorrect — false positives for all special types

```rust
// Rust — serde.rs
pub fn is_secret(s: &Struct) -> bool {
    s.fields.contains_key("4dabf18193072939515e22adb298388d")  // only checks key presence
}
```

The special sig key `4dabf18193072939515e22adb298388d` is present in **all** special wire types (assets, archives, resource refs, output values, secrets). Checking only for key presence returns `true` for every special type. The correct check requires also verifying the value equals `specialSecretSig` (`1b47061264138c4ac30d75fd1eb44270`):

```python
# Python — correct
def is_rpc_secret(value):
    return (isinstance(value, dict)
            and _special_sig_key in value
            and value[_special_sig_key] == _special_secret_sig)
```

Any Rust code that calls `is_secret` on an asset/archive/resource-ref Struct will produce a wrong answer.

---

### 2. `is_unknown` operates on the wrong type

```rust
// Rust — serde.rs
pub fn is_unknown(s: &Struct) -> bool {
    if let Some(sig) = s.fields.get("4dabf18193072939515e22adb298388d") ...
    // checks for special sig key on a Struct
}
```

In every upstream SDK, the unknown sentinel is a **bare string value** (`"04da6b54-80e4-46f7-96ec-b56ff0331ba9"`), not an object with the sig key:

```typescript
// Node — rpc.ts
export const unknownValue = "04da6b54-80e4-46f7-96ec-b56ff0331ba9";
// ...
} else if (prop === unknownValue) {
    return isDryRun() || keepUnknowns ? unknown : undefined;
}
```

The Rust function can never return `true` because an unknown value arriving from the engine is a `StringValue`, not a `StructValue` containing the sig key. The correct signature should be `fn is_unknown(v: &Value) -> bool` checking `Kind::StringValue(s) if s == UNKNOWN_SIG`.

---

### 3. Unknown values pass through deserialization as plain strings

Because `proto_value_to_json` performs only structural conversion, the sentinel string `"04da6b54-80e4-46f7-96ec-b56ff0331ba9"` arrives from the engine and is transparently returned as a `serde_json::Value::String`. The caller has no way to distinguish it from a legitimate user string. All upstream SDKs explicitly intercept it:

```python
# Python — rpc.py
if value == UNKNOWN:
    return Unknown() if settings.is_dry_run() or keep_unknowns else None
```

---

### 4. Secret propagation is missing from array/object deserialization

In Node and Python, when deserializing an array or object, if any nested element is a secret, the secretness is "bubbled up" to the container:

```typescript
// Node — rpc.ts  (array case in deserializeProperty)
if (hadSecret) {
    return { [specialSigKey]: specialSecretSig, value: elems };
}
```

```python
# Python — rpc.py
if any(is_rpc_secret(v) for v in values):
    return wrap_rpc_secret([unwrap_rpc_secret(v) for v in values])
```

Rust's `struct_to_json` produces a flat JSON object with no secret propagation. A nested secret field produces a structurally correct JSON object but loses the "this container is secret" signal.

---

### 5. Type-level inconsistency between `wrap_secret` and `is_secret`/`unwrap_secret`

`wrap_secret` takes and returns `serde_json::Value`; `is_secret` and `unwrap_secret` take `&Struct`. A caller who calls `wrap_secret(v)` and then converts to a Struct can call `is_secret`, but the reverse (receiving a secret from the engine, calling `unwrap_secret`) must first call `struct_to_json` or work at the protobuf level directly. All upstream SDKs operate at one consistent level (their native dict type). The Rust API mixes levels, making it awkward to implement round-trips.

---

### 6. `wrap_secret` uses inline magic strings instead of constants

```rust
pub fn wrap_secret(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "4dabf18193072939515e22adb298388d": "1b47061264138c4ac30d75fd1eb44270",
        "value": value,
    })
}
```

Neither string references `SECRET_SIG` (which is actually the sig key, not the secret sig value) nor a missing `SECRET_VALUE_SIG` constant. If an upstream changes the value (extremely unlikely but possible via a version bump), this code would not be caught by a global search for the constant.

---

### 7. No `is_dry_run` awareness during deserialization

All three SDKs gate unknown-value handling on whether a preview is in progress (`isDryRun()` / `settings.is_dry_run()`). During a preview, unknown sentinels are preserved as explicit unknown markers; during `up`, they are collapsed to `None`/`undefined`. Rust's serde module has no access to this state and no deserialization path that even recognises the sentinel, so the behavior would always be the "wrong" one depending on context.

---

## API Surface Gaps

### Public constants missing
```rust
pub const SPECIAL_SIG_KEY: &str = "4dabf18193072939515e22adb298388d";  // rename SECRET_SIG
pub const SECRET_VALUE_SIG: &str = "1b47061264138c4ac30d75fd1eb44270";
pub const ASSET_SIG:         &str = "c44067f5952c0a294b673a41bacd8c17";
pub const ARCHIVE_SIG:       &str = "0def7320c3a5731c473e5ecbe6d01bc7";
pub const RESOURCE_SIG:      &str = "5cf8f73096256a8f31e491e813e4eb8e";
pub const OUTPUT_VALUE_SIG:  &str = "d0e6a833031e9bbcd3f4e8bde6ca49a4";
```

### Public traits missing
```rust
pub trait ResourcePackage: Send + Sync {
    fn version(&self) -> Option<semver::Version>;
    fn construct_provider(&self, ctx: &Context, name: &str, typ: &str, urn: &str)
        -> Result<Box<dyn ProviderResource>>;
}

pub trait ResourceModule: Send + Sync {
    fn version(&self) -> Option<semver::Version>;
    fn construct(&self, ctx: &Context, name: &str, typ: &str, urn: &str)
        -> Result<Box<dyn Resource>>;
}
```

### Public functions missing
```rust
// Serialization
pub async fn serialize_property(
    value: &dyn Any,
    deps: &mut Vec<Arc<dyn Resource>>,
    opts: &SerializationOptions,
) -> Result<serde_json::Value>;

pub async fn serialize_properties(
    inputs: &HashMap<String, Box<dyn Any>>,
    property_deps: &mut HashMap<String, Vec<Arc<dyn Resource>>>,
    opts: &SerializationOptions,
) -> Result<Struct>;

// Deserialization
pub fn deserialize_property(value: &serde_json::Value, keep_unknowns: bool) -> serde_json::Value;
pub fn deserialize_properties(props: &Struct, keep_unknowns: bool) -> serde_json::Value;
pub fn deserialize_resource(ref_struct: &Struct) -> Result<Arc<dyn Resource>>;
pub fn deserialize_output_value(ref_struct: &Struct) -> Output<serde_json::Value>;

// Secret utilities (on serde_json::Value, not &Struct)
pub fn is_rpc_secret(value: &serde_json::Value) -> bool;
pub fn unwrap_rpc_secret(value: serde_json::Value) -> serde_json::Value;
pub fn contains_unknowns(value: &serde_json::Value) -> bool;

// Registry
pub fn register_resource_package(pkg: &str, package: Arc<dyn ResourcePackage>) -> Result<()>;
pub fn get_resource_package(pkg: &str, version: Option<&semver::Version>)
    -> Option<Arc<dyn ResourcePackage>>;
pub fn register_resource_module(pkg: &str, mod_name: &str, module: Arc<dyn ResourceModule>)
    -> Result<()>;
pub fn get_resource_module(pkg: &str, mod_name: &str, version: Option<&semver::Version>)
    -> Option<Arc<dyn ResourceModule>>;

// Resolution lifecycle
pub fn transfer_properties(
    props: &HashMap<String, serde_json::Value>,
) -> HashMap<String, OutputResolver<serde_json::Value>>;

pub fn resolve_properties(
    resolvers: HashMap<String, OutputResolver<serde_json::Value>>,
    all_props: &serde_json::Value,
    deps: &HashMap<String, Vec<Arc<dyn Resource>>>,
);

pub fn resolve_outputs_due_to_exception(
    resolvers: HashMap<String, OutputResolver<serde_json::Value>>,
    err: Error,
);
```

### Public struct missing
```rust
pub struct SerializationOptions {
    pub keep_output_values: bool,
    pub exclude_resource_refs_from_deps: bool,
}
```

---

## Recommendations

### P0 — Fix existing bugs before anything else

1. **Rename `SECRET_SIG` → `SPECIAL_SIG_KEY` and add `SECRET_VALUE_SIG`**. All hardcoded inline strings in `wrap_secret`, `is_secret`, `unwrap_secret`, and `is_unknown` should reference named constants.

2. **Fix `is_secret`**: check `fields["4dabf18193072939515e22adb298388d"] == "1b47061264138c4ac30d75fd1eb44270"`, not just key presence. Add the missing `SECRET_VALUE_SIG` constant for this.

3. **Fix `is_unknown`**: change signature to `fn is_unknown(v: &Value) -> bool` and check `Kind::StringValue(s) if s == UNKNOWN_SIG`. The current Struct-based check can never return `true` for real unknown values.

### P1 — Add all six wire-format signature constants and basic type detection

Add `ASSET_SIG`, `ARCHIVE_SIG`, `RESOURCE_SIG`, `OUTPUT_VALUE_SIG`, rename `SECRET_SIG`. Add `is_special_type(v: &serde_json::Value) -> Option<WireType>` returning an enum. This gates all subsequent work and is a one-day effort with zero external dependencies.

### P2 — Implement `deserialize_property` / `deserialize_properties`

These are needed for any resource output to be readable. Start without asset/archive/resource reconstruction (fall back to plain JSON) and gate on `WireType` dispatch. Add `keepUnknowns` awareness and connect to `is_dry_run()` from `context.rs`. This unblocks the resource registration response path.

### P3 — Implement `serialize_property` / `serialize_properties` with `Output<T>` awaiting

Required for any resource with `Output<T>` inputs to be registered. Must handle the `Output<T>` future, extract secretness and dependency sets, and emit either `UNKNOWN_SIG` (preview), a secret wrapper, or the plain value depending on feature flags. Integrates with the existing `Output<T>` in `output.rs`.

### P4 — Implement the resource package/module registry

Required for `deserialize_resource` to reconstruct typed objects. This is a prerequisite for `pulumi-codegen`-emitted crates to work end-to-end. Model after Go's `versionedMap` with a `RwLock<HashMap<String, Vec<Box<dyn ResourcePackage>>>>`, doing semver-major-constrained best-match lookup.

### P5 — Add asset/archive serialization/deserialization

Implement `FileAsset`, `StringAsset`, `RemoteAsset`, `FileArchive`, `AssetArchive`, `RemoteArchive` types (or reuse from a future `pulumi-asset` crate), wire them into `serialize_property` and `deserialize_property`.

### P6 — Add output-value and resource-reference wire format

Implement the `keepOutputValues` / `supportsOutputValues` path in serialization and the corresponding deserialization, reconstructing `Output<T>` with proper known/secret/dependency state. Implement resource-reference serialization/deserialization gated on `supportsResourceReferences`.

### P7 — Add `transfer_properties` / `resolve_properties` / `resolve_outputs_due_to_exception`

These close the resource registration lifecycle. Without them every registered resource's output properties are permanently pending. Closely tied to `OutputResolver` in `output.rs`.

---

## Verdict

**MAJOR GAPS**

The Rust `serde.rs` is a correct but minimal structural JSON↔protobuf converter. It implements none of the Pulumi-specific serialization semantics (assets, archives, resource references, output values, the resource registry, dependency tracking, or the property resolution lifecycle), contains two confirmed correctness bugs (`is_secret` false positives for all special types; `is_unknown` can never return `true`), and is missing the entire public API surface that makes the serde layer usable within the resource registration pipeline.