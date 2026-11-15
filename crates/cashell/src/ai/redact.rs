//! Secret redaction before sending payloads to AI providers (AI-04).
//!
//! Scrubs API keys, bearer tokens, Telegram bot tokens, private keys, passwords,
//! connection strings, and user-provided extra secrets from outgoing payloads.

/// Redact sensitive secrets from prompt text and logs before sending to external AI providers.
pub fn redact_secrets(text: &str, extra_secrets: &[&str]) -> String {
    let mut result = text.to_string();

    // 1. Redact PEM Private Keys (-----BEGIN ... KEY----- ... -----END ... KEY-----)
    result = redact_pem_keys(&result);

    // 2. Redact Telegram Bot Tokens (\d{8,11}:[A-Za-z0-9_-]{35})
    result = redact_telegram_tokens(&result);

    // 3. Redact Bearer Tokens ("Bearer <token>")
    result = redact_bearer_tokens(&result);

    // 4. Redact well-known API key prefixes (sk-, ghp_, gho_, glpat-, pypi-, cash_, lic_)
    result = redact_prefixed_api_keys(&result);

    // 5. Redact URLs with embedded credentials (e.g. postgres://user:pass@host)
    result = redact_url_passwords(&result);

    // 6. Redact key-value pairs (password=..., api_key: ..., etc.)
    result = redact_kv_secrets(&result);

    // 7. Redact explicit extra secrets
    for secret in extra_secrets {
        let trimmed = secret.trim();
        // Ignore too-short tokens to avoid over-matching common substrings
        if trimmed.len() >= 4 {
            result = result.replace(trimmed, "[REDACTED]");
        }
    }

    result
}

fn redact_pem_keys(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rem = input;

    while let Some(start_idx) = rem.find("-----BEGIN ") {
        out.push_str(&rem[..start_idx]);
        let tail = &rem[start_idx..];
        if let Some(end_marker) = tail.find("-----END ") {
            if let Some(final_dashes) = tail[end_marker..].find("-----") {
                let full_end = end_marker + final_dashes + 5;
                out.push_str("[REDACTED_PRIVATE_KEY]");
                rem = &tail[full_end..];
                continue;
            }
        }
        // If not matched fully, preserve and break
        out.push_str(&rem[start_idx..start_idx + 11]);
        rem = &rem[start_idx + 11..];
    }
    out.push_str(rem);
    out
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'
}

fn redact_telegram_tokens(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Look for digit followed by 8 to 11 digits then ':'
        if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let digit_len = j - i;
            if (8..=11).contains(&digit_len) && j < bytes.len() && bytes[j] == b':' {
                // Check if following 35 chars are valid token chars
                let token_start = j + 1;
                let mut k = token_start;
                while k < bytes.len()
                    && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_' || bytes[k] == b'-')
                {
                    k += 1;
                }
                let token_len = k - token_start;
                if token_len == 35 {
                    out.push_str("[REDACTED_TELEGRAM_TOKEN]");
                    i = k;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn redact_bearer_tokens(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rem = input;

    while let Some(pos) = rem.to_ascii_lowercase().find("bearer ") {
        out.push_str(&rem[..pos]);
        out.push_str("Bearer ");
        let after = &rem[pos + 7..];
        let token_end = after
            .find(|c: char| !is_ident_char(c))
            .unwrap_or(after.len());
        let token = &after[..token_end];
        if token.len() >= 8 {
            out.push_str("[REDACTED_TOKEN]");
            rem = &after[token_end..];
        } else {
            out.push_str(token);
            rem = &after[token_end..];
        }
    }
    out.push_str(rem);
    out
}

fn redact_prefixed_api_keys(input: &str) -> String {
    let prefixes = [
        "sk-", "ghp_", "gho_", "glpat-", "pypi-", "cash_", "lic_", "key-",
    ];
    let mut out = String::with_capacity(input.len());
    let mut rem = input;

    'outer: while !rem.is_empty() {
        for prefix in &prefixes {
            if let Some(pos) = rem.find(prefix) {
                // Check prefix boundary
                if pos == 0 || !rem[..pos].chars().last().unwrap().is_ascii_alphanumeric() {
                    out.push_str(&rem[..pos]);
                    let after = &rem[pos + prefix.len()..];
                    let key_len = after
                        .find(|c: char| !is_ident_char(c))
                        .unwrap_or(after.len());
                    // Combined prefix + suffix >= 16 chars is typical for API keys
                    if key_len + prefix.len() >= 16 {
                        out.push_str("[REDACTED_API_KEY]");
                        rem = &after[key_len..];
                        continue 'outer;
                    }
                }
            }
        }
        // If no prefix matched at current start, advance
        let next_char = rem.chars().next().unwrap();
        out.push(next_char);
        rem = &rem[next_char.len_utf8()..];
    }
    out
}

fn redact_url_passwords(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rem = input;

    while let Some(proto_pos) = rem.find("://") {
        let scheme_start = rem[..proto_pos]
            .rfind(|c: char| !c.is_ascii_alphabetic())
            .map(|i| i + 1)
            .unwrap_or(0);
        out.push_str(&rem[..scheme_start]);
        let url_part = &rem[scheme_start..];
        let after_proto = &url_part[proto_pos - scheme_start + 3..];

        // Find user:password@
        if let Some(at_pos) = after_proto.find('@') {
            let auth_part = &after_proto[..at_pos];
            if let Some(colon_pos) = auth_part.find(':') {
                let user = &auth_part[..colon_pos];
                // Ensure no slashes or spaces in auth
                if !user.contains('/')
                    && !user.contains(' ')
                    && !auth_part[colon_pos + 1..].contains('/')
                {
                    out.push_str(&url_part[..proto_pos - scheme_start + 3]);
                    out.push_str(user);
                    out.push_str(":[REDACTED]@");
                    rem = &after_proto[at_pos + 1..];
                    continue;
                }
            }
        }

        out.push_str(&rem[scheme_start..proto_pos + 3]);
        rem = &rem[proto_pos + 3..];
    }
    out.push_str(rem);
    out
}

fn redact_kv_secrets(input: &str) -> String {
    let keys = [
        "password",
        "api_key",
        "apikey",
        "auth_token",
        "secret",
        "token",
        "bot_token",
    ];
    let mut lines = Vec::new();

    for line in input.lines() {
        let mut modified_line = line.to_string();
        for key in &keys {
            let lower = modified_line.to_ascii_lowercase();
            if let Some(pos) = lower.find(key) {
                // Must be at word boundary
                let before_ok = pos == 0
                    || !modified_line[..pos]
                        .chars()
                        .last()
                        .unwrap()
                        .is_ascii_alphanumeric();
                let after_key = &modified_line[pos + key.len()..];
                if before_ok {
                    let trimmed_after = after_key.trim_start();
                    if trimmed_after.starts_with('=') || trimmed_after.starts_with(':') {
                        let sep_char = trimmed_after.chars().next().unwrap();
                        let val_part = trimmed_after[1..].trim_start();
                        // Redact non-empty value
                        if !val_part.is_empty() && !val_part.starts_with("[REDACTED") {
                            let prefix = &modified_line[..pos + key.len()];
                            // Handle quoted value or single word
                            let quote = val_part.chars().next().unwrap();
                            if quote == '"' || quote == '\'' {
                                if let Some(end_quote) = val_part[1..].find(quote) {
                                    let rest = &val_part[1 + end_quote + 1..];
                                    modified_line =
                                        format!("{prefix}{sep_char} \"[REDACTED]\"{rest}");
                                } else {
                                    modified_line = format!("{prefix}{sep_char} \"[REDACTED]\"");
                                }
                            } else {
                                let end_val = val_part
                                    .find(|c: char| c.is_whitespace() || c == ',' || c == ';')
                                    .unwrap_or(val_part.len());
                                let rest = &val_part[end_val..];
                                modified_line = format!("{prefix}{sep_char} [REDACTED]{rest}");
                            }
                        }
                    }
                }
            }
        }
        lines.push(modified_line);
    }

    if input.ends_with('\n') {
        let mut res = lines.join("\n");
        res.push('\n');
        res
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_pem_private_key() {
        let text = "Log error: key invalid\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0...secret\n-----END RSA PRIVATE KEY-----\nEnd of log";
        let redacted = redact_secrets(text, &[]);
        assert!(!redacted.contains("MIIEowIBAAKCAQEA0"));
        assert!(redacted.contains("[REDACTED_PRIVATE_KEY]"));
    }

    #[test]
    fn test_redact_telegram_token() {
        let text = "telegram bot token: 123456789:ABCdefGHIjklMNOpqrSTUvwxYZ123456789 failed";
        let redacted = redact_secrets(text, &[]);
        assert!(!redacted.contains("123456789:ABCdefGHIjklMNOpqrSTUvwxYZ123456789"));
        assert!(redacted.contains("[REDACTED_TELEGRAM_TOKEN]"));
    }

    #[test]
    fn test_redact_bearer_token() {
        let text = "Authorization: Bearer my_super_secret_jwt_token_1234567890 in header";
        let redacted = redact_secrets(text, &[]);
        assert!(!redacted.contains("my_super_secret_jwt_token_1234567890"));
        assert!(redacted.contains("Bearer [REDACTED_TOKEN]"));
    }

    #[test]
    fn test_redact_prefixed_api_key() {
        let text = "Failed OpenAI connection: sk-proj-1234567890abcdef1234567890 was rejected";
        let redacted = redact_secrets(text, &[]);
        assert!(!redacted.contains("sk-proj-1234567890abcdef1234567890"));
        assert!(redacted.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn test_redact_url_password() {
        let text = "Connecting to postgres://admin:SuperSecretPass123!@db.internal:5432/cashell";
        let redacted = redact_secrets(text, &[]);
        assert!(!redacted.contains("SuperSecretPass123!"));
        assert!(redacted.contains("postgres://admin:[REDACTED]@db.internal:5432/cashell"));
    }

    #[test]
    fn test_redact_kv_secrets() {
        let text = "db_config: password = \"mypassword123\"\napi_key: secret-val-xyz";
        let redacted = redact_secrets(text, &[]);
        assert!(!redacted.contains("mypassword123"));
        assert!(!redacted.contains("secret-val-xyz"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn test_redact_extra_secrets() {
        let text = "License key CASH99-FOUNDR-DEV01-SECRET-ABCD active";
        let redacted = redact_secrets(text, &["CASH99-FOUNDR-DEV01-SECRET-ABCD"]);
        assert!(!redacted.contains("CASH99-FOUNDR-DEV01-SECRET-ABCD"));
        assert_eq!(
            text.replace("CASH99-FOUNDR-DEV01-SECRET-ABCD", "[REDACTED]"),
            redacted
        );
    }
}
