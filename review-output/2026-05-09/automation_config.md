# SDK Review: `automation_config`

**Date:** 2026-05-09  
**Rust file:** `pulumi-automation/src/config.rs`  

---

## Summary
The Rust `automation_config` module faithfully captures the core `ConfigValue` type and adds ergonomic Rust-idiomatic conveniences (`plaintext()`/`secret()` constructors, `From` impls) not present upstream. However, it is missing the `ConfigMap` type alias that is present in every upstream SDK, and it contains a meaningful security regression: the derived `Debug` implementation leaks secret plaintext values, whereas Python explicitly guards against this with a masking `__repr__`. There is also a latent wire-format serialization divergence.

---

## Missing Features

### 1. `ConfigMap` Type Alias
All upstream SDKs define a `ConfigMap` type as a first-class alias:

| SDK | Definition |
|---|---|
| **Node.js** | `export type ConfigMap = { [key: string]: ConfigValue };` |
| **Python** | `ConfigMap = MutableMapping[str, ConfigValue]` |
| **Go** *(inferred from public API)* | `type ConfigMap map[string]ConfigValue` |

Rust has no such alias. Every caller is forced to write `HashMap<String, ConfigValue>` (or `BTreeMap`, etc.) without any shared semantic name. This alias is not cosmetic — it is the return type of `Stack::get_all_config()`, the parameter type of `Stack::set_all_config()`, and appears throughout the automation API surface. Its absence here forces an inconsistency in public API signatures elsewhere in the crate.

**Fix:**
```rust
use std::collections::HashMap;

/// A map of configuration key-value pairs, keyed by config key (e.g. `"myproject:mykey"`).
pub type ConfigMap = HashMap<String, ConfigValue>;
```

### 2. Secret Masking in Display/Debug
Python explicitly defines a `__repr__` that returns `"[secret]"` for secret values rather than the plaintext:

```python
_SECRET_SENTINEL = "[secret]"

def __repr__(self):
    return _SECRET_SENTINEL if self.secret else repr(self.value)
```

The Rust implementation uses `#[derive(Debug)]`, which will emit:
```
ConfigValue { value: "my-actual-password", secret: true }
```
This is a security regression. Any secret config value that passes through a log line, panic message, or test failure output will have its plaintext exposed. This is not a theoretical concern — `tracing`/`log` macros, `unwrap()` panics, and test harness output all invoke `Debug`.

---

## Behavioral Divergences

### 1. `Debug` Leaks Secret Plaintext (Security)
As noted above, `#[derive(Debug)]` unconditionally formats `value`. Python's canonical behavior is to mask it. The Rust implementation should implement `Debug` manually:

```rust
impl fmt::Debug for ConfigValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.secret {
            f.debug_struct("ConfigValue")
                .field("value", &"[secret]")
                .field("secret", &true)
                .finish()
        } else {
            f.debug_struct("ConfigValue")
                .field("value", &self.value)
                .field("secret", &false)
                .finish()
        }
    }
}
```

### 2. Serialization Always Emits `"secret": false` for Plaintext Values
Node.js defines `secret` as an optional field (`secret?: boolean`), meaning the Pulumi CLI wire format omits it entirely for plaintext values. The Rust implementation, using `#[derive(Serialize)]` without `skip_serializing_if`, always emits `"secret": false`:

```json
// Rust output for a plaintext value
{"value": "hello", "secret": false}

// Expected by Pulumi CLI / Node.js interop
{"value": "hello"}
```

This can cause issues when the serialized config is consumed by or compared against output from other Pulumi SDKs or the CLI itself. The fix is:

```rust
#[serde(default, skip_serializing_if = "std::ops::Not::not")]
pub secret: bool,
```

### 3. `secret` Field Name vs. Upstream Optional Semantics
On deserialization, the Rust `#[serde(default)]` correctly handles a missing `secret` key (defaults to `false`), which matches Node.js's `secret?: boolean`. This part is correct. The divergence is only on the *serialization* path as described above.

---

## API Surface Gaps

| Element | Node.js | Python | Rust | Notes |
|---|---|---|---|---|
| `ConfigMap` type alias | ✅ | ✅ | ❌ | Critical — used throughout automation API |
| `ConfigValue::new(value, secret)` generic constructor | ✅ (interface literal) | ✅ (`__init__`) | ⚠️ | Rust splits into `plaintext()`/`secret()` — acceptable but asymmetric |
| `Display` / `__repr__` with secret masking | ❌ | ✅ | ❌ | Missing from both Node.js and Rust |
| Custom `Debug` with secret masking | N/A | ✅ | ❌ | Rust derives it; should be manual |
| `PartialEq` / `Eq` | N/A | N/A | ❌ | Not upstream-required, but absence complicates testing |
| `_SECRET_SENTINEL` / masking constant | ❌ | ✅ (`_SECRET_SENTINEL`) | ❌ | Useful for tests and downstream code |
| `Default` impl | ❌ | ❌ | ❌ | Not required, but consistent with Rust conventions |

---

## Recommendations

**P0 — Security fix (implement immediately before any crate.io publish):**

1. **Replace `#[derive(Debug)]` with a manual `Debug` impl** that emits `"[secret]"` for the `value` field when `secret == true`. This is a correctness/security issue, not a style issue.

**P1 — API completeness (blocking for downstream crates):**

2. **Add `pub type ConfigMap = HashMap<String, ConfigValue>;`** to this module and re-export from the crate root. Every config-manipulating method in `Stack` depends on this type; without it, the public API is inconsistent.

3. **Fix serialization with `skip_serializing_if`** on the `secret` field to match the Pulumi wire format and interoperate cleanly with other SDKs and the CLI's JSON.

**P2 — Nice-to-have parity:**

4. **Add `impl fmt::Display for ConfigValue`** mirroring Python's `__repr__` behavior: display `[secret]` for secrets and the raw value for plaintext. This integrates naturally with `tracing` event fields and user-facing output.

5. **Derive `PartialEq` and `Eq`** to make unit testing and assertion ergonomics match what's natural in Rust. The upstream SDKs gain this for free from their type systems; Rust needs it explicit.

6. **Add a `pub const SECRET_SENTINEL: &str = "[secret]";`** (or private `_SECRET_SENTINEL`) constant so masking behavior is defined in exactly one place and downstream code can reference it consistently.

---

## Verdict

**MINOR GAPS**

The core `ConfigValue` struct is correctly modeled and the Rust-idiomatic conveniences are a net positive. However, the missing `ConfigMap` alias breaks the downstream automation API surface, the `Debug` secret leak is a real security concern warranting a P0 fix before publication, and the serialization divergence is a latent wire-format interoperability bug. None of these require architectural changes — all are fixable in under 50 lines.