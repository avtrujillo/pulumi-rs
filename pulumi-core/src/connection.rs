//! Abstractions over the gRPC connections to the Pulumi engine and resource monitor.
//!
//! [`MonitorConnection`] and [`EngineConnection`] define the operations that
//! the SDK needs from the engine, allowing concrete gRPC clients to be swapped
//! out for mocks in tests.

use std::future::Future;
use std::pin::Pin;

use tonic::transport::Channel;

use crate::error::Result;
use crate::proto::pulumirpc;

/// A boxed future that is `Send` — used for object-safe async trait methods.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ---------------------------------------------------------------------------
// MonitorConnection
// ---------------------------------------------------------------------------

/// Abstraction over the ResourceMonitor gRPC service.
///
/// All methods take `&self` because the underlying tonic client is cheaply
/// cloneable (it wraps an `Arc`'d channel). Methods return boxed futures
/// so the trait is object-safe (`dyn MonitorConnection`).
pub trait MonitorConnection: Send + Sync + 'static {
    fn register_resource(
        &self,
        req: pulumirpc::RegisterResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::RegisterResourceResponse>>;

    fn register_resource_outputs(
        &self,
        req: pulumirpc::RegisterResourceOutputsRequest,
    ) -> BoxFuture<'_, Result<()>>;

    fn read_resource(
        &self,
        req: pulumirpc::ReadResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::ReadResourceResponse>>;

    fn invoke(
        &self,
        req: pulumirpc::ResourceInvokeRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::InvokeResponse>>;

    fn call(
        &self,
        req: pulumirpc::ResourceCallRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::CallResponse>>;

    fn register_stack_transform(&self, req: pulumirpc::Callback) -> BoxFuture<'_, Result<()>>;

    fn supports_feature(&self, feature: &str) -> BoxFuture<'_, Result<bool>>;
}

// ---------------------------------------------------------------------------
// EngineConnection
// ---------------------------------------------------------------------------

/// Abstraction over the Engine gRPC service.
pub trait EngineConnection: Send + Sync + 'static {
    fn log(&self, req: pulumirpc::LogRequest) -> BoxFuture<'_, Result<()>>;

    fn set_root_resource(
        &self,
        req: pulumirpc::SetRootResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::SetRootResourceResponse>>;

    fn get_root_resource(
        &self,
        req: pulumirpc::GetRootResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::GetRootResourceResponse>>;
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
    fn register_resource(
        &self,
        req: pulumirpc::RegisterResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::RegisterResourceResponse>> {
        let mut client = self.client.clone();
        Box::pin(async move { Ok(client.register_resource(req).await?.into_inner()) })
    }

    fn register_resource_outputs(
        &self,
        req: pulumirpc::RegisterResourceOutputsRequest,
    ) -> BoxFuture<'_, Result<()>> {
        let mut client = self.client.clone();
        Box::pin(async move {
            client.register_resource_outputs(req).await?;
            Ok(())
        })
    }

    fn read_resource(
        &self,
        req: pulumirpc::ReadResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::ReadResourceResponse>> {
        let mut client = self.client.clone();
        Box::pin(async move { Ok(client.read_resource(req).await?.into_inner()) })
    }

    fn invoke(
        &self,
        req: pulumirpc::ResourceInvokeRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::InvokeResponse>> {
        let mut client = self.client.clone();
        Box::pin(async move { Ok(client.invoke(req).await?.into_inner()) })
    }

    fn call(
        &self,
        req: pulumirpc::ResourceCallRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::CallResponse>> {
        let mut client = self.client.clone();
        Box::pin(async move { Ok(client.call(req).await?.into_inner()) })
    }

    fn register_stack_transform(&self, req: pulumirpc::Callback) -> BoxFuture<'_, Result<()>> {
        let mut client = self.client.clone();
        Box::pin(async move {
            client.register_stack_transform(req).await?;
            Ok(())
        })
    }

    fn supports_feature(&self, feature: &str) -> BoxFuture<'_, Result<bool>> {
        let mut client = self.client.clone();
        let req = pulumirpc::SupportsFeatureRequest {
            id: feature.to_string(),
        };
        Box::pin(async move { Ok(client.supports_feature(req).await?.into_inner().has_support) })
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
    fn log(&self, req: pulumirpc::LogRequest) -> BoxFuture<'_, Result<()>> {
        let mut client = self.client.clone();
        Box::pin(async move {
            client.log(req).await?;
            Ok(())
        })
    }

    fn set_root_resource(
        &self,
        req: pulumirpc::SetRootResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::SetRootResourceResponse>> {
        let mut client = self.client.clone();
        Box::pin(async move { Ok(client.set_root_resource(req).await?.into_inner()) })
    }

    fn get_root_resource(
        &self,
        req: pulumirpc::GetRootResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::GetRootResourceResponse>> {
        let mut client = self.client.clone();
        Box::pin(async move { Ok(client.get_root_resource(req).await?.into_inner()) })
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
    fn register_resource(
        &self,
        req: pulumirpc::RegisterResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::RegisterResourceResponse>> {
        let urn = self.make_urn(&req.r#type, &req.name);
        let id = if req.custom {
            format!("mock-id-{}", req.name)
        } else {
            String::new()
        };
        Box::pin(async move {
            Ok(pulumirpc::RegisterResourceResponse {
                urn,
                id,
                object: req.object,
                stable: true,
                stables: vec![],
                property_dependencies: Default::default(),
                result: 0,
            })
        })
    }

    fn register_resource_outputs(
        &self,
        _req: pulumirpc::RegisterResourceOutputsRequest,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }

    fn read_resource(
        &self,
        req: pulumirpc::ReadResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::ReadResourceResponse>> {
        let urn = self.make_urn(&req.r#type, &req.name);
        Box::pin(async move {
            Ok(pulumirpc::ReadResourceResponse {
                urn,
                properties: req.properties,
            })
        })
    }

    fn invoke(
        &self,
        _req: pulumirpc::ResourceInvokeRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::InvokeResponse>> {
        Box::pin(async {
            Ok(pulumirpc::InvokeResponse {
                r#return: Some(prost_types::Struct {
                    fields: Default::default(),
                }),
                failures: vec![],
            })
        })
    }

    fn call(
        &self,
        _req: pulumirpc::ResourceCallRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::CallResponse>> {
        Box::pin(async {
            Ok(pulumirpc::CallResponse {
                r#return: Some(prost_types::Struct {
                    fields: Default::default(),
                }),
                failures: vec![],
                return_dependencies: Default::default(),
            })
        })
    }

    fn register_stack_transform(&self, _req: pulumirpc::Callback) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }

    fn supports_feature(&self, _feature: &str) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async { Ok(true) })
    }
}

/// A mock [`EngineConnection`] that accepts all operations as no-ops.
pub struct MockEngine {
    root_urn: tokio::sync::Mutex<String>,
}

impl MockEngine {
    pub fn new() -> Self {
        Self {
            root_urn: tokio::sync::Mutex::new(String::new()),
        }
    }
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineConnection for MockEngine {
    fn log(&self, _req: pulumirpc::LogRequest) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }

    fn set_root_resource(
        &self,
        req: pulumirpc::SetRootResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::SetRootResourceResponse>> {
        Box::pin(async move {
            *self.root_urn.lock().await = req.urn;
            Ok(pulumirpc::SetRootResourceResponse {})
        })
    }

    fn get_root_resource(
        &self,
        _req: pulumirpc::GetRootResourceRequest,
    ) -> BoxFuture<'_, Result<pulumirpc::GetRootResourceResponse>> {
        Box::pin(async {
            let urn = self.root_urn.lock().await.clone();
            Ok(pulumirpc::GetRootResourceResponse { urn })
        })
    }
}
