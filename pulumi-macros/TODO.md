# TODO — pulumi-macros

## Tests

**DONE.** `trybuild` compile tests cover all four derives: five programs that
must compile and six that must fail with a specific error. See CLAUDE.md.

Possible additions:
- Fail cases for `ComponentMethod` (missing `token` / `args` / `returns`) and
  for `ProviderFunction` missing `args` / `returns`; only the resource and
  component cases and one provider-function case are covered today.

## Toolchain

Uses edition 2024, which is stable. This crate itself needs nothing from
nightly, but the workspace is pinned to nightly because `pulumi-core` uses
`impl_trait_in_assoc_type`. See the root TODO.md.
