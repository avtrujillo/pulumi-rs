# TODO — pulumi-core

## Stable Rust Support — Non-Goal

Stable Rust is a non-goal until the next-generation trait solver ships on
stable. We intentionally use `#![feature(impl_trait_in_assoc_type)]` and
edition 2024; no workarounds (boxed futures, edition downgrade) are planned.
See the root TODO.md for rationale.

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

## ~~Serde Module Visibility~~ DONE

`serde.rs` is now `pub`. `pulumi-engine` uses the shared implementations
instead of maintaining duplicate conversion functions.
