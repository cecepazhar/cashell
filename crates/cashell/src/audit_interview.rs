//! Interactive review interview for `cashell audit` findings.

use crate::commands::audit::{AuditReport, Finding, Severity};
use crate::output;
use anyhow::Result;
use chrono::Utc;
use colored::Colorize;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use serde::Serialize;
use std::io::{stdout, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum InterviewDecision {
    Accepted,
    Skipped,
    Pending,
}

impl InterviewDecision {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Accepted => "Accepted",
            Self::Skipped => "Skipped",
            Self::Pending => "Pending",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewedFinding {
    pub severity: String,
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
    pub recommendation: Option<String>,
    pub decision: InterviewDecision,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterviewSummary {
    pub accepted: usize,
    pub skipped: usize,
    pub pending: usize,
}

pub struct InterviewResult {
    pub findings: Vec<ReviewedFinding>,
    pub summary: InterviewSummary,
    pub quit: bool,
}

fn severity_colored(s: &Severity) -> String {
    match s {
        Severity::Critical => s.label().red().bold().to_string(),
        Severity::High => s.label().red().to_string(),
        Severity::Medium => s.label().yellow().to_string(),
        Severity::Low => s.label().blue().to_string(),
        Severity::Info => s.label().dimmed().to_string(),
    }
}

/// Run interactive interview for findings when in TTY mode and findings > 0.
pub fn run_interview(
    repo_path: &str,
    files: &[(String, Vec<Finding>)],
) -> Result<InterviewResult> {
    let mut flat: Vec<(&str, &Finding)> = Vec::new();
    for (path, fs) in files {
        for f in fs {
            flat.push((path.as_str(), f));
        }
    }

    if flat.is_empty() || !output::is_tty() {
        let findings: Vec<ReviewedFinding> = flat
            .into_iter()
            .map(|(file, f)| to_reviewed(file, f, InterviewDecision::Pending))
            .collect();
        return Ok(InterviewResult {
            summary: summarize(&findings),
            findings,
            quit: false,
        });
    }

    enable_raw_mode()?;
    let mut out = stdout();
    let _ = execute!(out, cursor::Hide);

    let mut reviewed: Vec<ReviewedFinding> = Vec::with_capacity(flat.len());
    let mut quit = false;

    for (i, (file, f)) in flat.iter().enumerate() {
        draw_finding_card(&mut out, i + 1, flat.len(), file, f)?;

        let decision = loop {
            if event::poll(std::time::Duration::from_millis(100))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        match k.code {
                            KeyCode::Enter | KeyCode::Char('y') => {
                                break InterviewDecision::Accepted;
                            }
                            KeyCode::Char('s') | KeyCode::Char('n') => {
                                break InterviewDecision::Skipped;
                            }
                            KeyCode::Char('q') | KeyCode::Esc => {
                                quit = true;
                                break InterviewDecision::Pending;
                            }
                            _ => {}
                        }
                    }
                }
            }
        };

        reviewed.push(to_reviewed(file, f, decision));

        if quit {
            break;
        }
    }

    // Mark remaining as Pending if quit early
    if quit && reviewed.len() < flat.len() {
        let start = reviewed.len();
        for (file, f) in flat.iter().skip(start) {
            reviewed.push(to_reviewed(file, f, InterviewDecision::Pending));
        }
    }

    let _ = execute!(out, cursor::Show);
    let _ = disable_raw_mode();
    let _ = writeln!(out, "\n");

    let _ = repo_path; // context for future fix; keep param for API symmetry

    Ok(InterviewResult {
        summary: summarize(&reviewed),
        findings: reviewed,
        quit,
    })
}

fn to_reviewed(file: &str, f: &Finding, decision: InterviewDecision) -> ReviewedFinding {
    ReviewedFinding {
        severity: f.severity.label().to_string(),
        file: file.to_string(),
        line: f.line,
        message: f.message.clone(),
        recommendation: f.recommendation.clone(),
        decision,
    }
}

fn summarize(findings: &[ReviewedFinding]) -> InterviewSummary {
    let accepted = findings.iter().filter(|f| f.decision == InterviewDecision::Accepted).count();
    let skipped = findings.iter().filter(|f| f.decision == InterviewDecision::Skipped).count();
    let pending = findings.len() - accepted - skipped;
    InterviewSummary {
        accepted,
        skipped,
        pending,
    }
}

fn draw_finding_card(
    out: &mut std::io::Stdout,
    curr: usize,
    total: usize,
    file: &str,
    f: &Finding,
) -> Result<()> {
    let sev = severity_colored(&f.severity);
    let line_str = f.line.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string());
    let rec_str = f
        .recommendation
        .as_deref()
        .map(|r| r.to_string())
        .unwrap_or_else(|| "—".to_string());

    let _ = writeln!(out, "  {}", "cashell audit — Interactive Interview".bold());
    let _ = writeln!(out, "  Finding {} of {}\n", curr, total);
    let _ = writeln!(out, "  {}  {}", "Severity:".dimmed(), sev);
    let _ = writeln!(out, "  {}  {}", "File:    ".dimmed(), file.cyan());
    let _ = writeln!(out, "  {}  {}\n", "Line:    ".dimmed(), line_str);
    let _ = writeln!(out, "  {}  {}", "Message: ".dimmed(), f.message);
    let _ = writeln!(out, "  {}  {}\n", "Fix:     ".dimmed(), rec_str.dimmed());
    let _ = writeln!(out, "  {}", "─".repeat(60));
    let _ = writeln!(
        out,
        "  [Enter/y] Accept & Next    [s/n] Skip / Ignore    [q] Quit & Save"
    );
    let _ = out.flush();
    Ok(())
}

/// Default audit report filename: `Audit_YYYY-MM-DD.md`.
pub fn default_report_filename() -> String {
    let date = chrono::Local::now().format("%Y-%m-%d");
    format!("Audit_{}.md", date)
}

/// Default audit report path: `audit/Audit_YYYY-MM-DD.md` relative to cwd.
pub fn default_report_path() -> String {
    format!("audit/{}", default_report_filename())
}

/// Ask whether to save the markdown report to `audit/Audit_YYYY-MM-DD.md`.
/// Non-TTY always returns `true` (auto-save) so CI pipelines get the artifact.
pub fn confirm_save(path_display: &str) -> bool {
    if !output::is_tty() {
        return true;
    }
    print!("  Save report to {}? [Y/n] ", path_display);
    let _ = stdout().flush();
    if enable_raw_mode().is_ok() {
        let res = loop {
            if let Ok(Event::Key(k)) = event::read() {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => break true,
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') => break false,
                        _ => {}
                    }
                }
            }
        };
        let _ = disable_raw_mode();
        println!();
        res
    } else {
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let t = input.trim();
            !t.eq_ignore_ascii_case("n") && !t.eq_ignore_ascii_case("no")
        } else {
            true
        }
    }
}

/// Build the markdown report with embedded JSON for `cashell fix` consumption.
pub fn build_markdown_report(
    repo_path: &str,
    files_scanned: usize,
    files_skipped: usize,
    totals: &SeverityTotals,
    findings: &[ReviewedFinding],
) -> String {
    let ts = Utc::now();
    let date_str = ts.format("%Y-%m-%d").to_string();
    let iso = ts.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let mut md = String::new();
    md.push_str(&format!("# cashell audit — {}\n\n", date_str));
    md.push_str(&format!("- **Repo path:** `{}`\n", repo_path));
    md.push_str(&format!("- **Timestamp (UTC):** {}\n", iso));
    md.push_str(&format!("- **Files scanned:** {}\n", files_scanned));
    md.push_str(&format!("- **Files skipped:** {}\n\n", files_skipped));

    md.push_str("## Summary\n\n");
    md.push_str("| Severity | Count |\n|---|---|\n");
    md.push_str(&format!("| CRITICAL | {} |\n", totals.critical));
    md.push_str(&format!("| HIGH | {} |\n", totals.high));
    md.push_str(&format!("| MEDIUM | {} |\n", totals.medium));
    md.push_str(&format!("| LOW | {} |\n", totals.low));
    md.push_str(&format!("| INFO | {} |\n\n", totals.info));

    md.push_str("## Findings\n\n");
    if findings.is_empty() {
        md.push_str("_No findings._\n");
    } else {
        md.push_str("| Severity | File | Line | Message | Recommendation | Decision |\n");
        md.push_str("|---|---|---|---|---|---|\n");
        for f in findings {
            let line_s = f.line.map(|n| n.to_string()).unwrap_or_else(|| "-".into());
            let rec_s = f
                .recommendation
                .as_deref()
                .unwrap_or("—")
                .replace('|', "\\|")
                .replace('\n', " ");
            let msg_s = f.message.replace('|', "\\|").replace('\n', " ");
            md.push_str(&format!(
                "| {} | `{}` | {} | {} | {} | {} |\n",
                f.severity, f.file, line_s, msg_s, rec_s, f.decision.label()
            ));
        }
    }

    // Embedded JSON for `cashell fix` consumption.
    #[derive(Serialize)]
    struct FixPayload {
        schema: &'static str,
        timestamp: String,
        repo_path: String,
        findings: Vec<ReviewedFinding>,
    }
    let payload = FixPayload {
        schema: "cashell-fix-findings/v1",
        timestamp: iso,
        repo_path: repo_path.to_string(),
        findings: findings.to_vec(),
    };
    let json = serde_json::to_string_pretty(&payload).unwrap_or_default();
    md.push_str("\n<!-- cashell:findings\n");
    md.push_str(&json);
    md.push_str("\n-->\n");
    md
}

/// Severity totals snapshot for markdown rendering.
pub struct SeverityTotals {
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub info: usize,
}

impl From<&AuditReport> for SeverityTotals {
    fn from(r: &AuditReport) -> Self {
        Self {
            critical: r.total_critical,
            high: r.total_high,
            medium: r.total_medium,
            low: r.total_low,
            info: r.total_info,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_findings() -> Vec<ReviewedFinding> {
        vec![
            ReviewedFinding {
                severity: "MEDIUM".to_string(),
                file: "src/main.rs".to_string(),
                line: Some(42),
                message: "Missing file size check before read".to_string(),
                recommendation: Some("Verify file metadata before reading".to_string()),
                decision: InterviewDecision::Accepted,
            },
            ReviewedFinding {
                severity: "HIGH".to_string(),
                file: "src/audit.rs".to_string(),
                line: None,
                message: "Unsafe unwrap on network response".to_string(),
                recommendation: None,
                decision: InterviewDecision::Skipped,
            },
        ]
    }

    #[test]
    fn build_markdown_report_includes_severity_table() {
        let totals = SeverityTotals {
            critical: 0,
            high: 1,
            medium: 1,
            low: 0,
            info: 0,
        };
        let md = build_markdown_report(
            "/tmp/fake-repo",
            2,
            0,
            &totals,
            &sample_findings(),
        );
        assert!(md.contains("## Summary"), "missing summary section");
        assert!(md.contains("| HIGH | 1 |"), "missing HIGH count");
        assert!(md.contains("| MEDIUM | 1 |"), "missing MEDIUM count");
        assert!(md.contains("src/main.rs"), "missing file reference");
    }

    #[test]
    fn build_markdown_report_embeds_json_for_cashell_fix() {
        let totals = SeverityTotals::default_like_zero();
        let md = build_markdown_report("/tmp/r", 0, 0, &totals, &[]);
        assert!(
            md.contains("<!-- cashell:findings"),
            "missing embedded JSON block"
        );
        assert!(
            md.contains("cashell-fix-findings/v1"),
            "missing schema marker"
        );
        // JSON block must be valid after extracting content between markers
        let start = md.find("{").expect("no JSON start");
        let end = md.rfind("}").expect("no JSON end");
        let json_slice = &md[start..=end];
        let v: serde_json::Value = serde_json::from_str(json_slice).expect("bad JSON");
        assert_eq!(v["schema"], "cashell-fix-findings/v1");
    }

    #[test]
    fn decision_label_is_correct() {
        assert_eq!(InterviewDecision::Accepted.label(), "Accepted");
        assert_eq!(InterviewDecision::Skipped.label(), "Skipped");
        assert_eq!(InterviewDecision::Pending.label(), "Pending");
    }

    trait DefaultLikeZero {
        fn default_like_zero() -> Self;
    }
    impl DefaultLikeZero for SeverityTotals {
        fn default_like_zero() -> Self {
            Self {
                critical: 0,
                high: 0,
                medium: 0,
                low: 0,
                info: 0,
            }
        }
    }
}

