# TODO

## Mock / Test Framework

**Purpose:** Let users unit-test their Pulumi programs without deploying anything.
Right now, the only way to verify a program is to run `pulumi up` against a real
engine. A mock framework would let you assert that the right resources are
registered with the right inputs, that outputs flow correctly through `Output<T>`
combinators, and that error paths behave as expected — all in-process, in
milliseconds.

**How it would work:**

1. **Trait abstraction over gRPC clients.** Today `Context` holds concrete
   `ResourceMonitorClient<Channel>` and `EngineClient<Channel>` behind
   `Arc<Mutex<>>`. Extract a trait (e.g. `ResourceMonitor`) with methods like
   `register_resource()`, `invoke()`, `read_resource()`, and provide two
   implementations: the real gRPC one and a `MockResourceMonitor` that records
   calls and returns canned responses.

2. **`MockEngine`** that implements the engine-side trait: `get_root_resource()`,
   `log()`, etc. It would store logged messages for assertion and return a
   synthetic root URN.

3. **`TestContext`** constructor that wires up mock clients without needing
   `PULUMI_*` env vars or network connections:
   ```rust
   let ctx = TestContext::new()
       .with_resource_response::<S3Bucket>("my-bucket", mock_outputs)
       .build();
   ```

4. **Assertion helpers** to inspect what was registered:
   ```rust
   let registrations = ctx.registered_resources();
   assert_eq!(registrations[0].type_token, "aws:s3/bucket:Bucket");
   assert_eq!(registrations[0].name, "my-bucket");
   ```

5. **Preview-mode simulation.** In preview, outputs are "unknown". The mock
   framework should support returning unknowns so users can test that their
   `Output::map` / `Output::flat_map` chains handle unknown values correctly.

**What changes:**
- `pulumi-core/src/context.rs` — extract traits over the gRPC clients
- New `pulumi-test` crate (or `pulumi-core/src/test_support.rs` behind a
  `test-support` feature) with `MockResourceMonitor`, `MockEngine`,
  `TestContext`, and assertion utilities

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
   | `union` | Generated enum |
   | `Asset` / `Archive` | SDK asset types (TBD) |
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

## Stable Rust Support

**Purpose:** The nightly-only requirement is the single biggest adoption barrier.
Many teams have policies against nightly in production. Removing the nightly
dependency makes the SDK viable for production use.

**Current nightly dependency:**

The only nightly feature used is **`impl_trait_in_assoc_type`**, declared in
`pulumi-core/src/lib.rs`. This enables the `IntoFuture` impls on builders to
return `impl Future` in the associated type position instead of boxing:

```rust
impl<R: Resource> IntoFuture for ResourceBuilder<R> {
    type Output = Result<RegisteredResource<R>>;
    type IntoFuture = impl Future<Output = Self::Output> + Send;  // nightly-only
    fn into_future(self) -> Self::IntoFuture { async move { ... } }
}
```

**Path to stable Rust:**

1. **Option A: Box the future.** Replace `impl Future` with
   `Pin<Box<dyn Future<Output = Self::Output> + Send>>`. This is the simplest
   change — one heap allocation per `.await`ed builder, negligible cost for
   infrastructure operations that do network I/O anyway. Used by virtually every
   async Rust library on stable.

2. **Option B: Named future types.** Use a concrete struct that implements
   `Future` manually. More boilerplate, avoids boxing, but hard to maintain as
   the async bodies evolve.

3. **Option C: Wait for stabilization.** `impl_trait_in_assoc_type` has been in
   nightly for years and is tracked in rust-lang/rust#63063. Stabilization is
   not imminent.

**Recommendation:** Option A (boxed futures). The performance difference is
immaterial for Pulumi programs — each `.await` does a gRPC round-trip that
dwarfs a heap allocation. This unblocks stable Rust with a one-line change per
builder. The affected builders are:

- `ResourceBuilder<R>` (`resource.rs`)
- `ReadBuilder<R>` (`resource.rs`)
- `ComponentBuilder<C>` (`resource.rs`)
- `RemoteComponentBuilder<C>` (`resource.rs`)
- `InvokeBuilder<F>` (`invoke.rs`)
- `CallBuilder<M>` (`invoke.rs`)
- `StackReferenceBuilder` (`stack_reference.rs`)
