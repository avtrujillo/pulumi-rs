//! Native engine backend for stack lifecycle operations.
//!
//! When the `native-engine` feature is enabled, this module provides
//! [`NativeStack`] — an alternative to [`crate::Stack`] that uses the
//! Rust-native Pulumi engine instead of shelling out to the `pulumi` CLI.

use crate::config::ConfigValue;
use crate::error::{Error, Result};
use crate::event::{
    DiagnosticEvent, EngineEvent, PreludeEvent, ResourcePreEvent, StepEventMetadata,
    StepEventStateMetadata, SummaryEvent,
};
use crate::stack::{OutputValue, UpResult};
use crate::workspace::LocalWorkspace;
use pulumi_engine::events::EngineEvent as NativeEvent;
use pulumi_engine::secrets::PassphraseSecretsManager;
use pulumi_engine::{Checkpoint, EngineOptions, PulumiEngine};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// On-disk stack settings file, matching the Pulumi CLI's `Pulumi.<stack>.yaml`.
///
/// Plain config values are stored as YAML scalars; secret values are stored
/// as `{ secure: "v1:NONCE:CT" }` ciphertext objects. The `encryptionsalt`
/// holds the passphrase provider's salt state. Files written here can be
/// read by the real Pulumi CLI and vice versa.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct StackFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    encryptionsalt: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    config: BTreeMap<String, StackFileValue>,
}

/// A single config entry in `Pulumi.<stack>.yaml`: either an encrypted
/// secret (`secure:` mapping) or a plain YAML value.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum StackFileValue {
    Secure { secure: String },
    Plain(serde_yaml::Value),
}

/// Render a plain YAML config value as the string form `ConfigValue` carries.
fn yaml_value_to_string(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => String::new(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim_end()
            .to_string(),
    }
}

/// A stack handle that uses the native Rust engine instead of the Pulumi CLI.
///
/// Created via [`LocalWorkspace::native_stack`] (requires the `native-engine` feature).
#[derive(Debug, Clone)]
pub struct NativeStack {
    workspace: LocalWorkspace,
    name: String,
    /// The program command to run (e.g. `["./target/release/my-program"]`).
    program: Vec<String>,
    /// Explicit passphrase override; falls back to `PULUMI_CONFIG_PASSPHRASE`
    /// / `PULUMI_CONFIG_PASSPHRASE_FILE` when unset.
    passphrase: Option<String>,
    /// Cached secrets manager, keyed by its salt state. PBKDF2 key derivation
    /// is deliberately expensive (1M iterations), so it must not be repeated
    /// for every config read/write.
    secrets_cache: Arc<Mutex<Option<PassphraseSecretsManager>>>,
}

impl NativeStack {
    pub(crate) fn new(workspace: LocalWorkspace, name: String, program: Vec<String>) -> Self {
        Self {
            workspace,
            name,
            program,
            passphrase: None,
            secrets_cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Sets the passphrase used to encrypt/decrypt secret config values and
    /// checkpoint state, overriding the `PULUMI_CONFIG_PASSPHRASE` /
    /// `PULUMI_CONFIG_PASSPHRASE_FILE` environment variables.
    pub fn with_passphrase(mut self, passphrase: impl Into<String>) -> Self {
        self.passphrase = Some(passphrase.into());
        self
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
    // Config persistence — Pulumi.<stack>.yaml, interoperable with the real CLI
    // ---------------------------------------------------------------------------

    /// Path to the stack settings file: `<work_dir>/Pulumi.<stack>.yaml`.
    fn stack_file_path(&self) -> PathBuf {
        self.workspace
            .work_dir()
            .join(format!("Pulumi.{}.yaml", self.name))
    }

    /// Path of the pre-YAML config file this crate used to write. Read as a
    /// migration fallback and deleted on the next save.
    fn legacy_config_path(&self) -> PathBuf {
        self.workspace
            .work_dir()
            .join(".pulumi-rs")
            .join(format!("{}.config.json", self.name))
    }

    /// Path to the checkpoint file for this stack.
    fn checkpoint_path(&self) -> PathBuf {
        self.workspace
            .work_dir()
            .join(".pulumi-rs")
            .join(format!("{}.json", self.name))
    }

    /// Resolve the passphrase: explicit override, then
    /// `PULUMI_CONFIG_PASSPHRASE`, then `PULUMI_CONFIG_PASSPHRASE_FILE`.
    fn resolve_passphrase(&self) -> Option<String> {
        if let Some(p) = &self.passphrase {
            return Some(p.clone());
        }
        if let Ok(p) = std::env::var("PULUMI_CONFIG_PASSPHRASE")
            && !p.is_empty()
        {
            return Some(p);
        }
        if let Ok(path) = std::env::var("PULUMI_CONFIG_PASSPHRASE_FILE")
            && !path.is_empty()
            && let Ok(contents) = std::fs::read_to_string(path)
        {
            let trimmed = contents.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
        None
    }

    fn require_passphrase(&self) -> Result<String> {
        self.resolve_passphrase().ok_or_else(|| {
            Error::Custom(
                "a passphrase is required to encrypt/decrypt secret config values; \
                 set PULUMI_CONFIG_PASSPHRASE (or PULUMI_CONFIG_PASSPHRASE_FILE), \
                 or use NativeStack::with_passphrase"
                    .to_string(),
            )
        })
    }

    /// Get a secrets manager for an existing salt state, using the cache when
    /// the salt matches (PBKDF2 at 1M iterations is ~1s per derivation).
    fn manager_for_salt(&self, salt_state: &str) -> Result<PassphraseSecretsManager> {
        {
            let cache = self.secrets_cache.lock().unwrap();
            if let Some(m) = cache.as_ref()
                && m.salt_state() == salt_state
            {
                return Ok(m.clone());
            }
        }
        let passphrase = self.require_passphrase()?;
        let mgr = PassphraseSecretsManager::from_salt(&passphrase, salt_state)
            .map_err(|e| Error::Custom(format!("unable to restore secrets manager: {e}")))?;
        *self.secrets_cache.lock().unwrap() = Some(mgr.clone());
        Ok(mgr)
    }

    /// Create a secrets manager with a fresh salt (first secret write).
    fn new_manager(&self) -> Result<PassphraseSecretsManager> {
        let passphrase = self.require_passphrase()?;
        let mgr = PassphraseSecretsManager::new(&passphrase)
            .map_err(|e| Error::Custom(format!("unable to create secrets manager: {e}")))?;
        *self.secrets_cache.lock().unwrap() = Some(mgr.clone());
        Ok(mgr)
    }

    /// The salt state persisted in `Pulumi.<stack>.yaml`, if any.
    fn stack_file_salt(&self) -> Option<String> {
        let contents = std::fs::read_to_string(self.stack_file_path()).ok()?;
        let sf: StackFile = serde_yaml::from_str(&contents).ok()?;
        sf.encryptionsalt
    }

    /// The salt state persisted in the checkpoint, if any.
    fn checkpoint_salt(&self) -> Option<String> {
        let cp = Checkpoint::load(&self.checkpoint_path()).ok()??;
        let sp = cp.secrets_provider?;
        sp.state.get("salt")?.as_str().map(String::from)
    }

    /// Load all config values from `Pulumi.<stack>.yaml`, decrypting `secure:`
    /// entries. Falls back to the legacy plaintext JSON file, and returns an
    /// empty map if neither exists.
    fn load_config_map(&self) -> Result<HashMap<String, ConfigValue>> {
        let path = self.stack_file_path();
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return self.load_legacy_config_map();
            }
            Err(e) => return Err(Error::Io(e)),
        };

        let stack_file: StackFile = serde_yaml::from_str(&contents)
            .map_err(|e| Error::Custom(format!("failed to parse {}: {e}", path.display())))?;
        let salt = stack_file.encryptionsalt;

        let mut map = HashMap::new();
        for (key, val) in stack_file.config {
            match val {
                StackFileValue::Secure { secure } => {
                    let salt = salt.as_deref().ok_or_else(|| {
                        Error::Custom(format!(
                            "{} has 'secure:' values but no encryptionsalt",
                            path.display()
                        ))
                    })?;
                    let plaintext = self
                        .manager_for_salt(salt)?
                        .decrypt_sync(&secure)
                        .map_err(|e| {
                            Error::Custom(format!("failed to decrypt config '{key}': {e}"))
                        })?;
                    let value = String::from_utf8(plaintext).map_err(|_| {
                        Error::Custom(format!("config '{key}' decrypted to non-UTF-8 data"))
                    })?;
                    map.insert(key, ConfigValue::secret(value));
                }
                StackFileValue::Plain(v) => {
                    map.insert(key, ConfigValue::plaintext(yaml_value_to_string(&v)));
                }
            }
        }
        Ok(map)
    }

    /// Read the legacy `.pulumi-rs/<stack>.config.json` flat map (plaintext).
    fn load_legacy_config_map(&self) -> Result<HashMap<String, ConfigValue>> {
        match std::fs::read_to_string(self.legacy_config_path()) {
            Ok(contents) => serde_json::from_str(&contents).map_err(Error::Json),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Save all config values to `Pulumi.<stack>.yaml`, encrypting secret
    /// values. Requires a passphrase when any value is secret. Removes the
    /// legacy plaintext JSON file once the YAML is written.
    fn save_config_map(&self, config: &HashMap<String, ConfigValue>) -> Result<()> {
        // Reuse an existing salt so previously written ciphertexts and the
        // checkpoint stay decryptable with the same key; generate a fresh
        // one only on the first secret write of a new stack.
        let existing_salt = self.stack_file_salt().or_else(|| self.checkpoint_salt());

        let manager = if config.values().any(|v| v.is_secret) {
            Some(match &existing_salt {
                Some(salt) => self.manager_for_salt(salt)?,
                None => self.new_manager()?,
            })
        } else {
            None
        };

        let mut entries = BTreeMap::new();
        for (key, cv) in config {
            let entry = if cv.is_secret {
                let secure = manager
                    .as_ref()
                    .expect("manager is constructed when any value is secret")
                    .encrypt_sync(cv.value.as_bytes())
                    .map_err(|e| {
                        Error::Custom(format!("failed to encrypt config '{key}': {e}"))
                    })?;
                StackFileValue::Secure { secure }
            } else {
                StackFileValue::Plain(serde_yaml::Value::String(cv.value.clone()))
            };
            entries.insert(key.clone(), entry);
        }

        let stack_file = StackFile {
            encryptionsalt: manager
                .as_ref()
                .map(|m| m.salt_state().to_string())
                .or(existing_salt),
            config: entries,
        };
        let contents = serde_yaml::to_string(&stack_file)
            .map_err(|e| Error::Custom(format!("failed to serialize stack file: {e}")))?;
        std::fs::write(self.stack_file_path(), contents).map_err(Error::Io)?;

        // The YAML file is now authoritative; drop the legacy plaintext file
        // so secret values don't linger on disk unencrypted.
        let legacy = self.legacy_config_path();
        if legacy.exists() {
            let _ = std::fs::remove_file(&legacy);
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // EngineOptions construction
    // ---------------------------------------------------------------------------

    fn engine_options(&self, dry_run: bool) -> Result<EngineOptions> {
        let project = self
            .workspace
            .work_dir()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();

        // Load config from disk and split into values + secret key list.
        // Fails when secret values exist but no/wrong passphrase is set —
        // running with silently dropped config would be worse.
        let config_map = self.load_config_map()?;
        let mut config = HashMap::new();
        let mut config_secret_keys = Vec::new();
        for (k, v) in &config_map {
            config.insert(k.clone(), v.value.clone());
            if v.is_secret {
                config_secret_keys.push(k.clone());
            }
        }

        // Prefer the checkpoint's salt (prior state must stay decryptable),
        // then the stack file's (so both artifacts share one key), then a
        // fresh salt. No passphrase means no secrets manager, as before.
        let secrets_manager = if self.resolve_passphrase().is_some() {
            match self.checkpoint_salt().or_else(|| self.stack_file_salt()) {
                Some(salt) => Some(self.manager_for_salt(&salt)?),
                None => Some(self.new_manager()?),
            }
        } else {
            None
        };

        Ok(EngineOptions {
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
        })
    }

    // ---------------------------------------------------------------------------
    // Lifecycle operations
    // ---------------------------------------------------------------------------

    /// Runs the Pulumi program to create or update resources using the native engine.
    pub async fn up(&self) -> Result<UpResult> {
        let opts = self.engine_options(false)?;
        let engine = PulumiEngine::new(opts);
        let result = engine.up().await?;

        let outputs = convert_outputs(&result.outputs);
        let events = convert_events(result.events);

        Ok(UpResult {
            stdout: result.stdout,
            stderr: result.stderr,
            outputs,
            events,
        })
    }

    /// Runs the Pulumi program in preview mode using the native engine.
    pub async fn preview(&self) -> Result<crate::stack::PreviewResult> {
        let opts = self.engine_options(true)?;
        let engine = PulumiEngine::new(opts);
        let result = engine.preview().await?;

        Ok(crate::stack::PreviewResult {
            stdout: result.stdout,
            stderr: result.stderr,
            events: convert_events(result.events),
        })
    }

    /// Destroys all resources in the stack using the native engine.
    pub async fn destroy(&self) -> Result<crate::stack::DestroyResult> {
        let opts = self.engine_options(false)?;
        let engine = PulumiEngine::new(opts);
        let result = engine.destroy().await?;

        Ok(crate::stack::DestroyResult {
            stdout: result.stdout,
            stderr: result.stderr,
            events: convert_events(result.events),
        })
    }

    /// Refreshes the stack state using the native engine.
    pub async fn refresh(&self) -> Result<crate::stack::RefreshResult> {
        let opts = self.engine_options(false)?;
        let engine = PulumiEngine::new(opts);
        let result = engine.refresh().await?;

        Ok(crate::stack::RefreshResult {
            stdout: result.stdout,
            stderr: result.stderr,
            events: convert_events(result.events),
        })
    }

    // ---------------------------------------------------------------------------
    // Stack outputs
    // ---------------------------------------------------------------------------

    /// Returns the current stack outputs by reading the checkpoint file.
    pub fn outputs(&self) -> Result<HashMap<String, OutputValue>> {
        match Checkpoint::load(&self.checkpoint_path()).map_err(Error::Io)? {
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

/// Convert native engine events into the automation-layer EngineEvent format.
fn convert_events(native_events: Vec<NativeEvent>) -> Vec<EngineEvent> {
    native_events
        .into_iter()
        .enumerate()
        .map(|(i, ev)| {
            let seq = i as i64 + 1;
            match ev {
                NativeEvent::Prelude { config } => EngineEvent {
                    sequence: seq,
                    prelude_event: Some(PreludeEvent { config }),
                    resource_pre_event: None,
                    summary_event: None,
                    diagnostic_event: None,
                },
                NativeEvent::ResourceStep {
                    op,
                    urn,
                    resource_type,
                    old_inputs,
                    old_outputs,
                    new_inputs,
                    new_outputs,
                } => {
                    let old = if old_inputs.is_null() && old_outputs.is_null() {
                        None
                    } else {
                        Some(StepEventStateMetadata {
                            resource_type: resource_type.clone(),
                            urn: urn.clone(),
                            inputs: old_inputs,
                            outputs: old_outputs,
                        })
                    };
                    let new = if new_inputs.is_null() && new_outputs.is_null() {
                        None
                    } else {
                        Some(StepEventStateMetadata {
                            resource_type: resource_type.clone(),
                            urn: urn.clone(),
                            inputs: new_inputs,
                            outputs: new_outputs,
                        })
                    };
                    EngineEvent {
                        sequence: seq,
                        prelude_event: None,
                        resource_pre_event: Some(ResourcePreEvent {
                            metadata: StepEventMetadata {
                                op,
                                urn,
                                resource_type,
                                old,
                                new,
                            },
                        }),
                        summary_event: None,
                        diagnostic_event: None,
                    }
                }
                NativeEvent::Diagnostic {
                    urn,
                    severity,
                    message,
                } => EngineEvent {
                    sequence: seq,
                    prelude_event: None,
                    resource_pre_event: None,
                    summary_event: None,
                    diagnostic_event: Some(DiagnosticEvent {
                        urn,
                        severity,
                        message,
                    }),
                },
                NativeEvent::Summary {
                    may_update,
                    duration_seconds,
                    resource_changes,
                } => EngineEvent {
                    sequence: seq,
                    prelude_event: None,
                    resource_pre_event: None,
                    summary_event: Some(SummaryEvent {
                        may_update,
                        duration_seconds,
                        resource_changes,
                    }),
                    diagnostic_event: None,
                },
            }
        })
        .collect()
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

    /// A stack with a passphrase configured, for tests that touch secrets.
    fn make_stack_with_passphrase(tmp: &TempDir) -> NativeStack {
        make_stack(tmp).with_passphrase("test-passphrase")
    }

    #[test]
    fn config_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack_with_passphrase(&tmp);

        stack.set_config("myproject:region", ConfigValue::plaintext("us-east-1")).unwrap();
        stack.set_config("myproject:token", ConfigValue::secret("s3cr3t")).unwrap();

        let all = stack.get_all_config().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all["myproject:region"].value, "us-east-1");
        assert!(!all["myproject:region"].is_secret);
        assert_eq!(all["myproject:token"].value, "s3cr3t");
        assert!(all["myproject:token"].is_secret);
    }

    /// Secret values must be ciphertext on disk, in the real CLI's
    /// `Pulumi.<stack>.yaml` shape: an `encryptionsalt` plus `secure:` entries.
    #[test]
    fn secret_config_is_encrypted_at_rest() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack_with_passphrase(&tmp);

        stack.set_config("proj:password", ConfigValue::secret("hunter2")).unwrap();
        stack.set_config("proj:region", ConfigValue::plaintext("us-east-1")).unwrap();

        let raw = std::fs::read_to_string(tmp.path().join("Pulumi.dev.yaml")).unwrap();
        assert!(!raw.contains("hunter2"), "plaintext secret leaked to disk:\n{raw}");
        assert!(raw.contains("secure: v1:"), "expected secure ciphertext entry:\n{raw}");
        assert!(raw.contains("encryptionsalt: v1:"), "expected salt state:\n{raw}");
        assert!(raw.contains("us-east-1"), "plain values stay readable:\n{raw}");

        // And it round-trips back through decryption.
        let cv = stack.get_config("proj:password").unwrap();
        assert_eq!(cv.value, "hunter2");
        assert!(cv.is_secret);
    }

    /// The salt must stay stable across saves so earlier ciphertexts remain
    /// decryptable, including from a fresh stack handle (no warm cache).
    #[test]
    fn salt_is_stable_across_saves() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack_with_passphrase(&tmp);

        stack.set_config("proj:first", ConfigValue::secret("one")).unwrap();
        let salt_before = stack.stack_file_salt().unwrap();
        stack.set_config("proj:second", ConfigValue::secret("two")).unwrap();
        assert_eq!(stack.stack_file_salt().unwrap(), salt_before);

        let fresh = make_stack_with_passphrase(&tmp);
        let all = fresh.get_all_config().unwrap();
        assert_eq!(all["proj:first"].value, "one");
        assert_eq!(all["proj:second"].value, "two");
    }

    /// Writing a secret without any passphrase is a hard error, not a
    /// silent plaintext fallback.
    #[test]
    fn set_secret_without_passphrase_errors() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);
        let err = stack
            .set_config("proj:token", ConfigValue::secret("s3cr3t"))
            .unwrap_err();
        assert!(err.to_string().contains("PULUMI_CONFIG_PASSPHRASE"));
        // Nothing was written.
        assert!(!tmp.path().join("Pulumi.dev.yaml").exists());
    }

    /// Reading secrets with the wrong passphrase fails passphrase validation.
    #[test]
    fn wrong_passphrase_errors() {
        let tmp = TempDir::new().unwrap();
        make_stack(&tmp)
            .with_passphrase("correct")
            .set_config("proj:token", ConfigValue::secret("s3cr3t"))
            .unwrap();

        let stack = make_stack(&tmp).with_passphrase("wrong");
        assert!(stack.get_config("proj:token").is_err());
    }

    /// Plain-only config needs no passphrase at any point.
    #[test]
    fn plain_config_needs_no_passphrase() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);
        stack.set_config("proj:region", ConfigValue::plaintext("eu-west-1")).unwrap();
        assert_eq!(stack.get_config("proj:region").unwrap().value, "eu-west-1");

        let raw = std::fs::read_to_string(tmp.path().join("Pulumi.dev.yaml")).unwrap();
        assert!(!raw.contains("encryptionsalt"), "no salt without secrets:\n{raw}");
    }

    /// The pre-YAML flat JSON file is still readable, and the first save
    /// migrates to YAML and removes it.
    #[test]
    fn legacy_json_config_migrates_to_yaml() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);

        let legacy_dir = tmp.path().join(".pulumi-rs");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let legacy_path = legacy_dir.join("dev.config.json");
        std::fs::write(
            &legacy_path,
            r#"{"proj:region": {"value": "us-west-2", "secret": false}}"#,
        )
        .unwrap();

        // Readable through the fallback.
        assert_eq!(stack.get_config("proj:region").unwrap().value, "us-west-2");

        // A write migrates to YAML and deletes the legacy file.
        stack.set_config("proj:zone", ConfigValue::plaintext("us-west-2a")).unwrap();
        assert!(tmp.path().join("Pulumi.dev.yaml").exists());
        assert!(!legacy_path.exists());
        assert_eq!(stack.get_config("proj:region").unwrap().value, "us-west-2");
        assert_eq!(stack.get_config("proj:zone").unwrap().value, "us-west-2a");
    }

    /// A stack file written by the real Pulumi CLI parses: plain scalars of
    /// non-string YAML types are stringified.
    #[test]
    fn parses_cli_style_stack_file_scalars() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack(&tmp);
        std::fs::write(
            tmp.path().join("Pulumi.dev.yaml"),
            "config:\n  proj:count: 3\n  proj:enabled: true\n  proj:name: web\n",
        )
        .unwrap();

        let all = stack.get_all_config().unwrap();
        assert_eq!(all["proj:count"].value, "3");
        assert_eq!(all["proj:enabled"].value, "true");
        assert_eq!(all["proj:name"].value, "web");
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
        let stack = make_stack_with_passphrase(&tmp);
        stack.set_config("proj:region", ConfigValue::plaintext("eu-west-1")).unwrap();
        stack.set_config("proj:token", ConfigValue::secret("tok")).unwrap();

        let opts = stack.engine_options(false).unwrap();
        assert_eq!(opts.config.get("proj:region").map(String::as_str), Some("eu-west-1"));
        assert_eq!(opts.config.get("proj:token").map(String::as_str), Some("tok"));
        assert!(opts.config_secret_keys.contains(&"proj:token".to_string()));
        assert!(!opts.config_secret_keys.contains(&"proj:region".to_string()));
    }

    /// The engine's secrets manager reuses the stack file's salt, so the
    /// checkpoint and config file share one key.
    #[test]
    fn engine_options_reuses_stack_file_salt() {
        let tmp = TempDir::new().unwrap();
        let stack = make_stack_with_passphrase(&tmp);
        stack.set_config("proj:token", ConfigValue::secret("tok")).unwrap();

        let opts = stack.engine_options(false).unwrap();
        let mgr = opts.secrets_manager.expect("manager present when passphrase set");
        assert_eq!(mgr.salt_state(), stack.stack_file_salt().unwrap());
    }
}
