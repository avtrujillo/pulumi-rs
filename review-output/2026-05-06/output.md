# SDK Review: `output`

**Date:** 2026-05-06  
**Rust file:** `pulumi-core/src/output.rs`  

---

## Summary

The Rust `Output<T>` implementation covers the structural basics — a resolved/pending/unknown distinction, `map`/`flat_map`, `all`/`all2`/`all3`, and an `OutputResolver` — but contains **three behavioral correctness bugs** that break preview semantics and secret/dependency tracking, and is missing a substantial set of utility features present in every upstream SDK. It is not ready for production use or crates.io publication in this state.

---

## Missing Features

### 1. `unsecret()` — present in all three upstream SDKs
- Go: `Unsecret(input Output) Output` / `UnsecretWithContext`
- Node.js: `unsecret<T>(val: Output<T>): Output<T>`
- Python: `Output.unsecret(val: Output[U]) -> Output[U]`

The Rust SDK exposes `OutputResolver::set_secret(bool)` at construction time but provides no way to strip secretness from an already-constructed `Output`.

### 2. `Input<T>` type alias — present in all three upstream SDKs
- Go: `type Input = internal.Input`
- Node.js: `type Input<T> = T | Promise<T> | OutputInstance<T>`
- Python: `Input = Union[T, Awaitable[T], "Output[T]"]`

Rust has no `Input<T>` unification type. Every API that should accept either a plain value or an `Output<T>` currently only accepts one form. This affects every resource builder method downstream.

### 3. `from_input` / deep-unwrap — present in all three upstream SDKs
- Go: `ToOutput(v any)` / `ToOutputWithContext`
- Node.js: `output<T>(val: Input<T>)` with recursive unwrapping through arrays, maps, Promises
- Python: `Output.from_input(val)` with recursive unwrapping through dicts, lists, tuples, input types

Rust has no equivalent. You cannot lift a `T` or `Future<T>` or nested structure into an `Output<T>` without writing it manually.

### 4. `concat` / `interpolate` / string utilities
- Node.js: `concat(...params: Input<any>[]): Output<string>`, tagged template `interpolate`
- Python: `Output.concat(*args: Input[str])`, `Output.format(fmt, *args, **kwargs)`

No string combinators in Rust at all.

### 5. JSON utilities
- Go: `JSONMarshal(v any) StringOutput`, `JSONUnmarshal(data StringInput) AnyOutput`
- Node.js: `jsonStringify(obj, replacer?, space?)`, `jsonParse(text, reviver?)`
- Python: `Output.json_dumps(obj, ...)`, `Output.json_loads(s, ...)`

Absent from Rust.

### 6. `run_with_unknowns` / `runWithUnknowns` on combinators
- Node.js: `apply(func, runWithUnknowns?: boolean)` — lets callbacks see unknown sentinel values
- Python: `apply(func, run_with_unknowns: bool = False)` — same

Rust's `map` and `flat_map` have no such parameter and cannot implement proxy-style property lifting.

### 7. `Unknown` sentinel value
- Node.js: `class Unknown`, `unknown` singleton, `isUnknown(val)`, `containsUnknowns(value)`
- Python: `class Unknown`, `UNKNOWN` singleton, `contains_unknowns(val)`

Rust represents "unknown" as an unresolved `oneshot` channel that returns an error on drop — there is no sentinel object representing an unknown value within an otherwise-resolved aggregate.

### 8. `deferred_output` / `DeferredOutput`
- Go: `DeferredOutput[T any](ctx) (pulumix.Output[T], func(Output))`
- Node.js: `deferredOutput<T>(): [Output<T>, (source: Output<T>) => void]`
- Python: `deferred_output() -> tuple[Output[T], Callable[[Output[T]], None]]`

Rust's `Output::unresolved()` provides a similar capability but takes a concrete `T` value rather than another `Output<T>` as the resolver argument. The upstream pattern allows resolving with an entire `Output` (merging its metadata), not just a bare value.

### 9. Resource objects as dependencies, not URN strings
All upstream SDKs track `Resource` objects in dependency sets. Rust stores `Vec<String>` (URNs only). This means:
- No `OutputWithDependencies(ctx, o Output, deps ...Resource)` equivalent (Go)
- Dependency graph reconstruction is lossy
- `all2`/`all3`/`all` cannot collect `Resource` objects from their inputs

### 10. `Output.all(**kwargs)` → `Output<dict>` (Python)
Python's `Output.all(foo=a, bar=b)` produces `Output[{"foo": ..., "bar": ...}]`. The Rust `all` only supports homogeneous `Vec<Output<T>>` → `Output<Vec<T>>`.

---

## Behavioral Divergences

### D1 — **Critical**: `Output::unknown()` poisons combinators with errors instead of propagating unknownness

**Rust behavior:**
```rust
pub fn unknown() -> Self {
    let (_tx, rx) = oneshot::channel::<Result<T, Arc<Error>>>();
    // _tx is immediately dropped → rx returns Err on await
    // future resolves to Err("output value is unknown")
}
```
When you call `.map(f)` on an unknown output, `source.await` returns `Err(...)`, so `f` is never called and the result is an **error**, not a properly-typed unknown output. The `meta.known` flag is set to `false` but the future itself carries an error.

**Expected behavior (all three upstream SDKs):** An unknown output's `is_known` is `false`; combinators propagate `is_known = false` without calling the callback and without producing an error. Preview mode depends on this being an observable-but-non-fatal condition.

**Impact:** Every `.map()` / `.flat_map()` chain on an unknown output returns `Err(...)` from `.get()` instead of `Ok(default_value)` with `is_known() == false`. This breaks any program that inspects properties of preview-time outputs.

---

### D2 — **Critical**: `all`, `all2`, `all3` discard all metadata from inputs

```rust
pub fn all2<A, B>(a: &Output<A>, b: &Output<B>) -> Output<(A, B)> {
    // ...
    Output {
        future,
        meta: OutputMeta::new(), // ← always known=true, secret=false, deps=[]
    }
}
```

All three combinators create a **fresh** `OutputMeta` with:
- `known = true` regardless of whether any input is unknown
- `secret = false` regardless of whether any input is secret
- empty `deps` regardless of the inputs' dependencies

**Expected behavior:** A merged output is known iff *all* inputs are known; secret if *any* input is secret; deps are the union of all inputs' deps. This is explicitly stated in all three SDKs:

- Node.js: `isKnown = Promise.all(...).then(ps => ps.every(b => b))`, `isSecret = Promise.all(...).then(ps => ps.some(b => b))`
- Python: `known = all(d.is_known for d in all_data)`, `secret = any(d.is_secret for d in all_data)`
- Go: same via `internal.ResolveOutput`

---

### D3 — **Significant**: `flat_map` only merges `deps` from inner output, loses inner `known` and `secret`

```rust
pub fn flat_map<U, F>(&self, f: F) -> Output<U> {
    let meta = OutputMeta::copy_from(&self.meta); // copies outer meta
    // ...
    // Only this gets merged from inner:
    out_meta.deps.lock().unwrap().extend(inner_deps.iter().cloned());
    // inner.meta.known and inner.meta.secret are IGNORED
}
```

If `f` returns a secret `Output<U>`, the result's `is_secret()` will still reflect the *outer* output's secretness. Same for `known`.

**Expected behavior (all SDKs):** The result should be secret if *either* the outer or inner output is secret. It should be unknown if *either* is unknown. Example from Node.js `applyHelperAsync`: `liftInnerOutput` merges `isSecret: isSecret || innerIsSecret`.

---

### D4 — **Significant**: `resolve_unknown` API requires a concrete value, conflating unknown with known

```rust
pub fn resolve_unknown(self, default: T) {
    *self.meta.known.lock().unwrap() = false;
    self.resolve(default); // sends Ok(default) to the channel
}
```

The upstream `DeferredOutput` / `deferredOutput` pattern resolves with another `Output<T>` (forwarding its full state including knowness). In Rust you must provide a fake concrete `T` value. Callers must conjure a `default: T` which is meaningless — it will be returned by `.get()` while `is_known()` says `false`, which is internally contradictory and can mislead code that does not check `is_known()`.

---

### D5 — **Moderate**: `get()` is a legitimate async resolution, not a deploy-time guard

- Node.js: `get()` throws `"Cannot call '.get' during update or preview"`
- Python: `get()` raises `"Cannot call '.get' during update or preview"`

Rust's `get()` actually awaits and returns the value. There is no mechanism to prevent user code from accidentally consuming output values in contexts where they should be using `map`/`apply`.

---

### D6 — **Moderate**: `Output<T>` has no `Display` impl that warns about the misuse

Node.js overrides `toString` and `toJSON` to produce a helpful error/warning. Python overrides `__str__`. Rust has a `Debug` impl (`Output { .. }`) but no `Display` impl that could warn when an output is accidentally formatted into a string.

---

## API Surface Gaps

| Feature | Go | Node.js | Python | Rust |
|---|---|---|---|---|
| `unsecret(output)` | ✅ `Unsecret` | ✅ `unsecret` | ✅ `Output.unsecret` | ❌ |
| `to_secret(val)` | ✅ `ToSecret` | ✅ `secret` | ✅ `Output.secret` | ⚠️ `Output::secret(T)` only — no `Input<T>` form |
| `Input<T>` type | ✅ | ✅ | ✅ | ❌ |
| `Inputs` map type | ✅ | ✅ | ✅ | ❌ |
| `from_input` deep-unwrap | ✅ `ToOutput` | ✅ `output()` | ✅ `from_input` | ❌ |
| `concat` | — | ✅ | ✅ | ❌ |
| `interpolate` | — | ✅ | ✅ `format` | ❌ |
| `json_marshal` | ✅ | ✅ `jsonStringify` | ✅ `json_dumps` | ❌ |
| `json_unmarshal` | ✅ | ✅ `jsonParse` | ✅ `json_loads` | ❌ |
| `Unknown` sentinel | — | ✅ | ✅ | ❌ |
| `contains_unknowns(val)` | — | ✅ | ✅ | ❌ |
| `deferred_output` (Output→Output resolve) | ✅ | ✅ | ✅ | ⚠️ only `T`-resolve |
| `is_secret()` on result of `all` | ✅ | ✅ | ✅ | ❌ (always false) |
| `is_known()` on result of `all` | ✅ | ✅ | ✅ | ❌ (always true) |
| Dep propagation through `all` | ✅ | ✅ | ✅ | ❌ |
| `run_with_unknowns` on apply | — | ✅ | ✅ | ❌ |
| Resource object deps | ✅ | ✅ | ✅ | ❌ (URN strings only) |
| `all(**kwargs)` → dict output | — | — | ✅ | ❌ |
| `OutputResolver` via Output | ✅ | ✅ | ✅ | ⚠️ only via bare `T` |
| `Display`/`__str__` guard | — | ✅ | ✅ | ❌ |

---

## Recommendations

**P0 — Fix behavioral correctness bugs before any further work:**

1. **Remodel `Output::unknown()` to not use a dropped sender.** Replace it with a resolved `Ok(sentinel_value)` future combined with `known = false`, or introduce an explicit `OutputValue<T>` enum (`Known(T)`, `Unknown`, `Secret(T)`) as the inner type. Without this, `map` chains on unknown outputs produce errors during preview, breaking all preview-mode programs.

2. **Fix `all` / `all2` / `all3` metadata propagation.** Collect `is_known` (AND of all inputs), `is_secret` (OR of all inputs), and `deps` (union of all inputs) when constructing the combined output. This is a one-liner fix per combinator but semantically critical.

3. **Fix `flat_map` to propagate inner `known` and `secret`.** After resolving the inner `Output<U>`, merge `inner.meta.known` (AND with outer) and `inner.meta.secret` (OR with outer) into `out_meta`.

**P1 — Core API completeness (needed before any public release):**

4. **Add `Input<T>` type.** Define `type Input<T> = either T or Output<T>` (an enum or trait). All builder methods that currently take `Output<T>` should accept `Input<T>`. This is the single highest-leverage ergonomics improvement.

5. **Add `unsecret(output: Output<T>) -> Output<T>`.** It is a one-function addition and is present in all three SDKs. Without it, secrets can never be downgraded.

6. **Fix `resolve_unknown` to not require a fake value.** Change the unknown representation so `Output::unknown()` resolves to a dedicated `OutputValue::Unknown` variant rather than an error, then `resolve_unknown` can simply set that state without needing a dummy `default: T`.

**P2 — Utility completeness (needed before crates.io publish):**

7. **Add `concat` / `format` for `Output<String>`.** At minimum: `pub fn concat(outputs: Vec<Output<String>>) -> Output<String>`. This is used in nearly every Pulumi program.

8. **Add `json_to_string` / `json_from_str` combinators.** Trivial to implement on top of `map` using `serde_json`.

9. **Add resource object tracking alongside URN strings.** Change `deps: Mutex<Vec<String>>` to include resource handles, or at minimum add `resource_deps: Mutex<Vec<Arc<dyn Resource + Send + Sync>>>`.

10. **Add `from_future_result<T>(fut: Future<Output = Result<T, E>>) -> Output<T>`.** The current `from_future` only accepts infallible futures, which is too restrictive for most real async work.

**P3 — Developer experience:**

11. **Implement `Display` for `Output<T>` with a helpful panic/warning**, mirroring Node.js `toString` and Python `__str__`. This catches the accidental `format!("{}", my_output)` mistake early.

12. **Add `run_with_unknowns` parameter to `map`/`flat_map`** once the unknown sentinel is properly modeled.

---

## Verdict

**SIGNIFICANT GAPS**

The core scaffolding (`Output<T>`, resolver pattern, `map`/`flat_map`) is architecturally sound, but three behavioral correctness bugs (D1, D2, D3) mean the implementation does not correctly implement Pulumi preview semantics or secret propagation. These are not cosmetic — they will produce silent data loss (secrets escaping) and wrong error behavior (errors instead of unknowns during `pulumi preview`) for programs using any combinator that crosses unknown or secret outputs. Combined with the absence of `Input<T>`, `unsecret`, string utilities, and JSON utilities, the module is missing approximately 40% of the API surface that downstream codegen and user programs require.