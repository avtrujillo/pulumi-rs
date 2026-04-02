//! Engine orchestrator — starts gRPC servers and runs the user's Pulumi program.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use tokio::process::Command;
use tonic::transport::Server;

use crate::engine_service::EngineServiceImpl;
use crate::error::{Error, Result};
use crate::monitor_service::ResourceMonitorImpl;
use crate::pulumirpc;
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
}

/// The Rust-native Pulumi engine.
pub struct PulumiEngine {
    options: EngineOptions,
}

impl PulumiEngine {
    pub fn new(options: EngineOptions) -> Self {
        Self { options }
    }

    /// Run the Pulumi program (equivalent to `pulumi up`).
    ///
    /// Loads prior state from the checkpoint, runs the program to register
    /// resources (diffing against prior state), then saves the new checkpoint.
    pub async fn up(&self) -> Result<UpResult> {
        self.run_program(false).await
    }

    /// Run the Pulumi program in preview/dry-run mode.
    ///
    /// Loads prior state and runs the program, but does not persist the
    /// resulting checkpoint.
    pub async fn preview(&self) -> Result<UpResult> {
        self.run_program(true).await
    }

    /// Destroy all resources in the stack.
    ///
    /// Loads the checkpoint, logs each resource that would be deleted (in
    /// reverse dependency order), then clears the checkpoint. Provider delete
    /// calls are not yet implemented — this only updates state.
    pub async fn destroy(&self) -> Result<DestroyResult> {
        let checkpoint_path = self.options.checkpoint_path();
        let checkpoint = Checkpoint::load(&checkpoint_path)
            .map_err(|e| Error::Custom(format!("failed to load checkpoint: {e}")))?;

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
                    let mut urns: Vec<_> = cp.resources.iter().map(|r| &r.urn).collect();
                    urns.reverse();

                    for urn in &urns {
                        if let Some(res) = resources.get(*urn) {
                            // TODO: Call provider.Delete for custom resources.
                            let action = if res.custom { "delete" } else { "remove" };
                            eprintln!("[engine] {action}: {} ({})", urn, res.resource_type);
                            summary.push_str(&format!(
                                "- {action} {} ({})\n",
                                res.name, res.resource_type
                            ));
                        }
                    }

                    // Save empty checkpoint.
                    let empty = state.empty_checkpoint().await;
                    empty
                        .save(&checkpoint_path)
                        .map_err(|e| Error::Custom(format!("failed to save checkpoint: {e}")))?;

                    summary.push_str(&format!("\nDestroyed {} resource(s).\n", urns.len()));
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
    /// Loads the checkpoint and re-reads each resource's current state.
    /// Provider read calls are not yet implemented — this currently just
    /// validates and re-saves the checkpoint.
    pub async fn refresh(&self) -> Result<RefreshResult> {
        let checkpoint_path = self.options.checkpoint_path();
        let checkpoint = Checkpoint::load(&checkpoint_path)
            .map_err(|e| Error::Custom(format!("failed to load checkpoint: {e}")))?;

        let mut summary = String::new();

        match checkpoint {
            None => {
                summary.push_str("No checkpoint found, nothing to refresh.\n");
            }
            Some(cp) => {
                let resource_count = cp.resources.len();
                // TODO: For each resource, call provider.Read to get current state.
                // For now, just re-save the checkpoint as-is.
                for res in &cp.resources {
                    eprintln!("[engine] refresh: {} ({})", res.urn, res.resource_type);
                }

                cp.save(&checkpoint_path)
                    .map_err(|e| Error::Custom(format!("failed to save checkpoint: {e}")))?;

                summary.push_str(&format!("Refreshed {resource_count} resource(s).\n"));
            }
        }

        Ok(RefreshResult {
            stdout: summary,
            stderr: String::new(),
        })
    }

    async fn run_program(&self, dry_run: bool) -> Result<UpResult> {
        let checkpoint_path = self.options.checkpoint_path();

        // Load prior state from checkpoint.
        let state = match Checkpoint::load(&checkpoint_path) {
            Ok(Some(cp)) => EngineState::from_checkpoint(&cp),
            Ok(None) => EngineState::new(self.options.project.clone(), self.options.stack.clone()),
            Err(e) => {
                return Err(Error::Custom(format!("failed to load checkpoint: {e}")));
            }
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

        // Start the ResourceMonitor server.
        let monitor_svc = ResourceMonitorImpl::new(state.clone(), dry_run || self.options.dry_run);
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

        // Abort the gRPC servers.
        monitor_handle.abort();
        engine_handle.abort();

        if !output.status.success() {
            return Err(Error::ProgramFailed {
                code: output.status.code().unwrap_or(-1),
                stdout,
                stderr,
            });
        }

        // Log resources deleted from prior state (present before, not registered now).
        let deleted_urns = state.get_deleted_urns().await;
        for urn in &deleted_urns {
            eprintln!("[engine] delete: {urn}");
        }

        let outputs = state.get_stack_outputs().await;

        // Save checkpoint (unless dry run).
        if !dry_run && !self.options.dry_run {
            let checkpoint = state.to_checkpoint().await;
            checkpoint
                .save(&checkpoint_path)
                .map_err(|e| Error::Custom(format!("failed to save checkpoint: {e}")))?;
        }

        Ok(UpResult {
            outputs,
            stdout,
            stderr,
        })
    }
}
