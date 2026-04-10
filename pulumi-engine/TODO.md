# TODO — pulumi-engine

## Transforms

`RegisterStackTransform` and `RegisterStackInvokeTransform` are accepted
but ignored. Implement transform execution during resource registration
so that user-registered transforms are applied to resource inputs before
they are sent to providers.

## Test Coverage

`diff.rs` has 10 unit tests (4 for resource diff, 6 for refresh diff). Add tests for:
- `monitor_service.rs` — resource registration lifecycle, provider delegation
- `engine_service.rs` — logging, root resource management
- `state.rs` — checkpoint serialization/deserialization roundtrips
- `provider.rs` — provider launch, caching, shutdown
- `orchestrator.rs` — end-to-end `up`/`destroy`/`refresh` flows (with mock providers)

## Provider Plugin Improvements

- Provider shutdown: ensure providers are cleanly shut down after engine operations
- Provider configuration: support `Configure` calls to set provider-level config
- Plugin download: automatic provider plugin installation when not found locally

## Edition

Uses edition 2024 (nightly). Stable Rust is a non-goal until the next-gen
trait solver ships. See the root TODO.md.
