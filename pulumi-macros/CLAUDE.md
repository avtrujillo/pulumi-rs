# pulumi-macros

Procedural macro crate providing derive macros for the Pulumi Rust SDK. Re-exported by the `pulumi` crate when the `macros` feature is enabled.

## Build

```bash
cargo build -p pulumi-macros
cargo test -p pulumi-macros     # trybuild compile tests; 5–30s depending on cache
cargo clippy -p pulumi-macros
```

## Tests

`tests/derive_tests.rs` is a single `trybuild` test that compiles the files under `tests/`:

- `tests/pass/` — five programs that must compile: `resource`, `resource_with_version`, `component_resource`, `provider_function`, `component_method`.
- `tests/fail/` — six programs that must fail with the exact error in the matching `.stderr` file: missing `type_token` / `inputs` / `outputs` on a resource, missing `type_token` on a component, missing `token` on a provider function, and an unknown attribute.

The `.stderr` files contain compiler output, so a toolchain bump can change them. If the fail tests break after a bump, check that the message still makes sense and regenerate with `TRYBUILD=overwrite cargo test -p pulumi-macros`.

The four `ignore`d doc examples in `src/lib.rs` show up as ignored doctests.

## Dependencies

- **proc-macro2** — token manipulation
- **quote** — code generation
- **syn** v2 (with `full` feature) — Rust syntax parsing

Dev-only: **trybuild** (compile tests), **pulumi-core** and **serde** (so the test programs can use the real traits).

## Derive Macros

### `#[derive(Resource)]`

Implements `pulumi_core::resource::Resource` for custom cloud resources.

**Required attributes:**
- `#[pulumi(type_token = "aws:s3/bucket:Bucket")]`
- `#[pulumi(inputs = BucketArgs)]`
- `#[pulumi(outputs = BucketOutputs)]`

**Optional attributes:**
- `#[pulumi(version = "6.0.0")]`
- `#[pulumi(plugin_download_url = "...")]`

### `#[derive(ComponentResource)]`

Implements `pulumi_core::resource::ComponentResource` for logical grouping resources.

**Required attributes:**
- `#[pulumi(type_token = "my:module:MyComponent")]`

### `#[derive(ProviderFunction)]`

Implements `pulumi_core::invoke::ProviderFunction` for read-only provider functions.

**Required attributes:**
- `#[pulumi(token = "aws:index/getAmi:getAmi")]`
- `#[pulumi(args = GetAmiArgs)]`
- `#[pulumi(returns = GetAmiResult)]`

**Optional attributes:**
- `#[pulumi(version = "6.0.0")]`
- `#[pulumi(plugin_download_url = "...")]`

### `#[derive(ComponentMethod)]`

Implements `pulumi_core::invoke::ComponentMethod` for a method on a remote component resource.

**Required attributes:**
- `#[pulumi(token = "my:module:MyComponent/doThing")]`
- `#[pulumi(args = DoThingArgs)]`
- `#[pulumi(returns = DoThingResult)]`

**Optional attributes:**
- `#[pulumi(version = "6.0.0")]`
- `#[pulumi(plugin_download_url = "...")]`

## Source Structure

Single file: `src/lib.rs` (~413 lines). Contains all four derive macro entry points and the shared `PulumiAttrs` parser (`parse_pulumi_attrs()`), which extracts `#[pulumi(...)]` attributes using `syn::parse_nested_meta`. An unrecognized attribute name is a compile error (`unknown pulumi attribute: ...`), and each derive reports its own missing required attributes.

Both `Resource` and `ComponentResource` use `type_token`; `ProviderFunction` and `ComponentMethod` use `token`. `pulumi-codegen` emits these attribute names, so renaming one means changing the generator too.
