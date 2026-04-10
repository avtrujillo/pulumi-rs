//! Engine orchestrator — starts gRPC servers and runs the user's Pulumi program.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use tokio::process::Command;
use tonic::transport::Server;

use crate::diff::{RefreshDiff, diff_refresh};
use crate::engine_service::EngineServiceImpl;
use crate::error::{Error, Result};
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
}

/// The result of a destroy operation.
#[derive(Debug, Clone)]
pub struct DestroyResult {
    /// Summary of destroyed resources.
    pub stdout: String,
    /// Any warnings or errors.
    pub stderr: String,
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
        let checkpoint_path = self.options.checkpoint_path();
        let checkpoint = self.load_checkpoint(&checkpoint_path).await?;

        let mut summary = String::new();

        match checkpoint {
            None => {
                summary.push_str("No resources to destroy (no checkpoint found).\n");
            }
            Some(cp) => {
                let state = EngineState::from_checkpoint(&cp);
                let resources = state.get_prior_resources().await;

                if resources.is_empty() {
                    summary.push_str("No resources to destroy.\n");
                } else {
                    // Delete in reverse order (children before parents).
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
                    }

                    self.providers.shutdown_all().await;

                    // Save empty checkpoint.
                    let empty = state.empty_checkpoint().await;
                    self.save_checkpoint(&empty, &checkpoint_path).await?;

                    summary.push_str(&format!("\nDestroyed {} resource(s).\n", ordered.len()));
                }
            }
        }

        Ok(DestroyResult {
            stdout: summary,
            stderr: String::new(),
        })
    }

    /// Refresh the stack state.
    ///
    /// Calls [`Provider::read`] for each custom resource to sync state with
    /// the actual cloud provider, then saves the updated checkpoint.
    pub async fn refresh(&self) -> Result<RefreshResult> {
        let checkpoint_path = self.options.checkpoint_path();
        let checkpoint = self.load_checkpoint(&checkpoint_path).await?;

        let mut summary = String::new();

        match checkpoint {
            None => {
                summary.push_str("No checkpoint found, nothing to refresh.\n");
            }
            Some(mut cp) => {
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

                summary = format_refresh_summary(&diffs);
                return Ok(RefreshResult {
                    stdout: summary,
                    stderr: String::new(),
                    diffs,
                });
            }
        }

        Ok(RefreshResult {
            stdout: summary,
            stderr: String::new(),
            diffs: vec![],
        })
    }

    async fn run_program(&self, dry_run: bool) -> Result<UpResult> {
        let checkpoint_path = self.options.checkpoint_path();

        // Load prior state from checkpoint.
        let state = match self.load_checkpoint(&checkpoint_path).await? {
            Some(cp) => EngineState::from_checkpoint(&cp),
            None => EngineState::new(self.options.project.clone(), self.options.stack.clone()),
        };

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
        let engine_svc = EngineServiceImpl::new(state.clone());
        let engine_handle = tokio::spawn(async move {
            Server::builder()
                .add_service(pulumirpc::engine_server::EngineServer::new(engine_svc))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                    engine_listener,
                ))
                .await
        });

        // Build the config JSON.
        let config_json = serde_json::to_string(&self.options.env).unwrap_or_default();

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
            .env("PULUMI_CONFIG", &config_json);

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
                } else {
                    eprintln!("[engine] delete: {urn}");
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

        Ok(UpResult {
            outputs,
            stdout,
            stderr,
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
