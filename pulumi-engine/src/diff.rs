//! Resource diff logic — compares old vs new resource state to determine actions.

use crate::state::ResourceState;

/// The action to take for a resource during an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceAction {
    /// Resource is new and needs to be created.
    Create,
    /// Resource exists and its inputs have changed.
    Update,
    /// Resource exists and its inputs are unchanged.
    Same,
}

/// The result of diffing a resource against its prior state.
#[derive(Debug, Clone)]
pub struct ResourceDiff {
    pub urn: String,
    pub action: ResourceAction,
    /// The prior state, if the resource existed before.
    pub old: Option<ResourceState>,
}

/// Diff a newly registered resource against optional prior state.
///
/// Compares inputs to determine if the resource needs to be created, updated,
/// or is unchanged.
pub fn diff_resource(
    urn: &str,
    new_inputs: &serde_json::Value,
    prior: Option<&ResourceState>,
    ignore_changes: &[String],
) -> ResourceDiff {
    match prior {
        None => ResourceDiff {
            urn: urn.to_string(),
            action: ResourceAction::Create,
            old: None,
        },
        Some(old) => {
            let inputs_changed = has_changes(&old.inputs, new_inputs, ignore_changes);
            ResourceDiff {
                urn: urn.to_string(),
                action: if inputs_changed {
                    ResourceAction::Update
                } else {
                    ResourceAction::Same
                },
                old: Some(old.clone()),
            }
        }
    }
}

/// Check if inputs have changed, ignoring specified property paths.
fn has_changes(
    old: &serde_json::Value,
    new: &serde_json::Value,
    ignore_changes: &[String],
) -> bool {
    if ignore_changes.is_empty() {
        return old != new;
    }

    // Compare objects key-by-key, skipping ignored keys.
    match (old, new) {
        (serde_json::Value::Object(old_map), serde_json::Value::Object(new_map)) => {
            // Check for keys in new that differ from old (excluding ignored).
            for (k, new_v) in new_map {
                if ignore_changes.iter().any(|ig| ig == k) {
                    continue;
                }
                match old_map.get(k) {
                    Some(old_v) if old_v == new_v => {}
                    Some(_) => return true, // Changed
                    None => return true,     // Added
                }
            }
            // Check for keys in old that are missing from new (excluding ignored).
            for k in old_map.keys() {
                if ignore_changes.iter().any(|ig| ig == k) {
                    continue;
                }
                if !new_map.contains_key(k) {
                    return true; // Removed
                }
            }
            false
        }
        _ => old != new,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_new_resource() {
        let diff = diff_resource(
            "urn:pulumi:dev::proj::pkg:mod:Res::name",
            &serde_json::json!({"key": "value"}),
            None,
            &[],
        );
        assert_eq!(diff.action, ResourceAction::Create);
        assert!(diff.old.is_none());
    }

    #[test]
    fn test_diff_same_resource() {
        let prior = ResourceState {
            urn: "urn:pulumi:dev::proj::pkg:mod:Res::name".into(),
            id: "id-123".into(),
            resource_type: "pkg:mod:Res".into(),
            name: "name".into(),
            custom: true,
            parent: String::new(),
            inputs: serde_json::json!({"key": "value"}),
            outputs: serde_json::json!({"key": "value"}),
            dependencies: vec![],
        };
        let diff = diff_resource(
            &prior.urn,
            &serde_json::json!({"key": "value"}),
            Some(&prior),
            &[],
        );
        assert_eq!(diff.action, ResourceAction::Same);
    }

    #[test]
    fn test_diff_updated_resource() {
        let prior = ResourceState {
            urn: "urn:pulumi:dev::proj::pkg:mod:Res::name".into(),
            id: "id-123".into(),
            resource_type: "pkg:mod:Res".into(),
            name: "name".into(),
            custom: true,
            parent: String::new(),
            inputs: serde_json::json!({"key": "old-value"}),
            outputs: serde_json::json!({"key": "old-value"}),
            dependencies: vec![],
        };
        let diff = diff_resource(
            &prior.urn,
            &serde_json::json!({"key": "new-value"}),
            Some(&prior),
            &[],
        );
        assert_eq!(diff.action, ResourceAction::Update);
    }

    #[test]
    fn test_diff_ignore_changes() {
        let prior = ResourceState {
            urn: "urn:pulumi:dev::proj::pkg:mod:Res::name".into(),
            id: "id-123".into(),
            resource_type: "pkg:mod:Res".into(),
            name: "name".into(),
            custom: true,
            parent: String::new(),
            inputs: serde_json::json!({"key": "old", "ignored": "old"}),
            outputs: serde_json::json!({}),
            dependencies: vec![],
        };
        // Only "ignored" changed, and it's in ignore_changes.
        let diff = diff_resource(
            &prior.urn,
            &serde_json::json!({"key": "old", "ignored": "new"}),
            Some(&prior),
            &["ignored".into()],
        );
        assert_eq!(diff.action, ResourceAction::Same);

        // "key" also changed, so it should be an update.
        let diff = diff_resource(
            &prior.urn,
            &serde_json::json!({"key": "new", "ignored": "new"}),
            Some(&prior),
            &["ignored".into()],
        );
        assert_eq!(diff.action, ResourceAction::Update);
    }
}
