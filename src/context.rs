use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::{Error, Result};
use crate::proto::pulumirpc;
use crate::proto::pulumirpc::engine_client::EngineClient;
use crate::proto::pulumirpc::resource_monitor_client::ResourceMonitorClient;

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
        let monitor_addr = std::env::var("PULUMI_MONITOR")
            .map_err(|_| Error::MissingEnv("PULUMI_MONITOR"))?;
        let engine_addr =
            std::env::var("PULUMI_ENGINE").map_err(|_| Error::MissingEnv("PULUMI_ENGINE"))?;
        let project =
            std::env::var("PULUMI_PROJECT").map_err(|_| Error::MissingEnv("PULUMI_PROJECT"))?;
        let stack =
            std::env::var("PULUMI_STACK").map_err(|_| Error::MissingEnv("PULUMI_STACK"))?;
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

/// The inner state of a Pulumi [`Context`], shared behind an `Arc<Mutex<...>>`.
struct ContextInner {
    monitor: ResourceMonitorClient<tonic::transport::Channel>,
    engine: EngineClient<tonic::transport::Channel>,
    /// The URN of the root stack resource.
    root_urn: Option<String>,
}

/// The Pulumi program context.
///
/// `Context` holds the gRPC connections to the Pulumi engine and resource monitor.
/// It is the entry point for registering resources, invoking functions, and logging.
///
/// A `Context` is created by [`run`](crate::run), which sets up connections and
/// passes the context to your program function.
#[derive(Clone)]
pub struct Context {
    inner: Arc<Mutex<ContextInner>>,
    settings: Settings,
}

impl Context {
    /// Creates a new context by connecting to the Pulumi engine and resource monitor.
    pub(crate) async fn new(settings: Settings) -> Result<Self> {
        let monitor_endpoint = to_endpoint(&settings.monitor_addr)?;
        let engine_endpoint = to_endpoint(&settings.engine_addr)?;

        let monitor = ResourceMonitorClient::connect(monitor_endpoint).await?;
        let engine = EngineClient::connect(engine_endpoint).await?;

        // Query the root resource URN.
        let mut engine_clone = engine.clone();
        let root_resp = engine_clone
            .get_root_resource(pulumirpc::GetRootResourceRequest {})
            .await?;
        let root_urn = {
            let urn = root_resp.into_inner().urn;
            if urn.is_empty() {
                None
            } else {
                Some(urn)
            }
        };

        let ctx = Context {
            settings: settings.clone(),
            inner: Arc::new(Mutex::new(ContextInner {
                monitor,
                engine,
                root_urn,
            })),
        };

        Ok(ctx)
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
        self.settings.config.get(key).map(String::as_str)
    }

    /// Gets a required configuration value, returning an error if missing.
    pub fn require_config(&self, key: &str) -> Result<&str> {
        self.get_config(key)
            .ok_or_else(|| Error::Custom(format!("missing required config: {key}")))
    }

    /// Returns the URN of the root stack resource, if known.
    pub async fn root_urn(&self) -> Option<String> {
        self.inner.lock().await.root_urn.clone()
    }

    /// Sets the root resource URN (called after stack registration).
    pub(crate) async fn set_root_urn(&self, urn: String) {
        self.inner.lock().await.root_urn = Some(urn);
    }

    /// Returns a clone of the resource monitor client for internal use.
    pub(crate) async fn monitor(
        &self,
    ) -> ResourceMonitorClient<tonic::transport::Channel> {
        self.inner.lock().await.monitor.clone()
    }

    /// Returns a clone of the engine client for internal use.
    pub(crate) async fn engine(&self) -> EngineClient<tonic::transport::Channel> {
        self.inner.lock().await.engine.clone()
    }

    /// Returns whether a feature is supported by the resource monitor.
    #[allow(dead_code)]
    pub(crate) async fn supports_feature(&self, feature: &str) -> Result<bool> {
        let mut monitor = self.monitor().await;
        let resp = monitor
            .supports_feature(pulumirpc::SupportsFeatureRequest {
                id: feature.to_string(),
            })
            .await?;
        Ok(resp.into_inner().has_support)
    }
}

/// Converts an address string (potentially just `host:port`) to a proper endpoint URI.
fn to_endpoint(addr: &str) -> Result<tonic::transport::Endpoint> {
    let uri = if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.to_string()
    } else {
        format!("http://{addr}")
    };
    tonic::transport::Endpoint::from_shared(uri).map_err(|e| {
        Error::Custom(format!("invalid endpoint address: {e}"))
    })
}
