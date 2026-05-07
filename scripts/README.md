# SDK Review Harness

Pre-publication review tooling that compares each Rust SDK module against its Go, Node.js, and Python counterparts in the upstream Pulumi SDK (in the `upstream/` submodule).

## Quick start

```bash
# 1. Install dependencies (Python 3.9+)
pip install -r scripts/requirements.txt

# 2. Run the first review pass (reviews all 15 modules, ~2–5 min per module)
python scripts/sdk_review.py --all

# 3. Check the results
python scripts/sdk_review.py --report
```

Expected `--report` output after a full run:

```
SDK Review Progress — 15 modules total

Module                    Status       Date         Verdict
----------------------------------------------------------------------
output                    done         2026-05-06   MINOR GAPS
resource                  done         2026-05-06   SIGNIFICANT GAPS
context                   done         2026-05-06   MINOR GAPS
invoke                    done         2026-05-06   MINOR GAPS
log                       done         2026-05-06   COMPLETE
...

15/15 complete
```

After editing a Rust file, re-run `--all` and only the modules whose files changed get re-reviewed:

```bash
# Edit pulumi-core/src/resource.rs, then:
python scripts/sdk_review.py --all
# → skips the 14 unchanged modules, re-reviews resource
```

## Setup

**Requirements:** Python 3.9+, Claude Code installed and authenticated. No separate API key needed — the script invokes the `claude` CLI using your existing Claude Code session.

```bash
pip install -r scripts/requirements.txt
```

The `upstream/` submodule (`https://github.com/avtrujillo/pulumi`) is public — no credentials required. If it hasn't been checked out yet (fresh clone, Claude Code Web session, CI), the script detects this automatically and runs:

```bash
git submodule update --init --depth 1 upstream
```

You don't need to do anything manually.

## Common workflows

### First run — review everything

```bash
python scripts/sdk_review.py --all
```

Reviews all 15 modules in order, saving results to `review-output/YYYY-MM-DD/`. Each module takes 2–5 minutes. With 15 modules this is a 30–75 minute run; use `--max` or `--token-budget` to break it across sessions:

```bash
# Review at most 8 modules, then stop (safe to re-run — resumes where it left off)
python scripts/sdk_review.py --all --max 8
```

### Check status

```bash
python scripts/sdk_review.py --report
```

Shows all 15 modules with their current status (`pending`, `done`, `stale`, `error`), review date, and verdict.

### Review a single module

```bash
python scripts/sdk_review.py --module output
```

Skips the module if it's already up-to-date. Add `--force` to re-review regardless:

```bash
python scripts/sdk_review.py --module output --force
```

### After editing Rust source

```bash
# Edit one or more files, then:
python scripts/sdk_review.py --all
```

`--all` automatically detects which modules have changed files (via SHA256 hashes stored in `progress.json`) and only re-reviews those. Unchanged modules are skipped.

### Cap spending for a session

```bash
# Stop after consuming 500K input tokens across the batch
python scripts/sdk_review.py --all --token-budget 500000
```

Useful when running unattended overnight. The script prints how many tokens were used before stopping, and picks up where it left off on the next run.

### Read a review

Results are plain markdown in `review-output/YYYY-MM-DD/<module>.md`:

```bash
cat review-output/2026-05-06/output.md
```

## Module mapping

[`sdk-review-modules.yml`](sdk-review-modules.yml) maps each Rust source file to its upstream counterparts:

```yaml
- name: output
  description: "Output<T> type, combinators (map, flat_map, all), resolution"
  rust: pulumi-core/src/output.rs
  go:
    - upstream/sdk/go/pulumi/types.go
    - upstream/sdk/go/pulumi/types_builtins.go
  nodejs:
    - upstream/sdk/nodejs/output.ts
  python:
    - upstream/sdk/python/lib/pulumi/output.py
```

15 modules are defined — 11 in `pulumi-core` and 4 in `pulumi-automation`.

## Output format

Each review file contains:

- Metadata header (date, model, token usage, source file)
- **Summary** — 2-3 sentence overall assessment
- **Missing Features** — features present upstream but absent from Rust
- **Behavioral Divergences** — semantic differences from the upstream SDKs
- **API Surface Gaps** — public types, methods, or traits missing from Rust
- **Recommendations** — prioritized list of what to implement first
- **Verdict** — one of `COMPLETE`, `MINOR GAPS`, `SIGNIFICANT GAPS`, or `MAJOR GAPS`

The verdict is extracted and stored in `progress.json` for the report view.

## How it works

Each review is a single `claude --print` subprocess call structured as:

1. **System prompt** — passed via `--system-prompt`, asks Claude to act as a senior Rust reviewer and output a structured report (Summary, Missing Features, Behavioral Divergences, API Surface Gaps, Recommendations, Verdict).
2. **User prompt** — all Go, Node.js, and Python upstream files for the module concatenated with the Rust source, piped to the CLI via stdin.

The CLI runs non-interactively (`--print`), with tools disabled (`--tools ""`) and session persistence off (`--no-session-persistence`), so each review is a clean, self-contained call. Individual files are truncated at 80KB to stay within context limits.

## Progress tracking

`review-output/progress.json` records the status, date, verdict, output path, and SHA256 file hashes for each module:

```json
{
  "output": {
    "status": "done",
    "date": "2026-05-06",
    "verdict": "MINOR GAPS",
    "output": "review-output/2026-05-06/output.md",
    "hashes": {
      "upstream_sha": "a1b2c3d4e5f6a7b8",
      "pulumi-core/src/output.rs": "deadbeef12345678",
      "upstream/sdk/go/pulumi/types.go": "cafebabe87654321"
    }
  },
  "resource": {
    "status": "pending"
  }
}
```

A module is marked **stale** (and re-queued for `--all`) whenever any of its hashes differ from the stored values — either because you edited the Rust file or because the upstream submodule was updated.

## Pacing large runs

With 15 modules and ~2–5 minutes per review, a full run takes 30–75 minutes. Use `--max` to break it across sessions — progress is saved after each module so you can always resume:

```bash
# First session: review 8 modules
python scripts/sdk_review.py --all --max 8

# Next session: picks up the remaining 7 automatically
python scripts/sdk_review.py --all
```

## Nightly batch runs with Claude Code routines

The review harness is designed to run unattended while you sleep. Claude Code's built-in scheduler (the `CronCreate` tool) fires a prompt at a set time; Claude then runs the script via its Bash tool.

### Setting up the nightly routine

Type this into Claude Code once per session:

> Set up a nightly routine at 2:23 AM that runs `python scripts/sdk_review.py --all --max 8` in the `/home/avtrujillo/Workspace/pulumi` directory, then prints the progress report.

Claude will create a recurring cron job that fires at 2:23 AM and returns a job ID like `cron_abc123`. Keep that ID — you'll need it to cancel the job.

To review fewer modules per night, adjust `--max`:

> Set up a nightly routine at 2:23 AM that runs `python scripts/sdk_review.py --all --max 4` in `/home/avtrujillo/Workspace/pulumi`, then prints the progress report.

### Checking active routines

> List my active Claude Code routines.

Claude will call `CronList` and show you all scheduled jobs with their IDs and next fire times.

### Cancelling a routine

> Cancel the nightly SDK review routine. (The job ID is cron_abc123.)

Or just:

> Cancel all my active routines.

### Important limitations

- **Session-scoped** — routines die when the Claude Code session ends (browser tab closed, app quit, CLI exited). Re-create the routine at the start of each session where you want overnight reviews to run.
- **7-day auto-expiry** — recurring jobs fire one final time after 7 days, then auto-delete. Re-create weekly if you want continuous nightly coverage.
- **Idle only** — the job only fires when Claude Code is idle (not mid-conversation). If you're actively using Claude at 2:23 AM, the job is deferred until the next idle window.

### Typical overnight session

Before bed:

```
You:   Set up a nightly routine at 2:23 AM to run `python scripts/sdk_review.py
       --all --max 8` in /home/avtrujillo/Workspace/pulumi and print the report.
Claude: Done — job cron_abc123 will fire at 2:23 AM. Good night.
```

In the morning, check results:

```bash
python scripts/sdk_review.py --report
cat review-output/$(date +%Y-%m-%d)/output.md   # read a specific review
```

## Planned improvements

These are deferred until after the first full review run establishes baseline verdicts:

- **Differential review mode** (`--mode update`) — instead of re-sending full file contents for stale modules, send a `git diff` of what changed and ask Claude to revise the previous verdict. Cheaper and faster for small iterative fixes.
- **Larger provider validation for `pulumi-codegen`** — the code generator has only been tested against `pulumi-random`; needs validation against docker, AWS, and other providers that exercise deeply nested modules and complex type references. See `pulumi-codegen/CLAUDE.md`.
