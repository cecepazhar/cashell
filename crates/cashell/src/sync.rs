//! Pro Sync — zero-knowledge E2EE vault engine & GCC REST client (Phase 2).
//!
//! - [`ZeroKnowledgeCrypto`] — Argon2id KDF + AES-256-GCM client-side encryption.
//!   Keys are derived locally from user passphrase and never transmitted.
//! - [`SyncEngine`] — push/pull client for `GET/PUT /vault` on the GCC REST API.
//! - [`EncryptedVaultPayload`] — serializable encrypted envelope.

use anyhow::{anyhow, bail, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::pro::SessionManager;

// ── Argon2id Default Production Parameters ──────────────────────────────────
pub const DEFAULT_M_COST_KIB: u32 = 65536; // 64 MiB memory cost
pub const DEFAULT_T_COST: u32 = 3;         // 3 iterations
pub const DEFAULT_P_COST: u32 = 4;         // 4-way parallelism

/// Standard local file path for the zero-knowledge plaintext vault.
///
/// The file contains the user's vault document (credentials, notes, etc.) in
/// plain text. It is encrypted client-side before any network transmission,
/// and decrypted locally on pull.
pub const ZERO_KNOWLEDGE_VAULT_PATH: &str = ".cashell/vault.local";

// ── ZeroKnowledgeCrypto ─────────────────────────────────────────────────────

/// Zero-Knowledge End-to-End Encryption Engine.
///
/// Combines Argon2id password-based key derivation with AES-256-GCM authenticated
/// encryption. Master passphrases never leave the client device; the cloud GCC
/// coordinator only ever receives and stores encrypted envelopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZeroKnowledgeCrypto {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for ZeroKnowledgeCrypto {
    fn default() -> Self {
        Self::new()
    }
}

/// A serialized encrypted envelope persisted or transmitted to `/vault`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedVaultPayload {
    pub version: u32,
    pub hwid: String,
    pub salt: String,       // 16 bytes hex-encoded
    pub nonce: String,      // 12 bytes hex-encoded (AES-GCM 96-bit IV)
    pub ciphertext: String, // hex-encoded ciphertext + 16-byte poly1305/GCM tag
    pub updated_unix: i64,
}

impl ZeroKnowledgeCrypto {
    /// Create a new crypto instance with standard production Argon2id parameters.
    pub fn new() -> Self {
        Self {
            m_cost_kib: DEFAULT_M_COST_KIB,
            t_cost: DEFAULT_T_COST,
            p_cost: DEFAULT_P_COST,
        }
    }

    /// Create with custom cost parameters (useful for fast automated tests).
    pub fn with_params(m_cost_kib: u32, t_cost: u32, p_cost: u32) -> Self {
        Self {
            m_cost_kib,
            t_cost,
            p_cost,
        }
    }

    /// Derive a 256-bit (32-byte) AES key from `passphrase` and raw `salt` bytes.
    pub fn derive_key(&self, passphrase: &str, salt: &[u8]) -> Result<[u8; 32]> {
        if salt.is_empty() {
            bail!("salt cannot be empty");
        }
        let params = Params::new(self.m_cost_kib, self.t_cost, self.p_cost, None)
            .map_err(|e| anyhow!("invalid argon2 parameters: {e}"))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

        let mut key = [0u8; 32];
        argon2
            .hash_password_into(passphrase.as_bytes(), salt, &mut key)
            .map_err(|e| anyhow!("argon2id key derivation error: {e}"))?;
        Ok(key)
    }

    /// Encrypt UTF-8 plaintext string using user passphrase and machine HWID.
    pub fn encrypt(
        &self,
        passphrase: &str,
        hwid: &str,
        plaintext: &str,
    ) -> Result<EncryptedVaultPayload> {
        self.encrypt_bytes(passphrase, hwid, plaintext.as_bytes())
    }

    /// Encrypt raw binary data using user passphrase and machine HWID.
    pub fn encrypt_bytes(
        &self,
        passphrase: &str,
        hwid: &str,
        plaintext: &[u8],
    ) -> Result<EncryptedVaultPayload> {
        let mut rng = rand::thread_rng();
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 12];
        rng.fill(&mut salt);
        rng.fill(&mut nonce);

        let key = self.derive_key(passphrase, &salt)?;
        let ciphertext = aes_gcm_encrypt(&key, &nonce, plaintext)?;

        Ok(EncryptedVaultPayload {
            version: 1,
            hwid: hwid.to_string(),
            salt: hex_encode(&salt),
            nonce: hex_encode(&nonce),
            ciphertext: hex_encode(&ciphertext),
            updated_unix: now_unix(),
        })
    }

    /// Decrypt an [`EncryptedVaultPayload`] back to a UTF-8 string.
    pub fn decrypt(&self, passphrase: &str, payload: &EncryptedVaultPayload) -> Result<String> {
        let bytes = self.decrypt_bytes(passphrase, payload)?;
        String::from_utf8(bytes).map_err(|e| anyhow!("decrypted vault is not valid utf-8: {e}"))
    }

    /// Decrypt an [`EncryptedVaultPayload`] back to raw bytes.
    pub fn decrypt_bytes(
        &self,
        passphrase: &str,
        payload: &EncryptedVaultPayload,
    ) -> Result<Vec<u8>> {
        let salt = hex_decode(&payload.salt)?;
        let nonce = hex_decode(&payload.nonce)?;
        let ciphertext = hex_decode(&payload.ciphertext)?;

        if nonce.len() != 12 {
            bail!("invalid AES-GCM nonce length: expected 12 bytes, got {}", nonce.len());
        }

        let key = self.derive_key(passphrase, &salt)?;
        aes_gcm_decrypt(&key, &nonce, &ciphertext)
    }
}

// ── AES-256-GCM Low-Level Helpers ───────────────────────────────────────────

fn aes_gcm_encrypt(key: &[u8; 32], nonce: &[u8], pt: &[u8]) -> Result<Vec<u8>> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| anyhow!("invalid AES-256 key slice length"))?;
    let gcm_nonce = Nonce::from_slice(nonce);
    cipher
        .encrypt(gcm_nonce, pt)
        .map_err(|_| anyhow!("AES-256-GCM encryption failed"))
}

fn aes_gcm_decrypt(key: &[u8; 32], nonce: &[u8], ct: &[u8]) -> Result<Vec<u8>> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| anyhow!("invalid AES-256 key slice length"))?;
    let gcm_nonce = Nonce::from_slice(nonce);
    cipher
        .decrypt(gcm_nonce, ct)
        .map_err(|_| anyhow!("AES-256-GCM authentication/decryption failed (wrong passphrase or corrupted ciphertext)"))
}

// ── Hex Helpers ─────────────────────────────────────────────────────────────

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    let s = s.trim().trim_start_matches("0x");
    if s.len() % 2 != 0 {
        bail!("odd hex length: {}", s.len());
    }
    s.as_bytes()
        .chunks(2)
        .map(|c| {
            let str_val = std::str::from_utf8(c).map_err(|e| anyhow!("invalid utf8: {e}"))?;
            u8::from_str_radix(str_val, 16).map_err(|e| anyhow!("invalid hex byte: {e}"))
        })
        .collect()
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── SyncEngine ──────────────────────────────────────────────────────────────

/// Result details returned after a push/pull operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub success: bool,
    pub status_code: u16,
    pub bytes_transferred: usize,
    pub timestamp: i64,
}

/// Push/Pull synchronization engine for the GCC REST API.
///
/// Interacts directly with the `/vault` endpoint:
/// - `PUT /vault` — push encrypted vault envelope
/// - `GET /vault` — pull encrypted vault envelope
#[derive(Debug, Clone)]
pub struct SyncEngine {
    pub session: SessionManager,
    pub crypto: ZeroKnowledgeCrypto,
    passphrase: String,
    custom_endpoint: Option<String>,
}

impl SyncEngine {
    /// Create a sync engine with default production crypto parameters.
    pub fn new(session: SessionManager, passphrase: String) -> Self {
        Self {
            session,
            crypto: ZeroKnowledgeCrypto::new(),
            passphrase,
            custom_endpoint: None,
        }
    }

    /// Create a sync engine with custom crypto configuration.
    pub fn with_crypto(
        session: SessionManager,
        passphrase: String,
        crypto: ZeroKnowledgeCrypto,
    ) -> Self {
        Self {
            session,
            crypto,
            passphrase,
            custom_endpoint: None,
        }
    }

    /// Set an explicit endpoint override (e.g., for local test servers).
    pub fn with_endpoint(mut self, endpoint: &str) -> Self {
        self.custom_endpoint = Some(endpoint.to_string());
        self
    }

    /// Resolved vault URL on the GCC coordinator.
    /// Interacts directly with `/vault` (or custom endpoint).
    pub fn vault_url(&self) -> String {
        if let Some(ref ep) = self.custom_endpoint {
            return ep.clone();
        }
        let base = self.session.api_base.trim_end_matches('/');
        format!("{base}/vault")
    }

    /// Fallback vault URL under `/api/v1/vault` for gateway routing.
    pub fn api_v1_vault_url(&self) -> String {
        let base = self.session.api_base.trim_end_matches('/');
        if base.ends_with("/api/v1") {
            format!("{base}/vault")
        } else {
            format!("{base}/api/v1/vault")
        }
    }

    /// Push vault plaintext to the GCC REST API via `PUT /vault`.
    /// Encrypts data client-side before transmission.
    pub fn push(&self, vault_text: &str) -> Result<SyncResult> {
        let hwid = self.session.hwid();
        let payload = self.crypto.encrypt(&self.passphrase, hwid, vault_text)?;
        self.push_payload(&payload)
    }

    /// Push raw [`EncryptedVaultPayload`] envelope to `PUT /vault`.
    pub fn push_payload(&self, payload: &EncryptedVaultPayload) -> Result<SyncResult> {
        let body_bytes = serde_json::to_vec(payload)?;
        let client = reqwest::blocking::Client::new();
        let primary_url = self.vault_url();

        let req = client
            .put(&primary_url)
            .header(
                "Authorization",
                format!("Bearer {}", self.session.access_token()),
            )
            .header("Content-Type", "application/json")
            .body(body_bytes.clone());

        let resp = match req.send() {
            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND && self.custom_endpoint.is_none() => {
                // Try fallback gateway route
                let fallback_url = self.api_v1_vault_url();
                client
                    .put(&fallback_url)
                    .header(
                        "Authorization",
                        format!("Bearer {}", self.session.access_token()),
                    )
                    .header("Content-Type", "application/json")
                    .body(body_bytes.clone())
                    .send()
                    .map_err(|e| anyhow!("PUT {fallback_url} failed: {e}"))?
            }
            Ok(r) => r,
            Err(e) => return Err(anyhow!("PUT {primary_url} failed: {e}")),
        };

        let status = resp.status();
        if !status.is_success() {
            bail!("vault push rejected by GCC: HTTP {}", status);
        }

        Ok(SyncResult {
            success: true,
            status_code: status.as_u16(),
            bytes_transferred: body_bytes.len(),
            timestamp: now_unix(),
        })
    }

    /// Pull the encrypted vault from GCC via `GET /vault` and decrypt it locally.
    /// Returns `Ok(None)` if no remote vault has been initialized yet (404).
    pub fn pull(&self) -> Result<Option<String>> {
        let payload = match self.pull_payload()? {
            Some(p) => p,
            None => return Ok(None),
        };
        let decrypted = self.crypto.decrypt(&self.passphrase, &payload)?;
        Ok(Some(decrypted))
    }

    /// Pull raw [`EncryptedVaultPayload`] envelope from `GET /vault`.
    pub fn pull_payload(&self) -> Result<Option<EncryptedVaultPayload>> {
        let client = reqwest::blocking::Client::new();
        let primary_url = self.vault_url();

        let req = client
            .get(&primary_url)
            .header(
                "Authorization",
                format!("Bearer {}", self.session.access_token()),
            );

        let resp = match req.send() {
            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND && self.custom_endpoint.is_none() => {
                // Try fallback gateway route
                let fallback_url = self.api_v1_vault_url();
                client
                    .get(&fallback_url)
                    .header(
                        "Authorization",
                        format!("Bearer {}", self.session.access_token()),
                    )
                    .send()
                    .map_err(|e| anyhow!("GET {fallback_url} failed: {e}"))?
            }
            Ok(r) => r,
            Err(e) => return Err(anyhow!("GET {primary_url} failed: {e}")),
        };

        match resp.status() {
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            s if s.is_success() => {
                let payload: EncryptedVaultPayload = resp
                    .json()
                    .map_err(|e| anyhow!("failed to parse vault payload JSON: {e}"))?;
                Ok(Some(payload))
            }
            s => bail!("vault pull failed with HTTP status: {}", s),
        }
    }

    /// Offline encryption helper without network calls.
    pub fn encrypt_vault(&self, vault_text: &str) -> Result<EncryptedVaultPayload> {
        self.crypto
            .encrypt(&self.passphrase, self.session.hwid(), vault_text)
    }

    /// Offline decryption helper without network calls.
    pub fn decrypt_vault(&self, payload: &EncryptedVaultPayload) -> Result<String> {
        self.crypto.decrypt(&self.passphrase, payload)
    }
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_knowledge_roundtrip() {
        let crypto = ZeroKnowledgeCrypto::with_params(65536, 1, 1);
        let secret_text = "# Sovereign Knowledge Base\n\n- Project Alpha: Confidential\n- Key: 42";
        let passphrase = "correct horse battery staple sovereign 2026";
        let hwid = "hwid-mac-8821";

        let payload = crypto
            .encrypt(passphrase, hwid, secret_text)
            .expect("encryption failed");

        assert_eq!(payload.hwid, hwid);
        assert_ne!(payload.ciphertext, secret_text);
        assert_eq!(payload.version, 1);

        let decrypted = crypto
            .decrypt(passphrase, &payload)
            .expect("decryption failed");

        assert_eq!(decrypted, secret_text);
    }

    #[test]
    fn test_zero_knowledge_wrong_passphrase_fails() {
        let crypto = ZeroKnowledgeCrypto::with_params(65536, 1, 1);
        let secret_text = "Highly secret infrastructure topology";
        let payload = crypto
            .encrypt("passphrase-correct", "hwid-1", secret_text)
            .unwrap();

        let result = crypto.decrypt("passphrase-WRONG", &payload);
        assert!(result.is_err(), "decryption with wrong passphrase must fail");
    }

    #[test]
    fn test_envelope_serialization() {
        let crypto = ZeroKnowledgeCrypto::with_params(65536, 1, 1);
        let payload = crypto.encrypt("mypass", "hwid-node-1", "sample content").unwrap();

        let json = serde_json::to_string_pretty(&payload).unwrap();
        let deserialized: EncryptedVaultPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, payload);
    }

    #[test]
    fn test_sync_engine_endpoints() {
        let session = SessionManager::new(
            "fake-jwt-access".to_string(),
            "fake-jwt-refresh".to_string(),
            "hwid-abc".to_string(),
            "https://gcc.fathforce.com".to_string(),
        );
        let engine = SyncEngine::new(session, "master-pass".to_string());
        assert_eq!(engine.vault_url(), "https://gcc.fathforce.com/vault");
        assert_eq!(engine.api_v1_vault_url(), "https://gcc.fathforce.com/api/v1/vault");
    }
}
