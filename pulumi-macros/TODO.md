# TODO — pulumi-macros

## Tests

No tests exist for the derive macros. Add compile-pass and compile-fail tests
using `trybuild` to verify:
- Correct code generation for `#[derive(Resource)]`, `#[derive(ComponentResource)]`,
  `#[derive(ProviderFunction)]`
- Proper error messages for missing required attributes
- Handling of optional attributes (`version`, `plugin_download_url`)

## ComponentMethod Derive Macro

Consider adding `#[derive(ComponentMethod)]` to pair with the
`ComponentMethod` trait in `pulumi-core/src/invoke.rs`, reducing boilerplate
for component method definitions.

## Edition

Uses edition 2024 (nightly). Stable Rust is a non-goal until the next-gen
trait solver ships. See the root TODO.md.
