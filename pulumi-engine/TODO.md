# TODO — pulumi-engine

## Transforms

**DONE.** `RegisterStackTransform` callbacks are now stored in
`ResourceMonitorImpl` and invoked sequentially (in registration order) via
the Callbacks gRPC service before each resource registration. Properties
and options (`ignore_changes`, `additional_secret_outputs`) returned by
transforms are applied before diffing and provider operations.

`RegisterStackInvokeTransform` is still accepted but not yet executed
(invoke-level transforms are a follow-up).

## Test Coverage

**DONE**, except for `provider.rs`. 85 unit tests:
- `monitor_service.rs` — 26: resource registration lifecycle (create / same / update / dry-run), provider delegation for invoke, call and read, transforms, hooks, plus two gRPC smoke tests
- `orchestrator.rs` — 18: `up` / `preview` / `destroy` / `refresh` flows with mock providers, including three that spawn a real subprocess
- `secrets.rs` — 12: encrypt / decrypt roundtrips, wrong key, salt handling
- `diff.rs` — 10: 4 for resource diff, 6 for refresh diff
- `engine_service.rs` — 10: logging, root resource management, plus two gRPC smoke tests
- `state.rs` — 7: checkpoint roundtrips with and without encryption, secret redaction
- `provider.rs` — 2: package-name parsing only

Still open:
- `provider.rs` — provider launch, caching and shutdown are untested (they need a real provider plugin)

## Handler/Shim Refactor

**DONE.** Each RPC in `monitor_service.rs` and `engine_service.rs` is a
`handle_*` method holding the logic plus a thin tonic shim. See CLAUDE.md.

## Error Size

**DONE.** `Error::ProviderStatus` holds `Box<tonic::Status>`, which cleared
clippy's `result_large_err` for this crate's own error type. Handlers still
return `tonic::Status` directly (tonic requires it); the workspace
`clippy.toml` exempts that type.

## Clippy Warnings

Five style warnings remain, all mechanical:
- `monitor_service.rs` — `apply_transforms` takes eight arguments (limit is seven)
- `test_utils.rs` — four `MockProvider` methods return `impl Future` and could be `async fn`

## Provider Plugin Improvements

- Provider shutdown: ensure providers are cleanly shut down after engine operations
- Provider configuration: support `Configure` calls to set provider-level config
- Plugin download: automatic provider plugin installation when not found locally

## Toolchain

Uses edition 2024, which is stable. This crate itself needs nothing from
nightly, but it depends on `pulumi-core`, which uses
`impl_trait_in_assoc_type`. See the root TODO.md.
