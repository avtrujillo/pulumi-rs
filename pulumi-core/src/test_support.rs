//! Utilities for unit-testing Pulumi programs without a running engine.
//!
//! # Example
//!
//! ```ignore
//! use pulumi_core::test_support::TestContextBuilder;
//! use pulumi_core::resource::ResourceBuilder;
//!
//! let test_ctx = TestContextBuilder::new()
//!     .with_resource_response::<MyBucket>(
//!         "my-bucket",
//!         serde_json::json!({ "arn": "arn:aws:s3:::my-bucket" }),
//!     )
//!     .build();
//!
//! // Run program logic
//! let result = ResourceBuilder::<MyBucket>::new(test_ctx.context(), "my-bucket", args)
//!     .await
//!     .unwrap();
//! assert_eq!(result.outputs.arn, "arn:aws:s3:::my-bucket");
//!
//! // Inspect what was registered
//! let regs = test_ctx.registered_resources();
//! assert_eq!(regs[0].type_token, "aws:s3/bucket:Bucket");
//! assert_eq!(regs[0].name, "my-bucket");
//! ```

use std::collections::HashMap;

use crate::connection::{
    InvokeRecording, LogRecord, MockEngine, MockMonitor, ResourceRegistration,
};
use crate::context::{Context, Settings};
use crate::resource::Resource;

/// Fluent builder for [`TestContext`].
pub struct TestContextBuilder {
    project: String,
    stack: String,
    preview: bool,
    config: HashMap<String, String>,
    config_secret_keys: Vec<String>,
    responses: HashMap<(String, String), serde_json::Value>,
    errors: HashMap<(String, String), String>,
}

impl Default for TestContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestContextBuilder {
    pub fn new() -> Self {
        Self {
            project: "test".into(),
            stack: "dev".into(),
            preview: false,
            config: HashMap::new(),
            config_secret_keys: Vec::new(),
            responses: HashMap::new(),
            errors: HashMap::new(),
        }
    }

    /// Sets the project name (default: `"test"`).
    pub fn project(mut self, project: impl Into<String>) -> Self {
        self.project = project.into();
        self
    }

    /// Sets the stack name (default: `"dev"`).
    pub fn stack(mut self, stack: impl Into<String>) -> Self {
        self.stack = stack.into();
        self
    }

    /// Enables preview mode. The context will report `is_dry_run() == true` and
    /// resource registrations will return empty outputs.
    pub fn preview(mut self) -> Self {
        self.preview = true;
        self
    }

    /// Adds a configuration value accessible via `ctx.get_config(key)`.
    pub fn with_config(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.insert(key.into(), value.into());
        self
    }

    /// Marks a config key as secret (returned by `ctx.is_config_secret(key)`).
    pub fn with_secret_config(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let key = key.into();
        self.config.insert(key.clone(), value.into());
        self.config_secret_keys.push(key);
        self
    }

    /// Registers canned outputs for a specific resource type and name.
    ///
    /// When the program registers a resource of type `R` named `name`, the mock
    /// monitor returns `outputs` instead of echoing the inputs. Pass a
    /// `serde_json::json!({...})` literal matching the shape of `R::Outputs`.
    pub fn with_resource_response<R: Resource>(
        mut self,
        name: impl Into<String>,
        outputs: serde_json::Value,
    ) -> Self {
        self.responses
            .insert((R::TYPE_TOKEN.to_string(), name.into()), outputs);
        self
    }

    /// Injects an error for a specific resource type and name.
    ///
    /// When the program registers a resource of type `R` named `name`, the mock
    /// monitor returns an error containing `message`. Use this to test error-handling
    /// paths in your Pulumi programs.
    pub fn with_resource_error<R: Resource>(
        mut self,
        name: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        self.errors
            .insert((R::TYPE_TOKEN.to_string(), name.into()), message.into());
        self
    }

    /// Builds the [`TestContext`].
    pub fn build(self) -> TestContext {
        let settings = Settings {
            monitor_addr: String::new(),
            engine_addr: String::new(),
            project: self.project.clone(),
            stack: self.stack.clone(),
            dry_run: self.preview,
            parallel: -1,
            organization: String::new(),
            config: self.config,
            config_secret_keys: self.config_secret_keys,
        };
        let monitor = MockMonitor::with_options(
            self.project,
            self.stack,
            self.responses,
            self.errors,
            self.preview,
        );
        let engine = MockEngine::new();
        let ctx = Context::for_testing(monitor.clone(), engine.clone(), settings);
        TestContext { ctx, monitor, engine }
    }
}

/// A Pulumi program context backed by mock connections for unit testing.
///
/// Create via [`TestContextBuilder`] (or the convenience alias `TestContext::builder()`),
/// pass `test_ctx.context()` into your program logic, then call
/// `test_ctx.registered_resources()` to assert on what was registered.
pub struct TestContext {
    ctx: Context<MockMonitor, MockEngine>,
    monitor: MockMonitor,
    engine: MockEngine,
}

impl TestContext {
    /// Returns a [`TestContextBuilder`] with default settings.
    pub fn builder() -> TestContextBuilder {
        TestContextBuilder::new()
    }

    /// Returns a reference to the underlying context for use in Pulumi programs.
    pub fn context(&self) -> &Context<MockMonitor, MockEngine> {
        &self.ctx
    }

    /// Returns all `register_resource` calls made against this context so far.
    pub fn registered_resources(&self) -> Vec<ResourceRegistration> {
        self.monitor.recorded_registrations()
    }

    /// Returns all `invoke` (provider function) calls made against this context so far.
    pub fn invoked_functions(&self) -> Vec<InvokeRecording> {
        self.monitor.recorded_invocations()
    }

    /// Returns all log messages sent to the engine from this context so far.
    pub fn recorded_logs(&self) -> Vec<LogRecord> {
        self.engine.recorded_logs()
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::resource::{RegisteredResource, ResourceBuilder};

    struct TestBucket;

    #[derive(Serialize)]
    struct TestBucketArgs {
        name: String,
    }

    #[derive(Deserialize, Clone)]
    struct TestBucketOutputs {
        arn: Option<String>,
    }

    impl Resource for TestBucket {
        const TYPE_TOKEN: &'static str = "test:index:Bucket";
        type Inputs = TestBucketArgs;
        type Outputs = TestBucketOutputs;
    }

    #[tokio::test]
    async fn test_basic_registration_recorded() {
        let test_ctx = TestContextBuilder::new().build();

        let _: RegisteredResource<TestBucket> = ResourceBuilder::new(
            test_ctx.context(),
            "my-bucket",
            TestBucketArgs { name: "test".into() },
        )
        .await
        .unwrap();

        let regs = test_ctx.registered_resources();
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].type_token, "test:index:Bucket");
        assert_eq!(regs[0].name, "my-bucket");
        assert!(regs[0].custom);
    }

    #[tokio::test]
    async fn test_urn_format() {
        let test_ctx = TestContextBuilder::new()
            .project("my-proj")
            .stack("staging")
            .build();

        let result: RegisteredResource<TestBucket> = ResourceBuilder::new(
            test_ctx.context(),
            "my-bucket",
            TestBucketArgs { name: "test".into() },
        )
        .await
        .unwrap();

        assert_eq!(
            result.urn,
            "urn:pulumi:staging::my-proj::test:index:Bucket::my-bucket"
        );
    }

    #[tokio::test]
    async fn test_canned_response() {
        let test_ctx = TestContextBuilder::new()
            .with_resource_response::<TestBucket>(
                "my-bucket",
                serde_json::json!({ "arn": "arn:test:::my-bucket" }),
            )
            .build();

        let result: RegisteredResource<TestBucket> = ResourceBuilder::new(
            test_ctx.context(),
            "my-bucket",
            TestBucketArgs { name: "test".into() },
        )
        .await
        .unwrap();

        assert_eq!(result.outputs.arn.as_deref(), Some("arn:test:::my-bucket"));
    }

    #[tokio::test]
    async fn test_unconfigured_resource_echoes_inputs() {
        let test_ctx = TestContextBuilder::new().build();

        // No canned response — outputs are echoed from inputs.
        // TestBucketOutputs.arn is Option<String>, so a missing field => None.
        let result: RegisteredResource<TestBucket> = ResourceBuilder::new(
            test_ctx.context(),
            "my-bucket",
            TestBucketArgs { name: "test".into() },
        )
        .await
        .unwrap();

        // Inputs have no "arn" field, so it deserializes as None.
        assert_eq!(result.outputs.arn, None);
    }

    #[tokio::test]
    async fn test_preview_mode() {
        let test_ctx = TestContextBuilder::new().preview().build();

        assert!(test_ctx.context().is_dry_run());

        // Registration still succeeds and is recorded.
        let result: RegisteredResource<TestBucket> = ResourceBuilder::new(
            test_ctx.context(),
            "my-bucket",
            TestBucketArgs { name: "test".into() },
        )
        .await
        .unwrap();

        // Empty outputs in preview mode.
        assert_eq!(result.outputs.arn, None);

        let regs = test_ctx.registered_resources();
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].name, "my-bucket");
    }

    #[tokio::test]
    async fn test_multiple_registrations() {
        let test_ctx = TestContextBuilder::new().build();
        let ctx = test_ctx.context();

        let _: RegisteredResource<TestBucket> =
            ResourceBuilder::new(ctx, "bucket-a", TestBucketArgs { name: "a".into() })
                .await
                .unwrap();
        let _: RegisteredResource<TestBucket> =
            ResourceBuilder::new(ctx, "bucket-b", TestBucketArgs { name: "b".into() })
                .await
                .unwrap();

        let regs = test_ctx.registered_resources();
        assert_eq!(regs.len(), 2);
        assert_eq!(regs[0].name, "bucket-a");
        assert_eq!(regs[1].name, "bucket-b");
    }

    #[tokio::test]
    async fn test_config_access() {
        let test_ctx = TestContextBuilder::new()
            .with_config("app:region", "us-east-1")
            .with_secret_config("app:token", "secret-value")
            .build();

        let ctx = test_ctx.context();
        assert_eq!(ctx.get_config("app:region"), Some("us-east-1"));
        assert_eq!(ctx.get_config("app:token"), Some("secret-value"));
        assert!(ctx.is_config_secret("app:token"));
        assert!(!ctx.is_config_secret("app:region"));
    }

    #[tokio::test]
    async fn test_registration_records_inputs() {
        let test_ctx = TestContextBuilder::new().build();

        let _: RegisteredResource<TestBucket> = ResourceBuilder::new(
            test_ctx.context(),
            "my-bucket",
            TestBucketArgs { name: "recorded-name".into() },
        )
        .await
        .unwrap();

        let regs = test_ctx.registered_resources();
        assert_eq!(regs[0].inputs["name"], "recorded-name");
    }

    #[tokio::test]
    async fn test_protect_recorded() {
        let test_ctx = TestContextBuilder::new().build();

        let _: RegisteredResource<TestBucket> = ResourceBuilder::new(
            test_ctx.context(),
            "protected",
            TestBucketArgs { name: "x".into() },
        )
        .protect()
        .await
        .unwrap();

        let regs = test_ctx.registered_resources();
        assert!(regs[0].protect);
    }

    #[tokio::test]
    async fn test_builder_alias() {
        // TestContext::builder() is an alias for TestContextBuilder::new()
        let test_ctx = TestContext::builder().build();
        assert_eq!(test_ctx.context().project(), "test");
        assert_eq!(test_ctx.context().stack(), "dev");
    }

    #[tokio::test]
    async fn test_with_resource_error_propagates() {
        let test_ctx = TestContextBuilder::new()
            .with_resource_error::<TestBucket>("bad-bucket", "simulated provider failure")
            .build();

        let result: crate::error::Result<RegisteredResource<TestBucket>> = ResourceBuilder::new(
            test_ctx.context(),
            "bad-bucket",
            TestBucketArgs { name: "x".into() },
        )
        .await;
        let Err(err) = result else { panic!("expected an error") };
        assert!(err.to_string().contains("simulated provider failure"));
    }

    #[tokio::test]
    async fn test_with_resource_error_only_affects_named_resource() {
        let test_ctx = TestContextBuilder::new()
            .with_resource_error::<TestBucket>("bad-bucket", "fail")
            .build();

        // Different name — should succeed
        let result: crate::error::Result<RegisteredResource<TestBucket>> = ResourceBuilder::new(
            test_ctx.context(),
            "good-bucket",
            TestBucketArgs { name: "x".into() },
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_recorded_logs_captures_engine_log() {
        use crate::log;
        let test_ctx = TestContextBuilder::new().build();
        log::info(test_ctx.context(), "test message", None).await.unwrap();
        let logs = test_ctx.recorded_logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message, "test message");
        assert_eq!(logs[0].severity, 1); // INFO
    }
}
