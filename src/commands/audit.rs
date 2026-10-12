//! Audit command — `cashell audit` — walks a repo and AI-audits source files.
//!
//! Per-file finding protocol: the model replies with one line per finding:
//!   SEVERITY|LINE|MESSAGE
//! where SEVERITY ∈ {CRITICAL, HIGH, MEDIUM, LOW}.
//! `NONE` on a line means the file is clean.
//! Any line not matching the pattern (prose, markdown, etc.) degrades to an
//! INFO finding so we never panic.
//!
//! ponytail: walkdir crate would simplify the walk; use it if added to Cargo.toml.

use crate::cli::AuditCommand;
use crate::config::Config;
use crate::output;
use anyhow::Result;
use colored::Colorize;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Finding severity ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl Severity {
    fn label(&self) -> &str {
        match self {
            Severity::Critical => "CRITICAL",
            Severity::High => "HIGH",
            Severity::Medium => "MEDIUM",
            Severity::Low => "LOW",
            Severity::Info => "INFO",
        }
    }
}

// ── Per-file findings ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub line: Option<u32>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileFindings {
    pub path: String,
    pub findings: Vec<Finding>,
}

// ── Audit report ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize)]
pub struct AuditReport {
    pub repo_path: String,
    pub files_scanned: usize,
    pub files_skipped: usize,
    pub total_critical: usize,
    pub total_high: usize,
    pub total_medium: usize,
    pub total_low: usize,
    pub total_info: usize,
    pub files: Vec<FileFindings>,
}

impl AuditReport {
    pub fn exit_code(&self) -> i32 {
        if self.total_critical > 0 || self.total_high > 0 {
            2
        } else if self.total_medium > 0 || self.total_low > 0 {
            1
        } else {
            0
        }
    }
}

// ── Finding parser ───────────────────────────────────────────────────────────

/// Parse the model's reply into a vec of findings.
/// Contract: one line per finding `SEVERITY|LINE|MESSAGE` or `NONE`.
/// Anything not matching degrades to a single Info finding.
pub fn parse_findings(raw: &str) -> Vec<Finding> {
    let mut results = Vec::new();
    let mut any_structured = false;

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case("none") {
            any_structured = true;
            continue;
        }
        let mut parts = line.splitn(3, '|');
        let sev_str = parts.next().unwrap_or("").trim();
        let line_str = parts.next().unwrap_or("").trim();
        let msg = parts.next().unwrap_or("").trim();

        let severity = match sev_str.to_ascii_uppercase().as_str() {
            "CRITICAL" => Some(Severity::Critical),
            "HIGH" => Some(Severity::High),
            "MEDIUM" => Some(Severity::Medium),
            "LOW" => Some(Severity::Low),
            _ => None,
        };
        if let Some(sev) = severity {
            any_structured = true;
            let lineno = line_str.parse::<u32>().ok();
            results.push(Finding {
                severity: sev,
                line: lineno,
                message: msg.to_string(),
            });
        }
    }

    if !any_structured {
        // Prose fallback — surface raw text so no info is lost.
        results.push(Finding {
            severity: Severity::Info,
            line: None,
            message: raw.trim().to_string(),
        });
    }
    results
}

// ── File discovery ─────────────────────────────────────────────────────────

const SOURCE_EXTS: &[&str] = &[
    "rs", "go", "ts", "tsx", "js", "jsx", "svelte", "py", "php", "sql", "toml", "yaml", "yml",
    "json", "md", "sh", "templ", "css",
];

const EXTRA_IGNORE: &[&str] = &[
    ".venv", "venv", "dist", "build", "vendor", "__pycache__", ".svelte-kit",
];

const MAX_FILE_BYTES: u64 = 200 * 1024; // 200 KB

pub fn should_ignore_dir(name: &str, ignore_patterns: &[String]) -> bool {
    ignore_patterns.iter().any(|p| p == name) || EXTRA_IGNORE.contains(&name)
}

fn has_source_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SOURCE_EXTS.contains(&e))
        .unwrap_or(false)
}

/// Recursive walk using std::fs (no walkdir dep).
pub fn collect_files(
    dir: &Path,
    ignore_patterns: &[String],
    out: &mut Vec<PathBuf>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let path = entry.path();
        if meta.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if should_ignore_dir(name, ignore_patterns) {
                continue;
            }
            collect_files(&path, ignore_patterns, out);
        } else if meta.is_file() && has_source_ext(&path) {
            out.push(path);
        }
    }
}

/// Files changed vs HEAD via `git diff --name-only HEAD`.
fn git_changed_files(root: &Path) -> Result<Vec<PathBuf>> {
    let out = Command::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .current_dir(root)
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| root.join(l))
        .filter(|p| has_source_ext(p))
        .collect())
}

/// ponytail: minimal glob — supports `*` and `?` wildcards, matching the
/// relative path (e.g. "src/*.rs"). Upgrade: use the `glob` crate.
fn glob_match(pattern: &str, rel_path: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = rel_path.chars().collect();
    fn rec(p: &[char], i: usize, s: &[char], j: usize) -> bool {
        if i == p.len() {
            return j == s.len();
        }
        match p[i] {
            '*' => {
                // Try matching 0..n chars.
                let mut k = j;
                loop {
                    if rec(p, i + 1, s, k) {
                        return true;
                    }
                    if k >= s.len() {
                        break;
                    }
                    k += 1;
                }
                false
            }
            '?' if j < s.len() => rec(p, i + 1, s, j + 1),
            c if j < s.len() && s[j] == c => rec(p, i + 1, s, j + 1),
            _ => false,
        }
    }
    rec(&p, 0, &s, 0)
}

// ── System prompt ─────────────────────────────────────────────────────────────

const AUDIT_SYSTEM_PROMPT: &str = "\
You are a code auditor. For each finding reply with exactly one line per issue \
using this pipe-separated format: SEVERITY|LINE|MESSAGE\n\
SEVERITY must be one of: CRITICAL, HIGH, MEDIUM, LOW\n\
LINE is the line number (integer) or 0 if not applicable.\n\
Reply with NONE (and nothing else) when the file has no issues.\n\
Do not add prose, markdown, headers, or explanations outside this format.";

// ── Rendering ─────────────────────────────────────────────────────────────────

fn sev_colored(s: &Severity) -> String {
    match s {
        Severity::Critical => s.label().red().bold().to_string(),
        Severity::High => s.label().red().to_string(),
        Severity::Medium => s.label().yellow().to_string(),
        Severity::Low => s.label().blue().to_string(),
        Severity::Info => s.label().dimmed().to_string(),
    }
}

fn print_report(report: &AuditReport) {
    println!("\n  {:<8} {:<6} {:<40} MESSAGE", "SEVERITY", "LINE", "FILE");
    println!("  {}", "─".repeat(90));
    for ff in &report.files {
        if ff.findings.is_empty() {
            continue;
        }
        for f in &ff.findings {
            let line_s = f.line.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string());
            let short_path = ff.path.chars().rev().take(38).collect::<String>().chars().rev().collect::<String>();
            println!(
                "  {:<18} {:<6} {:<40} {}",
                sev_colored(&f.severity),
                line_s,
                short_path,
                f.message
            );
        }
    }
    println!(
        "\n  {} scanned, {} skipped | {} {} {} {} {}",
        report.files_scanned,
        report.files_skipped,
        format!("CRITICAL:{}", report.total_critical).red(),
        format!("HIGH:{}", report.total_high).red(),
        format!("MEDIUM:{}", report.total_medium).yellow(),
        format!("LOW:{}", report.total_low).blue(),
        format!("INFO:{}", report.total_info).dimmed(),
    );
    println!();
}

// ── Main runner ───────────────────────────────────────────────────────────────

pub fn run(cmd: &AuditCommand, cfg: &Config) -> Result<i32> {
    let path = cmd
        .path
        .as_deref()
        .map(Path::new)
        .unwrap_or(Path::new("."));
    let abs = fs::canonicalize(path)?;

    // File list
    let mut all_files: Vec<PathBuf> = if cmd.changed {
        git_changed_files(&abs)?
    } else {
        let mut v = Vec::new();
        collect_files(&abs, &cfg.workspace.ignore_patterns, &mut v);
        v
    };
    all_files.sort();

    // Optional --include glob filter (matches relative path).
    if let Some(glob) = &cmd.include {
        let abs_clone = abs.clone();
        all_files.retain(|f| {
            f.strip_prefix(&abs_clone)
                .ok()
                .map(|rel| glob_match(glob, &rel.display().to_string()))
                .unwrap_or(false)
        });
    }

    let max = cmd.max_files as usize;
    let skipped_over_limit = all_files.len().saturating_sub(max);
    all_files.truncate(max);

    let mut report = AuditReport {
        repo_path: abs.display().to_string(),
        ..Default::default()
    };

    let timeout = if cmd.timeout > 0 { cmd.timeout } else { cfg.ai.timeout_seconds };

    // Build a per-file config with the audit timeout (which may differ from default).
    let mut audit_cfg = cfg.clone();
    audit_cfg.ai.timeout_seconds = timeout;

    for file_path in &all_files {
        let rel = file_path.strip_prefix(&abs).unwrap_or(file_path);
        let rel_str = rel.display().to_string();

        eprint!("  auditing {} ...", rel_str);
        let _ = std::io::stderr().flush();

        // Size + UTF-8 check
        let meta = match fs::metadata(file_path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!(" [skip: {}]", e);
                report.files_skipped += 1;
                continue;
            }
        };
        if meta.len() > MAX_FILE_BYTES {
            eprintln!(" [skip: >200KB]");
            report.files_skipped += 1;
            continue;
        }
        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => {
                eprintln!(" [skip: not UTF-8]");
                report.files_skipped += 1;
                continue;
            }
        };

        let prompt = format!("File: {rel_str}\n```\n{content}\n```");
        let raw = match crate::ai::query_buffered(&audit_cfg, Some(AUDIT_SYSTEM_PROMPT), &prompt) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(" [ai error: {}]", e);
                report.files_skipped += 1;
                continue;
            }
        };

        let findings = parse_findings(&raw);
        eprintln!(" {} findings", findings.len());

        for f in &findings {
            match f.severity {
                Severity::Critical => report.total_critical += 1,
                Severity::High => report.total_high += 1,
                Severity::Medium => report.total_medium += 1,
                Severity::Low => report.total_low += 1,
                Severity::Info => report.total_info += 1,
            }
        }
        report.files.push(FileFindings {
            path: rel_str,
            findings,
        });
        report.files_scanned += 1;
    }

    report.files_skipped += skipped_over_limit;

    let is_json = cmd.json || !output::is_tty();
    let rendered = if is_json {
        serde_json::to_string_pretty(&report)?
    } else {
        // Build a String version of print_report for --out support
        let mut buf = String::new();
        buf.push_str(&format!(
            "\n  {:<8} {:<6} {:<40} MESSAGE\n",
            "SEVERITY", "LINE", "FILE"
        ));
        buf.push_str(&format!("  {}\n", "─".repeat(90)));
        for ff in &report.files {
            for f in &ff.findings {
                let line_s = f.line.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string());
                let short_path = ff.path.chars().rev().take(38).collect::<String>().chars().rev().collect::<String>();
                buf.push_str(&format!(
                    "  {:<10} {:<6} {:<40} {}\n",
                    f.severity.label(),
                    line_s,
                    short_path,
                    f.message
                ));
            }
        }
        buf.push_str(&format!(
            "\n  {} scanned, {} skipped | CRITICAL:{} HIGH:{} MEDIUM:{} LOW:{} INFO:{}\n\n",
            report.files_scanned,
            report.files_skipped,
            report.total_critical,
            report.total_high,
            report.total_medium,
            report.total_low,
            report.total_info,
        ));
        buf
    };

    if let Some(out_path) = &cmd.out {
        fs::write(out_path, &rendered)?;
        output::success(&format!("Report written to {}", out_path));
    }

    if is_json {
        println!("{rendered}");
    } else {
        print_report(&report);
    }

    Ok(report.exit_code())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_findings_valid_lines() {
        let raw = "CRITICAL|42|SQL injection risk\nHIGH|0|Unsafe unwrap\nNONE";
        let f = parse_findings(raw);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(f[0].line, Some(42));
        assert!(f[0].message.contains("SQL injection"));
        assert_eq!(f[1].severity, Severity::High);
    }

    #[test]
    fn parse_findings_none_yields_empty() {
        let f = parse_findings("NONE");
        assert!(f.is_empty());
    }

    #[test]
    fn parse_findings_prose_falls_back_to_info() {
        let raw = "This file looks fine, nothing to report here.";
        let f = parse_findings(raw);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Info);
        assert!(f[0].message.contains("looks fine"));
    }

    #[test]
    fn parse_findings_empty_input_is_info() {
        let f = parse_findings("   ");
        assert!(f.is_empty() || f[0].severity == Severity::Info);
    }

    #[test]
    fn ignore_pattern_filters_correctly() {
        let patterns = vec![
            "target".to_string(),
            "node_modules".to_string(),
            ".git".to_string(),
        ];
        assert!(should_ignore_dir("target", &patterns));
        assert!(should_ignore_dir("node_modules", &patterns));
        assert!(!should_ignore_dir("src", &patterns));
        assert!(!should_ignore_dir("cashell", &patterns));
        // extra ignore list
        assert!(should_ignore_dir("__pycache__", &patterns));
        assert!(should_ignore_dir(".venv", &patterns));
    }

    #[test]
    fn audit_exit_code_critical_is_2() {
        let mut r = AuditReport::default();
        r.total_critical = 1;
        assert_eq!(r.exit_code(), 2);
    }

    #[test]
    fn audit_exit_code_medium_is_1() {
        let mut r = AuditReport::default();
        r.total_medium = 3;
        assert_eq!(r.exit_code(), 1);
    }

    #[test]
    fn audit_exit_code_clean_is_0() {
        let r = AuditReport::default();
        assert_eq!(r.exit_code(), 0);
    }
}
