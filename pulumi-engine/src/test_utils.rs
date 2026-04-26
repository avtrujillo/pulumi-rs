//! Shared test utilities for pulumi-engine unit tests.
//!
//! Provides:
//! - [`MockProvider`] — a [`Provider`] implementation that records `delete`
//!   calls and returns canned responses for the other CRUD operations.
//! - [`TestEngine`] — an in-process harness that spawns the ResourceMonitor
//!   and Engine gRPC servers on ephemeral ports and exposes tonic clients,
//!   for tests that exercise the full transport stack (the `tonic` shim +
//!   wire encoding) end-to-end.
//! - [`state_with_prior`] — convenience for building an [`EngineState`] with
//!   pre-populated prior-run resources, so tests can exercise the
//!   create/update/same diff paths without writing checkpoints to disk.

use crate::engine_service::EngineServiceImpl;
use crate::error::Result;
use crate::monitor_service::ResourceMonitorImpl;
use crate::provider::{Provider, ProviderManager, ProviderResult};
use crate::pulumirpc;
use crate::state::{Checkpoint, EngineState, ResourceState};
use std::sync::{Arc, Mutex};
use tonic::transport::{Channel, Server};

/// A minimal [`Provider`] implementation for unit tests.
///
/// Records `delete` calls by URN. All other operations return success with
/// empty/default data. `launch` always succeeds immediately (no subprocess).
#[derive(Clone)]
pub struct MockProvider {
    /// URNs of resources that have been deleted via this provider.
    pub deleted: Arc<Mutex<Vec<String>>>,
    /// If set, the id and properties returned by `read`.
    pub read_response: Option<pulumirpc::ReadResponse>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            deleted: Arc::new(Mutex::new(Vec::new())),
            read_response: None,
        }
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for MockProvider {
    fn launch(
        _package: &str,
        _engine_addr: &str,
    ) -> impl std::future::Future<Output = Result<Self>> + Send {
        async { Ok(MockProvider::new()) }
    }

    fn shutdown(&mut self) -> impl std::future::Future<Output = ()> + Send {
        async {}
    }

    fn create(
        &mut self,
        req: pulumirpc::CreateRequest,
    ) -> impl std::future::Future<Output = ProviderResult<pulumirpc::CreateResponse>> + Send {
        let urn = req.urn.clone();
        async move {
            let name = urn.rsplit("::").next().unwrap_or("resource").to_string();
            Ok(pulumirpc::CreateResponse {
                id: format!("mock-id-{name}"),
                properties: req.properties,
                ..Default::default()
            })
        }
    }

    fn read(
        &mut self,
        req: pulumirpc::ReadRequest,
    ) -> impl std::future::Future<Output = ProviderResult<pulumirpc::ReadResponse>> + Send {
        let response = self.read_response.clone();
        let id = req.id.clone();
        let properties = req.properties.clone();
        async move {
            Ok(response.unwrap_or(pulumirpc::ReadResponse {
                id,
                properties,
                inputs: None,
                ..Default::default()
            }))
        }
    }

    fn update(
        &mut self,
        req: pulumirpc::UpdateRequest,
    ) -> impl std::future::Future<Output = ProviderResult<pulumirpc::UpdateResponse>> + Send {
        let news = req.news.clone();
        async move {
            Ok(pulumirpc::UpdateResponse {
                properties: news,
                ..Default::default()
            })
        }
    }

    fn delete(
        &mut self,
        req: pulumirpc::DeleteRequest,
    ) -> impl std::future::Future<Output = ProviderResult<()>> + Send {
        let deleted = self.deleted.clone();
        let urn = req.urn.clone();
        async move {
            deleted.lock().unwrap().push(urn);
            Ok(())
        }
    }

    fn invoke(
        &mut self,
        _req: pulumirpc::InvokeRequest,
    ) -> impl std::future::Future<Output = ProviderResult<pulumirpc::InvokeResponse>> + Send {
        async {
            Ok(pulumirpc::InvokeResponse {
                r#return: Some(prost_types::Struct::default()),
                failures: vec![],
            })
        }
    }

    fn call(
        &mut self,
        _req: pulumirpc::CallRequest,
    ) -> impl std::future::Future<Output = ProviderResult<pulumirpc::CallResponse>> + Send {
        async {
            Ok(pulumirpc::CallResponse {
                r#return: Some(prost_types::Struct::default()),
                ..Default::default()
            })
        }
    }
}

/// Build an [`EngineState`] pre-populated with the given `prior` resources.
///
/// The resources are written to an in-memory [`Checkpoint`] and loaded via
/// [`EngineState::from_checkpoint`], so they appear under the prior-run map
/// the same way they would after a real `pulumi up` saved state. This is the
/// primary way tests set up the create/update/same diff paths in
/// `register_resource`.
pub(crate) fn state_with_prior(prior: Vec<ResourceState>) -> EngineState {
    let cp = Checkpoint {
        version: 1,
        project: "test".into(),
        stack: "dev".into(),
        resources: prior,
        outputs: serde_json::json!({}),
        secrets_provider: None,
    };
    EngineState::from_checkpoint(&cp)
}

/// In-process harness that runs the ResourceMonitor and Engine gRPC servers
/// on ephemeral local ports. Returns tonic clients connected to those servers
/// plus the shared [`EngineState`] so tests can assert on internal state
/// after RPCs complete.
///
/// Use for tests that need to exercise the `tonic` trait shims and wire
/// encoding (rather than calling handlers directly). For most coverage,
/// prefer calling `handle_*` methods directly — they're faster and let you
/// assert on `EngineState` without a client roundtrip.
///
/// Servers are aborted on `Drop`.
pub(crate) struct TestEngine {
    pub state: EngineState,
    monitor_url: String,
    engine_url: String,
    monitor_handle: tokio::task::JoinHandle<()>,
    engine_handle: tokio::task::JoinHandle<()>,
}

impl TestEngine {
    pub async fn new() -> Self {
        let state = EngineState::new("test".into(), "dev".into());
        let providers: ProviderManager<MockProvider> = ProviderManager::new();

        let monitor_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let engine_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();

        let monitor_url = format!("http://{}", monitor_listener.local_addr().unwrap());
        let engine_url = format!("http://{}", engine_listener.local_addr().unwrap());

        let monitor_svc = ResourceMonitorImpl::new(state.clone(), false, providers, None);
        let monitor_handle = tokio::spawn(async move {
            let _ = Server::builder()
                .add_service(
                    pulumirpc::resource_monitor_server::ResourceMonitorServer::new(monitor_svc),
                )
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                    monitor_listener,
                ))
                .await;
        });

        let engine_svc = EngineServiceImpl::new(state.clone(), None);
        let engine_handle = tokio::spawn(async move {
            let _ = Server::builder()
                .add_service(pulumirpc::engine_server::EngineServer::new(engine_svc))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                    engine_listener,
                ))
                .await;
        });

        Self {
            state,
            monitor_url,
            engine_url,
            monitor_handle,
            engine_handle,
        }
    }

    pub async fn monitor_client(
        &self,
    ) -> pulumirpc::resource_monitor_client::ResourceMonitorClient<Channel> {
        // Retry a few times — the spawned server may not have started accepting yet.
        for _ in 0..20 {
            if let Ok(client) =
                pulumirpc::resource_monitor_client::ResourceMonitorClient::connect(
                    self.monitor_url.clone(),
                )
                .await
            {
                return client;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("failed to connect to in-process ResourceMonitor at {}", self.monitor_url);
    }

    pub async fn engine_client(&self) -> pulumirpc::engine_client::EngineClient<Channel> {
        for _ in 0..20 {
            if let Ok(client) =
                pulumirpc::engine_client::EngineClient::connect(self.engine_url.clone()).await
            {
                return client;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("failed to connect to in-process Engine at {}", self.engine_url);
    }
}

impl Drop for TestEngine {
    fn drop(&mut self) {
        self.monitor_handle.abort();
        self.engine_handle.abort();
    }
}
