//! Implementation of the ResourceMonitor gRPC service.
//!
//! Handles resource registration, invocations, and feature queries from the
//! Pulumi program. Routes CRUD operations to provider plugins via the
//! [`Provider`](crate::provider::Provider) trait.
//!
//! ## Handler / shim split
//!
//! The actual logic for each RPC lives in `handle_*` methods on the inherent
//! `impl ResourceMonitorImpl<P>` block. The `tonic` trait `impl` is a thin
//! shim that unwraps `Request<T>`, calls the handler, and wraps the result in
//! `Response<T>`. Tests can call the handlers directly with plain prost types
//! without going through the gRPC layer.

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
    invoke_transforms: Arc<Mutex<Vec<pulumirpc::Callback>>>,
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
            invoke_transforms: Arc::new(Mutex::new(Vec::new())),
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

    /// Invoke all registered stack invoke transforms against the given invoke request,
    /// returning the (possibly modified) args.
    async fn apply_invoke_transforms(
        &self,
        token: &str,
        mut args: Option<prost_types::Struct>,
    ) -> Result<Option<prost_types::Struct>, Status> {
        let transforms = self.invoke_transforms.lock().await.clone();
        for callback in transforms {
            let transform_req = pulumirpc::TransformInvokeRequest {
                token: token.to_string(),
                args: args.clone(),
                options: None,
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
                .map_err(|e| Status::internal(format!("invoke transform callback failed: {e}")))?;

            let resp_bytes = response.into_inner().response;
            let transform_resp = pulumirpc::TransformInvokeResponse::decode(&*resp_bytes)
                .map_err(|e| {
                    Status::internal(format!("failed to decode TransformInvokeResponse: {e}"))
                })?;

            args = transform_resp.args;
        }
        Ok(args)
    }

    // -----------------------------------------------------------------------
    // Handler methods.
    //
    // These contain the actual logic for each RPC, take plain prost types,
    // and return `Result<T, Status>`. The tonic trait `impl` below is a thin
    // shim that delegates to these.
    // -----------------------------------------------------------------------

    pub(crate) async fn handle_supports_feature(
        &self,
        req: pulumirpc::SupportsFeatureRequest,
    ) -> Result<pulumirpc::SupportsFeatureResponse, Status> {
        let supported = matches!(
            req.id.as_str(),
            "secrets" | "resourceReferences" | "outputValues" | "aliasSpecs"
        );
        Ok(pulumirpc::SupportsFeatureResponse {
            has_support: supported,
        })
    }

    pub(crate) async fn handle_invoke(
        &self,
        mut req: pulumirpc::ResourceInvokeRequest,
    ) -> Result<pulumirpc::InvokeResponse, Status> {
        // Apply registered invoke transforms before calling the provider.
        req.args = self.apply_invoke_transforms(&req.tok, req.args).await?;

        let package = match provider::package_name_from_token(&req.tok) {
            Some(pkg) => pkg,
            None => {
                return Ok(pulumirpc::InvokeResponse {
                    r#return: Some(prost_types::Struct {
                        fields: Default::default(),
                    }),
                    failures: vec![],
                });
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

        Ok(resp)
    }

    pub(crate) async fn handle_call(
        &self,
        req: pulumirpc::ResourceCallRequest,
    ) -> Result<pulumirpc::CallResponse, Status> {
        let package = match provider::package_name_from_token(&req.tok) {
            Some(pkg) => pkg,
            None => {
                return Ok(pulumirpc::CallResponse {
                    r#return: Some(prost_types::Struct {
                        fields: Default::default(),
                    }),
                    failures: vec![],
                    return_dependencies: Default::default(),
                });
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

        Ok(resp)
    }

    pub(crate) async fn handle_read_resource(
        &self,
        req: pulumirpc::ReadResourceRequest,
    ) -> Result<pulumirpc::ReadResourceResponse, Status> {
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

        Ok(pulumirpc::ReadResourceResponse { urn, properties })
    }

    pub(crate) async fn handle_register_resource(
        &self,
        mut req: pulumirpc::RegisterResourceRequest,
    ) -> Result<pulumirpc::RegisterResourceResponse, Status> {
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

        Ok(pulumirpc::RegisterResourceResponse {
            urn,
            id,
            object: output_properties,
            stable: true,
            stables: vec![],
            property_dependencies: Default::default(),
            result: 0,
        })
    }

    pub(crate) async fn handle_register_resource_outputs(
        &self,
        req: pulumirpc::RegisterResourceOutputsRequest,
    ) -> Result<(), Status> {
        let root_urn = self.state.get_root_urn().await;
        if req.urn == root_urn
            && let Some(outputs) = &req.outputs
        {
            self.state
                .set_stack_outputs(proto_struct_to_json(outputs))
                .await;
        }
        Ok(())
    }

    pub(crate) async fn handle_register_stack_transform(
        &self,
        callback: pulumirpc::Callback,
    ) -> Result<(), Status> {
        self.transforms.lock().await.push(callback);
        Ok(())
    }

    pub(crate) async fn handle_register_stack_invoke_transform(
        &self,
        callback: pulumirpc::Callback,
    ) -> Result<(), Status> {
        self.invoke_transforms.lock().await.push(callback);
        Ok(())
    }

    pub(crate) async fn handle_register_resource_hook(
        &self,
        _req: pulumirpc::RegisterResourceHookRequest,
    ) -> Result<(), Status> {
        Ok(())
    }

    pub(crate) async fn handle_register_error_hook(
        &self,
        _req: pulumirpc::RegisterErrorHookRequest,
    ) -> Result<(), Status> {
        Ok(())
    }

    pub(crate) async fn handle_register_package(
        &self,
        req: pulumirpc::RegisterPackageRequest,
    ) -> Result<pulumirpc::RegisterPackageResponse, Status> {
        let package_ref = format!("{}@{}", req.name, req.version);
        Ok(pulumirpc::RegisterPackageResponse {
            r#ref: package_ref,
        })
    }

    pub(crate) async fn handle_signal_and_wait_for_shutdown(&self) -> Result<(), Status> {
        Ok(())
    }
}

#[tonic::async_trait]
impl<P: Provider> pulumirpc::resource_monitor_server::ResourceMonitor for ResourceMonitorImpl<P> {
    async fn supports_feature(
        &self,
        request: Request<pulumirpc::SupportsFeatureRequest>,
    ) -> Result<Response<pulumirpc::SupportsFeatureResponse>, Status> {
        let resp = self.handle_supports_feature(request.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn invoke(
        &self,
        request: Request<pulumirpc::ResourceInvokeRequest>,
    ) -> Result<Response<pulumirpc::InvokeResponse>, Status> {
        let resp = self.handle_invoke(request.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn call(
        &self,
        request: Request<pulumirpc::ResourceCallRequest>,
    ) -> Result<Response<pulumirpc::CallResponse>, Status> {
        let resp = self.handle_call(request.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn read_resource(
        &self,
        request: Request<pulumirpc::ReadResourceRequest>,
    ) -> Result<Response<pulumirpc::ReadResourceResponse>, Status> {
        let resp = self.handle_read_resource(request.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn register_resource(
        &self,
        request: Request<pulumirpc::RegisterResourceRequest>,
    ) -> Result<Response<pulumirpc::RegisterResourceResponse>, Status> {
        let resp = self.handle_register_resource(request.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn register_resource_outputs(
        &self,
        request: Request<pulumirpc::RegisterResourceOutputsRequest>,
    ) -> Result<Response<()>, Status> {
        self.handle_register_resource_outputs(request.into_inner())
            .await?;
        Ok(Response::new(()))
    }

    async fn register_stack_transform(
        &self,
        request: Request<pulumirpc::Callback>,
    ) -> Result<Response<()>, Status> {
        self.handle_register_stack_transform(request.into_inner())
            .await?;
        Ok(Response::new(()))
    }

    async fn register_stack_invoke_transform(
        &self,
        request: Request<pulumirpc::Callback>,
    ) -> Result<Response<()>, Status> {
        self.handle_register_stack_invoke_transform(request.into_inner())
            .await?;
        Ok(Response::new(()))
    }

    async fn register_resource_hook(
        &self,
        request: Request<pulumirpc::RegisterResourceHookRequest>,
    ) -> Result<Response<()>, Status> {
        self.handle_register_resource_hook(request.into_inner())
            .await?;
        Ok(Response::new(()))
    }

    async fn register_error_hook(
        &self,
        request: Request<pulumirpc::RegisterErrorHookRequest>,
    ) -> Result<Response<()>, Status> {
        self.handle_register_error_hook(request.into_inner())
            .await?;
        Ok(Response::new(()))
    }

    async fn register_package(
        &self,
        request: Request<pulumirpc::RegisterPackageRequest>,
    ) -> Result<Response<pulumirpc::RegisterPackageResponse>, Status> {
        let resp = self.handle_register_package(request.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn signal_and_wait_for_shutdown(
        &self,
        _request: Request<()>,
    ) -> Result<Response<()>, Status> {
        self.handle_signal_and_wait_for_shutdown().await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderManager;
    use crate::state::EngineState;
    use crate::test_utils::MockProvider;

    fn make_monitor(dry_run: bool) -> ResourceMonitorImpl<MockProvider> {
        let state = EngineState::new("test".into(), "dev".into());
        ResourceMonitorImpl::new(state, dry_run, ProviderManager::new(), None)
    }

    fn make_monitor_with_state(
        state: EngineState,
        dry_run: bool,
    ) -> ResourceMonitorImpl<MockProvider> {
        ResourceMonitorImpl::new(state, dry_run, ProviderManager::new(), None)
    }

    // --- supports_feature ---

    #[tokio::test]
    async fn test_supports_known_features() {
        let svc = make_monitor(false);
        for feature in ["secrets", "resourceReferences", "outputValues", "aliasSpecs"] {
            let resp = svc
                .handle_supports_feature(pulumirpc::SupportsFeatureRequest {
                    id: feature.into(),
                })
                .await
                .unwrap();
            assert!(resp.has_support, "{feature} should be supported");
        }
    }

    #[tokio::test]
    async fn test_unsupported_feature_returns_false() {
        let svc = make_monitor(false);
        let resp = svc
            .handle_supports_feature(pulumirpc::SupportsFeatureRequest {
                id: "unknownFeature".into(),
            })
            .await
            .unwrap();
        assert!(!resp.has_support);
    }

    // --- register_resource (builtin / component / dry_run) ---

    #[tokio::test]
    async fn test_register_builtin_stack_resource() {
        let svc = make_monitor(false);
        let resp = svc
            .handle_register_resource(pulumirpc::RegisterResourceRequest {
                r#type: "pulumi:pulumi:Stack".into(),
                name: "dev".into(),
                custom: false,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(resp.urn.contains("pulumi:pulumi:Stack"));
        assert!(resp.urn.contains("dev"));
        assert!(resp.id.is_empty());
    }

    #[tokio::test]
    async fn test_register_component_resource_no_provider() {
        let svc = make_monitor(false);
        let resp = svc
            .handle_register_resource(pulumirpc::RegisterResourceRequest {
                r#type: "my:module:Component".into(),
                name: "comp".into(),
                custom: false,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(resp.id.is_empty());
        assert!(resp.urn.contains("my:module:Component::comp"));
    }

    #[tokio::test]
    async fn test_register_custom_resource_dry_run_skips_provider() {
        let svc = make_monitor(true);
        let resp = svc
            .handle_register_resource(pulumirpc::RegisterResourceRequest {
                r#type: "aws:s3/bucket:Bucket".into(),
                name: "my-bucket".into(),
                custom: true,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(!resp.urn.is_empty());
        // Dry run: no provider create called, so no real ID assigned.
        assert!(resp.id.is_empty());
    }

    #[tokio::test]
    async fn test_register_custom_resource_non_dry_run_gets_id() {
        let svc = make_monitor(false);
        let resp = svc
            .handle_register_resource(pulumirpc::RegisterResourceRequest {
                r#type: "aws:s3/bucket:Bucket".into(),
                name: "my-bucket".into(),
                custom: true,
                ..Default::default()
            })
            .await
            .unwrap();
        // MockProvider::create returns "mock-id-<name>".
        assert!(!resp.id.is_empty());
    }

    #[tokio::test]
    async fn test_register_resource_stores_in_state() {
        let state = EngineState::new("test".into(), "dev".into());
        let svc = make_monitor_with_state(state.clone(), false);
        svc.handle_register_resource(pulumirpc::RegisterResourceRequest {
            r#type: "pulumi:pulumi:Stack".into(),
            name: "stack".into(),
            custom: false,
            ..Default::default()
        })
        .await
        .unwrap();
        let resources = state.get_resources().await;
        assert_eq!(resources.len(), 1);
    }

    // --- register_resource_outputs ---

    #[tokio::test]
    async fn test_register_resource_outputs_sets_stack_outputs() {
        let state = EngineState::new("test".into(), "dev".into());
        let stack_urn = "urn:pulumi:dev::test::pulumi:pulumi:Stack::stack";
        state.set_root_urn(stack_urn.to_string()).await;

        let svc = make_monitor_with_state(state.clone(), false);

        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "url".into(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::StringValue(
                    "https://example.com".into(),
                )),
            },
        );
        svc.handle_register_resource_outputs(pulumirpc::RegisterResourceOutputsRequest {
            urn: stack_urn.to_string(),
            outputs: Some(prost_types::Struct { fields }),
        })
        .await
        .unwrap();

        let outputs = state.get_stack_outputs().await;
        assert_eq!(outputs["url"], "https://example.com");
    }

    #[tokio::test]
    async fn test_register_resource_outputs_non_root_ignored() {
        let state = EngineState::new("test".into(), "dev".into());
        state
            .set_root_urn("urn:pulumi:dev::test::pulumi:pulumi:Stack::stack".to_string())
            .await;
        let svc = make_monitor_with_state(state.clone(), false);

        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "key".into(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::StringValue("value".into())),
            },
        );
        svc.handle_register_resource_outputs(pulumirpc::RegisterResourceOutputsRequest {
            urn: "urn:pulumi:dev::test::some:other:Resource::res".to_string(),
            outputs: Some(prost_types::Struct { fields }),
        })
        .await
        .unwrap();

        // Stack outputs should still be empty (root URN didn't match).
        let outputs = state.get_stack_outputs().await;
        assert_eq!(outputs, serde_json::Value::Object(Default::default()));
    }

    // --- register_stack_transform ---

    #[tokio::test]
    async fn test_register_stack_transform_stored() {
        let svc = make_monitor(false);
        svc.handle_register_stack_transform(pulumirpc::Callback {
            target: "127.0.0.1:12345".into(),
            token: "my-token".into(),
        })
        .await
        .unwrap();
        let transforms = svc.transforms.lock().await;
        assert_eq!(transforms.len(), 1);
        assert_eq!(transforms[0].token, "my-token");
    }

    #[tokio::test]
    async fn test_register_stack_invoke_transform_stored() {
        let svc = make_monitor(false);
        svc.handle_register_stack_invoke_transform(pulumirpc::Callback {
            target: "127.0.0.1:12345".into(),
            token: "invoke-token".into(),
        })
        .await
        .unwrap();
        let transforms = svc.invoke_transforms.lock().await;
        assert_eq!(transforms.len(), 1);
        assert_eq!(transforms[0].token, "invoke-token");
    }

    // --- invoke ---

    #[tokio::test]
    async fn test_invoke_builtin_token_returns_empty_without_provider() {
        let svc = make_monitor(false);
        let resp = svc
            .handle_invoke(pulumirpc::ResourceInvokeRequest {
                tok: "pulumi:pulumi:getStack".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(resp.failures.is_empty());
        assert!(resp.r#return.is_some());
    }

    #[tokio::test]
    async fn test_invoke_provider_token_delegates_to_provider() {
        let svc = make_monitor(false);
        let resp = svc
            .handle_invoke(pulumirpc::ResourceInvokeRequest {
                tok: "aws:ec2/getAmi:getAmi".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(resp.failures.is_empty());
    }

    // --- register_package ---

    #[tokio::test]
    async fn test_register_package_returns_versioned_ref() {
        let svc = make_monitor(false);
        let resp = svc
            .handle_register_package(pulumirpc::RegisterPackageRequest {
                name: "aws".into(),
                version: "6.0.0".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(resp.r#ref, "aws@6.0.0");
    }
}
