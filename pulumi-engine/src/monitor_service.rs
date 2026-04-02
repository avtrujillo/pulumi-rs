//! Implementation of the ResourceMonitor gRPC service.
//!
//! Handles resource registration, invocations, and feature queries from the
//! Pulumi program. Diffs against prior state to determine create vs update.

use crate::diff::{self, ResourceAction};
use crate::pulumirpc;
use crate::state::{EngineState, ResourceState};
use tonic::{Request, Response, Status};

pub struct ResourceMonitorImpl {
    state: EngineState,
    dry_run: bool,
}

impl ResourceMonitorImpl {
    pub fn new(state: EngineState, dry_run: bool) -> Self {
        Self { state, dry_run }
    }
}

#[tonic::async_trait]
impl pulumirpc::resource_monitor_server::ResourceMonitor for ResourceMonitorImpl {
    async fn supports_feature(
        &self,
        request: Request<pulumirpc::SupportsFeatureRequest>,
    ) -> Result<Response<pulumirpc::SupportsFeatureResponse>, Status> {
        let feature = request.into_inner().id;
        let supported = matches!(
            feature.as_str(),
            "secrets" | "resourceReferences" | "outputValues" | "aliasSpecs"
        );
        Ok(Response::new(pulumirpc::SupportsFeatureResponse {
            has_support: supported,
        }))
    }

    async fn invoke(
        &self,
        request: Request<pulumirpc::ResourceInvokeRequest>,
    ) -> Result<Response<pulumirpc::InvokeResponse>, Status> {
        let req = request.into_inner();
        // TODO: Route to actual provider plugins. For now, return empty.
        eprintln!("[engine] invoke: {} (not yet implemented)", req.tok);
        Ok(Response::new(pulumirpc::InvokeResponse {
            r#return: Some(prost_types::Struct {
                fields: Default::default(),
            }),
            failures: vec![],
        }))
    }

    async fn call(
        &self,
        request: Request<pulumirpc::ResourceCallRequest>,
    ) -> Result<Response<pulumirpc::CallResponse>, Status> {
        let req = request.into_inner();
        // TODO: Route to actual provider plugins.
        eprintln!("[engine] call: {} (not yet implemented)", req.tok);
        Ok(Response::new(pulumirpc::CallResponse {
            r#return: Some(prost_types::Struct {
                fields: Default::default(),
            }),
            failures: vec![],
            return_dependencies: Default::default(),
        }))
    }

    async fn read_resource(
        &self,
        request: Request<pulumirpc::ReadResourceRequest>,
    ) -> Result<Response<pulumirpc::ReadResourceResponse>, Status> {
        let req = request.into_inner();
        let urn = self
            .state
            .make_urn(&req.r#type, &req.name, &req.parent)
            .await;

        // TODO: Actually read from the provider. For now, echo back properties.
        Ok(Response::new(pulumirpc::ReadResourceResponse {
            urn,
            properties: req.properties,
        }))
    }

    async fn register_resource(
        &self,
        request: Request<pulumirpc::RegisterResourceRequest>,
    ) -> Result<Response<pulumirpc::RegisterResourceResponse>, Status> {
        let req = request.into_inner();

        let urn = self
            .state
            .make_urn(&req.r#type, &req.name, &req.parent)
            .await;

        let inputs = req
            .object
            .as_ref()
            .map(proto_struct_to_json)
            .unwrap_or(serde_json::Value::Object(Default::default()));

        // Diff against prior state to determine action.
        let prior = self.state.get_prior_resource(&urn).await;
        let diff_result = diff::diff_resource(&urn, &inputs, prior.as_ref(), &req.ignore_changes);

        // Determine the resource ID.
        let id = match diff_result.action {
            ResourceAction::Same | ResourceAction::Update => {
                // Keep the prior ID.
                prior.as_ref().map(|p| p.id.clone()).unwrap_or_default()
            }
            ResourceAction::Create => {
                if req.custom && !self.dry_run {
                    // TODO: Call the actual provider. For now, generate a synthetic ID.
                    format!("{}-id", req.name)
                } else {
                    String::new()
                }
            }
        };

        let action_label = match diff_result.action {
            ResourceAction::Create => "create",
            ResourceAction::Update => "update",
            ResourceAction::Same => "same",
        };
        if diff_result.action != ResourceAction::Same {
            eprintln!("[engine] {action_label}: {} ({})", urn, req.r#type);
        }

        // Collect dependencies from the request.
        let dependencies = req.dependencies.clone();

        // Store the resource state.
        let resource_state = ResourceState {
            urn: urn.clone(),
            id: id.clone(),
            resource_type: req.r#type.clone(),
            name: req.name.clone(),
            custom: req.custom,
            parent: req.parent.clone(),
            inputs,
            outputs: req
                .object
                .as_ref()
                .map(proto_struct_to_json)
                .unwrap_or(serde_json::Value::Object(Default::default())),
            dependencies,
        };
        self.state.register_resource(resource_state).await;

        Ok(Response::new(pulumirpc::RegisterResourceResponse {
            urn,
            id,
            object: req.object,
            stable: true,
            stables: vec![],
            property_dependencies: Default::default(),
            result: 0, // SUCCESS
        }))
    }

    async fn register_resource_outputs(
        &self,
        request: Request<pulumirpc::RegisterResourceOutputsRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();

        // If this is the stack resource, capture the outputs.
        let root_urn = self.state.get_root_urn().await;
        if req.urn == root_urn {
            if let Some(outputs) = &req.outputs {
                self.state
                    .set_stack_outputs(proto_struct_to_json(outputs))
                    .await;
            }
        }

        Ok(Response::new(()))
    }

    async fn register_stack_transform(
        &self,
        _request: Request<pulumirpc::Callback>,
    ) -> Result<Response<()>, Status> {
        Ok(Response::new(()))
    }

    async fn register_stack_invoke_transform(
        &self,
        _request: Request<pulumirpc::Callback>,
    ) -> Result<Response<()>, Status> {
        Ok(Response::new(()))
    }

    async fn register_resource_hook(
        &self,
        _request: Request<pulumirpc::RegisterResourceHookRequest>,
    ) -> Result<Response<()>, Status> {
        Ok(Response::new(()))
    }

    async fn register_error_hook(
        &self,
        _request: Request<pulumirpc::RegisterErrorHookRequest>,
    ) -> Result<Response<()>, Status> {
        Ok(Response::new(()))
    }

    async fn register_package(
        &self,
        request: Request<pulumirpc::RegisterPackageRequest>,
    ) -> Result<Response<pulumirpc::RegisterPackageResponse>, Status> {
        let req = request.into_inner();
        let package_ref = format!("{}@{}", req.name, req.version);
        Ok(Response::new(pulumirpc::RegisterPackageResponse {
            r#ref: package_ref,
        }))
    }

    async fn signal_and_wait_for_shutdown(
        &self,
        _request: Request<()>,
    ) -> Result<Response<()>, Status> {
        Ok(Response::new(()))
    }
}

/// Convert a protobuf Struct to a serde_json::Value.
pub(crate) fn proto_struct_to_json(s: &prost_types::Struct) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (k, v) in &s.fields {
        map.insert(k.clone(), proto_value_to_json(v));
    }
    serde_json::Value::Object(map)
}

fn proto_value_to_json(v: &prost_types::Value) -> serde_json::Value {
    use prost_types::value::Kind;
    match &v.kind {
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::NumberValue(n)) => serde_json::json!(n),
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::StructValue(s)) => proto_struct_to_json(s),
        Some(Kind::ListValue(l)) => {
            let items: Vec<_> = l.values.iter().map(proto_value_to_json).collect();
            serde_json::Value::Array(items)
        }
        None => serde_json::Value::Null,
    }
}
