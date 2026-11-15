//! SSL Certificate Expiration Probing and Parsing (U-03).
//!
//! Inspects TLS certificates during HTTPS connection handshakes, extracts
//! the `notAfter` validity timestamp, and calculates days remaining until expiration.

use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeZone, Utc};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Parse `notAfter` expiration timestamp from DER-encoded X.509 certificate bytes.
pub fn parse_cert_expiry_from_der(der: &[u8]) -> Result<DateTime<Utc>> {
    let mut pos = 0;
    while pos + 15 < der.len() {
        let tag = der[pos];
        if tag == 0x17 || tag == 0x18 {
            let len = der[pos + 1] as usize;
            if (11..=30).contains(&len) && pos + 2 + len <= der.len() {
                if let Ok(s) = std::str::from_utf8(&der[pos + 2..pos + 2 + len]) {
                    if s.ends_with('Z') {
                        // Found notBefore, now look immediately for notAfter
                        let next_pos = pos + 2 + len;
                        if next_pos + 2 < der.len() {
                            let next_tag = der[next_pos];
                            if next_tag == 0x17 || next_tag == 0x18 {
                                let next_len = der[next_pos + 1] as usize;
                                if (11..=30).contains(&next_len)
                                    && next_pos + 2 + next_len <= der.len()
                                {
                                    if let Ok(after_s) = std::str::from_utf8(
                                        &der[next_pos + 2..next_pos + 2 + next_len],
                                    ) {
                                        if after_s.ends_with('Z') {
                                            return parse_asn1_time_str(after_s, next_tag == 0x18);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        pos += 1;
    }

    Err(anyhow!(
        "Failed to locate Validity sequence in certificate DER"
    ))
}

/// Parse UTCTime (`YYMMDDHHMMSSZ`) or GeneralizedTime (`YYYYMMDDHHMMSSZ`) string to [`DateTime<Utc>`].
pub fn parse_asn1_time_str(s: &str, generalized: bool) -> Result<DateTime<Utc>> {
    let clean = s.trim_end_matches('Z');
    if generalized {
        // YYYYMMDDHHMMSS
        if clean.len() < 14 {
            return Err(anyhow!("Invalid GeneralizedTime length"));
        }
        let year: i32 = clean[0..4].parse()?;
        let month: u32 = clean[4..6].parse()?;
        let day: u32 = clean[6..8].parse()?;
        let hour: u32 = clean[8..10].parse()?;
        let min: u32 = clean[10..12].parse()?;
        let sec: u32 = clean[12..14].parse()?;
        Utc.with_ymd_and_hms(year, month, day, hour, min, sec)
            .single()
            .ok_or_else(|| anyhow!("Invalid timestamp fields"))
    } else {
        // YYMMDDHHMMSS
        if clean.len() < 12 {
            return Err(anyhow!("Invalid UTCTime length"));
        }
        let yy: i32 = clean[0..2].parse()?;
        let year = if yy < 50 { 2000 + yy } else { 1900 + yy };
        let month: u32 = clean[2..4].parse()?;
        let day: u32 = clean[4..6].parse()?;
        let hour: u32 = clean[6..8].parse()?;
        let min: u32 = clean[8..10].parse()?;
        let sec: u32 = clean[10..12].parse()?;
        Utc.with_ymd_and_hms(year, month, day, hour, min, sec)
            .single()
            .ok_or_else(|| anyhow!("Invalid timestamp fields"))
    }
}

/// Calculate days remaining until certificate expiry. Negative indicates expired.
pub fn days_until_expiry(expiry: &DateTime<Utc>, now: &DateTime<Utc>) -> i64 {
    let duration = *expiry - *now;
    duration.num_days()
}

/// Check if days remaining should trigger an SSL alert tier:
/// Returns `Some("3d")` if `<= 3` days, `Some("14d")` if `<= 14` days, or `None`.
pub fn evaluate_ssl_alert_tier(days_left: i64) -> Option<&'static str> {
    if days_left <= 3 {
        Some("3d")
    } else if days_left <= 14 {
        Some("14d")
    } else {
        None
    }
}

/// Probe the SSL certificate of a remote HTTPS host and port (default 443).
pub async fn probe_ssl_expiry(
    host: &str,
    port: u16,
    timeout_duration: Duration,
) -> Result<DateTime<Utc>> {
    use tokio::net::TcpStream;
    use tokio_rustls::rustls::client::danger::{
        HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
    };
    use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, Error, SignatureScheme};
    use tokio_rustls::TlsConnector;

    #[derive(Debug)]
    struct CapturingVerifier {
        captured_der: Arc<Mutex<Option<Vec<u8>>>>,
    }

    impl ServerCertVerifier for CapturingVerifier {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> std::result::Result<ServerCertVerified, Error> {
            let mut lock = self.captured_der.lock().unwrap();
            *lock = Some(end_entity.as_ref().to_vec());
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::ED25519,
            ]
        }
    }

    let captured = Arc::new(Mutex::new(None));
    let verifier = Arc::new(CapturingVerifier {
        captured_der: captured.clone(),
    });

    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(config));
    let addr = format!("{host}:{port}");

    let stream = tokio::time::timeout(timeout_duration, TcpStream::connect(&addr))
        .await
        .map_err(|_| anyhow!("Connection timeout to {addr}"))??;

    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| anyhow!("Invalid DNS name '{host}': {e}"))?;

    let _ = tokio::time::timeout(timeout_duration, connector.connect(server_name, stream)).await;

    let der_opt = captured.lock().unwrap().take();
    let der = der_opt
        .ok_or_else(|| anyhow!("No certificate captured during TLS handshake with {addr}"))?;

    parse_cert_expiry_from_der(&der)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_asn1_time_str_utctime() {
        // "270124182325Z" -> Jan 24, 2027 18:23:25 UTC
        let parsed = parse_asn1_time_str("270124182325Z", false).expect("valid UTCTime");
        assert_eq!(parsed.to_rfc3339(), "2027-01-24T18:23:25+00:00");
    }

    #[test]
    fn test_parse_asn1_time_str_generalized() {
        // "20281231235959Z" -> Dec 31, 2028 23:59:59 UTC
        let parsed = parse_asn1_time_str("20281231235959Z", true).expect("valid GeneralizedTime");
        assert_eq!(parsed.to_rfc3339(), "2028-12-31T23:59:59+00:00");
    }

    #[test]
    fn test_ssl_alert_tiers() {
        assert_eq!(evaluate_ssl_alert_tier(20), None);
        assert_eq!(evaluate_ssl_alert_tier(14), Some("14d"));
        assert_eq!(evaluate_ssl_alert_tier(10), Some("14d"));
        assert_eq!(evaluate_ssl_alert_tier(3), Some("3d"));
        assert_eq!(evaluate_ssl_alert_tier(1), Some("3d"));
        assert_eq!(evaluate_ssl_alert_tier(-1), Some("3d"));
    }
}
