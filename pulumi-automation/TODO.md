# TODO — pulumi-automation

## Tests

- Integration tests using a real `pulumi` binary (gated behind a feature flag
  or separate workspace member)

## NativeStack Parity

**DONE.** `NativeStack` now mirrors `Stack` for the core API:
- Config operations (`get_config`, `set_config`, `get_all_config`, `remove_config`) persist to `.pulumi-rs/<stack>.config.json` and are wired into the program via `PULUMI_CONFIG`/`PULUMI_CONFIG_SECRET_KEYS`
- `outputs()` reads directly from the checkpoint file
- Secret detection in outputs via `is_json_secret()` was already implemented

Remaining gap: structured engine events (equivalent to `--event-log` in the CLI) are not yet surfaced — `events` is always `Vec::new()` in all result types.

## Error Handling Improvements

- `CommandFailed` includes raw stdout/stderr — consider parsing structured
  error output when available
- Add `From<pulumi_engine::Error>` conversion for the `native-engine` path

## Edition

Uses edition 2024 (nightly). Stable Rust is a non-goal until the next-gen
trait solver ships. See the root TODO.md.
