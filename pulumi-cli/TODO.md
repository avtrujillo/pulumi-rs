# TODO — pulumi-cli

## Native Engine Support

Add a `--native` flag or `native-engine` feature to use `NativeStack` from
`pulumi-automation` instead of spawning the `pulumi` CLI binary. This would
allow fully self-contained Rust-native Pulumi operations without requiring the
Go-based CLI to be installed.

## Additional Commands

- `logs` — show resource logs
- `whoami` — show current identity
- `plugin` — manage provider plugins

## Output Formatting

- Add colored output for better readability (currently uses `NO_COLOR=1`)
- Show resource diffs during `preview` output

## Tests

- Integration tests that verify command output format

## Edition

Uses edition 2024 (nightly). Stable Rust is a non-goal until the next-gen
trait solver ships. See the root TODO.md.
