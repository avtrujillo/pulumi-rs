//! Provider plugin lifecycle management.
//!
//! Defines the [`Provider`] trait for abstracting over provider implementations,
//! and [`GrpcProvider`] which launches real provider plugins as subprocesses and
//! communicates with them over gRPC.
//!
//! [`ProviderManager<P>`] caches provider instances by package name so each
//! provider is launched at most once per engine run.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::Status;
use tonic::transport::Channel;

use crate::error::{Error, Result};
use crate::pulumirpc;

/// Alias for a provider operation result (uses `tonic::Status` as the error type).
pub type ProviderResult<T> = std::result::Result<T, Status>;

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

/// A provider that can perform CRUD operations on cloud resources.
///
/// Each method returns `impl Future + Send` (RPITIT), so implementations
/// produce unboxed, zero-cost futures without requiring `#[async_trait]`.
pub trait Provider: Clone + Send + Sync + 'static {
    /// Launch a new provider instance for the given package name.
    ///
    /// `engine_addr` is the `host:port` of the Engine gRPC server so the
    /// provider plugin can call back (e.g. for logging).
    fn launch(package: &str, engine_addr: &str) -> impl Future<Output = Result<Self>> + Send;

    /// Shut down this provider instance, releasing any resources.
    fn shutdown(&mut self) -> impl Future<Output = ()> + Send;

    /// Create a new resource.
    fn create(
        &mut self,
        req: pulumirpc::CreateRequest,
    ) -> impl Future<Output = ProviderResult<pulumirpc::CreateResponse>> + Send;

    /// Read the current state of an existing resource.
    fn read(
        &mut self,
        req: pulumirpc::ReadRequest,
    ) -> impl Future<Output = ProviderResult<pulumirpc::ReadResponse>> + Send;

    /// Update an existing resource.
    fn update(
        &mut self,
        req: pulumirpc::UpdateRequest,
    ) -> impl Future<Output = ProviderResult<pulumirpc::UpdateResponse>> + Send;

    /// Delete an existing resource.
    fn delete(
        &mut self,
        req: pulumirpc::DeleteRequest,
    ) -> impl Future<Output = ProviderResult<()>> + Send;

    /// Invoke a provider function (read-only).
    fn invoke(
        &mut self,
        req: pulumirpc::InvokeRequest,
    ) -> impl Future<Output = ProviderResult<pulumirpc::InvokeResponse>> + Send;

    /// Call a component method on the provider.
    fn call(
        &mut self,
        req: pulumirpc::CallRequest,
    ) -> impl Future<Output = ProviderResult<pulumirpc::CallResponse>> + Send;
}

// ---------------------------------------------------------------------------
// GrpcProvider
// ---------------------------------------------------------------------------

type GrpcClient = pulumirpc::resource_provider_client::ResourceProviderClient<Channel>;

/// A provider backed by a gRPC connection to a `pulumi-resource-<pkg>` subprocess.
#[derive(Clone)]
pub struct GrpcProvider {
    client: GrpcClient,
    /// Shared handle to the child process; killed on shutdown.
    child: Arc<Mutex<Option<tokio::process::Child>>>,
}

impl Provider for GrpcProvider {
    fn launch(package: &str, engine_addr: &str) -> impl Future<Output = Result<Self>> + Send {
        let package = package.to_string();
        let engine_addr = engine_addr.to_string();
        async move {
            let plugin_name = format!("pulumi-resource-{package}");
            eprintln!("[engine] launching provider plugin: {plugin_name}");

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

            let channel = Channel::from_shared(addr.clone())
                .map_err(|e| Error::Custom(format!("invalid provider address '{addr}': {e}")))?
                .connect()
                .await
                .map_err(|e| {
                    Error::Custom(format!(
                        "failed to connect to provider '{plugin_name}': {e}"
                    ))
                })?;

            let mut client = GrpcClient::new(channel);

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

            Ok(GrpcProvider {
                client,
                child: Arc::new(Mutex::new(Some(child))),
            })
        }
    }

    fn shutdown(&mut self) -> impl Future<Output = ()> + Send {
        let child = self.child.clone();
        async move {
            if let Some(mut child) = child.lock().await.take() {
                let _ = child.kill().await;
            }
        }
    }

    fn create(
        &mut self,
        req: pulumirpc::CreateRequest,
    ) -> impl Future<Output = ProviderResult<pulumirpc::CreateResponse>> + Send {
        let mut client = self.client.clone();
        async move { Ok(client.create(req).await?.into_inner()) }
    }

    fn read(
        &mut self,
        req: pulumirpc::ReadRequest,
    ) -> impl Future<Output = ProviderResult<pulumirpc::ReadResponse>> + Send {
        let mut client = self.client.clone();
        async move { Ok(client.read(req).await?.into_inner()) }
    }

    fn update(
        &mut self,
        req: pulumirpc::UpdateRequest,
    ) -> impl Future<Output = ProviderResult<pulumirpc::UpdateResponse>> + Send {
        let mut client = self.client.clone();
        async move { Ok(client.update(req).await?.into_inner()) }
    }

    fn delete(
        &mut self,
        req: pulumirpc::DeleteRequest,
    ) -> impl Future<Output = ProviderResult<()>> + Send {
        let mut client = self.client.clone();
        async move {
            client.delete(req).await?;
            Ok(())
        }
    }

    fn invoke(
        &mut self,
        req: pulumirpc::InvokeRequest,
    ) -> impl Future<Output = ProviderResult<pulumirpc::InvokeResponse>> + Send {
        let mut client = self.client.clone();
        async move { Ok(client.invoke(req).await?.into_inner()) }
    }

    fn call(
        &mut self,
        req: pulumirpc::CallRequest,
    ) -> impl Future<Output = ProviderResult<pulumirpc::CallResponse>> + Send {
        let mut client = self.client.clone();
        async move { Ok(client.call(req).await?.into_inner()) }
    }
}

// ---------------------------------------------------------------------------
// ProviderManager<P>
// ---------------------------------------------------------------------------

/// Manages a cache of provider instances, launching new ones on demand.
///
/// `P` appears in the inner `HashMap`, so `PhantomData` is not needed — the
/// compiler already tracks the type parameter through `providers`.
#[derive(Clone)]
pub struct ProviderManager<P: Provider> {
    inner: Arc<Mutex<HashMap<String, P>>>,
    engine_addr: Arc<Mutex<String>>,
}

impl<P: Provider> Default for ProviderManager<P> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            engine_addr: Arc::new(Mutex::new(String::new())),
        }
    }
}

impl<P: Provider> ProviderManager<P> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the engine address (called once the engine gRPC server is bound).
    pub async fn set_engine_addr(&self, addr: String) {
        *self.engine_addr.lock().await = addr;
    }

    /// Get or launch a provider for the given package.
    pub async fn get_provider(&self, package: &str) -> Result<P> {
        let mut providers = self.inner.lock().await;
        if let Some(provider) = providers.get(package) {
            return Ok(provider.clone());
        }

        let engine_addr = self.engine_addr.lock().await.clone();
        let provider = P::launch(package, &engine_addr).await?;
        providers.insert(package.to_string(), provider.clone());
        Ok(provider)
    }

    /// Shut down all running provider instances.
    pub async fn shutdown_all(&self) {
        let mut providers = self.inner.lock().await;
        for (name, provider) in providers.iter_mut() {
            eprintln!("[engine] shutting down provider: {name}");
            provider.shutdown().await;
        }
        providers.clear();
    }
}

// ---------------------------------------------------------------------------
// Helpers (not generic)
// ---------------------------------------------------------------------------

/// Extract the package name from a Pulumi type token.
///
/// Type tokens have the form `<pkg>:<module>/<name>:<type>` or `<pkg>:<module>:<type>`.
/// Provider resources have the form `pulumi:providers:<pkg>`.
/// Returns `None` for built-in `pulumi:*` types.
pub fn package_name(type_token: &str) -> Option<String> {
    if let Some(pkg) = type_token.strip_prefix("pulumi:providers:") {
        return Some(pkg.to_string());
    }
    let colon = type_token.find(':')?;
    let pkg = &type_token[..colon];
    if pkg == "pulumi" {
        return None;
    }
    Some(pkg.to_string())
}

/// Extract the package name from an invoke/call function token.
///
/// Function tokens have the form `<pkg>:<module>/<name>:<function>` or `<pkg>:<module>:<function>`.
/// Returns `None` for built-in `pulumi:*` tokens.
pub fn package_name_from_token(token: &str) -> Option<String> {
    let colon = token.find(':')?;
    let pkg = &token[..colon];
    if pkg == "pulumi" {
        return None;
    }
    Some(pkg.to_string())
}

/// Read the provider's gRPC port from its stdout.
///
/// Pulumi provider plugins print their port number on a single line to stdout
/// when they are ready to accept connections.
async fn read_provider_port(stdout: tokio::process::ChildStdout) -> Result<u16> {
    use tokio::io::AsyncBufReadExt;
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut line = String::new();

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
    pulumi_core::serde::json_to_struct(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_name_from_type_token() {
        assert_eq!(package_name("aws:s3/bucket:Bucket"), Some("aws".into()));
        assert_eq!(
            package_name("azure-native:storage:StorageAccount"),
            Some("azure-native".into())
        );
        assert_eq!(package_name("pulumi:providers:aws"), Some("aws".into()));
        assert_eq!(package_name("pulumi:pulumi:Stack"), None);
    }

    #[test]
    fn test_package_name_from_function_token() {
        assert_eq!(
            package_name_from_token("aws:ec2/getAmi:getAmi"),
            Some("aws".into())
        );
        assert_eq!(package_name_from_token("pulumi:pulumi:getStack"), None);
    }
}
