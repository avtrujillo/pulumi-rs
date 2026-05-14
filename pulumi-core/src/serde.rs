//! Conversion between `serde_json::Value` and protobuf `google.protobuf.Struct`/`Value`.
//!
//! Pulumi sends resource properties as protobuf Struct values over gRPC. This module
//! handles the bidirectional conversion with JSON, which is the natural Rust interchange
//! format via `serde`.

use prost_types::value::Kind;
use prost_types::{ListValue, Struct, Value};
use serde_json::Map;

/// Special signature key shared by all Pulumi special wire-format types (secrets, assets,
/// archives, resource references, output values). The value at this key identifies which type.
pub const SECRET_SIG: &str = "4dabf18193072939515e22adb298388d";
/// Value at [`SECRET_SIG`] that marks a struct as a secret wrapper.
pub const SECRET_VALUE_SIG: &str = "1b47061264138c4ac30d75fd1eb44270";
/// Sentinel UUID used as a bare string to represent an unknown value during preview.
pub const UNKNOWN_SIG: &str = "04da6b54-80e4-46f7-96ec-b56ff0331ba9";

/// Converts a `serde_json::Value` to a protobuf `Value`.
pub fn json_to_proto_value(json: &serde_json::Value) -> Value {
    Value {
        kind: Some(json_to_kind(json)),
    }
}

fn json_to_kind(json: &serde_json::Value) -> Kind {
    match json {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(*b),
        serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Kind::StringValue(s.clone()),
        serde_json::Value::Array(arr) => Kind::ListValue(ListValue {
            values: arr.iter().map(json_to_proto_value).collect(),
        }),
        serde_json::Value::Object(map) => Kind::StructValue(json_map_to_struct(map)),
    }
}

/// Converts a JSON object (map) to a protobuf Struct.
pub fn json_map_to_struct(map: &Map<String, serde_json::Value>) -> Struct {
    Struct {
        fields: map
            .iter()
            .map(|(k, v)| (k.clone(), json_to_proto_value(v)))
            .collect(),
    }
}

/// Converts a `serde_json::Value::Object` to a protobuf `Struct`.
/// Returns an empty struct if the value is not an object.
pub fn json_to_struct(json: &serde_json::Value) -> Struct {
    match json {
        serde_json::Value::Object(map) => json_map_to_struct(map),
        _ => Struct::default(),
    }
}

/// Converts a protobuf `Value` to a `serde_json::Value`.
pub fn proto_value_to_json(value: &Value) -> serde_json::Value {
    match &value.kind {
        None | Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::NumberValue(n)) => {
            // Preserve integers when the float has no fractional part.
            if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                serde_json::Value::Number((*n as i64).into())
            } else {
                serde_json::Value::Number(
                    serde_json::Number::from_f64(*n).unwrap_or_else(|| 0.into()),
                )
            }
        }
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::ListValue(list)) => {
            serde_json::Value::Array(list.values.iter().map(proto_value_to_json).collect())
        }
        Some(Kind::StructValue(s)) => struct_to_json(s),
    }
}

/// Converts a protobuf `Struct` to a `serde_json::Value::Object`.
pub fn struct_to_json(s: &Struct) -> serde_json::Value {
    let map: Map<String, serde_json::Value> = s
        .fields
        .iter()
        .map(|(k, v)| (k.clone(), proto_value_to_json(v)))
        .collect();
    serde_json::Value::Object(map)
}

/// Wraps a value as a Pulumi secret in the wire format.
pub fn wrap_secret(value: serde_json::Value) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(
        SECRET_SIG.to_string(),
        serde_json::Value::String(SECRET_VALUE_SIG.to_string()),
    );
    map.insert("value".to_string(), value);
    serde_json::Value::Object(map)
}

/// Checks if a protobuf Struct represents a Pulumi secret wrapper.
///
/// Returns `true` only when the struct has the special signature key with the secret-specific
/// value. A key-only check produces false positives for assets, archives, and resource refs,
/// which all share the same key but carry a different marker value.
pub fn is_secret(s: &Struct) -> bool {
    s.fields
        .get(SECRET_SIG)
        .and_then(|v| match &v.kind {
            Some(Kind::StringValue(s)) => Some(s.as_str()),
            _ => None,
        })
        .is_some_and(|v| v == SECRET_VALUE_SIG)
}

/// Unwraps a Pulumi secret, returning the inner value.
pub fn unwrap_secret(s: &Struct) -> Option<&Value> {
    if is_secret(s) {
        s.fields.get("value")
    } else {
        None
    }
}

/// Checks if a protobuf Value represents an unknown (preview-time placeholder).
///
/// Unknown values are encoded as a bare `StringValue` containing [`UNKNOWN_SIG`], not as a
/// `Struct`. The previous `&Struct` signature could never return `true`; this fixes it.
pub fn is_unknown(v: &Value) -> bool {
    matches!(&v.kind, Some(Kind::StringValue(s)) if s == UNKNOWN_SIG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_to_struct_roundtrip() {
        let json = serde_json::json!({
            "name": "my-bucket",
            "tags": {"env": "dev"},
            "count": 42,
            "enabled": true,
            "items": [1, 2, 3],
            "nothing": null
        });

        let s = json_to_struct(&json);
        let back = struct_to_json(&s);
        assert_eq!(json, back);
    }

    /// Wrap a value as a secret, convert to proto Struct, and verify detection + unwrapping.
    #[test]
    fn test_secret_wrapping() {
        let val = serde_json::json!("my-password");
        let wrapped = wrap_secret(val.clone());

        let s = json_to_struct(&wrapped);
        assert!(is_secret(&s));

        let unwrapped = unwrap_secret(&s).unwrap();
        assert_eq!(proto_value_to_json(unwrapped), val);
    }

    /// A struct with the shared signature key but an asset marker value must not be detected
    /// as a secret — guards against the false-positive that a key-only check would produce.
    #[test]
    fn test_is_secret_no_false_positive_for_asset() {
        let asset_sig_value = "c44067f5952c0a294b673a41bacd8c17";
        let asset_like = serde_json::json!({
            "4dabf18193072939515e22adb298388d": asset_sig_value,
            "path": "/some/file"
        });
        let s = json_to_struct(&asset_like);
        assert!(!is_secret(&s), "asset-like struct should not be detected as secret");
    }

    /// A bare StringValue containing the unknown sentinel UUID must be detected as unknown.
    #[test]
    fn test_is_unknown_detects_sentinel_string() {
        let unknown = Value {
            kind: Some(Kind::StringValue(UNKNOWN_SIG.to_string())),
        };
        assert!(is_unknown(&unknown));
    }

    /// A StringValue with any other content is not unknown.
    #[test]
    fn test_is_unknown_false_for_regular_string() {
        let val = Value {
            kind: Some(Kind::StringValue("not-an-unknown".to_string())),
        };
        assert!(!is_unknown(&val));
    }

    /// A Struct value (even one shaped like a secret wrapper) is never an unknown.
    #[test]
    fn test_is_unknown_false_for_struct() {
        let s = Value {
            kind: Some(Kind::StructValue(Struct::default())),
        };
        assert!(!is_unknown(&s));
    }
}
