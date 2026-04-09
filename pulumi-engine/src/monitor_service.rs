//! Implementation of the ResourceMonitor gRPC service.
//!
//! Handles resource registration, invocations, and feature queries from the
//! Pulumi program. Routes CRUD operations to provider plugins via the
//! [`Provider`](crate::provider::Provider) trait.

use crate::diff::{self, ResourceAction};
use crate::provider::{self, Provider, ProviderManager};
use crate::pulumirpc;
use crate::secrets::is_json_secret;
use crate::state::{EngineState, ResourceState};
use tonic::{Request, Response, Status};

pub struct ResourceMonitorImpl<P: Provider> {
    state: EngineState,
    dry_run: bool,
    providers: ProviderManager<P>,
}

impl<P: Provider> ResourceMonitorImpl<P> {
    pub fn new(state: EngineState, dry_run: bool, providers: ProviderManager<P>) -> Self {
        Self {
            state,
            dry_run,
            providers,
        }
    }
}

#[tonic::async_trait]
impl<P: Provider> pulumirpc::resource_monitor_server::ResourceMonitor for ResourceMonitorImpl<P> {
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

        let package = match provider::package_name_from_token(&req.tok) {
            Some(pkg) => pkg,
            None => {
                return Ok(Response::new(pulumirpc::InvokeResponse {
                    r#return: Some(prost_types::Struct {
                        fields: Default::default(),
                    }),
                    failures: vec![],
                }));
            }
        };

        let mut provider = self
            .providers
            .get_provider(&package)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let resp = provider
            .invoke(pulumirpc::InvokeRequest {
                tok: req.tok,
                args: req.args,
                ..Default::default()
            })
            .await
            .map_err(|e| Status::internal(format!("provider invoke failed: {e}")))?;

        Ok(Response::new(resp))
    }

    async fn call(
        &self,
        request: Request<pulumirpc::ResourceCallRequest>,
    ) -> Result<Response<pulumirpc::CallResponse>, Status> {
        let req = request.into_inner();

        let package = match provider::package_name_from_token(&req.tok) {
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

        let mut provider = self
            .providers
            .get_provider(&package)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let resp = provider
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

        Ok(Response::new(resp))
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

        let properties = if let Some(package) = provider::package_name(&req.r#type) {
            let mut provider = self
                .providers
                .get_provider(&package)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;

            let resp = provider
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

            resp.properties
        } else {
            req.properties
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

        let needs_provider = req.custom && provider::package_name(&req.r#type).is_some();

        // Execute the provider operation and get ID + outputs.
        let (id, output_properties) = match diff_result.action {
            ResourceAction::Same => {
                let id = prior.as_ref().map(|p| p.id.clone()).unwrap_or_default();
                (id, req.object.clone())
            }
            ResourceAction::Create => {
                if needs_provider {
                    if self.dry_run {
                        (String::new(), req.object.clone())
                    } else {
                        let package = provider::package_name(&req.r#type).unwrap();
                        let mut provider = self
                            .providers
                            .get_provider(&package)
                            .await
                            .map_err(|e| Status::internal(e.to_string()))?;

                        let resp = provider
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

                        (resp.id, resp.properties)
                    }
                } else {
                    (String::new(), req.object.clone())
                }
            }
            ResourceAction::Update => {
                let prior_id = prior.as_ref().map(|p| p.id.clone()).unwrap_or_default();

                if needs_provider {
                    if self.dry_run {
                        (prior_id, req.object.clone())
                    } else {
                        let package = provider::package_name(&req.r#type).unwrap();
                        let mut provider = self
                            .providers
                            .get_provider(&package)
                            .await
                            .map_err(|e| Status::internal(e.to_string()))?;

                        let prior_outputs = prior
                            .as_ref()
                            .map(|p| provider::json_to_proto_struct(&p.outputs));
                        let prior_inputs = prior
                            .as_ref()
                            .map(|p| provider::json_to_proto_struct(&p.inputs));

                        let resp = provider
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

        let outputs_json = output_properties
            .as_ref()
            .map(proto_struct_to_json)
            .unwrap_or_else(|| inputs.clone());

        let dependencies = req.dependencies.clone();

        // Collect secret property names from inputs, outputs, and the
        // additional_secret_outputs declared by the program.
        let mut secret_props: Vec<String> = collect_secret_keys(&inputs);
        secret_props.extend(collect_secret_keys(&outputs_json));
        secret_props.extend(req.additional_secret_outputs.iter().cloned());
        secret_props.sort();
        secret_props.dedup();

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
            secret_properties: secret_props,
        };
        self.state.register_resource(resource_state).await;

        Ok(Response::new(pulumirpc::RegisterResourceResponse {
            urn,
            id,
            object: output_properties,
            stable: true,
            stables: vec![],
            property_dependencies: Default::default(),
            result: 0,
        }))
    }

    async fn register_resource_outputs(
        &self,
        request: Request<pulumirpc::RegisterResourceOutputsRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();

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
    pulumi_core::serde::struct_to_json(s)
}

/// Collect top-level property names whose values are secret-wrapped.
fn collect_secret_keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys = Vec::new();
    if let serde_json::Value::Object(map) = value {
        for (k, v) in map {
            if is_json_secret(v) {
                keys.push(k.clone());
            }
        }
    }
    keys
}
