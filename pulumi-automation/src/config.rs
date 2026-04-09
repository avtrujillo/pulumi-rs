//! Stack configuration values.

use serde::{Deserialize, Serialize};

/// A configuration value with optional secret designation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigValue {
    /// The configuration value.
    pub value: String,
    /// Whether this value is a secret.
    #[serde(default)]
    pub secret: bool,
}

impl ConfigValue {
    /// Creates a plaintext config value.
    pub fn plaintext(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            secret: false,
        }
    }

    /// Creates a secret config value.
    pub fn secret(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            secret: true,
        }
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

    #[test]
    fn plaintext_sets_value_and_not_secret() {
        let cv = ConfigValue::plaintext("my-value");
        assert_eq!(cv.value, "my-value");
        assert!(!cv.secret);
    }

    #[test]
    fn secret_sets_value_and_secret_flag() {
        let cv = ConfigValue::secret("s3cret");
        assert_eq!(cv.value, "s3cret");
        assert!(cv.secret);
    }

    #[test]
    fn from_string_creates_plaintext() {
        let cv = ConfigValue::from(String::from("hello"));
        assert_eq!(cv.value, "hello");
        assert!(!cv.secret);
    }

    #[test]
    fn from_str_creates_plaintext() {
        let cv = ConfigValue::from("world");
        assert_eq!(cv.value, "world");
        assert!(!cv.secret);
    }

    #[test]
    fn json_serialization_roundtrip() {
        let original = ConfigValue::secret("top-secret");
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: ConfigValue = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.value, "top-secret");
        assert!(deserialized.secret);
    }

    #[test]
    fn deserialize_missing_secret_defaults_to_false() {
        let json = r#"{"value": "plain"}"#;
        let cv: ConfigValue = serde_json::from_str(json).unwrap();
        assert_eq!(cv.value, "plain");
        assert!(!cv.secret);
    }
}
