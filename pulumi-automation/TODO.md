# TODO — pulumi-automation

## Tests

- Integration tests using a real `pulumi` binary (gated behind a feature flag
  or separate workspace member)

## NativeStack Parity

`NativeStack` (behind `native-engine` feature) mirrors `Stack` but currently:
- Sets `secret: false` for all output values (no secret detection)
- Does not support config operations (`get_config`, `set_config`, etc.)
- Does not surface engine events the way the CLI-based `Stack` does

## Error Handling Improvements

- `CommandFailed` includes raw stdout/stderr — consider parsing structured
  error output when available
- Add `From<pulumi_engine::Error>` conversion for the `native-engine` path

## Edition

Uses edition 2024 (nightly). Stable Rust is a non-goal until the next-gen
trait solver ships. See the root TODO.md.
