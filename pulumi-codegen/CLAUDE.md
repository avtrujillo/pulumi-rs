# pulumi-codegen

Code generator that reads Pulumi provider schema JSON and emits typed Rust
crates with derive macros, serde attributes, and doc comments.

**Full design:** `PLAN.md` in this directory.

## Current Status

**All 6 stages complete.** Full pipeline from schema JSON to generated Rust crate with 123 passing tests + working CLI. Generated `pulumi-random` crate passes `cargo check`.

## Implementation Stages

| Stage | Status | Description |
|-------|--------|-------------|
| 1 | DONE | Crate skeleton + schema JSON deserialization (`schema.rs`) |
| 2 | DONE | Naming utilities: camelCase→snake_case, keyword escaping, token parsing (`naming.rs`) |
| 3 | DONE | IR construction: resolve `$ref`, map types to Rust, organize modules (`ir.rs`) |
| 4 | DONE | Code emission: generate `.rs` files from IR (`emit.rs`) |
| 5 | DONE | CLI binary (`main.rs`) with clap |
| 6 | DONE | Validation against real provider schemas (random, docker) |

Update this table as stages are completed.

## Session Protocol

At the start of each session working on this crate:
1. Read this file and `PLAN.md` for context
2. Check the stage table above — pick up where we left off
3. After completing a stage, update the status in this table and commit

At the end of each session, present a progress summary in this format:

| Stage | Status | Token Pressure | Notes |
|-------|--------|---------------|-------|
| 1 | DONE | Low — independent schema types | |
| 2 | DONE | Low — self-contained naming functions | |
| 3 | IN PROGRESS | High — must hold schema + IR + type resolution simultaneously | Blocked on X |
| ... | | | |

This helps the user gauge how much context window remains and whether to
continue or start a fresh session. The "Token Pressure" column indicates how
much context the *next* stage will demand (see PLAN.md's AI-assisted
development section for the effort/difficulty framework).

**Stage-level context pressure estimates:**

| Stage | Throughput (effort) | Peak Context (difficulty) |
|-------|-------------------|--------------------------|
| 1 | Low — one file, mechanical serde types | Low — no cross-module reasoning |
| 2 | Low — one file, pure functions | Low — self-contained, heavily unit-tested |
| 3 | High — must resolve refs across schema | High — schema + IR + naming all in context |
| 4 | High — many files emitted | Moderate — IR is settled, emission is mechanical |
| 5 | Low — thin CLI wrapper | Low — just wiring clap to lib |
| 6 | Moderate — fix issues from real schemas | Moderate — debugging requires reading generated code |

Stages 1+2 fit comfortably in one session. Stage 3 is the peak-context stage
and may need a dedicated session. Stages 4+5 can share a session if 4 goes
smoothly. Stage 6 may surface issues that feed back into earlier stages.

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
