# pulumi

Public-facing facade crate that re-exports the entire `pulumi-core` API. This is the crate end users depend on.

## Build

```bash
cargo build -p pulumi
cargo build -p pulumi --features macros   # Include derive macros
```

Doctests are disabled (`doctest = false`).

## Features

- **`macros`** — Enables `pulumi-macros` dependency and re-exports `#[derive(Resource)]`, `#[derive(ComponentResource)]`, and `#[derive(ProviderFunction)]`.

## Source Structure

Single file: `src/lib.rs` (~33 lines). Contains:
- `pub use pulumi_core::*;` — re-exports all public items from the core crate
- Conditional `#[cfg(feature = "macros")]` block re-exporting the three derive macros from `pulumi-macros`

## Dependencies

- **pulumi-core** (required) — all core SDK functionality
- **pulumi-macros** (optional, via `macros` feature) — derive macros
