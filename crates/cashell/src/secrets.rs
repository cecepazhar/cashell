//! Secrets storage — encrypted at rest via AES-256-GCM + Argon2id (F-02).
//!
//! Stores sensitive credentials (daemon token, Telegram bot token, AI API keys,
//! webhook secrets) in an encrypted file at `~/.config/cashell/secrets.enc`
//! with strict 0600 file permissions.

use anyhow::{anyhow, bail, Result};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::license_local::set_permissions_0600;
use crate::paths;
use crate::sync::ZeroKnowledgeCrypto;

/// Standard schema version for secrets storage.
pub const SECRETS_SCHEMA_VERSION: u32 = 1;

/// In-memory plaintext secret dictionary.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
pub struct SecretStore {
    #[serde(default)]
    pub daemon_token: Option<String>,
    #[serde(default)]
    pub telegram_bot_token: Option<String>,
    #[serde(default)]
    pub ai_api_keys: HashMap<String, String>,
    #[serde(default)]
    pub webhook_secrets: HashMap<String, String>,
    #[serde(default)]
    pub custom: HashMap<String, String>,
}

impl SecretStore {
    /// Return the standard file path: `~/.config/cashell/secrets.enc`.
    pub fn default_path() -> PathBuf {
        paths::secrets_path()
    }

    /// Check if the secrets file exists on disk.
    pub fn exists() -> bool {
        Self::default_path().exists()
    }

    /// Retrieve a generic secret by key from either named fields or custom storage.
    pub fn get_secret(&self, key: &str) -> Option<&str> {
        match key {
            "daemon_token" => self.daemon_token.as_deref(),
            "telegram_bot_token" => self.telegram_bot_token.as_deref(),
            k if k.starts_with("ai:") => self.ai_api_keys.get(&k[3..]).map(|s| s.as_str()),
            k if k.starts_with("webhook:") => self.webhook_secrets.get(&k[8..]).map(|s| s.as_str()),
            _ => self.custom.get(key).map(|s| s.as_str()),
        }
    }

    /// Store a generic secret by key into named fields or custom storage.
    pub fn set_secret(&mut self, key: &str, val: &str) {
        match key {
            "daemon_token" => self.daemon_token = Some(val.to_string()),
            "telegram_bot_token" => self.telegram_bot_token = Some(val.to_string()),
            k if k.starts_with("ai:") => {
                self.ai_api_keys.insert(k[3..].to_string(), val.to_string());
            }
            k if k.starts_with("webhook:") => {
                self.webhook_secrets
                    .insert(k[8..].to_string(), val.to_string());
            }
            _ => {
                self.custom.insert(key.to_string(), val.to_string());
            }
        }
    }

    /// Get an AI provider API key (e.g. "openai", "anthropic", "gemini").
    pub fn get_ai_key(&self, provider: &str) -> Option<&str> {
        self.ai_api_keys.get(provider).map(|s| s.as_str())
    }

    /// Set an AI provider API key.
    pub fn set_ai_key(&mut self, provider: &str, key: &str) {
        self.ai_api_keys
            .insert(provider.to_ascii_lowercase(), key.trim().to_string());
    }

    /// Remove a secret by key.
    pub fn remove_secret(&mut self, key: &str) -> Option<String> {
        match key {
            "daemon_token" => self.daemon_token.take(),
            "telegram_bot_token" => self.telegram_bot_token.take(),
            k if k.starts_with("ai:") => self.ai_api_keys.remove(&k[3..]),
            k if k.starts_with("webhook:") => self.webhook_secrets.remove(&k[8..]),
            _ => self.custom.remove(key),
        }
    }

    /// Save the secrets encrypted to disk at `path` using `passphrase`.
    pub fn save_to_path(&self, path: &Path, passphrase: &str) -> Result<()> {
        let json_plain = serde_json::to_vec(self)?;
        let encrypted_payload = encrypt_bytes_with_argon2(passphrase, &json_plain)?;
        let data = serde_json::to_string_pretty(&encrypted_payload)?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Atomic write: write to temp file in same directory, set 0600, rename
        let tmp = path.with_extension("enc.tmp");
        std::fs::write(&tmp, data.as_bytes())?;
        set_permissions_0600(&tmp)?;
        std::fs::rename(&tmp, path)?;
        set_permissions_0600(path)?;

        Ok(())
    }

    /// Save the secrets to default path using `passphrase`.
    pub fn save(&self, passphrase: &str) -> Result<PathBuf> {
        let p = Self::default_path();
        self.save_to_path(&p, passphrase)?;
        Ok(p)
    }

    /// Load secrets from disk at `path` decrypted using `passphrase`.
    pub fn load_from_path(path: &Path, passphrase: &str) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let txt = std::fs::read_to_string(path)?;
        let env: EncryptedSecretPayload = serde_json::from_str(&txt)
            .map_err(|e| anyhow!("failed to parse encrypted secrets payload: {e}"))?;

        if env.version > SECRETS_SCHEMA_VERSION {
            bail!(
                "unsupported secrets envelope schema version {} (expected <= {})",
                env.version,
                SECRETS_SCHEMA_VERSION
            );
        }

        let plain_bytes = decrypt_bytes_with_argon2(passphrase, &env)?;
        let store: Self = serde_json::from_slice(&plain_bytes)
            .map_err(|e| anyhow!("decrypted secrets parse error: {e}"))?;

        Ok(store)
    }

    /// Load secrets from default path using `passphrase`.
    pub fn load(passphrase: &str) -> Result<Self> {
        Self::load_from_path(&Self::default_path(), passphrase)
    }

    /// Load secrets from default path using auto-resolved passphrase.
    pub fn load_default() -> Result<Self> {
        let pass = resolve_passphrase()?;
        Self::load(&pass)
    }

    /// Save secrets to default path using auto-resolved passphrase.
    pub fn save_default(&self) -> Result<PathBuf> {
        let pass = resolve_passphrase()?;
        self.save(&pass)
    }
}

/// Serialized envelope format for `secrets.enc`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncryptedSecretPayload {
    pub version: u32,
    pub salt: String,       // 16 bytes hex-encoded
    pub nonce: String,      // 12 bytes hex-encoded (AES-GCM 96-bit IV)
    pub ciphertext: String, // hex-encoded ciphertext + 16-byte tag
    pub updated_at: String, // ISO-8601 UTC
}

/// Resolve encryption passphrase with priority:
/// 1. `CASHELL_SECRETS_PASS` environment variable
/// 2. `~/.config/cashell/secrets.key` file (0600)
/// 3. Machine-bound deterministic fallback derived from HWID (headless safe)
pub fn resolve_passphrase() -> Result<String> {
    if let Ok(p) = std::env::var("CASHELL_SECRETS_PASS") {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let key_file = paths::config_dir().join("secrets.key");
    if key_file.exists() {
        if let Ok(content) = std::fs::read_to_string(&key_file) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }

    // Headless deterministic fallback tied to machine HWID and user
    let hwid = crate::pro::HwidGenerator::new()
        .get_or_create()
        .unwrap_or_else(|_| "cashell-default-hwid".to_string());
    let user = std::env::var("USER").unwrap_or_else(|_| "default".to_string());
    Ok(format!("cashell-auto-secret-{hwid}-{user}"))
}

// ── AES-256-GCM + Argon2id Helpers ──────────────────────────────────────────

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        bail!("invalid hex string length");
    }
    s.as_bytes()
        .chunks(2)
        .map(|chunk| {
            let text = std::str::from_utf8(chunk).map_err(|e| anyhow!("utf8 error: {e}"))?;
            u8::from_str_radix(text, 16).map_err(|e| anyhow!("invalid hex digit: {e}"))
        })
        .collect()
}

fn encrypt_bytes_with_argon2(passphrase: &str, plaintext: &[u8]) -> Result<EncryptedSecretPayload> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    let mut rng = rand::thread_rng();
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    rng.fill(&mut salt);
    rng.fill(&mut nonce);

    let crypto = ZeroKnowledgeCrypto::new();
    let key = crypto.derive_key(passphrase, &salt)?;

    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow!("invalid AES-256 key slice length"))?;
    let gcm_nonce = Nonce::from_slice(&nonce);
    let ciphertext = cipher
        .encrypt(gcm_nonce, plaintext)
        .map_err(|e| anyhow!("AES-256-GCM encryption failed: {e}"))?;

    let now_iso = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    Ok(EncryptedSecretPayload {
        version: SECRETS_SCHEMA_VERSION,
        salt: hex_encode(&salt),
        nonce: hex_encode(&nonce),
        ciphertext: hex_encode(&ciphertext),
        updated_at: now_iso,
    })
}

fn decrypt_bytes_with_argon2(
    passphrase: &str,
    payload: &EncryptedSecretPayload,
) -> Result<Vec<u8>> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    let salt = hex_decode(&payload.salt)?;
    let nonce = hex_decode(&payload.nonce)?;
    let ciphertext = hex_decode(&payload.ciphertext)?;

    if nonce.len() != 12 {
        bail!("invalid AES-GCM nonce length: expected 12 bytes");
    }

    let crypto = ZeroKnowledgeCrypto::new();
    let key = crypto.derive_key(passphrase, &salt)?;

    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow!("invalid AES-256 key slice length"))?;
    let gcm_nonce = Nonce::from_slice(&nonce);
    let plaintext = cipher
        .decrypt(gcm_nonce, ciphertext.as_slice())
        .map_err(|_| {
            anyhow!("AES-256-GCM authentication failed — incorrect passphrase or corrupt file")
        })?;

    Ok(plaintext)
}

// ── tests (F-02) ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_store_get_set() {
        let mut store = SecretStore::default();
        store.set_secret("daemon_token", "token-xyz-123");
        store.set_secret("telegram_bot_token", "123456:ABC-DEF");
        store.set_ai_key("openai", "sk-proj-test1234");
        store.set_secret("webhook:deploy", "whsec_test5678");
        store.set_secret("custom_key", "custom_value");

        assert_eq!(store.get_secret("daemon_token"), Some("token-xyz-123"));
        assert_eq!(
            store.get_secret("telegram_bot_token"),
            Some("123456:ABC-DEF")
        );
        assert_eq!(store.get_ai_key("openai"), Some("sk-proj-test1234"));
        assert_eq!(store.get_secret("ai:openai"), Some("sk-proj-test1234"));
        assert_eq!(store.get_secret("webhook:deploy"), Some("whsec_test5678"));
        assert_eq!(store.get_secret("custom_key"), Some("custom_value"));

        store.remove_secret("custom_key");
        assert_eq!(store.get_secret("custom_key"), None);
    }

    #[test]
    fn test_secret_store_encryption_and_permissions() {
        let tmp = std::env::temp_dir().join(format!("cashell-secrets-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let canary_secret = "CANARY_SECRET_SUPER_CONFIDENTIAL_998877";
        let mut store = SecretStore::default();
        store.daemon_token = Some(canary_secret.to_string());
        store.set_ai_key("anthropic", "sk-ant-test-token-4321");

        let file_path = tmp.join("secrets.enc");
        let pass = "my-secure-master-password";

        // Save
        store.save_to_path(&file_path, pass).expect("save secrets");
        assert!(file_path.exists());

        // 1. Verify file permissions: 0600 on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "secrets file permissions must be 0600");
        }

        // 2. Security: Verify canary secret plaintext is NEVER present on disk
        let raw_file_contents = std::fs::read_to_string(&file_path).unwrap();
        assert!(
            !raw_file_contents.contains(canary_secret),
            "plaintext canary secret leaked to disk!"
        );
        assert!(
            !raw_file_contents.contains("sk-ant-test-token-4321"),
            "plaintext API key leaked to disk!"
        );

        // 3. Decrypt with wrong passphrase fails
        let wrong_res = SecretStore::load_from_path(&file_path, "wrong-password");
        assert!(
            wrong_res.is_err(),
            "loading secrets with wrong passphrase must fail"
        );

        // 4. Decrypt with correct passphrase succeeds
        let loaded = SecretStore::load_from_path(&file_path, pass).expect("load secrets");
        assert_eq!(loaded.daemon_token.as_deref(), Some(canary_secret));
        assert_eq!(
            loaded.get_ai_key("anthropic"),
            Some("sk-ant-test-token-4321")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolve_passphrase_env_priority() {
        std::env::set_var("CASHELL_SECRETS_PASS", "explicit-env-pass-42");
        let pass = resolve_passphrase().unwrap();
        assert_eq!(pass, "explicit-env-pass-42");
        std::env::remove_var("CASHELL_SECRETS_PASS");

        // Fallback derives deterministic string
        let fallback_pass = resolve_passphrase().unwrap();
        assert!(fallback_pass.starts_with("cashell-auto-secret-"));
    }
}
