//! Secret encryption for checkpoint state.
//!
//! Defines the [`SecretsManager`] trait for pluggable encryption backends, and
//! [`PassphraseSecretsManager`] which derives an AES-256-GCM key from a user
//! passphrase via PBKDF2-HMAC-SHA256. The wire format matches the Go Pulumi
//! CLI's passphrase provider exactly, so ciphertexts and salt states written
//! by either implementation can be read by the other.
//!
//! ## Ciphertext format (Go: `config.symmetricCrypter.EncryptValue`)
//!
//! ```text
//! "v1:" + base64(nonce[12]) + ":" + base64(ciphertext || tag[16])
//! ```
//!
//! ## Salt state format (Go: `passphrase.newPassphraseSecretsManager`)
//!
//! ```text
//! "v1:" + base64(salt[8]) + ":" + <ciphertext of the literal string "pulumi">
//! ```
//!
//! The trailing segment is a validation token: decrypting it must yield
//! `"pulumi"`, proving the passphrase is correct before any real decryption.

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

/// The plaintext of the salt state's validation token. The Go CLI encrypts
/// this exact string and compares on load; we must match it for interop.
const VALIDATION_PLAINTEXT: &[u8] = b"pulumi";

impl PassphraseSecretsManager {
    /// Create a new manager with a fresh random salt.
    pub fn new(passphrase: &str) -> Result<Self, SecretsError> {
        let mut salt = [0u8; SALT_LEN];
        rand::thread_rng().fill_bytes(&mut salt);

        let key = derive_key(passphrase, &salt);

        // Encrypt the literal "pulumi" as a validation token — on load this
        // is decrypted and compared to verify the passphrase is correct.
        let validation = encrypt_bytes(&key, VALIDATION_PLAINTEXT)?;
        let salt_string = format!("v1:{}:{}", BASE64.encode(salt), validation);

        Ok(Self {
            key,
            salt,
            salt_string,
        })
    }

    /// Restore a manager from a previously persisted salt state string.
    ///
    /// The salt state has the format `"v1:BASE64_SALT:VALIDATION_CIPHERTEXT"`.
    /// The validation ciphertext is decrypted and must yield `"pulumi"`,
    /// otherwise the passphrase is wrong.
    pub fn from_salt(passphrase: &str, salt_string: &str) -> Result<Self, SecretsError> {
        let parts: Vec<&str> = salt_string.splitn(3, ':').collect();
        if parts.len() != 3 || parts[0] != "v1" {
            return Err(SecretsError::InvalidSalt(
                "expected format 'v1:BASE64_SALT:VALIDATION_CIPHERTEXT'".to_string(),
            ));
        }

        let salt = BASE64.decode(parts[1]).map_err(|e| {
            SecretsError::InvalidSalt(format!("bad base64 in salt: {e}"))
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

        // Validate the passphrase by decrypting the validation token and
        // checking its plaintext, exactly as the Go CLI does.
        let validation = decrypt_bytes(&key, parts[2])?;
        if validation != VALIDATION_PLAINTEXT {
            return Err(SecretsError::DecryptionFailed);
        }

        Ok(Self {
            key,
            salt: salt_arr,
            salt_string: salt_string.to_string(),
        })
    }

    /// Synchronously encrypt plaintext bytes into a `"v1:NONCE:CT"` string.
    ///
    /// Same operation as [`SecretsManager::encrypt`] without the async
    /// wrapper, for callers that are not in an async context (e.g. stack
    /// config file persistence).
    pub fn encrypt_sync(&self, plaintext: &[u8]) -> Result<String, SecretsError> {
        encrypt_bytes(&self.key, plaintext)
    }

    /// Synchronously decrypt a `"v1:NONCE:CT"` ciphertext string.
    pub fn decrypt_sync(&self, ciphertext: &str) -> Result<Vec<u8>, SecretsError> {
        decrypt_bytes(&self.key, ciphertext)
    }

    /// The persisted salt state string (`"v1:BASE64_SALT:VALIDATION"`), as
    /// stored in `Pulumi.<stack>.yaml`'s `encryptionsalt` and the checkpoint.
    pub fn salt_state(&self) -> &str {
        &self.salt_string
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
            .field("salt", &BASE64.encode(self.salt))
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

/// Encrypt bytes with AES-256-GCM, returning
/// `"v1:BASE64(nonce):BASE64(ciphertext || tag)"` — the Go CLI's format.
fn encrypt_bytes(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<String, SecretsError> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("key is 32 bytes");

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| SecretsError::EncryptionFailed)?;

    Ok(format!(
        "v1:{}:{}",
        BASE64.encode(nonce_bytes),
        BASE64.encode(&ciphertext)
    ))
}

/// Decrypt a `"v1:BASE64(nonce):BASE64(ciphertext || tag)"` string.
fn decrypt_bytes(key: &[u8; KEY_LEN], ciphertext: &str) -> Result<Vec<u8>, SecretsError> {
    let parts: Vec<&str> = ciphertext.splitn(3, ':').collect();
    if parts.len() != 3 || parts[0] != "v1" {
        return Err(SecretsError::InvalidCiphertext(
            "expected format 'v1:BASE64_NONCE:BASE64_CIPHERTEXT'".to_string(),
        ));
    }

    let nonce_bytes = BASE64.decode(parts[1]).map_err(|e| {
        SecretsError::InvalidCiphertext(format!("bad base64 in nonce: {e}"))
    })?;
    if nonce_bytes.len() != NONCE_LEN {
        return Err(SecretsError::InvalidCiphertext(format!(
            "nonce is {} bytes, expected {NONCE_LEN}",
            nonce_bytes.len()
        )));
    }

    let ciphertext_bytes = BASE64.decode(parts[2]).map_err(|e| {
        SecretsError::InvalidCiphertext(format!("bad base64 in ciphertext: {e}"))
    })?;
    if ciphertext_bytes.len() < 16 {
        return Err(SecretsError::InvalidCiphertext(format!(
            "ciphertext too short ({} bytes, need at least 16 for the GCM tag)",
            ciphertext_bytes.len()
        )));
    }

    let nonce = Nonce::from_slice(&nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(key).expect("key is 32 bytes");
    cipher
        .decrypt(nonce, ciphertext_bytes.as_slice())
        .map_err(|_| SecretsError::DecryptionFailed)
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
                // Skip if already encrypted (value is a "v1:..." string).
                if let Some(inner) = map.get("value") {
                    let already_encrypted = matches!(
                        inner,
                        serde_json::Value::String(s) if s.starts_with("v1:")
                    );
                    if !already_encrypted {
                        let plaintext = serde_json::to_vec(inner).expect("JSON serialization");
                        let ciphertext = manager.encrypt(&plaintext).await?;
                        map.insert(
                            "value".to_string(),
                            serde_json::Value::String(ciphertext),
                        );
                    }
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

    /// The ciphertext must be `v1:BASE64(nonce):BASE64(ct||tag)` — three
    /// colon-separated segments with a 12-byte nonce — to match the Go CLI.
    #[test]
    fn ciphertext_matches_go_wire_format() {
        let key = derive_key("test-passphrase", b"saltsalt");
        let ciphertext = encrypt_bytes(&key, b"value").unwrap();

        let parts: Vec<&str> = ciphertext.split(':').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "v1");
        assert_eq!(BASE64.decode(parts[1]).unwrap().len(), NONCE_LEN);
        // 5 plaintext bytes + 16-byte GCM tag.
        assert_eq!(BASE64.decode(parts[2]).unwrap().len(), 5 + 16);
    }

    /// The salt state must be `v1:BASE64(salt):<ciphertext of "pulumi">` —
    /// five colon-separated segments in total — to match the Go CLI.
    #[test]
    fn salt_state_matches_go_wire_format() {
        let mgr = PassphraseSecretsManager::new("pass").unwrap();
        let salt_state = mgr.salt_state();

        let parts: Vec<&str> = salt_state.split(':').collect();
        assert_eq!(parts.len(), 5, "v1:SALT:v1:NONCE:CT");
        assert_eq!(parts[0], "v1");
        assert_eq!(BASE64.decode(parts[1]).unwrap().len(), SALT_LEN);
        assert_eq!(parts[2], "v1");

        // The validation token decrypts to the literal "pulumi".
        let validation = salt_state.splitn(3, ':').nth(2).unwrap();
        let plaintext = mgr.decrypt_sync(validation).unwrap();
        assert_eq!(plaintext, VALIDATION_PLAINTEXT);
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

    /// `encrypt_sync`/`decrypt_sync` round-trip without an async runtime.
    #[test]
    fn sync_encrypt_decrypt_roundtrip() {
        let mgr = PassphraseSecretsManager::new("pass").unwrap();
        let ciphertext = mgr.encrypt_sync(b"raw string value").unwrap();
        assert_eq!(mgr.decrypt_sync(&ciphertext).unwrap(), b"raw string value");
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

    #[tokio::test]
    async fn double_encrypt_is_idempotent() {
        let mgr = PassphraseSecretsManager::new("pass").unwrap();

        let mut value = serde_json::json!({
            "secret": {
                SECRET_SIG: "1b47061264138c4ac30d75fd1eb44270",
                "value": "plaintext"
            }
        });

        // Encrypt once.
        encrypt_secrets(&mut value, &mgr).await.unwrap();
        let after_first = value.clone();

        // Encrypt again — should be a no-op (value already starts with "v1:").
        encrypt_secrets(&mut value, &mgr).await.unwrap();

        // The ciphertext should be unchanged (not double-encrypted).
        assert_eq!(
            value["secret"]["value"].as_str().unwrap(),
            after_first["secret"]["value"].as_str().unwrap(),
        );

        // Should still decrypt correctly.
        decrypt_secrets(&mut value, &mgr).await.unwrap();
        assert_eq!(value["secret"]["value"], "plaintext");
    }

    #[tokio::test]
    async fn decrypt_skips_unencrypted_secrets() {
        let mgr = PassphraseSecretsManager::new("pass").unwrap();

        // A secret wrapper with a plaintext (non-"v1:") value — e.g. from
        // a checkpoint that was never encrypted.
        let mut value = serde_json::json!({
            "password": {
                SECRET_SIG: "1b47061264138c4ac30d75fd1eb44270",
                "value": "plaintext-password"
            }
        });
        let original = value.clone();

        // Decrypting should leave it unchanged (no "v1:" prefix to decrypt).
        decrypt_secrets(&mut value, &mgr).await.unwrap();
        assert_eq!(value, original);
    }

    #[test]
    fn invalid_salt_error_does_not_leak_salt() {
        let result = PassphraseSecretsManager::from_salt("pass", "bad-format");
        let err = result.unwrap_err();
        let msg = err.to_string();
        // The error message should NOT contain the raw input.
        assert!(!msg.contains("bad-format"));
        assert!(msg.contains("expected format"));
    }

    #[test]
    fn debug_does_not_show_key() {
        let mgr = PassphraseSecretsManager::new("my-secret-passphrase").unwrap();
        let debug = format!("{mgr:?}");

        // Should show the salt hex but NOT the key or passphrase.
        assert!(debug.contains("PassphraseSecretsManager"));
        assert!(debug.contains("salt"));
        assert!(!debug.contains("my-secret-passphrase"));
        // The key is 32 bytes — verify no 64-char hex string appears
        // (the salt is only 16 hex chars).
        assert!(!debug.contains("key"));
    }
}
