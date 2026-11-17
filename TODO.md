# TODO

## Roadmap

| # | Item | Crate | Effort | Difficulty | Notes |
|---|------|-------|--------|------------|-------|
| 1 | crates.io publishing | all | Medium | Easy | API surface audit (`pulumi/src/lib.rs`, `pulumi-core/src/lib.rs`) and crate-level docs/README needed before publishing. Set `workspace.package.version`, maintain `CHANGELOG.md`, automate publish order with `cargo-release`. Includes a published-artifact smoke test (see section below) — existing integration tests use workspace path deps and don't validate the published tarball. |
| 2 | Codegen provider validation | `pulumi-codegen` | Medium | Medium | Only validated against `pulumi-random` and docker; run against AWS and other large providers that exercise deeply nested modules, complex `$ref` chains, and edge-case type references |
| 3 | Make CI run | repo | Low | Easy | The workflow only triggers on `main`, which does not exist, and would fail on the toolchain pin, clippy and formatting if it did. See "CI Never Runs" below. |
| 4 | Preview / unknown resource outputs | `pulumi-core` | Medium | Medium | Resource outputs are deserialized straight into typed structs with no handling of preview unknowns. Likely breaks `pulumi preview` for new resources; unverified. See section below. |
| 5 | Wire `Output<T>` into resource registration | `pulumi-core`, `pulumi-macros`, `pulumi-codegen` | High | Hard | `Output<T>` is not used by any builder, so there is no dependency tracking or secret propagation between resources. Subsumes item 4 if done fully. See section below. |
| 6 | Encrypt `NativeStack` secret config | `pulumi-automation` | Low | Medium | Secret config values are written to `.pulumi-rs/<stack>.config.json` in plaintext. See `pulumi-automation/TODO.md`. |

## AI-Assisted Development: Effort vs Difficulty

**Effort** and **Difficulty** map to different dimensions of LLM resource consumption when working on these tasks with an AI coding assistant:

- **Effort → Total token throughput.** How many files need to be read, how many edits made, how many tool calls issued. High effort means more cumulative tokens across the session — more code generated, more round trips. This is what causes sessions to hit context limits and trigger compression, potentially losing earlier context. It's largely a function of *breadth* — how many things need to change.

- **Difficulty → Peak context window pressure.** How much information needs to be held *simultaneously* to reason correctly. Hard tasks require reading and understanding multiple interconnected modules before writing a single line. This is a function of *depth* — how many moving parts must be in-context at once to make a correct decision.

Effort predicts how many sessions/messages a task takes, while difficulty predicts how likely any single step is to go wrong.

| # | Item | Effort (throughput) | Difficulty (peak context) |
|---|------|---|---|
| 1 | crates.io publishing | Moderate total — config and process steps | Low peak — each step is independent |
| 2 | Codegen provider validation | Moderate total — run codegen, fix issues iteratively | Moderate peak — need to understand provider schema edge cases while reading generated output |
| 3 | Make CI run | Low total — one workflow file, then a formatting pass and a handful of lint fixes | Low peak — each failure is independent |
| 4 | Preview / unknown resource outputs | Moderate total — reproduce, then touch the register/read paths and their tests | Moderate peak — wire format for unknowns, typed deserialization and preview semantics together |
| 5 | Wire `Output<T>` into resource registration | High total — builders, traits, derive macros, codegen output and every test that constructs inputs | High peak — `Output`, serialization, dependency reporting and the generated-code shape must all be in view to choose an API |
| 6 | Encrypt `NativeStack` secret config | Low total — one module plus tests | Moderate peak — must stay consistent with how the checkpoint's secrets manager derives and stores its key |

---

## MockMonitor Improvements ✓ Done

**Crate:** `pulumi-core`

`MockMonitor`/`TestContextBuilder` extended for finer-grained test assertions:

- **Recording** — added `recorded_reads()` (`ReadResourceRecording`) and
  `recorded_outputs()` (`OutputsRegistration`). `ResourceRegistration` now
  captures `provider`, `providers`, `aliases` (URN form + spec URNs),
  `version`, `plugin_download_url`, `import_id`, `remote`,
  `delete_before_replace`, `retain_on_delete`, `additional_secret_outputs`,
  `replace_on_changes`, `ignore_changes`.
- **Error injection** — extended beyond `register_resource` to
  `read_resource` (keyed by type+name), `invoke` (keyed by token), and
  `call` (keyed by token).
- **Canned responses** — added for `read_resource`, `invoke`, and `call`
  alongside the existing `register_resource` support.
- **Plumbing** — `with_options` now takes a single `MockMonitorOptions`
  struct (parameter list was getting unwieldy). `TestContextBuilder` gains
  `with_read_response/error`, `with_invoke_response/error`,
  `with_call_response/error`. `TestContext` gains `read_resources()`,
  `registered_outputs()`, `called_methods()` accessors.

Tests grew from 113 → 131 (`pulumi-core`).

## Engine Service Handler/Shim Refactor ✓ Done

**Crate:** `pulumi-engine`

`monitor_service.rs` and `engine_service.rs` are split into:

- **Handler methods** (`pub(crate) async fn handle_*`) on the inherent
  `impl ResourceMonitorImpl<P>` / `impl EngineServiceImpl` blocks. Take plain
  prost types, return `Result<T, Status>`. Contain all the actual logic.
- **Tonic trait shims**: 4-line wrappers that unwrap `Request<T>`, call the
  handler, and wrap the result in `Response<T>`. No business logic.

Existing unit tests now call handlers directly (no `Request::new(...)` /
`Response::into_inner()` boilerplate, no need to import the gRPC service
trait). All 67 tests pass.

This is the prerequisite for 5b: `TestEngine` harness + expanded coverage.

## Engine Test Coverage ✓ Done

**Crate:** `pulumi-engine`

Built on top of the 5a refactor. Two new pieces in `test_utils.rs`:

- **`TestEngine`** — in-process harness that binds both gRPC servers to
  ephemeral 127.0.0.1 ports, exposes `monitor_client()` / `engine_client()`
  tonic clients, and shares the `EngineState` with the test so assertions
  can inspect internal state after RPCs. Servers are aborted on `Drop`.
  Connect retry loop handles the spawn-then-bind race.
- **`state_with_prior(...)`** — builds an `EngineState` from a synthetic
  `Checkpoint`, so tests can populate prior-run resources without writing
  to disk. Used to exercise the `register_resource` Same/Update diff paths.

**New coverage** (85 tests total, up from 67):

- **`monitor_service.rs`** — direct handler tests for `read_resource`
  (builtin + custom), `call` (builtin + provider), `register_resource_hook`,
  `register_error_hook`, `signal_and_wait_for_shutdown`. New
  `register_resource` tests for the Same path (prior id preserved), Update
  path (provider.update called), and dry-run Update (provider skipped).
  Two `TestEngine` smoke tests exercise the tonic shim end-to-end
  (`register_resource` and `supports_feature`).
- **`engine_service.rs`** — two `TestEngine` smoke tests for `set_root` /
  `get_root` round-trip and `log` via gRPC.
- **`orchestrator.rs`** — refresh on a custom resource yields a `Same`
  diff via `MockProvider::read`'s default echo. Three subprocess-based
  tests (`/bin/true` / `/bin/false`) verify the full `up` / `preview`
  lifecycle: gRPC servers start, child program runs with `PULUMI_*` env
  vars, servers abort on exit, checkpoint written by `up` but not by
  `preview`, `Error::ProgramFailed` on non-zero exit.

**Default pattern**: call `handle_*` methods directly (fast, easy
`EngineState` assertions). Reach for `TestEngine` only when the test is
about transport — wire encoding, status code propagation, or shim wiring
that handler-level tests can't see.

## Integration Tests ✓ Done

**Crate:** `integration-tests/` (workspace member, `publish = false`)

**Run:**
```
cargo test -p integration-tests --features integration
```
Tests skip gracefully when `pulumi` is not on PATH.

**Architecture:** `TestHarness` in `integration-tests/src/lib.rs` builds the
testdata binary (`cargo build -p pulumi-test-<name>`), creates a throw-away
local file-backend state dir (`PULUMI_BACKEND_URL=file://<tmpdir>`), inits a
`dev` stack via `pulumi-automation`, then drives `up` / `destroy` /
`remove_stack`.  Testdata programs live in `integration-tests/testdata/*/` and
are workspace members (shared `target/`).

**Implemented tests:**
| Test | File | What it validates |
|------|------|-------------------|
| `basic_resource` | `tests/basic_resource.rs` | `random:RandomString` registered; `result` (16-char string) and `length` (16) in outputs |
| `config` | `tests/config.rs` | Plain-text config set before `up`, read via `ctx.require_config`, exported and asserted |
| `secrets` | `tests/secrets.rs` | `random:RandomPassword` registered; `result` marked `secret: true`; `length` output plain |

**Remaining coverage gaps** (future work):
- `Output<T>` combinator chains (`map`, `flat_map`, `all`)
- Stack references
- Resource options (parent, depends_on, protect, aliases)
- Component resources with `register_resource_outputs`
- Provider function invocations (`invoke`)
- Error cases (missing required config, invalid inputs)

## Code Generation / Provider SDKs

**Purpose:** Other Pulumi SDKs (Go, TypeScript, Python, .NET) ship typed provider
packages (`pulumi-aws`, `pulumi-gcp`, etc.) generated from provider schemas. Without
codegen, Rust users must hand-write every `Resource` impl, `Inputs` struct, and
`Outputs` struct — hundreds of types per provider, error-prone and impossible to
keep in sync with upstream schema changes.

**How it would work:**

1. **Schema source.** Every Pulumi provider exposes a JSON schema via the
   `GetSchema()` gRPC call (defined in `provider.proto`). The schema describes
   every resource type, function, their input/output properties, types, defaults,
   deprecations, and descriptions.

2. **`pulumi-codegen` tool** (a standalone binary or build-script library) that:
   - Fetches or reads a provider schema JSON file
   - For each **resource**: generates an `Inputs` struct (`#[derive(Serialize)]`),
     an `Outputs` struct (`#[derive(Deserialize, Clone)]`), and a unit struct with
     `#[derive(Resource)]` plus the appropriate `#[pulumi(...)]` attributes
   - For each **function**: generates `Args` and `Returns` structs plus a unit
     struct with `#[derive(ProviderFunction)]`
   - Handles nested object types, enums, arrays, maps, optional fields
   - Generates doc comments from schema descriptions
   - Handles naming collisions and Rust keyword escaping

3. **Output structure** per provider:
   ```
   pulumi-aws/
   ├── Cargo.toml          # depends on pulumi (with macros feature)
   ├── src/
   │   ├── lib.rs
   │   ├── s3/
   │   │   ├── mod.rs
   │   │   ├── bucket.rs        # Bucket, BucketArgs, BucketOutputs
   │   │   └── bucket_object.rs
   │   ├── ec2/
   │   │   ├── mod.rs
   │   │   └── instance.rs
   │   └── ...
   ```

4. **Schema type mapping:**
   | Pulumi Schema Type | Rust Type |
   |---|---|
   | `string` | `String` |
   | `integer` | `i64` |
   | `number` | `f64` |
   | `boolean` | `bool` |
   | `array<T>` | `Vec<T>` |
   | `map<T>` | `HashMap<String, T>` |
   | `object` (named) | Generated struct |
   | `union` (`oneOf`) | Generated `#[serde(untagged)]` enum in `crate::types` |
   | `Asset` / `Archive` | `pulumi::Asset` / `pulumi::Archive` |
   | optional property | `Option<T>` |

5. **Versioning:** Generated crates pin to a specific provider plugin version
   via the `VERSION` constant, so `pulumi-aws = "6.50.0"` always uses the
   matching provider binary.

## crates.io Publishing and Versioning

**Purpose:** Let users `cargo add pulumi` instead of pointing at a git repo. A
clear versioning story builds trust and lets the ecosystem (provider SDKs,
third-party integrations) depend on stable interfaces.

**What needs to happen:**

1. **Stabilize public API surface.** Audit re-exports in `pulumi/src/lib.rs` and
   `pulumi-core/src/lib.rs`. Mark internal modules `pub(crate)`. Ensure trait
   signatures, builder APIs, and error types are ones we're willing to support.

2. **Workspace-level versioning.** All crates in the workspace should share a
   version (or at least have a clear compatibility matrix). Use
   `workspace.package.version` in the root `Cargo.toml`.

3. **Changelog and semver.** Follow semver strictly — breaking changes bump major.
   Maintain a `CHANGELOG.md` tracking what changed per release.

4. **Publish order matters.** `pulumi-core` must publish before `pulumi-macros`
   and `pulumi`, since they depend on it. Automate this with `cargo-release` or
   a CI workflow.

5. **Feature flags documentation.** Document `macros` feature and any future
   features (`test-support`, etc.) in crate-level docs and README.

6. **Published-artifact smoke test.** The existing `integration-tests/` crate
   uses workspace `path = "..."` deps, so it verifies the SDK source but not
   the published tarball. Things that would slip through: a file accidentally
   `exclude`d from `Cargo.toml`, a `proto/` directory missing from the package,
   a feature flag that compiles in-tree but fails when fetched from crates.io,
   a path-only dev-dependency leaking into a non-dev section.

   Two pieces of work:
   - **Pre-release:** `cargo publish --dry-run` for every workspace crate as a
     CI check on release PRs. Catches packaging bugs without burning a version.
   - **Post-publish:** a separate smoke test (CI job or `xtask`) that creates a
     temp project *outside* the workspace, runs `cargo add pulumi = "X.Y.Z"`,
     builds a minimal program against the published version, and runs it
     through `pulumi up` against the file backend. Must not see the workspace
     so the consumption path is exercised honestly.

   Optionally pin one `integration-tests/testdata/*` program to the published
   version once available, so day-to-day `cargo test` exercises consumption
   too — at the cost of testdata lagging HEAD.

## `Output<T>` Metadata Propagation ✓ Done

**Crate:** `pulumi-core`

`Output<T>` now resolves to its value *together with* its metadata (known, secret, deps), instead of keeping the metadata in shared mutable state next to the future. Combinators merge metadata at the moment they produce the value, so `flat_map` picks up the inner output's secretness and knownness.

- `is_known()`, `is_secret()` and `dependencies()` are async and wait for the output to resolve, matching the Node, Python and Go SDKs. A rejected output reports not known, not secret, no deps.
- `OutputResolver::add_deps` and `set_secret` take `&mut self`; `add_deps` is no longer async. What they set takes effect at resolution, so outputs derived *before* the resolver was marked secret or unknown now see it.
- Deps are deduplicated in first-seen order and no longer sit behind a lock.
- An unknown output is a state rather than an error internally: combinators skip their callback and stay unknown. Awaiting one still returns `Err("output value is unknown")`.

The `JoinHandle` approach planned here was dropped: a spawned task does not run until the caller yields, so a synchronous `is_secret()` straight after `flat_map` would still have been wrong. Eager execution of `map`/`flat_map` callbacks is tracked separately below.

This replaces an interim fix from July 2026 (PR #24), which kept the accessors synchronous: `flat_map` ran its callback immediately when the source had already resolved (`now_or_never()`) and spawned a task otherwise. That was correct for resolved sources, but for a pending source the metadata only converged shortly *after* resolution, so a synchronous `is_secret()` could still briefly report `false` for a secret.

## Toolchain Bump to `nightly-2026-11-14` ✓ Done

**Crate:** all

The pin moved from `nightly-2026-03-03` (1.96-nightly) to `nightly-2026-11-14` (1.102-nightly). This crosses two compiler switches that are on by default on nightly: the next-generation trait solver (August 2026) and the Polonius Alpha borrow checker (August 2026). Neither broke anything: the workspace compiles without warnings and all tests pass.

- Cargo's new build-directory layout (1.100) is in effect. Final binaries still land in `target/debug/`, so the `integration-tests/testdata/*/Pulumi.yaml` paths are unaffected.
- Clippy now applies `result_large_err` to `async fn` (fixed in 1.98), which surfaced the error-size issue below.
- The integration tests were **not** run on the new toolchain; see the open item below.

## Error Size (`result_large_err`) ✓ Done

**Crate:** `pulumi-core`, `pulumi-engine`

`tonic::Status` is 176 bytes, over clippy's 128-byte limit for `Err` variants. `pulumi::Error` carried one inline, so every user `async fn` returning `pulumi::Result<T>` got a clippy warning on Rust 1.98+; the crate-wide `allow` in `pulumi-core` hid it only inside that crate.

- `pulumi_core::Error::Rpc` and `pulumi_engine::Error::ProviderStatus` now hold `Box<tonic::Status>`. The `From<tonic::Status>` impls box it, so `?` is unchanged.
- The `#![allow(clippy::result_large_err)]` in `pulumi-core/src/lib.rs` is removed.
- A workspace `clippy.toml` sets `large-error-ignored = ["tonic::Status"]` for the engine's gRPC handlers, which must return `Status` directly because tonic's service traits require it.

## Config Secret Handling in CLI and Workspace ✓ Done

**Crate:** `pulumi-cli`, `pulumi-automation`

From PR #24 (July 2026):

- `config ls` masks secret values (text and JSON) unless `--show-secrets` is passed, matching the upstream CLI.
- `LocalWorkspace::get_all_config` passes `--show-secrets`, matching the upstream automation SDKs.
- `set_config` passes the value after a `--` separator with an explicit `--secret`/`--plaintext` flag, so a value that looks like a flag cannot be interpreted as one.

## `output.rs` — Dep Representation

**Crate:** `pulumi-core`

Deps are a `Vec<String>` of URNs carried in the resolved value and merged with a linear `contains` check. Evaluate whether URN strings are the right representation (the other SDKs track resource objects) and whether merging needs a set for outputs with very many deps.

## Audit `std::sync` Usage in Async Contexts

**Crate:** all

Search for uses of `std::sync::Mutex`, `std::sync::RwLock`, and similar primitives held across `.await` points. These can deadlock or cause unexpected blocking on a tokio worker thread. Replace with `tokio::sync` equivalents where appropriate.

## Audit `unwrap()` and Other Potential Panics

**Crate:** all

Search the codebase for `unwrap()`, `expect()`, `panic!`, and indexing operations that can panic. For each, determine whether the panic is truly unreachable (and should be replaced with a clearer `unreachable!()` or removed) or whether it represents a real failure mode that should surface as a `Result` or `Error` instead.

## `output.rs` — Audit `.clone()` Usage

**Crate:** `pulumi-core`

The metadata refactor removed the `Arc`/lock clones, but two kinds remain and are worth auditing:

- **Per-awaiter clones of the resolved data.** `Shared` hands each awaiter a clone of what the future resolved to, which is now the value *and* its deps `Vec`. Every combinator and every `get()` pays for that.
- **Accessors clone the whole value to read one flag.** `is_known()`, `is_secret()` and `dependencies()` each await a clone of the full resolved data, including `T`, just to return a `bool` or the deps.

Putting the resolved data behind an `Arc` (so awaiters share it) would address both, at the cost of one more allocation per output.

## Automated Code Review — Compare Against All SDKs

**Crate:** `scripts/`

The review harness currently compares the Rust SDK against a single upstream SDK. Expand coverage to compare against all reference SDKs (Go, TypeScript, Python, .NET) to catch divergences that appear in only one language's implementation.

## Automated Code Review — Fix Remaining Identified Issues

**Crate:** various

The initial automated review pass identified issues beyond secret propagation (which was addressed first). Work through the remaining flagged items from `review-output/` and address them.

Status as of 2026-11-16 (`python scripts/sdk_review.py --report`):

- **Stale, need re-review before acting on them:** `output`, `context`, `error`, `serde`, `automation_workspace`, `automation_config`. All six were changed after their review.
- **Verdict "MAJOR GAPS":** `asset` and `automation_workspace`. These are the two to work on first once the reviews are fresh.
- The May review of `output` predates the metadata refactor. Of its findings, metadata loss in `all`/`all2`/`all3` and in `flat_map` is fixed; still open are the missing utilities (`unsecret`, an `Input<T>` type, `concat`/`interpolate`, JSON helpers) and `resolve_unknown` requiring a placeholder value.

## Eager Execution of `map` / `flat_map` Callbacks

**Crate:** `pulumi-core`

Outputs here are lazy: a `map` or `flat_map` callback only runs when something awaits the result. In the Node, Python and Go SDKs an `apply` callback runs as soon as its source resolves, whether or not anything consumes the result, and programs rely on that for side effects. Decide whether to match that (spawning a task per combinator, which needs a runtime at construction time and a way to wait for outstanding tasks before the program exits) or to document the difference.

## Run Integration Tests on the New Toolchain

**Crate:** `integration-tests`

The opt-in integration tests (`cargo test -p integration-tests --features integration`) have not been run since the bump to `nightly-2026-11-14`, the `Output<T>` refactor, or the PR #24 config changes, because they need the `pulumi` binary on PATH. Run them on a machine that has it. `secrets` and `config` are the ones most likely to be affected.

## Remaining Clippy Warnings

**Crate:** `pulumi-codegen`, `pulumi-engine`, `pulumi-automation`

Thirteen style warnings remain with `--all-features`, all mechanical:

- `pulumi-codegen`: four collapsible `if`s (`ir.rs`, `naming.rs`) and one derivable `Default` (`emit.rs`).
- `pulumi-engine`: one function with eight arguments (`monitor_service.rs`, `apply_transforms`) and four test helpers in `test_utils.rs` that could be `async fn`.
- `pulumi-automation`: three in `native.rs`, which is only compiled with the `native-engine` feature — a redundant closure and two needless `.to_string()` calls.

CI runs clippy with `-D warnings` but without `--all-targets`, so it sees the seven of these that are in non-test code (five in `pulumi-codegen`, `apply_transforms`, and the redundant closure in `native.rs`). Those have to go before CI can pass; the other six are in test code.

## CI Never Runs

**Crate:** repo (`.github/workflows/ci.yml`)

The workflow triggers on pushes and pull requests to `main`, but the repository's branches are `master` and `nightly`; there is no `main`. So nothing has been checking pushes or PRs. A compile error in `pulumi-cli` (a missed rename) sat on `nightly` from May to November 2026 for this reason.

To fix, in order:

1. Change the triggers to the branches actually in use (`nightly`, and `master` if it is still meant to receive work).
2. Stop hard-coding the toolchain. The workflow names `nightly-2026-03-03` twice (the cache key and the install step) while `rust-toolchain.toml` now says `nightly-2026-11-14`. Read the pin from `rust-toolchain.toml` so there is one source of truth.
3. Make the checks pass. As of 2026-11-16, build and test pass with `--all-features`, but `cargo clippy ... -- -D warnings` fails on the seven non-test warnings above and `cargo fmt --all -- --check` fails on 30 files. Consider adding `--all-targets` to the clippy step so test code is linted too. The formatting fix is one `cargo fmt --all`, best done as its own commit so it doesn't bury real changes.
4. Decide whether the integration tests should run in CI. They need the `pulumi` binary and currently skip without it, so today CI would report them as passing without running them.

## Preview / Unknown Resource Outputs

**Crate:** `pulumi-core`

`ResourceBuilder`, `ReadBuilder` and `RemoteComponentBuilder` deserialize the engine's response straight into the resource's typed `Outputs` struct (`serde_json::from_value(result.outputs)` in `resource.rs`). Nothing on that path checks `ctx.is_dry_run()` or the unknown sentinel; `serde::is_unknown` exists but is only used in its own tests.

During a preview, the engine cannot know the outputs of a resource that has not been created yet. Expected consequences, **inferred from the code and not yet reproduced**:

- a required non-string output field fails to deserialize, so the program errors during `pulumi preview`;
- a string field silently receives the sentinel UUID (`04da6b54-80e4-46f7-96ec-b56ff0331ba9`) as if it were a real value.

Nothing would catch either today: `Stack::up` passes `--skip-preview`, the integration tests only run `up`, and the one unit test for preview mode (`test_preview_mode` in `test_support.rs`) uses an outputs struct whose only field is an `Option`, so `MockMonitor`'s empty preview outputs deserialize without complaint.

First step: reproduce with a real `pulumi preview` (or the native engine with `dry_run`) against one of the `integration-tests/testdata` programs. The proper fix is the next item; a stopgap would be to detect unknowns before deserializing and surface a clear error or an explicit "unknown" result.

## Wire `Output<T>` into Resource Registration

**Crate:** `pulumi-core`, `pulumi-macros`, `pulumi-codegen`

`Output<T>` is correct in isolation but nothing uses it. Outside `output.rs`, no code calls its accessors or combinators:

- Resource and invoke builders take plain `Serialize` inputs, so an input cannot be another resource's not-yet-known output.
- They return plain deserialized `Outputs`, so outputs carry no known/secret/dependency metadata.
- `RegisterResourceRequest.property_dependencies` is always sent empty (`resource.rs`), so the engine learns about dependencies only through explicit `depends_on`.
- Secretness of a resource output does not follow the value into another resource's inputs.

In the Node, Python and Go SDKs these are the same mechanism: resource properties are `Output`s, inputs accept `Input<T>` (a value or an `Output`), and serializing the inputs is what collects dependencies and secrets. The May 2026 review of `output` flagged the missing `Input<T>` type for the same reason.

This is a design decision before it is an implementation task, because it changes the shape of every generated resource (`Args` fields become `Input<T>`, `Outputs` fields become `Output<T>`) and therefore `pulumi-macros` and `pulumi-codegen` too. It should be settled before publishing, since it cannot be changed compatibly afterwards.

## Dependency Refresh Before Publishing

**Crate:** all

`Cargo.lock` has not been updated since May 2026. Before publishing:

- Check how far behind the direct dependencies are. `tonic` 0.12 and `prost` 0.13 are believed to be behind their current releases (not verified). This matters more than usual because tonic types appear in the public `Error` enums, so the tonic version is part of the public API.
- Turn on Cargo's dependency cooldown first (`global-min-publish-age` under `[registry]` in `.cargo/config.toml`, available since Cargo 1.100), so `cargo update` skips versions published in the last few days. The August 2026 `arrayref` supply-chain attack was live for under two hours; a cooldown would have skipped it.
- Update in small steps and run `cargo test --workspace --all-features` between them.

## `#[non_exhaustive]` on Public Error Enums

**Crate:** `pulumi-core`, `pulumi-automation`, `pulumi-engine`

The public `Error` enums are plain enums. Once published, adding a variant is a breaking change for anyone who matches on them exhaustively. Mark them `#[non_exhaustive]` before the first release. The same question applies to other public enums that are likely to grow (`Asset`, `Archive`, `EngineEvent`).

## Stable Rust Support — Non-Goal

**Stable Rust is a non-goal until the next-generation trait solver ships.**

The SDK relies on `impl_trait_in_assoc_type` (rust-lang/rust#63063), which
requires nightly. (Edition 2024, which the workspace also uses, has been stable
since Rust 1.85.) Rather than degrading the API with boxed futures or manual
future types as a workaround, we are waiting for the next-gen trait solver to
land on stable — at which point `impl_trait_in_assoc_type` can stabilize.

Status as of November 2026: the next-gen trait solver has been on by default on
nightly since August 2026, and the Rust blog says the team plans to stabilize
it "in the next months". The tracking issue for `impl_trait_in_assoc_type` is
still open and marked blocked on that work. Rust 1.100 is the current stable.

The nightly toolchain is pinned in `rust-toolchain.toml` (`nightly-2026-11-14`)
so builds are reproducible regardless.
