# TODO — pulumi-cli

## Native Engine Support

**DONE.** `--native` flag (and `native-engine` feature) added to `up`, `preview`,
`destroy`, and `refresh` commands. When passed, the command uses `NativeStack` from
`pulumi-automation` instead of spawning the Go-based `pulumi` CLI binary. A
`--program` flag selects the program binary and is required with `--native`;
without it the command exits with an error.

## Additional Commands

**DONE.** Added:
- `whoami` — shows current identity via `LocalWorkspace::whoami()`
- `logs --stack <NAME>` — shows resource logs via `LocalWorkspace::logs()`
- `plugin ls` — lists installed provider plugins
- `plugin install <NAME> <VERSION>` — installs a provider plugin
- `plugin rm <NAME>` — removes a provider plugin

## Output Formatting

**DONE.**
- Colored `Error:` prefix on all error messages (`colored` crate)
- `format_diff_events()` renders `ResourcePreEvent` list as colored `+`/`-`/`~` diffs
  during `preview` output

## Config Commands and Secret Masking

**DONE.** `config get` / `set` / `rm` / `ls` and `stack export` / `import` are
implemented. Since PR #24 (July 2026), `config ls` masks secret values in both
text and JSON output unless `--show-secrets` is passed, matching the upstream
`pulumi` CLI.

## Tests

- 37 unit tests in `main.rs` covering command-line parsing for every command
  and diff rendering
- Integration tests that verify end-to-end command behavior against a real workspace
  remain as future work. In particular, nothing tests that `config ls` actually
  masks a secret: the `--show-secrets` test only checks that the flag parses.

## Toolchain

Uses edition 2024, which is stable. This crate itself needs nothing from
nightly, but the workspace is pinned to nightly because `pulumi-core` uses
`impl_trait_in_assoc_type`. See the root TODO.md.
