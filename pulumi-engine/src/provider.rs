//! Provider plugin lifecycle management.
//!
//! Launches provider plugins as subprocesses, connects to their gRPC services,
//! and routes CRUD/invoke/call operations through them.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;

use crate::error::{Error, Result};
use crate::pulumirpc;

type ProviderClient = pulumirpc::resource_provider_client::ResourceProviderClient<Channel>;

/// Manages provider plugin processes and their gRPC connections.
#[derive(Clone)]
pub struct ProviderManager {
    inner: Arc<Mutex<ProviderManagerInner>>,
    engine_addr: Arc<Mutex<String>>,
}

struct ProviderManagerInner {
    /// Running provider plugins, keyed by package name (e.g. "aws").
    providers: HashMap<String, ProviderInstance>,
}

struct ProviderInstance {
    client: ProviderClient,
    child: tokio::process::Child,
}

impl Default for ProviderManager {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ProviderManagerInner {
                providers: HashMap::new(),
            })),
            engine_addr: Arc::new(Mutex::new(String::new())),
        }
    }
}

impl ProviderManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the engine address (called once the engine gRPC server is bound).
    pub async fn set_engine_addr(&self, addr: String) {
        *self.engine_addr.lock().await = addr;
    }

    /// Extract the package name from a Pulumi type token.
    ///
    /// Type tokens have the form `<pkg>:<module>/<name>:<type>` or `<pkg>:<module>:<type>`.
    /// Provider resources have the form `pulumi:providers:<pkg>`.
    pub fn package_name(type_token: &str) -> Option<String> {
        if type_token.starts_with("pulumi:providers:") {
            return Some(
                type_token
                    .strip_prefix("pulumi:providers:")
                    .unwrap()
                    .to_string(),
            );
        }
        let colon = type_token.find(':')?;
        let pkg = &type_token[..colon];
        // Skip built-in types
        if pkg == "pulumi" {
            return None;
        }
        Some(pkg.to_string())
    }

    /// Extract the package name from an invoke/call function token.
    ///
    /// Function tokens have the form `<pkg>:<module>/<name>:<function>` or `<pkg>:<module>:<function>`.
    pub fn package_name_from_token(token: &str) -> Option<String> {
        let colon = token.find(':')?;
        let pkg = &token[..colon];
        if pkg == "pulumi" {
            return None;
        }
        Some(pkg.to_string())
    }

    /// Get or launch a provider plugin for the given package.
    pub async fn get_provider(&self, package: &str) -> Result<ProviderClient> {
        let mut inner = self.inner.lock().await;
        if let Some(instance) = inner.providers.get(package) {
            return Ok(instance.client.clone());
        }

        // Launch the provider plugin.
        let plugin_name = format!("pulumi-resource-{package}");
        eprintln!("[engine] launching provider plugin: {plugin_name}");

        let engine_addr = self.engine_addr.lock().await.clone();

        // Provider plugins accept the engine address as an argument and print their
        // own gRPC port to stdout.
        let mut child = tokio::process::Command::new(&plugin_name)
            .arg(&engine_addr)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                Error::Custom(format!(
                    "failed to launch provider plugin '{plugin_name}': {e}"
                ))
            })?;

        // Read the port number from stdout.
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Custom(format!("provider '{plugin_name}' has no stdout")))?;

        let port = read_provider_port(stdout).await.map_err(|e| {
            Error::Custom(format!(
                "failed to read port from provider '{plugin_name}': {e}"
            ))
        })?;

        let addr = format!("http://127.0.0.1:{port}");
        eprintln!("[engine] provider {plugin_name} listening on {addr}");

        // Connect to the provider's gRPC service.
        let channel = Channel::from_shared(addr.clone())
            .map_err(|e| Error::Custom(format!("invalid provider address '{addr}': {e}")))?
            .connect()
            .await
            .map_err(|e| {
                Error::Custom(format!(
                    "failed to connect to provider '{plugin_name}': {e}"
                ))
            })?;

        let mut client = ProviderClient::new(channel);

        // Configure the provider.
        client
            .configure(pulumirpc::ConfigureRequest {
                accept_secrets: true,
                accept_resources: true,
                sends_old_inputs: true,
                sends_old_inputs_to_delete: true,
                ..Default::default()
            })
            .await
            .map_err(|e| {
                Error::Custom(format!("failed to configure provider '{plugin_name}': {e}"))
            })?;

        inner.providers.insert(
            package.to_string(),
            ProviderInstance {
                client: client.clone(),
                child,
            },
        );

        Ok(client)
    }

    /// Shut down all running provider plugins.
    pub async fn shutdown_all(&self) {
        let mut inner = self.inner.lock().await;
        for (name, mut instance) in inner.providers.drain() {
            eprintln!("[engine] shutting down provider: {name}");
            let _ = instance.child.kill().await;
        }
    }
}

/// Read the provider's gRPC port from its stdout.
///
/// Pulumi provider plugins print their port number on a single line to stdout
/// when they are ready to accept connections.
async fn read_provider_port(stdout: tokio::process::ChildStdout) -> Result<u16> {
    use tokio::io::AsyncBufReadExt;
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut line = String::new();

    // Read with a timeout — if the provider doesn't print a port within 30s, fail.
    let read_result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        reader.read_line(&mut line).await
    })
    .await
    .map_err(|_| Error::Custom("timed out waiting for provider to print port".into()))?
    .map_err(|e| Error::Custom(format!("failed to read provider stdout: {e}")))?;

    if read_result == 0 {
        return Err(Error::Custom("provider exited before printing port".into()));
    }

    let port: u16 = line.trim().parse().map_err(|e| {
        Error::Custom(format!(
            "provider printed invalid port '{}': {e}",
            line.trim()
        ))
    })?;

    Ok(port)
}

/// Convert a `serde_json::Value` to a `prost_types::Struct`.
pub(crate) fn json_to_proto_struct(value: &serde_json::Value) -> prost_types::Struct {
    match value {
        serde_json::Value::Object(map) => {
            let fields = map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_proto_value(v)))
                .collect();
            prost_types::Struct { fields }
        }
        _ => prost_types::Struct {
            fields: Default::default(),
        },
    }
}

fn json_to_proto_value(value: &serde_json::Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match value {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(*b),
        serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Kind::StringValue(s.clone()),
        serde_json::Value::Array(arr) => Kind::ListValue(prost_types::ListValue {
            values: arr.iter().map(json_to_proto_value).collect(),
        }),
        serde_json::Value::Object(_) => Kind::StructValue(json_to_proto_struct(value)),
    };
    prost_types::Value { kind: Some(kind) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_name_from_type_token() {
        assert_eq!(
            ProviderManager::package_name("aws:s3/bucket:Bucket"),
            Some("aws".into())
        );
        assert_eq!(
            ProviderManager::package_name("azure-native:storage:StorageAccount"),
            Some("azure-native".into())
        );
        assert_eq!(
            ProviderManager::package_name("pulumi:providers:aws"),
            Some("aws".into())
        );
        // Built-in Pulumi types should return None
        assert_eq!(ProviderManager::package_name("pulumi:pulumi:Stack"), None);
    }

    #[test]
    fn test_package_name_from_function_token() {
        assert_eq!(
            ProviderManager::package_name_from_token("aws:ec2/getAmi:getAmi"),
            Some("aws".into())
        );
        assert_eq!(
            ProviderManager::package_name_from_token("pulumi:pulumi:getStack"),
            None
        );
    }
}
