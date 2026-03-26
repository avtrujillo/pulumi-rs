# pulumi-macros

Procedural macro crate providing derive macros for the Pulumi Rust SDK. Re-exported by the `pulumi` crate when the `macros` feature is enabled.

## Build

```bash
cargo build -p pulumi-macros
cargo clippy -p pulumi-macros
```

No tests currently exist in this crate.

## Dependencies

- **proc-macro2** — token manipulation
- **quote** — code generation
- **syn** (with `full` feature) — Rust syntax parsing

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

## Source Structure

Single file: `src/lib.rs` (~316 lines). Contains all three derive macro entry points and the shared `PulumiAttrs` parser (`parse_pulumi_attrs()`), which extracts `#[pulumi(...)]` attributes using `syn::parse_nested_meta`.
