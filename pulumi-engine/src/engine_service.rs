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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{drain, new_collector, EngineEvent};
    use crate::state::EngineState;
    use pulumirpc::engine_server::Engine;
    use tonic::Request;

    fn make_engine(events: Option<EventCollector>) -> EngineServiceImpl {
        let state = EngineState::new("test".into(), "dev".into());
        EngineServiceImpl::new(state, events)
    }

    // --- root resource ---

    #[tokio::test]
    async fn test_set_and_get_root_resource() {
        let svc = make_engine(None);
        svc.set_root_resource(Request::new(pulumirpc::SetRootResourceRequest {
            urn: "urn:pulumi:dev::proj::pulumi:pulumi:Stack::stack".into(),
        }))
        .await
        .unwrap();
        let resp = svc
            .get_root_resource(Request::new(pulumirpc::GetRootResourceRequest {}))
            .await
            .unwrap();
        assert_eq!(
            resp.into_inner().urn,
            "urn:pulumi:dev::proj::pulumi:pulumi:Stack::stack"
        );
    }

    #[tokio::test]
    async fn test_get_root_resource_initially_empty() {
        let svc = make_engine(None);
        let resp = svc
            .get_root_resource(Request::new(pulumirpc::GetRootResourceRequest {}))
            .await
            .unwrap();
        assert!(resp.into_inner().urn.is_empty());
    }

    // --- log ---

    #[tokio::test]
    async fn test_log_emits_diagnostic_event() {
        let collector = new_collector();
        let svc = make_engine(Some(collector.clone()));
        svc.log(Request::new(pulumirpc::LogRequest {
            severity: 1,
            message: "hello from program".into(),
            urn: "urn:pulumi:dev::proj::pkg:mod:Res::r".into(),
            stream_id: 0,
            ephemeral: false,
        }))
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
            svc.log(Request::new(pulumirpc::LogRequest {
                severity: severity_int,
                message: expected.into(),
                urn: String::new(),
                stream_id: 0,
                ephemeral: false,
            }))
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
        svc.log(Request::new(pulumirpc::LogRequest {
            severity: 99,
            message: "msg".into(),
            urn: String::new(),
            stream_id: 0,
            ephemeral: false,
        }))
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
        svc.log(Request::new(pulumirpc::LogRequest {
            severity: 1,
            message: "msg".into(),
            urn: String::new(),
            stream_id: 0,
            ephemeral: false,
        }))
        .await
        .unwrap();
    }

    // --- require_pulumi_version / start_debugging ---

    #[tokio::test]
    async fn test_require_pulumi_version_accepts_any() {
        let svc = make_engine(None);
        svc.require_pulumi_version(Request::new(pulumirpc::RequirePulumiVersionRequest {
            pulumi_version_range: ">=3.0.0".into(),
        }))
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_start_debugging_is_noop() {
        let svc = make_engine(None);
        svc.start_debugging(Request::new(pulumirpc::StartDebuggingRequest {
            config: None,
            message: "attach debugger".into(),
        }))
        .await
        .unwrap();
    }
}
