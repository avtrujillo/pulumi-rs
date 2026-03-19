use std::future::{Future, IntoFuture};

use crate::context::Context;
use crate::error::{Error, Result};
use crate::resource::{register_resource_inner, ResourceOptions};

/// A reference to another Pulumi stack's outputs.
///
/// Stack references allow you to read exported outputs from a different stack,
/// enabling cross-stack dependencies. The referenced stack must be in the same
/// Pulumi organization (or the appropriate backend).
///
/// # Example
///
/// ```ignore
/// // Read outputs from the "networking" stack in the same project.
/// let net_stack = StackReferenceBuilder::new(&ctx, "org/project/networking")
///     .await?;
///
/// // Get a specific output value.
/// let vpc_id: String = net_stack.require_output("vpcId")?;
/// ```
#[derive(Debug, Clone)]
pub struct StackReference {
    /// The URN of the stack reference resource.
    pub urn: String,
    /// All exported outputs from the referenced stack as a JSON object.
    pub outputs: serde_json::Value,
}

impl StackReference {
    /// Gets an output value by key, returning `None` if missing.
    pub fn get_output(&self, key: &str) -> Option<&serde_json::Value> {
        self.outputs.get(key)
    }

    /// Gets a required output value by key, returning an error if missing.
    pub fn require_output<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<T> {
        let val = self.get_output(key).ok_or_else(|| {
            Error::Custom(format!(
                "stack reference output {key:?} not found"
            ))
        })?;
        serde_json::from_value(val.clone())
            .map_err(|e| Error::Custom(format!("stack reference output {key:?}: {e}")))
    }

    /// Gets an output value deserialized as type `T`, returning `None` if missing.
    pub fn get_output_typed<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        match self.get_output(key) {
            None => Ok(None),
            Some(val) => serde_json::from_value(val.clone())
                .map(Some)
                .map_err(|e| Error::Custom(format!("stack reference output {key:?}: {e}"))),
        }
    }
}

/// A builder for creating a stack reference.
///
/// The `stack_name` should be the fully-qualified stack name, typically
/// in the form `"org/project/stack"`.
pub struct StackReferenceBuilder {
    ctx: Context,
    name: String,
    stack_name: String,
    opts: ResourceOptions,
}

impl StackReferenceBuilder {
    /// Creates a new stack reference builder.
    ///
    /// `stack_name` is the fully-qualified name of the stack to reference
    /// (e.g. `"org/project/stack"`). This is used as both the resource name
    /// and the stack identifier.
    pub fn new(ctx: &Context, stack_name: impl Into<String>) -> Self {
        let stack_name = stack_name.into();
        StackReferenceBuilder {
            ctx: ctx.clone(),
            name: stack_name.clone(),
            stack_name,
            opts: ResourceOptions::default(),
        }
    }

    /// Overrides the resource name (defaults to the stack name).
    pub fn resource_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Sets the parent resource URN.
    pub fn parent(mut self, urn: impl Into<String>) -> Self {
        self.opts.parent = Some(urn.into());
        self
    }

    /// Adds a dependency on another resource.
    pub fn depends_on(mut self, urn: impl Into<String>) -> Self {
        self.opts.depends_on.push(urn.into());
        self
    }
}

const STACK_REFERENCE_TYPE: &str = "pulumi:pulumi:StackReference";

impl IntoFuture for StackReferenceBuilder {
    type Output = Result<StackReference>;
    type IntoFuture = impl Future<Output = Self::Output> + Send;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let inputs = serde_json::json!({ "name": self.stack_name });
            let result = register_resource_inner(
                &self.ctx,
                STACK_REFERENCE_TYPE,
                &self.name,
                inputs,
                &self.opts,
                true,  // custom resource
                false, // not remote
            )
            .await?;

            // The outputs from a StackReference include all the referenced
            // stack's exports under "outputs", plus "name" and
            // "secretOutputNames". We expose the raw outputs object so users
            // can access individual keys.
            let stack_outputs = result
                .outputs
                .get("outputs")
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));

            Ok(StackReference {
                urn: result.urn,
                outputs: stack_outputs,
            })
        }
    }
}
