//! Implementation of the Engine gRPC service.
//!
//! Handles logging and root resource management for the Pulumi program.

use crate::events::{self, EventCollector};
use crate::pulumirpc;
use crate::state::EngineState;
use tonic::{Request, Response, Status};

pub struct EngineServiceImpl {
    state: EngineState,
    events: Option<EventCollector>,
}

impl EngineServiceImpl {
    pub fn new(state: EngineState, events: Option<EventCollector>) -> Self {
        Self { state, events }
    }
}

#[tonic::async_trait]
impl pulumirpc::engine_server::Engine for EngineServiceImpl {
    async fn log(&self, request: Request<pulumirpc::LogRequest>) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let severity = match req.severity {
            0 => "debug",
            1 => "info",
            2 => "warning",
            3 => "error",
            _ => "info",
        };

        let urn_suffix = if req.urn.is_empty() {
            String::new()
        } else {
            format!(" ({})", req.urn)
        };
        eprintln!("[{}]{urn_suffix} {}", severity.to_uppercase(), req.message);

        if let Some(ev) = &self.events {
            events::emit(
                ev,
                events::EngineEvent::Diagnostic {
                    urn: req.urn.clone(),
                    severity: severity.to_string(),
                    message: req.message.clone(),
                },
            );
        }

        Ok(Response::new(()))
    }

    async fn get_root_resource(
        &self,
        _request: Request<pulumirpc::GetRootResourceRequest>,
    ) -> Result<Response<pulumirpc::GetRootResourceResponse>, Status> {
        let urn = self.state.get_root_urn().await;
        Ok(Response::new(pulumirpc::GetRootResourceResponse { urn }))
    }

    async fn set_root_resource(
        &self,
        request: Request<pulumirpc::SetRootResourceRequest>,
    ) -> Result<Response<pulumirpc::SetRootResourceResponse>, Status> {
        let urn = request.into_inner().urn;
        self.state.set_root_urn(urn).await;
        Ok(Response::new(pulumirpc::SetRootResourceResponse {}))
    }

    async fn start_debugging(
        &self,
        _request: Request<pulumirpc::StartDebuggingRequest>,
    ) -> Result<Response<()>, Status> {
        // No-op for now.
        Ok(Response::new(()))
    }

    async fn require_pulumi_version(
        &self,
        _request: Request<pulumirpc::RequirePulumiVersionRequest>,
    ) -> Result<Response<pulumirpc::RequirePulumiVersionResponse>, Status> {
        // Accept any version for now.
        Ok(Response::new(pulumirpc::RequirePulumiVersionResponse {}))
    }
}
