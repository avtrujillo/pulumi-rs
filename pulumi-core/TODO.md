# TODO — pulumi-core

## Stable Rust Support — Non-Goal

Stable Rust is a non-goal until the next-generation trait solver ships on
stable. We intentionally use `#![feature(impl_trait_in_assoc_type)]`, which is
the only thing that requires nightly (edition 2024 is stable); no workarounds
(boxed futures) are planned. See the root TODO.md for rationale and current
status.

## Open Items (details in the root TODO.md)

- **Preview / unknown resource outputs** — `resource.rs` deserializes the
  engine's outputs straight into the typed `Outputs` struct (three sites:
  `ResourceBuilder`, `ReadBuilder`, `RemoteComponentBuilder`) with no handling
  of preview unknowns. Likely breaks `pulumi preview` for new resources, or
  passes the unknown sentinel through as a real string. Inferred from the
  code; reproduce first.
- **Wire `Output<T>` into resource registration** — no builder takes or
  returns an `Output`, and `property_dependencies` is always sent empty, so
  there is no dependency tracking or secret propagation between resources.
  This is a design decision that also reshapes `pulumi-macros` and
  `pulumi-codegen` output; settle it before publishing.
- **`#[non_exhaustive]` on `Error`** (and likely `Asset` / `Archive`) before
  the first release, so adding a variant later is not a breaking change.
- **Dep representation** — deps are a `Vec<String>` of URNs; the other SDKs
  track resource objects.
- **`.clone()` audit in `output.rs`** — each awaiter, and each call to
  `is_known()` / `is_secret()` / `dependencies()`, clones the whole resolved
  value.
- **Eager execution of `map` / `flat_map` callbacks** — outputs here are lazy;
  the other SDKs run `apply` callbacks as soon as the source resolves.
- **`std::sync` locks across `.await`** and **`unwrap()` / panic audit** —
  workspace-wide audits that include this crate.
- **Stale module reviews** — `output`, `context`, `error` and `serde` changed
  after their May 2026 review and need re-reviewing. `asset` was reviewed as
  having major gaps.
- **Public `Error` exposes tonic types** — `Rpc(Box<tonic::Status>)` and
  `Transport(tonic::transport::Error)` tie this crate's semver to tonic's.
  Decide before publishing whether that is acceptable.

## `Output<T>` Metadata Propagation

**DONE.** Metadata (known, secret, deps) now travels with the resolved value,
`is_known()` / `is_secret()` / `dependencies()` are async, and `flat_map`
propagates the inner output's secretness and knownness. See the root TODO.md.

## Error Size

**DONE.** `Error::Rpc` holds `Box<tonic::Status>`, and the crate-wide
`#![allow(clippy::result_large_err)]` is gone. See the root TODO.md.

## Flesh Out Mock Implementations

**DONE.** `MockMonitor` and `MockEngine` in `connection.rs` have configurable
per-resource responses, preview-mode simulation (returning unknowns), error
injection for `register_resource`, `read_resource`, `invoke` and `call`, and
full call recording directly on `MockMonitor` (also exposed through
`TestContext`).

## Test Coverage

**DONE.** Every module with logic has unit tests (160 in total):

- `output.rs` — 40 tests: resolution, `map` / `flat_map` / `all*`, rejection, and secret / unknown / dependency propagation, including outputs derived before their source resolved
- `connection.rs` — 37 tests: MockMonitor (type/name/custom, URN format, id for custom vs component, parent, depends_on, protect, provider/version, aliases, change lists, preview, canned responses, error injection, echo, clone sharing, invoke, call, read_resource, register_resource_outputs, supports_feature), MockEngine (initial empty root, set/get root, clone sharing, log)
- `test_support.rs` — 19 tests: TestContextBuilder responses and errors for resources, reads, invokes and calls; config; preview mode; recording accessors
- `resource.rs` — 11 tests: alias_to_proto (URN/Spec/NoParent/ParentUrn), ResourceBuilder (depends_on, parent), ComponentBuilder (custom=false, register_outputs), RemoteComponentBuilder, ReadBuilder (id preservation, props echo)
- `stack_reference.rs` — 11 tests: get_output, require_output (present/missing/wrong-type), get_output_typed (present/missing/wrong-type), builder (outputs extraction, resource_name override, URN format)
- `transform.rs` — 11 tests: proto_opts_to_resource_options (None, basic fields, empty provider, import_id, custom_timeouts), resource_options_to_proto_opts, proto_alias_to_alias (Urn, Spec fields, ParentUrn, NoParent, None variant)
- `context.rs` — 10 tests: config parsing, typed accessors, secret detection
- `log.rs` — 9 tests: Severity→i32 conversions (Debug/Info/Warning/Error), all 5 log functions return Ok
- `invoke.rs` — 6 tests: InvokeBuilder (basic, provider option, failure→InvokeFailure), CallBuilder (basic, arg_deps, failure→InvokeFailure)
- `serde.rs` — 6 tests: JSON↔Struct roundtrip, secret wrapping, secret detection (no false positive for assets), unknown-sentinel detection

`asset.rs`, `stack.rs` and `error.rs` have no unit tests of their own.
