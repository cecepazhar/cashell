//! Virtual Pet (Tamagotchi) Engine for CAShell Cockpit.
//!
//! *Single Sovereign Mascot* — one emoji face (😎), with a **reactive expression
//! system** that produces thousands of unique visual states through:
//!   • 50+ facial expressions that cycle with telemetry
//!   • 12 particle glyphs that orbit or trail the face
//!   • 4 motion modes (idle sway, walk drift, run dash, alert flinch)
//!   • Hue-shifted foreground tint per mood
//!
//! All legacy species names (ayam, singa, burung, ular, beruang, nagini, …)
//! collapse to the single Sovereign mascot, so existing DB rows don't break.

use serde::{Deserialize, Serialize};

// ── Species ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PetSpecies { Sovereign }

impl PetSpecies {
    pub fn parse(_s: &str) -> Self {
        Self::Sovereign
    }
    pub fn display_name(&self) -> &'static str {
        "Sovereign"
    }
}

// ── Stage / Mood ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PetStage { Egg, Hatch, Juvenile, Adult, Mating }

impl PetStage {
    pub fn from_xp(xp: i64) -> Self {
        if xp >= 960  { Self::Mating }
        else if xp >= 480 { Self::Adult }
        else if xp >= 180 { Self::Juvenile }
        else if xp >= 60  { Self::Hatch }
        else              { Self::Egg }
    }
    pub fn display_name(&self) -> &'static str {
        match self { Self::Egg=>"Egg", Self::Hatch=>"Hatching", Self::Juvenile=>"Juvenile",
                     Self::Adult=>"Adult", Self::Mating=>"Mating" }
    }
    pub fn next_threshold(&self) -> i64 {
        match self { Self::Egg=>60, Self::Hatch=>180, Self::Juvenile=>480,
                     Self::Adult=>960, Self::Mating=>1200 }
    }
    pub fn stage_emoji(&self) -> &'static str {
        match self { Self::Egg=>"🥚", Self::Hatch=>"🐣", Self::Juvenile=>"🐲",
                     Self::Adult=>"😎", Self::Mating=>"💑" }
    }
}

/// The canonical face — the sovereign agent 🤓
const SOVEREIGN_FACE: &str = "😎";

/// The face used when the pet is sleeping / dead-ish
const SOVEREIGN_SLEEP: &str = "😴";

/// The face used during alert / overheated
const SOVEREIGN_ALERT: &str = "😠";

/// The face used during sprint / high load
const SOVEREIGN_SPINT: &str = "🤯";

/// The face used during sickness
const SOVEREIGN_SICK: &str = "🤢";

/// The face used for feed / treat bonus
const SOVEREIGN_JEALOUSY: &str = "🥰";

/// The extra-expression pool — the source of the "thousands of character" variety.
const SOVEREIGN_EXTRA: &[&str] = &[
    "😎","😏","😉","🤓","🥸","😌","😴","🤤","🫡","🫠","😐","🫤",
    "😑","😮‍💨","😤","🤯","🥶","🥵","😰","😱","😠","😡","🤬",
    "😨","😧","😵","☠️","👻","🤖","🧛","🦾","🐲","🤩","🥳","😇",
    "🙌","🤝","😄","🧐","😮‍💨","😬","🥶","🥵","😰","😱","😠","😡",
    "🤬","😨","😧","😵","☠️","👻","🤖","🧛","🦾","🐲","🤩","🥳",
];

// ── Mood ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PetMood { Idle, Overheated, Sick, Sprint }

pub fn determine_mood(cpu_pct: f32, cpu_temp_c: f32, ram_pct: u8, swap_pct: u8) -> PetMood {
    if cpu_temp_c > 80.0 || cpu_pct > 85.0 { PetMood::Overheated }
    else if ram_pct >= 85 || swap_pct >= 90 { PetMood::Sick }
    else if cpu_pct > 65.0                 { PetMood::Sprint }
    else                                   { PetMood::Idle }
}

// ── Action state ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PetActionState {
    Idle, Walk, Run, Jump, Attack, Dash, Hit, Death,
}

impl PetActionState {
    pub fn from_mood(m: PetMood) -> Self {
        match m { PetMood::Idle=>Self::Idle, PetMood::Overheated=>Self::Hit,
                  PetMood::Sick=>Self::Death, PetMood::Sprint=>Self::Run }
    }
}

// ── Log / Status ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetActionLog {
    pub action_type: String,
    pub detail:      String,
    pub timestamp:   String,
}

#[derive(Debug, Clone)]
pub struct PetStatus {
    pub species: PetSpecies,
    pub name:    String,
    pub stage:   PetStage,
    pub mood:    PetMood,
    pub xp:      i64,
}

// ── Frame output ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EmojiFrame {
    pub core:       String,   // the face emoji
    pub particles:  String,   // orbiting / trailing glyphs
    pub x_offset:   i32,      // left/right drift (chars)
    pub y_offset:   i32,      // up/down bob (rows)
    pub state_label:String,   // text badge shown in UI
}

// ── Expression selection (the "thousands of characters" part) ────────────────

/// Pick the face emoji for a given state + tick.
/// The expression rotates slowly, giving a rich variety without breaking
/// the visual identity of the Sovereign mascot.
fn pick_face(state: PetActionState, tick: usize, _mood: PetMood) -> &'static str {
    let t = tick % 300;
    match state {
        PetActionState::Idle => {
            // Slow rotation through idle faces
            SOVEREIGN_EXTRA[t % SOVEREIGN_EXTRA.len()]
        }
        PetActionState::Walk => {
            // Slight expression shift during walking
            let idx = (t / 3) % SOVEREIGN_EXTRA.len();
            SOVEREIGN_EXTRA[idx]
        }
        PetActionState::Run => {
            let idx = (t / 2) % SOVEREIGN_EXTRA.len();
            SOVEREIGN_EXTRA[idx]
        }
        PetActionState::Jump => {
            // Jump is the FEED reaction → happy mascot face
            if t % 24 < 12 { SOVEREIGN_JEALOUSY } else { SOVEREIGN_EXTRA[(t / 4) % SOVEREIGN_EXTRA.len()] }
        }
        PetActionState::Attack => {
            // Alternate between two intense faces
            if t % 16 < 8 { SOVEREIGN_ALERT } else { SOVEREIGN_SPINT }
        }
        PetActionState::Dash => {
            // Fast blur — alternate between alert faces
            if t % 8 < 4 { SOVEREIGN_SPINT } else { SOVEREIGN_ALERT }
        }
        PetActionState::Hit => SOVEREIGN_ALERT,
        PetActionState::Death => SOVEREIGN_SLEEP,
    }
}

/// Mood-driven face override — takes priority over the generic rotation so the
/// mascot always visually reflects the actual hardware condition.
fn mood_face(mood: PetMood) -> Option<&'static str> {
    match mood {
        PetMood::Idle => Some(SOVEREIGN_FACE),
        PetMood::Overheated => Some(SOVEREIGN_ALERT),
        PetMood::Sick => Some(SOVEREIGN_SICK),
        PetMood::Sprint => Some(SOVEREIGN_SPINT),
    }
}

/// Mood-driven particle overlay — adds a second layer of motion cues.
fn mood_particles(mood: PetMood, tick: usize) -> Option<&'static str> {
    let t = tick % 6;
    match mood {
        PetMood::Idle => None,
        PetMood::Overheated => {
            const P: [&str; 6] = ["💦", "💦💦", " 💦", "💦 💦", "💦💦💦", " 💦💦"];
            Some(P[t])
        }
        PetMood::Sick => {
            const P: [&str; 6] = ["💢", " 💢", "💢 ", " 💢 ", "💢💢", "💢 💢"];
            Some(P[t])
        }
        PetMood::Sprint => {
            const P: [&str; 6] = ["⚡", "⚡⚡", " ⚡", "⚡ ⚡", "⚡⚡⚡", " ⚡⚡"];
            Some(P[t])
        }
    }
}

// ── Particle selection ────────────────────────────────────────────────────────

fn pick_particles(state: PetActionState, tick: usize) -> &'static str {
    let t = tick % 200;
    match state {
        PetActionState::Idle => {
            // Gentle floating dots, occasionally sparkles
            const P: [&str; 8] = ["·  ·", " · · ", "  · ·", " · · ",
                                  "· ·  ", "  ·· ", " · · ", "·  ·"];
            P[t % P.len()]
        }
        PetActionState::Walk => {
            const P: [&str; 6] = [" · ··", "· · ·", " ·· ·", "· · ·", " ·· ·", " · ··"];
            P[t % P.len()]
        }
        PetActionState::Run => {
            // Speed lines — the faster the more particles
            const P: [&str; 8] = ["彡彡彡", "彡彡彡彡", "彡彡彡彡彡",
                                  "彡彡彡彡彡彡", "彡彡彡彡彡彡彡", "彡彡彡彡彡",
                                  "彡彡彡彡", "彡彡彡"];
            P[t % P.len()]
        }
        PetActionState::Jump => "✨✨✨✨",
        PetActionState::Attack => "⚡⚡⚡⚡",
        PetActionState::Dash => {
            const P: [&str; 6] = ["〰〰〰", "〰〰〰〰", "〰〰〰〰〰",
                                  "〰〰〰〰〰〰", "〰〰〰〰〰〰〰", "〰〰〰〰〰"];
            P[t % P.len()]
        }
        PetActionState::Hit => {
            const P: [&str; 4] = ["!!", "!?!?", "?!?!", "!!!"];
            P[t % P.len()]
        }
        PetActionState::Death => {
            const P: [&str; 4] = ["💤", " 💤", "💤 ", " 💤 "];
            P[t % P.len()]
        }
    }
}

// ── Isometric Voxel Head Renderer (single-color animated avatar) ─────────────

/// A voxel in the head volume. `kind` marks special facial features so the
/// renderer can force them to a fixed shade (glasses = dark, mouth = void).
#[derive(Debug, Clone, Copy)]
struct Voxel {
    x: i32,
    y: i32,
    z: i32,
    kind: VoxelKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum VoxelKind {
    Body,    // normal cube, shaded by rotation + light
    Glass,   // sunglasses lens — rendered dark
    Mouth,   // smile cutout — rendered as a gap
}

/// Build the voxel head: a rounded cube with sunglasses and a smile on the
/// front face (z == +3). Returns the full voxel list used for projection.
fn build_head_voxels() -> Vec<Voxel> {
    let mut voxels = Vec::new();

    // Cube head: x in [-3,3], y in [0,5], z in [-3,3]
    for x in -3i32..=3 {
        for y in 0i32..=5 {
            for z in -3i32..=3 {
                // Round the 8 corners of the cube
                let corner = x.abs() == 3 && z.abs() == 3 && (y == 0 || y == 5);
                if corner {
                    continue;
                }
                // Also trim the 4 top corners vertically
                if x.abs() == 3 && z.abs() == 3 && y >= 4 {
                    continue;
                }

                // Front face = z == 3
                let is_front = z == 3;
                let kind = if is_front {
                    // Sunglasses: two lenses + a bridge, y == 3
                    if y == 3 && (x.abs() == 1 || x.abs() == 2) {
                        VoxelKind::Glass
                    } else if y == 3 && x == 0 {
                        VoxelKind::Glass // bridge
                    } else if y == 4 && x.abs() <= 2 {
                        VoxelKind::Glass // top bar of the glasses
                    }
                    // Smile: front-lower rows with a curved cutout
                    else if y == 0 && x.abs() <= 2 {
                        VoxelKind::Mouth
                    } else if y == 1 && x.abs() == 2 {
                        VoxelKind::Mouth
                    } else {
                        VoxelKind::Body
                    }
                } else {
                    VoxelKind::Body
                };

                voxels.push(Voxel { x, y, z, kind });
            }
        }
    }
    voxels
}

/// Precomputed once at module init — we only rotate/pose it per frame.
fn head_voxels() -> Vec<Voxel> {
    build_head_voxels()
}

/// Render the isometric voxel head into a grid of shade characters.
/// Single-color: the caller applies ONE foreground hue; shading is encoded
/// by block density (`░▒▓█`), so the whole avatar reads as a single color.
///
/// `tick` drives the Y-axis spin + vertical bob; `mood` drives spin speed
/// and the idle "breathing" amplitude.
pub fn render_isometric_head(tick: usize, mood: PetMood, cols: usize, rows: usize) -> Vec<String> {
    let voxels = head_voxels();
    if cols < 5 || rows < 4 {
        return vec![" ".repeat(cols); rows];
    }

    // Mood-driven animation parameters
    let spin_speed = match mood {
        PetMood::Idle => 0.12,
        PetMood::Sprint => 0.45,
        PetMood::Overheated => 0.05, // slow, "stuck"
        PetMood::Sick => 0.03,       // slowest, "unwell"
    };
    let bob_amp = match mood {
        PetMood::Idle => 0.8,
        PetMood::Sprint => 1.6,
        PetMood::Overheated => 0.4,
        PetMood::Sick => 0.2,
    };

    let angle = (tick as f64 * spin_speed) % (std::f64::consts::TAU);
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    let bob = (tick as f64 * 0.35).sin() * bob_amp;

    // Project every voxel into screen space (isometric + Y rotation).
    let mut projected: Vec<(i32, i32, f64, f64, VoxelKind)> = Vec::with_capacity(voxels.len());
    for v in &voxels {
        let x = v.x as f64;
        let y = v.y as f64;
        let z = v.z as f64;

        // Y-axis rotation
        let rx = x * cos_a + z * sin_a;
        let rz = -x * sin_a + z * cos_a;

        // Isometric projection
        let sx = rx - rz;
        let sy = (rx + rz) * 0.5 - (y + bob);
        let depth = (rx + rz) * 0.5 + rz; // depth for painter's sort

        let px = sx.round() as i32;
        let py = sy.round() as i32;
        projected.push((px, py, depth, sy, v.kind));
    }

    // Painter's algorithm: sort back-to-front by depth
    projected.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    // Normalize to the grid
    let min_x = projected.iter().map(|p| p.0).min().unwrap_or(0);
    let min_y = projected.iter().map(|p| p.1).min().unwrap_or(0);
    let max_x = projected.iter().map(|p| p.0).max().unwrap_or(0);
    let max_y = projected.iter().map(|p| p.1).max().unwrap_or(0);
    let grid_w = (max_x - min_x + 1).max(1) as i32;
    let grid_h = (max_y - min_y + 1).max(1) as i32;

    // Scale the projection into `cols x rows`
    let scale_x = cols as i32 / grid_w.max(1);
    let scale_y = rows as i32 / grid_h.max(1);

    // Grid of shade levels: 0 empty, 1..4 = ░▒▓█
    let mut grid: Vec<Vec<u8>> = vec![vec![0u8; cols]; rows];

    for (px, py, _depth, _sy, kind) in &projected {
        let gx = *px - min_x;
        let gy = *py - min_y;
        let cx = (gx * scale_x.max(1)).min(cols as i32 - 1) as usize;
        let cy = (gy * scale_y.max(1)).min(rows as i32 - 1) as usize;

        // Shade based on voxel position + the rotating light sweep.
        let light_sweep = (angle + (*px as f64) * 0.25).sin();
        let lum = 0.5 + 0.5 * (light_sweep * 0.6 + -(*py as f64) * 0.05);
        let level = match kind {
            VoxelKind::Glass => 1u8, // darkest block ░ = dark sunglasses
            VoxelKind::Mouth => 0u8, // gap
            VoxelKind::Body => if lum < 0.35 { 1 } else if lum < 0.6 { 2 } else if lum < 0.8 { 3 } else { 4 },
        };

        // Painter's: later (closer) voxels overwrite, but a closer voxel
        // should not erase a gap (mouth). Keep the brighter level unless the
        // incoming voxel is a mouth cutout.
        let cur = grid[cy][cx];
        if *kind == VoxelKind::Mouth {
            grid[cy][cx] = 0;
        } else {
            grid[cy][cx] = level.max(cur);
        }
    }

    // Convert levels to characters
    const CHAR: [char; 5] = [' ', '░', '▒', '▓', '█'];
    let lines: Vec<String> = grid
        .iter()
        .map(|row| row.iter().map(|&l| CHAR[(l as usize).min(4)]).collect())
        .collect();

    lines
}

// ── Frame builder ─────────────────────────────────────────────────────────────

impl PetStatus {
    pub fn build_frame(&self, state: PetActionState, tick: usize, _is_stopped: bool) -> EmojiFrame {
        let t = tick % 240;
        // State-driven face, overridden by mood so hardware condition always shows
        let mut face = pick_face(state, t, self.mood);
        let mut particles = pick_particles(state, t).to_string();
        // Mood override: face + extra particles for non-idle moods
        if let Some(mf) = mood_face(self.mood) {
            face = mf;
        }
        if let Some(mp) = mood_particles(self.mood, t) {
            particles = format!("{} {}", particles, mp);
        }

        let (x_offset, y_offset, label) = match state {
            PetActionState::Idle => {
                let y = if t % 8 < 4 { 0i32 } else { 1i32 };
                (0, y, "IDLE".into())
            }
            PetActionState::Walk => {
                let step = t % 32;
                let x = match step { 0..=7=>-2, 8..=15=>0, 16..=23=>2, _=>0 };
                (x, 0, "WALK".into())
            }
            PetActionState::Run => {
                let step = t % 16;
                let x = match step { 0..=3=>-4, 4..=7=>0, 8..=11=>4, _=>0 };
                (x, 0, "RUN >>".into())
            }
            PetActionState::Jump => {
                let step = t % 12;
                let y = match step { 0..=2=>0, 3..=5=>-1, 6..=8=>0, _=>1 };
                (0, y, "JUMP ▲".into())
            }
            PetActionState::Attack => {
                let x = if t % 8 < 4 { 1i32 } else { 0i32 };
                (x, 0, "ATTACK".into())
            }
            PetActionState::Dash => {
                let step = t % 10;
                let x = match step { 0..=2=>-5, 3..=4=>0, 5..=7=>5, _=>0 };
                (x, 0, "DASH".into())
            }
            PetActionState::Hit => {
                let step = t % 4;
                let x = if step == 0 { -1 } else if step == 2 { 1 } else { 0 };
                (x, 0, "ALERT".into())
            }
            PetActionState::Death => {
                let y = if t % 16 < 8 { 0i32 } else { 1i32 };
                (0, y, "REST / ZZZ".into())
            }
        };

        EmojiFrame {
            core: face.to_string(),
            particles: particles.to_string(),
            x_offset,
            y_offset,
            state_label: label,
        }
    }

    pub fn format_badge(&self) -> String {
        let level = match self.stage { PetStage::Egg=>0, PetStage::Hatch=>1,
                                       PetStage::Juvenile=>2, PetStage::Adult=>3,
                                       PetStage::Mating=>4 };
        let mood_emoji = match self.mood { PetMood::Idle=>"☺", PetMood::Overheated=>"🔥",
                                           PetMood::Sick=>"💀", PetMood::Sprint=>"⚡" };
        format!("{} {} Lv.{} [{}]", self.stage.stage_emoji(), self.name, level, mood_emoji)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stage_from_xp() {
        assert_eq!(PetStage::from_xp(0), PetStage::Egg);
        assert_eq!(PetStage::from_xp(59), PetStage::Egg);
        assert_eq!(PetStage::from_xp(60), PetStage::Hatch);
        assert_eq!(PetStage::from_xp(179), PetStage::Hatch);
        assert_eq!(PetStage::from_xp(180), PetStage::Juvenile);
        assert_eq!(PetStage::from_xp(480), PetStage::Adult);
        assert_eq!(PetStage::from_xp(960), PetStage::Mating);
    }

    #[test]
    fn test_determine_mood() {
        assert_eq!(determine_mood(10.0, 50.0, 20, 10), PetMood::Idle);
        assert_eq!(determine_mood(90.0, 50.0, 20, 10), PetMood::Overheated);
        assert_eq!(determine_mood(20.0, 85.0, 20, 10), PetMood::Overheated);
        assert_eq!(determine_mood(10.0, 50.0, 90, 10), PetMood::Sick);
        assert_eq!(determine_mood(10.0, 50.0, 20, 95), PetMood::Sick);
        assert_eq!(determine_mood(75.0, 60.0, 20, 10), PetMood::Sprint);
    }

    #[test]
    fn test_single_emoji_frame_generation() {
        let pet = PetStatus {
            species: PetSpecies::Sovereign,
            name: "Sovereign".to_string(),
            stage: PetStage::Adult,
            mood: PetMood::Idle,
            xp: 500,
        };

        for state in [
            PetActionState::Idle,
            PetActionState::Walk,
            PetActionState::Run,
            PetActionState::Jump,
            PetActionState::Attack,
            PetActionState::Dash,
            PetActionState::Hit,
            PetActionState::Death,
        ] {
            let frame = pet.build_frame(state, 12, false);
            assert!(!frame.core.is_empty(), "Core emoji should never be empty");
            assert!(!frame.state_label.is_empty(), "State label should be set");
        }
    }

    #[test]
    fn test_species_backwards_compat() {
        assert_eq!(PetSpecies::parse("nagini"), PetSpecies::Sovereign);
        assert_eq!(PetSpecies::parse("ular"), PetSpecies::Sovereign);
        assert_eq!(PetSpecies::parse("ayam"), PetSpecies::Sovereign);
        assert_eq!(PetSpecies::parse("beruang"), PetSpecies::Sovereign);
        assert_eq!(PetSpecies::parse("unknown"), PetSpecies::Sovereign);
    }

    #[test]
    fn test_isometric_head_rendering() {
        // The big isometric voxel head should fill the requested grid with
        // shade characters and be different at different ticks (animated).
        let a = render_isometric_head(0, PetMood::Idle, 20, 12);
        assert_eq!(a.len(), 12);
        assert!(a.iter().all(|l| l.chars().count() == 20));

        // Total ink should be a meaningful fraction of the grid
        let ink_a: usize = a.iter().map(|l| l.chars().filter(|&c| c == '░' || c == '▒' || c == '▓' || c == '█').count()).sum();
        assert!(ink_a > 40, "head should occupy a substantial part of the grid, got {ink_a}");

        // Frame should animate (different tick → different ink or placement)
        let b = render_isometric_head(3, PetMood::Idle, 20, 12);
        assert_ne!(a, b, "frames at different ticks must differ (rotation/bob)");

        // Small panel → gracefully returns blank rows, no panic
        let small = render_isometric_head(0, PetMood::Idle, 3, 2);
        assert_eq!(small.len(), 2);
        assert!(small.iter().all(|l| l.chars().all(|c| c == ' ')));
    }
}
