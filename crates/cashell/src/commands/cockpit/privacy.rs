//! Privacy mode (Pro): mask sensitive text on every tab for screen sharing.
//!
//! Runs as a pass over the finished frame buffer, so every tab, modal and
//! panel is covered without each renderer knowing about it.
//!
//! - **Standard** (default): IPv4/IPv6 addresses, domains & URL hosts,
//!   e-mail addresses, plus known host and user names (this machine's
//!   hostname, the OS user, SSH users of fleet servers).
//! - **Strict**: Standard + names of servers, containers, apps, monitors,
//!   analytics sites and the pet.
//!
//! Masking keeps the width (one `•` per character) and keeps separators
//! (`.`, `:`, `@`, `/`) and the top-level domain, so layouts do not shift.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

pub const LEVELS: [&str; 2] = ["Standard", "Strict"];
const MASK: char = '•';

/// Top-level domains that mark a token as a domain. Reverse-DNS app ids such
/// as `com.anthropic.Claude` end in a word that is not here, so they stay.
const TLDS: &[&str] = &[
    "com", "net", "org", "id", "io", "dev", "app", "co", "ai", "me", "xyz", "cloud", "tech",
    "info", "biz", "site", "online", "store", "local", "internal", "lan", "home", "test", "sh",
    "us", "uk", "sg", "my", "eu", "de", "fr", "nl", "jp", "au", "ca", "in", "web", "link", "pro",
    "page", "run", "host", "space", "live", "ts",
];

fn is_strict(level: &str) -> bool {
    level.eq_ignore_ascii_case("Strict")
}

/// Mask sensitive text inside `area` of `buf`.
/// `known` = exact strings to hide (already filtered for the level).
pub fn apply(buf: &mut Buffer, area: Rect, known: &[String]) {
    for y in area.y..area.y.saturating_add(area.height) {
        // Row as chars; multi-char/wide cells become a separator.
        let mut xs: Vec<u16> = Vec::with_capacity(area.width as usize);
        let mut chars: Vec<char> = Vec::with_capacity(area.width as usize);
        for x in area.x..area.x.saturating_add(area.width) {
            let Some(cell) = buf.cell((x, y)) else {
                continue;
            };
            let mut it = cell.symbol().chars();
            let ch = match (it.next(), it.next()) {
                (Some(c), None) => c,
                _ => '\u{0}',
            };
            xs.push(x);
            chars.push(ch);
        }
        let mask = mask_row(&chars, known);
        for (i, m) in mask.iter().enumerate() {
            if !*m {
                continue;
            }
            if let (Some(&x), Some(&c)) = (xs.get(i), chars.get(i)) {
                if is_separator(c) {
                    continue;
                }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(MASK);
                }
            }
        }
    }
}

fn is_separator(c: char) -> bool {
    matches!(c, '.' | ':' | '@' | '/' | ' ' | '-' | '_' | '\u{0}')
}

/// Which positions of `row` to mask.
pub fn mask_row(row: &[char], known: &[String]) -> Vec<bool> {
    let mut out = vec![false; row.len()];
    mark_known(row, known, &mut out);
    mark_tokens(row, &mut out);
    out
}

fn mark_known(row: &[char], known: &[String], out: &mut [bool]) {
    for k in known {
        let kc: Vec<char> = k.chars().collect();
        if kc.len() < 3 || kc.len() > row.len() {
            continue;
        }
        let mut i = 0;
        while i + kc.len() <= row.len() {
            let hit = row.get(i..i + kc.len()).is_some_and(|w| w == kc.as_slice());
            let before_ok = i == 0 || row.get(i - 1).map_or(true, |c| !c.is_alphanumeric());
            let after_ok = row.get(i + kc.len()).map_or(true, |c| !c.is_alphanumeric());
            if hit && before_ok && after_ok {
                for m in out.iter_mut().skip(i).take(kc.len()) {
                    *m = true;
                }
                i += kc.len();
            } else {
                i += 1;
            }
        }
    }
}

/// Tokens = runs of chars that can appear in IPs, hosts and e-mails.
fn mark_tokens(row: &[char], out: &mut [bool]) {
    let tok_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | '@' | '_');
    let mut i = 0;
    while i < row.len() {
        if !row.get(i).is_some_and(|&c| tok_char(c)) {
            i += 1;
            continue;
        }
        let start = i;
        while row.get(i).is_some_and(|&c| tok_char(c)) {
            i += 1;
        }
        // Trim trailing punctuation such as "example.com." or "host:".
        let mut end = i;
        while end > start
            && row
                .get(end - 1)
                .is_some_and(|c| matches!(c, '.' | ':' | '-'))
        {
            end -= 1;
        }
        let tok: String = row.get(start..end).unwrap_or_default().iter().collect();
        if let Some(range) = sensitive_range(&tok) {
            for m in out.iter_mut().skip(start + range.0).take(range.1 - range.0) {
                *m = true;
            }
        }
    }
}

/// Part of `tok` to mask (byte == char offsets: tokens are ASCII).
fn sensitive_range(tok: &str) -> Option<(usize, usize)> {
    // host:port → judge the host part only
    let host_end = if is_ipv6(tok) {
        tok.len()
    } else {
        tok.rfind(':')
            .filter(|&p| {
                tok.get(p + 1..)
                    .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()))
            })
            .unwrap_or(tok.len())
    };
    let host = tok.get(..host_end)?;
    if is_ipv4(host) || is_ipv6(host) {
        return Some((0, host_end));
    }
    if let Some(at) = host.find('@') {
        let domain = host.get(at + 1..)?;
        if is_domain(domain) {
            return Some((0, host_end - tld_len(domain) - 1));
        }
    }
    if is_domain(host) {
        // keep ".com" so viewers still see it is a domain
        return Some((0, host_end - tld_len(host) - 1));
    }
    None
}

fn tld_len(domain: &str) -> usize {
    domain.rsplit('.').next().map_or(0, str::len)
}

fn is_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.chars().all(|c| c.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

fn is_ipv6(s: &str) -> bool {
    let colons = s.matches(':').count();
    colons >= 2
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
        // clock times like 16:35:05 are digits only
        && (s.contains("::") || s.chars().any(|c| c.is_ascii_alphabetic()))
        && s.split(':').all(|g| g.len() <= 4)
}

fn is_domain(s: &str) -> bool {
    let labels: Vec<&str> = s.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let Some(tld) = labels.last() else {
        return false;
    };
    let tld = tld.to_ascii_lowercase();
    TLDS.contains(&tld.as_str())
        && labels.iter().all(|l| {
            !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        // first label must contain a letter ("1.2.ai" is not a host)
        && labels.first().is_some_and(|l| l.chars().any(|c| c.is_ascii_alphabetic()))
}

/// Exact strings to hide for `level`, from what the cockpit knows.
pub fn known_strings(app: &super::CockpitApp, level: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    v.push(app.hostname.clone());
    if let Ok(u) = std::env::var("USER") {
        v.push(u);
    }
    for node in app.cfg.daemon.nodes.values() {
        v.push(node.user.clone());
    }
    if is_strict(level) {
        v.extend(app.cfg.daemon.nodes.keys().cloned());
        for c in &app.latest_snap.docker_containers {
            // "node::name" → hide both parts
            for part in c.name.split("::") {
                v.push(part.to_string());
            }
        }
        v.extend(
            app.cfg
                .uptime
                .custom_monitors
                .iter()
                .map(|m| m.name.clone()),
        );
        v.extend(
            app.latest_snap
                .uptime_monitors
                .iter()
                .map(|m| m.name.clone()),
        );
        v.extend(
            app.analytic_sites()
                .into_iter()
                .filter(|s| !s.id.is_empty())
                .map(|s| s.name),
        );
        if let Some(Ok(Some(pet))) = app.db.as_ref().map(|d| d.get_active_pet()) {
            v.push(pet.name);
        }
    }
    v.retain(|s| s.chars().count() >= 3 && s != "root" && s != "local" && s != "Local");
    v.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(s: &str, known: &[&str]) -> String {
        let row: Vec<char> = s.chars().collect();
        let known: Vec<String> = known.iter().map(|k| k.to_string()).collect();
        let m = mask_row(&row, &known);
        row.iter()
            .zip(m)
            .map(|(&c, m)| if m && !is_separator(c) { MASK } else { c })
            .collect()
    }

    #[test]
    fn test_masks_ipv4_with_port_and_keeps_dots() {
        assert_eq!(
            masked("Tailscale 100.76.150.46:8443 ok", &[]),
            "Tailscale •••.••.•••.••:8443 ok"
        );
    }

    #[test]
    fn test_masks_domain_but_keeps_tld_and_path() {
        assert_eq!(
            masked("https://cagames.fathforce.com/x", &[]),
            "https://•••••••.•••••••••.com/x"
        );
        assert_eq!(
            masked("mail hi@cecepazhar.com", &[]),
            "mail ••@••••••••••.com"
        );
    }

    #[test]
    fn test_leaves_times_versions_files_and_app_ids_alone() {
        for s in [
            "16:35:05",
            "v2.1.0-rc2",
            "config.toml",
            "com.anthropic.Claude",
            "45.2%",
        ] {
            assert_eq!(masked(s, &[]), s);
        }
    }

    #[test]
    fn test_apply_masks_a_rendered_buffer_row() {
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        buf.set_string(
            0,
            0,
            "Hostinger VPS 100.76.150.46 ONLINE",
            ratatui::style::Style::default(),
        );
        apply(&mut buf, area, &["Hostinger VPS".to_string()]);
        let row: String = (0..34)
            .filter_map(|x| buf.cell((x, 0)).map(|c| c.symbol().to_string()))
            .collect();
        assert_eq!(row, "••••••••• ••• •••.••.•••.•• ONLINE");
    }

    #[test]
    fn test_masks_ipv6_and_known_names_on_word_boundaries() {
        assert_eq!(masked("fe80::1ab:2", &[]), "••••::•••:•");
        assert_eq!(
            masked("Local (cax1) cax10", &["cax1"]),
            "Local (••••) cax10"
        );
    }
}
