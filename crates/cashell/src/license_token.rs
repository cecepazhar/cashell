//! GCC License v2 — offline verification of the Ed25519 `signed_token` GCC returns from
//! `/api/v1/licenses/activate` and `/verify`.
//!
//! Standard: Notes/4 - Product (Notes)/33 GCC/01-skema-keamanan-lisensi-gcc.md. The client holds
//! only the public key (compiled in), so it can check a licence but never mint one. Every
//! uncertain case fails closed: no valid signature for this product and device → Free.

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

/// This app's SKU in GCC License v2 tokens.
pub const PRODUCT_SKU: &str = "CASHELL";

/// Clock rollback tolerance before the token is no longer trusted offline.
pub const CLOCK_DRIFT_TOLERANCE_SECS: i64 = 300;

/// The token exactly as GCC sends it. `payload` is the signed string: verify those bytes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignedToken {
    pub payload: String,
    pub signature: String,
    pub key_version: u32,
}

/// Claims this client relies on. Unknown fields are ignored (the payload may grow).
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    pub v: u32,
    #[serde(default)]
    pub products: Vec<String>,
    pub hwid: String,
    #[serde(default)]
    pub tier: String,
    pub exp: i64,
    #[serde(default)]
    pub lic_exp: i64,
}

/// Why a token does not grant Pro.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("no signed license token")]
    Missing,
    #[error("unknown license key version")]
    UnknownKey,
    #[error("license signature is invalid")]
    BadSignature,
    #[error("license token payload is invalid")]
    BadPayload,
    #[error("license is for another product")]
    OtherProduct,
    #[error("license is bound to another device")]
    OtherDevice,
    #[error("system clock moved backwards; connect to GCC to re-verify")]
    ClockRollback,
    #[error("offline license period ended; connect to GCC to refresh")]
    Expired,
}

/// Default V1 public key (hex). Override at build time with `CA_LICENSE_PUBKEY_V1`.
/// Must match the public key GCC logs at startup for `GCC_LICENSE_SEED_HEX`.
const DEFAULT_PUBKEY_V1: &str = "ac5b9954c8dc7648131e9682a90999de4d38a15deb00095eee4f6be9521e1dcf";

/// Public key for `version`, compiled in. Old versions stay listed after a rotation.
pub fn public_key(version: u32) -> Option<VerifyingKey> {
    #[cfg(test)]
    if version == TEST_KEY_VERSION {
        return Some(test_support::signing_key().verifying_key());
    }
    let hex_key = match version {
        1 => Some(option_env!("CA_LICENSE_PUBKEY_V1").unwrap_or(DEFAULT_PUBKEY_V1)),
        2 => option_env!("CA_LICENSE_PUBKEY_V2"),
        _ => None,
    }?;
    VerifyingKey::from_bytes(&decode_hex32(hex_key.trim())?).ok()
}

fn decode_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(s.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).ok()?;
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

/// Verifies `token` for this product and device. `last_seen` is the latest wall clock this
/// licence has been checked at (monotonic ratchet against clock rollback).
pub fn verify(
    token: Option<&SignedToken>,
    key_for: impl Fn(u32) -> Option<VerifyingKey>,
    device_id: &str,
    last_seen: i64,
    now: i64,
) -> Result<Claims, TokenError> {
    let token = token.ok_or(TokenError::Missing)?;
    let key = key_for(token.key_version).ok_or(TokenError::UnknownKey)?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(&token.signature)
        .map_err(|_| TokenError::BadSignature)?;
    let signature = Signature::from_slice(&sig_bytes).map_err(|_| TokenError::BadSignature)?;
    key.verify_strict(token.payload.as_bytes(), &signature)
        .map_err(|_| TokenError::BadSignature)?;
    let claims: Claims =
        serde_json::from_str(&token.payload).map_err(|_| TokenError::BadPayload)?;
    if claims.v != 2 {
        return Err(TokenError::BadPayload);
    }
    if !claims.products.iter().any(|p| p == PRODUCT_SKU) {
        return Err(TokenError::OtherProduct);
    }
    if claims.hwid != device_id {
        return Err(TokenError::OtherDevice);
    }
    if now < last_seen - CLOCK_DRIFT_TOLERANCE_SECS {
        return Err(TokenError::ClockRollback);
    }
    if now >= claims.exp {
        return Err(TokenError::Expired);
    }
    Ok(claims)
}

/// Key version 0 is never issued by GCC (versions start at 1); test builds map it to a fixed
/// test key so other modules' tests can build genuinely signed tokens.
#[cfg(test)]
pub const TEST_KEY_VERSION: u32 = 0;

#[cfg(test)]
pub mod test_support {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    pub fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[42u8; 32])
    }

    /// A token signed with the test key, valid for this machine's CASHell device id.
    pub fn valid_token() -> SignedToken {
        token(
            &[PRODUCT_SKU],
            &crate::license_gcc::device_id(),
            chrono::Utc::now().timestamp() + 14 * 86_400,
        )
    }

    pub fn token(products: &[&str], hwid: &str, exp: i64) -> SignedToken {
        let payload = serde_json::json!({ "v": 2, "lic_id": "l", "sku": "CASHELL",
            "products": products, "tier": "lifetime", "features": ["pro"], "hwid": hwid,
            "max_devices": 1, "iat": 0, "exp": exp, "lic_exp": 0, "nonce": "n" })
        .to_string();
        let signature = base64::engine::general_purpose::STANDARD
            .encode(signing_key().sign(payload.as_bytes()).to_bytes());
        SignedToken {
            payload,
            signature,
            key_version: TEST_KEY_VERSION,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn test_key(_: u32) -> Option<VerifyingKey> {
        Some(signing_key().verifying_key())
    }

    fn token(products: &[&str], hwid: &str, exp: i64) -> SignedToken {
        let payload = serde_json::json!({ "v": 2, "lic_id": "l", "sku": "CASHELL",
            "products": products, "tier": "lifetime", "features": ["pro"], "hwid": hwid,
            "max_devices": 1, "iat": 0, "exp": exp, "lic_exp": 0, "nonce": "n" })
        .to_string();
        let signature = base64::engine::general_purpose::STANDARD
            .encode(signing_key().sign(payload.as_bytes()).to_bytes());
        SignedToken {
            payload,
            signature,
            key_version: 1,
        }
    }

    #[test]
    fn genuine_token_is_pro() {
        let t = token(&["CASHELL"], "dev", 1_000);
        assert!(verify(Some(&t), test_key, "dev", 0, 10).is_ok());
    }

    #[test]
    fn all_access_founder_token_is_pro() {
        let t = token(&["CATERM", "CAMARK", "CASHELL"], "dev", 1_000);
        assert!(verify(Some(&t), test_key, "dev", 0, 10).is_ok());
    }

    #[test]
    fn missing_token_is_not_pro() {
        assert_eq!(
            verify(None, test_key, "dev", 0, 10),
            Err(TokenError::Missing)
        );
    }

    #[test]
    fn forged_unsigned_token_is_not_pro() {
        let mut t = token(&["CASHELL"], "dev", i64::MAX);
        t.signature = String::new();
        assert_eq!(
            verify(Some(&t), test_key, "dev", 0, 10),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn one_flipped_byte_is_not_pro() {
        let mut t = token(&["CASHELL"], "dev", 1_000);
        t.payload = t.payload.replace("\"exp\":1000", "\"exp\":9000");
        assert_eq!(
            verify(Some(&t), test_key, "dev", 0, 10),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn other_product_is_not_pro() {
        let t = token(&["CATERM"], "dev", 1_000);
        assert_eq!(
            verify(Some(&t), test_key, "dev", 0, 10),
            Err(TokenError::OtherProduct)
        );
    }

    #[test]
    fn other_device_is_not_pro() {
        let t = token(&["CASHELL"], "dev", 1_000);
        assert_eq!(
            verify(Some(&t), test_key, "laptop-2", 0, 10),
            Err(TokenError::OtherDevice)
        );
    }

    #[test]
    fn expired_token_is_not_pro() {
        let t = token(&["CASHELL"], "dev", 1_000);
        assert_eq!(
            verify(Some(&t), test_key, "dev", 0, 1_000),
            Err(TokenError::Expired)
        );
    }

    #[test]
    fn clock_rollback_requires_reverify() {
        let t = token(&["CASHELL"], "dev", 100_000);
        assert_eq!(
            verify(Some(&t), test_key, "dev", 50_000, 40_000),
            Err(TokenError::ClockRollback)
        );
    }

    #[test]
    fn unknown_key_version_is_not_pro() {
        let mut t = token(&["CASHELL"], "dev", 1_000);
        t.key_version = 9;
        assert_eq!(
            verify(Some(&t), public_key, "dev", 0, 10),
            Err(TokenError::UnknownKey)
        );
    }
}
