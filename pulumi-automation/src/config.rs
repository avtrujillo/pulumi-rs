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
        Self { value: value.into(), secret: false }
    }

    /// Creates a secret config value.
    pub fn secret(value: impl Into<String>) -> Self {
        Self { value: value.into(), secret: true }
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
