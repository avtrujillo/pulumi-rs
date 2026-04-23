//! Shared test utilities for pulumi-engine unit tests.

use crate::error::Result;
use crate::provider::{Provider, ProviderResult};
use crate::pulumirpc;
use std::sync::{Arc, Mutex};

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
