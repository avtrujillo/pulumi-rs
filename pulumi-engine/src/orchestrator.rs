//! Engine orchestrator — starts gRPC servers and runs the user's Pulumi program.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use tokio::process::Command;
use tonic::transport::Server;

use crate::diff::{RefreshDiff, diff_refresh};
use crate::engine_service::EngineServiceImpl;
use crate::error::{Error, Result};
use crate::events;
use crate::monitor_service::ResourceMonitorImpl;
use crate::provider::{self, GrpcProvider, Provider, ProviderManager, json_to_proto_struct};
use crate::pulumirpc;
use crate::secrets::PassphraseSecretsManager;
use crate::state::{Checkpoint, EngineState};

/// Configuration for a Pulumi engine run.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// The project name.
    pub project: String,
    /// The stack name.
    pub stack: String,
    /// Working directory (should contain Pulumi.yaml).
    pub work_dir: PathBuf,
    /// The program command to run (e.g. `["./target/release/my-program"]`).
    pub program: Vec<String>,
    /// Whether this is a dry run (preview).
    pub dry_run: bool,
    /// Pulumi configuration key-value pairs passed to the program via PULUMI_CONFIG.
    /// Keys should be fully-qualified (e.g. `"myproject:apiKey"`).
    pub config: HashMap<String, String>,
    /// Config keys whose values are secret (passed via PULUMI_CONFIG_SECRET_KEYS).
    pub config_secret_keys: Vec<String>,
    /// Extra environment variables to pass to the program.
    pub env: HashMap<String, String>,
    /// Path to the checkpoint file for state persistence.
    /// Defaults to `<work_dir>/.pulumi-rs/<stack>.json`.
    pub checkpoint_path: Option<PathBuf>,
    /// Secrets manager for encrypting/decrypting secret values in checkpoints.
    /// When `None`, secrets are stored in plaintext.
    pub secrets_manager: Option<PassphraseSecretsManager>,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            project: String::new(),
            stack: String::new(),
            work_dir: PathBuf::from("."),
            program: Vec::new(),
            dry_run: false,
            config: HashMap::new(),
            config_secret_keys: Vec::new(),
            env: HashMap::new(),
            checkpoint_path: None,
            secrets_manager: None,
        }
    }
}

impl EngineOptions {
    /// Resolve the checkpoint file path.
    fn checkpoint_path(&self) -> PathBuf {
        self.checkpoint_path.clone().unwrap_or_else(|| {
            self.work_dir
                .join(".pulumi-rs")
                .join(format!("{}.json", self.stack))
        })
    }
}

/// The result of running a Pulumi program through the native engine.
#[derive(Debug, Clone)]
pub struct UpResult {
    /// Stack outputs collected from RegisterResourceOutputs on the stack resource.
    pub outputs: serde_json::Value,
    /// The stdout from the user program.
    pub stdout: String,
    /// The stderr from the user program.
    pub stderr: String,
    /// Structured events emitted during the operation.
    pub events: Vec<crate::events::EngineEvent>,
}

/// The result of a destroy operation.
#[derive(Debug, Clone)]
pub struct DestroyResult {
    /// Summary of destroyed resources.
    pub stdout: String,
    /// Any warnings or errors.
    pub stderr: String,
    /// Structured events emitted during the operation.
    pub events: Vec<crate::events::EngineEvent>,
}

/// The result of a refresh operation.
#[derive(Debug, Clone)]
pub struct RefreshResult {
    /// Summary output.
    pub stdout: String,
    /// Any warnings or errors.
    pub stderr: String,
    /// Per-resource drift information.
    pub diffs: Vec<RefreshDiff>,
    /// Structured events emitted during the operation.
    pub events: Vec<crate::events::EngineEvent>,
}

/// The Rust-native Pulumi engine, parameterized over a [`Provider`] implementation.
///
/// Defaults to [`GrpcProvider`], which launches real provider plugin subprocesses.
/// Substitute a different `P` (e.g. a mock) for testing without real providers.
pub struct PulumiEngine<P: Provider = GrpcProvider> {
    options: EngineOptions,
    providers: ProviderManager<P>,
}

impl PulumiEngine {
    pub fn new(options: EngineOptions) -> Self {
        Self {
            options,
            providers: ProviderManager::new(),
        }
    }
}

impl<P: Provider> PulumiEngine<P> {
    /// Create an engine with a custom provider manager (e.g. for testing).
    pub fn with_providers(options: EngineOptions, providers: ProviderManager<P>) -> Self {
        Self { options, providers }
    }

    /// Load a checkpoint, decrypting secrets if a secrets manager is configured.
    async fn load_checkpoint(&self, path: &Path) -> Result<Option<Checkpoint>> {
        if let Some(sm) = &self.options.secrets_manager {
            Checkpoint::load_encrypted(path, sm)
                .await
                .map_err(|e| Error::Custom(format!("failed to load checkpoint: {e}")))
        } else {
            Checkpoint::load(path)
                .map_err(|e| Error::Custom(format!("failed to load checkpoint: {e}")))
        }
    }

    /// Save a checkpoint, encrypting secrets if a secrets manager is configured.
    async fn save_checkpoint(&self, checkpoint: &Checkpoint, path: &Path) -> Result<()> {
        if let Some(sm) = &self.options.secrets_manager {
            checkpoint
                .save_encrypted(path, sm)
                .await
                .map_err(|e| Error::Custom(format!("failed to save checkpoint: {e}")))
        } else {
            checkpoint
                .save(path)
                .map_err(|e| Error::Custom(format!("failed to save checkpoint: {e}")))
        }
    }

    /// Run the Pulumi program (equivalent to `pulumi up`).
    pub async fn up(&self) -> Result<UpResult> {
        self.run_program(false).await
    }

    /// Run the Pulumi program in preview/dry-run mode.
    pub async fn preview(&self) -> Result<UpResult> {
        self.run_program(true).await
    }

    /// Destroy all resources in the stack.
    ///
    /// Calls [`Provider::delete`] for each custom resource in reverse
    /// dependency order, then clears the checkpoint.
    pub async fn destroy(&self) -> Result<DestroyResult> {
        let start = std::time::Instant::now();
        let checkpoint_path = self.options.checkpoint_path();
        let checkpoint = self.load_checkpoint(&checkpoint_path).await?;

        let mut ev: Vec<events::EngineEvent> = Vec::new();
        let mut summary = String::new();

        match checkpoint {
            None => {
                summary.push_str("No resources to destroy (no checkpoint found).\n");
            }
            Some(cp) => {
                let config_json = serde_json::to_value(&self.options.config).unwrap_or_default();
                ev.push(events::EngineEvent::Prelude { config: config_json });

                let state = EngineState::from_checkpoint(&cp);
                let resources = state.get_prior_resources().await;

                if resources.is_empty() {
                    summary.push_str("No resources to destroy.\n");
                } else {
                    let mut ordered: Vec<_> = cp.resources.iter().collect();
                    ordered.reverse();

                    for res in &ordered {
                        if res.custom {
                            if let Some(package) = provider::package_name(&res.resource_type) {
                                match self.providers.get_provider(&package).await {
                                    Ok(mut provider) => {
                                        eprintln!(
                                            "[engine] delete: {} ({})",
                                            res.urn, res.resource_type
                                        );
                                        let delete_result = provider
                                            .delete(pulumirpc::DeleteRequest {
                                                id: res.id.clone(),
                                                urn: res.urn.clone(),
                                                properties: Some(json_to_proto_struct(
                                                    &res.outputs,
                                                )),
                                                timeout: 0.0,
                                                old_inputs: Some(json_to_proto_struct(&res.inputs)),
                                                name: res.name.clone(),
                                                r#type: res.resource_type.clone(),
                                                ..Default::default()
                                            })
                                            .await;
                                        if let Err(e) = delete_result {
                                            eprintln!(
                                                "[engine] warning: delete failed for {}: {e}",
                                                res.urn
                                            );
                                        }
                                        summary.push_str(&format!(
                                            "- delete {} ({})\n",
                                            res.name, res.resource_type
                                        ));
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "[engine] warning: could not connect to provider for {}: {e}",
                                            res.resource_type
                                        );
                                        summary.push_str(&format!(
                                            "- delete {} ({}) [provider unavailable]\n",
                                            res.name, res.resource_type
                                        ));
                                    }
                                }
                            }
                        } else {
                            eprintln!("[engine] remove: {} ({})", res.urn, res.resource_type);
                            summary.push_str(&format!(
                                "- remove {} ({})\n",
                                res.name, res.resource_type
                            ));
                        }
                        ev.push(events::EngineEvent::ResourceStep {
                            op: "delete".to_string(),
                            urn: res.urn.clone(),
                            resource_type: res.resource_type.clone(),
                            old_inputs: res.inputs.clone(),
                            old_outputs: res.outputs.clone(),
                            new_inputs: serde_json::Value::Null,
                            new_outputs: serde_json::Value::Null,
                        });
                    }

                    self.providers.shutdown_all().await;

                    let empty = state.empty_checkpoint().await;
                    self.save_checkpoint(&empty, &checkpoint_path).await?;

                    summary.push_str(&format!("\nDestroyed {} resource(s).\n", ordered.len()));
                }
            }
        }

        let resource_changes = events::count_resource_changes(&ev);
        ev.push(events::EngineEvent::Summary {
            may_update: true,
            duration_seconds: start.elapsed().as_secs() as i64,
            resource_changes,
        });

        Ok(DestroyResult {
            stdout: summary,
            stderr: String::new(),
            events: ev,
        })
    }

    /// Refresh the stack state.
    ///
    /// Calls [`Provider::read`] for each custom resource to sync state with
    /// the actual cloud provider, then saves the updated checkpoint.
    pub async fn refresh(&self) -> Result<RefreshResult> {
        let start = std::time::Instant::now();
        let checkpoint_path = self.options.checkpoint_path();
        let checkpoint = self.load_checkpoint(&checkpoint_path).await?;

        let mut ev: Vec<events::EngineEvent> = Vec::new();
        let mut summary = String::new();

        match checkpoint {
            None => {
                summary.push_str("No checkpoint found, nothing to refresh.\n");
            }
            Some(mut cp) => {
                let config_json = serde_json::to_value(&self.options.config).unwrap_or_default();
                ev.push(events::EngineEvent::Prelude { config: config_json });

                let mut updated_resources = Vec::new();
                let mut diffs = Vec::new();

                for res in &cp.resources {
                    eprintln!("[engine] refresh: {} ({})", res.urn, res.resource_type);

                    if res.custom {
                        if let Some(package) = provider::package_name(&res.resource_type) {
                            match self.providers.get_provider(&package).await {
                                Ok(mut provider) => {
                                    let read_result = provider
                                        .read(pulumirpc::ReadRequest {
                                            id: res.id.clone(),
                                            urn: res.urn.clone(),
                                            properties: Some(json_to_proto_struct(&res.outputs)),
                                            inputs: Some(json_to_proto_struct(&res.inputs)),
                                            name: res.name.clone(),
                                            r#type: res.resource_type.clone(),
                                            ..Default::default()
                                        })
                                        .await;

                                    match read_result {
                                        Ok(resp) if resp.id.is_empty() => {
                                            // Provider returned empty ID — resource
                                            // was deleted out-of-band. Remove from state.
                                            eprintln!(
                                                "[engine] refresh: {} deleted upstream",
                                                res.urn
                                            );
                                            diffs.push(diff_refresh(
                                                &res.urn,
                                                &res.outputs,
                                                None,
                                            ));
                                        }
                                        Ok(resp) => {
                                            let live_outputs = resp
                                                .properties
                                                .as_ref()
                                                .map(crate::monitor_service::proto_struct_to_json);

                                            diffs.push(diff_refresh(
                                                &res.urn,
                                                &res.outputs,
                                                live_outputs.as_ref(),
                                            ));

                                            let mut refreshed = res.clone();
                                            refreshed.id = resp.id;
                                            if let Some(outputs) = live_outputs {
                                                refreshed.outputs = outputs;
                                            }
                                            if let Some(inputs) = resp.inputs {
                                                refreshed.inputs =
                                                    crate::monitor_service::proto_struct_to_json(
                                                        &inputs,
                                                    );
                                            }
                                            refreshed.refresh_before_update =
                                                resp.refresh_before_update;
                                            updated_resources.push(refreshed);
                                        }
                                        Err(e) => {
                                            eprintln!(
                                                "[engine] warning: refresh read failed for {}: {e}",
                                                res.urn
                                            );
                                            updated_resources.push(res.clone());
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!(
                                        "[engine] warning: could not connect to provider for {}: {e}",
                                        res.resource_type
                                    );
                                    updated_resources.push(res.clone());
                                }
                            }
                        } else {
                            updated_resources.push(res.clone());
                        }
                    } else {
                        updated_resources.push(res.clone());
                    }
                }

                self.providers.shutdown_all().await;

                cp.resources = updated_resources;
                self.save_checkpoint(&cp, &checkpoint_path).await?;

                // Emit a ResourceStep event for each refreshed resource.
                for d in &diffs {
                    use crate::diff::RefreshAction;
                    let op = match d.action {
                        RefreshAction::Same => "same",
                        RefreshAction::Updated => "update",
                        RefreshAction::Deleted => "delete",
                    };
                    // For refresh we don't have the full old/new state here,
                    // so use Null placeholders — the diff already captures the
                    // changed keys.
                    ev.push(events::EngineEvent::ResourceStep {
                        op: op.to_string(),
                        urn: d.urn.clone(),
                        resource_type: String::new(),
                        old_inputs: serde_json::Value::Null,
                        old_outputs: serde_json::Value::Null,
                        new_inputs: serde_json::Value::Null,
                        new_outputs: serde_json::Value::Null,
                    });
                }

                let resource_changes = events::count_resource_changes(&ev);
                ev.push(events::EngineEvent::Summary {
                    may_update: false,
                    duration_seconds: start.elapsed().as_secs() as i64,
                    resource_changes,
                });

                summary = format_refresh_summary(&diffs);
                return Ok(RefreshResult {
                    stdout: summary,
                    stderr: String::new(),
                    diffs,
                    events: ev,
                });
            }
        }

        Ok(RefreshResult {
            stdout: summary,
            stderr: String::new(),
            diffs: vec![],
            events: ev,
        })
    }

    async fn run_program(&self, dry_run: bool) -> Result<UpResult> {
        let start = std::time::Instant::now();
        let checkpoint_path = self.options.checkpoint_path();

        // Load prior state from checkpoint.
        let state = match self.load_checkpoint(&checkpoint_path).await? {
            Some(cp) => EngineState::from_checkpoint(&cp),
            None => EngineState::new(self.options.project.clone(), self.options.stack.clone()),
        };

        // Shared event collector — passed to both gRPC services.
        let event_collector = events::new_collector();

        // Emit the prelude event with the current config.
        events::emit(
            &event_collector,
            events::EngineEvent::Prelude {
                config: serde_json::to_value(&self.options.config).unwrap_or_default(),
            },
        );

        // Bind the gRPC servers to ephemeral ports.
        let monitor_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let engine_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

        let monitor_listener = tokio::net::TcpListener::bind(monitor_addr).await?;
        let engine_listener = tokio::net::TcpListener::bind(engine_addr).await?;

        let monitor_port = monitor_listener.local_addr()?.port();
        let engine_port = engine_listener.local_addr()?.port();

        let monitor_addr_str = format!("127.0.0.1:{monitor_port}");
        let engine_addr_str = format!("127.0.0.1:{engine_port}");

        // Tell the provider manager where the engine is so plugins can connect.
        self.providers
            .set_engine_addr(engine_addr_str.clone())
            .await;

        // Start the ResourceMonitor server.
        let monitor_svc = ResourceMonitorImpl::new(
            state.clone(),
            dry_run || self.options.dry_run,
            self.providers.clone(),
            Some(event_collector.clone()),
        );
        let monitor_handle = tokio::spawn(async move {
            Server::builder()
                .add_service(
                    pulumirpc::resource_monitor_server::ResourceMonitorServer::new(monitor_svc),
                )
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                    monitor_listener,
                ))
                .await
        });

        // Start the Engine server.
        let engine_svc = EngineServiceImpl::new(state.clone(), Some(event_collector.clone()));
        let engine_handle = tokio::spawn(async move {
            Server::builder()
                .add_service(pulumirpc::engine_server::EngineServer::new(engine_svc))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                    engine_listener,
                ))
                .await
        });

        // Build the PULUMI_CONFIG JSON ({"key": "value", ...}).
        let config_json = serde_json::to_string(&self.options.config).unwrap_or_default();
        let secret_keys_json =
            serde_json::to_string(&self.options.config_secret_keys).unwrap_or_default();

        // Spawn the user's program with PULUMI_* env vars.
        let program = &self.options.program;
        if program.is_empty() {
            return Err(Error::Custom("no program command specified".into()));
        }

        let mut cmd = Command::new(&program[0]);
        if program.len() > 1 {
            cmd.args(&program[1..]);
        }

        cmd.current_dir(&self.options.work_dir)
            .env("PULUMI_MONITOR", &monitor_addr_str)
            .env("PULUMI_ENGINE", &engine_addr_str)
            .env("PULUMI_PROJECT", &self.options.project)
            .env("PULUMI_STACK", &self.options.stack)
            .env(
                "PULUMI_DRY_RUN",
                if dry_run || self.options.dry_run {
                    "true"
                } else {
                    "false"
                },
            )
            .env("PULUMI_PARALLEL", "-1")
            .env("PULUMI_CONFIG", &config_json)
            .env("PULUMI_CONFIG_SECRET_KEYS", &secret_keys_json);

        for (k, v) in &self.options.env {
            cmd.env(k, v);
        }

        let output = cmd.output().await.map_err(Error::Spawn)?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        // Abort the gRPC servers and shut down providers.
        monitor_handle.abort();
        engine_handle.abort();
        self.providers.shutdown_all().await;

        if !output.status.success() {
            return Err(Error::ProgramFailed {
                code: output.status.code().unwrap_or(-1),
                stdout,
                stderr,
            });
        }

        // Handle resources deleted from prior state (not re-registered).
        let deleted_urns = state.get_deleted_urns().await;
        for urn in &deleted_urns {
            let prior = state.get_prior_resource(urn).await;
            if let Some(res) = prior {
                if res.custom {
                    if let Some(package) = provider::package_name(&res.resource_type) {
                        if !dry_run && !self.options.dry_run {
                            match self.providers.get_provider(&package).await {
                                Ok(mut provider) => {
                                    eprintln!("[engine] delete: {urn}");
                                    let _ = provider
                                        .delete(pulumirpc::DeleteRequest {
                                            id: res.id.clone(),
                                            urn: urn.clone(),
                                            properties: Some(json_to_proto_struct(&res.outputs)),
                                            timeout: 0.0,
                                            old_inputs: Some(json_to_proto_struct(&res.inputs)),
                                            name: res.name.clone(),
                                            r#type: res.resource_type.clone(),
                                            ..Default::default()
                                        })
                                        .await;
                                }
                                Err(e) => {
                                    eprintln!("[engine] warning: could not delete {urn}: {e}");
                                }
                            }
                        } else {
                            eprintln!("[engine] delete (preview): {urn}");
                        }
                    }
                    events::emit(
                        &event_collector,
                        events::EngineEvent::ResourceStep {
                            op: "delete".to_string(),
                            urn: urn.clone(),
                            resource_type: res.resource_type.clone(),
                            old_inputs: res.inputs.clone(),
                            old_outputs: res.outputs.clone(),
                            new_inputs: serde_json::Value::Null,
                            new_outputs: serde_json::Value::Null,
                        },
                    );
                } else {
                    eprintln!("[engine] delete: {urn}");
                    events::emit(
                        &event_collector,
                        events::EngineEvent::ResourceStep {
                            op: "delete".to_string(),
                            urn: urn.clone(),
                            resource_type: String::new(),
                            old_inputs: serde_json::Value::Null,
                            old_outputs: serde_json::Value::Null,
                            new_inputs: serde_json::Value::Null,
                            new_outputs: serde_json::Value::Null,
                        },
                    );
                }
            } else {
                eprintln!("[engine] delete: {urn}");
            }
        }

        // Shut down any providers launched for deletes.
        self.providers.shutdown_all().await;

        let outputs = state.get_stack_outputs().await;

        // Save checkpoint (unless dry run).
        if !dry_run && !self.options.dry_run {
            let checkpoint = state.to_checkpoint().await;
            self.save_checkpoint(&checkpoint, &checkpoint_path).await?;
        }

        // Emit summary and collect all events.
        let collected = events::drain(&event_collector);
        let resource_changes = events::count_resource_changes(&collected);
        events::emit(
            &event_collector,
            events::EngineEvent::Summary {
                may_update: !dry_run && !self.options.dry_run,
                duration_seconds: start.elapsed().as_secs() as i64,
                resource_changes,
            },
        );
        let all_events = events::drain(&event_collector);

        Ok(UpResult {
            outputs,
            stdout,
            stderr,
            events: all_events,
        })
    }
}

/// Build a human-readable refresh summary from per-resource diffs.
fn format_refresh_summary(diffs: &[RefreshDiff]) -> String {
    use crate::diff::RefreshAction;

    if diffs.is_empty() {
        return String::new();
    }

    let mut same = 0u32;
    let mut updated = 0u32;
    let mut deleted = 0u32;
    let mut lines = Vec::new();

    for d in diffs {
        match d.action {
            RefreshAction::Same => same += 1,
            RefreshAction::Updated => {
                updated += 1;
                if d.changed_keys.is_empty() {
                    lines.push(format!("  ~ {} — outputs changed", d.urn));
                } else {
                    lines.push(format!(
                        "  ~ {} — outputs changed: [{}]",
                        d.urn,
                        d.changed_keys.join(", "),
                    ));
                }
            }
            RefreshAction::Deleted => {
                deleted += 1;
                lines.push(format!("  - {} — deleted upstream", d.urn));
            }
        }
    }

    let total = same + updated + deleted;
    let mut summary = format!("Refreshed {total} resource(s):");
    if same > 0 {
        summary.push_str(&format!(" {same} unchanged"));
    }
    if updated > 0 {
        if same > 0 {
            summary.push(',');
        }
        summary.push_str(&format!(" {updated} updated"));
    }
    if deleted > 0 {
        if same > 0 || updated > 0 {
            summary.push(',');
        }
        summary.push_str(&format!(" {deleted} deleted"));
    }
    summary.push('\n');

    for line in &lines {
        summary.push_str(line);
        summary.push('\n');
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{RefreshAction, RefreshDiff};
    use crate::events::EngineEvent;
    use crate::provider::ProviderManager;
    use crate::state::ResourceState;
    use crate::test_utils::MockProvider;

    fn make_engine(dir: &std::path::Path) -> PulumiEngine<MockProvider> {
        let opts = EngineOptions {
            project: "test-proj".into(),
            stack: "dev".into(),
            work_dir: dir.to_path_buf(),
            ..Default::default()
        };
        PulumiEngine::with_providers(opts, ProviderManager::new())
    }

    fn sample_resource(name: &str, custom: bool) -> ResourceState {
        ResourceState {
            urn: format!("urn:pulumi:dev::test-proj::pkg:mod:Res::{name}"),
            id: if custom { format!("id-{name}") } else { String::new() },
            resource_type: "pkg:mod:Res".into(),
            name: name.into(),
            custom,
            parent: String::new(),
            inputs: serde_json::json!({}),
            outputs: serde_json::json!({}),
            dependencies: vec![],
            secret_properties: vec![],
            refresh_before_update: false,
        }
    }

    fn save_checkpoint(dir: &std::path::Path, resources: Vec<ResourceState>) {
        let cp = Checkpoint {
            version: 1,
            project: "test-proj".into(),
            stack: "dev".into(),
            resources,
            outputs: serde_json::json!({}),
            secrets_provider: None,
        };
        let path = dir.join(".pulumi-rs").join("dev.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        cp.save(&path).unwrap();
    }

    // --- destroy ---

    #[tokio::test]
    async fn destroy_no_checkpoint_returns_no_resources_message() {
        let dir = tempfile::tempdir().unwrap();
        let engine = make_engine(dir.path());
        let result = engine.destroy().await.unwrap();
        assert!(result.stdout.contains("No resources"));
        // Summary event should still be emitted.
        assert!(result.events.iter().any(|e| matches!(e, EngineEvent::Summary { .. })));
    }

    #[tokio::test]
    async fn destroy_component_only_checkpoint_clears_state() {
        let dir = tempfile::tempdir().unwrap();
        save_checkpoint(dir.path(), vec![sample_resource("comp", false)]);

        let engine = make_engine(dir.path());
        let result = engine.destroy().await.unwrap();
        assert!(result.stdout.contains("remove"));

        let cp_path = dir.path().join(".pulumi-rs").join("dev.json");
        let loaded = Checkpoint::load(&cp_path).unwrap().unwrap();
        assert!(loaded.resources.is_empty(), "checkpoint should be cleared after destroy");
    }

    #[tokio::test]
    async fn destroy_custom_resource_checkpoint_clears_state() {
        let dir = tempfile::tempdir().unwrap();
        save_checkpoint(dir.path(), vec![sample_resource("bucket", true)]);

        let engine = make_engine(dir.path());
        let result = engine.destroy().await.unwrap();
        assert!(result.stdout.contains("Destroyed"));

        let cp_path = dir.path().join(".pulumi-rs").join("dev.json");
        let loaded = Checkpoint::load(&cp_path).unwrap().unwrap();
        assert!(loaded.resources.is_empty());
    }

    #[tokio::test]
    async fn destroy_emits_resource_step_events() {
        let dir = tempfile::tempdir().unwrap();
        save_checkpoint(
            dir.path(),
            vec![sample_resource("a", false), sample_resource("b", true)],
        );

        let engine = make_engine(dir.path());
        let result = engine.destroy().await.unwrap();

        let delete_events: Vec<_> = result
            .events
            .iter()
            .filter(|e| matches!(e, EngineEvent::ResourceStep { op, .. } if op == "delete"))
            .collect();
        assert_eq!(delete_events.len(), 2);
    }

    // --- refresh ---

    #[tokio::test]
    async fn refresh_no_checkpoint_returns_empty_diffs() {
        let dir = tempfile::tempdir().unwrap();
        let engine = make_engine(dir.path());
        let result = engine.refresh().await.unwrap();
        assert!(result.diffs.is_empty());
        assert!(result.stdout.contains("No checkpoint"));
    }

    #[tokio::test]
    async fn refresh_component_resource_skips_provider() {
        let dir = tempfile::tempdir().unwrap();
        save_checkpoint(dir.path(), vec![sample_resource("comp", false)]);

        let engine = make_engine(dir.path());
        let result = engine.refresh().await.unwrap();
        // Component resource has no provider read; diffs list is empty.
        assert!(result.diffs.is_empty());
    }

    // --- up (error cases) ---

    #[tokio::test]
    async fn up_empty_program_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let engine = make_engine(dir.path());
        let err = engine.up().await.unwrap_err();
        assert!(err.to_string().contains("no program"));
    }

    #[tokio::test]
    async fn preview_empty_program_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let engine = make_engine(dir.path());
        let err = engine.preview().await.unwrap_err();
        assert!(err.to_string().contains("no program"));
    }

    #[test]
    fn test_format_summary_empty() {
        assert_eq!(format_refresh_summary(&[]), "");
    }

    #[test]
    fn test_format_summary_all_same() {
        let diffs = vec![
            RefreshDiff {
                urn: "urn:pulumi:dev::proj::pkg:mod:A::a".into(),
                action: RefreshAction::Same,
                changed_keys: vec![],
            },
            RefreshDiff {
                urn: "urn:pulumi:dev::proj::pkg:mod:B::b".into(),
                action: RefreshAction::Same,
                changed_keys: vec![],
            },
        ];
        let summary = format_refresh_summary(&diffs);
        assert_eq!(summary, "Refreshed 2 resource(s): 2 unchanged\n");
    }

    #[test]
    fn test_format_summary_updated_with_keys() {
        let diffs = vec![RefreshDiff {
            urn: "urn:pulumi:dev::proj::aws:s3:Bucket::b".into(),
            action: RefreshAction::Updated,
            changed_keys: vec!["tags".into(), "versioning".into()],
        }];
        let summary = format_refresh_summary(&diffs);
        assert!(summary.starts_with("Refreshed 1 resource(s): 1 updated\n"));
        assert!(summary.contains("~ urn:pulumi:dev::proj::aws:s3:Bucket::b"));
        assert!(summary.contains("[tags, versioning]"));
    }

    #[test]
    fn test_format_summary_updated_no_keys() {
        let diffs = vec![RefreshDiff {
            urn: "urn:pulumi:dev::proj::pkg:mod:R::r".into(),
            action: RefreshAction::Updated,
            changed_keys: vec![],
        }];
        let summary = format_refresh_summary(&diffs);
        assert!(summary.contains("outputs changed\n"));
        assert!(!summary.contains('['));
    }

    #[test]
    fn test_format_summary_deleted() {
        let diffs = vec![RefreshDiff {
            urn: "urn:pulumi:dev::proj::aws:ec2:Instance::gone".into(),
            action: RefreshAction::Deleted,
            changed_keys: vec![],
        }];
        let summary = format_refresh_summary(&diffs);
        assert!(summary.starts_with("Refreshed 1 resource(s): 1 deleted\n"));
        assert!(summary.contains("- urn:pulumi:dev::proj::aws:ec2:Instance::gone"));
        assert!(summary.contains("deleted upstream"));
    }

    #[test]
    fn test_format_summary_mixed() {
        let diffs = vec![
            RefreshDiff {
                urn: "urn:pulumi:dev::proj::pkg:mod:A::a".into(),
                action: RefreshAction::Same,
                changed_keys: vec![],
            },
            RefreshDiff {
                urn: "urn:pulumi:dev::proj::pkg:mod:B::b".into(),
                action: RefreshAction::Updated,
                changed_keys: vec!["size".into()],
            },
            RefreshDiff {
                urn: "urn:pulumi:dev::proj::pkg:mod:C::c".into(),
                action: RefreshAction::Deleted,
                changed_keys: vec![],
            },
        ];
        let summary = format_refresh_summary(&diffs);
        assert!(summary.starts_with("Refreshed 3 resource(s): 1 unchanged, 1 updated, 1 deleted\n"));
        // Only updated and deleted resources get detail lines.
        assert!(summary.contains("~ urn:pulumi:dev::proj::pkg:mod:B::b"));
        assert!(summary.contains("- urn:pulumi:dev::proj::pkg:mod:C::c"));
        assert!(!summary.contains("pkg:mod:A::a"));
    }
}
