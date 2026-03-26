//! In-memory engine state tracking registered resources and stack outputs.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A registered resource's state.
#[derive(Debug, Clone)]
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
    /// The resource's output properties.
    pub outputs: serde_json::Value,
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
    /// All registered resources, keyed by URN.
    resources: HashMap<String, ResourceState>,
    /// Stack outputs (set by RegisterResourceOutputs on the stack resource).
    stack_outputs: serde_json::Value,
    /// Counter for generating unique URNs.
    urn_counter: u64,
}

impl EngineState {
    pub fn new(project: String, stack: String) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EngineStateInner {
                project,
                stack,
                root_urn: String::new(),
                resources: HashMap::new(),
                stack_outputs: serde_json::Value::Object(Default::default()),
                urn_counter: 0,
            })),
        }
    }

    /// Generate a URN for a resource.
    pub async fn make_urn(&self, resource_type: &str, name: &str, parent: &str) -> String {
        let mut inner = self.inner.lock().await;
        inner.urn_counter += 1;

        // URN format: urn:pulumi:<stack>::<project>::<type>::<name>
        // If parent is set, the type is nested under the parent's type.
        if parent.is_empty() {
            format!(
                "urn:pulumi:{}::{}::{}::{}",
                inner.stack, inner.project, resource_type, name
            )
        } else {
            // Extract parent type from parent URN for nesting
            let parent_type = parent
                .split("::")
                .nth(2)
                .unwrap_or("pulumi:pulumi:Stack");
            format!(
                "urn:pulumi:{}::{}::{}${}::{}",
                inner.stack, inner.project, parent_type, resource_type, name
            )
        }
    }

    /// Register a resource and return its URN.
    pub async fn register_resource(&self, state: ResourceState) -> String {
        let urn = state.urn.clone();
        let mut inner = self.inner.lock().await;
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

    /// Get all registered resources.
    pub async fn get_resources(&self) -> HashMap<String, ResourceState> {
        let inner = self.inner.lock().await;
        inner.resources.clone()
    }
}
