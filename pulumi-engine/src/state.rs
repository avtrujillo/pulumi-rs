//! Engine state tracking registered resources and stack outputs.
//!
//! Supports saving/loading checkpoints to disk for persistence across runs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A registered resource's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceState {
    /// The auto-assigned URN.
    pub urn: String,
    /// The provider-assigned ID (empty for component resources).
    pub id: String,
    /// The resource type token.
    pub resource_type: String,
    /// The resource name.
    pub name: String,
    /// Whether this is a custom (provider-managed) resource.
    pub custom: bool,
    /// The parent URN, if any.
    pub parent: String,
    /// The resource's input properties (what was requested).
    pub inputs: serde_json::Value,
    /// The resource's output properties (what the provider returned).
    pub outputs: serde_json::Value,
    /// URNs this resource depends on.
    pub dependencies: Vec<String>,
}

/// Serializable checkpoint format for persisting state to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Checkpoint format version.
    pub version: i32,
    /// The project name.
    pub project: String,
    /// The stack name.
    pub stack: String,
    /// All resources in registration order.
    pub resources: Vec<ResourceState>,
    /// Stack outputs.
    pub outputs: serde_json::Value,
}

impl Checkpoint {
    /// Load a checkpoint from a JSON file. Returns None if the file doesn't exist.
    pub fn load(path: &Path) -> Result<Option<Self>, std::io::Error> {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let checkpoint: Checkpoint = serde_json::from_str(&contents)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(Some(checkpoint))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Save the checkpoint to a JSON file, creating parent directories if needed.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)
    }
}

/// Shared engine state, protected by a mutex for concurrent access from gRPC handlers.
#[derive(Debug, Clone)]
pub struct EngineState {
    inner: Arc<Mutex<EngineStateInner>>,
}

#[derive(Debug)]
struct EngineStateInner {
    /// The project name.
    project: String,
    /// The stack name.
    stack: String,
    /// The root resource URN.
    root_urn: String,
    /// All registered resources, keyed by URN (current run).
    resources: HashMap<String, ResourceState>,
    /// Resource registration order (for deterministic serialization).
    resource_order: Vec<String>,
    /// Previous run's resources, keyed by URN (loaded from checkpoint).
    prior_resources: HashMap<String, ResourceState>,
    /// Stack outputs (set by RegisterResourceOutputs on the stack resource).
    stack_outputs: serde_json::Value,
    /// Counter for generating unique URNs.
    urn_counter: u64,
}

impl EngineState {
    /// Create a fresh engine state with no prior resources.
    pub fn new(project: String, stack: String) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EngineStateInner {
                project,
                stack,
                root_urn: String::new(),
                resources: HashMap::new(),
                resource_order: Vec::new(),
                prior_resources: HashMap::new(),
                stack_outputs: serde_json::Value::Object(Default::default()),
                urn_counter: 0,
            })),
        }
    }

    /// Create engine state from a previously saved checkpoint.
    pub fn from_checkpoint(checkpoint: &Checkpoint) -> Self {
        let prior_resources: HashMap<String, ResourceState> = checkpoint
            .resources
            .iter()
            .map(|r| (r.urn.clone(), r.clone()))
            .collect();

        Self {
            inner: Arc::new(Mutex::new(EngineStateInner {
                project: checkpoint.project.clone(),
                stack: checkpoint.stack.clone(),
                root_urn: String::new(),
                resources: HashMap::new(),
                resource_order: Vec::new(),
                prior_resources,
                stack_outputs: checkpoint.outputs.clone(),
                urn_counter: 0,
            })),
        }
    }

    /// Generate a URN for a resource.
    pub async fn make_urn(&self, resource_type: &str, name: &str, parent: &str) -> String {
        let mut inner = self.inner.lock().await;
        inner.urn_counter += 1;

        if parent.is_empty() {
            format!(
                "urn:pulumi:{}::{}::{}::{}",
                inner.stack, inner.project, resource_type, name
            )
        } else {
            let parent_type = parent.split("::").nth(2).unwrap_or("pulumi:pulumi:Stack");
            format!(
                "urn:pulumi:{}::{}::{}${}::{}",
                inner.stack, inner.project, parent_type, resource_type, name
            )
        }
    }

    /// Look up a resource from the prior (checkpoint) state by URN.
    pub async fn get_prior_resource(&self, urn: &str) -> Option<ResourceState> {
        let inner = self.inner.lock().await;
        inner.prior_resources.get(urn).cloned()
    }

    /// Register a resource in the current run and return its URN.
    pub async fn register_resource(&self, state: ResourceState) -> String {
        let urn = state.urn.clone();
        let mut inner = self.inner.lock().await;
        if !inner.resources.contains_key(&urn) {
            inner.resource_order.push(urn.clone());
        }
        inner.resources.insert(urn.clone(), state);
        urn
    }

    /// Set the root resource URN.
    pub async fn set_root_urn(&self, urn: String) {
        let mut inner = self.inner.lock().await;
        inner.root_urn = urn;
    }

    /// Get the root resource URN.
    pub async fn get_root_urn(&self) -> String {
        let inner = self.inner.lock().await;
        inner.root_urn.clone()
    }

    /// Set the stack outputs.
    pub async fn set_stack_outputs(&self, outputs: serde_json::Value) {
        let mut inner = self.inner.lock().await;
        inner.stack_outputs = outputs;
    }

    /// Get the stack outputs.
    pub async fn get_stack_outputs(&self) -> serde_json::Value {
        let inner = self.inner.lock().await;
        inner.stack_outputs.clone()
    }

    /// Get all registered resources from the current run.
    pub async fn get_resources(&self) -> HashMap<String, ResourceState> {
        let inner = self.inner.lock().await;
        inner.resources.clone()
    }

    /// Get URNs from prior state that were NOT registered in the current run.
    /// These are resources that should be deleted (in reverse order).
    pub async fn get_deleted_urns(&self) -> Vec<String> {
        let inner = self.inner.lock().await;
        let mut deleted: Vec<String> = inner
            .prior_resources
            .keys()
            .filter(|urn| !inner.resources.contains_key(*urn))
            .cloned()
            .collect();
        // Reverse so children are deleted before parents.
        deleted.reverse();
        deleted
    }

    /// Get all prior resources (from the checkpoint).
    pub async fn get_prior_resources(&self) -> HashMap<String, ResourceState> {
        let inner = self.inner.lock().await;
        inner.prior_resources.clone()
    }

    /// Build a checkpoint from the current state.
    pub async fn to_checkpoint(&self) -> Checkpoint {
        let inner = self.inner.lock().await;
        let resources: Vec<ResourceState> = inner
            .resource_order
            .iter()
            .filter_map(|urn| inner.resources.get(urn).cloned())
            .collect();

        Checkpoint {
            version: 1,
            project: inner.project.clone(),
            stack: inner.stack.clone(),
            resources,
            outputs: inner.stack_outputs.clone(),
        }
    }

    /// Build an empty checkpoint (for after destroy).
    pub async fn empty_checkpoint(&self) -> Checkpoint {
        let inner = self.inner.lock().await;
        Checkpoint {
            version: 1,
            project: inner.project.clone(),
            stack: inner.stack.clone(),
            resources: Vec::new(),
            outputs: serde_json::Value::Object(Default::default()),
        }
    }
}
