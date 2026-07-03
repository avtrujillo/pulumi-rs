# TODO

## Roadmap

| # | Item | Crate | Effort | Difficulty | Notes |
|---|------|-------|--------|------------|-------|
| 1 | crates.io publishing | all | Medium | Easy | API surface audit (`pulumi/src/lib.rs`, `pulumi-core/src/lib.rs`) and crate-level docs/README needed before publishing. Set `workspace.package.version`, maintain `CHANGELOG.md`, automate publish order with `cargo-release`. Includes a published-artifact smoke test (see section below) — existing integration tests use workspace path deps and don't validate the published tarball. |
| 2 | Codegen provider validation | `pulumi-codegen` | Medium | Medium | Only validated against `pulumi-random` and docker; run against AWS and other large providers that exercise deeply nested modules, complex `$ref` chains, and edge-case type references |

## AI-Assisted Development: Effort vs Difficulty

**Effort** and **Difficulty** map to different dimensions of LLM resource consumption when working on these tasks with an AI coding assistant:

- **Effort → Total token throughput.** How many files need to be read, how many edits made, how many tool calls issued. High effort means more cumulative tokens across the session — more code generated, more round trips. This is what causes sessions to hit context limits and trigger compression, potentially losing earlier context. It's largely a function of *breadth* — how many things need to change.

- **Difficulty → Peak context window pressure.** How much information needs to be held *simultaneously* to reason correctly. Hard tasks require reading and understanding multiple interconnected modules before writing a single line. This is a function of *depth* — how many moving parts must be in-context at once to make a correct decision.

Effort predicts how many sessions/messages a task takes, while difficulty predicts how likely any single step is to go wrong.

| # | Item | Effort (throughput) | Difficulty (peak context) |
|---|------|---|---|
| 1 | crates.io publishing | Moderate total — config and process steps | Low peak — each step is independent |
| 2 | Codegen provider validation | Moderate total — run codegen, fix issues iteratively | Moderate peak — need to understand provider schema edge cases while reading generated output |

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

## `flat_map` Secret/Known Propagation ✓ Done

**Crate:** `pulumi-core`

`Output::flat_map` now propagates inner known/secret metadata eagerly, in two
tiers:

- **Resolved source (fast path):** the source future is polled with
  `now_or_never()`; if it has already resolved, `f` runs synchronously inside
  `flat_map` and the inner metadata is merged before returning, so
  `is_known()` / `is_secret()` are correct immediately.
- **Pending source:** the composed `Shared` future is additionally spawned on
  the current Tokio runtime (when one exists), so the merge happens as soon as
  the source resolves even if the caller never polls the result. Covered by
  `test_flat_map_pending_source_converges_after_resolve`.

The originally proposed `JoinHandle` refactor was rejected: eager spawning
alone cannot make a synchronous `is_secret()` check deterministic (on a
current-thread runtime the spawned task hasn't run by the time the caller
asserts), whereas the `now_or_never` fast path is deterministic for the
resolved-source case the failing tests exercised. Known limitation: with a
pending source, metadata converges shortly after resolution rather than
atomically with it — matching the upstream SDKs, where secretness is itself
promise-like.

## Native Engine — Secret Config Encryption at Rest ✓ Done

**Crates:** `pulumi-engine`, `pulumi-automation`

Two changes, closing the last plaintext-secrets-at-rest gap in the native
engine path:

- **Go wire-format fix (`pulumi-engine/src/secrets.rs`).** The previous
  format (hex salt, single-blob `v1:BASE64(nonce||ct||tag)`, passphrase as
  validation plaintext) did *not* actually match the Go CLI despite the docs
  claiming so. Now: ciphertext `v1:BASE64(nonce):BASE64(ct||tag)`, salt state
  `v1:BASE64(salt):<ciphertext of "pulumi">` — true interop with the real
  CLI's passphrase provider. Pre-existing encrypted checkpoints in the old
  format are invalidated (pre-release, no compat guarantee). Added
  `encrypt_sync`/`decrypt_sync`/`salt_state` for non-async callers.

- **`Pulumi.<stack>.yaml` stack file (`pulumi-automation/src/native.rs`).**
  `NativeStack` config now persists to the real CLI's stack file format
  instead of a plaintext JSON file: plain values as YAML scalars, secrets as
  `secure:` ciphertext with a top-level `encryptionsalt`. The stack file's
  salt is created on first secret write, reuses the checkpoint's salt when
  one exists, and `engine_options` prefers checkpoint salt then stack-file
  salt so both artifacts converge on one key. Passphrase comes from
  `with_passphrase()` (tests), `PULUMI_CONFIG_PASSPHRASE`, or
  `PULUMI_CONFIG_PASSPHRASE_FILE`; secret operations without one are hard
  errors. Legacy `.pulumi-rs/<stack>.config.json` files are read as a
  fallback and deleted after the first YAML save. The secrets manager is
  cached per stack handle (PBKDF2 at 1M iterations is ~1s per derivation).

Not yet verified: actual round-trip against a real `pulumi` CLI binary
(would make a good integration test — needs `pulumi` on PATH).

## `output.rs` — Dep Tracking Refactor

**Crate:** `pulumi-core`

The current `deps: Mutex<Vec<String>>` in `OutputMeta` accumulates URN strings lazily as combinators resolve. Evaluate whether the representation, locking strategy, and propagation model are correct and efficient — particularly for deeply chained outputs.

## Audit `std::sync` Usage in Async Contexts

**Crate:** all

Search for uses of `std::sync::Mutex`, `std::sync::RwLock`, and similar primitives held across `.await` points. These can deadlock or cause unexpected blocking on a tokio worker thread. Replace with `tokio::sync` equivalents where appropriate.

## Audit `unwrap()` and Other Potential Panics

**Crate:** all

Search the codebase for `unwrap()`, `expect()`, `panic!`, and indexing operations that can panic. For each, determine whether the panic is truly unreachable (and should be replaced with a clearer `unreachable!()` or removed) or whether it represents a real failure mode that should surface as a `Result` or `Error` instead.

## `output.rs` — Audit `.clone()` Usage

**Crate:** `pulumi-core`

`output.rs` clones futures, `Arc`s, and metadata structs heavily to satisfy the borrow checker across async boundaries. Audit all `.clone()` calls to determine which are unavoidable and which indicate a structural problem (e.g., unnecessary duplication of work, or a design that fights the ownership model).

## Automated Code Review — Compare Against All SDKs

**Crate:** `scripts/`

The review harness currently compares the Rust SDK against a single upstream SDK. Expand coverage to compare against all reference SDKs (Go, TypeScript, Python, .NET) to catch divergences that appear in only one language's implementation.

## Automated Code Review — Fix Remaining Identified Issues

**Crate:** various

The initial automated review pass identified issues beyond secret propagation (which was addressed first). Work through the remaining flagged items from `review-output/` and address them.

## Stable Rust Support — Non-Goal

**Stable Rust is a non-goal until the next-generation trait solver ships.**

The SDK relies on `impl_trait_in_assoc_type` (rust-lang/rust#63063) and Rust
edition 2024, both of which require nightly. Rather than degrading the API with
boxed futures or manual future types as a workaround, we are waiting for the
next-gen trait solver to land on stable — at which point
`impl_trait_in_assoc_type` (and potentially edition 2024) will stabilize
naturally.

The nightly toolchain is pinned in `rust-toolchain.toml` (`nightly-2026-03-03`)
so builds are reproducible regardless.
