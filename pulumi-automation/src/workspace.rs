//! Workspace manages the interaction with a Pulumi project on disk.

use crate::cmd::run_pulumi_cmd;
use crate::config::ConfigValue;
use crate::error::{Error, Result};
use crate::stack::Stack;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Information about a stack as returned by `pulumi stack ls`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackSummary {
    /// The fully qualified stack name.
    pub name: String,
    /// Whether this is the currently selected stack.
    #[serde(default)]
    pub current: bool,
    /// The last update time, if any.
    pub last_update: Option<String>,
    /// The number of resources in this stack.
    #[serde(default)]
    pub resource_count: Option<i64>,
}

/// A local workspace backed by a Pulumi project directory on disk.
///
/// This is the main entry point for the Automation API. It corresponds to a
/// `Pulumi.yaml` project and lets you create, select, and manage stacks.
#[derive(Debug, Clone)]
pub struct LocalWorkspace {
    work_dir: PathBuf,
    env_vars: HashMap<String, String>,
}

impl LocalWorkspace {
    /// Creates a new workspace rooted at the given directory.
    ///
    /// The directory should contain a `Pulumi.yaml` project file.
    pub fn new(work_dir: impl Into<PathBuf>) -> Self {
        Self {
            work_dir: work_dir.into(),
            env_vars: HashMap::new(),
        }
    }

    /// Sets an environment variable that will be passed to all CLI invocations.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Returns the working directory.
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    fn env_pairs(&self) -> Vec<(&str, &str)> {
        self.env_vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    /// Creates a new stack and returns a handle to it.
    pub async fn create_stack(&self, name: &str) -> Result<Stack> {
        let result = run_pulumi_cmd(
            &self.work_dir,
            &["stack", "init", name],
            &self.env_pairs(),
        )
        .await;

        match result {
            Ok(_) => Ok(Stack::new(self.clone(), name.to_string())),
            Err(Error::CommandFailed { stderr, .. }) if stderr.contains("already exists") => {
                Err(Error::StackAlreadyExists(name.to_string()))
            }
            Err(e) => Err(e),
        }
    }

    /// Selects an existing stack and returns a handle to it.
    pub async fn select_stack(&self, name: &str) -> Result<Stack> {
        let result = run_pulumi_cmd(
            &self.work_dir,
            &["stack", "select", name],
            &self.env_pairs(),
        )
        .await;

        match result {
            Ok(_) => Ok(Stack::new(self.clone(), name.to_string())),
            Err(Error::CommandFailed { stderr, .. })
                if stderr.contains("no stack named") || stderr.contains("not found") =>
            {
                Err(Error::StackNotFound(name.to_string()))
            }
            Err(e) => Err(e),
        }
    }

    /// Creates a stack if it doesn't exist, otherwise selects it.
    pub async fn create_or_select_stack(&self, name: &str) -> Result<Stack> {
        match self.create_stack(name).await {
            Ok(stack) => Ok(stack),
            Err(Error::StackAlreadyExists(_)) => self.select_stack(name).await,
            Err(e) => Err(e),
        }
    }

    /// Removes a stack.
    ///
    /// If `force` is true, the stack will be removed even if it still contains
    /// resources.
    pub async fn remove_stack(&self, name: &str, force: bool) -> Result<()> {
        let mut args = vec!["stack", "rm", name, "--yes"];
        if force {
            args.push("--force");
        }
        run_pulumi_cmd(&self.work_dir, &args, &self.env_pairs()).await?;
        Ok(())
    }

    /// Lists all stacks in this workspace.
    pub async fn list_stacks(&self) -> Result<Vec<StackSummary>> {
        let output = run_pulumi_cmd(
            &self.work_dir,
            &["stack", "ls", "--json"],
            &self.env_pairs(),
        )
        .await?;
        let stacks: Vec<StackSummary> = serde_json::from_str(&output.stdout)?;
        Ok(stacks)
    }

    /// Gets a configuration value for the given stack.
    pub async fn get_config(&self, stack: &str, key: &str) -> Result<ConfigValue> {
        let output = run_pulumi_cmd(
            &self.work_dir,
            &["config", "get", key, "--stack", stack, "--json"],
            &self.env_pairs(),
        )
        .await?;
        let value: ConfigValue = serde_json::from_str(&output.stdout)?;
        Ok(value)
    }

    /// Sets a configuration value for the given stack.
    pub async fn set_config(&self, stack: &str, key: &str, value: &ConfigValue) -> Result<()> {
        let mut args = vec!["config", "set", key, &value.value, "--stack", stack];
        if value.secret {
            args.push("--secret");
        }
        run_pulumi_cmd(&self.work_dir, &args, &self.env_pairs()).await?;
        Ok(())
    }

    /// Gets all configuration values for the given stack.
    pub async fn get_all_config(
        &self,
        stack: &str,
    ) -> Result<HashMap<String, ConfigValue>> {
        let output = run_pulumi_cmd(
            &self.work_dir,
            &["config", "--stack", stack, "--json"],
            &self.env_pairs(),
        )
        .await?;
        let config: HashMap<String, ConfigValue> = serde_json::from_str(&output.stdout)?;
        Ok(config)
    }

    /// Removes a configuration value for the given stack.
    pub async fn remove_config(&self, stack: &str, key: &str) -> Result<()> {
        run_pulumi_cmd(
            &self.work_dir,
            &["config", "rm", key, "--stack", stack],
            &self.env_pairs(),
        )
        .await?;
        Ok(())
    }
}
