//! Implementation of the Engine gRPC service.
//!
//! Handles logging and root resource management for the Pulumi program.
//!
//! ## Handler / shim split
//!
//! The actual logic for each RPC lives in `handle_*` methods on the inherent
//! `impl EngineServiceImpl` block. The `tonic` trait `impl` is a thin shim
//! that unwraps `Request<T>`, calls the handler, and wraps the result in
//! `Response<T>`. Tests can call the handlers directly with plain prost types
//! without going through the gRPC layer.

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

    // -----------------------------------------------------------------------
    // Handler methods.
    //
    // These contain the actual logic for each RPC, take plain prost types,
    // and return `Result<T, Status>`. The tonic trait `impl` below is a thin
    // shim that delegates to these.
    // -----------------------------------------------------------------------

    pub(crate) async fn handle_log(&self, req: pulumirpc::LogRequest) -> Result<(), Status> {
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

        Ok(())
    }

    pub(crate) async fn handle_get_root_resource(
        &self,
        _req: pulumirpc::GetRootResourceRequest,
    ) -> Result<pulumirpc::GetRootResourceResponse, Status> {
        let urn = self.state.get_root_urn().await;
        Ok(pulumirpc::GetRootResourceResponse { urn })
    }

    pub(crate) async fn handle_set_root_resource(
        &self,
        req: pulumirpc::SetRootResourceRequest,
    ) -> Result<pulumirpc::SetRootResourceResponse, Status> {
        self.state.set_root_urn(req.urn).await;
        Ok(pulumirpc::SetRootResourceResponse {})
    }

    pub(crate) async fn handle_start_debugging(
        &self,
        _req: pulumirpc::StartDebuggingRequest,
    ) -> Result<(), Status> {
        // No-op for now.
        Ok(())
    }

    pub(crate) async fn handle_require_pulumi_version(
        &self,
        _req: pulumirpc::RequirePulumiVersionRequest,
    ) -> Result<pulumirpc::RequirePulumiVersionResponse, Status> {
        // Accept any version for now.
        Ok(pulumirpc::RequirePulumiVersionResponse {})
    }
}

#[tonic::async_trait]
impl pulumirpc::engine_server::Engine for EngineServiceImpl {
    async fn log(&self, request: Request<pulumirpc::LogRequest>) -> Result<Response<()>, Status> {
        self.handle_log(request.into_inner()).await?;
        Ok(Response::new(()))
    }

    async fn get_root_resource(
        &self,
        request: Request<pulumirpc::GetRootResourceRequest>,
    ) -> Result<Response<pulumirpc::GetRootResourceResponse>, Status> {
        let resp = self.handle_get_root_resource(request.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn set_root_resource(
        &self,
        request: Request<pulumirpc::SetRootResourceRequest>,
    ) -> Result<Response<pulumirpc::SetRootResourceResponse>, Status> {
        let resp = self.handle_set_root_resource(request.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn start_debugging(
        &self,
        request: Request<pulumirpc::StartDebuggingRequest>,
    ) -> Result<Response<()>, Status> {
        self.handle_start_debugging(request.into_inner()).await?;
        Ok(Response::new(()))
    }

    async fn require_pulumi_version(
        &self,
        request: Request<pulumirpc::RequirePulumiVersionRequest>,
    ) -> Result<Response<pulumirpc::RequirePulumiVersionResponse>, Status> {
        let resp = self
            .handle_require_pulumi_version(request.into_inner())
            .await?;
        Ok(Response::new(resp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{drain, new_collector, EngineEvent};
    use crate::state::EngineState;

    fn make_engine(events: Option<EventCollector>) -> EngineServiceImpl {
        let state = EngineState::new("test".into(), "dev".into());
        EngineServiceImpl::new(state, events)
    }

    // --- root resource ---

    #[tokio::test]
    async fn test_set_and_get_root_resource() {
        let svc = make_engine(None);
        svc.handle_set_root_resource(pulumirpc::SetRootResourceRequest {
            urn: "urn:pulumi:dev::proj::pulumi:pulumi:Stack::stack".into(),
        })
        .await
        .unwrap();
        let resp = svc
            .handle_get_root_resource(pulumirpc::GetRootResourceRequest {})
            .await
            .unwrap();
        assert_eq!(
            resp.urn,
            "urn:pulumi:dev::proj::pulumi:pulumi:Stack::stack"
        );
    }

    #[tokio::test]
    async fn test_get_root_resource_initially_empty() {
        let svc = make_engine(None);
        let resp = svc
            .handle_get_root_resource(pulumirpc::GetRootResourceRequest {})
            .await
            .unwrap();
        assert!(resp.urn.is_empty());
    }

    // --- log ---

    #[tokio::test]
    async fn test_log_emits_diagnostic_event() {
        let collector = new_collector();
        let svc = make_engine(Some(collector.clone()));
        svc.handle_log(pulumirpc::LogRequest {
            severity: 1,
            message: "hello from program".into(),
            urn: "urn:pulumi:dev::proj::pkg:mod:Res::r".into(),
            stream_id: 0,
            ephemeral: false,
        })
        .await
        .unwrap();

        let events = drain(&collector);
        assert_eq!(events.len(), 1);
        match &events[0] {
            EngineEvent::Diagnostic { severity, message, urn } => {
                assert_eq!(severity, "info");
                assert_eq!(message, "hello from program");
                assert_eq!(urn, "urn:pulumi:dev::proj::pkg:mod:Res::r");
            }
            other => panic!("expected Diagnostic, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_log_severity_mapping() {
        let collector = new_collector();
        let svc = make_engine(Some(collector.clone()));

        for (severity_int, expected) in [(0, "debug"), (1, "info"), (2, "warning"), (3, "error")] {
            svc.handle_log(pulumirpc::LogRequest {
                severity: severity_int,
                message: expected.into(),
                urn: String::new(),
                stream_id: 0,
                ephemeral: false,
            })
            .await
            .unwrap();
        }

        let events = drain(&collector);
        let severities: Vec<&str> = events
            .iter()
            .filter_map(|ev| {
                if let EngineEvent::Diagnostic { severity, .. } = ev {
                    Some(severity.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(severities, vec!["debug", "info", "warning", "error"]);
    }

    #[tokio::test]
    async fn test_log_unknown_severity_defaults_to_info() {
        let collector = new_collector();
        let svc = make_engine(Some(collector.clone()));
        svc.handle_log(pulumirpc::LogRequest {
            severity: 99,
            message: "msg".into(),
            urn: String::new(),
            stream_id: 0,
            ephemeral: false,
        })
        .await
        .unwrap();
        let events = drain(&collector);
        if let EngineEvent::Diagnostic { severity, .. } = &events[0] {
            assert_eq!(severity, "info");
        }
    }

    #[tokio::test]
    async fn test_log_no_collector_does_not_panic() {
        let svc = make_engine(None);
        svc.handle_log(pulumirpc::LogRequest {
            severity: 1,
            message: "msg".into(),
            urn: String::new(),
            stream_id: 0,
            ephemeral: false,
        })
        .await
        .unwrap();
    }

    // --- require_pulumi_version / start_debugging ---

    #[tokio::test]
    async fn test_require_pulumi_version_accepts_any() {
        let svc = make_engine(None);
        svc.handle_require_pulumi_version(pulumirpc::RequirePulumiVersionRequest {
            pulumi_version_range: ">=3.0.0".into(),
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_start_debugging_is_noop() {
        let svc = make_engine(None);
        svc.handle_start_debugging(pulumirpc::StartDebuggingRequest {
            config: None,
            message: "attach debugger".into(),
        })
        .await
        .unwrap();
    }

    // --- in-process server smoke tests (covers the tonic shim wiring) ---

    #[tokio::test]
    async fn test_smoke_set_and_get_root_resource_via_grpc() {
        use crate::test_utils::TestEngine;

        let harness = TestEngine::new().await;
        let mut client = harness.engine_client().await;

        let stack_urn = "urn:pulumi:dev::test::pulumi:pulumi:Stack::stack";
        client
            .set_root_resource(tonic::Request::new(pulumirpc::SetRootResourceRequest {
                urn: stack_urn.into(),
            }))
            .await
            .unwrap();

        let resp = client
            .get_root_resource(tonic::Request::new(pulumirpc::GetRootResourceRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(resp.urn, stack_urn);

        // The shared state should reflect what we set via gRPC.
        assert_eq!(harness.state.get_root_urn().await, stack_urn);
    }

    #[tokio::test]
    async fn test_smoke_log_via_grpc() {
        use crate::test_utils::TestEngine;

        let harness = TestEngine::new().await;
        let mut client = harness.engine_client().await;

        // No event collector wired in TestEngine; just verifies the call
        // completes successfully through the shim.
        client
            .log(tonic::Request::new(pulumirpc::LogRequest {
                severity: 1,
                message: "hello via grpc".into(),
                urn: String::new(),
                stream_id: 0,
                ephemeral: false,
            }))
            .await
            .unwrap();
    }
}
