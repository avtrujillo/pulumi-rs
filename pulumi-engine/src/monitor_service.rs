//! Implementation of the ResourceMonitor gRPC service.
//!
//! Handles resource registration, invocations, and feature queries from the
//! Pulumi program. Routes CRUD operations to provider plugins via gRPC.

use crate::diff::{self, ResourceAction};
use crate::provider::ProviderManager;
use crate::pulumirpc;
use crate::state::{EngineState, ResourceState};
use tonic::{Request, Response, Status};

pub struct ResourceMonitorImpl {
    state: EngineState,
    dry_run: bool,
    providers: ProviderManager,
}

impl ResourceMonitorImpl {
    pub fn new(state: EngineState, dry_run: bool, providers: ProviderManager) -> Self {
        Self {
            state,
            dry_run,
            providers,
        }
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

        let package = match ProviderManager::package_name_from_token(&req.tok) {
            Some(pkg) => pkg,
            None => {
                // Built-in function, return empty.
                return Ok(Response::new(pulumirpc::InvokeResponse {
                    r#return: Some(prost_types::Struct {
                        fields: Default::default(),
                    }),
                    failures: vec![],
                }));
            }
        };

        let mut client = self
            .providers
            .get_provider(&package)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let response = client
            .invoke(pulumirpc::InvokeRequest {
                tok: req.tok,
                args: req.args,
                ..Default::default()
            })
            .await
            .map_err(|e| Status::internal(format!("provider invoke failed: {e}")))?;

        Ok(Response::new(response.into_inner()))
    }

    async fn call(
        &self,
        request: Request<pulumirpc::ResourceCallRequest>,
    ) -> Result<Response<pulumirpc::CallResponse>, Status> {
        let req = request.into_inner();

        let package = match ProviderManager::package_name_from_token(&req.tok) {
            Some(pkg) => pkg,
            None => {
                return Ok(Response::new(pulumirpc::CallResponse {
                    r#return: Some(prost_types::Struct {
                        fields: Default::default(),
                    }),
                    failures: vec![],
                    return_dependencies: Default::default(),
                }));
            }
        };

        let mut client = self
            .providers
            .get_provider(&package)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let response = client
            .call(pulumirpc::CallRequest {
                tok: req.tok,
                args: req.args,
                arg_dependencies: req
                    .arg_dependencies
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            pulumirpc::call_request::ArgumentDependencies { urns: v.urns },
                        )
                    })
                    .collect(),
                ..Default::default()
            })
            .await
            .map_err(|e| Status::internal(format!("provider call failed: {e}")))?;

        Ok(Response::new(response.into_inner()))
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

        let package = ProviderManager::package_name(&req.r#type);

        let (_id, properties) = if let Some(package) = package {
            let mut client = self
                .providers
                .get_provider(&package)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;

            let response = client
                .read(pulumirpc::ReadRequest {
                    id: req.id.clone(),
                    urn: urn.clone(),
                    properties: req.properties.clone(),
                    inputs: req.properties.clone(),
                    name: req.name.clone(),
                    r#type: req.r#type.clone(),
                    ..Default::default()
                })
                .await
                .map_err(|e| Status::internal(format!("provider read failed: {e}")))?;

            let resp = response.into_inner();
            (resp.id, resp.properties)
        } else {
            // Built-in type, echo back.
            (req.id, req.properties)
        };

        Ok(Response::new(pulumirpc::ReadResourceResponse {
            urn,
            properties,
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

        let package = if req.custom {
            ProviderManager::package_name(&req.r#type)
        } else {
            None
        };

        // Execute the provider operation and get ID + outputs.
        let (id, output_properties) = match diff_result.action {
            ResourceAction::Same => {
                let id = prior.as_ref().map(|p| p.id.clone()).unwrap_or_default();
                (id, req.object.clone())
            }
            ResourceAction::Create => {
                if let Some(ref package) = package {
                    if self.dry_run {
                        // Preview: don't call the provider.
                        (String::new(), req.object.clone())
                    } else {
                        let mut client = self
                            .providers
                            .get_provider(package)
                            .await
                            .map_err(|e| Status::internal(e.to_string()))?;

                        let response = client
                            .create(pulumirpc::CreateRequest {
                                urn: urn.clone(),
                                properties: req.object.clone(),
                                timeout: req
                                    .custom_timeouts
                                    .as_ref()
                                    .and_then(|t| t.create.parse::<f64>().ok())
                                    .unwrap_or(0.0),
                                preview: false,
                                name: req.name.clone(),
                                r#type: req.r#type.clone(),
                                ..Default::default()
                            })
                            .await
                            .map_err(|e| {
                                Status::internal(format!("provider create failed: {e}"))
                            })?;

                        let resp = response.into_inner();
                        (resp.id, resp.properties)
                    }
                } else {
                    // Component resource or built-in — no provider call.
                    (String::new(), req.object.clone())
                }
            }
            ResourceAction::Update => {
                let prior_id = prior.as_ref().map(|p| p.id.clone()).unwrap_or_default();

                if let Some(ref package) = package {
                    if self.dry_run {
                        (prior_id, req.object.clone())
                    } else {
                        let mut client = self
                            .providers
                            .get_provider(package)
                            .await
                            .map_err(|e| Status::internal(e.to_string()))?;

                        let prior_outputs = prior
                            .as_ref()
                            .map(|p| crate::provider::json_to_proto_struct(&p.outputs));
                        let prior_inputs = prior
                            .as_ref()
                            .map(|p| crate::provider::json_to_proto_struct(&p.inputs));

                        let response = client
                            .update(pulumirpc::UpdateRequest {
                                id: prior_id,
                                urn: urn.clone(),
                                olds: prior_outputs,
                                news: req.object.clone(),
                                timeout: req
                                    .custom_timeouts
                                    .as_ref()
                                    .and_then(|t| t.update.parse::<f64>().ok())
                                    .unwrap_or(0.0),
                                ignore_changes: req.ignore_changes.clone(),
                                preview: false,
                                old_inputs: prior_inputs,
                                name: req.name.clone(),
                                r#type: req.r#type.clone(),
                                ..Default::default()
                            })
                            .await
                            .map_err(|e| {
                                Status::internal(format!("provider update failed: {e}"))
                            })?;

                        let resp = response.into_inner();
                        // Update returns new properties; keep the same ID.
                        let id = prior.as_ref().map(|p| p.id.clone()).unwrap_or_default();
                        (id, resp.properties)
                    }
                } else {
                    (prior_id, req.object.clone())
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

        // Use provider outputs if available, otherwise fall back to inputs.
        let outputs_json = output_properties
            .as_ref()
            .map(proto_struct_to_json)
            .unwrap_or_else(|| inputs.clone());

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
            outputs: outputs_json,
            dependencies,
        };
        self.state.register_resource(resource_state).await;

        Ok(Response::new(pulumirpc::RegisterResourceResponse {
            urn,
            id,
            object: output_properties,
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
        if req.urn == root_urn
            && let Some(outputs) = &req.outputs
        {
            self.state
                .set_stack_outputs(proto_struct_to_json(outputs))
                .await;
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
