# pulumi-cli

Rust-native CLI for Pulumi stack operations, built on `pulumi-automation`. Produces the `pulumi-rs` binary.

## Build & Run

```bash
cargo build -p pulumi-cli
cargo build -p pulumi-cli --features native-engine   # enables --native
cargo test -p pulumi-cli                             # 37 unit tests
cargo run -p pulumi-cli -- --help
cargo run -p pulumi-cli -- up --stack dev
```

## Features

- **`native-engine`** — Forwards to `pulumi-automation/native-engine`. Without it, passing `--native` prints an error telling the user to recompile with the feature, and exits with code 1.

## Dependencies

- **pulumi-automation** — all stack/workspace operations
- **clap** (v4, derive) — CLI argument parsing
- **tokio** (rt-multi-thread, macros) — async runtime
- **serde_json** — JSON output formatting
- **colored** — colored `Error:` prefix and diff markers

## CLI Commands

```
pulumi-rs [--cwd <PATH>] <COMMAND>
```

| Command | Description |
|---------|-------------|
| `up --stack <NAME> [--json] [--native --program <BIN>]` | Create or update resources |
| `preview --stack <NAME> [--json] [--native --program <BIN>]` | Show pending changes |
| `destroy --stack <NAME> [--json] [--native --program <BIN>]` | Tear down all resources |
| `refresh --stack <NAME> [--json] [--native --program <BIN>]` | Sync stack state from cloud |
| `stack init <NAME>` | Initialize a new stack |
| `stack ls [--json]` | List all stacks |
| `stack rm <NAME> [--force]` | Remove a stack |
| `stack output --stack <NAME>` | Display stack outputs as JSON |
| `stack export --stack <NAME>` | Print the stack's deployment state as JSON |
| `stack import --stack <NAME> [--file <PATH>]` | Import deployment state from a file, or stdin if omitted |
| `config get <KEY> --stack <NAME> [--json]` | Get a configuration value |
| `config set <KEY> <VALUE> --stack <NAME> [--secret]` | Set a configuration value |
| `config rm <KEY> --stack <NAME>` | Remove a configuration value |
| `config ls --stack <NAME> [--json] [--show-secrets]` | List configuration values |
| `whoami [--json]` | Show the logged-in Pulumi user |
| `logs --stack <NAME>` | Fetch log entries for a stack |
| `plugin ls [--json]` | List installed plugins |
| `plugin install <KIND> <NAME> <VERSION>` | Install a plugin |
| `plugin rm <KIND> <NAME> [VERSION]` | Remove a plugin (all versions if none given) |

`--native` runs the operation on the Rust-native engine via `NativeStack` instead of spawning the `pulumi` binary; it requires `--program`, the path of the program binary to run.

## Secret masking in `config ls`

`LocalWorkspace::get_all_config` returns secrets decrypted, so `config ls` masks them itself: unless `--show-secrets` is passed, every secret's value is replaced with `[secret]` before printing, in both text and JSON output. With `--show-secrets` the real value is printed followed by a ` [secret]` marker (text mode). Any new command that prints config must do the same masking.

## Source Structure

Single file: `src/main.rs` (~1040 lines, roughly the last 380 of them tests). Uses clap derive for a `Cli` struct with a `Commands` enum and nested `StackCommands`, `ConfigCommands` and `PluginCommands`. Each command variant maps directly to `LocalWorkspace` / `Stack` / `NativeStack` methods from `pulumi-automation`. Errors are printed to stderr with a colored `Error:` prefix and exit code 1. `format_diff_events()` renders resource events as colored `+` / `-` / `~` lines for `preview`.

## Tests

37 unit tests in `main.rs`. Most parse a command line with clap and assert on the resulting `Commands` value, so adding a field to a command variant (as `--show-secrets` did) breaks the exhaustive patterns in existing tests until they are updated. Three cover `format_diff_events`. None of them run a real stack operation.

## Architecture

```
CLI args (clap) -> Commands enum -> LocalWorkspace / Stack methods -> pulumi subprocess
```

The CLI is a thin translation layer between user input and the automation API.
