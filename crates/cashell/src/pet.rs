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
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            // backward-compat: everything → Sovereign
            "nagini" | "dragon" | "ular" | "snake" |
            "ayam" | "singa" | "burung" | "beruang" |
            "lion" | "bird" | "bear" |
            "" | _ => Self::Sovereign,
        }
    }
    pub fn display_name(&self) -> &'static str { "Sovereign" }
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
const SOVEREIGN_FACE: &'static str = "😎";

/// The face used when the pet is sleeping / dead-ish
const SOVEREIGN_SLEEP: &'static str = "😴";

/// The face used during alert / overheated
const SOVEREIGN_ALERT: &'static str = "😠";

/// The face used during sprint / high load
const SOVEREIGN_SPINT: &'static str = "🤯";

/// The face used during sickness
const SOVEREIGN_SICK: &'static str = "🤢";

/// The face used for feed / treat bonus
const SOVEREIGN_JEALOUSY: &'static str = "🥰";

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
                let x = match step { 0..=7=>-2, 8..=15=>0, 16..=23=>2, _=>0 } as i32;
                (x, 0, "WALK".into())
            }
            PetActionState::Run => {
                let step = t % 16;
                let x = match step { 0..=3=>-4, 4..=7=>0, 8..=11=>4, _=>0 } as i32;
                (x, 0, "RUN >>".into())
            }
            PetActionState::Jump => {
                let step = t % 12;
                let y = match step { 0..=2=>0, 3..=5=>-1, 6..=8=>0, _=>1 } as i32;
                (0, y, "JUMP ▲".into())
            }
            PetActionState::Attack => {
                let x = if t % 8 < 4 { 1i32 } else { 0i32 };
                (x, 0, "ATTACK".into())
            }
            PetActionState::Dash => {
                let step = t % 10;
                let x = match step { 0..=2=>-5, 3..=4=>0, 5..=7=>5, _=>0 } as i32;
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
}
