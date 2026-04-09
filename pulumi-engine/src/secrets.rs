//! Secret encryption for checkpoint state.
//!
//! Defines the [`SecretsManager`] trait for pluggable encryption backends, and
//! [`PassphraseSecretsManager`] which derives an AES-256-GCM key from a user
//! passphrase via PBKDF2-HMAC-SHA256. The ciphertext format is compatible with
//! the Go Pulumi SDK's passphrase provider.
//!
//! ## Ciphertext format
//!
//! ```text
//! "v1:" + base64(nonce[12] || ciphertext || tag[16])
//! ```
//!
//! ## Salt format
//!
//! ```text
//! "v1:" + hex(salt[8]) + ":" + base64(passphrase_validation_ciphertext)
//! ```

use std::fmt;
use std::future::Future;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha256;

use pulumi_core::serde::SECRET_SIG;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from secret encryption/decryption operations.
#[derive(Debug)]
pub enum SecretsError {
    /// The ciphertext is malformed (missing prefix, bad base64, too short).
    InvalidCiphertext(String),
    /// AES-GCM decryption failed (wrong key or corrupted data).
    DecryptionFailed,
    /// AES-GCM encryption failed.
    EncryptionFailed,
    /// The salt string is malformed.
    InvalidSalt(String),
    /// No passphrase was provided.
    MissingPassphrase,
}

impl fmt::Display for SecretsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretsError::InvalidCiphertext(msg) => write!(f, "invalid ciphertext: {msg}"),
            SecretsError::DecryptionFailed => write!(f, "decryption failed (wrong passphrase?)"),
            SecretsError::EncryptionFailed => write!(f, "encryption failed"),
            SecretsError::InvalidSalt(msg) => write!(f, "invalid salt: {msg}"),
            SecretsError::MissingPassphrase => write!(f, "missing passphrase"),
        }
    }
}

impl std::error::Error for SecretsError {}

// ---------------------------------------------------------------------------
// SecretsManager trait — uses RPITIT like Provider and MonitorConnection
// ---------------------------------------------------------------------------

/// A pluggable secrets encryption backend.
///
/// Methods return `impl Future + Send` (RPITIT) for zero-cost async dispatch,
/// following the same pattern as [`Provider`](crate::provider::Provider) and
/// the connection traits in `pulumi-core`.
pub trait SecretsManager: Clone + Send + Sync + 'static {
    /// Encrypt plaintext bytes, returning a ciphertext string (e.g. `"v1:BASE64..."`).
    fn encrypt(&self, plaintext: &[u8]) -> impl Future<Output = Result<String, SecretsError>> + Send;

    /// Decrypt a ciphertext string back to plaintext bytes.
    fn decrypt(&self, ciphertext: &str) -> impl Future<Output = Result<Vec<u8>, SecretsError>> + Send;

    /// Serializable state for persisting in the checkpoint (e.g. the salt).
    fn state(&self) -> impl Future<Output = SecretsProviderState> + Send;
}


// ---------------------------------------------------------------------------
// SecretsProviderState — serialized into checkpoints
// ---------------------------------------------------------------------------

/// Persisted metadata about the secrets provider, stored in the checkpoint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecretsProviderState {
    /// Provider type identifier (e.g. `"passphrase"`).
    #[serde(rename = "type")]
    pub provider_type: String,
    /// Provider-specific state (e.g. `{"salt": "v1:HEX:BASE64"}`).
    pub state: serde_json::Value,
}

// ---------------------------------------------------------------------------
// PassphraseSecretsManager
// ---------------------------------------------------------------------------

/// PBKDF2 iteration count — matches the Go Pulumi SDK.
const PBKDF2_ITERATIONS: u32 = 1_000_000;
/// AES-256 key length in bytes.
const KEY_LEN: usize = 32;
/// AES-GCM nonce length in bytes.
const NONCE_LEN: usize = 12;
/// PBKDF2 salt length in bytes.
const SALT_LEN: usize = 8;

/// Secrets manager that derives an AES-256-GCM key from a passphrase using
/// PBKDF2-HMAC-SHA256, compatible with the Go Pulumi SDK's passphrase provider.
#[derive(Clone)]
pub struct PassphraseSecretsManager {
    /// The derived 256-bit encryption key.
    key: [u8; KEY_LEN],
    /// The salt used for key derivation, stored for checkpoint persistence.
    salt: [u8; SALT_LEN],
    /// The full salt string including validation ciphertext.
    salt_string: String,
}

impl PassphraseSecretsManager {
    /// Create a new manager with a fresh random salt.
    pub fn new(passphrase: &str) -> Result<Self, SecretsError> {
        let mut salt = [0u8; SALT_LEN];
        rand::thread_rng().fill_bytes(&mut salt);

        let key = derive_key(passphrase, &salt);

        // Encrypt the passphrase itself as a validation token — on load we
        // decrypt this to verify the passphrase is correct before proceeding.
        let validation = encrypt_bytes(&key, passphrase.as_bytes())?;
        let salt_string = format!("v1:{}:{}", hex_encode(&salt), validation);

        Ok(Self {
            key,
            salt,
            salt_string,
        })
    }

    /// Restore a manager from a previously persisted salt string.
    ///
    /// The salt string has the format `"v1:HEX_SALT:BASE64_VALIDATION"`.
    /// The validation ciphertext is decrypted to verify the passphrase.
    pub fn from_salt(passphrase: &str, salt_string: &str) -> Result<Self, SecretsError> {
        let parts: Vec<&str> = salt_string.splitn(3, ':').collect();
        if parts.len() != 3 || parts[0] != "v1" {
            return Err(SecretsError::InvalidSalt(format!(
                "expected 'v1:HEX:BASE64', got '{salt_string}'"
            )));
        }

        let salt = hex_decode(parts[1]).map_err(|e| {
            SecretsError::InvalidSalt(format!("bad hex in salt: {e}"))
        })?;
        if salt.len() != SALT_LEN {
            return Err(SecretsError::InvalidSalt(format!(
                "salt is {} bytes, expected {SALT_LEN}",
                salt.len()
            )));
        }

        let mut salt_arr = [0u8; SALT_LEN];
        salt_arr.copy_from_slice(&salt);

        let key = derive_key(passphrase, &salt_arr);

        // Validate the passphrase by decrypting the validation token.
        let validation_ciphertext = parts[2];
        decrypt_bytes(&key, validation_ciphertext)?;

        Ok(Self {
            key,
            salt: salt_arr,
            salt_string: salt_string.to_string(),
        })
    }
}

impl SecretsManager for PassphraseSecretsManager {
    fn encrypt(&self, plaintext: &[u8]) -> impl Future<Output = Result<String, SecretsError>> + Send {
        let key = self.key;
        let plaintext = plaintext.to_vec();
        async move { encrypt_bytes(&key, &plaintext) }
    }

    fn decrypt(&self, ciphertext: &str) -> impl Future<Output = Result<Vec<u8>, SecretsError>> + Send {
        let key = self.key;
        let ciphertext = ciphertext.to_string();
        async move { decrypt_bytes(&key, &ciphertext) }
    }

    fn state(&self) -> impl Future<Output = SecretsProviderState> + Send {
        let salt_string = self.salt_string.clone();
        async move {
            SecretsProviderState {
                provider_type: "passphrase".to_string(),
                state: serde_json::json!({ "salt": salt_string }),
            }
        }
    }
}

impl fmt::Debug for PassphraseSecretsManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PassphraseSecretsManager")
            .field("salt", &hex_encode(&self.salt))
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Crypto helpers
// ---------------------------------------------------------------------------

/// Derive a 256-bit key from a passphrase and salt via PBKDF2-HMAC-SHA256.
fn derive_key(passphrase: &str, salt: &[u8]) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), salt, PBKDF2_ITERATIONS, &mut key);
    key
}

/// Encrypt bytes with AES-256-GCM, returning `"v1:BASE64(nonce || ciphertext || tag)"`.
fn encrypt_bytes(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<String, SecretsError> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("key is 32 bytes");

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| SecretsError::EncryptionFailed)?;

    // Prepend the nonce to the ciphertext for storage.
    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);

    Ok(format!("v1:{}", BASE64.encode(&blob)))
}

/// Decrypt a `"v1:BASE64(...)"` ciphertext string.
fn decrypt_bytes(key: &[u8; KEY_LEN], ciphertext: &str) -> Result<Vec<u8>, SecretsError> {
    let encoded = ciphertext.strip_prefix("v1:").ok_or_else(|| {
        SecretsError::InvalidCiphertext("missing 'v1:' prefix".to_string())
    })?;

    let blob = BASE64.decode(encoded).map_err(|e| {
        SecretsError::InvalidCiphertext(format!("bad base64: {e}"))
    })?;

    if blob.len() < NONCE_LEN + 16 {
        return Err(SecretsError::InvalidCiphertext(format!(
            "ciphertext too short ({} bytes, need at least {})",
            blob.len(),
            NONCE_LEN + 16
        )));
    }

    let (nonce_bytes, ciphertext_bytes) = blob.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher = Aes256Gcm::new_from_slice(key).expect("key is 32 bytes");
    cipher
        .decrypt(nonce, ciphertext_bytes)
        .map_err(|_| SecretsError::DecryptionFailed)
}

// ---------------------------------------------------------------------------
// Hex helpers (avoids pulling in a hex crate for 8-byte salts)
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("odd-length hex string".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// JSON tree walk — encrypt/decrypt secret values in checkpoint data
// ---------------------------------------------------------------------------

/// Recursively walk a JSON value, encrypting all secret-wrapped values in place.
///
/// Secret values have the shape:
/// ```json
/// { "4dabf18193072939515e22adb298388d": "1b47061264138c4ac30d75fd1eb44270", "value": <V> }
/// ```
///
/// After encryption, `<V>` is replaced with `"v1:BASE64..."`.
pub async fn encrypt_secrets<S: SecretsManager>(
    value: &mut serde_json::Value,
    manager: &S,
) -> Result<(), SecretsError> {
    match value {
        serde_json::Value::Object(map) => {
            if map.contains_key(SECRET_SIG) {
                // This is a secret wrapper — encrypt the inner value.
                if let Some(inner) = map.get("value") {
                    let plaintext = serde_json::to_vec(inner).expect("JSON serialization");
                    let ciphertext = manager.encrypt(&plaintext).await?;
                    map.insert(
                        "value".to_string(),
                        serde_json::Value::String(ciphertext),
                    );
                }
            } else {
                // Recurse into non-secret objects.
                for val in map.values_mut() {
                    Box::pin(encrypt_secrets(val, manager)).await?;
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                Box::pin(encrypt_secrets(item, manager)).await?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Recursively walk a JSON value, decrypting all secret-wrapped values in place.
///
/// Finds secret wrappers where `"value"` is a `"v1:..."` ciphertext string
/// and replaces it with the decrypted JSON value.
pub async fn decrypt_secrets<S: SecretsManager>(
    value: &mut serde_json::Value,
    manager: &S,
) -> Result<(), SecretsError> {
    match value {
        serde_json::Value::Object(map) => {
            if map.contains_key(SECRET_SIG) {
                if let Some(serde_json::Value::String(ciphertext)) = map.get("value")
                    && ciphertext.starts_with("v1:")
                {
                    let plaintext = manager.decrypt(ciphertext).await?;
                    let decrypted: serde_json::Value =
                        serde_json::from_slice(&plaintext).map_err(|_| {
                            SecretsError::InvalidCiphertext(
                                "decrypted value is not valid JSON".to_string(),
                            )
                        })?;
                    map.insert("value".to_string(), decrypted);
                }
            } else {
                for val in map.values_mut() {
                    Box::pin(decrypt_secrets(val, manager)).await?;
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                Box::pin(decrypt_secrets(item, manager)).await?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Check if a JSON value is a secret wrapper (has the magic signature key).
pub fn is_json_secret(value: &serde_json::Value) -> bool {
    matches!(value, serde_json::Value::Object(map) if map.contains_key(SECRET_SIG))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = derive_key("test-passphrase", b"saltsalt");
        let plaintext = b"hello, secrets!";

        let ciphertext = encrypt_bytes(&key, plaintext).unwrap();
        assert!(ciphertext.starts_with("v1:"));

        let decrypted = decrypt_bytes(&key, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails_decryption() {
        let key1 = derive_key("correct-passphrase", b"saltsalt");
        let key2 = derive_key("wrong-passphrase", b"saltsalt");

        let ciphertext = encrypt_bytes(&key1, b"secret data").unwrap();
        let result = decrypt_bytes(&key2, &ciphertext);
        assert!(matches!(result, Err(SecretsError::DecryptionFailed)));
    }

    #[test]
    fn manager_new_and_restore() {
        let mgr = PassphraseSecretsManager::new("my-passphrase").unwrap();
        let salt_string = &mgr.salt_string;

        // Restoring with the same passphrase should succeed.
        let restored = PassphraseSecretsManager::from_salt("my-passphrase", salt_string);
        assert!(restored.is_ok());

        // Restoring with a wrong passphrase should fail.
        let bad = PassphraseSecretsManager::from_salt("wrong-passphrase", salt_string);
        assert!(matches!(bad, Err(SecretsError::DecryptionFailed)));
    }

    #[test]
    fn invalid_salt_format() {
        let result = PassphraseSecretsManager::from_salt("pass", "bad-format");
        assert!(matches!(result, Err(SecretsError::InvalidSalt(_))));
    }

    #[test]
    fn hex_roundtrip() {
        let bytes = [0xde, 0xad, 0xbe, 0xef, 0x01, 0x23, 0x45, 0x67];
        let encoded = hex_encode(&bytes);
        assert_eq!(encoded, "deadbeef01234567");
        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(decoded, bytes);
    }

    #[tokio::test]
    async fn encrypt_decrypt_json_secrets() {
        let mgr = PassphraseSecretsManager::new("test-pass").unwrap();

        let mut value = serde_json::json!({
            "name": "my-bucket",
            "password": {
                SECRET_SIG: "1b47061264138c4ac30d75fd1eb44270",
                "value": "super-secret-password"
            },
            "nested": {
                "apiKey": {
                    SECRET_SIG: "1b47061264138c4ac30d75fd1eb44270",
                    "value": {"key": "abc123", "region": "us-east-1"}
                }
            },
            "tags": ["a", "b"]
        });

        let original = value.clone();

        // Encrypt in place.
        encrypt_secrets(&mut value, &mgr).await.unwrap();

        // The secret values should now be ciphertext strings.
        let pw = &value["password"]["value"];
        assert!(pw.is_string());
        assert!(pw.as_str().unwrap().starts_with("v1:"));

        let ak = &value["nested"]["apiKey"]["value"];
        assert!(ak.is_string());
        assert!(ak.as_str().unwrap().starts_with("v1:"));

        // Non-secret values should be untouched.
        assert_eq!(value["name"], "my-bucket");
        assert_eq!(value["tags"], serde_json::json!(["a", "b"]));

        // Decrypt back.
        decrypt_secrets(&mut value, &mgr).await.unwrap();
        assert_eq!(value, original);
    }

    #[tokio::test]
    async fn manager_trait_encrypt_decrypt() {
        let mgr = PassphraseSecretsManager::new("test").unwrap();
        let plaintext = b"hello world";

        let ciphertext = mgr.encrypt(plaintext).await.unwrap();
        let decrypted = mgr.decrypt(&ciphertext).await.unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn manager_state_roundtrip() {
        let mgr = PassphraseSecretsManager::new("my-pass").unwrap();
        let state = mgr.state().await;

        assert_eq!(state.provider_type, "passphrase");
        let salt = state.state["salt"].as_str().unwrap();
        assert!(salt.starts_with("v1:"));

        // Should be able to restore from the state.
        let restored = PassphraseSecretsManager::from_salt("my-pass", salt).unwrap();
        let restored_state = restored.state().await;
        assert_eq!(state.state, restored_state.state);
    }
}
