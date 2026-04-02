//! Stack represents a deployed Pulumi stack and exposes lifecycle operations.

use crate::cmd::run_pulumi_cmd;
use crate::config::ConfigValue;
use crate::error::Result;
use crate::event::EngineEvent;
use crate::workspace::LocalWorkspace;
use serde::Deserialize;
use std::collections::HashMap;

/// The result of an `up` operation.
#[derive(Debug, Clone)]
pub struct UpResult {
    /// The stdout from the CLI.
    pub stdout: String,
    /// The stderr from the CLI.
    pub stderr: String,
    /// Stack outputs after the update.
    pub outputs: HashMap<String, OutputValue>,
    /// Structured engine events, if an event log was used.
    pub events: Vec<EngineEvent>,
}

/// The result of a `preview` operation.
#[derive(Debug, Clone)]
pub struct PreviewResult {
    /// The stdout from the CLI.
    pub stdout: String,
    /// The stderr from the CLI.
    pub stderr: String,
    /// Structured engine events, if an event log was used.
    pub events: Vec<EngineEvent>,
}

/// The result of a `destroy` operation.
#[derive(Debug, Clone)]
pub struct DestroyResult {
    /// The stdout from the CLI.
    pub stdout: String,
    /// The stderr from the CLI.
    pub stderr: String,
    /// Structured engine events, if an event log was used.
    pub events: Vec<EngineEvent>,
}

/// The result of a `refresh` operation.
#[derive(Debug, Clone)]
pub struct RefreshResult {
    /// The stdout from the CLI.
    pub stdout: String,
    /// The stderr from the CLI.
    pub stderr: String,
    /// Structured engine events, if an event log was used.
    pub events: Vec<EngineEvent>,
}

/// A stack output value.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct OutputValue {
    /// The output value.
    pub value: serde_json::Value,
    /// Whether this output is a secret.
    #[serde(default)]
    pub secret: bool,
}

/// A handle to a specific Pulumi stack that exposes lifecycle operations.
///
/// Created via [`LocalWorkspace::create_stack`], [`LocalWorkspace::select_stack`],
/// or [`LocalWorkspace::create_or_select_stack`].
#[derive(Debug, Clone)]
pub struct Stack {
    workspace: LocalWorkspace,
    name: String,
}

impl Stack {
    pub(crate) fn new(workspace: LocalWorkspace, name: String) -> Self {
        Self { workspace, name }
    }

    /// Returns the stack name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns a reference to the workspace.
    pub fn workspace(&self) -> &LocalWorkspace {
        &self.workspace
    }

    /// Runs `pulumi up` to create or update resources.
    pub async fn up(&self) -> Result<UpResult> {
        let output = run_pulumi_cmd(
            self.workspace.work_dir(),
            &["up", "--stack", &self.name, "--yes", "--skip-preview"],
            &[],
        )
        .await?;

        let outputs = self.outputs().await.unwrap_or_default();

        Ok(UpResult {
            stdout: output.stdout,
            stderr: output.stderr,
            outputs,
            events: Vec::new(),
        })
    }

    /// Runs `pulumi preview` to show pending changes.
    pub async fn preview(&self) -> Result<PreviewResult> {
        let output = run_pulumi_cmd(
            self.workspace.work_dir(),
            &["preview", "--stack", &self.name],
            &[],
        )
        .await?;

        Ok(PreviewResult {
            stdout: output.stdout,
            stderr: output.stderr,
            events: Vec::new(),
        })
    }

    /// Runs `pulumi refresh` to reconcile state with the actual cloud resources.
    pub async fn refresh(&self) -> Result<RefreshResult> {
        let output = run_pulumi_cmd(
            self.workspace.work_dir(),
            &["refresh", "--stack", &self.name, "--yes", "--skip-preview"],
            &[],
        )
        .await?;

        Ok(RefreshResult {
            stdout: output.stdout,
            stderr: output.stderr,
            events: Vec::new(),
        })
    }

    /// Runs `pulumi destroy` to tear down all resources in this stack.
    pub async fn destroy(&self) -> Result<DestroyResult> {
        let output = run_pulumi_cmd(
            self.workspace.work_dir(),
            &["destroy", "--stack", &self.name, "--yes", "--skip-preview"],
            &[],
        )
        .await?;

        Ok(DestroyResult {
            stdout: output.stdout,
            stderr: output.stderr,
            events: Vec::new(),
        })
    }

    /// Gets the current stack outputs.
    pub async fn outputs(&self) -> Result<HashMap<String, OutputValue>> {
        let output = run_pulumi_cmd(
            self.workspace.work_dir(),
            &["stack", "output", "--stack", &self.name, "--json"],
            &[],
        )
        .await?;

        let outputs: HashMap<String, OutputValue> = serde_json::from_str(&output.stdout)?;
        Ok(outputs)
    }

    /// Gets a configuration value.
    pub async fn get_config(&self, key: &str) -> Result<ConfigValue> {
        self.workspace.get_config(&self.name, key).await
    }

    /// Sets a configuration value.
    pub async fn set_config(&self, key: &str, value: &ConfigValue) -> Result<()> {
        self.workspace.set_config(&self.name, key, value).await
    }

    /// Gets all configuration values.
    pub async fn get_all_config(&self) -> Result<HashMap<String, ConfigValue>> {
        self.workspace.get_all_config(&self.name).await
    }

    /// Removes a configuration value.
    pub async fn remove_config(&self, key: &str) -> Result<()> {
        self.workspace.remove_config(&self.name, key).await
    }

    /// Exports the stack's deployment state as JSON.
    pub async fn export_state(&self) -> Result<serde_json::Value> {
        let output = run_pulumi_cmd(
            self.workspace.work_dir(),
            &["stack", "export", "--stack", &self.name],
            &[],
        )
        .await?;

        let state: serde_json::Value = serde_json::from_str(&output.stdout)?;
        Ok(state)
    }

    /// Imports a previously exported deployment state.
    pub async fn import_state(&self, state: &serde_json::Value) -> Result<()> {
        let json = serde_json::to_string(state)?;
        // Use `echo | pulumi stack import` via shell
        run_pulumi_cmd(
            self.workspace.work_dir(),
            &[
                "stack",
                "import",
                "--stack",
                &self.name,
                "--file",
                "/dev/stdin",
            ],
            &[],
        )
        .await
        .map_err(|_| {
            crate::error::Error::Custom(format!(
                "stack import failed; state payload was {} bytes",
                json.len()
            ))
        })?;
        Ok(())
    }
}
