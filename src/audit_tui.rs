//! Live inline progress for `cashell audit` (Claude Code style).
//!
//! Direct crossterm line rendering — never AlternateScreen, so terminal
//! scrollback above the block is preserved. A RAII guard enables raw mode
//! + hides the cursor and restores both on normal return, panic, and Ctrl-C.
//!
//! Workers send progress events over an `mpsc` channel; a single UI thread
//! owns the terminal and redraws on a fixed cadence.

use anyhow::Result;
use colored::Colorize;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::Print,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::collections::VecDeque;
use std::io::{stdout, Write};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ── Progress events ──────────────────────────────────────────────────────────

/// Progress event pushed by worker threads. `worker` is a 0-based slot id
/// chosen by the worker (`n % jobs`) — display only.
pub enum Ev {
    Started {
        worker: usize,
        path: String,
    },
    Done {
        worker: usize,
        path: String,
        findings: usize,
        ok: bool,
    },
    Finished,
}

// ── Terminal guard (RAII + panic-safe) ──────────────────────────────────────

struct TermGuard;

impl TermGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), cursor::Hide)?;
        restore_now();
        let prev = panic::take_hook();
        // ponytail: hook not uninstalled on Drop — the audit command is the
        // last thing main() runs. Upgrade: store an id and swap back in Drop.
        panic::set_hook(Box::new(move |info| {
            restore_now();
            prev(info);
        }));
        Ok(Self)
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        restore_now();
    }
}

fn restore_now() {
    let _ = execute!(stdout(), cursor::Show);
    let _ = disable_raw_mode();
}

// ── Live progress state ─────────────────────────────────────────────────────

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

struct State {
    workers: usize,
    total: usize,
    done: usize,
    findings: usize,
    slots: Vec<Option<String>>,
    recent: VecDeque<(String, usize, bool)>,
    tick: usize,
    start: Instant,
}

impl State {
    fn new(workers: usize, total: usize) -> Self {
        let workers = workers.max(1);
        Self {
            workers,
            total,
            done: 0,
            findings: 0,
            slots: vec![None; workers],
            recent: VecDeque::new(),
            tick: 0,
            start: Instant::now(),
        }
    }

    fn handle(&mut self, ev: Ev) {
        match ev {
            Ev::Started { worker, path } => {
                let w = self.slot(worker);
                self.slots[w] = Some(path);
            }
            Ev::Done {
                worker,
                path,
                findings,
                ok,
            } => {
                let w = self.slot(worker);
                if self.slots[w].as_deref() == Some(&path) {
                    self.slots[w] = None;
                }
                self.done += 1;
                self.findings += findings;
                self.recent.push_back((path, findings, ok));
                while self.recent.len() > self.recent_cap() {
                    self.recent.pop_front();
                }
            }
            Ev::Finished => {}
        }
    }

    fn slot(&self, worker: usize) -> usize {
        worker.min(self.workers - 1)
    }

    fn recent_cap(&self) -> usize {
        self.workers.clamp(1, 4)
    }

    /// Rendered height: header + worker slots + recent lines.
    fn height(&self) -> usize {
        1 + self.workers + self.recent_cap()
    }
}

// ── UI loop ──────────────────────────────────────────────────────────────────

/// The inline TUI is only worth it on a wide interactive terminal with no
/// machine-readable/quiet output requested. Everything else falls back to the
/// existing per-file stderr lines.
pub fn should_use_tui(json: bool, quiet: bool) -> bool {
    if json || quiet || !crate::output::is_tty() {
        return false;
    }
    matches!(crossterm::terminal::size(), Ok((cols, _)) if cols >= 60)
}

/// Run the inline progress loop until `Finished` / channel close.
/// Restores the terminal on return. `cancel` is set on Ctrl-C.
pub fn run_live(
    rx: &Receiver<Ev>,
    jobs: usize,
    total: usize,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let _guard = TermGuard::new()?;
    let mut st = State::new(jobs, total);
    let mut lines = 0usize;
    let mut last_draw = Instant::now() - Duration::from_secs(1);

    loop {
        let mut finished = false;
        loop {
            match rx.try_recv() {
                Ok(Ev::Finished) => {
                    finished = true;
                    break;
                }
                Ok(ev) => st.handle(ev),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true; // sender gone without Finished
                    break;
                }
            }
        }

        if event::poll(Duration::ZERO)? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press
                    && k.modifiers.contains(KeyModifiers::CONTROL)
                    && k.code == KeyCode::Char('c')
                {
                    cancel.store(true, Ordering::SeqCst);
                    clear(&mut lines);
                    return Ok(());
                }
            }
        }

        if finished {
            break;
        }
        // ponytail: drain + redraw on a 40 ms tick with spinner every 80 ms;
        // upgrade to crossterm event::poll with timeout + dirty-flag diffing
        // when this becomes latency-sensitive.
        if last_draw.elapsed() >= Duration::from_millis(80) {
            draw(&mut st, &mut lines)?;
            last_draw = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(40));
    }

    // Blank the region; the audit report then prints over it.
    clear(&mut lines);
    Ok(())
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn elapsed(start: Instant) -> String {
    let s = start.elapsed().as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn gauge(pct: usize) -> String {
    const N: usize = 10;
    let filled = (pct * N / 100).min(N);
    "█".repeat(filled) + &"░".repeat(N - filled)
}

fn trunc(p: &str) -> String {
    let n = p.chars().count();
    if n <= 44 {
        return p.to_string();
    }
    let tail: String = p.chars().skip(n - 41).collect();
    format!("…{tail}")
}

fn header_line(st: &State) -> String {
    let pct = st
        .done
        .saturating_mul(100)
        .checked_div(st.total)
        .unwrap_or(100);
    format!(
        "  {}  auditing {}/{}  {:>3}%  {}  {} findings  {}",
        SPINNER[st.tick % SPINNER.len()].cyan(),
        st.done,
        st.total,
        pct,
        gauge(pct).green(),
        st.findings.to_string().yellow(),
        elapsed(st.start).dimmed(),
    )
}

fn draw(st: &mut State, lines: &mut usize) -> Result<()> {
    let h = st.height();
    let mut out = stdout();
    // Move back to the top of the block. MoveToPreviousLine(0) means "1 up" in
    // ECMA-48, so only emit it when we really need to climb.
    move_to_block_top(&mut out, *lines)?;
    st.tick = st.tick.wrapping_add(1);

    let mut body = Vec::with_capacity(h);
    body.push(header_line(st));
    for (i, slot) in st.slots.iter().enumerate() {
        body.push(match slot {
            Some(p) => format!("  w{:<2} {} {}", i, "↳".dimmed(), trunc(p).dimmed()),
            None => format!("  w{:<2} {}", i, "idle".dimmed()),
        });
    }
    for (i, (p, f, ok)) in st.recent.iter().enumerate() {
        let mark = if *ok { "✓".green() } else { "✗".red() };
        let count = if *f == 0 {
            "clean".dimmed().to_string()
        } else {
            format!("{f} findings").dimmed().to_string()
        };
        body.push(format!(
            "  {} {} {} {}",
            mark,
            format!("[{i}]").dimmed(),
            trunc(p),
            count
        ));
    }
    while body.len() < h {
        body.push(String::new());
    }

    for (i, l) in body.iter().enumerate() {
        execute!(out, Clear(ClearType::CurrentLine), Print(l))?;
        if i + 1 < h {
            execute!(out, Print("\r\n"))?;
        }
    }
    out.flush()?;
    *lines = h;
    Ok(())
}

fn clear(lines: &mut usize) {
    if *lines == 0 {
        return;
    }
    let mut out = stdout();
    let _ = move_to_block_top(&mut out, *lines);
    for i in 0..*lines {
        let _ = execute!(out, Clear(ClearType::CurrentLine));
        if i + 1 < *lines {
            let _ = execute!(out, Print("\r\n"));
        }
    }
    // Leave the cursor at the top of the block so the report overwrites cleanly.
    let _ = move_to_block_top(&mut out, *lines);
    let _ = out.flush();
    *lines = 0;
}

/// Position the cursor at the start of the block (top line, column 0).
fn move_to_block_top(out: &mut std::io::Stdout, lines: usize) -> std::io::Result<()> {
    if lines > 1 {
        execute!(out, cursor::MoveToPreviousLine(lines as u16 - 1))?;
    }
    execute!(out, cursor::MoveToColumn(0))?;
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauge_fills_proportionally() {
        assert_eq!(gauge(0), "░░░░░░░░░░");
        assert_eq!(gauge(50), "█████░░░░░");
        assert_eq!(gauge(100), "██████████");
        assert_eq!(gauge(150), "██████████"); // clamped
    }

    #[test]
    fn trunc_keeps_short_paths_and_shortens_long_ones() {
        assert_eq!(trunc("src/main.rs"), "src/main.rs");
        let long = "a".repeat(60);
        let tr = trunc(&long);
        assert!(tr.starts_with('…'));
        assert_eq!(tr.chars().count(), 42); // '…' + 41 chars
    }

    #[test]
    fn state_tracks_slots_findings_and_recent_cap() {
        let mut st = State::new(2, 5);
        assert_eq!(st.slots.len(), 2);
        assert_eq!(st.height(), 1 + 2 + 2); // header + 2 worker slots + 2 recent

        st.handle(Ev::Started {
            worker: 0,
            path: "src/a.rs".into(),
        });
        st.handle(Ev::Started {
            worker: 1,
            path: "src/b.rs".into(),
        });
        assert_eq!(st.slots[0].as_deref(), Some("src/a.rs"));
        assert_eq!(st.slots[1].as_deref(), Some("src/b.rs"));

        st.handle(Ev::Done {
            worker: 0,
            path: "src/a.rs".into(),
            findings: 3,
            ok: true,
        });
        assert_eq!(st.slots[0], None);
        assert_eq!(st.done, 1);
        assert_eq!(st.findings, 3);
        assert_eq!(st.recent.len(), 1);

        // Slot 0 keeps its contents when a mismatched path completes.
        st.handle(Ev::Started {
            worker: 0,
            path: "src/c.rs".into(),
        });
        st.handle(Ev::Done {
            worker: 0,
            path: "src/a.rs".into(),
            findings: 1,
            ok: true,
        });
        assert_eq!(st.slots[0].as_deref(), Some("src/c.rs"));

        // Recent deque is capped at min(workers, 4) = 2.
        for i in 0..5 {
            st.handle(Ev::Done {
                worker: 1,
                path: format!("file_{i}.rs"),
                findings: 0,
                ok: true,
            });
        }
        assert_eq!(st.recent.len(), 2);
        assert_eq!(st.done, 7);
    }

    #[test]
    fn worker_slot_is_clamped_to_slot_count() {
        let mut st = State::new(2, 1);
        st.handle(Ev::Started {
            worker: 99,
            path: "x.rs".into(),
        });
        assert_eq!(st.slots[1].as_deref(), Some("x.rs"));
    }

    #[test]
    fn should_use_tui_refuses_json_and_quiet() {
        assert!(!should_use_tui(true, false));
        assert!(!should_use_tui(false, true));
        assert!(!should_use_tui(true, true));
    }

    #[test]
    fn header_shows_progress_counts() {
        let mut st = State::new(1, 4);
        st.handle(Ev::Done {
            worker: 0,
            path: "a.rs".into(),
            findings: 2,
            ok: true,
        });
        let h = header_line(&st);
        assert!(h.contains("auditing 1/4"), "header was {h:?}");
        assert!(h.contains("findings"), "header was {h:?}");
        assert!(h.contains(" 25%"), "header was {h:?}");
    }
}
