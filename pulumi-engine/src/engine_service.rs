//! Implementation of the Engine gRPC service.
//!
//! Handles logging and root resource management for the Pulumi program.

use crate::pulumirpc;
use crate::state::EngineState;
use tonic::{Request, Response, Status};

pub struct EngineServiceImpl {
    state: EngineState,
}

impl EngineServiceImpl {
    pub fn new(state: EngineState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl pulumirpc::engine_server::Engine for EngineServiceImpl {
    async fn log(&self, request: Request<pulumirpc::LogRequest>) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let severity = match req.severity {
            0 => "DEBUG",
            1 => "INFO",
            2 => "WARN",
            3 => "ERROR",
            _ => "UNKNOWN",
        };

        // For now, just print to stderr like the real engine does.
        let urn_suffix = if req.urn.is_empty() {
            String::new()
        } else {
            format!(" ({})", req.urn)
        };
        eprintln!("[{severity}]{urn_suffix} {}", req.message);

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
