# TODO — pulumi-core

## Stable Rust Support

Remove the `#![feature(impl_trait_in_assoc_type)]` requirement by boxing
the futures returned by `IntoFuture` impls on builders. Affected types:
`ResourceBuilder`, `ReadBuilder`, `ComponentBuilder`, `RemoteComponentBuilder`,
`InvokeBuilder`, `CallBuilder`, `StackReferenceBuilder`. Also requires
downgrading from edition 2024 to 2021.

See the root TODO.md for full analysis and recommendations.

## Flesh Out Mock Implementations

`MockMonitor` and `MockEngine` in `connection.rs` exist but are minimal.
Extend them to support:
- Configurable per-resource responses (canned outputs keyed by resource type/name)
- Recording of all calls for assertion
- Preview-mode simulation (returning unknowns)
- Error injection for testing error paths

## Test Coverage

- `resource.rs` — no unit tests (relies on integration testing via gRPC)
- `invoke.rs` — no unit tests
- `stack_reference.rs` — no unit tests
- `transform.rs` — no unit tests
- `connection.rs` — no unit tests for mock implementations
- `log.rs` — no unit tests

With the `MockMonitor`/`MockEngine` traits now in place, adding unit tests
for resource registration, invocation, and stack operations is feasible
without a running engine.

## Serde Module Visibility

`serde.rs` is `pub(crate)`. Consider whether any serialization utilities
should be public for users implementing custom resource types.
