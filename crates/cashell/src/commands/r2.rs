//! r2 command — upload assets to Cloudflare R2 (S3-compatible SigV4).
//!
//! REQ-P4-01. Pure-Rust SigV4 (sha2 + hmac), streaming chunked upload,
//! `--dry-run` prints the exact canonical request offline.

use crate::cli::{R2Command, R2Mode};
use crate::output;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const SERVICE: &str = "s3";
const REGION: &str = "auto";
const CHUNK: usize = 64 * 1024;

// ponytail: fixed placeholders so --dry-run works without secrets; real
// uploads always use env creds (checked before these are reached).
const PLACEHOLDER_KEY: &str = "AKIAPLACEHOLDER";
const PLACEHOLDER_SECRET: &str = "placeholder-secret";
const PLACEHOLDER_ACCOUNT: &str = "placeholder-account";

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a file, streamed — flat memory for any size.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf).with_context(|| "read payload")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// AWS URI encoding. `encode_slash=false` keeps path separators.
fn uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn hmac_sha256(key: &[u8], data: &str) -> Result<Vec<u8>> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).map_err(|e| anyhow!("hmac key error: {e}"))?;
    m.update(data.as_bytes());
    Ok(m.finalize().into_bytes().to_vec())
}

/// A fully computed SigV4 signed request.
#[derive(Debug, Clone)]
pub struct SignedRequest {
    pub canonical_request: String,
    pub string_to_sign: String,
    pub authorization: String,
    pub signed_headers: String,
    pub host: String,
    pub amz_date: String,
    pub payload_sha256: String,
}

/// Build the SigV4 canonical request, string-to-sign and Authorization header.
/// Deterministic: every input is a parameter (no clock, no env reads).
pub fn build_signed(
    method: &str,
    host: &str,
    canonical_uri: &str,
    access_key: &str,
    secret_key: &str,
    amz_date: &str,
    payload_sha256: &str,
) -> Result<SignedRequest> {
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical_headers =
        format!("host:{host}\nx-amz-content-sha256:{payload_sha256}\nx-amz-date:{amz_date}\n");
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_sha256}"
    );

    let datestamp = amz_date
        .get(..8)
        .ok_or_else(|| anyhow!("amz_date too short: {amz_date}"))?;
    let scope = format!("{datestamp}/{REGION}/{SERVICE}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );

    let k_date = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), datestamp)?;
    let k_region = hmac_sha256(&k_date, REGION)?;
    let k_service = hmac_sha256(&k_region, SERVICE)?;
    let k_signing = hmac_sha256(&k_service, "aws4_request")?;
    let signature = hex(&hmac_sha256(&k_signing, &string_to_sign)?);

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );

    Ok(SignedRequest {
        canonical_request,
        string_to_sign,
        authorization,
        signed_headers: signed_headers.to_string(),
        host: host.to_string(),
        amz_date: amz_date.to_string(),
        payload_sha256: payload_sha256.to_string(),
    })
}

/// Credentials from environment. Clear error, never a panic.
pub fn creds_from_env() -> Result<(String, String, String)> {
    let get = |k: &str| -> Result<String> {
        std::env::var(k).map_err(|_| anyhow!("missing env var {k} (R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, R2_ACCOUNT_ID required)"))
    };
    Ok((
        get("R2_ACCESS_KEY_ID")?,
        get("R2_SECRET_ACCESS_KEY")?,
        get("R2_ACCOUNT_ID")?,
    ))
}

/// Public edge mirror URL.
pub fn public_url(app: &str, key: &str) -> String {
    format!("https://{app}.fath.my.id/{key}")
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("md") => "text/markdown",
        Some("txt") => "text/plain",
        Some("json") => "application/json",
        Some("html") => "text/html",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Entry point. Returns the process exit code.
pub fn run(cmd: &R2Command) -> Result<i32> {
    match &cmd.mode {
        R2Mode::Upload {
            local_path,
            bucket,
            key,
            app,
            dry_run,
        } => upload(local_path, bucket, key.as_deref(), app.as_deref(), *dry_run),
    }
}

fn upload(
    local_path: &str,
    bucket: &str,
    key: Option<&str>,
    app: Option<&str>,
    dry_run: bool,
) -> Result<i32> {
    let path = Path::new(local_path);
    if !path.is_file() {
        anyhow::bail!("local path is not a file: {}", path.display());
    }
    let key = match key {
        Some(k) if !k.is_empty() => k.to_string(),
        _ => path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("cannot derive key from {}", path.display()))?
            .to_string(),
    };

    let digest = sha256_file(path)?;

    let (akid, secret, account, placeholder) = match creds_from_env() {
        Ok((a, s, ac)) => (a, s, ac, false),
        Err(e) => {
            if !dry_run {
                return Err(e);
            }
            (
                PLACEHOLDER_KEY.to_string(),
                PLACEHOLDER_SECRET.to_string(),
                PLACEHOLDER_ACCOUNT.to_string(),
                true,
            )
        }
    };

    let host = format!("{account}.r2.cloudflarestorage.com");
    let canonical_uri = format!("/{}/{}", uri_encode(bucket, false), uri_encode(&key, false));
    let endpoint = format!("https://{host}{canonical_uri}");
    let amz_date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let signed = build_signed(
        "PUT",
        &host,
        &canonical_uri,
        &akid,
        &secret,
        &amz_date,
        &digest,
    )?;

    if placeholder {
        output::info("R2 credentials absent — placeholder values used (dry-run only)");
    }

    let app_slug = app.unwrap_or(bucket).to_string();
    let public = public_url(&app_slug, &key);

    if dry_run {
        output::kv("sha256", &digest);
        output::kv("bucket", bucket);
        output::kv("key", &key);
        output::kv("dry_run", "true");
        println!("canonical_request:");
        println!("{}", signed.canonical_request);
        println!("string_to_sign:");
        println!("{}", signed.string_to_sign);
        println!("signed_headers: {}", signed.signed_headers);
        println!("authorization: {}", signed.authorization);
        println!("x-amz-content-sha256: {digest}");
        println!("x-amz-date: {amz_date}");
        println!("target_url: {endpoint}");
        println!("public_url: {public}");
        return Ok(0);
    }

    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let client = reqwest::blocking::Client::builder()
        .build()
        .context("build http client")?;
    let resp = client
        .put(&endpoint)
        .header("Authorization", &signed.authorization)
        .header("x-amz-content-sha256", &digest)
        .header("x-amz-date", &amz_date)
        .header("content-type", content_type(path))
        .body(reqwest::blocking::Body::new(file))
        .send()
        .with_context(|| format!("PUT {endpoint}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        anyhow::bail!("upload failed: {status} {body}");
    }

    output::kv("sha256", &digest);
    output::kv("bucket", bucket);
    output::kv("key", &key);
    output::kv("url", &public);
    output::success("uploaded");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        // NIST FIPS 180-4 vector for "abc"
        let mut h = Sha256::new();
        h.update(b"abc");
        assert_eq!(
            hex(&h.finalize()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sigv4_canonical_request_fixed_inputs() {
        let signed = build_signed(
            "PUT",
            "test.r2.cloudflarestorage.com",
            "/test/foo/bar.txt",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20130524T000000Z",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .expect("sign");

        let expected = "\
PUT
/test/foo/bar.txt

host:test.r2.cloudflarestorage.com
x-amz-content-sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
x-amz-date:20130524T000000Z

host;x-amz-content-sha256;x-amz-date
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(signed.canonical_request, expected);
        assert_eq!(
            signed.signed_headers,
            "host;x-amz-content-sha256;x-amz-date"
        );
        assert!(signed
            .authorization
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20130524/auto/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature="));
        // Deterministic: same inputs -> same signature.
        let again = build_signed(
            "PUT",
            "test.r2.cloudflarestorage.com",
            "/test/foo/bar.txt",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20130524T000000Z",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .expect("sign");
        assert_eq!(signed.authorization, again.authorization);
    }

    #[test]
    fn sha256_file_streams() {
        let dir = std::env::temp_dir().join("cashell_p4_sha");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let f = dir.join("abc.txt");
        std::fs::write(&f, b"abc").expect("write");
        assert_eq!(
            sha256_file(&f).expect("hash"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn public_url_shape() {
        assert_eq!(
            public_url("caterm", "assets/logo.png"),
            "https://caterm.fath.my.id/assets/logo.png"
        );
    }

    #[test]
    fn uri_encode_keeps_slashes() {
        assert_eq!(uri_encode("a/b c/δ.md", false), "a/b%20c/%CE%B4.md");
    }
}
