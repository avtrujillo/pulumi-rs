//! Structured engine events emitted during stack operations.

use serde::Deserialize;

/// A Pulumi engine event emitted during `up`, `preview`, `destroy`, or `refresh`.
///
/// When `--event-log` is used, the CLI writes one JSON object per line. Each
/// object has exactly one of these event fields set.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineEvent {
    /// Incremented for each event in a given operation.
    #[serde(default)]
    pub sequence: i64,

    /// Emitted once at the start of an operation.
    pub prelude_event: Option<PreludeEvent>,

    /// Emitted for each resource step (create, update, delete, same, etc.).
    pub resource_pre_event: Option<ResourcePreEvent>,

    /// Emitted once an operation completes.
    pub summary_event: Option<SummaryEvent>,

    /// Diagnostic messages (info, warning, error).
    pub diagnostic_event: Option<DiagnosticEvent>,
}

/// Emitted at the start of an operation with configuration info.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreludeEvent {
    /// The configuration values for this operation.
    pub config: serde_json::Value,
}

/// Emitted before a resource step is executed.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePreEvent {
    /// The step metadata.
    pub metadata: StepEventMetadata,
}

/// Metadata about a single resource step.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepEventMetadata {
    /// The operation type: "create", "update", "delete", "same", etc.
    pub op: String,
    /// The resource URN.
    pub urn: String,
    /// The resource type token.
    #[serde(rename = "type")]
    pub resource_type: String,
    /// The old state (before the operation), if any.
    pub old: Option<StepEventStateMetadata>,
    /// The new state (after the operation), if any.
    pub new: Option<StepEventStateMetadata>,
}

/// State metadata attached to a step event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepEventStateMetadata {
    /// The resource type token.
    #[serde(rename = "type")]
    pub resource_type: String,
    /// The resource URN.
    pub urn: String,
    /// The input properties.
    #[serde(default)]
    pub inputs: serde_json::Value,
    /// The output properties.
    #[serde(default)]
    pub outputs: serde_json::Value,
}

/// Emitted at the end of an operation with a summary.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryEvent {
    /// Whether the operation may have changed resources.
    #[serde(default)]
    pub may_update: bool,
    /// Duration in seconds.
    #[serde(default)]
    pub duration_seconds: i64,
    /// Resource changes by operation type (e.g. `{"create": 3, "same": 1}`).
    #[serde(default)]
    pub resource_changes: serde_json::Value,
}

/// A diagnostic message from the engine.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEvent {
    /// The URN this diagnostic is associated with, if any.
    #[serde(default)]
    pub urn: String,
    /// The severity: "info", "warning", "error".
    pub severity: String,
    /// The diagnostic message.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_prelude_event() {
        let json = r#"{"sequence": 1, "preludeEvent": {"config": {"aws:region": "us-east-1"}}}"#;
        let event: EngineEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.sequence, 1);
        let prelude = event.prelude_event.expect("prelude_event should be Some");
        assert_eq!(prelude.config["aws:region"], "us-east-1");
        assert!(event.resource_pre_event.is_none());
        assert!(event.summary_event.is_none());
        assert!(event.diagnostic_event.is_none());
    }

    #[test]
    fn deserialize_resource_pre_event() {
        let json = r#"{
            "sequence": 2,
            "resourcePreEvent": {
                "metadata": {
                    "op": "create",
                    "urn": "urn:pulumi:dev::project::aws:s3/bucket:Bucket::my-bucket",
                    "type": "aws:s3/bucket:Bucket",
                    "new": {
                        "type": "aws:s3/bucket:Bucket",
                        "urn": "urn:pulumi:dev::project::aws:s3/bucket:Bucket::my-bucket",
                        "inputs": {"bucket": "my-bucket"},
                        "outputs": {}
                    }
                }
            }
        }"#;
        let event: EngineEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.sequence, 2);
        let rpe = event
            .resource_pre_event
            .expect("resource_pre_event should be Some");
        assert_eq!(rpe.metadata.op, "create");
        assert_eq!(
            rpe.metadata.urn,
            "urn:pulumi:dev::project::aws:s3/bucket:Bucket::my-bucket"
        );
        assert_eq!(rpe.metadata.resource_type, "aws:s3/bucket:Bucket");
        assert!(rpe.metadata.old.is_none());
        let new_state = rpe.metadata.new.expect("new state should be Some");
        assert_eq!(new_state.resource_type, "aws:s3/bucket:Bucket");
        assert_eq!(new_state.inputs["bucket"], "my-bucket");
    }

    #[test]
    fn deserialize_summary_event() {
        let json = r#"{"sequence": 3, "summaryEvent": {"mayUpdate": true, "durationSeconds": 5, "resourceChanges": {"create": 1}}}"#;
        let event: EngineEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.sequence, 3);
        let summary = event.summary_event.expect("summary_event should be Some");
        assert!(summary.may_update);
        assert_eq!(summary.duration_seconds, 5);
        assert_eq!(summary.resource_changes["create"], 1);
    }

    #[test]
    fn deserialize_diagnostic_event() {
        let json =
            r#"{"sequence": 4, "diagnosticEvent": {"urn": "", "severity": "info", "message": "hello"}}"#;
        let event: EngineEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.sequence, 4);
        let diag = event
            .diagnostic_event
            .expect("diagnostic_event should be Some");
        assert_eq!(diag.urn, "");
        assert_eq!(diag.severity, "info");
        assert_eq!(diag.message, "hello");
    }

    #[test]
    fn deserialize_event_with_only_sequence() {
        let json = r#"{"sequence": 0}"#;
        let event: EngineEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.sequence, 0);
        assert!(event.prelude_event.is_none());
        assert!(event.resource_pre_event.is_none());
        assert!(event.summary_event.is_none());
        assert!(event.diagnostic_event.is_none());
    }
}
