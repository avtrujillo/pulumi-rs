# TODO — pulumi-codegen

## Status: Core pipeline complete (Stages 1–6)

## Remaining Work

- [x] **Proper union types** — `oneOf` now generates named `#[serde(untagged)]` Rust enums (e.g. `StringOrInteger`) in `crate::types`, deduplicated across the schema
- [x] **Asset/Archive types** — `pulumi-core` now provides `Asset` and `Archive` enums (re-exported as `pulumi::Asset` / `pulumi::Archive`); codegen emits these types for `pulumi.json#/Asset` and `pulumi.json#/Archive` refs
- [ ] **Validate against larger providers** — only tested with `pulumi-random`; test with docker, AWS, and other providers that exercise deeply nested modules, complex type references, and edge cases
  - **Unresolved: was docker ever validated?** The docs disagree. The root `TODO.md` roadmap (item 2) says codegen was validated "against `pulumi-random` and docker", and the stage table in this crate's `CLAUDE.md` describes Stage 6 as "(random, docker)". This item and the Future Work list in `CLAUDE.md` say only `pulumi-random`. The only schema checked into `tests/` is `random_schema.json`, so there is no automated docker coverage either way; what is unknown is whether docker was checked by hand. To settle it: run the generator against the docker provider schema, `cargo check` the result, then either add the schema as a test fixture or correct whichever statements turn out to be wrong.
- [ ] **Publish `pulumi` to crates.io** — generated crates default to `pulumi = "0.1"` which doesn't exist yet; the `--pulumi-crate-path` flag works around this for local dev
- [ ] **Clippy warnings** — five style warnings remain, all mechanical: four collapsible `if`s (`ir.rs:156`, `naming.rs:50`, `naming.rs:182`, `naming.rs:183`) and one derivable `Default` (`EmitOptions` in `emit.rs`)
- [x] **Redundant `type_token` branch** — `emit.rs` chose the attribute name with an `if` whose two arms were identical; removed in November 2026, generated output unchanged

## Tests

130 tests: 116 unit tests across `schema.rs`, `naming.rs`, `ir.rs` and `emit.rs`, and 14 in `tests/codegen_tests.rs` that run the pipeline against `tests/random_schema.json` and synthetic schemas.
