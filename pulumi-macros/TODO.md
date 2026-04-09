# TODO — pulumi-macros

## ~~Tests~~ DONE

Compile-pass and compile-fail tests added using `trybuild`. Coverage includes
all four derive macros (`Resource`, `ComponentResource`, `ProviderFunction`,
`ComponentMethod`), missing required attribute errors, and unknown attribute errors.

## ~~ComponentMethod Derive Macro~~ DONE

`#[derive(ComponentMethod)]` added. Re-exported by the `pulumi` crate under
the `macros` feature.

## Edition

Uses edition 2024 (nightly). Stable Rust is a non-goal until the next-gen
trait solver ships. See the root TODO.md.
