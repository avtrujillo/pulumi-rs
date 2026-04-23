# TODO — pulumi-codegen

## Status: Core pipeline complete (Stages 1–6)

## Remaining Work

- [x] **Proper union types** — `oneOf` now generates named `#[serde(untagged)]` Rust enums (e.g. `StringOrInteger`) in `crate::types`, deduplicated across the schema
- [x] **Asset/Archive types** — `pulumi-core` now provides `Asset` and `Archive` enums (re-exported as `pulumi::Asset` / `pulumi::Archive`); codegen emits these types for `pulumi.json#/Asset` and `pulumi.json#/Archive` refs
- [ ] **Validate against larger providers** — only tested with `pulumi-random`; test with docker, AWS, and other providers that exercise deeply nested modules, complex type references, and edge cases
- [ ] **Publish `pulumi` to crates.io** — generated crates default to `pulumi = "0.1"` which doesn't exist yet; the `--pulumi-crate-path` flag works around this for local dev
