# SDK Review: `automation_stack`

**Date:** 2026-05-09  
**Rust file:** `pulumi-automation/src/stack.rs`  

---

## Summary

The Rust `automation_stack` module is a thin skeleton that implements the four core lifecycle methods (`up`, `preview`, `refresh`, `destroy`) with zero options, no result summaries, no event streaming, and none of the auxiliary stack operations. Against the Go, Node.js, and Python SDKs — which share a consistent, mature API surface — the Rust implementation is missing roughly 80% of the expected feature set. It is not suitable for production use and would not be a functional drop-in replacement.

---

## Missing Features

### 1. Operations entirely absent

| Operation | Go | Node.js | Python | Rust |
|---|---|---|---|---|
| `ImportResources` | ✅ `ImportResources()` | ✅ `import()` | ✅ `import_resources()` | ❌ |
| `PreviewRefresh` | ✅ `PreviewRefresh()` | ✅ `previewRefresh()` | ✅ `preview_refresh()` | ❌ |
| `PreviewDestroy` | ✅ `PreviewDestroy()` | ✅ `previewDestroy()` | ✅ `preview_destroy()` | ❌ |
| `Rename` | ✅ `Rename()` | ✅ `rename()` | ✅ `rename()` | ❌ |
| `Cancel` | ✅ `Cancel()` | ✅ `cancel()` | ✅ `cancel()` | ❌ |
| `History` | ✅ `History()` | ✅ `history()` | ✅ `history()` | ❌ |
| `Info` | ✅ `Info()` | ✅ `info()` | ✅ `info()` | ❌ |
| `ChangeSecretsProvider` | ✅ | ❌ | ❌ | ❌ |

### 2. Config operations missing

All upstream SDKs have these methods on `Stack`; Rust has only `get_config`, `set_config`, `get_all_config`, and `remove_config`:

- `set_all_config(config: HashMap<String, ConfigValue>)` — Go `SetAllConfig`, Node `setAllConfig`, Python `set_all_config`
- `set_all_config_json(json: &str)` — Go `SetAllConfigJson`, Node `setAllConfigJson`, Python `set_all_config_json`
- `remove_all_config(keys: &[&str])` — Go `RemoveAllConfig`, Node `removeAllConfig`, Python `remove_all_config`
- `refresh_config()` — Go `RefreshConfig`, Node `refreshConfig`, Python `refresh_config`
- Go-specific `WithOptions` variants: `GetConfigWithOptions`, `SetConfigWithOptions`, `GetAllConfigWithOptions`, `SetAllConfigWithOptions`, `RemoveConfigWithOptions`, `RemoveAllConfigWithOptions`

### 3. Tag operations entirely absent

- `get_tag(key: &str) -> String` — Go `GetTag`, Node `getTag`, Python `get_tag`
- `set_tag(key: &str, value: &str)` — Go `SetTag`, Node `setTag`, Python `set_tag`
- `remove_tag(key: &str)` — Go `RemoveTag`, Node `removeTag`, Python `remove_tag`
- `list_tags() -> HashMap<String, String>` — Go `ListTags`, Node `listTags`, Python `list_tags`

### 4. ESC environment operations entirely absent

- `add_environments(envs: &[&str])` — Go `AddEnvironments`, Node `addEnvironments`, Python `add_environments`
- `list_environments() -> Vec<String>` — Go `ListEnvironments`, Node `listEnvironments`, Python `list_environments`
- `remove_environment(env: &str)` — Go `RemoveEnvironment`, Node `removeEnvironment`, Python `remove_environment`

### 5. Static constructors missing from `Stack` itself

All upstream SDKs expose static/class methods on `Stack` for creation, selection, and upsert:

- `Stack::create(name, workspace)` — Go `NewStack`, Node `Stack.create`, Python `Stack.create`
- `Stack::select(name, workspace)` — Go `SelectStack`, Node `Stack.select`, Python `Stack.select`
- `Stack::create_or_select(name, workspace)` — Go `UpsertStack`, Node `Stack.createOrSelect`, Python `Stack.create_or_select`

### 6. Utility functions absent

- `fully_qualified_stack_name(org, project, stack) -> String` — present in all three upstream SDKs (`FullyQualifiedStackName` in Go, `fullyQualifiedStackName` in Node, `fully_qualified_stack_name` in Python)
- `get_permalink(stdout: &str) -> Result<String>` — Go `GetPermalink` standalone function + `.GetPermalink()` methods on `UpResult`, `PreviewResult`, `RefreshResult`, `DestroyResult`

### 7. Inline programs not supported

All upstream SDKs allow passing a runtime function (`PulumiFn` / `pulumi.RunFunc`) that is served over a local gRPC `LanguageRuntime` server during `up`/`preview`/`refresh`/`destroy`. Rust has no such mechanism.

### 8. Options structs entirely absent

Every lifecycle method on Rust takes zero arguments; all upstream SDKs accept option bags with dozens of fields. The following options are missing from all four operations:

`parallel`, `message`, `expect_no_changes`, `diff`, `replace`, `target`, `exclude`, `policy_packs`, `policy_pack_configs`, `target_dependents`, `exclude_dependents`, `user_agent`, `color`, `refresh`, `suppress_outputs`, `suppress_progress`, `continue_on_error`, `attach_debugger`, `config_file`, `run_program`, `show_secrets`, `on_output: Fn(&str)`, `on_error: Fn(&str)`, `on_event: Fn(EngineEvent)`, `plan` (up/preview), `import_file` (preview), `clear_pending_creates` (refresh), `remove` (destroy), `exclude_protected` (destroy), `debug_log_opts` (Go).

---

## Behavioral Divergences

### 1. `import_state` is broken

```rust
// stack.rs lines 196–212
run_pulumi_cmd(
    self.workspace.work_dir(),
    &["stack", "import", "--stack", &self.name, "--file", "/dev/stdin"],
    &[],
)
```

The implementation serializes `state` to `json` but never pipes it to the process's stdin. The `/dev/stdin` trick fails silently and also does not work on Windows. All upstream SDKs delegate to `workspace.ImportStack`/`workspace.import_stack` which write the JSON to a temp file and pass `--file <path>`.

### 2. `outputs()` uses the wrong CLI command

```rust
&["stack", "output", "--stack", &self.name, "--json"]
```

`pulumi stack output --json` returns a flat `{key: value}` map, **not** `{key: {value: ..., secret: true}}`. The `OutputValue` struct therefore never gets populated correctly — `secret` will always be `false` (the serde default). All upstream SDKs call `workspace.StackOutputs()`/`stack_outputs()`, which uses `pulumi stack output --json --show-secrets` and parses the richer format.

### 3. `events` field is always empty — wrong event model

```rust
// UpResult, PreviewResult, RefreshResult, DestroyResult all have:
pub events: Vec<EngineEvent>,
// ...and are always populated as:
events: Vec::new(),
```

All upstream SDKs deliver events as they occur via a streaming `on_event: Fn(EngineEvent)` callback using `--event-log` (file tail or gRPC `StreamEvents`). Rust has inverted the model — collecting into a result field — but never implements the collection. The field is structural dead weight.

### 4. Missing `--exec-kind` flag

All upstream SDKs always append `--exec-kind=auto.local` (or `auto.inline` for inline programs) to every CLI invocation. Rust omits this, which means the Pulumi engine cannot distinguish automation API calls from manual CLI calls, breaking telemetry and potentially behavior.

### 5. Missing `PULUMI_DEBUG_COMMANDS` environment variable

All upstream SDKs set `PULUMI_DEBUG_COMMANDS=true` in the subprocess environment to unlock automation-API-only features. Rust's `run_pulumi_cmd` (not shown but called without env overrides) does not set this, so commands that require it silently fail or behave differently.

### 6. `serialize_args_for_op` / `post_command_callback` hooks not called

All upstream SDKs call `workspace.SerializeArgsForOp(stackName)` before the CLI invocation (to allow the workspace to inject additional arguments) and `workspace.PostCommandCallback(stackName)` after success. Rust calls neither. This breaks workspace customization points.

### 7. `export_state` / `import_state` naming vs. upstream

All upstream SDKs name these `export_stack`/`import_stack` (`ExportStack`/`ImportStack` in Go). The Rust naming diverges without justification and creates a cognitive mismatch for developers familiar with the other SDKs.

### 8. `outputs()` called unconditionally in `up()` without error propagation

```rust
let outputs = self.outputs().await.unwrap_or_default();
```

Silently swallows output-fetch errors. Upstream SDKs propagate this as a hard error.

### 9. No `--show-secrets` handling in `History` / `info`

All upstream SDKs gate `--show-secrets` on whether it's a remote workspace and user preference. Since Rust has no `history()` or `info()`, result structs (`UpResult`, `RefreshResult`, `DestroyResult`) also lack a `summary: UpdateSummary` field, so the caller gets no structured post-operation metadata.

### 10. No remote workspace support

All upstream SDKs have `isRemote()` / `_remote` detection and `remoteArgs()` generation (populating `--remote`, `--remote-git-branch`, `--remote-env`, etc.). Rust has no such logic.

### 11. No CLI version gating

Go and Node.js check CLI semver before using features like `PreviewRefresh`/`PreviewDestroy` (>= 3.105.0) and inline programs for refresh/destroy (>= 3.181.0), returning actionable errors. Rust has no version checks.

---

## API Surface Gaps

### Types entirely missing

| Type | Present in | Notes |
|---|---|---|
| `UpdateSummary` | Go, Node, Python | Rich struct with `kind`, `start_time`, `end_time`, `message`, `environment`, `config`, `result`, `version`, `resource_changes`, `Deployment` |
| `OpType` (enum/string) | Go, Node, Python | `"create"`, `"update"`, `"delete"`, `"replace"`, … (14 variants) |
| `OpMap` | Go, Node, Python | `HashMap<OpType, u32>` |
| `ImportResource` | Go, Node, Python | Struct for resources to import |
| `ImportResult` | Go, Node, Python | Has `generated_code: String` |
| `RenameResult` | Go, Node, Python | |
| `PreviewStep` | Go | With `diff_reasons`, `replace_reasons`, `detailed_diff` |
| `PropertyDiff` | Go | `kind`, `input_diff` |
| `UpOptions` / `up::Options` | Go, Node, Python | See §Missing Features §8 |
| `PreviewOptions` | Go, Node, Python | |
| `RefreshOptions` | Go, Node, Python | |
| `DestroyOptions` | Go, Node, Python | |
| `RenameOptions` | Go, Node | |
| `ImportOptions` | Go, Node, Python | |
| `ChangeSecretsProviderOptions` | Go | |
| `GlobalOpts` / `DebugLogOpts` | Go, Node | Shared across all operation options |

### Fields missing on existing result types

**`UpResult`** — missing `summary: UpdateSummary`

**`PreviewResult`** — missing `change_summary: HashMap<OpType, u32>` (has `events: Vec<EngineEvent>` instead, which is wrong)

**`RefreshResult`** — missing `summary: UpdateSummary`

**`DestroyResult`** — missing `summary: UpdateSummary`

### Methods missing on result types

- `UpResult::get_permalink(&self) -> Result<String>` (Go, Node)
- `PreviewResult::get_permalink(&self) -> Result<String>` (Go, Node)
- `RefreshResult::get_permalink(&self) -> Result<String>` (Go, Node)
- `DestroyResult::get_permalink(&self) -> Result<String>` (Go, Node)

### Trait/interface gaps

- No `Workspace` trait abstracting over workspace implementations (all upstream SDKs have this, enabling custom backends)
- No error type variants for `IsConcurrentUpdateError` (Go) or `StackNotFoundError` (Node, Python)

---

## Recommendations

**Priority 1 — Fix correctness bugs (blocking)**

1. **Fix `import_state`**: write JSON to a temp file and pass `--file <path>`. Remove `/dev/stdin`. Add Windows compatibility.
2. **Fix `outputs()`**: switch to the workspace's `stack_outputs()` method, or use `--show-secrets` and parse the `{value, secret}` envelope properly.
3. **Add `PULUMI_DEBUG_COMMANDS=true`** and `--exec-kind=auto.local` to all CLI invocations via `run_pulumi_cmd`.
4. **Call `serialize_args_for_op` and `post_command_callback`** on every CLI operation.

**Priority 2 — Core API completeness (unblocks real usage)**

5. **Add `UpdateSummary`** struct and wire it into `UpResult`, `RefreshResult`, `DestroyResult` by calling `history(page_size=1)` after each operation (matching Go/Node/Python).
6. **Add `OpType` enum + `OpMap`** type alias; wire `change_summary: OpMap` into `PreviewResult` (replacing `events`).
7. **Add options structs** for `up`, `preview`, `refresh`, `destroy` with at minimum: `parallel`, `message`, `expect_no_changes`, `diff`, `target`, `exclude`, `target_dependents`, `exclude_dependents`, `replace`, `color`, `refresh`, `suppress_outputs`, `suppress_progress`, `continue_on_error`, `show_secrets`.
8. **Add `history()` and `info()`** methods.
9. **Add `cancel()`**.

**Priority 3 — Auxiliary operations**

10. **Add tag operations**: `get_tag`, `set_tag`, `remove_tag`, `list_tags`.
11. **Add remaining config operations**: `set_all_config`, `set_all_config_json`, `remove_all_config`, `refresh_config`.
12. **Add ESC operations**: `add_environments`, `list_environments`, `remove_environment`.
13. **Add `fully_qualified_stack_name()`** utility function.
14. **Add `get_permalink()`** on result types.
15. **Add `rename()`** and `import_resources()`.

**Priority 4 — Advanced features**

16. **Add `on_event` callback + event log machinery** (file tail or gRPC `StreamEvents`) with `--event-log` flag. Remove the dead `events: Vec<EngineEvent>` field from result types.
17. **Add `preview_refresh()` and `preview_destroy()`** with CLI version gating (>= 3.105.0).
18. **Add inline program support** (gRPC `LanguageRuntime` server + `--client` flag), with version gating (>= 3.181.0 for refresh/destroy).
19. **Add remote workspace support** (`is_remote()`, `remote_args()`).
20. **Align naming**: rename `export_state`/`import_state` → `export_stack`/`import_stack`.

---

## Verdict

**MAJOR GAPS**

The Rust implementation covers fewer than 20% of the API surface of the upstream SDKs. The existing methods are functionally incomplete (silent bugs in `outputs()` and `import_state`, empty `events` fields, missing mandatory CLI flags). Every auxiliary operation — history, cancel, rename, tags, ESC environments, import resources, preview variants, inline programs, event streaming — is absent. No option structs exist for any operation. The module cannot replace any upstream SDK for real automation workloads in its current state.