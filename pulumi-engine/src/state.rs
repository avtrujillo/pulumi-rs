//! Engine state tracking registered resources and stack outputs.
//!
//! Supports saving/loading checkpoints to disk for persistence across runs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::secrets::{self, SecretsManager, SecretsProviderState};

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
    /// Property names that contain secret values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_properties: Vec<String>,
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
    /// Secrets provider configuration (type + state such as salt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets_provider: Option<SecretsProviderState>,
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
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Save the checkpoint with secret values encrypted.
    ///
    /// Clones the checkpoint, walks all resource inputs/outputs and stack
    /// outputs to encrypt secret-wrapped values, then writes to disk.
    pub async fn save_encrypted<S: SecretsManager>(
        &self,
        path: &Path,
        manager: &S,
    ) -> Result<(), std::io::Error> {
        let mut cp = self.clone();
        cp.secrets_provider = Some(manager.state().await);

        for res in &mut cp.resources {
            secrets::encrypt_secrets(&mut res.inputs, manager)
                .await
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            secrets::encrypt_secrets(&mut res.outputs, manager)
                .await
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        secrets::encrypt_secrets(&mut cp.outputs, manager)
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        cp.save(path)
    }

    /// Load a checkpoint and decrypt secret values.
    ///
    /// After deserializing, walks all resource inputs/outputs and stack
    /// outputs to decrypt any `"v1:..."` ciphertext strings inside secret
    /// wrappers.
    pub async fn load_encrypted<S: SecretsManager>(
        path: &Path,
        manager: &S,
    ) -> Result<Option<Self>, std::io::Error> {
        let mut cp = match Self::load(path)? {
            Some(cp) => cp,
            None => return Ok(None),
        };

        for res in &mut cp.resources {
            secrets::decrypt_secrets(&mut res.inputs, manager)
                .await
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            secrets::decrypt_secrets(&mut res.outputs, manager)
                .await
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        secrets::decrypt_secrets(&mut cp.outputs, manager)
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        Ok(Some(cp))
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
    /// Secrets provider state loaded from the prior checkpoint.
    secrets_provider: Option<SecretsProviderState>,
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
                secrets_provider: None,
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
                secrets_provider: checkpoint.secrets_provider.clone(),
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

    /// Get the secrets provider state from the prior checkpoint, if any.
    pub async fn get_secrets_provider(&self) -> Option<SecretsProviderState> {
        let inner = self.inner.lock().await;
        inner.secrets_provider.clone()
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
            secrets_provider: inner.secrets_provider.clone(),
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
            secrets_provider: inner.secrets_provider.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pulumi_core::serde::SECRET_SIG;

    fn secret_value(v: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            SECRET_SIG: "1b47061264138c4ac30d75fd1eb44270",
            "value": v,
        })
    }

    fn sample_checkpoint() -> Checkpoint {
        Checkpoint {
            version: 1,
            project: "test-project".into(),
            stack: "dev".into(),
            resources: vec![ResourceState {
                urn: "urn:pulumi:dev::test::pkg:mod:Res::myres".into(),
                id: "res-id-1".into(),
                resource_type: "pkg:mod:Res".into(),
                name: "myres".into(),
                custom: true,
                parent: String::new(),
                inputs: serde_json::json!({
                    "name": "my-bucket",
                    "password": secret_value(serde_json::json!("hunter2")),
                }),
                outputs: serde_json::json!({
                    "name": "my-bucket",
                    "arn": "arn:aws:s3:::my-bucket",
                    "connectionString": secret_value(serde_json::json!("postgres://user:pass@host/db")),
                }),
                dependencies: vec![],
                secret_properties: vec!["password".into(), "connectionString".into()],
            }],
            outputs: serde_json::json!({
                "url": "https://example.com",
                "dbPassword": secret_value(serde_json::json!("super-secret")),
            }),
            secrets_provider: None,
        }
    }

    #[test]
    fn checkpoint_roundtrip_no_encryption() {
        let cp = sample_checkpoint();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        cp.save(&path).unwrap();
        let loaded = Checkpoint::load(&path).unwrap().unwrap();

        assert_eq!(loaded.project, cp.project);
        assert_eq!(loaded.stack, cp.stack);
        assert_eq!(loaded.resources.len(), 1);
        assert_eq!(loaded.resources[0].inputs, cp.resources[0].inputs);
        assert_eq!(loaded.resources[0].outputs, cp.resources[0].outputs);
        assert_eq!(loaded.outputs, cp.outputs);
        assert_eq!(
            loaded.resources[0].secret_properties,
            cp.resources[0].secret_properties,
        );
    }

    #[tokio::test]
    async fn checkpoint_encrypted_roundtrip() {
        let mgr =
            crate::secrets::PassphraseSecretsManager::new("test-passphrase").unwrap();
        let cp = sample_checkpoint();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        // Save with encryption.
        cp.save_encrypted(&path, &mgr).await.unwrap();

        // Read the raw JSON to verify secrets are encrypted.
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        // The password in inputs should be a "v1:..." ciphertext string.
        let pw = &raw["resources"][0]["inputs"]["password"]["value"];
        assert!(pw.is_string());
        assert!(pw.as_str().unwrap().starts_with("v1:"));

        // The connectionString in outputs should also be encrypted.
        let cs = &raw["resources"][0]["outputs"]["connectionString"]["value"];
        assert!(cs.is_string());
        assert!(cs.as_str().unwrap().starts_with("v1:"));

        // Stack output secret should be encrypted.
        let db = &raw["outputs"]["dbPassword"]["value"];
        assert!(db.is_string());
        assert!(db.as_str().unwrap().starts_with("v1:"));

        // Non-secret values should be untouched.
        assert_eq!(raw["resources"][0]["inputs"]["name"], "my-bucket");
        assert_eq!(raw["outputs"]["url"], "https://example.com");

        // Secrets provider state should be persisted.
        assert_eq!(raw["secrets_provider"]["type"], "passphrase");
        assert!(raw["secrets_provider"]["state"]["salt"]
            .as_str()
            .unwrap()
            .starts_with("v1:"));

        // Load and decrypt — should match original.
        let loaded = Checkpoint::load_encrypted(&path, &mgr).await.unwrap().unwrap();
        assert_eq!(loaded.resources[0].inputs, cp.resources[0].inputs);
        assert_eq!(loaded.resources[0].outputs, cp.resources[0].outputs);
        assert_eq!(loaded.outputs, cp.outputs);
    }

    #[tokio::test]
    async fn checkpoint_encrypted_wrong_passphrase_fails() {
        let mgr = crate::secrets::PassphraseSecretsManager::new("correct").unwrap();
        let cp = sample_checkpoint();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        cp.save_encrypted(&path, &mgr).await.unwrap();

        // Loading with wrong passphrase should fail.
        let salt = mgr.state().await;
        let salt_str = salt.state["salt"].as_str().unwrap();
        let bad_mgr =
            crate::secrets::PassphraseSecretsManager::from_salt("wrong", salt_str);
        assert!(bad_mgr.is_err());
    }

    #[tokio::test]
    async fn checkpoint_encrypted_restores_from_salt() {
        let mgr = crate::secrets::PassphraseSecretsManager::new("my-pass").unwrap();
        let cp = sample_checkpoint();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        cp.save_encrypted(&path, &mgr).await.unwrap();

        // Restore the manager from the checkpoint's salt.
        let raw_cp = Checkpoint::load(&path).unwrap().unwrap();
        let salt = raw_cp.secrets_provider.as_ref().unwrap().state["salt"]
            .as_str()
            .unwrap();
        let restored =
            crate::secrets::PassphraseSecretsManager::from_salt("my-pass", salt).unwrap();

        // Should be able to decrypt.
        let loaded = Checkpoint::load_encrypted(&path, &restored)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.resources[0].inputs, cp.resources[0].inputs);
        assert_eq!(loaded.outputs, cp.outputs);
    }

    #[tokio::test]
    async fn engine_state_preserves_secrets_provider() {
        let mgr = crate::secrets::PassphraseSecretsManager::new("pass").unwrap();
        let cp = sample_checkpoint();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");

        // Save encrypted checkpoint.
        cp.save_encrypted(&path, &mgr).await.unwrap();

        // Load and feed into EngineState.
        let loaded = Checkpoint::load_encrypted(&path, &mgr).await.unwrap().unwrap();
        let state = EngineState::from_checkpoint(&loaded);

        // The secrets_provider should be preserved.
        let sp = state.get_secrets_provider().await;
        assert!(sp.is_some());
        assert_eq!(sp.unwrap().provider_type, "passphrase");

        // Building a new checkpoint from engine state should carry it forward.
        let new_cp = state.to_checkpoint().await;
        assert!(new_cp.secrets_provider.is_some());
    }
}
