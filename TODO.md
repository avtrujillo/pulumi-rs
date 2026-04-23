# TODO

## Roadmap

| # | Item | Crate | Effort | Difficulty | Notes |
|---|------|-------|--------|------------|-------|
| 1 | Unit test coverage (resource.rs, invoke.rs, etc.) | `pulumi-core` | Medium | Easy | **DONE.** 77 new tests across resource.rs, invoke.rs, stack_reference.rs, log.rs, transform.rs, connection.rs (104 total, up from 27). |
| 2 | TestContext builder | `pulumi-core` | Medium | Medium | **DONE.** `TestContextBuilder` and `TestContext` in `test_support.rs`; supports canned responses, preview mode, config injection, and registration assertions. |
| 3 | Transforms | `pulumi-engine` | Medium | Hard | **DONE.** `register_stack_transform` now stores callbacks and invokes them sequentially via gRPC before each resource registration. |
| 4 | NativeStack parity | `pulumi-automation` | Medium | Medium | **DONE.** Config operations (`get_config`, `set_config`, `get_all_config`, `remove_config`) persisted to `.pulumi-rs/<stack>.config.json`; `outputs()` reads from checkpoint; `PULUMI_CONFIG`/`PULUMI_CONFIG_SECRET_KEYS` wired correctly to the program; structured engine events (Prelude, ResourceStep, Diagnostic, Summary) now populated in all result types. |
| 5 | Engine test coverage | `pulumi-engine` | Medium | Medium | `orchestrator.rs`, `engine_service.rs`, and `monitor_service.rs` have zero tests; requires standing up in-process gRPC servers or refactoring for testability |
| 6 | Code generation | `pulumi-codegen` | XL | Very Hard | **DONE.** Core pipeline complete (Stages 1–6). Asset/Archive types now use `pulumi::Asset`/`pulumi::Archive`. Remaining: union types (#9), larger provider validation (#10). See `pulumi-codegen/TODO.md`. |
| 7 | Integration tests | new crate | Large | Medium | **Soft blocker:** depends on #8 (generated test crates default to `pulumi = "0.1"` which doesn't exist yet; use `--pulumi-crate-path` to work around locally). Also requires the `pulumi-random` provider plugin binary available on PATH in CI. End-to-end tests using `pulumi-random` + automation API: resource registration, `Output<T>` combinators, secrets, config, stack references, resource options, error cases. Gate behind `--features integration` or a separate workspace member. |
| 8 | crates.io publishing | all | Medium | Easy | **Soft blocker for #7.** API surface audit (`pulumi/src/lib.rs`, `pulumi-core/src/lib.rs`) and crate-level docs/README needed before publishing. Set `workspace.package.version`, maintain `CHANGELOG.md`, automate publish order with `cargo-release`. |
| 9 | Codegen union types | `pulumi-codegen` | Medium | Hard | **DONE.** `oneOf` now generates named `#[serde(untagged)]` Rust enums (e.g. `StringOrInteger`) in `crate::types`, deduplicated across the schema. |
| 10 | Codegen provider validation | `pulumi-codegen` | Medium | Medium | Only validated against `pulumi-random` and docker; run against AWS and other large providers that exercise deeply nested modules, complex `$ref` chains, and edge-case type references |
| 11 | MockMonitor improvements | `pulumi-core` | Small | Low | Add error injection and full call recording to `MockMonitor` in `connection.rs` for finer-grained test assertions beyond what `TestContext` provides |

## AI-Assisted Development: Effort vs Difficulty

**Effort** and **Difficulty** map to different dimensions of LLM resource consumption when working on these tasks with an AI coding assistant:

- **Effort → Total token throughput.** How many files need to be read, how many edits made, how many tool calls issued. High effort means more cumulative tokens across the session — more code generated, more round trips. This is what causes sessions to hit context limits and trigger compression, potentially losing earlier context. It's largely a function of *breadth* — how many things need to change.

- **Difficulty → Peak context window pressure.** How much information needs to be held *simultaneously* to reason correctly. Hard tasks require reading and understanding multiple interconnected modules before writing a single line. This is a function of *depth* — how many moving parts must be in-context at once to make a correct decision.

Effort predicts how many sessions/messages a task takes, while difficulty predicts how likely any single step is to go wrong.

| # | Item | Effort (throughput) | Difficulty (peak context) |
|---|------|---|---|
| 1 | Unit test coverage | Many tests to write, but each is independent → moderate total | Low peak — patterns are repetitive, mocks already exist |
| 2 | TestContext builder | Moderate total — one API surface to build | Moderate peak — mock layer + public API design need to be in context together |
| 3 | Transforms | Moderate total — not a lot of code | High peak — orchestrator + registration flow + ordering semantics all need to be in-context simultaneously |
| 4 | NativeStack parity | Moderate total — filling in missing features one by one | Moderate peak — need CLI Stack as reference while building NativeStack |
| 5 | Engine test coverage | Moderate total — similar scope to #1 | Moderate peak — gRPC server setup and service internals must be understood together |
| 6 | Code generation | Very high total — massive code output across many files | High peak — schema mapping, naming rules, type system all in-context at once |
| 7 | Integration tests | High total — many test programs to write | Moderate peak — each test is self-contained |
| 8 | crates.io publishing | Moderate total — config and process steps | Low peak — each step is independent |
| 9 | Codegen union types | Moderate total — ir.rs + emit.rs + tests | High peak — must hold `oneOf` semantics, enum generation, serde representation, and existing type system simultaneously |
| 10 | Codegen provider validation | Moderate total — run codegen, fix issues iteratively | Moderate peak — need to understand provider schema edge cases while reading generated output |
| 11 | MockMonitor improvements | Low total — small extension of existing mock | Low peak — patterns already established in `connection.rs` |

---

## Integration Tests

**Purpose:** Validate the SDK end-to-end against a real Pulumi engine. Catch
regressions in gRPC serialization, resource registration lifecycle, output
resolution, stack exports, and error handling that unit tests with mocks can't.

**How they would work:**

1. **Test programs** in an `integration-tests/` directory, each a small Pulumi
   project with `Pulumi.yaml` and a Rust binary that uses `pulumi::run()`.

2. **Use `pulumi-automation`** to drive each test: create a temp stack, run `up`,
   assert on outputs, run `destroy`, remove the stack. This keeps tests
   self-contained and cleanup automatic.

3. **Provider choice:** Use `pulumi-random` or another lightweight provider that
   doesn't require cloud credentials, so tests run in CI without secrets.

4. **What to test:**
   - Basic resource registration and output retrieval
   - `Output<T>` combinator chains (`map`, `flat_map`, `all`)
   - Secret propagation through the resource graph
   - Config access (`get_config`, `require_config`)
   - Stack references
   - Resource options (parent, depends_on, protect, aliases)
   - Error cases (missing required config, invalid inputs)
   - Component resources with `register_resource_outputs`
   - Provider function invocations (`invoke`)

5. **CI gating:** These are slow (seconds per test), so gate them behind
   `cargo test --features integration` or a separate `cargo test -p integration-tests`
   workspace member that only runs in CI.

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
   | `union` (`oneOf`) | Generated enum (see #9) |
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
