//! Native engine backend for stack lifecycle operations.
//!
//! When the `native-engine` feature is enabled, this module provides
//! [`NativeStack`] — an alternative to [`crate::Stack`] that uses the
//! Rust-native Pulumi engine instead of shelling out to the `pulumi` CLI.

use crate::config::ConfigValue;
use crate::error::{Error, Result};
use crate::stack::{OutputValue, UpResult};
use crate::workspace::LocalWorkspace;
use pulumi_engine::secrets::PassphraseSecretsManager;
use pulumi_engine::{Checkpoint, EngineOptions, PulumiEngine};
use std::collections::HashMap;
use std::path::PathBuf;

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

    // ---------------------------------------------------------------------------
    // Config persistence
    // ---------------------------------------------------------------------------

    /// Path to the native config file: `<work_dir>/.pulumi-rs/<stack>.config.json`.
    fn config_path(&self) -> PathBuf {
        self.workspace
            .work_dir()
            .join(".pulumi-rs")
            .join(format!("{}.config.json", self.name))
    }

    /// Load all config values from disk. Returns an empty map if the file doesn't exist.
    fn load_config_map(&self) -> Result<HashMap<String, ConfigValue>> {
        let path = self.config_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                serde_json::from_str(&contents).map_err(|e| Error::Json(e))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Save all config values to disk, creating parent directories as needed.
    fn save_config_map(&self, config: &HashMap<String, ConfigValue>) -> Result<()> {
        let path = self.config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::Io)?;
        }
        let contents = serde_json::to_string_pretty(config)?;
        std::fs::write(&path, contents).map_err(Error::Io)?;
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // EngineOptions construction
    // ---------------------------------------------------------------------------

    fn engine_options(&self, dry_run: bool) -> EngineOptions {
        let project = self
            .workspace
            .work_dir()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();

        // Load config from disk and split into values + secret key list.
        let config_map = self.load_config_map().unwrap_or_default();
        let mut config = HashMap::new();
        let mut config_secret_keys = Vec::new();
        for (k, v) in &config_map {
            config.insert(k.clone(), v.value.clone());
            if v.secret {
                config_secret_keys.push(k.clone());
            }
        }

        let secrets_manager = std::env::var("PULUMI_CONFIG_PASSPHRASE")
            .ok()
            .filter(|p| !p.is_empty())
            .and_then(|passphrase| {
                let checkpoint_path = self
                    .workspace
                    .work_dir()
                    .join(".pulumi-rs")
                    .join(format!("{}.json", self.name));
                if let Ok(Some(cp)) = Checkpoint::load(&checkpoint_path)
                    && let Some(sp) = &cp.secrets_provider
                    && let Some(salt) = sp.state.get("salt").and_then(|s| s.as_str())
                {
                    return PassphraseSecretsManager::from_salt(&passphrase, salt).ok();
                }
                PassphraseSecretsManager::new(&passphrase).ok()
            });

        EngineOptions {
            project,
            stack: self.name.clone(),
            work_dir: self.workspace.work_dir().to_path_buf(),
            program: self.program.clone(),
            dry_run,
            config,
            config_secret_keys,
            env: HashMap::new(),
            checkpoint_path: None,
            secrets_manager,
        }
    }

    // ---------------------------------------------------------------------------
    // Lifecycle operations
    // ---------------------------------------------------------------------------

    /// Runs the Pulumi program to create or update resources using the native engine.
    pub async fn up(&self) -> Result<UpResult> {
        let opts = self.engine_options(false);
        let engine = PulumiEngine::new(opts);
        let result = engine
            .up()
            .await
            .map_err(|e| Error::Custom(e.to_string()))?;

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

    /// Destroys all resources in the stack using the native engine.
    pub async fn destroy(&self) -> Result<crate::stack::DestroyResult> {
        let opts = self.engine_options(false);
        let engine = PulumiEngine::new(opts);
        let result = engine
            .destroy()
            .await
            .map_err(|e| Error::Custom(e.to_string()))?;

        Ok(crate::stack::DestroyResult {
            stdout: result.stdout,
            stderr: result.stderr,
            events: Vec::new(),
        })
    }

    /// Refreshes the stack state using the native engine.
    pub async fn refresh(&self) -> Result<crate::stack::RefreshResult> {
        let opts = self.engine_options(false);
        let engine = PulumiEngine::new(opts);
        let result = engine
            .refresh()
            .await
            .map_err(|e| Error::Custom(e.to_string()))?;

        Ok(crate::stack::RefreshResult {
            stdout: result.stdout,
            stderr: result.stderr,
            events: Vec::new(),
        })
    }

    // ---------------------------------------------------------------------------
    // Stack outputs
    // ---------------------------------------------------------------------------

    /// Returns the current stack outputs by reading the checkpoint file.
    pub fn outputs(&self) -> Result<HashMap<String, OutputValue>> {
        let checkpoint_path = self
            .workspace
            .work_dir()
            .join(".pulumi-rs")
            .join(format!("{}.json", self.name));

        match Checkpoint::load(&checkpoint_path).map_err(Error::Io)? {
            None => Ok(HashMap::new()),
            Some(cp) => Ok(convert_outputs(&cp.outputs)),
        }
    }

    // ---------------------------------------------------------------------------
    // Config operations
    // ---------------------------------------------------------------------------

    /// Gets a configuration value by key.
    pub fn get_config(&self, key: &str) -> Result<ConfigValue> {
        let config = self.load_config_map()?;
        config
            .get(key)
            .cloned()
            .ok_or_else(|| Error::Custom(format!("config key not found: {key}")))
    }

    /// Sets a configuration value.
    pub fn set_config(&self, key: &str, value: ConfigValue) -> Result<()> {
        let mut config = self.load_config_map()?;
        config.insert(key.to_string(), value);
        self.save_config_map(&config)
    }

    /// Returns all configuration values for this stack.
    pub fn get_all_config(&self) -> Result<HashMap<String, ConfigValue>> {
        self.load_config_map()
    }

    /// Removes a configuration value.
    pub fn remove_config(&self, key: &str) -> Result<()> {
        let mut config = self.load_config_map()?;
        config.remove(key);
        self.save_config_map(&config)
    }
}

/// Convert serde_json::Value outputs into the HashMap<String, OutputValue> format.
///
/// Detects secret-wrapped values (containing the Pulumi magic signature key)
/// and sets the `secret` flag accordingly, unwrapping the inner value.
fn convert_outputs(value: &serde_json::Value) -> HashMap<String, OutputValue> {
    use pulumi_engine::secrets::is_json_secret;

    let mut map = HashMap::new();
    if let serde_json::Value::Object(obj) = value {
        for (k, v) in obj {
            let (output_value, secret) = if is_json_secret(v) {
                let inner = v
                    .as_object()
                    .and_then(|m| m.get("value"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                (inner, true)
            } else {
                (v.clone(), false)
            };
            map.insert(k.clone(), OutputValue { value: output_value, secret });
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_stack(tmp: &TempDir) -> NativeStack {
        let ws = LocalWorkspace::new(tmp.path());
        NativeStack::new(ws, "dev".into(), vec![])
    }

    #[test]
    fn config_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);

        stack.set_config("myproject:region", ConfigValue::plaintext("us-east-1")).unwrap();
        stack.set_config("myproject:token", ConfigValue::secret("s3cr3t")).unwrap();

        let all = stack.get_all_config().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all["myproject:region"].value, "us-east-1");
        assert!(!all["myproject:region"].secret);
        assert_eq!(all["myproject:token"].value, "s3cr3t");
        assert!(all["myproject:token"].secret);
    }

    #[test]
    fn get_config_individual() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);
        stack.set_config("proj:key", ConfigValue::plaintext("val")).unwrap();
        let cv = stack.get_config("proj:key").unwrap();
        assert_eq!(cv.value, "val");
    }

    #[test]
    fn get_config_missing_returns_error() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);
        assert!(stack.get_config("missing:key").is_err());
    }

    #[test]
    fn remove_config() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);
        stack.set_config("proj:key", ConfigValue::plaintext("val")).unwrap();
        stack.remove_config("proj:key").unwrap();
        assert!(stack.get_config("proj:key").is_err());
    }

    #[test]
    fn get_all_config_empty_when_no_file() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);
        let all = stack.get_all_config().unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn outputs_empty_when_no_checkpoint() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);
        let outputs = stack.outputs().unwrap();
        assert!(outputs.is_empty());
    }

    #[test]
    fn engine_options_populates_config() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);
        stack.set_config("proj:region", ConfigValue::plaintext("eu-west-1")).unwrap();
        stack.set_config("proj:token", ConfigValue::secret("tok")).unwrap();

        let opts = stack.engine_options(false);
        assert_eq!(opts.config.get("proj:region").map(String::as_str), Some("eu-west-1"));
        assert_eq!(opts.config.get("proj:token").map(String::as_str), Some("tok"));
        assert!(opts.config_secret_keys.contains(&"proj:token".to_string()));
        assert!(!opts.config_secret_keys.contains(&"proj:region".to_string()));
    }
}
