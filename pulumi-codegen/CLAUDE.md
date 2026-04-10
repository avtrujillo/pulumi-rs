# pulumi-codegen

Code generator that reads Pulumi provider schema JSON and emits typed Rust
crates with derive macros, serde attributes, and doc comments.

**Full design:** `PLAN.md` in this directory.

## Current Status

**Stage 0 — Plan complete, no code written yet.**

Next step: Stage 1 (crate skeleton + schema parsing) + Stage 2 (naming utilities).

## Implementation Stages

| Stage | Status | Description |
|-------|--------|-------------|
| 1 | NOT STARTED | Crate skeleton + schema JSON deserialization (`schema.rs`) |
| 2 | NOT STARTED | Naming utilities: camelCase→snake_case, keyword escaping, token parsing (`naming.rs`) |
| 3 | NOT STARTED | IR construction: resolve `$ref`, map types to Rust, organize modules (`ir.rs`) |
| 4 | NOT STARTED | Code emission: generate `.rs` files from IR (`emit.rs`) |
| 5 | NOT STARTED | CLI binary (`main.rs`) with clap |
| 6 | NOT STARTED | Validation against real provider schemas (random, docker) |

Update this table as stages are completed.

## Architecture (Quick Reference)

Three-phase pipeline: **Schema JSON → Resolved IR → Rust source files**.

```
schema.rs          → PackageSchema, ResourceSpec, PropertySpec, TypeSpec, ...
naming.rs          → camel_to_snake_case(), escape_rust_keyword(), token_to_module_and_name()
ir.rs              → ResolvedPackage, ResolvedResource, ResolvedField, ResolvedType, ...
emit.rs            → Emitter that writes Cargo.toml, lib.rs, mod.rs, per-resource .rs files
main.rs            → CLI: --schema path --out dir
```

## What the Generated Code Looks Like

For each **resource** (e.g., `random:index/randomId:RandomId`):
- `RandomIdArgs` struct — `#[derive(Serialize)]`, fields from `inputProperties`
- `RandomIdOutputs` struct — `#[derive(Deserialize, Clone)]`, fields from `properties`
- `RandomId` unit struct — `#[derive(pulumi::Resource)]` with `#[pulumi(type_token = "...")]`

For each **function** (e.g., `aws:s3/getBucket:getBucket`):
- `GetBucketArgs` — `#[derive(Serialize)]`
- `GetBucketResult` — `#[derive(Deserialize)]`
- `GetBucket` unit struct — `#[derive(pulumi::ProviderFunction)]`

For each **complex type** (object): `#[derive(Serialize, Deserialize, Clone)]` struct.
For each **enum type**: `#[derive(Serialize, Deserialize, Clone, PartialEq)]` enum.

## Key Design Decisions (Settled)

These decisions are final — do not revisit unless there's a concrete problem.

1. **String-based code emission** (not `quote!`/proc-macros). We're writing source
   files to disk, not expanding macros at compile time. Use `format!`/`writeln!`.

2. **Always emit `#[serde(rename = "originalName")]`** on every field, even when
   the Rust name happens to match. Simpler codegen, safer against refactoring.

3. **Always emit `#[serde(skip_serializing_if = "Option::is_none")]`** on optional
   input fields. Providers distinguish absent vs null.

4. **Union types (`oneOf`) → `serde_json::Value`** for now. Proper enum generation
   is a follow-up.

5. **Asset/Archive → `serde_json::Value`** until the SDK adds dedicated types.

6. **`index` module → crate root** (not a submodule named `index`).

7. **Types live in `types/` submodule** to avoid name collisions with resources.

8. **Keyword escaping**: `r#keyword` for most Rust keywords; `self_`, `crate_`,
   `super_`, `Self_` for the four that don't support `r#`. Always paired with
   `#[serde(rename)]`.

9. **Module names**: `meta.moduleFormat` regex extracts module from token. Default
   regex is `(.*)(?:/[^/]*)`. Convert to snake_case. Replace `.` with `_`.

10. **Skip overlays** (`isOverlay: true`) — these are hand-written by SDK authors.

11. **`isComponent` resources** use `#[derive(pulumi::ComponentResource)]` instead
    of `#[derive(pulumi::Resource)]`.

12. **No dependency on `pulumi` or `pulumi-core`** from this crate. The codegen
    only generates source text. The *generated* crates depend on `pulumi`.

## Naming Conventions

| Schema | Rust |
|--------|------|
| Type token `aws:s3/bucket:Bucket` | module `s3`, struct `Bucket` |
| Resource inputs | `{Name}Args` (e.g., `BucketArgs`) |
| Resource outputs | `{Name}Outputs` (e.g., `BucketOutputs`) |
| Function args | `{Name}Args` (e.g., `GetBucketArgs`) |
| Function returns | `{Name}Result` (e.g., `GetBucketResult`) |
| Property `bucketPrefix` | field `bucket_prefix` + `#[serde(rename = "bucketPrefix")]` |
| File for `Bucket` | `bucket.rs` |
| Acronyms: `vpcId` | `vpc_id` |
| Acronyms: `SHA256Hash` | `sha256_hash` |

## Schema Format (Quick Reference)

Pulumi provider schema JSON top-level:
- `name`, `version`, `description` — package metadata
- `meta.moduleFormat` — regex to extract module from type token
- `resources` — map of type token → `ResourceSpec`
- `functions` — map of function token → `FunctionSpec`
- `types` — map of type token → object or enum definition
- `provider` — the provider resource itself (skip for now)

ResourceSpec has: `inputProperties`, `properties`, `requiredInputs`, `required`,
`description`, `deprecationMessage`, `isComponent`, `isOverlay`, `methods`.

Type references: `{"type": "string"}`, `{"type": "array", "items": ...}`,
`{"type": "object", "additionalProperties": ...}`, `{"$ref": "#/types/..."}`,
`{"oneOf": [...]}`. Built-ins: `pulumi.json#/Any`, `pulumi.json#/Asset`,
`pulumi.json#/Archive`.

## Type Mapping

| Schema Type | Rust Type |
|-------------|-----------|
| `"string"` | `String` |
| `"integer"` | `i64` |
| `"number"` | `f64` |
| `"boolean"` | `bool` |
| `"array"` + `items` | `Vec<T>` |
| `"object"` + `additionalProperties` | `std::collections::HashMap<String, T>` |
| `$ref` to named type | generated struct/enum (qualified path) |
| `$ref` to `pulumi.json#/Any` | `serde_json::Value` |
| `$ref` to `pulumi.json#/Asset` | `serde_json::Value` |
| `$ref` to `pulumi.json#/Archive` | `serde_json::Value` |
| `oneOf` | `serde_json::Value` |
| not in `required`/`requiredInputs` | `Option<T>` |

## Test Strategy

- **Schema parsing**: deserialize `pulumi-random` schema JSON, assert structure
- **Naming**: unit tests for camel→snake, keyword escaping, token parsing
- **IR**: load schema → build IR → assert resource/field counts and types
- **End-to-end**: generate crate from `pulumi-random` schema → `cargo check` it

## Dependencies

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
regex = "1"
clap = { version = "4", features = ["derive"] }

[dev-dependencies]
tempfile = "3"
```

## Build

```bash
cargo build -p pulumi-codegen
cargo test -p pulumi-codegen
```

Note: this crate is NOT yet in the workspace `Cargo.toml` members list.
Add it when Stage 1 begins:
```toml
members = ["pulumi-core", "pulumi-macros", "pulumi", "pulumi-automation", "pulumi-cli", "pulumi-engine", "pulumi-codegen"]
```
