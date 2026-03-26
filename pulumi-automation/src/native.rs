//! Native engine backend for stack lifecycle operations.
//!
//! When the `native-engine` feature is enabled, this module provides
//! [`NativeStack`] — an alternative to [`crate::Stack`] that uses the
//! Rust-native Pulumi engine instead of shelling out to the `pulumi` CLI.

use crate::error::{Error, Result};
use crate::stack::{OutputValue, UpResult};
use crate::workspace::LocalWorkspace;
use pulumi_engine::{EngineOptions, PulumiEngine};
use std::collections::HashMap;

/// A stack handle that uses the native Rust engine instead of the Pulumi CLI.
///
/// Created via [`LocalWorkspace::native_stack`] (requires the `native-engine` feature).
#[derive(Debug, Clone)]
pub struct NativeStack {
    workspace: LocalWorkspace,
    name: String,
    /// The program command to run (e.g. `["./target/release/my-program"]`).
    program: Vec<String>,
}

impl NativeStack {
    pub(crate) fn new(workspace: LocalWorkspace, name: String, program: Vec<String>) -> Self {
        Self {
            workspace,
            name,
            program,
        }
    }

    /// Returns the stack name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns a reference to the workspace.
    pub fn workspace(&self) -> &LocalWorkspace {
        &self.workspace
    }

    fn engine_options(&self, dry_run: bool) -> EngineOptions {
        // Read the project name from the workspace directory name as a default.
        let project = self
            .workspace
            .work_dir()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();

        EngineOptions {
            project,
            stack: self.name.clone(),
            work_dir: self.workspace.work_dir().to_path_buf(),
            program: self.program.clone(),
            dry_run,
            env: HashMap::new(),
        }
    }

    /// Runs the Pulumi program to create or update resources using the native engine.
    pub async fn up(&self) -> Result<UpResult> {
        let opts = self.engine_options(false);
        let engine = PulumiEngine::new(opts);
        let result = engine.up().await.map_err(|e| Error::Custom(e.to_string()))?;

        let outputs = convert_outputs(&result.outputs);

        Ok(UpResult {
            stdout: result.stdout,
            stderr: result.stderr,
            outputs,
            events: Vec::new(),
        })
    }

    /// Runs the Pulumi program in preview mode using the native engine.
    pub async fn preview(&self) -> Result<crate::stack::PreviewResult> {
        let opts = self.engine_options(true);
        let engine = PulumiEngine::new(opts);
        let result = engine
            .preview()
            .await
            .map_err(|e| Error::Custom(e.to_string()))?;

        Ok(crate::stack::PreviewResult {
            stdout: result.stdout,
            stderr: result.stderr,
            events: Vec::new(),
        })
    }
}

/// Convert serde_json::Value outputs into the HashMap<String, OutputValue> format.
fn convert_outputs(value: &serde_json::Value) -> HashMap<String, OutputValue> {
    let mut map = HashMap::new();
    if let serde_json::Value::Object(obj) = value {
        for (k, v) in obj {
            map.insert(
                k.clone(),
                OutputValue {
                    value: v.clone(),
                    secret: false,
                },
            );
        }
    }
    map
}
