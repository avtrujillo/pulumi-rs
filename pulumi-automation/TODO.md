# TODO — pulumi-automation

## Tests

- Integration tests using a real `pulumi` binary (gated behind a feature flag
  or separate workspace member)

## NativeStack Parity

**DONE.** `NativeStack` now mirrors `Stack` for the core API:
- Config operations (`get_config`, `set_config`, `get_all_config`, `remove_config`) persist to `.pulumi-rs/<stack>.config.json` and are wired into the program via `PULUMI_CONFIG`/`PULUMI_CONFIG_SECRET_KEYS`
- `outputs()` reads directly from the checkpoint file
- Secret detection in outputs via `is_json_secret()` was already implemented

Structured engine events are now surfaced — see below.

## Engine Events

**DONE.** `NativeStack` now populates the `events` field in all result types:
- `Prelude` — emitted at the start of each operation with the current config
- `ResourceStep` — emitted for each resource (create/update/same/delete) with old/new inputs and outputs
- `Diagnostic` — emitted for every `Engine.Log` RPC call from the program
- `Summary` — emitted at the end with duration and per-op resource change counts

Events are collected via a shared `EventCollector` (`Arc<Mutex<Vec<EngineEvent>>>`) passed
to both the `ResourceMonitorImpl` and `EngineServiceImpl` gRPC service handlers, then
converted to the `pulumi_automation::event::EngineEvent` format in `native.rs`.

## Error Handling Improvements

- `CommandFailed` includes raw stdout/stderr — consider parsing structured
  error output when available
- Add `From<pulumi_engine::Error>` conversion for the `native-engine` path

## Edition

Uses edition 2024 (nightly). Stable Rust is a non-goal until the next-gen
trait solver ships. See the root TODO.md.
