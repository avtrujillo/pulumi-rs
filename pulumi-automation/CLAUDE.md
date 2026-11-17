# pulumi-automation

Automation API for Pulumi — drive stack operations (`up`, `preview`, `destroy`, `refresh`) programmatically by wrapping the Pulumi CLI as a subprocess. Requires the `pulumi` binary on PATH.

## Why subprocesses?

This crate invokes the `pulumi` CLI binary rather than using gRPC directly. This is necessary because the Pulumi engine is embedded inside the CLI — there is no standalone engine server to connect to. The lifecycle is:

1. `pulumi-automation` spawns `pulumi up` (or preview/destroy/refresh)
2. The CLI starts the engine and resource monitor internally
3. The engine runs the user's program, which uses `pulumi-core` to talk gRPC back to the engine
4. The program exits, the CLI completes the deployment

So `pulumi-automation` sits *outside* the CLI (driving it), while `pulumi-core` sits *inside* (called by it). Both depend on having `pulumi` installed.

## Build

```bash
cargo build -p pulumi-automation
cargo test -p pulumi-automation
cargo clippy -p pulumi-automation
```

Doctests are disabled (`doctest = false`).

## Dependencies

- **tokio** (process, io-util) — async subprocess spawning
- **serde / serde_json** — JSON parsing of CLI output
- **pulumi-engine** (optional, via `native-engine` feature) — Rust-native engine backend
- **clap** is NOT a dependency (that's in `pulumi-cli`)
- **thiserror** is NOT a dependency either; `error.rs` has hand-written `Display` / `Error` / `From` impls

Dev-only: **tokio** (rt-multi-thread, macros) and **tempfile**.

## Features

- **`native-engine`** — Enables `NativeStack`, which uses the Rust-native `pulumi-engine` instead of spawning the `pulumi` CLI binary. Adds a dependency on `pulumi-engine`.

## Module Guide

| Module | Purpose |
|--------|---------|
| `workspace.rs` | `LocalWorkspace` — main entry point. Wraps a project directory. Stack methods: `create_stack()`, `select_stack()`, `create_or_select_stack()`, `list_stacks()`, `remove_stack()`. Config methods: `get_config()`, `set_config()`, `get_all_config()`, `remove_config()`. Also `whoami()`, `logs()`, `list_plugins()`, `install_plugin()`, `remove_plugin()`, and `native_stack()` (feature-gated). Supports env injection via `with_env()`. |
| `stack.rs` | `Stack` — represents a deployed stack. Lifecycle: `up()`, `preview()`, `destroy()`, `refresh()`. State: `outputs()`, `export_state()`, `import_state()`. Config delegation to workspace. |
| `cmd.rs` | `run_pulumi_cmd(work_dir, args, env)` — spawns `pulumi` subprocess with `PULUMI_NON_INTERACTIVE=true` and `NO_COLOR=1`. Returns `CmdOutput { stdout, stderr }`. |
| `config.rs` | `ConfigValue` — `value` plus `is_secret` (serialized as `"secret"` to match the CLI's JSON; a missing key defaults to `false`). Constructors: `plaintext()`, `secret()`. Implements `From<String>` and `From<&str>`. `Debug` is hand-written so secret values print as `"[secret]"`. Also the `ConfigMap` alias. |
| `event.rs` | `EngineEvent` and sub-types (`PreludeEvent`, `ResourcePreEvent`, `SummaryEvent`, `DiagnosticEvent`) for parsing structured CLI JSON output. |
| `error.rs` | `Error` enum: `CliNotFound`, `CommandFailed`, `Io`, `Json`, `StackNotFound`, `StackAlreadyExists`, `Custom`. `CommandFailed`'s message shows only the `error:` lines from stderr when there are any (`extract_error_lines()`). With `native-engine`, `From<pulumi_engine::Error>` maps `ProgramFailed` to `CommandFailed`. |
| `native.rs` | `NativeStack` — alternative to `Stack` that uses `pulumi-engine` directly instead of CLI subprocess. Feature-gated behind `native-engine`. Lifecycle: `up()`, `preview()`, `destroy()`, `refresh()`. Also `outputs()` (read from the checkpoint) and config methods that persist to `.pulumi-rs/<stack>.config.json`; these are synchronous, unlike their `Stack` counterparts. |

## Result Types

- `UpResult` — stdout, stderr, outputs (`HashMap<String, OutputValue>`), events
- `PreviewResult` — stdout, stderr, events
- `DestroyResult` — stdout, stderr, events
- `RefreshResult` — stdout, stderr, events
- `OutputValue` — JSON value + secret flag
- `StackSummary` — name, current, last_update, resource_count
- `WhoAmIResult` — result of `whoami()`
- `PluginInfo` — one entry from `list_plugins()`

## Secret handling

- `get_all_config()` passes `--show-secrets`, as the upstream automation SDKs do, so secrets come back **decrypted** with `is_secret: true`. Callers that display config must mask them; `pulumi-cli`'s `config ls` does.
- `set_config()` builds its arguments in `set_config_args()`: an explicit `--secret` or `--plaintext`, then `--`, then the value. The separator stops a value that starts with `-` from being read as a CLI flag. Keep the value last and after `--`.
- `ConfigValue`'s `Debug` masks secrets, so they do not leak through `{:?}`, panics or test failures.

## Tests

30 unit tests: `error.rs` (15), `config.rs` (7), `event.rs` (5), `workspace.rs` (3, all on `set_config_args`). `native.rs` has its own tests, compiled only with `--features native-engine`. Nothing here runs the real `pulumi` binary; the end-to-end tests live in the `integration-tests` workspace member.

## Conventions

- All operations are async (tokio), except `NativeStack`'s config and output accessors
- CLI flags like `--yes` and `--skip-preview` are added automatically for non-interactive use
- Error type is hand-written (no `thiserror`), with `From` impls for `std::io::Error` and `serde_json::Error`
