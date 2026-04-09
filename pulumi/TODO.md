# TODO — pulumi

## crates.io Publishing

This is the user-facing crate that should be published to crates.io. Before
publishing:
- Audit the re-export surface from `pulumi-core` — ensure internal types are
  not accidentally exposed
- Add crate-level documentation with examples
- Add a README.md suitable for crates.io display
- Coordinate version with `pulumi-core` and `pulumi-macros`

See the root TODO.md "crates.io Publishing and Versioning" section.

## Feature Flags Documentation

Document available features (`macros`) in crate-level docs. As new features
are added (e.g., `test-support`), keep the documentation updated.

## Edition

Currently uses edition 2024 (nightly). Downgrade to 2021 when the workspace
moves to stable Rust.
