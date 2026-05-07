# SDK Review: `resource`

**Date:** 2026-05-06  
**Rust file:** `pulumi-core/src/resource.rs`  

---

## Summary

The Rust `resource` module covers the fundamental registration path (custom resources, component resources, remote components, reads), aliases, custom timeouts, and basic builder ergonomics. However, it is missing the entire resource lifecycle hook system, per-resource transforms/transformations, multi-provider support, several `ResourceOptions` fields that are wired in all three upstream SDKs (though their proto fields *exist* in `register_resource_inner` but are hardcoded to empty), runtime resource state tracking needed for protect/provider inheritance, property-level dependency maps, and critical utilities like `create_urn`, `merge_options`, and `DependencyResource`. Programs using this SDK cannot leverage hooks, transforms, `replacement_trigger`, `replace_with`, `hide_diffs`, or `env_var_mappings`, and parent-protect inheritance is silently broken.

---

## Missing Features

### 1. Resource Lifecycle Hook System — Entirely Absent
All three upstream SDKs (Go `ResourceHookBinding`, Node.js `ResourceHookBinding`, Python `ResourceHookBinding`) expose before/after lifecycle callbacks for create, update, and delete, plus error retry hooks. None of this exists in Rust. The proto field is hardcoded:

```rust
// register_resource_inner
hooks: None,   // ← always None, no hook binding ever sent
```

Missing types: `ResourceHook`, `ErrorHook`, `ResourceHookBinding`, `ResourceHookOptions`, `ResourceHookFunction`, `ErrorHookFunction`, `ResourceHookArgs`, `ErrorHookArgs`.

### 2. Per-Resource `transforms` / `transformations` — Absent from `ResourceOptions`
Stack-level transforms exist in `transform.rs`, but per-resource transforms (the new async `ResourceTransform` callback style, Go `Transforms []ResourceTransform`, Python `transforms: list[ResourceTransform]`, Node.js `transforms`) and old-style synchronous `transformations` are both absent from `ResourceOptions` and hardcoded:

```rust
transforms: Vec::new(),   // ← always empty
```

### 3. Multi-Provider `providers` Map — Missing
Component resources need a bag of providers (keyed by package name) inherited by child resources. Go `Providers []ProviderResource`, Python `providers: Mapping[str, ProviderResource]`, Node.js `ComponentResourceOptions.providers`. In Rust, only a single `provider: Option<String>` exists. The proto field is hardcoded:

```rust
providers: HashMap::new(),   // ← always empty, no provider bag ever sent
```

### 4. `replace_with`, `replacement_trigger`, `hide_diffs`, `env_var_mappings` — Options Exist in Proto but Not Surfaced
All four fields exist in the `RegisterResourceRequest` proto (code even constructs them!) but are hardcoded and not exposed in `ResourceOptions`:

```rust
replace_with: Vec::new(),           // Go: ReplaceWith, Node: replaceWith, Python: replace_with
replacement_trigger: None,          // Go: ReplacementTrigger, Node: replacementTrigger, Python: replacement_trigger
hide_diffs: Vec::new(),             // Go: HideDiffs, Node: hideDiffs, Python: hide_diffs
env_var_mappings: HashMap::new(),   // Go: EnvVarMappings, Node: envVarMappings, Python: env_var_mappings
```

### 5. `urn` Option / `getResource` — Missing
All upstream SDKs support looking up an existing resource by its URN (not by cloud provider ID). This triggers a separate `getResource` code path (Go `URN_ string`, Node.js `opts.urn`, Python `opts.urn`). Rust has no `urn` field in `ResourceOptions` and no `getResource` implementation. `ReadBuilder` covers `readResource` (by provider ID) but not this case.

### 6. `DependencyResource` / `DependencyProviderResource` — Missing
Used by all SDKs to create synthetic resources for dependency tracking in remote components. Required for the `newDependency` callback in `registerResource` (Node.js) / `DependencyResource` (Python/Node.js) to reconstruct resource references from URNs in responses. Rust has no equivalent.

### 7. `ProviderResource` Type — Missing
All upstream SDKs have a first-class `ProviderResource` type (Go interface with `getPackage()`, Python `ProviderResource` class, Node.js `ProviderResource` class). In Rust, providers are opaque reference strings with no associated type. This prevents typed provider passing and `getPackage()`-based routing.

### 8. `mergeOptions()` / `ResourceOptions.merge()` — Missing
All upstream SDKs provide options merging (Go `merge()`, Node.js `mergeOptions()`, Python `ResourceOptions.merge()`), with well-defined semantics: collections are concatenated, scalars take the later value. Component resource authors need this to forward merged options to children.

### 9. `create_urn()` Utility — Missing
Go `createUrn`, Node.js `createUrn`, Python `create_urn` — computes a full Pulumi URN from name, type, parent, stack, project. Used by alias inheritance computation and code-generated SDKs.

### 10. `allAliases()` / Alias Inheritance — Missing
All upstream SDKs implement alias inheritance: when a parent resource has aliases, child resources whose names are derived from the parent's name automatically inherit corresponding aliases. The logic for `inheritedChildAlias` and `allAliases` is completely absent.

### 11. Property-Level Dependency Tracking — Hardcoded Empty
All upstream SDKs build a `property_dependencies` map that tracks which resource URNs each input property depends on. This enables fine-grained update ordering. Rust hardcodes:

```rust
property_dependencies: HashMap::new(),   // ← always empty
```

This is structural — it requires `Output<T>` integration to collect per-property resource references at serialization time.

### 12. Source Position / Stack Trace — Hardcoded None
Node.js and Python capture call-site source position and send it to the engine for better error messages and diagnostics. Rust hardcodes:

```rust
source_position: None,
stack_trace: None,
```

### 13. Package Reference / Parameterization — Missing
Go `Parameterization([]byte)`, Node.js/Python `packageRef`. Rust hardcodes `package_ref: String::new()`.

---

## Behavioral Divergences

### 1. `protect` / `retain_on_delete` Are `bool` Instead of `Option<bool>` — Inheritance Broken
**Critical.** Upstream SDKs use tri-state (`*bool` in Go, `Optional[bool]` in Python, `boolean | undefined` in Node.js) so that `None`/`nil`/`undefined` means "inherit from parent." Rust uses `bool` with `false` default:

```rust
pub protect: bool,           // no way to express "not set, inherit from parent"
pub retain_on_delete: bool,  // same problem
```

A resource whose parent has `protect = true` will silently receive `protect = false` in Rust, because `false` is passed to the engine instead of letting the engine apply inheritance.

### 2. `delete_before_replace_defined` Logic Is Wrong
The proto has separate `delete_before_replace` and `delete_before_replace_defined` fields so the engine can distinguish "user said false" from "user didn't specify." Rust sets:

```rust
delete_before_replace_defined: opts.delete_before_replace,
```

This means `defined` is `true` only when `delete_before_replace = true`. You can never express "I explicitly set this to false" because the `bool` type can't encode that, and `defined` will be `false` in that case — wrong semantics.

### 3. No Parent-Chain Provider Resolution
Upstream SDKs walk the parent chain to resolve which provider to use for a resource when `opts.provider` is not set. Rust passes `opts.provider.clone().unwrap_or_default()` — if not set, the provider is always the empty string, relying entirely on engine defaults. No programmatic provider inheritance is possible.

### 4. No `protect` Inheritance from Parent
Related to divergence #1 — upstream SDKs explicitly check `if opts.protect is None: opts.protect = opts.parent._protect`. No such logic in Rust.

### 5. `ComponentBuilder` Never Sends Inputs
Local component resources in Rust receive an empty JSON object regardless. In Python and Node.js, component resource inputs are forwarded to the engine (with the `PULUMI_NODEJS_SKIP_COMPONENT_INPUTS` opt-out). This means component resource props are always empty in Rust, which may break providers that inspect them.

### 6. `PULUMI_DISABLE_RESOURCE_REFERENCES` Ignored
All upstream SDKs check this env var to set `accept_resources`. Rust hardcodes `accept_resources: true`.

### 7. `ReadBuilder` Conflates Two Distinct Concepts
Upstream SDKs have three resource flows: `registerResource`, `readResource` (by provider ID — `opts.id`), and `getResource` (by URN — `opts.urn`). Rust's `ReadBuilder` covers only `readResource`. The `import_id` in `ResourceOptions` is also distinct (it imports cloud state into a `registerResource` call, not a `readResource` call).

### 8. `read_resource_inner` Returns Empty `property_deps`
```rust
property_deps: HashMap::new(),  // ReadBuilder always returns empty property_deps
```
Upstream SDKs carry property dependency information through read operations.

---

## API Surface Gaps

**Types entirely missing from public API:**

| Upstream Name | Go | Node.js | Python | Rust |
|---|---|---|---|---|
| `ResourceHook` | ✅ | ✅ | ✅ | ❌ |
| `ErrorHook` | ✅ | ✅ | ✅ | ❌ |
| `ResourceHookBinding` | ✅ | ✅ | ✅ | ❌ |
| `ResourceHookFunction` | ✅ | ✅ | ✅ | ❌ |
| `ErrorHookFunction` | ✅ | ✅ | ✅ | ❌ |
| `ResourceHookArgs` | ✅ | ✅ | ✅ | ❌ |
| `ErrorHookArgs` | ✅ | ✅ | ✅ | ❌ |
| `ProviderResource` | ✅ | ✅ | ✅ | ❌ |
| `DependencyResource` | ✅ | ✅ | ✅ | ❌ |
| `DependencyProviderResource` | ✅ | ✅ | ✅ | ❌ |
| `ResourceTransformation` | ✅ | ✅ | ✅ | ❌ |
| `ResourceTransformationArgs` | ✅ | ✅ | ✅ | ❌ |
| `ResourceTransform` | ✅ | ✅ | ✅ | ❌ |
| `ResourceTransformArgs` | ✅ | ✅ | ✅ | ❌ |

**Functions missing:**

| Function | Present in |
|---|---|
| `create_urn(name, type, parent, project, stack)` | Go, Node.js, Python |
| `merge_options(opts1, opts2)` | Go, Node.js, Python |
| `all_aliases(child_aliases, child_name, child_type, parent)` | Go, Node.js, Python |
| `get_resource(urn)` | Go, Node.js, Python |
| `register_resource_hook(hook)` | Go, Node.js, Python |
| `register_error_hook(hook)` | Go, Node.js, Python |

**Fields missing from `ResourceOptions`:**

| Field | Type |
|---|---|
| `providers` | `HashMap<String, String>` (package → provider ref) |
| `replace_with` | `Vec<String>` |
| `replacement_trigger` | `Option<serde_json::Value>` |
| `hide_diffs` | `Vec<String>` |
| `env_var_mappings` | `HashMap<String, String>` |
| `transforms` | `Vec<ResourceTransform>` |
| `transformations` | `Vec<ResourceTransformation>` |
| `hooks` | `Option<ResourceHookBinding>` |
| `urn` | `Option<String>` (for `getResource`) |

**Builder methods missing:**

| Builder | Missing Methods |
|---|---|
| `ResourceBuilder` | `providers()`, `replace_with()`, `replacement_trigger()`, `hide_diffs()`, `env_var_mappings()`, `version()` override, `plugin_download_url()` override |
| `ComponentBuilder` | `depends_on()`, `provider()`, `providers()`, `ignore_changes()`, `version()` |
| `RemoteComponentBuilder` | `protect()`, `alias()`, `ignore_changes()`, `delete_before_replace()` |
| `ReadBuilder` | `protect()`, `ignore_changes()`, `additional_secret_outputs()` |

---

## Recommendations

**P0 — Correctness bugs to fix before any release:**

1. **Change `protect`, `retain_on_delete` to `Option<bool>` in `ResourceOptions`** and implement parent-chain inheritance in `register_resource_inner`. Pass `None` as the proto field when unset. This is a silent semantic bug that corrupts the protect flag.

2. **Fix `delete_before_replace_defined`**: Use `Option<bool>` for the field so the proto field can correctly reflect whether the user explicitly set it: `delete_before_replace_defined: self.opts.delete_before_replace.is_some()`.

**P1 — High-impact missing options (proto fields already exist):**

3. **Surface `replace_with`, `replacement_trigger`, `hide_diffs`, `env_var_mappings` in `ResourceOptions`** and wire them through the builders. The hardcoded empty values in `register_resource_inner` just need to become `opts.*` references. Low implementation cost, high completeness gain.

4. **Add `providers: HashMap<String, String>` to `ResourceOptions`** and wire to the `providers` proto field. Component resource authors are completely blocked without this.

5. **Add `urn: Option<String>` to `ResourceOptions` and implement `get_resource()`** as a third path alongside `ReadBuilder` and `ResourceBuilder`.

**P2 — Core ergonomics:**

6. **Add `create_urn()` utility function**. Required by code-generated SDKs for alias computation and URN construction. Straightforward to implement using the same logic as Python/Node.js.

7. **Add `ResourceOptions::merge()`**. Component resource authors need this to correctly propagate options to children without destructively overwriting them.

8. **Add `all_aliases()` and alias inheritance logic**. Without this, renaming a component resource doesn't propagate alias URNs to its children — resources are unnecessarily destroyed.

9. **Add `transforms` / `transformations` to `ResourceOptions`** and wire to the existing proto field. The callback infrastructure in `transform.rs` already exists for stack transforms; per-resource transforms use the same gRPC mechanism.

**P3 — Significant features:**

10. **Implement Resource Lifecycle Hooks** (`ResourceHook`, `ErrorHook`, `ResourceHookBinding`). Requires callback gRPC server (same pattern as `transform.rs`). All three upstream SDKs support this; it's increasingly used by providers and codegen SDKs.

11. **Add `ProviderResource` type/trait** with `package()` method. Enables typed provider passing, `providers` map construction from provider instances, and proper `getPackage()`-based routing.

12. **Add `DependencyResource` / `DependencyProviderResource`**. Required for the `new_dependency` callback in `register_resource_inner` when reconstructing resource references from `property_dependencies` in responses.

13. **Property-level dependency tracking**. Requires integrating `Output<T>` collection during serialization. The `property_dependencies` map is always empty, which means the engine cannot perform fine-grained ordering optimization.

**P4 — Polish:**

14. **Source position tracking** — capture the call-site location and populate `source_position` / `stack_trace`.
15. **Check `PULUMI_DISABLE_RESOURCE_REFERENCES`** before hardcoding `accept_resources: true`.
16. **Add missing builder methods**: `ComponentBuilder::depends_on()`, `ComponentBuilder::provider()`, `RemoteComponentBuilder::protect()`, `ReadBuilder::additional_secret_outputs()`.

---

## Verdict

**SIGNIFICANT GAPS**

The core resource registration path is functional. However, the resource lifecycle hook system is entirely absent, per-resource transforms are missing, multi-provider support is hardcoded to empty, four options already in the proto are hardcoded rather than wired, `protect` inheritance is silently broken due to the `bool` vs `Option<bool>` mismatch, property-level dependency tracking is always empty, and critical utilities (`create_urn`, `mergeOptions`, `allAliases`) required for component resource authoring and code-generated SDKs do not exist. This SDK cannot be used as a drop-in replacement for the upstream SDKs in production programs that use components, hooks, transforms, or multi-provider configurations.