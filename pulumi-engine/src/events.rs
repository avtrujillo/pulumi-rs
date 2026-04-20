//! Engine event types emitted during stack operations.
//!
//! Events are collected via an [`EventCollector`] shared between the
//! orchestrator and the gRPC service implementations. The orchestrator emits
//! [`EngineEvent::Prelude`] and [`EngineEvent::Summary`] directly; the gRPC
//! services emit [`EngineEvent::ResourceStep`] and [`EngineEvent::Diagnostic`]
//! as RPCs arrive from the user program.

use std::sync::{Arc, Mutex};

/// A structured event emitted by the native engine during a stack operation.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Emitted once at the start of an operation with the current config.
    Prelude {
        config: serde_json::Value,
    },
    /// Emitted for each resource step (create, update, same, delete).
    ResourceStep {
        op: String,
        urn: String,
        resource_type: String,
        /// Prior inputs (Null for creates).
        old_inputs: serde_json::Value,
        /// Prior outputs (Null for creates).
        old_outputs: serde_json::Value,
        /// Current inputs (Null for deletes).
        new_inputs: serde_json::Value,
        /// Current outputs (Null for deletes or when not yet known).
        new_outputs: serde_json::Value,
    },
    /// A diagnostic message from the user program via the Engine.Log RPC.
    Diagnostic {
        urn: String,
        severity: String,
        message: String,
    },
    /// Emitted once at the end of an operation.
    Summary {
        may_update: bool,
        duration_seconds: i64,
        /// Resource changes by op, e.g. `{"create": 3, "same": 1}`.
        resource_changes: serde_json::Value,
    },
}

/// Shared event list used to collect events across async tasks.
pub type EventCollector = Arc<Mutex<Vec<EngineEvent>>>;

pub fn new_collector() -> EventCollector {
    Arc::new(Mutex::new(Vec::new()))
}

pub fn emit(collector: &EventCollector, event: EngineEvent) {
    collector.lock().unwrap().push(event);
}

/// Returns a snapshot of all events collected so far.
pub fn drain(collector: &EventCollector) -> Vec<EngineEvent> {
    collector.lock().unwrap().clone()
}

/// Derive the resource_changes map from already-collected ResourceStep events.
pub fn count_resource_changes(events: &[EngineEvent]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for ev in events {
        if let EngineEvent::ResourceStep { op, .. } = ev {
            let entry = map
                .entry(op.clone())
                .or_insert(serde_json::Value::from(0i64));
            if let serde_json::Value::Number(n) = entry {
                let new_n = n.as_i64().unwrap_or(0) + 1;
                *entry = serde_json::Value::from(new_n);
            }
        }
    }
    serde_json::Value::Object(map)
}
