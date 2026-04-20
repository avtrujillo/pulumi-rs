//! Implementation of the ResourceMonitor gRPC service.
//!
//! Handles resource registration, invocations, and feature queries from the
//! Pulumi program. Routes CRUD operations to provider plugins via the
//! [`Provider`](crate::provider::Provider) trait.

use crate::diff::{self, ResourceAction};
use crate::events::{self, EventCollector};
use crate::provider::{self, Provider, ProviderManager};
use crate::pulumirpc;
use crate::secrets::is_json_secret;
use crate::state::{EngineState, ResourceState};
use prost::Message;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

pub struct ResourceMonitorImpl<P: Provider> {
    state: EngineState,
    dry_run: bool,
    providers: ProviderManager<P>,
    transforms: Arc<Mutex<Vec<pulumirpc::Callback>>>,
    callback_clients: Arc<
        Mutex<
            HashMap<
                String,
                pulumirpc::callbacks_client::CallbacksClient<tonic::transport::Channel>,
            >,
        >,
    >,
    events: Option<EventCollector>,
}

impl<P: Provider> ResourceMonitorImpl<P> {
    pub fn new(
        state: EngineState,
        dry_run: bool,
        providers: ProviderManager<P>,
        events: Option<EventCollector>,
    ) -> Self {
        Self {
            state,
            dry_run,
            providers,
            transforms: Arc::new(Mutex::new(Vec::new())),
            callback_clients: Arc::new(Mutex::new(HashMap::new())),
            events,
        }
    }

    /// Invoke all registered stack transforms against the given resource registration,
    /// returning the (possibly modified) object, ignore_changes, and additional_secret_outputs.
    async fn apply_transforms(
        &self,
        resource_type: &str,
        name: &str,
        custom: bool,
        parent: &str,
        mut object: Option<prost_types::Struct>,
        mut ignore_changes: Vec<String>,
        mut additional_secret_outputs: Vec<String>,
    ) -> Result<(Option<prost_types::Struct>, Vec<String>, Vec<String>), Status> {
        let transforms = self.transforms.lock().await.clone();
        for callback in transforms {
            let transform_req = pulumirpc::TransformRequest {
                r#type: resource_type.to_string(),
                name: name.to_string(),
                custom,
                parent: parent.to_string(),
                properties: object.clone(),
                options: Some(pulumirpc::TransformResourceOptions {
                    ignore_changes: ignore_changes.clone(),
                    additional_secret_outputs: additional_secret_outputs.clone(),
                    ..Default::default()
                }),
            };

            let req_bytes = transform_req.encode_to_vec();

            let mut client = {
                let mut clients = self.callback_clients.lock().await;
                if let Some(existing) = clients.get(&callback.target) {
                    existing.clone()
                } else {
                    let endpoint =
                        tonic::transport::Channel::from_shared(format!("http://{}", callback.target))
                            .map_err(|e| {
                                Status::internal(format!("invalid callback target: {e}"))
                            })?;
                    let channel = endpoint.connect_lazy();
                    let client = pulumirpc::callbacks_client::CallbacksClient::new(channel);
                    clients.insert(callback.target.clone(), client.clone());
                    client
                }
            };

            let response = client
                .invoke(pulumirpc::CallbackInvokeRequest {
                    token: callback.token.clone(),
                    request: req_bytes,
                })
                .await
                .map_err(|e| Status::internal(format!("transform callback failed: {e}")))?;

            let resp_bytes = response.into_inner().response;
            let transform_resp = pulumirpc::TransformResponse::decode(&*resp_bytes)
                .map_err(|e| Status::internal(format!("failed to decode TransformResponse: {e}")))?;

            object = transform_resp.properties;
            if let Some(opts) = transform_resp.options {
                ignore_changes = opts.ignore_changes;
                additional_secret_outputs = opts.additional_secret_outputs;
            }
        }
        Ok((object, ignore_changes, additional_secret_outputs))
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
        let mut req = request.into_inner();

        // Apply registered stack transforms before any other processing.
        let (transformed_object, transformed_ignore, transformed_secrets) = self
            .apply_transforms(
                &req.r#type,
                &req.name,
                req.custom,
                &req.parent,
                req.object.clone(),
                req.ignore_changes.clone(),
                req.additional_secret_outputs.clone(),
            )
            .await?;
        req.object = transformed_object;
        req.ignore_changes = transformed_ignore;
        req.additional_secret_outputs = transformed_secrets;

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

        // Emit a structured event for this resource step.
        if let Some(ev) = &self.events {
            let op = match diff_result.action {
                ResourceAction::Create => "create",
                ResourceAction::Update => "update",
                ResourceAction::Same => "same",
            };
            events::emit(
                ev,
                events::EngineEvent::ResourceStep {
                    op: op.to_string(),
                    urn: urn.clone(),
                    resource_type: req.r#type.clone(),
                    old_inputs: prior
                        .as_ref()
                        .map(|p| p.inputs.clone())
                        .unwrap_or(serde_json::Value::Null),
                    old_outputs: prior
                        .as_ref()
                        .map(|p| p.outputs.clone())
                        .unwrap_or(serde_json::Value::Null),
                    new_inputs: inputs.clone(),
                    new_outputs: outputs_json.clone(),
                },
            );
        }

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
            refresh_before_update: false,
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
        request: Request<pulumirpc::Callback>,
    ) -> Result<Response<()>, Status> {
        self.transforms.lock().await.push(request.into_inner());
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
