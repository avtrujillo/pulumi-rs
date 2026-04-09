# TODO — pulumi-engine

## Secret Encryption

Secrets are stored unencrypted in the checkpoint JSON. Implement:
- Encryption/decryption layer for secret values on checkpoint save/load
- Key management (passphrase-based, cloud KMS, or compatible with Pulumi's
  existing secrets providers)
- Secret metadata tracking through the resource graph

## Transforms

`RegisterStackTransform` and `RegisterStackInvokeTransform` are accepted
but ignored. Implement transform execution during resource registration
so that user-registered transforms are applied to resource inputs before
they are sent to providers.

## Refresh Improvements

`refresh()` calls `provider.read()` for each custom resource but state
syncing is best-effort. Improve by:
- Updating checkpoint inputs/outputs with values returned from provider reads
- Handling deleted resources (resources that no longer exist upstream)
- Reporting drift between checkpoint state and actual state

## Test Coverage

Only `diff.rs` has unit tests (4 tests). Add tests for:
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

Currently uses edition 2024 (nightly). Downgrade to 2021 when the workspace
moves to stable Rust.
