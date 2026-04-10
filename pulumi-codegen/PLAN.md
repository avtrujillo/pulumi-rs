# pulumi-codegen Implementation Plan

## Overview

`pulumi-codegen` is a new crate that reads a Pulumi provider schema JSON file and
generates a complete Rust crate with strongly-typed resource, function, and type
definitions. The generated code uses the `pulumi` crate's derive macros and traits
so that users can `cargo add pulumi-aws` instead of hand-writing hundreds of structs.

## What Gets Generated

For a provider like `random`, the tool produces:

```
pulumi-random/
├── Cargo.toml                  # depends on pulumi (with macros feature), serde, serde_json
├── src/
│   ├── lib.rs                  # re-exports all modules; contains "index" module items
│   ├── random_id.rs            # RandomId resource (from index module)
│   ├── random_password.rs      # RandomPassword resource
│   ├── random_string.rs        # ...etc
│   └── types/                  # complex types (objects + enums), if any
│       ├── mod.rs
│       └── ...
```

For a provider with modules like `aws`:

```
pulumi-aws/
├── Cargo.toml
├── src/
│   ├── lib.rs                  # pub mod s3; pub mod ec2; pub mod lambda; ...
│   ├── s3/
│   │   ├── mod.rs              # pub mod bucket; pub use bucket::*; ...
│   │   ├── bucket.rs           # Bucket, BucketArgs, BucketOutputs
│   │   └── bucket_object.rs
│   ├── ec2/
│   │   ├── mod.rs
│   │   └── instance.rs
│   └── types/
│       ├── mod.rs              # pub mod s3; pub mod ec2; ...
│       ├── s3/
│       │   ├── mod.rs
│       │   └── bucket_lifecycle_rule.rs
│       └── ...
```

### Per Resource

Given schema resource `aws:s3/bucket:Bucket`:

```rust
use serde::{Deserialize, Serialize};

/// An S3 bucket resource.
#[derive(Serialize)]
pub struct BucketArgs {
    /// The name of the bucket.
    #[serde(rename = "bucket")]
    pub bucket: Option<String>,
    /// The canned ACL to apply.
    #[serde(rename = "acl")]
    #[deprecated(note = "Use bucketAcl resource instead")]
    pub acl: Option<String>,
    // ...
}

/// An S3 bucket resource.
#[derive(Deserialize, Clone)]
pub struct BucketOutputs {
    /// The ARN of the bucket.
    #[serde(rename = "arn")]
    pub arn: String,
    /// The name of the bucket.
    #[serde(rename = "bucket")]
    pub bucket: String,
    // ...
}

/// An S3 bucket resource.
#[derive(pulumi::Resource)]
#[pulumi(type_token = "aws:s3/bucket:Bucket")]
#[pulumi(inputs = BucketArgs)]
#[pulumi(outputs = BucketOutputs)]
#[pulumi(version = "6.50.0")]
pub struct Bucket;
```

### Per Function

Given schema function `aws:s3/getBucket:getBucket`:

```rust
/// Get information about an existing S3 bucket.
#[derive(Serialize)]
pub struct GetBucketArgs {
    #[serde(rename = "bucket")]
    pub bucket: String,
}

#[derive(Deserialize)]
pub struct GetBucketResult {
    #[serde(rename = "arn")]
    pub arn: String,
    #[serde(rename = "bucket")]
    pub bucket: String,
    // ...
}

#[derive(pulumi::ProviderFunction)]
#[pulumi(token = "aws:s3/getBucket:getBucket")]
#[pulumi(args = GetBucketArgs)]
#[pulumi(returns = GetBucketResult)]
#[pulumi(version = "6.50.0")]
pub struct GetBucket;
```

### Per Complex Type (Object)

```rust
/// Configuration for bucket lifecycle rules.
#[derive(Serialize, Deserialize, Clone)]
pub struct BucketLifecycleRule {
    #[serde(rename = "enabled")]
    pub enabled: bool,
    #[serde(rename = "prefix")]
    pub prefix: Option<String>,
    // ...
}
```

Complex types get both `Serialize` and `Deserialize` because they can appear
in both inputs and outputs.

### Per Enum Type

```rust
/// The canned ACL to apply to the bucket.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub enum CannedAcl {
    #[serde(rename = "private")]
    Private,
    #[serde(rename = "public-read")]
    PublicRead,
    #[serde(rename = "public-read-write")]
    PublicReadWrite,
    // ...
}
```

---

## Architecture

The codegen has three phases, each cleanly separated:

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   Phase 1    │     │   Phase 2    │     │   Phase 3    │
│              │     │              │     │              │
│ Schema JSON  │────>│  Resolved IR │────>│  Rust Source  │
│  Parsing     │     │ Construction │     │  Emission    │
│              │     │              │     │              │
└──────────────┘     └──────────────┘     └──────────────┘
    serde            naming, refs,         String-based
  deserialize        type mapping         code generation
```

### Phase 1: Schema Parsing

Deserialize the provider schema JSON into Rust types using serde.

**Key types:**

```rust
/// Top-level provider schema.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSchema {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub meta: Option<Meta>,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceSpec>,
    #[serde(default)]
    pub functions: BTreeMap<String, FunctionSpec>,
    #[serde(default)]
    pub types: BTreeMap<String, ComplexTypeSpec>,
    pub provider: Option<ResourceSpec>,
    // language, config, etc. — parsed but not used for Rust codegen
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpec {
    pub description: Option<String>,
    #[serde(default)]
    pub input_properties: BTreeMap<String, PropertySpec>,
    #[serde(default)]
    pub properties: BTreeMap<String, PropertySpec>,
    #[serde(default)]
    pub required_inputs: Vec<String>,
    #[serde(default)]
    pub required: Vec<String>,
    pub deprecation_message: Option<String>,
    #[serde(default)]
    pub is_component: bool,
    #[serde(default)]
    pub is_overlay: bool,
    #[serde(default)]
    pub methods: BTreeMap<String, String>,
    pub state_inputs: Option<ObjectTypeSpec>,
    #[serde(default)]
    pub aliases: Vec<AliasSpec>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertySpec {
    // Type info (one of these patterns):
    #[serde(rename = "type")]
    pub type_: Option<String>,           // "string", "integer", etc.
    #[serde(rename = "$ref")]
    pub ref_: Option<String>,            // "#/types/pkg:mod:Name"
    pub items: Option<Box<TypeSpec>>,     // for arrays
    pub additional_properties: Option<Box<TypeSpec>>,  // for maps
    pub one_of: Option<Vec<TypeSpec>>,    // for unions

    // Metadata:
    pub description: Option<String>,
    pub deprecation_message: Option<String>,
    #[serde(default)]
    pub secret: bool,
    #[serde(rename = "default")]
    pub default_value: Option<serde_json::Value>,
    #[serde(default)]
    pub plain: bool,
    #[serde(default)]
    pub replace_on_changes: bool,
}
```

**Files:** `pulumi-codegen/src/schema.rs` (~200 lines)

### Phase 2: Resolved IR Construction

Walk the parsed schema and build an intermediate representation with all names
resolved, types mapped to Rust, and modules organized.

**Key responsibilities:**

1. **Token parsing** — Split `aws:s3/bucket:Bucket` into package `aws`,
   module `s3`, name `Bucket` using the `meta.moduleFormat` regex.

2. **Module organization** — Group resources, functions, and types by module.
   Map `index` to the crate root. Convert module names to snake_case
   (e.g., `applicationLoadBalancing` → `application_load_balancing`).

3. **Name generation** — For each resource `Foo`:
   - Resource struct: `Foo`
   - Input struct: `FooArgs`
   - Output struct: `FooOutputs`
   - File: `foo.rs` (snake_case of type name)

   For each function `getFoo`:
   - Function struct: `GetFoo`
   - Args struct: `GetFooArgs`
   - Result struct: `GetFooResult`
   - File: `get_foo.rs`

4. **Property name mapping** — camelCase → snake_case using a state machine
   that handles acronyms correctly:
   - `bucketPrefix` → `bucket_prefix`
   - `SHA256Hash` → `sha256_hash`
   - `vpcId` → `vpc_id`
   - `s3BucketArn` → `s3_bucket_arn`

5. **Type resolution** — Map each `PropertySpec` to a Rust type:

   | Schema | Rust Type |
   |--------|-----------|
   | `"string"` | `String` |
   | `"integer"` | `i64` |
   | `"number"` | `f64` |
   | `"boolean"` | `bool` |
   | `"array"` + `items` | `Vec<T>` |
   | `"object"` + `additionalProperties` | `std::collections::HashMap<String, T>` |
   | `$ref` → `#/types/pkg:mod:Name` | resolved struct/enum name with module path |
   | `$ref` → `pulumi.json#/Any` | `serde_json::Value` |
   | `$ref` → `pulumi.json#/Asset` | `serde_json::Value` (TBD: SDK asset type) |
   | `$ref` → `pulumi.json#/Archive` | `serde_json::Value` (TBD: SDK archive type) |
   | `oneOf` | `serde_json::Value` (see note below) |
   | optional property (not in `required`/`requiredInputs`) | `Option<T>` |

   **Union types (`oneOf`):** For the initial implementation, union types map to
   `serde_json::Value`. A follow-up can generate proper Rust enums with
   `#[serde(untagged)]` or discriminated union support.

6. **Keyword escaping** — When a property name (after snake_case conversion)
   collides with a Rust keyword:
   - Use raw identifier `r#type` for most keywords
   - Use `self_`, `crate_`, `super_`, `Self_` for the four keywords that
     don't support `r#`
   - Always emit `#[serde(rename = "originalName")]` so the wire name is preserved

7. **Collision detection** — When two types would generate the same Rust name
   in the same module, disambiguate by appending the module suffix
   (e.g., `Bucket` in both `s3` and `glacier` → no collision since different
   modules; but `BucketLifecycleRule` and `BucketLifecycleRule` from different
   tokens in the same module → append discriminating suffix).

**IR types:**

```rust
/// A fully resolved provider, ready for code emission.
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
    pub modules: BTreeMap<String, ResolvedModule>,  // "" = root
    pub types: BTreeMap<String, ResolvedType>,       // by original token
}

pub struct ResolvedModule {
    pub resources: Vec<ResolvedResource>,
    pub functions: Vec<ResolvedFunction>,
}

pub struct ResolvedResource {
    pub type_token: String,
    pub rust_name: String,           // "Bucket"
    pub file_name: String,           // "bucket"
    pub description: Option<String>,
    pub deprecation: Option<String>,
    pub is_component: bool,
    pub input_fields: Vec<ResolvedField>,
    pub output_fields: Vec<ResolvedField>,
}

pub struct ResolvedFunction {
    pub token: String,
    pub rust_name: String,           // "GetBucket"
    pub file_name: String,           // "get_bucket"
    pub description: Option<String>,
    pub deprecation: Option<String>,
    pub arg_fields: Vec<ResolvedField>,
    pub result_fields: Vec<ResolvedField>,
}

pub struct ResolvedField {
    pub original_name: String,       // "bucketPrefix" (wire name)
    pub rust_name: String,           // "bucket_prefix" or "r#type"
    pub rust_type: String,           // "Option<String>"
    pub description: Option<String>,
    pub deprecation: Option<String>,
    pub secret: bool,
    pub required: bool,
}

pub enum ResolvedType {
    Object(ResolvedObject),
    Enum(ResolvedEnum),
}

pub struct ResolvedObject {
    pub rust_name: String,
    pub module: String,
    pub file_name: String,
    pub description: Option<String>,
    pub fields: Vec<ResolvedField>,
}

pub struct ResolvedEnum {
    pub rust_name: String,
    pub module: String,
    pub file_name: String,
    pub description: Option<String>,
    pub underlying_type: String,     // "String", "i64", etc.
    pub variants: Vec<ResolvedEnumVariant>,
}

pub struct ResolvedEnumVariant {
    pub rust_name: String,           // "PublicRead"
    pub value: String,               // "public-read"
    pub description: Option<String>,
    pub deprecation: Option<String>,
}
```

**Files:** `pulumi-codegen/src/ir.rs` (~400 lines), `pulumi-codegen/src/naming.rs` (~150 lines)

### Phase 3: Code Emission

Walk the `ResolvedPackage` and write Rust source files to disk. Uses simple
string formatting (not proc-macros or `quote!` — those are compile-time tools;
codegen writes source strings to files).

**What gets emitted:**

1. **`Cargo.toml`** for the generated crate:
   ```toml
   [package]
   name = "pulumi-{provider}"
   version = "{schema.version}"
   edition = "2024"
   description = "Pulumi SDK for the {Provider} provider"
   license = "Apache-2.0"

   [dependencies]
   pulumi = { version = "0.1", features = ["macros"] }
   serde = { version = "1", features = ["derive"] }
   serde_json = "1"
   ```

2. **`src/lib.rs`** — module declarations + re-exports of root-level items.

3. **Per-module `mod.rs`** — `pub mod resource_file;` for each resource/function.

4. **Per-resource `.rs` file** — `Args` struct, `Outputs` struct, derive macro
   invocation on the unit struct. Includes doc comments and deprecation attributes.

5. **Per-function `.rs` file** — `Args` struct, `Result` struct, derive macro.

6. **Per-type `.rs` file** — Object struct or enum with serde derives.

**Formatting:** The emitted code won't be perfectly formatted. The user runs
`rustfmt` on the output (or the tool runs it automatically as a post-step).

**Files:** `pulumi-codegen/src/emit.rs` (~500 lines)

---

## Implementation Stages

These stages are ordered so that each one produces a testable, working artifact.

### Stage 1: Crate Skeleton + Schema Parsing (~1-2 sessions)

**Goal:** Parse any provider schema JSON into typed Rust structures.

1. Create `pulumi-codegen/Cargo.toml` with dependencies: `serde`, `serde_json`, `regex`, `clap`
2. Add `pulumi-codegen` to workspace `Cargo.toml` members
3. Implement `schema.rs` — all schema types with `#[derive(Deserialize)]`
4. Write a test that loads the `pulumi-random` schema and asserts basic structure
5. Write a test that loads a larger schema (e.g., `pulumi-aws` subset) to catch edge cases

**Test:** `cargo test -p pulumi-codegen` — schema deserialization roundtrip.

### Stage 2: Naming Utilities (~1 session)

**Goal:** Robust name conversion functions with comprehensive tests.

1. `naming.rs` — `camel_to_snake_case()` state machine, `to_pascal_case()`,
   `escape_rust_keyword()`, `token_to_module_and_name()`
2. Extensive unit tests covering:
   - Basic: `bucketPrefix` → `bucket_prefix`
   - Acronyms: `vpcId` → `vpc_id`, `SHA256Hash` → `sha256_hash`
   - Numbers: `s3Bucket` → `s3_bucket`, `ec2Instance` → `ec2_instance`
   - Keywords: `type` → `r#type`, `self` → `self_`
   - Module parsing: `aws:s3/bucket:Bucket` → `("s3", "Bucket")`
   - Index module: `random:index/randomId:RandomId` → `("", "RandomId")`

**Test:** `cargo test -p pulumi-codegen` — naming unit tests.

### Stage 3: IR Construction (~2 sessions)

**Goal:** Transform parsed schema into resolved IR.

1. `ir.rs` — IR types + builder that walks `PackageSchema`
2. Token parsing with `moduleFormat` regex
3. Type resolution — map `PropertySpec` → Rust type string
4. `$ref` resolution — follow references into `types` map
5. Required/optional field determination
6. Handle `isOverlay` — skip overlay resources/functions/types
7. Handle `isComponent` — use `ComponentResource` derive instead of `Resource`

**Test:** Load `pulumi-random` schema → build IR → assert resource count,
field names, types.

### Stage 4: Code Emission (~2 sessions)

**Goal:** Generate compilable Rust code from IR.

1. `emit.rs` — `Emitter` struct with methods for each output file type
2. Emit `Cargo.toml`, `lib.rs`, module `mod.rs` files
3. Emit resource files (Args + Outputs + derive macro)
4. Emit function files (Args + Result + derive macro)
5. Emit type files (objects + enums)
6. Doc comment generation from `description` (handle multi-line markdown)
7. Deprecation attribute generation

**Test:** Generate `pulumi-random` crate → run `rustfmt` → run `cargo check`
on the output.

### Stage 5: CLI Binary (~1 session)

**Goal:** Usable command-line tool.

1. `main.rs` — CLI using `clap`:
   ```
   pulumi-codegen --schema path/to/schema.json --out ./pulumi-random
   ```
2. Optional: `--format` flag to run `rustfmt` automatically
3. Optional: `--version-override` flag for testing

**Test:** End-to-end: schema JSON in → generated crate out → `cargo check` passes.

### Stage 6: Validation with Real Providers (~1 session)

**Goal:** Prove it works on real-world schemas.

1. Generate `pulumi-random` — small, no complex types
2. Generate `pulumi-docker` — medium, has some complex types
3. Fix any issues discovered
4. Add integration test that generates + compiles against a pinned schema

---

## Edge Cases and Design Decisions

### Asset / Archive Types

Pulumi has `Asset` and `Archive` built-in types (`pulumi.json#/Asset`,
`pulumi.json#/Archive`). The SDK doesn't currently define these types. For now,
map them to `serde_json::Value`. When the SDK adds proper Asset/Archive types,
update the codegen mapping.

### Union Types (`oneOf`)

Union types are complex — they can combine primitives, objects, and enums in
arbitrary combinations. For the initial implementation:
- Map all `oneOf` types to `serde_json::Value`
- This is safe and always works, just less typed
- Follow-up: generate `#[serde(untagged)]` enums for common patterns
  (e.g., string-or-object unions)

### Resource Methods

Some resources have `methods` that map to function tokens. These functions have
a special `__self__` input parameter referencing the resource. For the initial
implementation:
- Skip method generation (the functions are still generated standalone)
- Follow-up: generate convenience methods on the resource's output type

### The Provider Resource

The schema's `provider` field describes the provider resource itself. For the
initial implementation:
- Skip provider resource generation
- Users configure providers via Pulumi config or explicit provider construction
- Follow-up: generate a typed `Provider` resource

### `stateInputs`

Used for resource import/lookup. For the initial implementation:
- Skip `stateInputs` generation
- The core `ReadBuilder` already accepts untyped properties
- Follow-up: generate a `FooState` struct for typed import

### Deeply Nested Modules

Some providers (Kubernetes) have deeply nested module paths like
`admissionregistration.k8s.io/v1`. These need sanitization:
- Replace `.` with `_` in module names
- Replace `/` with `::` (nested Rust modules) or `_` (flat)
- Decision: use nested modules (`admissionregistration_k8s_io::v1`)

### Name Collisions Between Resources, Functions, and Types

In rare cases, a resource and a type may have the same name in the same module.
Resolution: types go in a `types` submodule, resources and functions in the
module root. This naturally avoids collisions.

### Optional Field Serialization

Input struct optional fields should skip serialization when `None`:
```rust
#[serde(rename = "bucketPrefix", skip_serializing_if = "Option::is_none")]
pub bucket_prefix: Option<String>,
```

This prevents sending `null` values for fields the user didn't set, which some
providers interpret differently from absent fields.

### `serde(rename)` on Every Field

Even when the Rust field name happens to match the wire name (e.g., `bucket`),
we still emit `#[serde(rename = "bucket")]`. This makes the codegen simpler
(always emit rename) and protects against future refactoring that might change
the Rust name.

---

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

The codegen crate does **not** depend on `pulumi` or `pulumi-core` — it only
generates source text. The generated crates depend on `pulumi`.

---

## File Summary

```
pulumi-codegen/
├── Cargo.toml
├── PLAN.md              (this file)
├── src/
│   ├── main.rs          CLI entry point (~50 lines)
│   ├── lib.rs           pub mod declarations (~10 lines)
│   ├── schema.rs        Schema JSON types (~200 lines)
│   ├── naming.rs        Name conversion utilities (~150 lines)
│   ├── ir.rs            IR types + builder (~400 lines)
│   └── emit.rs          Code emission (~500 lines)
└── tests/
    ├── random_schema.json       test fixture
    └── codegen_tests.rs         integration tests
```

**Estimated total:** ~1,300 lines of code + ~300 lines of tests.

---

## Session Breakdown (for AI-assisted development)

Given Pro plan context constraints, here's a suggested session plan:

| Session | Stage | Deliverable |
|---------|-------|-------------|
| 1 | Stage 1 + 2 | Crate skeleton, schema parsing, naming utilities with tests |
| 2 | Stage 3 | IR construction with type resolution and ref following |
| 3 | Stage 4 | Code emission — generate compilable Rust from IR |
| 4 | Stage 5 + 6 | CLI binary, validate against `pulumi-random`, fix issues |
| 5 | Polish | Edge cases, larger provider testing, documentation |

Each session produces a commit with passing tests, so progress is never lost.
