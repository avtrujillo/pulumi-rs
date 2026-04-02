use std::collections::HashMap;
use std::sync::Arc;

use crate::connection::{EngineConnection, GrpcEngine, GrpcMonitor, MonitorConnection};
use crate::error::{Error, Result};
use crate::proto::pulumirpc;

/// Runtime settings extracted from environment variables set by the Pulumi engine.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The address of the resource monitor gRPC endpoint.
    pub monitor_addr: String,
    /// The address of the engine gRPC endpoint.
    pub engine_addr: String,
    /// The current project name.
    pub project: String,
    /// The current stack name.
    pub stack: String,
    /// Whether this is a dry run (preview).
    pub dry_run: bool,
    /// The degree of parallelism for resource operations.
    pub parallel: i32,
    /// The organization name, if any.
    pub organization: String,
    /// Configuration values (key -> value).
    pub config: HashMap<String, String>,
    /// Keys whose values are secret.
    pub config_secret_keys: Vec<String>,
}

impl Settings {
    /// Reads settings from the environment variables set by the Pulumi engine.
    pub fn from_env() -> Result<Self> {
        let monitor_addr =
            std::env::var("PULUMI_MONITOR").map_err(|_| Error::MissingEnv("PULUMI_MONITOR"))?;
        let engine_addr =
            std::env::var("PULUMI_ENGINE").map_err(|_| Error::MissingEnv("PULUMI_ENGINE"))?;
        let project =
            std::env::var("PULUMI_PROJECT").map_err(|_| Error::MissingEnv("PULUMI_PROJECT"))?;
        let stack = std::env::var("PULUMI_STACK").map_err(|_| Error::MissingEnv("PULUMI_STACK"))?;
        let dry_run = std::env::var("PULUMI_DRY_RUN")
            .map(|v| v == "true")
            .unwrap_or(false);
        let parallel = std::env::var("PULUMI_PARALLEL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(-1);
        let organization = std::env::var("PULUMI_ORGANIZATION").unwrap_or_default();

        let config = parse_config(&std::env::var("PULUMI_CONFIG").unwrap_or_default());
        let config_secret_keys =
            parse_list(&std::env::var("PULUMI_CONFIG_SECRET_KEYS").unwrap_or_default());

        Ok(Settings {
            monitor_addr,
            engine_addr,
            project,
            stack,
            dry_run,
            parallel,
            organization,
            config,
            config_secret_keys,
        })
    }
}

impl Settings {
    /// Gets a configuration value by key.
    pub fn get_config(&self, key: &str) -> Option<&str> {
        self.config.get(key).map(String::as_str)
    }

    /// Gets a required configuration value, returning an error if missing.
    pub fn require_config(&self, key: &str) -> Result<&str> {
        self.get_config(key)
            .ok_or_else(|| Error::Custom(format!("missing required config: {key}")))
    }

    /// Gets a configuration value as a `bool`.
    ///
    /// Returns `None` if the key is missing. Returns an error if the value
    /// is not `"true"` or `"false"`.
    pub fn get_config_bool(&self, key: &str) -> Result<Option<bool>> {
        match self.get_config(key) {
            None => Ok(None),
            Some("true") => Ok(Some(true)),
            Some("false") => Ok(Some(false)),
            Some(v) => Err(Error::Custom(format!(
                "config {key}: expected \"true\" or \"false\", got {v:?}"
            ))),
        }
    }

    /// Gets a required configuration value as a `bool`.
    pub fn require_config_bool(&self, key: &str) -> Result<bool> {
        self.get_config_bool(key)?
            .ok_or_else(|| Error::Custom(format!("missing required config: {key}")))
    }

    /// Gets a configuration value as an `i64`.
    ///
    /// Returns `None` if the key is missing. Returns an error if the value
    /// cannot be parsed as an integer.
    pub fn get_config_int(&self, key: &str) -> Result<Option<i64>> {
        match self.get_config(key) {
            None => Ok(None),
            Some(v) => v
                .parse::<i64>()
                .map(Some)
                .map_err(|_| Error::Custom(format!("config {key}: expected integer, got {v:?}"))),
        }
    }

    /// Gets a required configuration value as an `i64`.
    pub fn require_config_int(&self, key: &str) -> Result<i64> {
        self.get_config_int(key)?
            .ok_or_else(|| Error::Custom(format!("missing required config: {key}")))
    }

    /// Gets a configuration value as an `f64`.
    ///
    /// Returns `None` if the key is missing. Returns an error if the value
    /// cannot be parsed as a floating-point number.
    pub fn get_config_float(&self, key: &str) -> Result<Option<f64>> {
        match self.get_config(key) {
            None => Ok(None),
            Some(v) => v
                .parse::<f64>()
                .map(Some)
                .map_err(|_| Error::Custom(format!("config {key}: expected number, got {v:?}"))),
        }
    }

    /// Gets a required configuration value as an `f64`.
    pub fn require_config_float(&self, key: &str) -> Result<f64> {
        self.get_config_float(key)?
            .ok_or_else(|| Error::Custom(format!("missing required config: {key}")))
    }

    /// Gets a configuration value deserialized as a JSON object of type `T`.
    ///
    /// Returns `None` if the key is missing. Returns an error if the value
    /// cannot be parsed as JSON or deserialized into `T`.
    pub fn get_config_object<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>> {
        match self.get_config(key) {
            None => Ok(None),
            Some(v) => serde_json::from_str(v)
                .map(Some)
                .map_err(|e| Error::Custom(format!("config {key}: {e}"))),
        }
    }

    /// Gets a required configuration value deserialized as a JSON object of type `T`.
    pub fn require_config_object<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<T> {
        self.get_config_object(key)?
            .ok_or_else(|| Error::Custom(format!("missing required config: {key}")))
    }

    /// Returns whether a configuration value is secret.
    pub fn is_config_secret(&self, key: &str) -> bool {
        self.config_secret_keys.contains(&key.to_string())
    }
}

fn parse_config(raw: &str) -> HashMap<String, String> {
    if raw.is_empty() {
        return HashMap::new();
    }
    // Pulumi sends config as JSON: {"key": "value", ...}
    serde_json::from_str(raw).unwrap_or_default()
}

fn parse_list(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    serde_json::from_str(raw).unwrap_or_default()
}

/// The Pulumi program context.
///
/// `Context` holds the connections to the Pulumi engine and resource monitor.
/// It is the entry point for registering resources, invoking functions, and logging.
///
/// A `Context` is created by [`run`](crate::run), which sets up connections and
/// passes the context to your program function.
///
/// For testing, use [`Context::for_testing`] to create a context backed by mock
/// connections instead of real gRPC clients.
#[derive(Clone)]
pub struct Context {
    monitor: Arc<dyn MonitorConnection>,
    engine: Arc<dyn EngineConnection>,
    settings: Settings,
}

impl Context {
    /// Creates a new context by connecting to the Pulumi engine and resource monitor.
    pub(crate) async fn new(settings: Settings) -> Result<Self> {
        let monitor_endpoint = to_endpoint(&settings.monitor_addr)?;
        let engine_endpoint = to_endpoint(&settings.engine_addr)?;

        let monitor_client =
            pulumirpc::resource_monitor_client::ResourceMonitorClient::connect(monitor_endpoint)
                .await?;
        let engine_client =
            pulumirpc::engine_client::EngineClient::connect(engine_endpoint).await?;

        let monitor = Arc::new(GrpcMonitor::new(monitor_client));
        let engine = Arc::new(GrpcEngine::new(engine_client));

        Ok(Context {
            settings,
            monitor,
            engine,
        })
    }

    /// Creates a context for testing with custom monitor and engine implementations.
    ///
    /// This allows running Pulumi programs against mock backends without a real
    /// engine or provider plugins.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use pulumi_core::connection::{MockMonitor, MockEngine};
    /// use pulumi_core::context::{Context, Settings};
    ///
    /// let settings = Settings { project: "test".into(), stack: "dev".into(), /* ... */ };
    /// let ctx = Context::for_testing(
    ///     MockMonitor::new("test", "dev"),
    ///     MockEngine::new(),
    ///     settings,
    /// );
    /// ```
    pub fn for_testing(
        monitor: impl MonitorConnection,
        engine: impl EngineConnection,
        settings: Settings,
    ) -> Self {
        Context {
            monitor: Arc::new(monitor),
            engine: Arc::new(engine),
            settings,
        }
    }

    /// Returns the current project name.
    pub fn project(&self) -> &str {
        &self.settings.project
    }

    /// Returns the current stack name.
    pub fn stack(&self) -> &str {
        &self.settings.stack
    }

    /// Returns whether this is a dry run (preview).
    pub fn is_dry_run(&self) -> bool {
        self.settings.dry_run
    }

    /// Returns the organization name.
    pub fn organization(&self) -> &str {
        &self.settings.organization
    }

    /// Gets a configuration value by key.
    ///
    /// The key should be in the form `namespace:key`, e.g. `"aws:region"`.
    pub fn get_config(&self, key: &str) -> Option<&str> {
        self.settings.get_config(key)
    }

    /// Gets a required configuration value, returning an error if missing.
    pub fn require_config(&self, key: &str) -> Result<&str> {
        self.settings.require_config(key)
    }

    /// Gets a configuration value as a `bool`.
    ///
    /// Returns `None` if the key is missing. Returns an error if the value
    /// is not `"true"` or `"false"`.
    pub fn get_config_bool(&self, key: &str) -> Result<Option<bool>> {
        self.settings.get_config_bool(key)
    }

    /// Gets a required configuration value as a `bool`.
    pub fn require_config_bool(&self, key: &str) -> Result<bool> {
        self.settings.require_config_bool(key)
    }

    /// Gets a configuration value as an `i64`.
    ///
    /// Returns `None` if the key is missing. Returns an error if the value
    /// cannot be parsed as an integer.
    pub fn get_config_int(&self, key: &str) -> Result<Option<i64>> {
        self.settings.get_config_int(key)
    }

    /// Gets a required configuration value as an `i64`.
    pub fn require_config_int(&self, key: &str) -> Result<i64> {
        self.settings.require_config_int(key)
    }

    /// Gets a configuration value as an `f64`.
    ///
    /// Returns `None` if the key is missing. Returns an error if the value
    /// cannot be parsed as a floating-point number.
    pub fn get_config_float(&self, key: &str) -> Result<Option<f64>> {
        self.settings.get_config_float(key)
    }

    /// Gets a required configuration value as an `f64`.
    pub fn require_config_float(&self, key: &str) -> Result<f64> {
        self.settings.require_config_float(key)
    }

    /// Gets a configuration value deserialized as a JSON object of type `T`.
    ///
    /// Returns `None` if the key is missing. Returns an error if the value
    /// cannot be parsed as JSON or deserialized into `T`.
    pub fn get_config_object<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>> {
        self.settings.get_config_object(key)
    }

    /// Gets a required configuration value deserialized as a JSON object of type `T`.
    pub fn require_config_object<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<T> {
        self.settings.require_config_object(key)
    }

    /// Returns whether a configuration value is secret.
    pub fn is_config_secret(&self, key: &str) -> bool {
        self.settings.is_config_secret(key)
    }

    /// Returns a reference to the monitor connection for internal use.
    pub(crate) fn monitor(&self) -> &dyn MonitorConnection {
        &*self.monitor
    }

    /// Returns a reference to the engine connection for internal use.
    pub(crate) fn engine(&self) -> &dyn EngineConnection {
        &*self.engine
    }

    /// Returns whether a feature is supported by the resource monitor.
    #[allow(dead_code)]
    pub(crate) async fn supports_feature(&self, feature: &str) -> Result<bool> {
        self.monitor.supports_feature(feature).await
    }
}

/// Converts an address string (potentially just `host:port`) to a proper endpoint URI.
fn to_endpoint(addr: &str) -> Result<tonic::transport::Endpoint> {
    let uri = if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.to_string()
    } else {
        format!("http://{addr}")
    };
    tonic::transport::Endpoint::from_shared(uri)
        .map_err(|e| Error::Custom(format!("invalid endpoint address: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_settings(config: HashMap<String, String>) -> Settings {
        Settings {
            monitor_addr: String::new(),
            engine_addr: String::new(),
            project: "test".into(),
            stack: "dev".into(),
            dry_run: false,
            parallel: -1,
            organization: String::new(),
            config_secret_keys: vec!["app:secret".into()],
            config,
        }
    }

    #[test]
    fn test_get_config_missing() {
        let s = test_settings(HashMap::new());
        assert_eq!(s.get_config("app:missing"), None);
    }

    #[test]
    fn test_get_config_string() {
        let s = test_settings(HashMap::from([("app:region".into(), "us-east-1".into())]));
        assert_eq!(s.get_config("app:region"), Some("us-east-1"));
    }

    #[test]
    fn test_require_config_missing() {
        let s = test_settings(HashMap::new());
        assert!(s.require_config("app:missing").is_err());
    }

    #[test]
    fn test_get_config_bool() {
        let s = test_settings(HashMap::from([
            ("app:enabled".into(), "true".into()),
            ("app:disabled".into(), "false".into()),
            ("app:bad".into(), "yes".into()),
        ]));
        assert_eq!(s.get_config_bool("app:enabled").unwrap(), Some(true));
        assert_eq!(s.get_config_bool("app:disabled").unwrap(), Some(false));
        assert_eq!(s.get_config_bool("app:missing").unwrap(), None);
        assert!(s.get_config_bool("app:bad").is_err());
    }

    #[test]
    fn test_get_config_int() {
        let s = test_settings(HashMap::from([
            ("app:count".into(), "42".into()),
            ("app:negative".into(), "-7".into()),
            ("app:bad".into(), "abc".into()),
        ]));
        assert_eq!(s.get_config_int("app:count").unwrap(), Some(42));
        assert_eq!(s.get_config_int("app:negative").unwrap(), Some(-7));
        assert_eq!(s.get_config_int("app:missing").unwrap(), None);
        assert!(s.get_config_int("app:bad").is_err());
    }

    #[test]
    fn test_get_config_float() {
        let s = test_settings(HashMap::from([
            ("app:rate".into(), "3.14".into()),
            ("app:bad".into(), "abc".into()),
        ]));
        assert!((s.get_config_float("app:rate").unwrap().unwrap() - 3.14).abs() < f64::EPSILON);
        assert_eq!(s.get_config_float("app:missing").unwrap(), None);
        assert!(s.get_config_float("app:bad").is_err());
    }

    #[test]
    fn test_get_config_object() {
        let s = test_settings(HashMap::from([(
            "app:tags".into(),
            r#"{"env":"prod","team":"infra"}"#.into(),
        )]));
        let tags: HashMap<String, String> = s.get_config_object("app:tags").unwrap().unwrap();
        assert_eq!(tags.get("env").unwrap(), "prod");
        assert_eq!(tags.get("team").unwrap(), "infra");
        assert_eq!(
            s.get_config_object::<HashMap<String, String>>("app:missing")
                .unwrap(),
            None
        );
    }

    #[test]
    fn test_is_config_secret() {
        let s = test_settings(HashMap::from([("app:secret".into(), "hunter2".into())]));
        assert!(s.is_config_secret("app:secret"));
        assert!(!s.is_config_secret("app:region"));
    }

    #[test]
    fn test_require_config_bool() {
        let s = test_settings(HashMap::from([("app:flag".into(), "true".into())]));
        assert_eq!(s.require_config_bool("app:flag").unwrap(), true);
        assert!(s.require_config_bool("app:missing").is_err());
    }

    #[test]
    fn test_require_config_int() {
        let s = test_settings(HashMap::from([("app:port".into(), "8080".into())]));
        assert_eq!(s.require_config_int("app:port").unwrap(), 8080);
        assert!(s.require_config_int("app:missing").is_err());
    }
}
