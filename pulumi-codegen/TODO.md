# TODO — pulumi-codegen

## Status: Core pipeline complete (Stages 1–6)

## Remaining Work

- [ ] **Proper union types** — `oneOf` currently emits `serde_json::Value`; generate proper Rust enums instead
- [ ] **Asset/Archive types** — currently `serde_json::Value`; use dedicated SDK types once `pulumi-core` adds them
- [ ] **Validate against larger providers** — only tested with `pulumi-random`; test with docker, AWS, and other providers that exercise deeply nested modules, complex type references, and edge cases
- [ ] **Publish `pulumi` to crates.io** — generated crates default to `pulumi = "0.1"` which doesn't exist yet; the `--pulumi-crate-path` flag works around this for local dev
