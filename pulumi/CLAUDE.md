# pulumi

Public-facing facade crate that re-exports the entire `pulumi-core` API. This is the crate end users depend on.

## Build

```bash
cargo build -p pulumi
cargo build -p pulumi --features macros   # Include derive macros
```

Doctests are disabled (`doctest = false`).

## Features

- **`macros`** — Enables `pulumi-macros` dependency and re-exports `#[derive(Resource)]`, `#[derive(ComponentResource)]`, `#[derive(ProviderFunction)]`, and `#[derive(ComponentMethod)]`.

## Source Structure

Single file: `src/lib.rs` (~38 lines). Contains:
- `pub use pulumi_core::*;` — re-exports all public items from the core crate
- Four `#[cfg(feature = "macros")]` re-exports, one per derive macro in `pulumi-macros`

There are no tests in this crate; it has no logic of its own.

## What users see through this crate

Because everything is re-exported, changes to `pulumi-core`'s public API are changes to this crate's API. Two that matter for user code:

- `pulumi::Error::Rpc` holds a `Box<tonic::Status>`, which keeps `pulumi::Result<T>` under clippy's `result_large_err` limit in user programs.
- `Output::is_known()`, `is_secret()` and `dependencies()` are async.

## Dependencies

- **pulumi-core** (required) — all core SDK functionality
- **pulumi-macros** (optional, via `macros` feature) — derive macros
