# TODO — pulumi

## crates.io Publishing

This is the user-facing crate that should be published to crates.io. Before
publishing:
- Audit the re-export surface from `pulumi-core` — ensure internal types are
  not accidentally exposed
- Add crate-level documentation with examples
- Add a README.md suitable for crates.io display
- Coordinate version with `pulumi-core` and `pulumi-macros`
- Decide whether the public `Error` should expose tonic types
  (`Rpc(Box<tonic::Status>)`, `Transport(tonic::transport::Error)`): as it
  stands, a tonic major-version bump is a breaking change for this crate

See the root TODO.md "crates.io Publishing and Versioning" section.

## Feature Flags Documentation

Document available features (`macros`) in crate-level docs. As new features
are added (e.g., `test-support`), keep the documentation updated.

The crate-level doc comment in `src/lib.rs` lists three derive macros; it is
missing `ComponentMethod`, which is also re-exported.

## Toolchain

Uses edition 2024, which is stable. Nightly is required only because
`pulumi-core` uses `impl_trait_in_assoc_type`; stable Rust is a non-goal until
the next-gen trait solver ships. See the root TODO.md.
