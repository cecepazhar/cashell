//! About tab: what CAShell is, who builds it, Free vs Pro, direction, support.

use super::settings::{benefit_lines, BUY_URL, FOUNDER_URL, SPONSOR_URL};
use super::{cockpit_theme, CockpitApp, BRAND_GREEN};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Tools CAShell aims to cover from one terminal (direction, not a claim of parity).
const DIRECTION: [(&str, &str); 5] = [
    ("cPanel", "Server: firewall, database, domain & SSL"),
    (
        "Portainer / Dokploy",
        "Container & Deploy di atas Docker/Podman",
    ),
    ("Uptime Kuma", "Uptime: monitor, insiden, status page"),
    (
        "Umami / Google Analytics",
        "Analytics self-hosted milik Anda",
    ),
    ("btop / cron / logs", "Dashboard, CRON, Log & AI BYOK"),
];

pub fn draw_about_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let is_pro = app.is_pro();
    let cols = if area.width >= 100 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area)
    };
    let (Some(&left), Some(&right)) = (cols.first(), cols.get(1)) else {
        return;
    };

    let text = Style::default().fg(palette.text);
    let muted = Style::default().fg(palette.text_muted);
    let bold = |c: Color| Style::default().fg(c).add_modifier(Modifier::BOLD);
    let link = Style::default()
        .fg(palette.accent)
        .add_modifier(Modifier::UNDERLINED);

    // ── Left: identity + direction ──────────────────────────────────────────
    let mut l = vec![
        Line::from(vec![
            Span::styled("❯❯", bold(BRAND_GREEN)),
            Span::styled("_", bold(Color::White)),
            Span::styled(
                format!(" CAShell v{}", env!("CARGO_PKG_VERSION")),
                bold(palette.accent),
            ),
            Span::styled(
                if is_pro { "  [PRO]" } else { "  [FREE]" },
                bold(if is_pro {
                    Color::Yellow
                } else {
                    palette.success
                }),
            ),
        ]),
        Line::from(Span::styled(
            "Zero-knowledge, local-first server cockpit & CLI.",
            text,
        )),
        Line::from(Span::styled(
            "Kelola server, container, uptime, analytics dan deploy dari satu terminal. \
             Data tetap di mesin Anda.",
            muted,
        )),
        Line::from(""),
        Line::from(Span::styled("DIBUAT OLEH", bold(palette.accent))),
        Line::from(vec![
            Span::styled("  Cecep Azhar", bold(palette.text)),
            Span::styled(" · Fathforce", text),
        ]),
        Line::from(vec![
            Span::styled("  ", muted),
            Span::styled("fathforce.com", link),
            Span::styled(" · ", muted),
            Span::styled("cecepazhar.com", link),
            Span::styled(" · GitHub ", muted),
            Span::styled("cecepazhar", link),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "ARAH PENGEMBANGAN — satu alat, banyak peran",
            bold(palette.accent),
        )),
    ];
    for (tool, role) in DIRECTION {
        l.push(Line::from(vec![
            Span::styled("  ▸ ", Style::default().fg(BRAND_GREEN)),
            Span::styled(format!("{tool:<25}"), bold(palette.text)),
            Span::styled(role, muted),
        ]));
    }
    l.push(Line::from(""));
    l.push(Line::from(Span::styled(
        "Sebagian peran sudah bisa dipakai, sebagian masih dibangun. \
         Nama produk lain milik pemiliknya masing-masing.",
        muted,
    )));
    f.render_widget(
        Paragraph::new(l).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" ■ About CAShell ")
                .border_style(Style::default().fg(palette.accent)),
        ),
        left,
    );

    // ── Right: Free vs Pro + support ───────────────────────────────────────
    let mut r = benefit_lines(&palette);
    r.remove(0); // leading blank line
    r.extend([
        Line::from(""),
        Line::from(vec![
            Span::styled("  Beli Pro      ", muted),
            Span::styled(BUY_URL, link),
        ]),
        Line::from(vec![
            Span::styled("  Founder (all) ", muted),
            Span::styled(FOUNDER_URL, link),
        ]),
        Line::from(Span::styled(
            "  Klik [ FREE · Get Pro ▸ ] di header untuk memasukkan kunci.",
            muted,
        )),
        Line::from(""),
        Line::from(Span::styled("SUPPORT & DONASI", bold(palette.accent))),
        Line::from(vec![
            Span::styled("  ♥ Sponsor  ", Style::default().fg(Color::LightRed)),
            Span::styled(SPONSOR_URL, link),
        ]),
        Line::from(vec![
            Span::styled("  ✉ Email    ", muted),
            Span::styled("hi@cecepazhar.com", link),
        ]),
        Line::from(Span::styled(
            "  Setiap dukungan membantu CAShell tetap independen dan gratis untuk fitur lokal.",
            muted,
        )),
    ]);
    f.render_widget(
        Paragraph::new(r).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" ★ Free vs Pro · Support ")
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        right,
    );
}
