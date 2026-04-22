//! Abstractions over the gRPC connections to the Pulumi engine and resource monitor.
//!
//! [`MonitorConnection`] and [`EngineConnection`] define the operations that
//! the SDK needs from the engine, allowing concrete gRPC clients to be swapped
//! out for mocks in tests.
//!
//! Trait methods use RPITIT (return-position `impl Trait` in traits) for
//! zero-cost async dispatch — no boxing, no vtables.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

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

/// A recorded `invoke` call captured by [`MockMonitor`] during testing.
#[derive(Debug, Clone)]
pub struct InvokeRecording {
    /// The function token (e.g. `"aws:ec2/getAmi:getAmi"`).
    pub token: String,
    /// The call arguments as JSON.
    pub args: serde_json::Value,
}

/// A recorded component method call captured by [`MockMonitor`] during testing.
#[derive(Debug, Clone)]
pub struct MethodCallRecording {
    /// The method token.
    pub token: String,
    /// The call arguments as JSON.
    pub args: serde_json::Value,
}

/// A log message recorded by [`MockEngine`] during testing.
#[derive(Debug, Clone)]
pub struct LogRecord {
    /// The severity level: 0=debug, 1=info, 2=warning, 3=error.
    pub severity: i32,
    /// The log message.
    pub message: String,
    /// The URN of the resource that logged the message, if any.
    pub urn: String,
}

/// A resource registration captured by [`MockMonitor`] during testing.
#[derive(Debug, Clone)]
pub struct ResourceRegistration {
    /// The Pulumi type token (e.g. `"aws:s3/bucket:Bucket"`).
    pub type_token: String,
    /// The logical resource name.
    pub name: String,
    /// The input properties as JSON.
    pub inputs: serde_json::Value,
    /// Whether this is a custom (leaf) resource.
    pub custom: bool,
    /// The parent resource URN, or empty if none.
    pub parent: String,
    /// URNs of resources this resource depends on.
    pub depends_on: Vec<String>,
    /// Whether the resource is protected from deletion.
    pub protect: bool,
}

/// A mock [`MonitorConnection`] that records calls and returns configurable responses.
///
/// By default, echoes inputs back as outputs and returns a synthetic URN for
/// `register_resource`. Use [`crate::test_support::TestContextBuilder`] to
/// configure per-resource canned responses, error injection, and preview mode.
#[derive(Clone)]
pub struct MockMonitor {
    project: String,
    stack: String,
    /// Canned outputs keyed by (type_token, name). Immutable after construction.
    responses: Arc<HashMap<(String, String), serde_json::Value>>,
    /// Error messages to inject keyed by (type_token, name). Immutable after construction.
    resource_errors: Arc<HashMap<(String, String), String>>,
    /// All register_resource calls, shared across clones.
    recordings: Arc<Mutex<Vec<ResourceRegistration>>>,
    /// All invoke calls, shared across clones.
    invoke_recordings: Arc<Mutex<Vec<InvokeRecording>>>,
    /// All call (component method) calls, shared across clones.
    call_recordings: Arc<Mutex<Vec<MethodCallRecording>>>,
    /// If true, return empty outputs (simulates pulumi preview).
    preview: bool,
}

impl MockMonitor {
    pub fn new(project: impl Into<String>, stack: impl Into<String>) -> Self {
        Self {
            project: project.into(),
            stack: stack.into(),
            responses: Arc::new(HashMap::new()),
            resource_errors: Arc::new(HashMap::new()),
            recordings: Arc::new(Mutex::new(Vec::new())),
            invoke_recordings: Arc::new(Mutex::new(Vec::new())),
            call_recordings: Arc::new(Mutex::new(Vec::new())),
            preview: false,
        }
    }

    /// Constructs a monitor with canned responses, error injection, and preview mode.
    /// Used by [`crate::test_support::TestContextBuilder`].
    pub(crate) fn with_options(
        project: String,
        stack: String,
        responses: HashMap<(String, String), serde_json::Value>,
        resource_errors: HashMap<(String, String), String>,
        preview: bool,
    ) -> Self {
        Self {
            project,
            stack,
            responses: Arc::new(responses),
            resource_errors: Arc::new(resource_errors),
            recordings: Arc::new(Mutex::new(Vec::new())),
            invoke_recordings: Arc::new(Mutex::new(Vec::new())),
            call_recordings: Arc::new(Mutex::new(Vec::new())),
            preview,
        }
    }

    /// Constructs a monitor with canned responses and preview mode (no error injection).
    /// Used by [`crate::test_support::TestContextBuilder`].
    pub(crate) fn with_responses(
        project: String,
        stack: String,
        responses: HashMap<(String, String), serde_json::Value>,
        preview: bool,
    ) -> Self {
        Self::with_options(project, stack, responses, HashMap::new(), preview)
    }

    fn make_urn(&self, resource_type: &str, name: &str) -> String {
        format!(
            "urn:pulumi:{}::{}::{}::{}",
            self.stack, self.project, resource_type, name
        )
    }

    /// Returns all resource registrations recorded so far.
    pub fn recorded_registrations(&self) -> Vec<ResourceRegistration> {
        self.recordings.lock().unwrap().clone()
    }

    /// Returns all `invoke` calls recorded so far.
    pub fn recorded_invocations(&self) -> Vec<InvokeRecording> {
        self.invoke_recordings.lock().unwrap().clone()
    }

    /// Returns all component method `call` calls recorded so far.
    pub fn recorded_calls(&self) -> Vec<MethodCallRecording> {
        self.call_recordings.lock().unwrap().clone()
    }
}

impl MonitorConnection for MockMonitor {
    async fn register_resource(
        &self,
        req: pulumirpc::RegisterResourceRequest,
    ) -> Result<pulumirpc::RegisterResourceResponse> {
        let key = (req.r#type.clone(), req.name.clone());
        if let Some(err_msg) = self.resource_errors.get(&key) {
            return Err(crate::error::Error::Custom(format!(
                "injected error for {}/{}: {err_msg}",
                req.r#type, req.name
            )));
        }

        let inputs_json = req
            .object
            .as_ref()
            .map(crate::serde::struct_to_json)
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));

        self.recordings.lock().unwrap().push(ResourceRegistration {
            type_token: req.r#type.clone(),
            name: req.name.clone(),
            inputs: inputs_json,
            custom: req.custom,
            parent: req.parent.clone(),
            depends_on: req.dependencies.clone(),
            protect: req.protect.unwrap_or(false),
        });

        let urn = self.make_urn(&req.r#type, &req.name);
        let id = if req.custom {
            format!("mock-id-{}", req.name)
        } else {
            String::new()
        };

        let object = if self.preview {
            Some(prost_types::Struct::default())
        } else {
            let key = (req.r#type.clone(), req.name.clone());
            match self.responses.get(&key) {
                Some(json) => Some(crate::serde::json_to_struct(json)),
                None => req.object,
            }
        };

        Ok(pulumirpc::RegisterResourceResponse {
            urn,
            id,
            object,
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
        req: pulumirpc::ResourceInvokeRequest,
    ) -> Result<pulumirpc::InvokeResponse> {
        let args = req
            .args
            .as_ref()
            .map(crate::serde::struct_to_json)
            .unwrap_or_default();
        self.invoke_recordings.lock().unwrap().push(InvokeRecording {
            token: req.tok.clone(),
            args,
        });
        Ok(pulumirpc::InvokeResponse {
            r#return: Some(prost_types::Struct {
                fields: Default::default(),
            }),
            failures: vec![],
        })
    }

    async fn call(&self, req: pulumirpc::ResourceCallRequest) -> Result<pulumirpc::CallResponse> {
        let args = req
            .args
            .as_ref()
            .map(crate::serde::struct_to_json)
            .unwrap_or_default();
        self.call_recordings.lock().unwrap().push(MethodCallRecording {
            token: req.tok.clone(),
            args,
        });
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

/// A mock [`EngineConnection`] that records log calls and manages root resource state.
#[derive(Clone)]
pub struct MockEngine {
    root_urn: std::sync::Arc<tokio::sync::Mutex<String>>,
    /// All log calls recorded, shared across clones.
    logs: Arc<Mutex<Vec<LogRecord>>>,
}

impl MockEngine {
    pub fn new() -> Self {
        Self {
            root_urn: std::sync::Arc::new(tokio::sync::Mutex::new(String::new())),
            logs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns all log messages recorded so far.
    pub fn recorded_logs(&self) -> Vec<LogRecord> {
        self.logs.lock().unwrap().clone()
    }
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineConnection for MockEngine {
    async fn log(&self, req: pulumirpc::LogRequest) -> Result<()> {
        self.logs.lock().unwrap().push(LogRecord {
            severity: req.severity,
            message: req.message,
            urn: req.urn,
        });
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::serde::struct_to_json;

    fn reg_req(type_token: &str, name: &str, custom: bool) -> pulumirpc::RegisterResourceRequest {
        pulumirpc::RegisterResourceRequest {
            r#type: type_token.into(),
            name: name.into(),
            custom,
            ..Default::default()
        }
    }

    // --- MockMonitor ---

    #[tokio::test]
    async fn test_mock_monitor_records_type_name_custom() {
        let m = MockMonitor::new("proj", "dev");
        m.register_resource(reg_req("test:a:A", "my-res", true)).await.unwrap();
        let recs = m.recorded_registrations();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].type_token, "test:a:A");
        assert_eq!(recs[0].name, "my-res");
        assert!(recs[0].custom);
    }

    #[tokio::test]
    async fn test_mock_monitor_urn_format() {
        let m = MockMonitor::new("myproj", "staging");
        let resp = m.register_resource(reg_req("test:idx:Res", "r1", true)).await.unwrap();
        assert_eq!(resp.urn, "urn:pulumi:staging::myproj::test:idx:Res::r1");
    }

    #[tokio::test]
    async fn test_mock_monitor_custom_resource_gets_id() {
        let m = MockMonitor::new("p", "s");
        let resp = m.register_resource(reg_req("test:t:T", "foo", true)).await.unwrap();
        assert_eq!(resp.id, "mock-id-foo");
    }

    #[tokio::test]
    async fn test_mock_monitor_component_gets_empty_id() {
        let m = MockMonitor::new("p", "s");
        let resp = m.register_resource(reg_req("test:t:C", "comp", false)).await.unwrap();
        assert!(resp.id.is_empty());
    }

    #[tokio::test]
    async fn test_mock_monitor_records_parent() {
        let m = MockMonitor::new("p", "s");
        let req = pulumirpc::RegisterResourceRequest {
            r#type: "test:t:T".into(),
            name: "child".into(),
            parent: "urn:pulumi:s::p::test:t:P::root".into(),
            custom: true,
            ..Default::default()
        };
        m.register_resource(req).await.unwrap();
        assert_eq!(m.recorded_registrations()[0].parent, "urn:pulumi:s::p::test:t:P::root");
    }

    #[tokio::test]
    async fn test_mock_monitor_records_depends_on() {
        let m = MockMonitor::new("p", "s");
        let req = pulumirpc::RegisterResourceRequest {
            r#type: "test:t:T".into(),
            name: "r".into(),
            custom: true,
            dependencies: vec!["urn:a".into(), "urn:b".into()],
            ..Default::default()
        };
        m.register_resource(req).await.unwrap();
        assert_eq!(m.recorded_registrations()[0].depends_on, ["urn:a", "urn:b"]);
    }

    #[tokio::test]
    async fn test_mock_monitor_records_protect() {
        let m = MockMonitor::new("p", "s");
        let req = pulumirpc::RegisterResourceRequest {
            r#type: "test:t:T".into(),
            name: "r".into(),
            custom: true,
            protect: Some(true),
            ..Default::default()
        };
        m.register_resource(req).await.unwrap();
        assert!(m.recorded_registrations()[0].protect);
    }

    #[tokio::test]
    async fn test_mock_monitor_preview_returns_empty_outputs() {
        let m = MockMonitor::with_responses("p".into(), "s".into(), HashMap::new(), true);
        let req = pulumirpc::RegisterResourceRequest {
            r#type: "test:t:T".into(),
            name: "r".into(),
            custom: true,
            object: Some(prost_types::Struct::default()),
            ..Default::default()
        };
        let resp = m.register_resource(req).await.unwrap();
        assert!(resp.object.unwrap_or_default().fields.is_empty());
    }

    #[tokio::test]
    async fn test_mock_monitor_canned_response() {
        let mut responses = HashMap::new();
        responses.insert(
            ("test:t:T".into(), "r".into()),
            serde_json::json!({ "arn": "arn:test:::r" }),
        );
        let m = MockMonitor::with_responses("p".into(), "s".into(), responses, false);
        let req = reg_req("test:t:T", "r", true);
        let resp = m.register_resource(req).await.unwrap();
        let json = struct_to_json(&resp.object.unwrap());
        assert_eq!(json["arn"], "arn:test:::r");
    }

    #[tokio::test]
    async fn test_mock_monitor_no_canned_response_echoes_inputs() {
        let m = MockMonitor::new("p", "s");
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "name".into(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::StringValue("my-name".into())),
            },
        );
        let req = pulumirpc::RegisterResourceRequest {
            r#type: "test:t:T".into(),
            name: "r".into(),
            custom: true,
            object: Some(prost_types::Struct { fields }),
            ..Default::default()
        };
        let resp = m.register_resource(req).await.unwrap();
        let json = struct_to_json(&resp.object.unwrap());
        assert_eq!(json["name"], "my-name");
    }

    #[tokio::test]
    async fn test_mock_monitor_clone_shares_recordings() {
        let m1 = MockMonitor::new("p", "s");
        let m2 = m1.clone();
        m2.register_resource(reg_req("test:t:T", "r", true)).await.unwrap();
        assert_eq!(m1.recorded_registrations().len(), 1);
    }

    #[tokio::test]
    async fn test_mock_monitor_invoke_returns_empty_success() {
        let m = MockMonitor::new("p", "s");
        let resp = m
            .invoke(pulumirpc::ResourceInvokeRequest {
                tok: "test:fn:fn".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(resp.failures.is_empty());
        assert!(resp.r#return.is_some());
    }

    #[tokio::test]
    async fn test_mock_monitor_call_returns_empty_success() {
        let m = MockMonitor::new("p", "s");
        let resp = m
            .call(pulumirpc::ResourceCallRequest {
                tok: "test:m:m".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(resp.failures.is_empty());
        assert!(resp.r#return.is_some());
    }

    #[tokio::test]
    async fn test_mock_monitor_read_resource_builds_urn_and_echoes_props() {
        let m = MockMonitor::new("myproj", "dev");
        let resp = m
            .read_resource(pulumirpc::ReadResourceRequest {
                r#type: "test:t:T".into(),
                name: "r".into(),
                id: "existing-id".into(),
                properties: Some(prost_types::Struct::default()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(resp.urn, "urn:pulumi:dev::myproj::test:t:T::r");
        assert!(resp.properties.is_some());
    }

    #[tokio::test]
    async fn test_mock_monitor_supports_feature_always_true() {
        let m = MockMonitor::new("p", "s");
        assert!(m.supports_feature("outputValues").await.unwrap());
        assert!(m.supports_feature("anything").await.unwrap());
    }

    // --- MockEngine ---

    #[tokio::test]
    async fn test_mock_engine_initial_root_is_empty() {
        let e = MockEngine::new();
        let resp = e
            .get_root_resource(pulumirpc::GetRootResourceRequest {})
            .await
            .unwrap();
        assert!(resp.urn.is_empty());
    }

    #[tokio::test]
    async fn test_mock_engine_set_and_get_root_resource() {
        let e = MockEngine::new();
        e.set_root_resource(pulumirpc::SetRootResourceRequest {
            urn: "urn:pulumi:dev::p::pulumi:pulumi:Stack::s".into(),
        })
        .await
        .unwrap();
        let resp = e
            .get_root_resource(pulumirpc::GetRootResourceRequest {})
            .await
            .unwrap();
        assert_eq!(resp.urn, "urn:pulumi:dev::p::pulumi:pulumi:Stack::s");
    }

    #[tokio::test]
    async fn test_mock_engine_clone_shares_root_urn() {
        let e1 = MockEngine::new();
        let e2 = e1.clone();
        e1.set_root_resource(pulumirpc::SetRootResourceRequest {
            urn: "shared-urn".into(),
        })
        .await
        .unwrap();
        let resp = e2
            .get_root_resource(pulumirpc::GetRootResourceRequest {})
            .await
            .unwrap();
        assert_eq!(resp.urn, "shared-urn");
    }

    #[tokio::test]
    async fn test_mock_engine_log_records_message() {
        let e = MockEngine::new();
        e.log(pulumirpc::LogRequest {
            severity: 3,
            message: "something went wrong".into(),
            urn: "urn:pulumi:dev::p::pkg:mod:R::r".into(),
            stream_id: 0,
            ephemeral: false,
        })
        .await
        .unwrap();
        let logs = e.recorded_logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].severity, 3);
        assert_eq!(logs[0].message, "something went wrong");
        assert_eq!(logs[0].urn, "urn:pulumi:dev::p::pkg:mod:R::r");
    }

    #[tokio::test]
    async fn test_mock_engine_clone_shares_logs() {
        let e1 = MockEngine::new();
        let e2 = e1.clone();
        e1.log(pulumirpc::LogRequest {
            severity: 1,
            message: "msg".into(),
            urn: String::new(),
            stream_id: 0,
            ephemeral: false,
        })
        .await
        .unwrap();
        assert_eq!(e2.recorded_logs().len(), 1);
    }

    // --- MockMonitor invoke/call recording ---

    #[tokio::test]
    async fn test_mock_monitor_invoke_records_token_and_args() {
        let m = MockMonitor::new("p", "s");
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "filters".into(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::StringValue("x86_64".into())),
            },
        );
        m.invoke(pulumirpc::ResourceInvokeRequest {
            tok: "aws:ec2/getAmi:getAmi".into(),
            args: Some(prost_types::Struct { fields }),
            ..Default::default()
        })
        .await
        .unwrap();
        let invocations = m.recorded_invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].token, "aws:ec2/getAmi:getAmi");
        assert_eq!(invocations[0].args["filters"], "x86_64");
    }

    #[tokio::test]
    async fn test_mock_monitor_call_records_token() {
        let m = MockMonitor::new("p", "s");
        m.call(pulumirpc::ResourceCallRequest {
            tok: "my:component:MyMethod".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        let calls = m.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].token, "my:component:MyMethod");
    }

    #[tokio::test]
    async fn test_mock_monitor_invoke_clone_shares_recordings() {
        let m1 = MockMonitor::new("p", "s");
        let m2 = m1.clone();
        m2.invoke(pulumirpc::ResourceInvokeRequest {
            tok: "aws:fn:fn".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(m1.recorded_invocations().len(), 1);
    }

    // --- Error injection ---

    #[tokio::test]
    async fn test_mock_monitor_injected_error_returned() {
        let mut errors = HashMap::new();
        errors.insert(("test:t:T".into(), "bad-res".into()), "simulated failure".into());
        let m = MockMonitor::with_options("p".into(), "s".into(), HashMap::new(), errors, false);
        let err = m
            .register_resource(reg_req("test:t:T", "bad-res", true))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("simulated failure"));
    }

    #[tokio::test]
    async fn test_mock_monitor_injected_error_only_for_matching_resource() {
        let mut errors = HashMap::new();
        errors.insert(("test:t:T".into(), "bad-res".into()), "fail".into());
        let m = MockMonitor::with_options("p".into(), "s".into(), HashMap::new(), errors, false);
        // Different name — should succeed
        let resp = m.register_resource(reg_req("test:t:T", "good-res", true)).await;
        assert!(resp.is_ok());
    }
}
