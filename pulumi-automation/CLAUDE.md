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
- **thiserror** — error type definitions
- **clap** is NOT a dependency (that's in `pulumi-cli`)

## Module Guide

| Module | Purpose |
|--------|---------|
| `workspace.rs` | `LocalWorkspace` — main entry point. Wraps a project directory. Methods: `create_stack()`, `select_stack()`, `create_or_select_stack()`, `list_stacks()`, `remove_stack()`. Config methods: `get_config()`, `set_config()`, `get_all_config()`, `remove_config()`. Supports env injection via `with_env()`. |
| `stack.rs` | `Stack` — represents a deployed stack. Lifecycle: `up()`, `preview()`, `destroy()`, `refresh()`. State: `outputs()`, `export_state()`, `import_state()`. Config delegation to workspace. |
| `cmd.rs` | `run_pulumi_cmd(work_dir, args, env)` — spawns `pulumi` subprocess with `PULUMI_NON_INTERACTIVE=true` and `NO_COLOR=1`. Returns `CmdOutput { stdout, stderr }`. |
| `config.rs` | `ConfigValue` — value + secret flag. Constructors: `plaintext()`, `secret()`. Implements `From<String>` and `From<&str>`. |
| `event.rs` | `EngineEvent` and sub-types (`PreludeEvent`, `ResourcePreEvent`, `SummaryEvent`, `DiagnosticEvent`) for parsing structured CLI JSON output. |
| `error.rs` | `Error` enum: `CliNotFound`, `CommandFailed`, `Io`, `Json`, `StackNotFound`, `StackAlreadyExists`, `Custom`. |

## Result Types

- `UpResult` — stdout, stderr, outputs (`HashMap<String, OutputValue>`), events
- `PreviewResult` — stdout, stderr, events
- `DestroyResult` — stdout, stderr, events
- `RefreshResult` — stdout, stderr, events
- `OutputValue` — JSON value + secret flag
- `StackSummary` — name, current, last_update, resource_count

## Conventions

- All operations are async (tokio)
- CLI flags like `--yes` and `--skip-preview` are added automatically for non-interactive use
- Error type uses `thiserror` for ergonomic `From` conversions
