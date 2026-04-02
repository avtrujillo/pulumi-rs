use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use prost::Message;
use tokio::sync::{Mutex, oneshot};

use crate::context::Context;
use crate::error::{Error, Result};
use crate::proto::pulumirpc;
use crate::proto::pulumirpc::callbacks_server::{Callbacks, CallbacksServer};
use crate::resource::{
    Alias, AliasParent, AliasSpec, CustomTimeouts, ResourceOptions, alias_to_proto,
};
use crate::serde::{json_to_struct, struct_to_json};

/// The arguments passed to a resource transform function.
#[derive(Debug, Clone)]
pub struct TransformArgs {
    /// The type token of the resource being registered.
    pub resource_type: String,
    /// The name of the resource.
    pub name: String,
    /// Whether this is a custom resource (vs. component).
    pub custom: bool,
    /// The parent URN.
    pub parent: String,
    /// The input properties as JSON.
    pub props: serde_json::Value,
    /// The resource options.
    pub opts: ResourceOptions,
}

/// The result returned from a resource transform function.
#[derive(Debug, Clone)]
pub struct TransformResult {
    /// The (possibly modified) input properties.
    pub props: serde_json::Value,
    /// The (possibly modified) resource options.
    pub opts: ResourceOptions,
}

/// A transform function that can modify resource registrations.
pub type TransformFn = Arc<dyn Fn(TransformArgs) -> TransformResult + Send + Sync + 'static>;

/// Internal state for the callback server.
struct CallbackState {
    transforms: HashMap<String, TransformFn>,
    next_id: u64,
}

/// A running callback server that dispatches transform invocations.
struct CallbackService {
    state: Arc<Mutex<CallbackState>>,
}

#[tonic::async_trait]
impl Callbacks for CallbackService {
    async fn invoke(
        &self,
        request: tonic::Request<pulumirpc::CallbackInvokeRequest>,
    ) -> std::result::Result<tonic::Response<pulumirpc::CallbackInvokeResponse>, tonic::Status>
    {
        let req = request.into_inner();
        let token = &req.token;

        let state = self.state.lock().await;
        let transform_fn = state
            .transforms
            .get(token)
            .ok_or_else(|| tonic::Status::not_found(format!("unknown callback token: {token}")))?
            .clone();
        drop(state);

        // Decode the TransformRequest from the raw bytes.
        let transform_req = pulumirpc::TransformRequest::decode(&*req.request).map_err(|e| {
            tonic::Status::invalid_argument(format!("failed to decode TransformRequest: {e}"))
        })?;

        let props = transform_req
            .properties
            .as_ref()
            .map(struct_to_json)
            .unwrap_or(serde_json::Value::Object(Default::default()));

        let opts = proto_opts_to_resource_options(transform_req.options.as_ref());

        let args = TransformArgs {
            resource_type: transform_req.r#type,
            name: transform_req.name,
            custom: transform_req.custom,
            parent: transform_req.parent,
            props,
            opts,
        };

        let result = transform_fn(args);

        let response = pulumirpc::TransformResponse {
            properties: Some(json_to_struct(&result.props)),
            options: Some(resource_options_to_proto_opts(&result.opts)),
        };

        let response_bytes = response.encode_to_vec();

        Ok(tonic::Response::new(pulumirpc::CallbackInvokeResponse {
            response: response_bytes,
        }))
    }
}

/// Shared state for the callback server, lazily started per-context.
struct ServerHandle {
    addr: SocketAddr,
    state: Arc<Mutex<CallbackState>>,
}

/// Manages the lifecycle of the callback server.
///
/// The server is started lazily on the first `register_stack_transform` call.
/// All subsequent transforms share the same server.
static CALLBACK_SERVER: Mutex<Option<ServerHandle>> = Mutex::const_new(None);

/// Starts the callback gRPC server if it isn't already running, then returns
/// the server's address and shared transform registry.
async fn ensure_callback_server() -> Result<(SocketAddr, Arc<Mutex<CallbackState>>)> {
    let mut guard = CALLBACK_SERVER.lock().await;
    if let Some(handle) = guard.as_ref() {
        return Ok((handle.addr, handle.state.clone()));
    }

    let state = Arc::new(Mutex::new(CallbackState {
        transforms: HashMap::new(),
        next_id: 0,
    }));

    let service = CallbackService {
        state: state.clone(),
    };

    // Bind to localhost on a random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| Error::Custom(format!("failed to bind callback server: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| Error::Custom(format!("failed to get callback server address: {e}")))?;

    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let (started_tx, started_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        let _ = started_tx.send(());
        tonic::transport::Server::builder()
            .add_service(CallbacksServer::new(service))
            .serve_with_incoming(incoming)
            .await
            .ok();
    });

    // Wait for the server task to start.
    let _ = started_rx.await;

    *guard = Some(ServerHandle {
        addr,
        state: state.clone(),
    });

    Ok((addr, state))
}

/// Registers a stack-level resource transform.
///
/// The transform function is called for every resource registration in the stack,
/// giving you the opportunity to modify the resource's properties and options
/// before registration.
///
/// # Example
///
/// ```ignore
/// use pulumi_core::transform::{register_stack_transform, TransformArgs, TransformResult};
/// use std::sync::Arc;
///
/// register_stack_transform(&ctx, Arc::new(|mut args: TransformArgs| {
///     // Add a "managed-by" tag to all resources.
///     if let serde_json::Value::Object(ref mut map) = args.props {
///         map.insert("managedBy".into(), "pulumi-rs".into());
///     }
///     TransformResult {
///         props: args.props,
///         opts: args.opts,
///     }
/// })).await?;
/// ```
pub async fn register_stack_transform(ctx: &Context, transform: TransformFn) -> Result<()> {
    let (addr, state) = ensure_callback_server().await?;

    let token = {
        let mut s = state.lock().await;
        let id = s.next_id;
        s.next_id += 1;
        let token = format!("transform-{id}");
        s.transforms.insert(token.clone(), transform);
        token
    };

    let callback = pulumirpc::Callback {
        target: format!("127.0.0.1:{}", addr.port()),
        token,
    };

    ctx.monitor()
        .register_stack_transform(callback)
        .await
        .map_err(|e| Error::Custom(format!("failed to register stack transform: {e}")))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Conversion helpers between ResourceOptions and proto TransformResourceOptions
// ---------------------------------------------------------------------------

fn proto_opts_to_resource_options(
    opts: Option<&pulumirpc::TransformResourceOptions>,
) -> ResourceOptions {
    let Some(opts) = opts else {
        return ResourceOptions::default();
    };

    ResourceOptions {
        parent: None, // parent is on TransformRequest, not options
        provider: if opts.provider.is_empty() {
            None
        } else {
            Some(opts.provider.clone())
        },
        depends_on: opts.depends_on.clone(),
        protect: opts.protect.unwrap_or(false),
        ignore_changes: opts.ignore_changes.clone(),
        version: opts.version.clone(),
        plugin_download_url: opts.plugin_download_url.clone(),
        additional_secret_outputs: opts.additional_secret_outputs.clone(),
        delete_before_replace: opts.delete_before_replace.unwrap_or(false),
        replace_on_changes: opts.replace_on_changes.clone(),
        retain_on_delete: opts.retain_on_delete.unwrap_or(false),
        import_id: if opts.import.is_empty() {
            None
        } else {
            Some(opts.import.clone())
        },
        aliases: opts.aliases.iter().map(proto_alias_to_alias).collect(),
        custom_timeouts: opts.custom_timeouts.as_ref().map(|t| CustomTimeouts {
            create: if t.create.is_empty() {
                None
            } else {
                Some(t.create.clone())
            },
            update: if t.update.is_empty() {
                None
            } else {
                Some(t.update.clone())
            },
            delete: if t.delete.is_empty() {
                None
            } else {
                Some(t.delete.clone())
            },
        }),
        deleted_with: if opts.deleted_with.is_empty() {
            None
        } else {
            Some(opts.deleted_with.clone())
        },
    }
}

fn resource_options_to_proto_opts(opts: &ResourceOptions) -> pulumirpc::TransformResourceOptions {
    pulumirpc::TransformResourceOptions {
        depends_on: opts.depends_on.clone(),
        protect: Some(opts.protect),
        ignore_changes: opts.ignore_changes.clone(),
        replace_on_changes: opts.replace_on_changes.clone(),
        version: opts.version.clone(),
        aliases: opts.aliases.iter().map(alias_to_proto).collect(),
        provider: opts.provider.clone().unwrap_or_default(),
        custom_timeouts: opts.custom_timeouts.as_ref().map(|t| {
            pulumirpc::register_resource_request::CustomTimeouts {
                create: t.create.clone().unwrap_or_default(),
                update: t.update.clone().unwrap_or_default(),
                delete: t.delete.clone().unwrap_or_default(),
            }
        }),
        plugin_download_url: opts.plugin_download_url.clone(),
        retain_on_delete: Some(opts.retain_on_delete),
        deleted_with: opts.deleted_with.clone().unwrap_or_default(),
        delete_before_replace: Some(opts.delete_before_replace),
        additional_secret_outputs: opts.additional_secret_outputs.clone(),
        providers: HashMap::new(),
        plugin_checksums: HashMap::new(),
        hooks: None,
        import: opts.import_id.clone().unwrap_or_default(),
        hide_diff: Vec::new(),
        replace_with: Vec::new(),
        replacement_trigger: None,
    }
}

fn proto_alias_to_alias(proto: &pulumirpc::Alias) -> Alias {
    match &proto.alias {
        Some(pulumirpc::alias::Alias::Urn(urn)) => Alias::Urn(urn.clone()),
        Some(pulumirpc::alias::Alias::Spec(spec)) => Alias::Spec(AliasSpec {
            name: if spec.name.is_empty() {
                None
            } else {
                Some(spec.name.clone())
            },
            r#type: if spec.r#type.is_empty() {
                None
            } else {
                Some(spec.r#type.clone())
            },
            stack: if spec.stack.is_empty() {
                None
            } else {
                Some(spec.stack.clone())
            },
            project: if spec.project.is_empty() {
                None
            } else {
                Some(spec.project.clone())
            },
            parent: spec.parent.as_ref().map(|p| match p {
                pulumirpc::alias::spec::Parent::ParentUrn(urn) => AliasParent::Urn(urn.clone()),
                pulumirpc::alias::spec::Parent::NoParent(_) => AliasParent::NoParent,
            }),
        }),
        None => Alias::Urn(String::new()),
    }
}
