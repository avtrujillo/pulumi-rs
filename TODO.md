# TODO

## Roadmap

| # | Item | Crate | Effort | Difficulty | Notes |
|---|------|-------|--------|------------|-------|
| 8 | crates.io publishing | all | Medium | Easy | **Soft blocker for #7.** API surface audit (`pulumi/src/lib.rs`, `pulumi-core/src/lib.rs`) and crate-level docs/README needed before publishing. Set `workspace.package.version`, maintain `CHANGELOG.md`, automate publish order with `cargo-release`. |
| 10 | Codegen provider validation | `pulumi-codegen` | Medium | Medium | Only validated against `pulumi-random` and docker; run against AWS and other large providers that exercise deeply nested modules, complex `$ref` chains, and edge-case type references |

## AI-Assisted Development: Effort vs Difficulty

**Effort** and **Difficulty** map to different dimensions of LLM resource consumption when working on these tasks with an AI coding assistant:

- **Effort → Total token throughput.** How many files need to be read, how many edits made, how many tool calls issued. High effort means more cumulative tokens across the session — more code generated, more round trips. This is what causes sessions to hit context limits and trigger compression, potentially losing earlier context. It's largely a function of *breadth* — how many things need to change.

- **Difficulty → Peak context window pressure.** How much information needs to be held *simultaneously* to reason correctly. Hard tasks require reading and understanding multiple interconnected modules before writing a single line. This is a function of *depth* — how many moving parts must be in-context at once to make a correct decision.

Effort predicts how many sessions/messages a task takes, while difficulty predicts how likely any single step is to go wrong.

| # | Item | Effort (throughput) | Difficulty (peak context) |
|---|------|---|---|
| 8 | crates.io publishing | Moderate total — config and process steps | Low peak — each step is independent |
| 10 | Codegen provider validation | Moderate total — run codegen, fix issues iteratively | Moderate peak — need to understand provider schema edge cases while reading generated output |

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
