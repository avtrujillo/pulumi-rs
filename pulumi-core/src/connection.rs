//! Abstractions over the gRPC connections to the Pulumi engine and resource monitor.
//!
//! [`MonitorConnection`] and [`EngineConnection`] define the operations that
//! the SDK needs from the engine, allowing concrete gRPC clients to be swapped
//! out for mocks in tests.
//!
//! Trait methods use RPITIT (return-position `impl Trait` in traits) for
//! zero-cost async dispatch — no boxing, no vtables.

use std::future::Future;

use tonic::transport::Channel;

use crate::error::Result;
use crate::proto::pulumirpc;

// ---------------------------------------------------------------------------
// MonitorConnection
// ---------------------------------------------------------------------------

/// Abstraction over the ResourceMonitor gRPC service.
///
/// All methods take `&self` because the underlying tonic client is cheaply
/// cloneable (it wraps an `Arc`'d channel). Methods return `impl Future`
/// via RPITIT for zero-cost monomorphized dispatch.
pub trait MonitorConnection: Clone + Send + Sync + 'static {
    fn register_resource(
        &self,
        req: pulumirpc::RegisterResourceRequest,
    ) -> impl Future<Output = Result<pulumirpc::RegisterResourceResponse>> + Send;

    fn register_resource_outputs(
        &self,
        req: pulumirpc::RegisterResourceOutputsRequest,
    ) -> impl Future<Output = Result<()>> + Send;

    fn read_resource(
        &self,
        req: pulumirpc::ReadResourceRequest,
    ) -> impl Future<Output = Result<pulumirpc::ReadResourceResponse>> + Send;

    fn invoke(
        &self,
        req: pulumirpc::ResourceInvokeRequest,
    ) -> impl Future<Output = Result<pulumirpc::InvokeResponse>> + Send;

    fn call(
        &self,
        req: pulumirpc::ResourceCallRequest,
    ) -> impl Future<Output = Result<pulumirpc::CallResponse>> + Send;

    fn register_stack_transform(
        &self,
        req: pulumirpc::Callback,
    ) -> impl Future<Output = Result<()>> + Send;

    fn supports_feature(&self, feature: &str) -> impl Future<Output = Result<bool>> + Send;
}

// ---------------------------------------------------------------------------
// EngineConnection
// ---------------------------------------------------------------------------

/// Abstraction over the Engine gRPC service.
pub trait EngineConnection: Clone + Send + Sync + 'static {
    fn log(&self, req: pulumirpc::LogRequest) -> impl Future<Output = Result<()>> + Send;

    fn set_root_resource(
        &self,
        req: pulumirpc::SetRootResourceRequest,
    ) -> impl Future<Output = Result<pulumirpc::SetRootResourceResponse>> + Send;

    fn get_root_resource(
        &self,
        req: pulumirpc::GetRootResourceRequest,
    ) -> impl Future<Output = Result<pulumirpc::GetRootResourceResponse>> + Send;
}

// ---------------------------------------------------------------------------
// GrpcMonitor
// ---------------------------------------------------------------------------

/// [`MonitorConnection`] backed by a real gRPC connection.
#[derive(Clone)]
pub struct GrpcMonitor {
    client: pulumirpc::resource_monitor_client::ResourceMonitorClient<Channel>,
}

impl GrpcMonitor {
    pub fn new(client: pulumirpc::resource_monitor_client::ResourceMonitorClient<Channel>) -> Self {
        Self { client }
    }
}

impl MonitorConnection for GrpcMonitor {
    async fn register_resource(
        &self,
        req: pulumirpc::RegisterResourceRequest,
    ) -> Result<pulumirpc::RegisterResourceResponse> {
        let mut client = self.client.clone();
        Ok(client.register_resource(req).await?.into_inner())
    }

    async fn register_resource_outputs(
        &self,
        req: pulumirpc::RegisterResourceOutputsRequest,
    ) -> Result<()> {
        let mut client = self.client.clone();
        client.register_resource_outputs(req).await?;
        Ok(())
    }

    async fn read_resource(
        &self,
        req: pulumirpc::ReadResourceRequest,
    ) -> Result<pulumirpc::ReadResourceResponse> {
        let mut client = self.client.clone();
        Ok(client.read_resource(req).await?.into_inner())
    }

    async fn invoke(
        &self,
        req: pulumirpc::ResourceInvokeRequest,
    ) -> Result<pulumirpc::InvokeResponse> {
        let mut client = self.client.clone();
        Ok(client.invoke(req).await?.into_inner())
    }

    async fn call(&self, req: pulumirpc::ResourceCallRequest) -> Result<pulumirpc::CallResponse> {
        let mut client = self.client.clone();
        Ok(client.call(req).await?.into_inner())
    }

    async fn register_stack_transform(&self, req: pulumirpc::Callback) -> Result<()> {
        let mut client = self.client.clone();
        client.register_stack_transform(req).await?;
        Ok(())
    }

    async fn supports_feature(&self, feature: &str) -> Result<bool> {
        let mut client = self.client.clone();
        let resp = client
            .supports_feature(pulumirpc::SupportsFeatureRequest {
                id: feature.to_string(),
            })
            .await?;
        Ok(resp.into_inner().has_support)
    }
}

// ---------------------------------------------------------------------------
// GrpcEngine
// ---------------------------------------------------------------------------

/// [`EngineConnection`] backed by a real gRPC connection.
#[derive(Clone)]
pub struct GrpcEngine {
    client: pulumirpc::engine_client::EngineClient<Channel>,
}

impl GrpcEngine {
    pub fn new(client: pulumirpc::engine_client::EngineClient<Channel>) -> Self {
        Self { client }
    }
}

impl EngineConnection for GrpcEngine {
    async fn log(&self, req: pulumirpc::LogRequest) -> Result<()> {
        let mut client = self.client.clone();
        client.log(req).await?;
        Ok(())
    }

    async fn set_root_resource(
        &self,
        req: pulumirpc::SetRootResourceRequest,
    ) -> Result<pulumirpc::SetRootResourceResponse> {
        let mut client = self.client.clone();
        Ok(client.set_root_resource(req).await?.into_inner())
    }

    async fn get_root_resource(
        &self,
        req: pulumirpc::GetRootResourceRequest,
    ) -> Result<pulumirpc::GetRootResourceResponse> {
        let mut client = self.client.clone();
        Ok(client.get_root_resource(req).await?.into_inner())
    }
}

// ---------------------------------------------------------------------------
// MockMonitor — for testing without a running engine
// ---------------------------------------------------------------------------

/// A mock [`MonitorConnection`] that returns configurable responses.
///
/// By default, returns a synthetic URN and empty outputs for `register_resource`,
/// and succeeds with no-ops for other operations.
///
/// # Example
///
/// ```ignore
/// use pulumi_core::connection::{MockMonitor, MockEngine};
/// use pulumi_core::context::{Context, Settings};
///
/// let monitor = MockMonitor::new("test-project", "dev");
/// let engine = MockEngine::new();
/// let settings = Settings { /* ... */ };
/// let ctx = Context::for_testing(monitor, engine, settings);
/// ```
#[derive(Clone)]
pub struct MockMonitor {
    project: String,
    stack: String,
}

impl MockMonitor {
    pub fn new(project: impl Into<String>, stack: impl Into<String>) -> Self {
        Self {
            project: project.into(),
            stack: stack.into(),
        }
    }

    fn make_urn(&self, resource_type: &str, name: &str) -> String {
        format!(
            "urn:pulumi:{}::{}::{}::{}",
            self.stack, self.project, resource_type, name
        )
    }
}

impl MonitorConnection for MockMonitor {
    async fn register_resource(
        &self,
        req: pulumirpc::RegisterResourceRequest,
    ) -> Result<pulumirpc::RegisterResourceResponse> {
        let urn = self.make_urn(&req.r#type, &req.name);
        let id = if req.custom {
            format!("mock-id-{}", req.name)
        } else {
            String::new()
        };
        Ok(pulumirpc::RegisterResourceResponse {
            urn,
            id,
            object: req.object,
            stable: true,
            stables: vec![],
            property_dependencies: Default::default(),
            result: 0,
        })
    }

    async fn register_resource_outputs(
        &self,
        _req: pulumirpc::RegisterResourceOutputsRequest,
    ) -> Result<()> {
        Ok(())
    }

    async fn read_resource(
        &self,
        req: pulumirpc::ReadResourceRequest,
    ) -> Result<pulumirpc::ReadResourceResponse> {
        let urn = self.make_urn(&req.r#type, &req.name);
        Ok(pulumirpc::ReadResourceResponse {
            urn,
            properties: req.properties,
        })
    }

    async fn invoke(
        &self,
        _req: pulumirpc::ResourceInvokeRequest,
    ) -> Result<pulumirpc::InvokeResponse> {
        Ok(pulumirpc::InvokeResponse {
            r#return: Some(prost_types::Struct {
                fields: Default::default(),
            }),
            failures: vec![],
        })
    }

    async fn call(&self, _req: pulumirpc::ResourceCallRequest) -> Result<pulumirpc::CallResponse> {
        Ok(pulumirpc::CallResponse {
            r#return: Some(prost_types::Struct {
                fields: Default::default(),
            }),
            failures: vec![],
            return_dependencies: Default::default(),
        })
    }

    async fn register_stack_transform(&self, _req: pulumirpc::Callback) -> Result<()> {
        Ok(())
    }

    async fn supports_feature(&self, _feature: &str) -> Result<bool> {
        Ok(true)
    }
}

/// A mock [`EngineConnection`] that accepts all operations as no-ops.
#[derive(Clone)]
pub struct MockEngine {
    root_urn: std::sync::Arc<tokio::sync::Mutex<String>>,
}

impl MockEngine {
    pub fn new() -> Self {
        Self {
            root_urn: std::sync::Arc::new(tokio::sync::Mutex::new(String::new())),
        }
    }
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineConnection for MockEngine {
    async fn log(&self, _req: pulumirpc::LogRequest) -> Result<()> {
        Ok(())
    }

    async fn set_root_resource(
        &self,
        req: pulumirpc::SetRootResourceRequest,
    ) -> Result<pulumirpc::SetRootResourceResponse> {
        *self.root_urn.lock().await = req.urn;
        Ok(pulumirpc::SetRootResourceResponse {})
    }

    async fn get_root_resource(
        &self,
        _req: pulumirpc::GetRootResourceRequest,
    ) -> Result<pulumirpc::GetRootResourceResponse> {
        let urn = self.root_urn.lock().await.clone();
        Ok(pulumirpc::GetRootResourceResponse { urn })
    }
}
