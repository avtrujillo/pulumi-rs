# TODO — pulumi-core

## Stable Rust Support — Non-Goal

Stable Rust is a non-goal until the next-generation trait solver ships on
stable. We intentionally use `#![feature(impl_trait_in_assoc_type)]` and
edition 2024; no workarounds (boxed futures, edition downgrade) are planned.
See the root TODO.md for rationale.

## Flesh Out Mock Implementations

`MockMonitor` and `MockEngine` in `connection.rs` exist but are minimal.
Extend them to support:
- Configurable per-resource responses (canned outputs keyed by resource type/name)
- Recording of all calls for assertion
- Preview-mode simulation (returning unknowns)
- Error injection for testing error paths

## Test Coverage

**DONE.** Unit tests added to all previously uncovered modules:

- `resource.rs` — 10 tests: alias_to_proto (URN/Spec/NoParent/ParentUrn), ResourceBuilder (depends_on, parent), ComponentBuilder (custom=false, register_outputs), RemoteComponentBuilder, ReadBuilder (id preservation, props echo)
- `invoke.rs` — 6 tests: InvokeBuilder (basic, provider option, failure→InvokeFailure), CallBuilder (basic, arg_deps, failure→InvokeFailure)
- `stack_reference.rs` — 10 tests: get_output, require_output, get_output_typed (present/missing/wrong-type), builder (outputs extraction, resource_name override, URN format)
- `transform.rs` — 11 tests: proto_opts_to_resource_options (None, basic fields, empty provider, import_id, custom_timeouts), resource_options_to_proto_opts, proto_alias_to_alias (Urn, Spec fields, ParentUrn, NoParent, None variant)
- `connection.rs` — 19 tests: MockMonitor (type/name/custom, URN format, id for custom vs component, parent, depends_on, protect, preview, canned response, echo, clone sharing, invoke, call, read_resource, supports_feature), MockEngine (initial empty root, set/get root, clone sharing, log)
- `log.rs` — 9 tests: Severity→i32 conversions (Debug/Info/Warning/Error), all 5 log functions return Ok
