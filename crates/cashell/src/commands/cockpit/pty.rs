//! Real shell for the Terminal tab: a PTY running the user's shell (or `ssh`
//! to a fleet server) whose output is parsed by a VT100 emulator and drawn
//! straight into the ratatui buffer, so prompts, colours, Tab completion,
//! `htop`, `vim`, Ctrl+C and `cd` behave like any terminal app.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    parser: Arc<Mutex<vt100::Parser>>,
    size: (u16, u16),
    /// What runs in it, for the title ("Local · bash", "ssh web1").
    pub label: String,
    /// Lines scrolled back into history (Shift+PgUp/PgDn).
    pub scrollback: usize,
}

impl PtySession {
    /// Spawn `cmd` in a new PTY of `rows`×`cols`.
    pub fn spawn(
        mut cmd: CommandBuilder,
        rows: u16,
        cols: u16,
        label: String,
    ) -> anyhow::Result<Self> {
        let rows = rows.max(2);
        let cols = cols.max(10);
        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 2000)));
        let sink = Arc::clone(&parser);
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let (Ok(mut p), Some(chunk)) = (sink.lock(), buf.get(..n)) {
                            p.process(chunk);
                        }
                    }
                }
            }
        });
        Ok(Self {
            master: pair.master,
            writer,
            child,
            parser,
            size: (rows, cols),
            label,
            scrollback: 0,
        })
    }

    /// Local login shell (`$SHELL`, else bash) in the home directory.
    pub fn local(rows: u16, cols: u16) -> anyhow::Result<Self> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut cmd = CommandBuilder::new(&shell);
        if let Some(home) = dirs::home_dir() {
            cmd.cwd(home);
        }
        let name = shell.rsplit('/').next().unwrap_or("shell").to_string();
        Self::spawn(cmd, rows, cols, format!("Local · {name}"))
    }

    /// Interactive `ssh` to a registered fleet server, using the same
    /// options as the rest of CAShell (strict host keys, key/password auth).
    pub fn remote(
        name: &str,
        node: &crate::config::NodeConfig,
        rows: u16,
        cols: u16,
    ) -> anyhow::Result<Self> {
        let std_cmd = crate::fleet_ssh::ssh_command(name, node, None);
        let mut cmd = CommandBuilder::new(std_cmd.get_program());
        cmd.arg("-t");
        for a in std_cmd.get_args() {
            cmd.arg(a);
        }
        for (k, v) in std_cmd.get_envs() {
            if let Some(v) = v {
                cmd.env(k, v);
            }
        }
        Self::spawn(cmd, rows, cols, format!("ssh {name}"))
    }

    /// Keep the PTY the same size as the panel.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(10));
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut p) = self.parser.lock() {
            p.set_size(rows, cols);
        }
    }

    pub fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.scrollback = 0;
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    pub fn scroll(&mut self, up: bool, lines: usize) {
        self.scrollback = if up {
            self.scrollback.saturating_add(lines)
        } else {
            self.scrollback.saturating_sub(lines)
        };
    }

    /// Draw the emulator screen into `area` of `buf`; returns the cursor
    /// position (absolute) when it is visible.
    pub fn render(&mut self, buf: &mut Buffer, area: Rect) -> Option<(u16, u16)> {
        let Ok(mut parser) = self.parser.lock() else {
            return None;
        };
        parser.set_scrollback(self.scrollback);
        // vt100 clamps to the history it has; read back the real offset.
        self.scrollback = parser.screen().scrollback();
        let screen = parser.screen();
        for row in 0..area.height {
            for col in 0..area.width {
                let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) else {
                    continue;
                };
                let Some(sc) = screen.cell(row, col) else {
                    cell.set_char(' ');
                    continue;
                };
                let text = sc.contents();
                if sc.is_wide_continuation() {
                    cell.set_symbol("");
                } else if text.is_empty() {
                    cell.set_char(' ');
                } else {
                    cell.set_symbol(&text);
                }
                let (mut fg, mut bg) = (map_color(sc.fgcolor()), map_color(sc.bgcolor()));
                if sc.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                    if fg == Color::Reset {
                        fg = Color::Black;
                    }
                    if bg == Color::Reset {
                        bg = Color::White;
                    }
                }
                cell.set_fg(fg);
                cell.set_bg(bg);
                let mut m = Modifier::empty();
                if sc.bold() {
                    m |= Modifier::BOLD;
                }
                if sc.italic() {
                    m |= Modifier::ITALIC;
                }
                if sc.underline() {
                    m |= Modifier::UNDERLINED;
                }
                cell.modifier = m;
            }
        }
        if self.scrollback > 0 || screen.hide_cursor() {
            return None;
        }
        let (cr, cc) = screen.cursor_position();
        (cr < area.height && cc < area.width).then_some((area.x + cc, area.y + cr))
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn map_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Bytes a terminal sends for `key` (xterm conventions).
pub fn key_bytes(key: &KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut out: Vec<u8> = match key.code {
        KeyCode::Char(c) if ctrl => {
            let l = c.to_ascii_lowercase();
            match l {
                'a'..='z' => vec![(l as u8) - b'a' + 1],
                '@' | ' ' => vec![0],
                '[' => vec![0x1b],
                '\\' => vec![0x1c],
                ']' => vec![0x1d],
                '^' => vec![0x1e],
                '_' => vec![0x1f],
                _ => c.to_string().into_bytes(),
            }
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    if alt && !out.is_empty() {
        out.insert(0, 0x1b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_bytes_follow_xterm() {
        let k = |c, m| KeyEvent::new(c, m);
        assert_eq!(
            key_bytes(&k(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            vec![3]
        );
        assert_eq!(key_bytes(&k(KeyCode::Enter, KeyModifiers::NONE)), b"\r");
        assert_eq!(key_bytes(&k(KeyCode::Up, KeyModifiers::NONE)), b"\x1b[A");
        assert_eq!(
            key_bytes(&k(KeyCode::Char('b'), KeyModifiers::ALT)),
            b"\x1bb"
        );
        assert_eq!(key_bytes(&k(KeyCode::Char('!'), KeyModifiers::SHIFT)), b"!");
    }

    #[cfg(unix)]
    #[test]
    fn test_pty_runs_a_command_and_renders_its_output() -> anyhow::Result<()> {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "printf 'hello-pty'"]);
        let mut s = PtySession::spawn(cmd, 5, 30, "test".into())?;
        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        let mut seen = false;
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(40));
            s.render(&mut buf, area);
            let row: String = (0..30)
                .filter_map(|x| buf.cell((x, 0)).map(|c| c.symbol().to_string()))
                .collect();
            if row.contains("hello-pty") {
                seen = true;
                break;
            }
        }
        assert!(seen, "PTY output must reach the screen");
        Ok(())
    }
}
