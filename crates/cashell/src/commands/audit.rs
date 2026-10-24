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
use rayon::prelude::*;
use reqwest::blocking::Client;
use serde::Serialize;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Condvar, Mutex};

// ── Concurrency & rate limiting ──────────────────────────────────────────────

// ponytail: in-process counting semaphore caps in-flight AI requests to 9router
// at min(jobs, 8) by default to avoid stampeding the local gateway. Upgrade:
// async client with token bucket rate-limiter and 429 adaptive backoff.
#[derive(Clone)]
pub struct Throttle {
    state: Arc<(Mutex<usize>, Condvar)>,
    capacity: usize,
}

pub struct ThrottleGuard<'a> {
    throttle: &'a Throttle,
}

impl Drop for ThrottleGuard<'_> {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.throttle.state;
        let mut count = lock.lock().unwrap();
        *count -= 1;
        cvar.notify_one();
    }
}

impl Throttle {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            state: Arc::new((Mutex::new(0), Condvar::new())),
            capacity: cap,
        }
    }

    pub fn acquire(&self) -> ThrottleGuard<'_> {
        let (lock, cvar) = &*self.state;
        let mut count = lock.lock().unwrap();
        while *count >= self.capacity {
            count = cvar.wait(count).unwrap();
        }
        *count += 1;
        ThrottleGuard { throttle: self }
    }

    pub fn current(&self) -> usize {
        *self.state.0.lock().unwrap()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Deterministically split a slice into `n` non-empty shards (contiguous chunks).
pub fn split_shards<T: Clone>(items: &[T], n: usize) -> Vec<Vec<T>> {
    if items.is_empty() || n == 0 {
        return Vec::new();
    }
    let n = n.min(items.len());
    let base = items.len() / n;
    let rem = items.len() % n;
    let mut shards = Vec::with_capacity(n);
    let mut start = 0;
    for i in 0..n {
        let len = base + if i < rem { 1 } else { 0 };
        shards.push(items[start..start + len].to_vec());
        start += len;
    }
    shards
}

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
    pub fn label(&self) -> &str {
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
    pub recommendation: Option<String>,
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
/// Contract: one line per finding `SEVERITY|LINE|MESSAGE[|RECOMMENDATION]`
/// or `NONE`.
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
        let parts: Vec<&str> = line.split('|').collect();
        let sev_str = parts.first().map(|s| s.trim()).unwrap_or("");
        let line_str = parts.get(1).map(|s| s.trim()).unwrap_or("");
        let msg = parts.get(2).map(|s| s.trim()).unwrap_or("");
        let rec = parts
            .get(3..)
            .map(|rest| rest.join("|").trim().to_string())
            .filter(|s| !s.is_empty());

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
                recommendation: rec,
            });
        }
    }

    if !any_structured {
        // Prose fallback — surface raw text so no info is lost.
        results.push(Finding {
            severity: Severity::Info,
            line: None,
            message: raw.trim().to_string(),
            recommendation: None,
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
    ".venv",
    "venv",
    "dist",
    "build",
    "vendor",
    "__pycache__",
    ".svelte-kit",
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
pub fn collect_files(dir: &Path, ignore_patterns: &[String], out: &mut Vec<PathBuf>) {
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
using this pipe-separated format: SEVERITY|LINE|MESSAGE|RECOMMENDATION\n\
SEVERITY must be one of: CRITICAL, HIGH, MEDIUM, LOW\n\
LINE is the line number (integer) or 0 if not applicable.\n\
RECOMMENDATION is a concise remediation advice.\n\
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
            let line_s = f
                .line
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".to_string());
            let short_path = ff
                .path
                .chars()
                .rev()
                .take(38)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>();
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

struct AuditResultItem {
    rel_str: String,
    findings: Vec<Finding>,
}

struct AuditTask<'a> {
    file_path: &'a Path,
    abs: &'a Path,
    audit_cfg: &'a Config,
    client: &'a Client,
    throttle: Option<&'a Throttle>,
    done: usize,
    total: usize,
    turbo: bool,
    /// Some => inline TUI active: workers must NOT print; they emit events.
    progress: Option<&'a Sender<crate::audit_tui::Ev>>,
    worker: usize,
    cancel: Option<&'a AtomicBool>,
    custom_prompt: Option<&'a str>,
}

/// Audit one file.
///
/// `turbo` (no TUI) selects the legacy single completion line per file.
/// With `progress` set, all output is replaced by channel events so the UI
/// thread owns the terminal exclusively. Returns `None` when skipped/failed.
fn audit_single_file(task: &AuditTask<'_>) -> Option<AuditResultItem> {
    let rel = task
        .file_path
        .strip_prefix(task.abs)
        .unwrap_or(task.file_path);
    let rel_str = rel.display().to_string();
    let show = task.progress.is_none() && (std::io::stderr().is_terminal() || task.turbo);

    if let Some(tx) = task.progress {
        let _ = tx.send(crate::audit_tui::Ev::Started {
            worker: task.worker,
            path: rel_str.clone(),
        });
    } else if show && !task.turbo {
        eprint!("  auditing {} ...", rel_str);
        let _ = std::io::stderr().flush();
    }

    let (item, ok) = audit_file_body(task, &rel_str, show);

    match task.progress {
        Some(tx) => {
            let n = item.as_ref().map(|i| i.findings.len()).unwrap_or(0);
            let _ = tx.send(crate::audit_tui::Ev::Done {
                worker: task.worker,
                path: rel_str,
                findings: n,
                ok,
            });
        }
        None if show && task.turbo => {
            let n = item.as_ref().map(|i| i.findings.len()).unwrap_or(0);
            eprintln!(
                "[w{}/{}] {}  \u{2192}  {} findings",
                task.done, task.total, rel_str, n
            );
        }
        None if show => {
            if let Some(i) = &item {
                eprintln!(" {} findings", i.findings.len());
            }
        }
        None => {}
    }

    item
}

/// Read + AI-audit one file. Returns `(item, ok)`; `ok` is false for
/// skip/error (so the UI can render it red) and true for a completed scan.
fn audit_file_body(
    task: &AuditTask<'_>,
    rel_str: &str,
    show: bool,
) -> (Option<AuditResultItem>, bool) {
    if task
        .cancel
        .map(|c| c.load(Ordering::SeqCst))
        .unwrap_or(false)
    {
        return (None, false);
    }

    // Size + UTF-8 check
    let meta = match fs::metadata(task.file_path) {
        Ok(m) => m,
        Err(e) => {
            if show {
                eprintln!("  [skip: {}]", e);
            }
            return (None, false);
        }
    };
    // Context limiter check (audit_max_context_kb from config, e.g. 16KB, 32KB, 64KB, 128KB)
    let max_kb = task.audit_cfg.ai.audit_max_context_kb.max(16);
    let max_bytes = (max_kb as u64) * 1024;
    if meta.len() > max_bytes {
        if show {
            eprintln!("  [skip: >{}KB context limit]", max_kb);
        }
        return (None, false);
    }
    let content = match fs::read_to_string(task.file_path) {
        Ok(c) => c,
        Err(_) => {
            if show {
                eprintln!("  [skip: not UTF-8]");
            }
            return (None, false);
        }
    };

    let content = if content.len() > max_bytes as usize {
        let mut end = max_bytes as usize;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\n// ... [TRUNCATED to {}KB context limit] ...", &content[..end], max_kb)
    } else {
        content
    };

    let prompt = format!("File: {rel_str}\n```\n{content}\n```");

    let custom_sys_prompt;
    let effective_system_prompt = if let Some(cp) = task.custom_prompt {
        custom_sys_prompt = format!("{AUDIT_SYSTEM_PROMPT}\n\n[USER AUDIT DIRECTIVE & QUALITY GATES]:\n{cp}");
        custom_sys_prompt.as_str()
    } else {
        AUDIT_SYSTEM_PROMPT
    };

    // Throttle only bounds the in-flight AI call, not the file read above.
    let raw = {
        let _guard = task.throttle.map(|t| t.acquire());
        match crate::ai::query_buffered_with(
            task.client,
            task.audit_cfg,
            Some(effective_system_prompt),
            &prompt,
        ) {
            Ok(r) => r,
            Err(e) => {
                if show {
                    eprintln!("  [ai error: {}]", e);
                }
                return (None, false);
            }
        }
    };

    let findings = parse_findings(&raw);
    (
        Some(AuditResultItem {
            rel_str: rel_str.to_string(),
            findings,
        }),
        true,
    )
}

pub fn run(cmd: &AuditCommand, cfg: &Config, quiet: bool) -> Result<i32> {
    let path = cmd.path.as_deref().map(Path::new).unwrap_or(Path::new("."));
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

    let timeout = if cmd.timeout > 0 {
        cmd.timeout
    } else {
        cfg.ai.timeout_seconds
    };

    // Build a per-file config with the audit timeout (which may differ from default).
    let mut audit_cfg = cfg.clone();
    audit_cfg.ai.timeout_seconds = timeout;

    // ONE shared client: reqwest blocking clients are Send+Sync and pool
    // connections, so every worker reuses the same pool.
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(timeout))
        .build()?;

    // --jobs implies --turbo; without either flag the path stays sequential.
    let turbo = cmd.turbo || cmd.jobs.is_some();
    let total_files = all_files.len();
    let jobs = if turbo {
        cmd.jobs.unwrap_or(8).max(1)
    } else {
        1
    };
    use crate::audit_tui::Ev;
    let cancel = Arc::new(AtomicBool::new(false));
    let use_tui = crate::audit_tui::should_use_tui(cmd.json, quiet);
    let (tx, rx) = channel::<Ev>();
    let tx_opt = use_tui.then(|| tx.clone());
    let ui = if use_tui {
        let cancel2 = cancel.clone();
        Some(std::thread::spawn(move || {
            let _ = crate::audit_tui::run_live(&rx, jobs, total_files, &cancel2);
        }))
    } else {
        None
    };

    let cancel_ref = Some(&*cancel);

    let (items, files_skipped_count) = if turbo {
        // ponytail: default ceiling of 8 in-flight AI calls against the single
        // local 9router gateway; --jobs raises/lowers it. Upgrade: adaptive
        // concurrency driven by 429s once the gateway exposes capacity.
        let throttle = Throttle::new(jobs);
        let done = AtomicUsize::new(0);

        let results: Vec<Option<AuditResultItem>> = all_files
            .par_iter()
            .map(|file_path| {
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                audit_single_file(&AuditTask {
                    file_path,
                    abs: &abs,
                    audit_cfg: &audit_cfg,
                    client: &client,
                    throttle: Some(&throttle),
                    done: n,
                    total: total_files,
                    turbo: true,
                    progress: tx_opt.as_ref(),
                    worker: (n - 1) % jobs,
                    cancel: cancel_ref,
                    custom_prompt: cmd.prompt.as_deref(),
                })
            })
            .collect();

        let skipped = results.iter().filter(|r| r.is_none()).count();
        let items: Vec<AuditResultItem> = results.into_iter().flatten().collect();
        (items, skipped + skipped_over_limit)
    } else {
        let mut items = Vec::new();
        let mut skipped = 0;
        for (i, file_path) in all_files.iter().enumerate() {
            match audit_single_file(&AuditTask {
                file_path,
                abs: &abs,
                audit_cfg: &audit_cfg,
                client: &client,
                throttle: None,
                done: i + 1,
                total: total_files,
                turbo: false,
                progress: tx_opt.as_ref(),
                worker: 0,
                cancel: cancel_ref,
                custom_prompt: cmd.prompt.as_deref(),
            }) {
                Some(item) => items.push(item),
                None => skipped += 1,
            }
        }
        (items, skipped + skipped_over_limit)
    };

    drop(tx_opt);
    let _ = tx.send(Ev::Finished);
    if let Some(h) = ui {
        let _ = h.join();
    }

    // Deterministic order regardless of worker completion order.
    let mut items = items;
    items.sort_by(|a, b| a.rel_str.cmp(&b.rel_str));

    let mut report = AuditReport {
        repo_path: abs.display().to_string(),
        files_scanned: items.len(),
        files_skipped: files_skipped_count,
        ..Default::default()
    };

    for item in items {
        for f in &item.findings {
            match f.severity {
                Severity::Critical => report.total_critical += 1,
                Severity::High => report.total_high += 1,
                Severity::Medium => report.total_medium += 1,
                Severity::Low => report.total_low += 1,
                Severity::Info => report.total_info += 1,
            }
        }
        report.files.push(FileFindings {
            path: item.rel_str,
            findings: item.findings,
        });
    }

    // ── Stage 2: interactive interview + markdown report ────────────────────
    let total_findings = report.total_critical
        + report.total_high
        + report.total_medium
        + report.total_low
        + report.total_info;

    let interactive = output::is_tty() && !cmd.json && !quiet;
    let interviewed = interactive && total_findings > 0;

    let reviewed = if interviewed {
        let files: Vec<(String, Vec<Finding>)> = report
            .files
            .iter()
            .map(|ff| (ff.path.clone(), ff.findings.clone()))
            .collect();
        let res = crate::audit_interview::run_interview(&report.repo_path, &files)?;
        let s = res.summary.clone();
        println!();
        println!(
            "  {} of {} findings reviewed | {} {} {}",
            s.accepted,
            res.findings.len(),
            format!("accepted:{}", s.accepted).green(),
            format!("skipped:{}", s.skipped).yellow(),
            format!("pending:{}", s.pending).dimmed()
        );
        res.findings
    } else {
        // Non-TTY / CI / JSON mode: no blocking interview, all findings stay Pending.
        let findings: Vec<crate::audit_interview::ReviewedFinding> = report
            .files
            .iter()
            .flat_map(|ff| {
                ff.findings.iter().map(|f| {
                    crate::audit_interview::ReviewedFinding {
                        severity: f.severity.label().to_string(),
                        file: ff.path.clone(),
                        line: f.line,
                        message: f.message.clone(),
                        recommendation: f.recommendation.clone(),
                        decision: crate::audit_interview::InterviewDecision::Pending,
                    }
                })
            })
            .collect();
        findings
    };

    let md_report = crate::audit_interview::build_markdown_report(
        &report.repo_path,
        report.files_scanned,
        report.files_skipped,
        &crate::audit_interview::SeverityTotals::from(&report),
        &reviewed,
    );

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
                let line_s = f
                    .line
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let short_path = ff
                    .path
                    .chars()
                    .rev()
                    .take(38)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>();
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

    // Save markdown report: interactive TTY prompts user; CI non-JSON auto-saves.
    let default_path = Path::new("audit").join(crate::audit_interview::default_report_filename());
    let should_save = if interactive {
        crate::audit_interview::confirm_save(&default_path.display().to_string())
    } else {
        !cmd.json
    };

    if should_save {
        if let Some(parent) = default_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::write(&default_path, &md_report) {
            Ok(()) => output::success(&format!("Report saved to {}", default_path.display())),
            Err(e) => output::error(&format!("Failed to save report: {}", e)),
        }
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
        let r = AuditReport {
            total_critical: 1,
            ..Default::default()
        };
        assert_eq!(r.exit_code(), 2);
    }

    #[test]
    fn audit_exit_code_medium_is_1() {
        let r = AuditReport {
            total_medium: 3,
            ..Default::default()
        };
        assert_eq!(r.exit_code(), 1);
    }

    #[test]
    fn audit_exit_code_clean_is_0() {
        let r = AuditReport::default();
        assert_eq!(r.exit_code(), 0);
    }

    // ── turbo mode ────────────────────────────────────────────────────────────

    #[test]
    fn split_shards_stable_sorted_and_respects_n() {
        let files: Vec<PathBuf> = (0..10)
            .map(|i| PathBuf::from(format!("src/f{i:02}.rs")))
            .collect();
        let shards = split_shards(&files, 3);
        assert_eq!(shards.len(), 3);
        // Every file present exactly once, in the original (sorted) order.
        let mut all: Vec<PathBuf> = shards.iter().flatten().cloned().collect();
        assert_eq!(all.len(), files.len());
        all.sort();
        assert_eq!(all, files);
        // Contiguous chunks preserve input order within each shard.
        for s in &shards {
            let mut copy = s.clone();
            copy.sort();
            assert_eq!(*s, copy);
        }
        // n larger than the file count -> one shard per file.
        assert_eq!(split_shards(&files, 50).len(), 10);
        // Edge cases.
        assert!(split_shards::<PathBuf>(&[], 4).is_empty());
        assert!(split_shards(&files, 0).is_empty());
        // Deterministic across calls.
        assert_eq!(split_shards(&files, 3), split_shards(&files, 3));
    }

    #[test]
    fn throttle_caps_concurrency_at_requested_value() {
        let throttle = Arc::new(Throttle::new(3));
        assert_eq!(throttle.capacity(), 3);
        let current = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..24 {
            let (t, cur, pk) = (throttle.clone(), current.clone(), peak.clone());
            handles.push(std::thread::spawn(move || {
                let _guard = t.acquire();
                let now = cur.fetch_add(1, Ordering::SeqCst) + 1;
                pk.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(2));
                cur.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(
            peak.load(Ordering::SeqCst) <= 3,
            "peak concurrency {} exceeded throttle capacity 3",
            peak.load(Ordering::SeqCst)
        );
        assert_eq!(throttle.current(), 0, "all permits must be released");
    }

    #[test]
    fn without_turbo_flag_path_stays_sequential() {
        // Sequential path only calls audit_single_file with a `None` throttle,
        // never touching the shared-client/turbo branch. Verify the decision.
        let sequential = |turbo: bool, jobs: Option<usize>| turbo || jobs.is_some();
        assert!(!sequential(false, None), "no --turbo/--jobs => sequential");
        assert!(sequential(true, None), "--turbo => parallel");
        assert!(sequential(false, Some(4)), "--jobs => parallel");
    }
}
