# SDK Review: `asset`

**Date:** 2026-05-06  
**Rust file:** `pulumi-core/src/asset.rs`  

---

## Summary

The Rust `asset` module provides a minimal skeletal implementation using two enums (`Asset` and `Archive`) with `serde` derives. While the high-level concept is present, the implementation has one **critical correctness bug** (wrong archive value type, breaking nested archives), a likely **wire-format serialization failure** (missing Pulumi sig keys), and is missing substantial API surface shared by all three upstream SDKs — most critically the `AssetOrArchive` union type and `Output<T>` integration.

---

## Missing Features

### 1. `AssetOrArchive` Union Type
All three upstream SDKs expose a first-class union of `Asset | Archive`:
- **Go**: `AssetOrArchive` interface implemented by both `Asset` and `Archive`
- **Node.js**: `AssetMap = { [name: string]: Asset | Archive }` uses the union directly
- **Python**: `dict[str, Union[Asset, Archive]]`

Rust has no equivalent. An `AssetOrArchive` enum is required so that `Archive::Assets` values can be heterogeneous (see Behavioral Divergences §1).

```rust
// Required — currently absent
pub enum AssetOrArchive {
    Asset(Asset),
    Archive(Archive),
}
```

### 2. Constructor / Factory Functions
All three SDKs provide named constructors. Go: `NewFileAsset(path)`, `NewStringAsset(text)`, `NewRemoteAsset(uri)`, `NewFileArchive(path)`, `NewRemoteArchive(uri)`, `NewAssetArchive(assets)`. Python and Node.js have equivalent class constructors with input validation. Rust exposes raw enum-variant construction and no validation.

### 3. `Output<T>` Integration
Go exposes `AssetInput`, `ArchiveInput`, `AssetOrArchiveInput` traits and `AssetOrArchiveOutput` so that assets and archives participate in the `Output<T>` dependency graph. Without this, users cannot pass an `Output<Asset>` as a resource input field typed `Asset`. The Rust SDK has a full `Output<T>` system in `output.rs` but `Asset`/`Archive` are not wired into it.

### 4. Input Type Widening
- **Python** accepts `Union[str, PathLike]` for `FileAsset`/`FileArchive` paths — idiomatic Rust equivalent would be `impl Into<PathBuf>` or `impl AsRef<Path>`.
- **Node.js** accepts `string | Promise<string>` — Rust equivalent would be `impl Into<String>` constructors.
Rust constructors (if they existed) should accept `impl AsRef<Path>` for path variants.

### 5. Input Validation
Python raises `TypeError` on wrong-typed constructor arguments. Go's `NewAssetArchive` calls `contract.Failf` for non-Asset/Archive map values. Rust has no validation whatsoever; a malformed `Archive::Assets` map containing values that cannot be represented on the wire will silently produce invalid output.

### 6. `Display` / `Debug` Parity
No `Display` implementation. The Go and Node.js SDKs emit human-readable representations in error messages and logging.

---

## Behavioral Divergences

### 1. `Archive::Assets` Accepts Only `Asset`, Not `Asset | Archive` *(correctness bug)*
This is the most critical functional bug. Every upstream SDK allows archive asset maps to hold **both** assets and archives (nested archives):

| SDK | Map value type |
|-----|----------------|
| Go | `map[string]any` (validated as `Asset` or `Archive`) |
| Node.js | `AssetMap = { [name: string]: Asset \| Archive }` |
| Python | `dict[str, Union[Asset, Archive]]` |
| **Rust** | `HashMap<String, Asset>` ← **`Archive` excluded** |

A user who tries to nest an archive inside another archive (a common and valid use case) is silently broken. The fix is:

```rust
// Current (wrong):
Assets { assets: std::collections::HashMap<String, Asset> }

// Required:
Assets { assets: std::collections::HashMap<String, AssetOrArchive> }
```

### 2. Pulumi Wire-Format Serialization Is Likely Broken
The Pulumi engine identifies assets and archives on the wire by a **"sig" discriminator key** embedded in the serialized struct:

| Type | Sig value |
|------|-----------|
| Any asset | `"c44067f5952974187fd530286b0b4621"` |
| Any archive | `"0def7320c3a5731c473c5decf2cc7f82"` |

The expected wire form for a `FileAsset` is:
```json
{
  "4dabf18193072939515e22adb298388d": "c44067f5952974187fd530286b0b4621",
  "path": "/some/file"
}
```

The current `#[serde(untagged)]` derive emits:
```json
{ "path": "/some/file" }
```

The sig key is absent. The Pulumi engine will not recognize this as an asset; it will treat it as an ordinary map object. The `serde.rs` module (per CLAUDE.md) handles the secrets sig (`4dabf18193072939515e22adb298388d` → `"1b47061264138c4ac30d75fd1eb44270"`) and unknowns, but there is no evidence it handles asset/archive sigs. Custom `Serialize`/`Deserialize` impls are required.

### 3. Variant Naming Inconsistency with Upstream Terminology
The Rust enum calls the in-memory text variant `Asset::Text { text }`. All upstream SDKs call this type `StringAsset` / `string_asset`. The Rust name `Text` is acceptable idiomatically, but the **serialized field name `"text"`** must match the Pulumi wire format (`"text"` is actually correct per the Go SDK's serialization) — however, this only matters if the sig key issue above is resolved.

### 4. No `invalid` Sentinel State
Go's `asset` and `archive` structs carry an `invalid bool` field used to create zero-values that fail gracefully when used. Rust enums naturally avoid this (you can't construct an invalid variant), but there is no parallel mechanism for propagating validation failures at construction time.

---

## API Surface Gaps

### Types Missing

| Upstream Type | SDKs | Status in Rust |
|---------------|------|----------------|
| `AssetOrArchive` (union type) | Go, Node, Python | ❌ Absent |
| `AssetInput` trait | Go | ❌ Absent |
| `ArchiveInput` trait | Go | ❌ Absent |
| `AssetOrArchiveInput` trait | Go | ❌ Absent |
| `AssetOrArchiveOutput` | Go | ❌ Absent |
| `AssetMap` alias | Node | ❌ Absent |

### Constructor Functions Missing

| Function | SDKs |
|----------|------|
| `new_file_asset(path: impl AsRef<Path>) -> Asset` | Go, Node, Python |
| `new_string_asset(text: impl Into<String>) -> Asset` | Go, Node, Python |
| `new_remote_asset(uri: impl Into<String>) -> Asset` | Go, Node, Python |
| `new_file_archive(path: impl AsRef<Path>) -> Archive` | Go, Node, Python |
| `new_remote_archive(uri: impl Into<String>) -> Archive` | Go, Node, Python |
| `new_asset_archive(assets: HashMap<String, AssetOrArchive>) -> Archive` | Go, Node, Python |

### Trait Implementations Missing

| Trait | Rationale |
|-------|-----------|
| `Serialize` / `Deserialize` (custom, not derived) | Pulumi sig-key wire format |
| `From<&str>` / `From<String>` for path/text/uri variants | Ergonomics |
| `From<PathBuf>` for file variants | Ergonomics, parity with Python `PathLike` |
| `Display` | Logging / error messages |
| `TryFrom<prost_types::Struct>` | Deserialization from engine responses |

### Methods Missing

Go exposes accessor methods on the `Asset` interface (`Path()`, `Text()`, `URI()`) and on `Archive` (`Assets()`, `Path()`, `URI()`). Rust uses enum destructuring instead of accessors, which is idiomatic, but convenience methods like `fn path(&self) -> Option<&str>` would be valuable:

```rust
impl Asset {
    pub fn path(&self) -> Option<&str> { /* ... */ }
    pub fn text(&self) -> Option<&str> { /* ... */ }
    pub fn uri(&self) -> Option<&str> { /* ... */ }
    pub fn is_file(&self) -> bool { matches!(self, Self::Path { .. }) }
    pub fn is_remote(&self) -> bool { matches!(self, Self::Uri { .. }) }
    pub fn is_string(&self) -> bool { matches!(self, Self::Text { .. }) }
}
```

---

## Recommendations

**Priority 1 — Fix the correctness bug (blocking):**
Change `Archive::Assets` to use `HashMap<String, AssetOrArchive>` and introduce the `AssetOrArchive` enum. This is a breaking API change that is far cheaper to make now than after stabilization.

**Priority 2 — Fix wire-format serialization (blocking for any real use):**
Replace the `#[serde(untagged)]` derives with hand-written `Serialize`/`Deserialize` impls that embed the Pulumi sig keys. Without this, assets passed to any provider will be silently misinterpreted. Consider a `const ASSET_SIG: &str = "c44067f5952974187fd530286b0b4621"` sentinel in a shared constants module alongside the existing secrets and unknown sentinels in `serde.rs`.

**Priority 3 — Add constructor functions with validation (high value):**
`new_file_asset()`, `new_string_asset()`, `new_remote_asset()`, and archive equivalents. Accept `impl AsRef<Path>` for paths. Validate that archive maps contain only `AssetOrArchive` values. Return `Result<Asset>` or panic with a clear message for invalid inputs (consistent with Go's `contract.Failf`).

**Priority 4 — `Output<T>` integration (required for real programs):**
Implement `IntoOutput` or equivalent for `Asset` and `Archive` so they can be used as `Output<Asset>` resource input fields. This mirrors Go's `AssetInput`/`ArchiveInput`/`AssetOrArchiveInput` traits.

**Priority 5 — Accessor convenience methods and `From` impls (ergonomics):**
Add `path()`, `text()`, `uri()`, `is_file()`, `is_remote()`, `is_string()` on `Asset`; `path()`, `uri()`, `assets()` on `Archive`. Add `From<&str>`, `From<String>`, `From<PathBuf>` for the appropriate variants.

---

## Verdict

**MAJOR GAPS**

The implementation has two blocking correctness issues (wrong archive value type; broken Pulumi wire-format serialization) that would cause silent data corruption or engine rejection in any real Pulumi program that uses assets or archives. Beyond those, the `AssetOrArchive` union type used pervasively in multi-language SDKs is absent, `Output<T>` integration is missing, and there are no constructor functions, validation, or ergonomic accessors. The current code amounts to a data-structure sketch rather than a functional implementation.