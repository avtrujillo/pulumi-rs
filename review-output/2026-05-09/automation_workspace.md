# SDK Review: `automation_workspace`

**Date:** 2026-05-09  
**Rust file:** `pulumi-automation/src/workspace.rs`  

---

## Summary

The Rust `pulumi-automation` workspace module is a minimal, concrete implementation covering roughly 20–25% of the upstream API surface. It lacks a `Workspace` abstraction trait entirely, is missing whole capability categories (project/stack settings, ESC environments, tags, state export/import, stack outputs, org management), and the operations that do exist have behavioral divergences from the reference implementations (missing flags, wrong CLI argument construction, absent version-gating).

---

## Missing Features

### 1. No `Workspace` Abstraction Trait
All three upstream SDKs define `Workspace` as an abstract contract (Go `interface`, Node `interface`, Python `ABC`). Rust has only a concrete `LocalWorkspace` struct. There is no trait that can be implemented by alternative backends (remote workspaces, mock workspaces for testing).

### 2. Project & Stack Settings Management (Entire Category)
All upstreams expose read/write of `Pulumi.yaml` and `Pulumi.<stack>.yaml`:
- Go: `ProjectSettings()`, `SaveProjectSettings()`, `StackSettings()`, `SaveStackSettings()`
- Node/Python: `projectSettings()`, `saveProjectSettings()`, `stackSettings()`, `saveStackSettings()`

Rust has none of these, and carries no `ProjectSettings` or `StackSettings` types.

### 3. ESC Environment Management (Entire Category)
Added in Go/Node/Python at CLI ≥ 3.95/3.99:
- `AddEnvironments` / `add_environments` (≥ 3.95.0)
- `ListEnvironments` / `list_environments` (≥ 3.99.0)
- `RemoveEnvironment` / `remove_environment` (≥ 3.95.0)

None present in Rust.

### 4. Stack Tag Management (Entire Category)
- Go: `GetTag`, `SetTag`, `RemoveTag`, `ListTags`
- Node: `getTag`, `setTag`, `removeTag`, `listTags`
- Python: `get_tag`, `set_tag`, `remove_tag`, `list_tags`

None present in Rust. No `TagMap` type either.

### 5. State Export/Import (Entire Category)
- Go: `ExportStack(ctx, name) (apitype.UntypedDeployment, error)`, `ImportStack(ctx, name, state)`
- Node/Python: `exportStack`, `importStack`

None present in Rust. No `Deployment` type.

### 6. Stack Outputs
- Go: `StackOutputs(ctx, stackName) (OutputMap, error)`
- Node/Python: `stackOutputs(stackName)`

Not present in Rust. No `OutputMap` / `OutputValue` types.

### 7. Missing Config Operations
| Feature | Go | Node | Python | Rust |
|---|---|---|---|---|
| `set_all_config` (bulk set) | ✓ | ✓ | ✓ | ✗ |
| `remove_all_config` (bulk remove) | ✓ | ✓ | ✓ | ✗ |
| `refresh_config` | ✓ | ✓ | ✓ | ✗ |
| `set_all_config_json` | ✓ | ✓ | ✓ | ✗ |
| `--path` flag on get/set/remove | ✓ | ✓ | ✓ | ✗ |
| `--config-file` flag | ✓ | ✗ | ✗ | ✗ |
| `ConfigOptions` struct | ✓ | — | — | ✗ |
| `GetAllConfigOptions` (show secrets) | ✓ | ✗ | ✗ | ✗ |

### 8. Org Management
- Go: `OrgGetDefault`, `OrgSetDefault`
- Python: `org_get_default`, `org_set_default`

Not present in Rust.

### 9. Workspace Extensibility Hooks
- Go/Node/Python: `serialize_args_for_op(stack_name)`, `post_command_callback(stack_name)`

Not present in Rust.

### 10. Secrets Provider Management
- Go: `ChangeStackSecretsProvider(ctx, stackName, newSecretsProvider, opts)` + `ChangeSecretsProviderOptions`

Not present in Rust. Also no `secretsProvider` field on `LocalWorkspace`, so stack `init` never passes `--secrets-provider`.

### 11. Inline Program Support
- All upstreams: `PulumiFn` type, `program` field on workspace, `NewStackInlineSource`/`UpsertStackInlineSource`/`SelectStackInlineSource`

Rust has no concept of an inline (in-process) program.

### 12. Dependency & Project Scaffolding
- All upstreams: `Install(opts *InstallOptions)` (`pulumi install`)
- Go/Python: `New(opts)` (`pulumi new`)

Not present in Rust. No `InstallOptions`, `NewOptions`, `NewResult` types.

### 13. Git Repo & Remote Workspace Support
- All upstreams: `GitRepo`, `GitAuth`, `SetupFn`, remote workspace fields (`remote_env_vars`, `remote_pre_run_commands`, `remote_skip_install_dependencies`, `ExecutorImage`, `remote_agent_pool_id`, `remote_inherit_settings`)
- Go/Node: Factory functions `NewStackRemoteSource`, `SelectStackRemoteSource`, `UpsertStackRemoteSource`

None present in Rust.

### 14. Convenience Stack Factory Functions
- Go: `NewStackLocalSource`, `UpsertStackLocalSource`, `SelectStackLocalSource`, and 6 remote/inline variants (9 total)
- Node: `LocalWorkspace.createStack`, `selectStack`, `createOrSelectStack` static methods with `InlineProgramArgs`/`LocalProgramArgs`
- Python: `create_stack`, `select_stack`, `create_or_select_stack` module-level functions with inline/local dispatch

Rust has none of these ergonomic entry points.

### 15. Workspace Metadata Accessors
- Go: `PulumiHome()`, `PulumiVersion()`, `PulumiCommand()`, `GetEnvVars()`, `SetEnvVars()`, `UnsetEnvVar()`
- Node/Python: `pulumiHome`, `pulumiVersion`, `pulumiCommand`, `cli_api`

Rust has none of these. `with_env()` is builder-only and cannot remove vars or enumerate them at runtime.

---

## Behavioral Divergences

### 1. `get_all_config()` Omits `--show-secrets`
All upstreams pass `--show-secrets` when calling `pulumi config --json`:
- Go: `args = append(args, "--show-secrets")` (when `opts.ShowSecrets`)
- Node: `["config", "--show-secrets", "--json", ...]`
- Python: `["config", "--show-secrets", "--json", ...]`

Rust calls `["config", "--stack", stack, "--json"]` without `--show-secrets`, causing secret values to be masked (`[secret]`) in the returned map.

### 2. `set_config()` Argument Construction Is Injection-Unsafe
All upstreams use `--` before the value to prevent argument injection:
- Go: `args = append(args, key, secretArg, "--", val.Value)`
- Node: `args.push(key, ..., "--non-interactive", "--", value.value)`
- Python: `args.extend([key, secret_arg, "--stack", stack_name, "--non-interactive", "--", value.value])`

Rust: `vec!["config", "set", key, &value.value, "--stack", stack]` — the value is placed positionally without `--`, meaning a value like `--secret` would be misinterpreted as a flag.

### 3. `install_plugin()` Signature Is Non-Standard
- Go: `InstallPlugin(ctx, name, version)` — always `resource` kind
- Node: `installPlugin(name, version, kind="resource")` — kind is last, optional
- Python: `install_plugin(name, version, kind="resource")` — kind is last, optional
- Rust: `install_plugin(kind, name, version)` — kind is **first** and required

This breaks ergonomic parity with every other SDK.

### 4. `remove_plugin()` Signature Is Non-Standard
- Go: `RemovePlugin(ctx, name, version)`
- Node: `removePlugin(name?, versionRange?, kind="resource")`
- Python: `remove_plugin(name?, version_range?, kind="resource")`
- Rust: `remove_plugin(kind, name, version: Option<&str>)` — kind first, version range semantics differ

### 5. No CLI Version-Gating
Go, Node, and Python all enforce minimum CLI versions before calling newer commands:
- `add_environments` requires ≥ 3.95.0
- `list_environments` requires ≥ 3.99.0
- `install` requires ≥ 3.91.0
- `whoami --json` requires ≥ 3.58.0 (falls back to plain `whoami` on older CLIs)

Rust has no `PulumiCommand` / version-check mechanism at all; calling a future-gated command against an old CLI will produce an opaque error.

### 6. `WhoAmIResult` Structural Differences
- Rust `organizations: Vec<String>` with `#[serde(default)]` — always a `Vec`, never `None`
- Go/Node/Python: `organizations` is optional/`omitempty`
- Missing `token_information: Option<TokenInformation>` field present in all upstreams
- Go has a separate `WhoAmI()` → `String` alongside `WhoAmIDetails()` → `WhoAmIResult`; Rust conflates both

### 7. `StackSummary` Missing Fields
```rust
// Rust
pub struct StackSummary {
    pub name: String,
    pub current: bool,
    pub last_update: Option<String>,
    pub resource_count: Option<i64>,
    // MISSING: update_in_progress, url
}
```
- Missing `update_in_progress: Option<bool>` (present in all upstreams as `updateInProgress`)
- Missing `url: Option<String>` (present in all upstreams)

### 8. `list_stacks()` Missing `--all` Flag
- Go: `optListOpts.All` → `args = append(args, "--all")`
- Node: `opts.all` → `args.push("--all")`
- Python: `include_all` → `args.append("--all")`

Rust `list_stacks()` takes no parameters; always queries only the current project.

### 9. `remove_stack()` Missing `--preserve-config`
- Node: `opts?.preserveConfig` → `args.push("--preserve-config")`
- Python: `preserve_config` parameter

Rust only supports `force: bool`.

### 10. `PULUMI_HOME` Never Injected into CLI Calls
All upstreams set `PULUMI_HOME` env when `pulumiHome` is configured:
```go
homeEnv := fmt.Sprintf("%s=%s", pulumiHomeEnv, l.PulumiHome())
env = append(env, homeEnv)
```
Rust has no `pulumiHome` field and never sets this env var, making it impossible to use custom Pulumi home directories.

### 11. `logs()` Has No Upstream Counterpart
`LocalWorkspace::logs()` calls `pulumi logs --stack` which is not part of any upstream workspace API. This is either a custom addition or misplaced — in upstreams, logs are an operation on `Stack`, not `Workspace`.

---

## API Surface Gaps

### Types Missing Entirely
| Type | Go | Node | Python | Rust |
|---|---|---|---|---|
| `Workspace` trait/interface | ✓ | ✓ | ✓ | ✗ |
| `ProjectSettings` | ✓ | ✓ | ✓ | ✗ |
| `StackSettings` | ✓ | ✓ | ✓ | ✗ |
| `ConfigMap` type alias | ✓ | ✓ | ✓ | ✗ |
| `ConfigOptions` | ✓ | — | — | ✗ |
| `GetAllConfigOptions` | ✓ | — | — | ✗ |
| `OutputMap` / `OutputValue` | ✓ | ✓ | ✓ | ✗ |
| `Deployment` | ✓ | ✓ | ✓ | ✗ |
| `TokenInformation` | ✓ | ✓ | ✓ | ✗ |
| `PulumiFn` | ✓ | ✓ | ✓ | ✗ |
| `ChangeSecretsProviderOptions` | ✓ | — | — | ✗ |
| `InstallOptions` | ✓ | ✓ | ✓ | ✗ |
| `NewOptions` / `NewResult` | ✓ | — | ✓ | ✗ |
| `GitRepo` / `GitAuth` / `SetupFn` | ✓ | ✓ | ✓ | ✗ |
| `EnvVarValue` (remote secret envs) | ✓ | ✓ | ✓ | ✗ |
| `ExecutorImage` / `DockerImageCredentials` | ✓ | ✓ | ✓ | ✗ |
| `LocalWorkspaceOptions` | ✓ | ✓ | ✓ | ✗ |
| `TagMap` | ✓ | ✓ | ✓ | ✗ |
| `InlineProgramArgs` / `LocalProgramArgs` | — | ✓ | — | ✗ |
| `ListOptions` / `RemoveOptions` | ✓ | ✓ | — | ✗ |

### Missing Public Methods on `LocalWorkspace`
```
project_settings() / save_project_settings()
stack_settings() / save_stack_settings()
add_environments() / list_environments() / remove_environment()
set_all_config() / remove_all_config() / refresh_config() / set_all_config_json()
get_tag() / set_tag() / remove_tag() / list_tags()
stack()                          // selected stack summary
export_stack() / import_stack()  // state management
stack_outputs()                  // output values
org_get_default() / org_set_default()
change_stack_secrets_provider()
install()                        // pulumi install
new()                            // pulumi new
pulumi_home() / pulumi_version() / pulumi_command()
get_env_vars() / unset_env_var() // runtime env mutation
serialize_args_for_op() / post_command_callback()  // extensibility hooks
```

---

## Recommendations

### Priority 1 — Correctness Fixes (Break Existing Behavior)
1. **Fix `set_config()` to use `--` before the value** and add `--non-interactive`. This is a correctness bug that causes silent misbehavior with values that look like flags.
2. **Fix `get_all_config()` to pass `--show-secrets`**. The current implementation returns masked secrets, making the method useless for most automation scenarios.
3. **Fix `StackSummary`** to add `update_in_progress: Option<bool>` and `url: Option<String>` so deserialization doesn't silently drop fields.
4. **Fix `WhoAmIResult`** to add `token_information: Option<TokenInformation>` and make `organizations: Option<Vec<String>>`.

### Priority 2 — Core Missing Operations (Frequently Used)
5. **Define a `Workspace` trait** mirroring the Go interface. `LocalWorkspace` should implement it. This is the architectural foundation everything else builds on, and its absence blocks downstream testing strategies.
6. **Add `project_settings()` / `save_project_settings()` / `stack_settings()` / `save_stack_settings()`** with `ProjectSettings` and `StackSettings` types. These are needed for any meaningful workspace configuration.
7. **Add `set_all_config()`, `remove_all_config()`, `refresh_config()`, `set_all_config_json()`**. The per-key operations are insufficient for real automation workflows.
8. **Add `stack_outputs()`** with `OutputMap`/`OutputValue`. This is a primary use case of the automation API.
9. **Add `export_stack()` / `import_stack()`** with `Deployment` type. Required for state recovery workflows.
10. **Add `LocalWorkspaceOptions`** struct and refactor constructors to match the factory-function pattern (`new_stack_local_source`, `upsert_stack_local_source`, `select_stack_local_source`, `new_stack_inline_source`, etc.).

### Priority 3 — Important Missing Categories
11. **Add tag operations** (`get_tag`, `set_tag`, `remove_tag`, `list_tags`). Used for stack metadata.
12. **Add ESC environment operations** (`add_environments`, `list_environments`, `remove_environment`) with CLI version-gating.
13. **Add `install()` / `new()`** with `InstallOptions`/`NewOptions` types.
14. **Add `org_get_default()` / `org_set_default()`**.
15. **Fix `install_plugin()` and `remove_plugin()` signatures** to match the standard `(name, version, kind="resource")` pattern.
16. **Add `pulumi_home` field** and inject `PULUMI_HOME` into all CLI invocations.

### Priority 4 — Advanced Features
17. **Add `PulumiCommand` integration** with version tracking and gating of CLI-version-dependent features.
18. **Add `change_stack_secrets_provider()`** and `secrets_provider` field on workspace.
19. **Add Git repo / remote workspace support** (`GitRepo`, `GitAuth`, remote execution fields).
20. **Add runtime env var mutation**: `get_env_vars()`, `set_env_vars()`, `unset_env_var()`.
21. **Remove or relocate `logs()`** — it has no upstream counterpart on `Workspace` and should be on `Stack` if anywhere.

---

## Verdict

**MAJOR GAPS**

The Rust implementation covers approximately 20–25% of the upstream API surface. Five entire capability categories are absent (project/stack settings, ESC environments, tags, state management, outputs), the existing methods contain correctness bugs (`set_config` argument injection risk, `get_all_config` masked secrets), there is no `Workspace` abstraction trait, and structural types (`StackSummary`, `WhoAmIResult`) are missing upstream-defined fields. The implementation is not suitable for production automation workloads in its current state.