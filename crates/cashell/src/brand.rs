//! CAShell brandmark: the double shell chevron `>>` with a cursor `_`
//! (see `Product/assets/icons/cashell/icon-transparent.svg`), rasterised to a
//! small pixel grid and drawn with Unicode half-blocks, so the CLI banner and
//! the Cockpit share one logo with no image assets.

/// RGB colour of one logo pixel; `None` = transparent.
pub type Px = Option<(u8, u8, u8)>;

pub const W: usize = 30;
pub const H: usize = 24;

/// Brand greens from the SVG gradient (`#4ADE80` → `#22C55E`).
const GREEN_TOP: (u8, u8, u8) = (0x4A, 0xDE, 0x80);
const GREEN_BOT: (u8, u8, u8) = (0x22, 0xC5, 0x5E);
const WHITE: (u8, u8, u8) = (0xFF, 0xFF, 0xFF);

fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + t * abx, a.1 + t * aby);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}

/// The logo as a `W`×`H` pixel grid (row-major).
pub fn pixels() -> Vec<Vec<Px>> {
    pixels_sized(W, H)
}

/// Logo rasterised to an arbitrary `w`×`h` pixel grid (keep `w:h` ≈ 5:4).
pub fn pixels_sized(w: usize, h: usize) -> Vec<Vec<Px>> {
    // Geometry in the SVG's 512-unit space, cropped to x 80..430, y 118..394.
    let (ox, oy) = (80.0_f32, 118.0_f32);
    let (sx, sy) = (350.0 / w.max(1) as f32, 276.0 / h.max(1) as f32);
    let half_stroke = 16.0_f32;
    let chevrons = [
        [(115.0, 152.0), (220.0, 256.0), (115.0, 360.0)],
        [(215.0, 152.0), (320.0, 256.0), (215.0, 360.0)],
    ];
    let cursor = [(338.0, 360.0), (408.0, 360.0)];
    let mut grid = vec![vec![None; w]; h];
    for (y, row) in grid.iter_mut().enumerate() {
        for (x, px) in row.iter_mut().enumerate() {
            let p = (ox + (x as f32 + 0.5) * sx, oy + (y as f32 + 0.5) * sy);
            if dist_to_segment(p, cursor[0], cursor[1]) <= half_stroke {
                *px = Some(WHITE);
                continue;
            }
            let on_chevron = chevrons.iter().any(|c| {
                dist_to_segment(p, c[0], c[1]) <= half_stroke
                    || dist_to_segment(p, c[1], c[2]) <= half_stroke
            });
            if on_chevron {
                let t = ((p.1 - 152.0) / 208.0).clamp(0.0, 1.0);
                *px = Some((
                    lerp(GREEN_TOP.0, GREEN_BOT.0, t),
                    lerp(GREEN_TOP.1, GREEN_BOT.1, t),
                    lerp(GREEN_TOP.2, GREEN_BOT.2, t),
                ));
            }
        }
    }
    grid
}

/// One terminal cell of the logo: position, glyph, foreground, optional background.
pub type Cell = (u16, u16, char, (u8, u8, u8), Option<(u8, u8, u8)>);

/// Logo as terminal cells (half-blocks) for a `w`×`h` pixel grid, i.e.
/// `w` columns × `h/2` rows. Transparent cells are omitted.
pub fn cells(w: usize, h: usize) -> Vec<Cell> {
    let g = pixels_sized(w, h);
    let mut out = Vec::new();
    for y in (0..h).step_by(2) {
        for x in 0..w {
            let top = g.get(y).and_then(|r| r.get(x)).copied().flatten();
            let bot = g.get(y + 1).and_then(|r| r.get(x)).copied().flatten();
            let (ch, fgc, bgc) = match (top, bot) {
                (None, None) => continue,
                (Some(t), None) => ('▀', t, None),
                (None, Some(b)) => ('▄', b, None),
                (Some(t), Some(b)) if t == b => ('█', t, None),
                (Some(t), Some(b)) => ('▀', t, Some(b)),
            };
            out.push((x as u16, (y / 2) as u16, ch, fgc, bgc));
        }
    }
    out
}

/// True when the terminal advertises 24-bit colour.
pub fn truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| v.contains("truecolor") || v.contains("24bit"))
        .unwrap_or(false)
}

fn fg(c: (u8, u8, u8), tc: bool) -> String {
    if tc {
        format!("\x1b[38;2;{};{};{}m", c.0, c.1, c.2)
    } else if c == WHITE {
        "\x1b[97m".to_string()
    } else {
        "\x1b[92m".to_string()
    }
}

fn bg(c: (u8, u8, u8), tc: bool) -> String {
    if tc {
        format!("\x1b[48;2;{};{};{}m", c.0, c.1, c.2)
    } else if c == WHITE {
        "\x1b[107m".to_string()
    } else {
        "\x1b[102m".to_string()
    }
}

/// The logo as ANSI-coloured terminal lines (two pixel rows per text row).
pub fn ansi_lines() -> Vec<String> {
    let tc = truecolor();
    let g = pixels();
    let mut lines = Vec::new();
    for y in (0..H).step_by(2) {
        let mut s = String::new();
        for x in 0..W {
            let top = g.get(y).and_then(|r| r.get(x)).copied().flatten();
            let bot = g.get(y + 1).and_then(|r| r.get(x)).copied().flatten();
            match (top, bot) {
                (None, None) => s.push(' '),
                (Some(t), None) => s.push_str(&format!("{}▀\x1b[0m", fg(t, tc))),
                (None, Some(b)) => s.push_str(&format!("{}▄\x1b[0m", fg(b, tc))),
                (Some(t), Some(b)) if t == b => s.push_str(&format!("{}█\x1b[0m", fg(t, tc))),
                (Some(t), Some(b)) => {
                    s.push_str(&format!("{}{}▀\x1b[0m", fg(t, tc), bg(b, tc)))
                }
            }
        }
        lines.push(s);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixels_have_chevrons_and_white_cursor() {
        let g = pixels();
        assert_eq!(g.len(), H);
        assert!(g.iter().all(|r| r.len() == W));
        let flat: Vec<_> = g.iter().flatten().flatten().collect();
        assert!(flat.iter().any(|c| **c == WHITE), "cursor missing");
        assert!(flat.iter().any(|c| c.1 > c.0 && c.1 > c.2), "green missing");
        // Some transparent area remains around the glyph.
        assert!(g.iter().flatten().any(|p| p.is_none()));
    }

    #[test]
    fn test_cells_fit_requested_box() {
        let c = cells(16, 12);
        assert!(!c.is_empty());
        assert!(c.iter().all(|(x, y, ..)| *x < 16 && *y < 6));
    }

    #[test]
    fn test_ansi_lines_row_count() {
        assert_eq!(ansi_lines().len(), H / 2);
    }
}
