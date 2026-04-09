# TODO — pulumi-cli

## Native Engine Support

Add a `--native` flag or `native-engine` feature to use `NativeStack` from
`pulumi-automation` instead of spawning the `pulumi` CLI binary. This would
allow fully self-contained Rust-native Pulumi operations without requiring the
Go-based CLI to be installed.

## ~~Additional Commands~~ PARTIALLY DONE

~~`config` — get/set/remove configuration values~~ DONE (`config get/set/rm/ls`)
~~`stack export` / `stack import` — state management~~ DONE

Remaining:
- `logs` — show resource logs
- `whoami` — show current identity
- `plugin` — manage provider plugins

## ~~Output Formatting~~ PARTIALLY DONE

~~Add `--json` flag for machine-readable output on all commands~~ DONE
  (`up`, `preview`, `destroy`, `refresh`, `stack ls`, `config get`, `config ls`)

Remaining:
- Add colored output for better readability (currently uses `NO_COLOR=1`)
- Show resource diffs during `preview` output

## ~~Tests~~ PARTIALLY DONE

~~CLI argument parsing tests using clap's test utilities~~ DONE (23 tests)

Remaining:
- Integration tests that verify command output format

## Edition

Uses edition 2024 (nightly). Stable Rust is a non-goal until the next-gen
trait solver ships. See the root TODO.md.
