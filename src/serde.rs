//! Conversion between `serde_json::Value` and protobuf `google.protobuf.Struct`/`Value`.
//!
//! Pulumi sends resource properties as protobuf Struct values over gRPC. This module
//! handles the bidirectional conversion with JSON, which is the natural Rust interchange
//! format via `serde`.

use prost_types::value::Kind;
use prost_types::{ListValue, Struct, Value};
use serde_json::Map;

/// Special key used by Pulumi to mark special values (secrets and unknowns) in the wire format.
pub const SPECIAL_SIG_KEY: &str = "4dabf18193072939515e22adb298388d";
/// Signature value indicating a secret.
pub const SECRET_SIG: &str = "1b47061264138c4ac30d75fd1eb44270";
/// Signature value indicating an unknown value.
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

/// Converts a protobuf `Value` to a `serde_json::Value`,
/// unwrapping Pulumi secret wrappers and replacing unknowns with `null`.
pub fn proto_value_to_json(value: &Value) -> serde_json::Value {
    match &value.kind {
        None | Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::NumberValue(n)) => {
            // Preserve integers when the float has no fractional part
            // and can be exactly represented (up to 2^53).
            let max_exact = (1i64 << 53) as f64;
            let min_exact = -(1i64 << 53) as f64;
            if n.fract() == 0.0 && *n >= min_exact && *n <= max_exact {
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
///
/// Automatically unwraps Pulumi secret wrappers (returning the inner value)
/// and replaces unknown values with `null`.
pub fn struct_to_json(s: &Struct) -> serde_json::Value {
    // Check for Pulumi special structs before doing a naive conversion.
    if is_secret(s) {
        return match unwrap_secret(s) {
            Some(inner) => proto_value_to_json(inner),
            None => serde_json::Value::Null,
        };
    }
    if is_unknown(s) {
        return serde_json::Value::Null;
    }

    let map: Map<String, serde_json::Value> = s
        .fields
        .iter()
        .map(|(k, v)| (k.clone(), proto_value_to_json(v)))
        .collect();
    serde_json::Value::Object(map)
}

/// Wraps a value as a Pulumi secret in the wire format.
#[cfg(test)]
pub fn wrap_secret(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        SPECIAL_SIG_KEY: SECRET_SIG,
        "value": value,
    })
}

/// Returns the special signature value if this struct is a Pulumi-tagged value.
fn special_sig(s: &Struct) -> Option<&str> {
    if let Some(sig) = s.fields.get(SPECIAL_SIG_KEY)
        && let Some(Kind::StringValue(v)) = &sig.kind
    {
        return Some(v.as_str());
    }
    None
}

/// Checks if a protobuf Struct value represents a Pulumi secret.
pub fn is_secret(s: &Struct) -> bool {
    special_sig(s) == Some(SECRET_SIG)
}

/// Unwraps a Pulumi secret, returning the inner value.
pub fn unwrap_secret(s: &Struct) -> Option<&Value> {
    if is_secret(s) {
        s.fields.get("value")
    } else {
        None
    }
}

/// Checks if a protobuf Struct value represents an unknown value.
pub fn is_unknown(s: &Struct) -> bool {
    special_sig(s) == Some(UNKNOWN_SIG)
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

    #[test]
    fn test_secret_wrapping() {
        let val = serde_json::json!("my-password");
        let wrapped = wrap_secret(val.clone());

        let s = json_to_struct(&wrapped);
        assert!(is_secret(&s));

        let unwrapped = unwrap_secret(&s).unwrap();
        assert_eq!(proto_value_to_json(unwrapped), val);
    }

    #[test]
    fn test_unknown_is_not_secret() {
        let unknown_json = serde_json::json!({
            SPECIAL_SIG_KEY: UNKNOWN_SIG,
        });
        let s = json_to_struct(&unknown_json);
        assert!(is_unknown(&s));
        assert!(!is_secret(&s));
        assert!(unwrap_secret(&s).is_none());
    }

    #[test]
    fn test_secret_is_not_unknown() {
        let secret_json = wrap_secret(serde_json::json!("password"));
        let s = json_to_struct(&secret_json);
        assert!(is_secret(&s));
        assert!(!is_unknown(&s));
    }

    #[test]
    fn test_struct_to_json_unwraps_secrets() {
        // A property that's wrapped as a secret should be unwrapped during conversion.
        let obj = serde_json::json!({
            "plainField": "hello",
            "secretField": {
                SPECIAL_SIG_KEY: SECRET_SIG,
                "value": "my-secret-value",
            }
        });
        let s = json_to_struct(&obj);
        let result = struct_to_json(&s);

        assert_eq!(result["plainField"], "hello");
        assert_eq!(result["secretField"], "my-secret-value");
    }

    #[test]
    fn test_struct_to_json_replaces_unknowns_with_null() {
        let obj = serde_json::json!({
            "knownField": 42,
            "unknownField": {
                SPECIAL_SIG_KEY: UNKNOWN_SIG,
            }
        });
        let s = json_to_struct(&obj);
        let result = struct_to_json(&s);

        assert_eq!(result["knownField"], 42);
        assert!(result["unknownField"].is_null());
    }

    #[test]
    fn test_integer_precision_boundary() {
        // 2^53 is the max integer exactly representable in f64.
        let max_exact: i64 = 1 << 53;
        let json = serde_json::json!(max_exact);
        let s = json_to_struct(&serde_json::json!({"n": json}));
        let back = struct_to_json(&s);
        assert_eq!(back["n"], max_exact);
    }
}
