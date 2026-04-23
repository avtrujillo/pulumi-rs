# TODO — pulumi-cli

## Native Engine Support

**DONE.** `--native` flag (and `native-engine` feature) added to `up`, `preview`,
`destroy`, and `refresh` commands. When passed, the command uses `NativeStack` from
`pulumi-automation` instead of spawning the Go-based `pulumi` CLI binary. A
`--program` flag selects the binary path; without it the current working directory
is used.

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

## Tests

- 36 unit tests added to `main.rs` covering command output format and diff rendering
- Integration tests that verify end-to-end command behavior against a real workspace
  remain as future work

## Edition

Uses edition 2024 (nightly). Stable Rust is a non-goal until the next-gen
trait solver ships. See the root TODO.md.
