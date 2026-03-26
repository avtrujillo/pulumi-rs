# pulumi-cli

Rust-native CLI for Pulumi stack operations, built on `pulumi-automation`. Produces the `pulumi-rs` binary.

## Build & Run

```bash
cargo build -p pulumi-cli
cargo run -p pulumi-cli -- --help
cargo run -p pulumi-cli -- up --stack dev
```

## Dependencies

- **pulumi-automation** — all stack/workspace operations
- **clap** (v4, derive) — CLI argument parsing
- **tokio** (rt-multi-thread, macros) — async runtime
- **serde_json** — JSON output formatting

## CLI Commands

```
pulumi-rs [--cwd <PATH>] <COMMAND>
```

| Command | Description |
|---------|-------------|
| `up --stack <NAME>` | Create or update resources |
| `preview --stack <NAME>` | Show pending changes |
| `destroy --stack <NAME>` | Tear down all resources |
| `refresh --stack <NAME>` | Sync stack state from cloud |
| `stack init <NAME>` | Initialize a new stack |
| `stack ls` | List all stacks |
| `stack rm <NAME> [--force]` | Remove a stack |
| `stack output --stack <NAME>` | Display stack outputs as JSON |

## Source Structure

Single file: `src/main.rs` (~190 lines). Uses clap derive for a `Cli` struct with `Commands` enum. Each command variant maps directly to `LocalWorkspace` / `Stack` methods from `pulumi-automation`. Errors are printed to stderr with exit code 1.

## Architecture

```
CLI args (clap) -> Commands enum -> LocalWorkspace / Stack methods -> pulumi subprocess
```

The CLI is a thin translation layer between user input and the automation API.
