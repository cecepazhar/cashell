//! Remediation command — `cashell fix` — applies minimal AI diffs from audit findings.

use crate::cli::FixCommand;
use crate::config::Config;
use crate::output;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use colored::Colorize;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    style::Print,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{stdout, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

// ── Models ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingItem {
    pub severity: String,
    pub file: String,
    #[serde(default)]
    pub line: Option<u32>,
    pub message: String,
    #[serde(default)]
    pub recommendation: Option<String>,
    #[serde(default)]
    pub decision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixStatus {
    Applied,
    Rejected,
    Skipped,
    Failed,
}

impl FixStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Applied => "Applied",
            Self::Rejected => "Rejected",
            Self::Skipped => "Skipped",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixFileResult {
    pub file: String,
    pub status: FixStatus,
    #[serde(default)]
    pub diff: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixResultsPayload {
    pub results: Vec<FixFileResult>,
}

// ── Finding parser ────────────────────────────────────────────────────────────

/// Parse the embedded `<!-- cashell:findings ... -->` JSON block from the report.
pub fn parse_embedded_findings(markdown: &str) -> Result<Vec<FindingItem>> {
    const TAGS: &[&str] = &["<!-- cashell:findings", "<!--cashell:findings"];
    const END_TAG: &str = "-->";

    let mut start_idx = None;
    let mut tag_len = 0;
    for &tag in TAGS {
        if let Some(idx) = markdown.find(tag) {
            start_idx = Some(idx);
            tag_len = tag.len();
            break;
        }
    }

    let start_idx = start_idx.ok_or_else(|| {
        anyhow::anyhow!("no '<!-- cashell:findings' block found in audit report")
    })?;

    let content_start = start_idx + tag_len;
    let end_offset = markdown[content_start..]
        .find(END_TAG)
        .ok_or_else(|| anyhow::anyhow!("unclosed '<!-- cashell:findings' comment block"))?;

    let json_str = markdown[content_start..content_start + end_offset].trim();

    #[derive(Deserialize)]
    struct Payload {
        #[serde(default)]
        findings: Vec<FindingItem>,
    }

    // Try structured wrapper { "findings": [...] }
    if let Ok(payload) = serde_json::from_str::<Payload>(json_str) {
        if !payload.findings.is_empty() {
            return Ok(payload.findings);
        }
    }

    // Try bare array [ ... ]
    if let Ok(findings) = serde_json::from_str::<Vec<FindingItem>>(json_str) {
        return Ok(findings);
    }

    // Fall back to serde error reporting
    let payload: Payload = serde_json::from_str(json_str)
        .map_err(|e| anyhow::anyhow!("failed to parse embedded findings JSON: {e}"))?;
    Ok(payload.findings)
}

// ── Report discovery ──────────────────────────────────────────────────────────

/// Auto-detect the latest audit report `.md` file in `audit/` dir.
pub fn detect_latest_report(dir: &Path) -> Result<PathBuf> {
    if !dir.exists() || !dir.is_dir() {
        bail!(
            "audit directory '{}' not found. Run 'cashell audit' first to generate an audit report.",
            dir.display()
        );
    }

    let mut candidates = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if filename.ends_with(".md") && !filename.starts_with("Audit_Result") {
                let mtime = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                candidates.push((path, filename, mtime));
            }
        }
    }

    if candidates.is_empty() {
        bail!(
            "no audit report found in '{}'. Run 'cashell audit' first to generate an audit report.",
            dir.display()
        );
    }

    candidates.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.1.cmp(&a.1)));
    Ok(candidates[0].0.clone())
}

// ── Slug matching ─────────────────────────────────────────────────────────────

pub fn matches_slug(path_str: &str, slug: &str) -> bool {
    let p = Path::new(path_str);
    path_str == slug
        || path_str.ends_with(slug)
        || path_str.ends_with(&format!("/{}", slug))
        || p.file_name().and_then(|f| f.to_str()) == Some(slug)
        || p.file_stem().and_then(|f| f.to_str()) == Some(slug)
}

pub fn filter_by_slugs(files: &[String], slugs_str: &str) -> Vec<String> {
    let slugs: Vec<&str> = slugs_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if slugs.is_empty() {
        return files.to_vec();
    }
    files
        .iter()
        .filter(|f| slugs.iter().any(|slug| matches_slug(f, slug)))
        .cloned()
        .collect()
}

// ── Exit code mapping ─────────────────────────────────────────────────────────

/// Exit codes: 0 = all applied, 1 = some rejected/skipped, 2 = any failed.
pub fn calculate_exit_code(results: &[FixFileResult]) -> i32 {
    if results.iter().any(|r| r.status == FixStatus::Failed) {
        2
    } else if results
        .iter()
        .any(|r| r.status == FixStatus::Rejected || r.status == FixStatus::Skipped)
    {
        1
    } else {
        0
    }
}

// ── Unified diff parser & applier ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<String>,
}

pub fn extract_diff(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(start_fence) = trimmed.find("```") {
        let after_start = &trimmed[start_fence + 3..];
        let content_start = after_start.find('\n').map(|i| i + 1).unwrap_or(0);
        let inner = &after_start[content_start..];
        if let Some(end_fence) = inner.find("```") {
            return inner[..end_fence].trim().to_string();
        }
    }
    if let Some(idx) = trimmed.find("--- ") {
        return trimmed[idx..].trim().to_string();
    }
    if let Some(idx) = trimmed.find("@@ ") {
        return trimmed[idx..].trim().to_string();
    }
    trimmed.to_string()
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize, usize, usize)> {
    let line = line.trim();
    let at_start = line.find("@@")?;
    let rest = &line[at_start + 2..];
    let at_end = rest.find("@@")?;
    let spec = rest[..at_end].trim();
    let mut parts = spec.split_whitespace();
    let old_part = parts.next()?.strip_prefix('-')?;
    let new_part = parts.next()?.strip_prefix('+')?;

    let (old_start, old_count) = if let Some((s, c)) = old_part.split_once(',') {
        (s.parse::<usize>().ok()?, c.parse::<usize>().ok()?)
    } else {
        (old_part.parse::<usize>().ok()?, 1)
    };

    let (new_start, new_count) = if let Some((s, c)) = new_part.split_once(',') {
        (s.parse::<usize>().ok()?, c.parse::<usize>().ok()?)
    } else {
        (new_part.parse::<usize>().ok()?, 1)
    };

    Some((old_start, old_count, new_start, new_count))
}

pub fn parse_unified_diff(raw_diff: &str) -> Result<Vec<DiffHunk>> {
    let cleaned = extract_diff(raw_diff);
    let mut hunks = Vec::new();
    let mut current_hunk: Option<DiffHunk> = None;

    for line in cleaned.lines() {
        if line.starts_with("@@") {
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }
            if let Some((old_start, old_count, new_start, new_count)) = parse_hunk_header(line) {
                current_hunk = Some(DiffHunk {
                    old_start,
                    old_count,
                    new_start,
                    new_count,
                    lines: Vec::new(),
                });
            }
        } else if let Some(ref mut hunk) = current_hunk {
            if line.starts_with('+')
                || line.starts_with('-')
                || line.starts_with(' ')
                || line.starts_with('\\')
            {
                hunk.lines.push(line.to_string());
            } else if line.is_empty() {
                hunk.lines.push(" ".to_string());
            }
        }
    }

    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }

    if hunks.is_empty() {
        let diff_lines: Vec<String> = cleaned
            .lines()
            .filter(|l| l.starts_with('+') || l.starts_with('-') || l.starts_with(' '))
            .map(|l| l.to_string())
            .collect();
        if !diff_lines.is_empty()
            && diff_lines
                .iter()
                .any(|l| l.starts_with('+') || l.starts_with('-'))
        {
            hunks.push(DiffHunk {
                old_start: 1,
                old_count: diff_lines
                    .iter()
                    .filter(|l| l.starts_with(' ') || l.starts_with('-'))
                    .count(),
                new_start: 1,
                new_count: diff_lines
                    .iter()
                    .filter(|l| l.starts_with(' ') || l.starts_with('+'))
                    .count(),
                lines: diff_lines,
            });
        }
    }

    if hunks.is_empty() {
        bail!("no valid diff hunks found in AI response");
    }

    Ok(hunks)
}

pub fn apply_diff(original: &str, hunks: &[DiffHunk]) -> Result<String> {
    if hunks.is_empty() {
        return Ok(original.to_string());
    }

    let orig_lines: Vec<String> = original.lines().map(|s| s.to_string()).collect();

    struct HunkApplication {
        matched_pos: usize,
        old_len: usize,
        replacement: Vec<String>,
    }

    let mut applications = Vec::with_capacity(hunks.len());

    for hunk in hunks {
        let expected_old: Vec<String> = hunk
            .lines
            .iter()
            .filter(|l| l.starts_with(' ') || l.starts_with('-'))
            .map(|l| l[1..].to_string())
            .collect();

        let replacement: Vec<String> = hunk
            .lines
            .iter()
            .filter(|l| l.starts_with(' ') || l.starts_with('+'))
            .map(|l| l[1..].to_string())
            .collect();

        if expected_old.is_empty() {
            let pos = hunk.old_start.saturating_sub(1).min(orig_lines.len());
            applications.push(HunkApplication {
                matched_pos: pos,
                old_len: 0,
                replacement,
            });
            continue;
        }

        let target_pos = hunk.old_start.saturating_sub(1);
        let max_scan = orig_lines.len().saturating_sub(expected_old.len());
        let mut found_pos = None;

        // 1. Exact match at target position
        if target_pos <= max_scan
            && orig_lines[target_pos..target_pos + expected_old.len()] == *expected_old.as_slice()
        {
            found_pos = Some(target_pos);
        }

        // 2. Exact match within +/- 50 lines
        if found_pos.is_none() {
            let window = 50;
            let start = target_pos.saturating_sub(window);
            let end = (target_pos + window).min(max_scan);
            for p in start..=end {
                if orig_lines[p..p + expected_old.len()] == *expected_old.as_slice() {
                    found_pos = Some(p);
                    break;
                }
            }
        }

        // 3. Scan whole file exact match
        if found_pos.is_none() {
            for p in 0..=max_scan {
                if orig_lines[p..p + expected_old.len()] == *expected_old.as_slice() {
                    found_pos = Some(p);
                    break;
                }
            }
        }

        // 4. Fuzzy match: trim whitespace
        if found_pos.is_none() {
            for p in 0..=max_scan {
                let matches_trimmed = (0..expected_old.len())
                    .all(|i| orig_lines[p + i].trim() == expected_old[i].trim());
                if matches_trimmed {
                    found_pos = Some(p);
                    break;
                }
            }
        }

        match found_pos {
            Some(p) => applications.push(HunkApplication {
                matched_pos: p,
                old_len: expected_old.len(),
                replacement,
            }),
            None => bail!(
                "failed to find matching context for diff hunk starting at line {}",
                hunk.old_start
            ),
        }
    }

    // Sort descending by position so earlier replacements do not invalidate later positions
    applications.sort_by_key(|a| std::cmp::Reverse(a.matched_pos));

    // Check for overlapping hunks
    for i in 1..applications.len() {
        let prev = &applications[i];
        let next = &applications[i - 1];
        if prev.matched_pos + prev.old_len > next.matched_pos {
            bail!("conflicting or overlapping diff hunks cannot be applied");
        }
    }

    let mut result_lines = orig_lines;
    for app in applications {
        result_lines.splice(
            app.matched_pos..app.matched_pos + app.old_len,
            app.replacement,
        );
    }

    let mut out = result_lines.join("\n");
    if (original.ends_with('\n') || !out.is_empty()) && !out.ends_with('\n') {
        out.push('\n');
    }

    // Safety guardrail: NEVER delete entire file
    if out.trim().is_empty() && !original.trim().is_empty() {
        bail!("safety guardrail: refusing to delete entire file");
    }

    Ok(out)
}

// ── Diff rendering & interactive confirm ──────────────────────────────────────

pub fn display_diff(diff: &str) {
    println!("\n  {}", "─".repeat(70));
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            println!("  {}", line.bold());
        } else if line.starts_with('+') {
            println!("  {}", line.green());
        } else if line.starts_with('-') {
            println!("  {}", line.red());
        } else if line.starts_with("@@") {
            println!("  {}", line.cyan());
        } else {
            println!("  {}", line);
        }
    }
    println!("  {}\n", "─".repeat(70));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    Apply,
    Reject,
    Quit,
}

pub fn prompt_confirm() -> Result<ConfirmAction> {
    print!("  Apply this diff? [Enter/y: apply, n: reject, q: stop] ");
    stdout().flush()?;

    enable_raw_mode()?;
    let action = loop {
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                            break ConfirmAction::Apply;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') => {
                            break ConfirmAction::Reject;
                        }
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                            break ConfirmAction::Quit;
                        }
                        _ => {}
                    }
                }
            }
        }
    };
    let _ = disable_raw_mode();
    println!();
    Ok(action)
}

// ── Interactive Candidate File Selection ──────────────────────────────────────

pub fn select_files_interactive(files: &[String]) -> Result<Vec<String>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    if files.len() == 1 {
        return Ok(files.to_vec());
    }

    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = execute!(stdout(), cursor::Show);
            let _ = disable_raw_mode();
        }
    }

    enable_raw_mode()?;
    let _guard = Guard;
    let mut out = stdout();
    let _ = execute!(out, cursor::Hide);

    let mut cursor_idx = 0usize;
    let mut selected = vec![true; files.len()];
    let mut lines_rendered = 0usize;

    loop {
        if lines_rendered > 0 {
            execute!(out, cursor::MoveToPreviousLine(lines_rendered as u16))?;
        }

        let mut lines = Vec::new();
        lines.push(format!("  {}", "cashell fix — Candidate files:".bold()));
        lines.push(format!(
            "  {}",
            "[↑/↓/j/k] Navigate  [Space] Toggle  [a] Toggle all  [Enter] Confirm  [q] Abort"
                .dimmed()
        ));
        lines.push(String::new());

        for (i, f) in files.iter().enumerate() {
            let pointer = if i == cursor_idx {
                ">".cyan().bold()
            } else {
                " ".normal()
            };
            let mark = if selected[i] {
                "[x]".green()
            } else {
                "[ ]".dimmed()
            };
            let name = if i == cursor_idx {
                f.bold()
            } else {
                f.normal()
            };
            lines.push(format!("  {} {} {}", pointer, mark, name));
        }

        for l in &lines {
            execute!(out, Clear(ClearType::CurrentLine), Print(l), Print("\r\n"))?;
        }
        out.flush()?;
        lines_rendered = lines.len();

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            cursor_idx = cursor_idx.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if cursor_idx + 1 < files.len() {
                                cursor_idx += 1;
                            }
                        }
                        KeyCode::Char(' ') => {
                            selected[cursor_idx] = !selected[cursor_idx];
                        }
                        KeyCode::Char('a') => {
                            let all_sel = selected.iter().all(|&s| s);
                            selected.fill(!all_sel);
                        }
                        KeyCode::Enter => {
                            if !selected.iter().any(|&s| s) {
                                selected[cursor_idx] = true;
                            }
                            break;
                        }
                        KeyCode::Char('q') | KeyCode::Esc => {
                            return Ok(Vec::new());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    drop(_guard);
    println!();

    let picked: Vec<String> = files
        .iter()
        .enumerate()
        .filter(|(i, _)| selected[*i])
        .map(|(_, f)| f.clone())
        .collect();

    Ok(picked)
}

// ── Markdown result builder ───────────────────────────────────────────────────

pub fn build_result_markdown(source_report_path: &str, results: &[FixFileResult]) -> String {
    let ts = Utc::now();
    let date_str = ts.format("%Y-%m-%d").to_string();
    let iso = ts.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let applied_count = results
        .iter()
        .filter(|r| r.status == FixStatus::Applied)
        .count();
    let rejected_count = results
        .iter()
        .filter(|r| r.status == FixStatus::Rejected)
        .count();
    let skipped_count = results
        .iter()
        .filter(|r| r.status == FixStatus::Skipped)
        .count();
    let failed_count = results
        .iter()
        .filter(|r| r.status == FixStatus::Failed)
        .count();
    let exit_code = calculate_exit_code(results);

    let mut md = String::new();
    md.push_str(&format!("# cashell fix — Audit Result — {}\n\n", date_str));
    md.push_str(&format!("- **Timestamp (UTC):** {}\n", iso));
    md.push_str(&format!("- **Source report:** `{}`\n", source_report_path));
    md.push_str(&format!("- **Files fixed:** {}\n\n", applied_count));

    md.push_str("## Summary\n\n");
    md.push_str("| Status | Count |\n|---|---|\n");
    md.push_str(&format!("| Applied | {} |\n", applied_count));
    md.push_str(&format!("| Rejected | {} |\n", rejected_count));
    md.push_str(&format!("| Skipped | {} |\n", skipped_count));
    md.push_str(&format!("| Failed | {} |\n", failed_count));
    md.push_str(&format!("| **Exit code** | **{}** |\n\n", exit_code));

    md.push_str("## File Results\n\n");
    for r in results {
        md.push_str(&format!("### `{}`\n\n", r.file));
        md.push_str(&format!("- **Status:** {}\n", r.status.label()));
        if let Some(err) = &r.error {
            md.push_str(&format!("- **Error:** {}\n", err));
        }
        if !r.diff.trim().is_empty() {
            md.push_str("\n```diff\n");
            md.push_str(r.diff.trim());
            md.push_str("\n```\n\n");
        } else {
            md.push_str("\n_No diff._\n\n");
        }
    }

    #[derive(Serialize)]
    struct ResultsPayload<'a> {
        results: &'a [FixFileResult],
    }

    let payload = ResultsPayload { results };
    let json = serde_json::to_string_pretty(&payload).unwrap_or_default();
    md.push_str("<!-- cashell:results\n");
    md.push_str(&json);
    md.push_str("\n-->\n");

    md
}

// ── System Prompt ─────────────────────────────────────────────────────────────

const FIX_SYSTEM_PROMPT: &str = "\
You are a code fixer. Given this file, its findings, and the user's interview decisions, \
propose a minimal targeted diff. Do NOT rewrite the whole file. Respond in unified diff format.";

// ── Main Runner ───────────────────────────────────────────────────────────────

pub fn run(cmd: &FixCommand, cfg: &Config) -> Result<i32> {
    // 1. Locate audit report
    let report_path = if let Some(p) = cmd.report_path.as_ref().or(cmd.from.as_ref()) {
        let path = PathBuf::from(p);
        if !path.exists() {
            bail!(
                "report file '{}' not found. Run 'cashell audit' first to generate an audit report.",
                path.display()
            );
        }
        path
    } else {
        detect_latest_report(Path::new("audit"))?
    };

    let report_content = fs::read_to_string(&report_path)
        .with_context(|| format!("cannot read audit report '{}'", report_path.display()))?;

    // 2. Parse embedded findings
    let findings = parse_embedded_findings(&report_content)?;
    if findings.is_empty() {
        if !cmd.json {
            output::info("No findings found in the audit report.");
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&FixResultsPayload {
                    results: Vec::new()
                })?
            );
        }
        return Ok(0);
    }

    // 3. Group findings by file path
    let mut file_groups: BTreeMap<String, Vec<FindingItem>> = BTreeMap::new();
    for f in findings {
        file_groups.entry(f.file.clone()).or_default().push(f);
    }

    let all_candidate_files: Vec<String> = file_groups.keys().cloned().collect();

    // 4. Filter candidate files: --files or interactive selection
    let interactive =
        output::is_tty() && std::io::stdin().is_terminal() && !cmd.json && !cmd.yes;

    let files_to_process = if let Some(slugs) = &cmd.files {
        let filtered = filter_by_slugs(&all_candidate_files, slugs);
        if filtered.is_empty() {
            output::error(&format!("No files matched --files filter: {slugs}"));
            return Ok(1);
        }
        filtered
    } else if interactive {
        let picked = select_files_interactive(&all_candidate_files)?;
        if picked.is_empty() {
            output::info("No files selected for fix.");
            return Ok(0);
        }
        picked
    } else {
        all_candidate_files
    };

    // 5. Configure AI client
    let timeout = cmd.timeout.unwrap_or(cfg.ai.timeout_seconds);
    let mut fix_cfg = cfg.clone();
    fix_cfg.ai.timeout_seconds = timeout;

    let mut results: Vec<FixFileResult> = Vec::new();

    // 6. Process each selected file
    for (idx, file_path) in files_to_process.iter().enumerate() {
        let findings_for_file = match file_groups.get(file_path) {
            Some(f) => f,
            None => continue,
        };

        if !cmd.json {
            output::info(&format!(
                "Processing {}/{} — {} ({} findings)",
                idx + 1,
                files_to_process.len(),
                file_path.bold(),
                findings_for_file.len()
            ));
        }

        // Read source file
        let source_content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                output::error(&format!("Cannot read source file '{file_path}': {e}"));
                results.push(FixFileResult {
                    file: file_path.to_string(),
                    status: FixStatus::Failed,
                    diff: String::new(),
                    error: Some(format!("failed to read source file: {e}")),
                });
                continue;
            }
        };

        // Format findings & decisions for AI
        let mut findings_desc = String::new();
        for (f_idx, f) in findings_for_file.iter().enumerate() {
            let line_s = f
                .line
                .map(|l| l.to_string())
                .unwrap_or_else(|| "-".to_string());
            findings_desc.push_str(&format!(
                "{}. [{}] line {}: {}\n   Recommendation: {}\n   User interview decision: {}\n",
                f_idx + 1,
                f.severity,
                line_s,
                f.message,
                f.recommendation.as_deref().unwrap_or("—"),
                f.decision.as_deref().unwrap_or("Pending")
            ));
        }

        let user_prompt = format!(
            "File: {file_path}\n\nFindings and interview decisions:\n{findings_desc}\n\nSource code of {file_path}:\n```\n{source_content}\n```"
        );

        // Query AI
        let raw_reply = match crate::ai::query_buffered(
            &fix_cfg,
            Some(FIX_SYSTEM_PROMPT),
            &user_prompt,
        ) {
            Ok(r) => r,
            Err(e) => {
                output::error(&format!("AI query failed for '{file_path}': {e}"));
                results.push(FixFileResult {
                    file: file_path.to_string(),
                    status: FixStatus::Failed,
                    diff: String::new(),
                    error: Some(format!("AI query failed: {e}")),
                });
                continue;
            }
        };

        let cleaned_diff = extract_diff(&raw_reply);
        if cleaned_diff.is_empty() || cleaned_diff.eq_ignore_ascii_case("NONE") {
            output::info(&format!("No changes needed for {file_path}"));
            results.push(FixFileResult {
                file: file_path.to_string(),
                status: FixStatus::Skipped,
                diff: String::new(),
                error: None,
            });
            continue;
        }

        // Parse unified diff hunks
        let hunks = match parse_unified_diff(&cleaned_diff) {
            Ok(h) => h,
            Err(e) => {
                output::error(&format!("Failed to parse diff for '{file_path}': {e}"));
                results.push(FixFileResult {
                    file: file_path.to_string(),
                    status: FixStatus::Failed,
                    diff: cleaned_diff,
                    error: Some(format!("diff parsing failed: {e}")),
                });
                continue;
            }
        };

        // Apply diff in-memory
        let new_content = match apply_diff(&source_content, &hunks) {
            Ok(c) => c,
            Err(e) => {
                output::error(&format!("Failed to apply diff to '{file_path}': {e}"));
                results.push(FixFileResult {
                    file: file_path.to_string(),
                    status: FixStatus::Failed,
                    diff: cleaned_diff,
                    error: Some(format!("diff application failed: {e}")),
                });
                continue;
            }
        };

        // Interactive confirm or auto-accept
        if interactive {
            display_diff(&cleaned_diff);
            match prompt_confirm()? {
                ConfirmAction::Apply => {}
                ConfirmAction::Reject => {
                    output::info(&format!("Rejected changes for {file_path}"));
                    results.push(FixFileResult {
                        file: file_path.to_string(),
                        status: FixStatus::Rejected,
                        diff: cleaned_diff,
                        error: None,
                    });
                    continue;
                }
                ConfirmAction::Quit => {
                    output::info("Stopping fix process.");
                    results.push(FixFileResult {
                        file: file_path.to_string(),
                        status: FixStatus::Skipped,
                        diff: cleaned_diff,
                        error: None,
                    });
                    for rem in files_to_process.iter().skip(idx + 1) {
                        results.push(FixFileResult {
                            file: rem.to_string(),
                            status: FixStatus::Skipped,
                            diff: String::new(),
                            error: None,
                        });
                    }
                    break;
                }
            }
        }

        // Write changes or dry-run
        if cmd.dry_run {
            output::success(&format!(
                "[dry-run] Validated diff for {file_path} (no changes written)"
            ));
            results.push(FixFileResult {
                file: file_path.to_string(),
                status: FixStatus::Applied,
                diff: cleaned_diff,
                error: None,
            });
        } else if !new_content.is_empty() {
            if let Err(e) = fs::write(file_path, &new_content) {
                output::error(&format!("Failed to write to '{file_path}': {e}"));
                results.push(FixFileResult {
                    file: file_path.to_string(),
                    status: FixStatus::Failed,
                    diff: cleaned_diff,
                    error: Some(format!("file write failed: {e}")),
                });
                continue;
            }
            output::success(&format!("Applied fix to {file_path}"));
            results.push(FixFileResult {
                file: file_path.to_string(),
                status: FixStatus::Applied,
                diff: cleaned_diff,
                error: None,
            });
        } else {
            output::error(&format!(
                "Safety guardrail: refusing to write empty content to {file_path}"
            ));
            results.push(FixFileResult {
                file: file_path.to_string(),
                status: FixStatus::Failed,
                diff: cleaned_diff,
                error: Some("refusing to write empty content".to_string()),
            });
        }
    }

    // 7. Write Audit_Result_YYYY-MM-DD.md
    let date = chrono::Local::now().format("%Y-%m-%d");
    let result_filename = format!("Audit_Result_{}.md", date);
    let result_path = if let Some(parent) = report_path.parent() {
        if parent.as_os_str().is_empty() {
            PathBuf::from(&result_filename)
        } else {
            parent.join(&result_filename)
        }
    } else {
        PathBuf::from(&result_filename)
    };

    let md_report = build_result_markdown(&report_path.display().to_string(), &results);
    if let Some(parent) = result_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&result_path, &md_report)
        .with_context(|| format!("cannot write result file '{}'", result_path.display()))?;

    let exit_code = calculate_exit_code(&results);

    // 8. Output summary
    if cmd.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&FixResultsPayload { results })?
        );
    } else {
        println!();
        output::success(&format!("Results saved to {}", result_path.display()));
        let applied = results
            .iter()
            .filter(|r| r.status == FixStatus::Applied)
            .count();
        let rejected = results
            .iter()
            .filter(|r| r.status == FixStatus::Rejected)
            .count();
        let skipped = results
            .iter()
            .filter(|r| r.status == FixStatus::Skipped)
            .count();
        let failed = results
            .iter()
            .filter(|r| r.status == FixStatus::Failed)
            .count();
        println!(
            "  {} files processed | {} {} {} {}",
            results.len(),
            format!("applied:{applied}").green(),
            format!("rejected:{rejected}").yellow(),
            format!("skipped:{skipped}").dimmed(),
            format!("failed:{failed}").red()
        );
        println!();
    }

    Ok(exit_code)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_embedded_findings_from_markdown() {
        let markdown = r#"
# cashell audit — 2026-10-14

- **Repo path:** `/tmp/sample-repo`

## Findings
| Severity | File | Line | Message | Recommendation | Decision |
|---|---|---|---|---|---|
| HIGH | `src/main.rs` | 42 | Unhandled unwrap | Use ok_or_else | Accepted |

<!-- cashell:findings
{
  "schema": "cashell-fix-findings/v1",
  "timestamp": "2026-10-14T00:00:00Z",
  "repo_path": "/tmp/sample-repo",
  "findings": [
    {
      "severity": "HIGH",
      "file": "src/main.rs",
      "line": 42,
      "message": "Unhandled unwrap",
      "recommendation": "Use ok_or_else",
      "decision": "Accepted"
    },
    {
      "severity": "MEDIUM",
      "file": "src/ai.rs",
      "line": null,
      "message": "Missing timeout check",
      "recommendation": null,
      "decision": "Skipped"
    }
  ]
}
-->
"#;

        let findings = parse_embedded_findings(markdown).expect("should parse embedded findings");
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, "HIGH");
        assert_eq!(findings[0].file, "src/main.rs");
        assert_eq!(findings[0].line, Some(42));
        assert_eq!(findings[0].message, "Unhandled unwrap");
        assert_eq!(
            findings[0].recommendation.as_deref(),
            Some("Use ok_or_else")
        );
        assert_eq!(findings[0].decision.as_deref(), Some("Accepted"));

        assert_eq!(findings[1].severity, "MEDIUM");
        assert_eq!(findings[1].file, "src/ai.rs");
        assert_eq!(findings[1].line, None);
        assert_eq!(findings[1].decision.as_deref(), Some("Skipped"));
    }

    #[test]
    fn test_exit_code_mapping() {
        // all applied = 0
        let all_applied = vec![
            FixFileResult {
                file: "a.rs".into(),
                status: FixStatus::Applied,
                diff: "".into(),
                error: None,
            },
            FixFileResult {
                file: "b.rs".into(),
                status: FixStatus::Applied,
                diff: "".into(),
                error: None,
            },
        ];
        assert_eq!(calculate_exit_code(&all_applied), 0);

        // some rejected / skipped = 1
        let some_rejected = vec![
            FixFileResult {
                file: "a.rs".into(),
                status: FixStatus::Applied,
                diff: "".into(),
                error: None,
            },
            FixFileResult {
                file: "b.rs".into(),
                status: FixStatus::Rejected,
                diff: "".into(),
                error: None,
            },
        ];
        assert_eq!(calculate_exit_code(&some_rejected), 1);

        let some_skipped = vec![FixFileResult {
            file: "a.rs".into(),
            status: FixStatus::Skipped,
            diff: "".into(),
            error: None,
        }];
        assert_eq!(calculate_exit_code(&some_skipped), 1);

        // any failed = 2
        let any_failed = vec![
            FixFileResult {
                file: "a.rs".into(),
                status: FixStatus::Applied,
                diff: "".into(),
                error: None,
            },
            FixFileResult {
                file: "b.rs".into(),
                status: FixStatus::Rejected,
                diff: "".into(),
                error: None,
            },
            FixFileResult {
                file: "c.rs".into(),
                status: FixStatus::Failed,
                diff: "".into(),
                error: Some("syntax error".into()),
            },
        ];
        assert_eq!(calculate_exit_code(&any_failed), 2);

        // empty = 0
        assert_eq!(calculate_exit_code(&[]), 0);
    }

    #[test]
    fn test_files_filter_picks_only_listed_slugs() {
        let files = vec![
            "src/commands/fix.rs".to_string(),
            "src/main.rs".to_string(),
            "src/ai.rs".to_string(),
            "src/output.rs".to_string(),
        ];

        let filtered = filter_by_slugs(&files, "fix.rs,main.rs");
        assert_eq!(
            filtered,
            vec![
                "src/commands/fix.rs".to_string(),
                "src/main.rs".to_string()
            ]
        );

        let filtered_stem = filter_by_slugs(&files, "ai");
        assert_eq!(filtered_stem, vec!["src/ai.rs".to_string()]);

        let filtered_full = filter_by_slugs(&files, "src/output.rs");
        assert_eq!(filtered_full, vec!["src/output.rs".to_string()]);

        let filtered_none = filter_by_slugs(&files, "nonexistent.rs");
        assert!(filtered_none.is_empty());
    }

    #[test]
    fn test_apply_unified_diff_single_hunk() {
        let original = "fn main() {\n    let old = 1;\n    println!(\"{old}\");\n}\n";
        let diff = r#"
--- a/src/main.rs
+++ b/src/main.rs
@@ -2,2 +2,2 @@
-    let old = 1;
-    println!("{old}");
+    let new = 2;
+    println!("{new}");
"#;
        let hunks = parse_unified_diff(diff).expect("diff should parse");
        let result = apply_diff(original, &hunks).expect("diff should apply");
        assert_eq!(
            result,
            "fn main() {\n    let new = 2;\n    println!(\"{new}\");\n}\n"
        );
    }

    #[test]
    fn test_apply_unified_diff_multiple_hunks() {
        let original = "line1\nline2\nline3\nline4\nline5\n";
        let diff = r#"
@@ -1,1 +1,1 @@
-line1
+line1_modified
@@ -5,1 +5,1 @@
-line5
+line5_modified
"#;
        let hunks = parse_unified_diff(diff).expect("diff should parse");
        let result = apply_diff(original, &hunks).expect("diff should apply");
        assert_eq!(result, "line1_modified\nline2\nline3\nline4\nline5_modified\n");
    }

    #[test]
    fn test_apply_unified_diff_fuzzy_line_offset() {
        let original = "header\n\n\nfn foo() {\n    let x = 1;\n}\n";
        let diff = r#"
@@ -2,1 +2,1 @@
-    let x = 1;
+    let x = 42;
"#;
        let hunks = parse_unified_diff(diff).expect("diff should parse");
        let result = apply_diff(original, &hunks).expect("diff should apply fuzzy offset");
        assert_eq!(result, "header\n\n\nfn foo() {\n    let x = 42;\n}\n");
    }

    #[test]
    fn test_apply_unified_diff_safety_no_deletion() {
        let original = "important code\ndo not delete\n";
        let diff = r#"
@@ -1,2 +0,0 @@
-important code
-do not delete
"#;
        let hunks = parse_unified_diff(diff).expect("diff should parse");
        let err = apply_diff(original, &hunks).expect_err("should reject deletion");
        assert!(err.to_string().contains("safety guardrail"));
    }

    #[test]
    fn test_build_result_markdown_contains_expected_sections() {
        let results = vec![FixFileResult {
            file: "src/main.rs".to_string(),
            status: FixStatus::Applied,
            diff: "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,1 +1,1 @@\n-old\n+new".to_string(),
            error: None,
        }];
        let md = build_result_markdown("audit/Audit_2026-10-14.md", &results);
        assert!(md.contains("# cashell fix — Audit Result —"));
        assert!(md.contains("`audit/Audit_2026-10-14.md`"));
        assert!(md.contains("| Applied | 1 |"));
        assert!(md.contains("### `src/main.rs`"));
        assert!(md.contains("<!-- cashell:results"));
    }
}
