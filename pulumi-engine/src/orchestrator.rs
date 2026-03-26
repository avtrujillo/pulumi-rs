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
use crate::state::EngineState;

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
        }
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

/// The Rust-native Pulumi engine.
pub struct PulumiEngine {
    options: EngineOptions,
}

impl PulumiEngine {
    pub fn new(options: EngineOptions) -> Self {
        Self { options }
    }

    /// Run the Pulumi program (equivalent to `pulumi up`).
    pub async fn up(&self) -> Result<UpResult> {
        self.run(false).await
    }

    /// Run the Pulumi program in preview/dry-run mode.
    pub async fn preview(&self) -> Result<UpResult> {
        self.run(true).await
    }

    async fn run(&self, dry_run: bool) -> Result<UpResult> {
        let state = EngineState::new(self.options.project.clone(), self.options.stack.clone());

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

        // Build the config JSON from any env vars matching the project prefix.
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

        // Pass through any extra env vars.
        for (k, v) in &self.options.env {
            cmd.env(k, v);
        }

        let output = cmd.output().await.map_err(Error::Spawn)?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        // Abort the gRPC servers now that the program is done.
        monitor_handle.abort();
        engine_handle.abort();

        if !output.status.success() {
            return Err(Error::ProgramFailed {
                code: output.status.code().unwrap_or(-1),
                stdout,
                stderr,
            });
        }

        let outputs = state.get_stack_outputs().await;

        Ok(UpResult {
            outputs,
            stdout,
            stderr,
        })
    }
}
