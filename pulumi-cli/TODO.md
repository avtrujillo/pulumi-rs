# TODO — pulumi-cli

## Native Engine Support

Add a `--native` flag or `native-engine` feature to use `NativeStack` from
`pulumi-automation` instead of spawning the `pulumi` CLI binary. This would
allow fully self-contained Rust-native Pulumi operations without requiring the
Go-based CLI to be installed.

## Additional Commands

Consider adding parity with the full `pulumi` CLI:
- `config` — get/set/remove configuration values
- `stack export` / `stack import` — state management
- `logs` — show resource logs
- `whoami` — show current identity
- `plugin` — manage provider plugins

## Output Formatting

- Add `--json` flag for machine-readable output on all commands
- Add colored output for better readability (currently uses `NO_COLOR=1`)
- Show resource diffs during `preview` output

## Tests

No tests exist. Add:
- CLI argument parsing tests using clap's test utilities
- Integration tests that verify command output format

## Edition

Currently uses edition 2024 (nightly). Downgrade to 2021 when the workspace
moves to stable Rust.
