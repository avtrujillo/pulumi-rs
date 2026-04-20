use std::future::{Future, IntoFuture};

use crate::connection::{EngineConnection, GrpcEngine, GrpcMonitor, MonitorConnection};
use crate::context::Context;
use crate::error::{Error, Result};
use crate::resource::{ResourceOptions, register_resource_inner};

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
        let val = self
            .get_output(key)
            .ok_or_else(|| Error::Custom(format!("stack reference output {key:?} not found")))?;
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
pub struct StackReferenceBuilder<
    M: MonitorConnection = GrpcMonitor,
    E: EngineConnection = GrpcEngine,
> {
    ctx: Context<M, E>,
    name: String,
    stack_name: String,
    opts: ResourceOptions,
}

impl<M: MonitorConnection, E: EngineConnection> StackReferenceBuilder<M, E> {
    /// Creates a new stack reference builder.
    ///
    /// `stack_name` is the fully-qualified name of the stack to reference
    /// (e.g. `"org/project/stack"`). This is used as both the resource name
    /// and the stack identifier.
    pub fn new(ctx: &Context<M, E>, stack_name: impl Into<String>) -> Self {
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

pub(crate) const STACK_REFERENCE_TYPE: &str = "pulumi:pulumi:StackReference";

impl<M: MonitorConnection, E: EngineConnection> IntoFuture for StackReferenceBuilder<M, E> {
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

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::resource::Resource;
    use crate::test_support::TestContextBuilder;

    // Fake Resource with the StackReference type token so TestContextBuilder
    // can inject canned responses for the builder tests.
    struct FakeStackRef;
    #[derive(Serialize)]
    struct FakeArgs {}
    #[derive(Deserialize, Clone)]
    struct FakeOutputs {}
    impl Resource for FakeStackRef {
        const TYPE_TOKEN: &'static str = STACK_REFERENCE_TYPE;
        type Inputs = FakeArgs;
        type Outputs = FakeOutputs;
    }

    fn make_ref() -> StackReference {
        StackReference {
            urn: "urn:test".into(),
            outputs: serde_json::json!({
                "vpcId": "vpc-123",
                "count": 5,
                "enabled": true
            }),
        }
    }

    // --- StackReference::get_output ---

    #[test]
    fn test_get_output_present() {
        let sr = make_ref();
        assert_eq!(sr.get_output("vpcId"), Some(&serde_json::json!("vpc-123")));
    }

    #[test]
    fn test_get_output_missing() {
        let sr = make_ref();
        assert_eq!(sr.get_output("notThere"), None);
    }

    // --- StackReference::require_output ---

    #[test]
    fn test_require_output_present() {
        let sr = make_ref();
        let v: String = sr.require_output("vpcId").unwrap();
        assert_eq!(v, "vpc-123");
    }

    #[test]
    fn test_require_output_missing_is_err() {
        let sr = make_ref();
        assert!(sr.require_output::<String>("missing").is_err());
    }

    #[test]
    fn test_require_output_wrong_type_is_err() {
        let sr = make_ref();
        // "vpcId" is a String, not an i64
        assert!(sr.require_output::<i64>("vpcId").is_err());
    }

    // --- StackReference::get_output_typed ---

    #[test]
    fn test_get_output_typed_present() {
        let sr = make_ref();
        let v: Option<i64> = sr.get_output_typed("count").unwrap();
        assert_eq!(v, Some(5));
    }

    #[test]
    fn test_get_output_typed_missing_is_none() {
        let sr = make_ref();
        let v: Option<String> = sr.get_output_typed("missing").unwrap();
        assert_eq!(v, None);
    }

    #[test]
    fn test_get_output_typed_wrong_type_is_err() {
        let sr = make_ref();
        // "vpcId" is a String, not an i64
        assert!(sr.get_output_typed::<i64>("vpcId").is_err());
    }

    // --- StackReferenceBuilder ---

    #[tokio::test]
    async fn test_builder_extracts_outputs_from_wrapper() {
        let tc = TestContextBuilder::new()
            .with_resource_response::<FakeStackRef>(
                "org/proj/infra",
                serde_json::json!({ "outputs": { "vpcId": "vpc-456" }, "name": "org/proj/infra" }),
            )
            .build();
        let sr = StackReferenceBuilder::new(tc.context(), "org/proj/infra")
            .await
            .unwrap();
        assert_eq!(sr.get_output("vpcId"), Some(&serde_json::json!("vpc-456")));
    }

    #[tokio::test]
    async fn test_builder_resource_name_override() {
        let tc = TestContextBuilder::new()
            .with_resource_response::<FakeStackRef>(
                "custom-ref-name",
                serde_json::json!({ "outputs": {} }),
            )
            .build();
        let sr = StackReferenceBuilder::new(tc.context(), "org/proj/infra")
            .resource_name("custom-ref-name")
            .await
            .unwrap();
        let regs = tc.registered_resources();
        // resource name is the override, not the stack name
        assert_eq!(regs[0].name, "custom-ref-name");
        // inputs still use the original stack name
        assert_eq!(regs[0].inputs["name"], "org/proj/infra");
        assert!(!sr.urn.is_empty());
    }

    #[tokio::test]
    async fn test_builder_urn_uses_stack_reference_type() {
        let tc = TestContextBuilder::new().build();
        let sr = StackReferenceBuilder::new(tc.context(), "org/proj/infra")
            .await
            .unwrap();
        assert!(sr.urn.contains(STACK_REFERENCE_TYPE));
    }
}
