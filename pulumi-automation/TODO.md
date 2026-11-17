# TODO — pulumi-automation

## Tests

**DONE.** Integration tests using a real `pulumi` binary now exist as the
`integration-tests` workspace member, gated behind its `integration` feature
(`cargo test -p integration-tests --features integration`). They drive
`LocalWorkspace` and `Stack` end to end.

Still open:
- They have not been run since the November 2026 toolchain bump or the config
  changes below, because they need `pulumi` on PATH. See the root TODO.md.
- `NativeStack` has no end-to-end test against a real provider.

## Config Secret Handling

**DONE** (PR #24, July 2026).
- `get_all_config` passes `--show-secrets`, matching the upstream automation
  SDKs. It previously returned masked values.
- `set_config` passes the value after a `--` separator with an explicit
  `--secret` / `--plaintext` flag, so a flag-like value cannot be interpreted
  as a CLI flag. Covered by three tests on `set_config_args`.

## `NativeStack` Stores Secret Config in Plaintext

`NativeStack::save_config_map` (`native.rs`) serializes the whole config map
with `serde_json::to_string_pretty` and writes it to
`<work_dir>/.pulumi-rs/<stack>.config.json`. A value set with
`ConfigValue::secret(...)` is written as-is, with `"secret": true` beside it.

The checkpoint in the same directory *is* encrypted when
`PULUMI_CONFIG_PASSPHRASE` is set, and the upstream CLI stores secret config
encrypted in `Pulumi.<stack>.yaml`, so this file is the one place a secret
sits unprotected on disk.

To fix: encrypt secret values with the same `PassphraseSecretsManager` the
checkpoint uses (`engine_options()` already builds one from the passphrase and
the checkpoint's salt), and decrypt on load. Decide what happens when no
passphrase is set: refusing to store a secret is safer than writing it in the
clear. Until then, anyone using `NativeStack` should keep `.pulumi-rs/` out of
version control.

## Clippy Warnings

Three warnings in `native.rs`, visible only with `--features native-engine`:
a redundant closure (`.map_err(|e| Error::Json(e))`) and two needless
`.to_string()` calls in its tests. CI lints with `--all-features` and
`-D warnings`, so the redundant closure blocks CI once it is running; the two
in test code would too if CI added `--all-targets`.

## `#[non_exhaustive]` on `Error`

Mark the public `Error` enum `#[non_exhaustive]` before the first release. See
the root TODO.md.

## Module Reviews

The May 2026 automated review rated `automation_workspace` as having **major
gaps**, and both it and `automation_config` have changed since, so they need
re-reviewing before the findings are acted on. See `review-output/` and the
root TODO.md.

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

**DONE.** `error.rs` no longer uses `thiserror`; manual `Display`/`Error`/`From` impls
give full control. `extract_error_lines()` surfaces only `error:` lines from CLI
stderr so `CommandFailed` messages are concise. Feature-gated
`From<pulumi_engine::Error>` maps `ProgramFailed → CommandFailed` for the
`native-engine` path. 15 tests added to `error.rs`.

## Toolchain

Uses edition 2024, which is stable. This crate itself needs nothing from
nightly, but the workspace is pinned to nightly because `pulumi-core` uses
`impl_trait_in_assoc_type`. See the root TODO.md.
