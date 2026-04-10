# TODO — pulumi-engine

## ~~Secret Encryption~~ (Done)

Implemented in `secrets.rs`. Passphrase-based encryption using AES-256-GCM
with PBKDF2-HMAC-SHA256 key derivation (1M iterations). Ciphertext format
(`v1:` + base64) is compatible with the Go Pulumi SDK. Activated via
`PULUMI_CONFIG_PASSPHRASE` env var or `EngineOptions::secrets_manager`.

Includes:
- `SecretsManager` trait (RPITIT) and `PassphraseSecretsManager` impl
- Recursive JSON tree walkers for encrypt/decrypt of secret-wrapped values
- Salt-based key restoration across runs
- Secret property tracking in `ResourceState`
- Custom `Debug` impls that redact secret values
- `debug_assert` guard against saving plaintext secrets when encryption is configured
- 22 tests (15 in `secrets.rs`, 7 in `state.rs`)

## Transforms

`RegisterStackTransform` and `RegisterStackInvokeTransform` are accepted
but ignored. Implement transform execution during resource registration
so that user-registered transforms are applied to resource inputs before
they are sent to providers.

## ~~Refresh Improvements~~ (Done)

`refresh()` now:
- Calls `provider.read()` for each custom resource and updates checkpoint
  inputs/outputs with the live values
- Detects deleted resources (empty ID from provider) and removes them from state
- Reports drift via `RefreshDiff` (Same/Updated/Deleted with changed output keys)
- Persists the `refresh_before_update` flag from `ReadResponse`
- Produces a detailed summary with per-resource change lines

Implemented across `orchestrator.rs` (refresh loop), `diff.rs` (`RefreshAction`,
`RefreshDiff`, `diff_refresh()`), and `state.rs` (`refresh_before_update` field).

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
