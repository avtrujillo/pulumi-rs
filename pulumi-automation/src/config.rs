//! Stack configuration values.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Type alias for a map of stack configuration values, keyed by config key
/// (e.g. `"myproject:mykey"`). Matches the `ConfigMap` type in all upstream SDKs.
pub type ConfigMap = HashMap<String, ConfigValue>;

/// A configuration value with optional secret designation.
///
/// Implements [`fmt::Debug`] manually so that secret values are masked as
/// `"[secret]"` rather than exposing the plaintext in logs, panic messages,
/// and test failure output.
#[derive(Clone, Serialize, Deserialize)]
pub struct ConfigValue {
    /// The configuration value.
    pub value: String,
    /// Whether this value is a secret. The JSON key is `"secret"` to match the
    /// Pulumi CLI wire format; the field name uses the `is_` prefix for clarity.
    #[serde(rename = "secret", default)]
    pub is_secret: bool,
}

impl ConfigValue {
    /// Creates a plaintext config value.
    pub fn plaintext(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            is_secret: false,
        }
    }

    /// Creates a secret config value.
    pub fn secret(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            is_secret: true,
        }
    }
}

impl fmt::Debug for ConfigValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value_repr: &dyn fmt::Debug = if self.is_secret {
            &"[secret]"
        } else {
            &self.value
        };
        f.debug_struct("ConfigValue")
            .field("value", value_repr)
            .field("is_secret", &self.is_secret)
            .finish()
    }
}

impl From<String> for ConfigValue {
    fn from(value: String) -> Self {
        Self::plaintext(value)
    }
}

impl From<&str> for ConfigValue {
    fn from(value: &str) -> Self {
        Self::plaintext(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `plaintext()` creates a non-secret value.
    #[test]
    fn plaintext_sets_value_and_not_secret() {
        let cv = ConfigValue::plaintext("my-value");
        assert_eq!(cv.value, "my-value");
        assert!(!cv.is_secret);
    }

    /// `secret()` creates a secret value.
    #[test]
    fn secret_sets_value_and_secret_flag() {
        let cv = ConfigValue::secret("s3cret");
        assert_eq!(cv.value, "s3cret");
        assert!(cv.is_secret);
    }

    /// `From<String>` produces a plaintext value.
    #[test]
    fn from_string_creates_plaintext() {
        let cv = ConfigValue::from(String::from("hello"));
        assert_eq!(cv.value, "hello");
        assert!(!cv.is_secret);
    }

    /// `From<&str>` produces a plaintext value.
    #[test]
    fn from_str_creates_plaintext() {
        let cv = ConfigValue::from("world");
        assert_eq!(cv.value, "world");
        assert!(!cv.is_secret);
    }

    /// JSON serialization and deserialization round-trips correctly.
    #[test]
    fn json_serialization_roundtrip() {
        let original = ConfigValue::secret("top-secret");
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: ConfigValue = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.value, "top-secret");
        assert!(deserialized.is_secret);
    }

    /// A missing `"secret"` key in JSON defaults to `is_secret: false`.
    #[test]
    fn deserialize_missing_secret_defaults_to_false() {
        let json = r#"{"value": "plain"}"#;
        let cv: ConfigValue = serde_json::from_str(json).unwrap();
        assert_eq!(cv.value, "plain");
        assert!(!cv.is_secret);
    }

    /// `Debug` masks the value for secret configs and shows it for plaintext.
    #[test]
    fn debug_masks_secret_value() {
        let secret = ConfigValue::secret("my-password");
        let debug_str = format!("{secret:?}");
        assert!(!debug_str.contains("my-password"), "secret value must not appear in Debug output");
        assert!(debug_str.contains("[secret]"));

        let plain = ConfigValue::plaintext("visible");
        let debug_str = format!("{plain:?}");
        assert!(debug_str.contains("visible"));
    }
}
