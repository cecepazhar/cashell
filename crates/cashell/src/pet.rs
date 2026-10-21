//! Virtual Pet (Tamagotchi) Engine for cashell cockpit.
//!
//! Provides pet lifecycle progression (Egg -> Hatch -> Juvenile -> Adult -> Mating -> Egg),
//! XP gain tracking, and an 8-state Sleek Isometric 3D-styled animation engine
//! (IDLE, WALK, RUN, JUMP, ATTACK, DASH, HIT, DEATH)
//! for 5 sovereign shadow companions: Ayam (Shadow Hunter), Singa (Shadow Beast),
//! Burung (Shadow Falcon), Ular (Shadow Serpent), Beruang (Shadow Golem).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PetSpecies {
    Ayam,
    Singa,
    Burung,
    Ular,
    Beruang,
}

impl PetSpecies {
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "singa" | "lion" => Self::Singa,
            "burung" | "bird" => Self::Burung,
            "ular" | "snake" => Self::Ular,
            "beruang" | "bear" => Self::Beruang,
            _ => Self::Ayam,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ayam => "Ayam (Hunter)",
            Self::Singa => "Singa (Beast)",
            Self::Burung => "Burung (Falcon)",
            Self::Ular => "Ular (Serpent)",
            Self::Beruang => "Beruang (Golem)",
        }
    }

    pub fn emoji(&self, stage: &PetStage) -> &'static str {
        match stage {
            PetStage::Egg => "🥚",
            PetStage::Hatch => match self {
                Self::Ayam => "🐣",
                Self::Singa => "🦁",
                Self::Burung => "🐦",
                Self::Ular => "🐍",
                Self::Beruang => "🐻",
            },
            PetStage::Juvenile => match self {
                Self::Ayam => "🐥",
                Self::Singa => "🦁",
                Self::Burung => "🐦",
                Self::Ular => "🐍",
                Self::Beruang => "🐻",
            },
            PetStage::Adult => match self {
                Self::Ayam => "🗡",
                Self::Singa => "🦁",
                Self::Burung => "🦅",
                Self::Ular => "🐍",
                Self::Beruang => "🐻",
            },
            PetStage::Mating => "💑",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PetStage {
    Egg,
    Hatch,
    Juvenile,
    Adult,
    Mating,
}

impl PetStage {
    pub fn from_xp(xp: i64) -> Self {
        if xp >= 960 {
            Self::Mating
        } else if xp >= 480 {
            Self::Adult
        } else if xp >= 180 {
            Self::Juvenile
        } else if xp >= 60 {
            Self::Hatch
        } else {
            Self::Egg
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Egg => "Egg",
            Self::Hatch => "Hatching",
            Self::Juvenile => "Juvenile",
            Self::Adult => "Adult",
            Self::Mating => "Mating",
        }
    }

    pub fn next_threshold(&self) -> i64 {
        match self {
            Self::Egg => 60,
            Self::Hatch => 180,
            Self::Juvenile => 480,
            Self::Adult => 960,
            Self::Mating => 1200,
        }
    }
}

/// The current physical/mental state of the pet, derived from hardware telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PetMood {
    Idle,       // Normal, comfortable
    Overheated, // CPU temp > 80C or CPU > 85%
    Sick,       // RAM > 85% or Swap > 90%
    Sprint,     // High CPU load
}

/// 8-state sprite animation system modeled after 2D action/fighter sprite sheets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PetActionState {
    Idle,   // 0: Standing relaxed, breathing, isometric shadow base
    Walk,   // 1: Pacing across with stepping feet
    Run,    // 2: Sprinting at high speed, dust particles
    Jump,   // 3: Airborne leap, shadow separates
    Attack, // 4: Dual dagger / claw strike with shadow aura
    Dash,   // 5: High-speed isometric dash with afterimages
    Hit,    // 6: Recoiling from damage / thermal shock
    Death,  // 7: Knocked down resting / sleeping on floor
}

/// A pet action event for the action history log.
#[derive(Debug, Clone)]
pub struct PetActionLog {
    pub timestamp: String,
    pub action_type: String,
    pub detail: String,
}

/// Tamagotchi-style animated pet model.
pub struct PetStatus {
    pub species: PetSpecies,
    pub name: String,
    pub stage: PetStage,
    pub mood: PetMood,
    pub xp: i64,
}

impl PetStatus {
    pub fn format_badge(&self) -> String {
        let level = match self.stage {
            PetStage::Egg => 0,
            PetStage::Hatch => 1,
            PetStage::Juvenile => 2,
            PetStage::Adult => 3,
            PetStage::Mating => 4,
        };
        let mood_emoji = match self.mood {
            PetMood::Idle => "☺",
            PetMood::Overheated => "🔥",
            PetMood::Sick => "💀",
            PetMood::Sprint => "⚡",
        };
        format!(
            "{} {} Lv.{} [{}]",
            self.species.emoji(&self.stage),
            self.name,
            level,
            mood_emoji
        )
    }

    /// Returns a reference to the frame array for the current mood.
    pub fn frames(&self) -> &'static [&'static [&'static str]] {
        match self.mood {
            PetMood::Idle => self.frames_for_state(PetActionState::Idle),
            PetMood::Sprint | PetMood::Overheated => self.frames_for_state(PetActionState::Run),
            PetMood::Sick => self.frames_for_state(PetActionState::Death),
        }
    }

    /// Returns the 4-frame animation array for any of the 8 action states.
    pub fn frames_for_state(&self, state: PetActionState) -> &'static [&'static [&'static str]] {
        match (self.species, state) {
            // === AYAM (SHADOW HUNTER) 8 ISOMETRIC STATES ===
            (PetSpecies::Ayam, PetActionState::Idle) => &HUNTER_ISO_IDLE,
            (PetSpecies::Ayam, PetActionState::Walk) => &HUNTER_ISO_WALK,
            (PetSpecies::Ayam, PetActionState::Run) => &HUNTER_ISO_RUN,
            (PetSpecies::Ayam, PetActionState::Jump) => &HUNTER_ISO_JUMP,
            (PetSpecies::Ayam, PetActionState::Attack) => &HUNTER_ISO_ATTACK,
            (PetSpecies::Ayam, PetActionState::Dash) => &HUNTER_ISO_DASH,
            (PetSpecies::Ayam, PetActionState::Hit) => &HUNTER_ISO_HIT,
            (PetSpecies::Ayam, PetActionState::Death) => &HUNTER_ISO_DEATH,

            // === SINGA (SHADOW BEAST) 8 ISOMETRIC STATES ===
            (PetSpecies::Singa, PetActionState::Idle) => &BEAST_ISO_IDLE,
            (PetSpecies::Singa, PetActionState::Walk) => &BEAST_ISO_WALK,
            (PetSpecies::Singa, PetActionState::Run) => &BEAST_ISO_RUN,
            (PetSpecies::Singa, PetActionState::Jump) => &BEAST_ISO_JUMP,
            (PetSpecies::Singa, PetActionState::Attack) => &BEAST_ISO_ATTACK,
            (PetSpecies::Singa, PetActionState::Dash) => &BEAST_ISO_DASH,
            (PetSpecies::Singa, PetActionState::Hit) => &BEAST_ISO_HIT,
            (PetSpecies::Singa, PetActionState::Death) => &BEAST_ISO_DEATH,

            // === BURUNG (SHADOW FALCON) 8 ISOMETRIC STATES ===
            (PetSpecies::Burung, PetActionState::Idle) => &FALCON_ISO_IDLE,
            (PetSpecies::Burung, PetActionState::Walk) => &FALCON_ISO_WALK,
            (PetSpecies::Burung, PetActionState::Run) => &FALCON_ISO_RUN,
            (PetSpecies::Burung, PetActionState::Jump) => &FALCON_ISO_JUMP,
            (PetSpecies::Burung, PetActionState::Attack) => &FALCON_ISO_ATTACK,
            (PetSpecies::Burung, PetActionState::Dash) => &FALCON_ISO_DASH,
            (PetSpecies::Burung, PetActionState::Hit) => &FALCON_ISO_HIT,
            (PetSpecies::Burung, PetActionState::Death) => &FALCON_ISO_DEATH,

            // === ULAR (SHADOW SERPENT) 8 ISOMETRIC STATES ===
            (PetSpecies::Ular, PetActionState::Idle) => &SERPENT_ISO_IDLE,
            (PetSpecies::Ular, PetActionState::Walk) => &SERPENT_ISO_WALK,
            (PetSpecies::Ular, PetActionState::Run) => &SERPENT_ISO_RUN,
            (PetSpecies::Ular, PetActionState::Jump) => &SERPENT_ISO_JUMP,
            (PetSpecies::Ular, PetActionState::Attack) => &SERPENT_ISO_ATTACK,
            (PetSpecies::Ular, PetActionState::Dash) => &SERPENT_ISO_DASH,
            (PetSpecies::Ular, PetActionState::Hit) => &SERPENT_ISO_HIT,
            (PetSpecies::Ular, PetActionState::Death) => &SERPENT_ISO_DEATH,

            // === BERUANG (SHADOW GOLEM) 8 ISOMETRIC STATES ===
            (PetSpecies::Beruang, PetActionState::Idle) => &GOLEM_ISO_IDLE,
            (PetSpecies::Beruang, PetActionState::Walk) => &GOLEM_ISO_WALK,
            (PetSpecies::Beruang, PetActionState::Run) => &GOLEM_ISO_RUN,
            (PetSpecies::Beruang, PetActionState::Jump) => &GOLEM_ISO_JUMP,
            (PetSpecies::Beruang, PetActionState::Attack) => &GOLEM_ISO_ATTACK,
            (PetSpecies::Beruang, PetActionState::Dash) => &GOLEM_ISO_DASH,
            (PetSpecies::Beruang, PetActionState::Hit) => &GOLEM_ISO_HIT,
            (PetSpecies::Beruang, PetActionState::Death) => &GOLEM_ISO_DEATH,
        }
    }
}

const HUNTER_ISO_IDLE: [&[&str]; 4] = [
    &[
        "         ▄▄████▄▄         ",
        "        ◢██▌▐████▐◣        ",
        "       ██▌◢▐[◉ ◉]▐▌██      ",
        "      ⚡ ▀▀▀█▼▼█▀▀ ⚡       ",
        "       ██▐▀▄▀██▄▀██        ",
        "      ░▒▓████████▓▒░       ",
    ],
    &[
        "         ▄▄████▄▄         ",
        "        ◢██▌▐████▐◣        ",
        "       ██▌◢▐[─ ─]▐▌██      ",
        "         ▀▀▀█▼▼█▀▀  ⚡      ",
        "       ██▐▀▄▀██▄▀██        ",
        "      ░▒▓████████▓▒░       ",
    ],
    &[
        "         ▄▄████▄▄   ⚡     ",
        "        ◢██▌▐████▐◣        ",
        "       ██▌◢▐[◉ ◉]▐▌██      ",
        "      ⚡ ▀▀▀█▼▼█▀▀          ",
        "       ██▐▀▄▀██▄▀██        ",
        "      ░▒▓████████▓▒░       ",
    ],
    &[
        "         ▄▄████▄▄         ",
        "        ◢██▌▐████▐◣        ",
        "       ██▌◢▐[✦ ✦]▐▌██      ",
        "      ⚡ ▀▀▀█▼▼█▀▀ ⚡       ",
        "       ██▐▀▄▀██▄▀██        ",
        "      ░▒▓████████▓▒░       ",
    ],
];

const HUNTER_ISO_WALK: [&[&str]; 4] = [
    &[
        "         ▄▄████▄▄         ",
        "        ◢██▌▐████▐◣        ",
        "       ██▌◢▐[◉ ◉]▐▌██      ",
        "         ▀▀▀█▼▼█▀▀         ",
        "      ▄▀▀      ▀▄▄          ",
        "      ░▒▓████████▓▒░       ",
    ],
    &[
        "         ▄▄████▄▄         ",
        "        ◢██▌▐████▐◣        ",
        "       ██▌◢▐[◉ ◉]▐▌██      ",
        "         ▀▀▀█▼▼█▀▀         ",
        "        ▄▀▀    ▀▄▄         ",
        "      ░▒▓████████▓▒░       ",
    ],
    &[
        "         ▄▄████▄▄         ",
        "        ◢██▌▐████▐◣        ",
        "       ██▌◢▐[◉ ◉]▐▌██      ",
        "         ▀▀▀█▼▼█▀▀         ",
        "       ▀▄▄      ▄▀▀        ",
        "      ░▒▓████████▓▒░       ",
    ],
    &[
        "         ▄▄████▄▄         ",
        "        ◢██▌▐████▐◣        ",
        "       ██▌◢▐[◉ ◉]▐▌██      ",
        "         ▀▀▀█▼▼█▀▀         ",
        "        ▄▀▀    ▀▄▄         ",
        "      ░▒▓████████▓▒░       ",
    ],
];

const HUNTER_ISO_RUN: [&[&str]; 4] = [
    &[
        "  彡    ▄▄████▄▄ >>     ",
        " 彡    ◢████████◣ >>    ",
        "      ███▌[◉ ◉]▐███     ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡   ▄▄████▄▄ >>>>   ",
        " 彡    ◢████████◣ >>>>  ",
        "      ███▌[✦ ✦]▐███     ",
        "     ⚡  ▀████▀  ⚡⚡     ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
    &[
        "  彡    ▄▄████▄▄ >>     ",
        " 彡    ◢████████◣ >>    ",
        "      ███▌[◉ ◉]▐███     ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡   ▄▄████▄▄ >>>>   ",
        " 彡    ◢████████◣ >>>>  ",
        "      ███▌[✦ ✦]▐███     ",
        "     ⚡  ▀████▀  ⚡⚡     ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
];

const HUNTER_ISO_JUMP: [&[&str]; 4] = [
    &[
        "                        ",
        "        ▄▄████▄▄        ",
        "       ◢████████◣       ",
        "      ███▌[◉ ◉]▐███     ",
        "       ▀████████▀       ",
        "    ░▒▓██████████▓▒░    ",
    ],
    &[
        "      ▲ ▄▄████▄▄ ▲      ",
        "     ⚡◢████████◣⚡     ",
        "      ███▌[✦ ✦]▐███     ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "      ░▒▓██████▓▒░      ",
    ],
    &[
        "     ⚡ ▄▄████▄▄ ⚡      ",
        "    ⚡⚡◢████████◣⚡⚡    ",
        "      ███▌[✦ ✦]▐███     ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "        ░▒▓██▓▒░        ",
    ],
    &[
        "                        ",
        "        ▄▄████▄▄        ",
        "       ◢████████◣       ",
        "      ███▌[◉ ◉]▐███     ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const HUNTER_ISO_ATTACK: [&[&str]; 4] = [
    &[
        "     ⚡  ▄▄████▄▄  ⚡     ",
        "    ══>◢████████◣       ",
        "      ███▌[✦ ✦]▐███     ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "    ⚡⚡  ▄▄████▄▄  ⚡⚡   ",
        "   ═══>◢████████◣<═══   ",
        "   ══>███▌[◣ ◢]▐███<══  ",
        "    ⚡⚡  ▀████▀  ⚡⚡    ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "    ⚡⚡  ▄▄████▄▄  ⚡⚡   ",
        "   ═══>◢████████◣<═══   ",
        "   ══>███▌[◣ ◢]▐███<══  ",
        "    ⚡⚡  ▀████▀  ⚡⚡    ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "        ▄▄████▄▄   彡   ",
        "       ◢████████◣  彡   ",
        "      ███▌[✦ ✦]▐███     ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const HUNTER_ISO_DASH: [&[&str]; 4] = [
    &[
        "  彡    ▄▄████▄▄ >>     ",
        " 彡   ◢████████◣ >>     ",
        " ░▒▓ ███▌[⚡ ⚡]▐███     ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡  ▄▄████▄▄ >>>>    ",
        " 彡   ◢████████◣ >>>>   ",
        " ░▒▓ ███▌[⚡ ⚡]▐███     ",
        " ░▒▓   ▀████▀           ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
    &[
        "  彡    ▄▄████▄▄ >>     ",
        " 彡   ◢████████◣ >>     ",
        " ░▒▓ ███▌[⚡ ⚡]▐███     ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        "        ▄▄████▄▄        ",
        "       ◢████████◣       ",
        "      ███▌[◉ ◉]▐███     ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const HUNTER_ISO_HIT: [&[&str]; 4] = [
    &[
        "        ▄▄████▄▄  💥    ",
        "       ◢████████◣ 💦    ",
        "      ███▌[• ﹏ •]▐███  ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "        ▄▄████▄▄  💦💦  ",
        "       ◢████████◣       ",
        "      ███▌[• ﹏ •]▐███  ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "        ▄▄████▄▄  💥    ",
        "       ◢████████◣ 💦    ",
        "      ███▌[• ﹏ •]▐███  ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "        ▄▄████▄▄  💦💦  ",
        "       ◢████████◣       ",
        "      ███▌[• ﹏ •]▐███  ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const HUNTER_ISO_DEATH: [&[&str]; 4] = [
    &[
        "        ▄▄████▄▄   ✦    ",
        "       ◢████████◣       ",
        "      ███▌[─ ─]▐███     ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "        ▄▄████▄▄   ✦✦   ",
        "       ◢████████◣       ",
        "      ███▌[─ ─]▐███     ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "        ▄▄████▄▄   ✦✦✦  ",
        "       ◢████████◣       ",
        "      ███▌[─ ─]▐███     ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "        ▄▄████▄▄   ✦✦   ",
        "       ◢████████◣       ",
        "      ███▌[─ ─]▐███     ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const BEAST_ISO_IDLE: [&[&str]; 4] = [
    &[
        "      ╭▓▓██████▓▓╮      ",
        "     ▓◢██[◉ ◉]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "     ⚡  ▀████▀  ⚡      ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮      ",
        "     ▓◢██[─ ─]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀   ⚡     ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮  ⚡   ",
        "     ▓◢██[◉ ◉]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "     ⚡  ▀████▀         ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮      ",
        "     ▓◢██[✦ ✦]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "     ⚡  ▀████▀  ⚡      ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const BEAST_ISO_WALK: [&[&str]; 4] = [
    &[
        "      ╭▓▓██████▓▓╮      ",
        "     ▓◢██[◉ ◉]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "      ▄▀        ▀▄      ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮      ",
        "     ▓◢██[◉ ◉]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮      ",
        "     ▓◢██[◉ ◉]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "       ▀▄      ▄▀       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮      ",
        "     ▓◢██[◉ ◉]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const BEAST_ISO_RUN: [&[&str]; 4] = [
    &[
        "  彡  ╭▓▓██████▓▓╮ >>   ",
        " 彡  ▓◢██[◉ ◉]██◣▓ >>   ",
        "      ▀██( ▼▼ )██▀      ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡 ╭▓▓██████▓▓╮ >>>> ",
        " 彡  ▓◢██[✦ ✦]██◣▓ >>>> ",
        "      ▀██( ▼▼ )██▀      ",
        "     ⚡  ▀████▀  ⚡⚡     ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
    &[
        "  彡  ╭▓▓██████▓▓╮ >>   ",
        " 彡  ▓◢██[◉ ◉]██◣▓ >>   ",
        "      ▀██( ▼▼ )██▀      ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡 ╭▓▓██████▓▓╮ >>>> ",
        " 彡  ▓◢██[✦ ✦]██◣▓ >>>> ",
        "      ▀██( ▼▼ )██▀      ",
        "     ⚡  ▀████▀  ⚡⚡     ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
];

const BEAST_ISO_JUMP: [&[&str]; 4] = [
    &[
        "                        ",
        "      ╭▓▓██████▓▓╮      ",
        "     ▓◢██[◉ ◉]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "       ▀████████▀       ",
        "    ░▒▓██████████▓▒░    ",
    ],
    &[
        "      ▲╭▓▓██████▓▓╮▲    ",
        "     ⚡▓◢██[✦ ✦]██◣▓⚡    ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "      ░▒▓██████▓▒░      ",
    ],
    &[
        "     ⚡ ╭▓▓██████▓▓╮ ⚡   ",
        "    ⚡⚡▓◢██[✦ ✦]██◣▓⚡⚡   ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "        ░▒▓██▓▒░        ",
    ],
    &[
        "                        ",
        "      ╭▓▓██████▓▓╮      ",
        "     ▓◢██[◉ ◉]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const BEAST_ISO_ATTACK: [&[&str]; 4] = [
    &[
        "    爪╭▓▓██████▓▓╮爪    ",
        "    ══>▓◢██[✦ ✦]██◣▓    ",
        "      ▀██( ▼▼ )██▀      ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "   ⚡爪╭▓▓██████▓▓╮爪⚡   ",
        "  ═══>▓◢██[◣ ◢]██◣▓<═══ ",
        "  ═══>▀██( ▼▼ )██▀<═══  ",
        "    ⚡⚡  ▀████▀  ⚡⚡    ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "   ⚡爪╭▓▓██████▓▓╮爪⚡   ",
        "  ═══>▓◢██[◣ ◢]██◣▓<═══ ",
        "  ═══>▀██( ▼▼ )██▀<═══  ",
        "    ⚡⚡  ▀████▀  ⚡⚡    ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮  彡  ",
        "     ▓◢██[✦ ✦]██◣▓  彡  ",
        "      ▀██( ▼▼ )██▀      ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const BEAST_ISO_DASH: [&[&str]; 4] = [
    &[
        "  彡  ╭▓▓██████▓▓╮ >>   ",
        " 彡  ▓◢██[⚡ ⚡]██◣▓ >>   ",
        " ░▒▓  ▀██( ►► )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡 ╭▓▓██████▓▓╮ >>>> ",
        " 彡  ▓◢██[⚡ ⚡]██◣▓ >>>> ",
        " ░▒▓  ▀██( ►► )██▀      ",
        " ░▒▓   ▀████▀           ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
    &[
        "  彡  ╭▓▓██████▓▓╮ >>   ",
        " 彡  ▓◢██[⚡ ⚡]██◣▓ >>   ",
        " ░▒▓  ▀██( ►► )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        "      ╭▓▓██████▓▓╮      ",
        "     ▓◢██[◉ ◉]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const BEAST_ISO_HIT: [&[&str]; 4] = [
    &[
        "      ╭▓▓██████▓▓╮ 💥   ",
        "     ▓◢██[• ﹏ •]██◣▓ 💦",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮ 💦💦 ",
        "     ▓◢██[• ﹏ •]██◣▓   ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮ 💥   ",
        "     ▓◢██[• ﹏ •]██◣▓ 💦",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮ 💦💦 ",
        "     ▓◢██[• ﹏ •]██◣▓   ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const BEAST_ISO_DEATH: [&[&str]; 4] = [
    &[
        "      ╭▓▓██████▓▓╮  ✦   ",
        "     ▓◢██[─ ─]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮  ✦✦  ",
        "     ▓◢██[─ ─]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮  ✦✦✦ ",
        "     ▓◢██[─ ─]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭▓▓██████▓▓╮  ✦✦  ",
        "     ▓◢██[─ ─]██◣▓      ",
        "      ▀██( ▼▼ )██▀      ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const FALCON_ISO_IDLE: [&[&str]; 4] = [
    &[
        "      ▲  ▄▄██▄▄  ▲      ",
        "    ╭━━━[◉ ◉]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "     ⚡  ◥████◤  ⚡      ",
        "        ◢████◣          ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲      ",
        "    ╭━━━[─ ─]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤   ⚡     ",
        "        ◢████◣          ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲  ⚡   ",
        "    ╭━━━[◉ ◉]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "     ⚡  ◥████◤          ",
        "        ◢████◣          ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲      ",
        "    ╭━━━[✦ ✦]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "     ⚡  ◥████◤  ⚡      ",
        "        ◢████◣          ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const FALCON_ISO_WALK: [&[&str]; 4] = [
    &[
        "      ▲  ▄▄██▄▄  ▲      ",
        "    ╭━━━[◉ ◉]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "      ▄▀        ▀▄      ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲      ",
        "    ╭━━━[◉ ◉]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲      ",
        "    ╭━━━[◉ ◉]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "       ▀▄      ▄▀       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲      ",
        "    ╭━━━[◉ ◉]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const FALCON_ISO_RUN: [&[&str]; 4] = [
    &[
        "  彡  ▲  ▄▄██▄▄  ▲ >>   ",
        " 彡 ╭━━━[◉ ◉]━━━╮ >>    ",
        "     ╰━━██(▲)██━━╯      ",
        "     ⚡  ◥████◤  ⚡      ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡 ▲  ▄▄██▄▄  ▲ >>>> ",
        " 彡 ╭━━━[✦ ✦]━━━╮ >>>>  ",
        "     ╰━━██(▲)██━━╯      ",
        "     ⚡  ◥████◤  ⚡⚡     ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
    &[
        "  彡  ▲  ▄▄██▄▄  ▲ >>   ",
        " 彡 ╭━━━[◉ ◉]━━━╮ >>    ",
        "     ╰━━██(▲)██━━╯      ",
        "     ⚡  ◥████◤  ⚡      ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡 ▲  ▄▄██▄▄  ▲ >>>> ",
        " 彡 ╭━━━[✦ ✦]━━━╮ >>>>  ",
        "     ╰━━██(▲)██━━╯      ",
        "     ⚡  ◥████◤  ⚡⚡     ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
];

const FALCON_ISO_JUMP: [&[&str]; 4] = [
    &[
        "                        ",
        "      ▲  ▄▄██▄▄  ▲      ",
        "    ╭━━━[◉ ◉]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "       ▀████████▀       ",
        "    ░▒▓██████████▓▒░    ",
    ],
    &[
        "      ▲ ▲▄▄██▄▄▲ ▲      ",
        "     ⚡╭━━[✦ ✦]━━╮⚡      ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "      ░▒▓██████▓▒░      ",
    ],
    &[
        "     ⚡ ▲▄▄██▄▄▲ ⚡      ",
        "    ⚡⚡╭━━[✦ ✦]━━╮⚡⚡     ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "        ░▒▓██▓▒░        ",
    ],
    &[
        "                        ",
        "      ▲  ▄▄██▄▄  ▲      ",
        "    ╭━━━[◉ ◉]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◢████◣          ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const FALCON_ISO_ATTACK: [&[&str]; 4] = [
    &[
        "     ⚡ ▲ ▄▄██▄▄ ▲ ⚡     ",
        "    ══>╭━━[✦ ✦]━━╮      ",
        "     ╰━━██(▲)██━━╯      ",
        "     ⚡  ◥████◤  ⚡      ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "    ⚡⚡ ▲ ▄▄██▄▄ ▲ ⚡⚡   ",
        "   ═══>╭━━[◣ ◢]━━╮<═══  ",
        "   ══>╰━━██(▲)██━━╯<══  ",
        "    ⚡⚡  ◥████◤  ⚡⚡    ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "    ⚡⚡ ▲ ▄▄██▄▄ ▲ ⚡⚡   ",
        "   ═══>╭━━[◣ ◢]━━╮<═══  ",
        "   ══>╰━━██(▲)██━━╯<══  ",
        "    ⚡⚡  ◥████◤  ⚡⚡    ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲  彡  ",
        "    ╭━━━[✦ ✦]━━━╮  彡   ",
        "     ╰━━██(▲)██━━╯      ",
        "     ⚡  ◥████◤  ⚡      ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const FALCON_ISO_DASH: [&[&str]; 4] = [
    &[
        "  彡  ▲  ▄▄██▄▄  ▲ >>   ",
        " 彡 ╭━━━[⚡ ⚡]━━━╮ >>    ",
        " ░▒▓ ╰━━██(►)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡 ▲  ▄▄██▄▄  ▲ >>>> ",
        " 彡 ╭━━━[⚡ ⚡]━━━╮ >>>>  ",
        " ░▒▓ ╰━━██(►)██━━╯      ",
        " ░▒▓    ◥████◤          ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
    &[
        "  彡  ▲  ▄▄██▄▄  ▲ >>   ",
        " 彡 ╭━━━[⚡ ⚡]━━━╮ >>    ",
        " ░▒▓ ╰━━██(►)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲      ",
        "    ╭━━━[◉ ◉]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const FALCON_ISO_HIT: [&[&str]; 4] = [
    &[
        "      ▲  ▄▄██▄▄  ▲  💥  ",
        "    ╭━━━[• ﹏ •]━━━╮ 💦 ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲ 💦💦 ",
        "    ╭━━━[• ﹏ •]━━━╮    ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲  💥  ",
        "    ╭━━━[• ﹏ •]━━━╮ 💦 ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲ 💦💦 ",
        "    ╭━━━[• ﹏ •]━━━╮    ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const FALCON_ISO_DEATH: [&[&str]; 4] = [
    &[
        "      ▲  ▄▄██▄▄  ▲   ✦  ",
        "    ╭━━━[─ ─]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ◢████◣          ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲   ✦✦ ",
        "    ╭━━━[─ ─]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ◢████◣          ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲  ✦✦✦ ",
        "    ╭━━━[─ ─]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ◢████◣          ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ▲  ▄▄██▄▄  ▲   ✦✦ ",
        "    ╭━━━[─ ─]━━━╮       ",
        "     ╰━━██(▲)██━━╯      ",
        "        ◥████◤          ",
        "        ◢████◣          ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const SERPENT_ISO_IDLE: [&[&str]; 4] = [
    &[
        "         .-'-'          ",
        "    .-=-'._   -'=-._   ",
        "   / .-=-'(o) (o).' \\  ",
        "   |  '=-'---'---'=-   ",
        "   '=-.  |  |  |  .=-  ",
        "    '=-'---'---'=-'-'  ",
    ],
    &[
        "         .-'-'          ",
        "    .-=-'._   -'=-._   ",
        "   / .-=-'(- - -)\\' \\  ",
        "   |  '=-'---'---'=-   ",
        "   '=-.  |  |  |  .=-  ",
        "    '=-'---'---'=-'-'  ",
    ],
    &[
        "         .-'-'          ",
        "    .-=-'._   -'=-._   ",
        "   / .-=-'(o) (o).' \\  ",
        "   |  '=-'---'---'=-   ",
        "   '=-.  |  |  |  .=-  ",
        "    '=-'---'---'=-'-'  ",
    ],
    &[
        "         .-'-'          ",
        "    .-=-'._   -'=-._   ",
        "   / .-=-'(* *)(')\\' \\  ",
        "   |  '=-'---'---'=-   ",
        "   '=-.  |  |  |  .=-  ",
        "    '=-'---'---'=-'-'  ",
    ],
];

const SERPENT_ISO_WALK: [&[&str]; 4] = [
    &[
        "  \\              /   ",
        "  / .-=-'(o) (o).\\  ",
        "  |  '=-'---'---'=| ",
        "   '=-.'         .=- ",
        "   /_/-'        '-\\_ ",
        "    '-'          '-'  ",
    ],
    &[
        "   \\             /   ",
        "   / .-=-'(o) (o).\\  ",
        "   |  '=-'---'---'=| ",
        "    '=-.'       .=-  ",
        "   /_/-'     '-\\_   ",
        "    '-'        '-'   ",
    ],
    &[
        "    \\            /   ",
        "    / .-=-'(o) (o).\\ ",
        "    |  '=-'---'---'=|",
        "     '=-.'     .=-   ",
        "    /_/-'    '-\\_    ",
        "     '-'       '-'    ",
    ],
    &[
        "  \\              /   ",
        "  / .-=-'(o) (o).\\  ",
        "  |  '=-'---'---'=| ",
        "   '=-.'         .=- ",
        "   /_/-'        '-\\_ ",
        "    '-'          '-'  ",
    ],
];

const SERPENT_ISO_RUN: [&[&str]; 4] = [
    &[
        "   >>    .-'-'      >>   ",
        "  >>.-=-'._   -'=-._ >>  ",
        "   / .-=-'(o) (o).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-    ",
        "    '=-'---'---'=-'-'    ",
    ],
    &[
        "  >>>>   .-'-'     >>>>  ",
        " >>>.-=-'._   -'=-._ >>> ",
        "   / .-=-'(^)(^).' \\     ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-    ",
        "    '=-'---'---'=-'-'    ",
    ],
    &[
        "   >>    .-'-'      >>   ",
        "  >>.-=-'._   -'=-._ >>  ",
        "   / .-=-'(o) (o).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-    ",
        "    '=-'---'---'=-'-'    ",
    ],
    &[
        "  >>>>   .-'-'     >>>>  ",
        " >>>.-=-'._   -'=-._ >>> ",
        "   / .-=-'(^)(^).' \\     ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-    ",
        "    '=-'---'---'=-'-'    ",
    ],
];

const SERPENT_ISO_JUMP: [&[&str]; 4] = [
    &[
        "                         ",
        "         .-'-'           ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(*) (*).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
    ],
    &[
        "        ^.-'-.^          ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(*) (*).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "       ^^.-'-.^^         ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(*) (*).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "         .-'-'           ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(o) (o).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
];

const SERPENT_ISO_ATTACK: [&[&str]; 4] = [
    &[
        "        /| .-'-. |\\      ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(=) (=).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "       //| .-'-. |\\\\     ",
        "   .-=-'._ (O) (O) -=-.  ",
        "   / .-=-'(=) (=).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "       //| .-'-. |\\\\     ",
        "   .-=-'._ (O) (O) -=-.  ",
        "   / .-=-'(=) (=).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "        /| .-'-. |\\      ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(=) (=).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
];

const SERPENT_ISO_DASH: [&[&str]; 4] = [
    &[
        "   ~>    .-'-'      ~>   ",
        "  ~~>.-=-'._  -'=-._ ~~> ",
        "   / .-=-'(>) (>).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "  ~~>>   .-'-'     ~~>>  ",
        " ~~~>.-=-'._  -'=-._~~~> ",
        "   / .-=-'(>) (>).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "   ~>    .-'-'      ~>   ",
        "  ~~>.-=-'._  -'=-._ ~~> ",
        "   / .-=-'(>) (>).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "         .-'-'           ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(o) (o).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
];

const SERPENT_ISO_HIT: [&[&str]; 4] = [
    &[
        "         .-'-'           ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(x) (x).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "         .-'-'           ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(>) (<).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "         .-'-'           ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(x) (x).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "         .-'-'           ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(>) (<).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
];

const SERPENT_ISO_DEATH: [&[&str]; 4] = [
    &[
        "         .-'-'    z      ",
        "    .-=-'._   -'=-._ z   ",
        "   / .-=-'(-) (-).' \\ Z  ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "         .-'-'     z     ",
        "    .-=-'._   -'=-._ Z   ",
        "   / .-=-'(-) (-).' \\Z   ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "         .-'-'    z      ",
        "    .-=-'._   -'=-._ z   ",
        "   / .-=-'(-) (-).' \\ Z  ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
    &[
        "         .-'-'           ",
        "    .-=-'._   -'=-._     ",
        "   / .-=-'(-) (-).' \\    ",
        "   |  '=-'---'---'=-     ",
        "   '=-.  |  |  |  .=-     ",
        "    '=-'---'---'=-'-'     ",
    ],
];

const GOLEM_ISO_IDLE: [&[&str]; 4] = [
    &[
        "      ╭(o)━━━━(o)╮      ",
        "     ◢███[◉ ◉]███◣      ",
        "     ████( ● )████      ",
        "     ⚡  ▀████▀  ⚡      ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮      ",
        "     ◢███[─ ─]███◣      ",
        "     ████( ● )████      ",
        "        ▀████▀   ⚡     ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮  ⚡   ",
        "     ◢███[◉ ◉]███◣      ",
        "     ████( ● )████      ",
        "     ⚡  ▀████▀         ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮      ",
        "     ◢███[✦ ✦]███◣      ",
        "     ████( ● )████      ",
        "     ⚡  ▀████▀  ⚡      ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const GOLEM_ISO_WALK: [&[&str]; 4] = [
    &[
        "      ╭(o)━━━━(o)╮      ",
        "     ◢███[◉ ◉]███◣      ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "      ▄▀        ▀▄      ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮      ",
        "     ◢███[◉ ◉]███◣      ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮      ",
        "     ◢███[◉ ◉]███◣      ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "       ▀▄      ▄▀       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮      ",
        "     ◢███[◉ ◉]███◣      ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const GOLEM_ISO_RUN: [&[&str]; 4] = [
    &[
        "  彡  ╭(o)━━━━(o)╮ >>   ",
        " 彡  ◢███[◉ ◉]███◣ >>   ",
        "     ████( ● )████      ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡 ╭(o)━━━━(o)╮ >>>> ",
        " 彡  ◢███[✦ ✦]███◣ >>>> ",
        "     ████( ● )████      ",
        "     ⚡  ▀████▀  ⚡⚡     ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
    &[
        "  彡  ╭(o)━━━━(o)╮ >>   ",
        " 彡  ◢███[◉ ◉]███◣ >>   ",
        "     ████( ● )████      ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡 ╭(o)━━━━(o)╮ >>>> ",
        " 彡  ◢███[✦ ✦]███◣ >>>> ",
        "     ████( ● )████      ",
        "     ⚡  ▀████▀  ⚡⚡     ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
];

const GOLEM_ISO_JUMP: [&[&str]; 4] = [
    &[
        "                        ",
        "      ╭(o)━━━━(o)╮      ",
        "     ◢███[◉ ◉]███◣      ",
        "     ████( ● )████      ",
        "       ▀████████▀       ",
        "    ░▒▓██████████▓▒░    ",
    ],
    &[
        "      ▲╭(o)━━━━(o)╮▲    ",
        "     ⚡◢███[✦ ✦]███◣⚡    ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "      ░▒▓██████▓▒░      ",
    ],
    &[
        "     ⚡ ╭(o)━━━━(o)╮ ⚡   ",
        "    ⚡⚡◢███[✦ ✦]███◣⚡⚡   ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "        ░▒▓██▓▒░        ",
    ],
    &[
        "                        ",
        "      ╭(o)━━━━(o)╮      ",
        "     ◢███[◉ ◉]███◣      ",
        "     ████( ● )████      ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const GOLEM_ISO_ATTACK: [&[&str]; 4] = [
    &[
        "     ⚡╭(o)━━━━(o)╮⚡    ",
        "    ══>◢███[✦ ✦]███◣    ",
        "     ████( ● )████      ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "    ⚡⚡╭(o)━━━━(o)╮⚡⚡  ",
        "   ═══>◢███[◣ ◢]███◣<═══ ",
        "   ══>████( ● )████<══  ",
        "    ⚡⚡  ▀████▀  ⚡⚡    ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "    ⚡⚡╭(o)━━━━(o)╮⚡⚡  ",
        "   ═══>◢███[◣ ◢]███◣<═══ ",
        "   ══>████( ● )████<══  ",
        "    ⚡⚡  ▀████▀  ⚡⚡    ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮  彡  ",
        "     ◢███[✦ ✦]███◣  彡  ",
        "     ████( ● )████      ",
        "     ⚡  ▀████▀  ⚡      ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const GOLEM_ISO_DASH: [&[&str]; 4] = [
    &[
        "  彡  ╭(o)━━━━(o)╮ >>   ",
        " 彡  ◢███[⚡ ⚡]███◣ >>   ",
        " ░▒▓ ████( ►► )████     ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        " 彡彡 ╭(o)━━━━(o)╮ >>>> ",
        " 彡  ◢███[⚡ ⚡]███◣ >>>> ",
        " ░▒▓ ████( ►► )████     ",
        " ░▒▓   ▀████▀           ",
        "        ▀▄    ▄▀        ",
        "   ░▒▓████████▓▒░       ",
    ],
    &[
        "  彡  ╭(o)━━━━(o)╮ >>   ",
        " 彡  ◢███[⚡ ⚡]███◣ >>   ",
        " ░▒▓ ████( ►► )████     ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "    ░▒▓████████▓▒░      ",
    ],
    &[
        "      ╭(o)━━━━(o)╮      ",
        "     ◢███[◉ ◉]███◣      ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const GOLEM_ISO_HIT: [&[&str]; 4] = [
    &[
        "      ╭(o)━━━━(o)╮ 💥   ",
        "     ◢███[• ﹏ •]███◣ 💦",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮ 💦💦 ",
        "     ◢███[• ﹏ •]███◣   ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮ 💥   ",
        "     ◢███[• ﹏ •]███◣ 💦",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮ 💦💦 ",
        "     ◢███[• ﹏ •]███◣   ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "        ▄▀    ▀▄        ",
        "     ░▒▓████████▓▒░     ",
    ],
];

const GOLEM_ISO_DEATH: [&[&str]; 4] = [
    &[
        "      ╭(o)━━━━(o)╮   ✦  ",
        "     ◢███[─ ─]███◣      ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮  ✦✦  ",
        "     ◢███[─ ─]███◣      ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮ ✦✦✦  ",
        "     ◢███[─ ─]███◣      ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
    &[
        "      ╭(o)━━━━(o)╮  ✦✦  ",
        "     ◢███[─ ─]███◣      ",
        "     ████( ● )████      ",
        "        ▀████▀          ",
        "       ◢████████◣       ",
        "     ░▒▓████████▓▒░     ",
    ],
];

/// Determines the pet's mood based on hardware telemetry.
pub fn determine_mood(cpu_pct: f32, cpu_temp_c: f32, ram_pct: u8, swap_pct: u8) -> PetMood {
    if ram_pct > 85 || swap_pct > 90 {
        return PetMood::Sick;
    }
    if cpu_temp_c > 80.0 || cpu_pct > 85.0 {
        return PetMood::Overheated;
    }
    if cpu_pct > 70.0 {
        return PetMood::Sprint;
    }
    PetMood::Idle
}

/// Colorizes a single row of an 8-bit isometric sprite into rich retro multi-hued spans.
pub fn colorize_retro_sprite_line(
    species: PetSpecies,
    state: PetActionState,
    line_idx: usize,
    text: &str,
    _mood: PetMood,
    is_stopped: bool,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current_buf = String::new();
    let mut current_color: Option<Color> = None;

    for ch in text.chars() {
        let ch_color = if ch == ' ' {
            current_color.unwrap_or(Color::Reset)
        } else if is_stopped {
            Color::Rgb(115, 135, 120)
        } else {
            match ch {
                '⚡' => Color::Rgb(255, 235, 60),
                '💥' => Color::Rgb(255, 135, 45),
                '💦' => Color::Rgb(100, 210, 255),
                '彡' => Color::Rgb(185, 195, 215),
                '═' | '<' => Color::Rgb(255, 85, 155),
                '>' => {
                    if species == PetSpecies::Ular {
                        Color::Rgb(255, 235, 60)
                    } else if state == PetActionState::Run || state == PetActionState::Dash {
                        Color::Rgb(255, 210, 75)
                    } else {
                        Color::Rgb(255, 85, 155)
                    }
                }
                '✦' => {
                    if state == PetActionState::Death {
                        Color::Rgb(190, 160, 230)
                    } else {
                        Color::Rgb(255, 245, 140)
                    }
                }
                '[' | ']' => Color::Rgb(140, 145, 160),
                '◉' => Color::Rgb(255, 255, 255),
                '•' => Color::Rgb(255, 115, 115),
                '─' => Color::Rgb(185, 195, 215),
                '►' => Color::Rgb(80, 230, 255),
                '(' | ')' => Color::Rgb(150, 155, 170),
                '▲' => {
                    if species == PetSpecies::Burung && line_idx == 2 {
                        Color::Rgb(255, 185, 30)
                    } else if species == PetSpecies::Burung && line_idx == 0 {
                        Color::Rgb(50, 225, 255)
                    } else {
                        Color::Rgb(250, 195, 60)
                    }
                }
                '▼' => {
                    if species == PetSpecies::Ayam || species == PetSpecies::Burung {
                        Color::Rgb(255, 145, 30)
                    } else if species == PetSpecies::Singa {
                        Color::Rgb(255, 245, 225)
                    } else {
                        Color::Rgb(255, 120, 120)
                    }
                }
                '∿' => Color::Rgb(185, 245, 125),
                '●' => Color::Rgb(0, 240, 255),
                '爪' => Color::Rgb(255, 215, 90),
                '░' => Color::Rgb(50, 58, 75),
                '▒' => Color::Rgb(65, 75, 95),
                '▓' => {
                    if species == PetSpecies::Singa && line_idx < 3 {
                        Color::Rgb(215, 105, 45) // Auburn mane
                    } else {
                        Color::Rgb(75, 88, 112)
                    }
                }
                // New pure ASCII serpent characters
                'z' | 'Z' => Color::Rgb(230, 215, 245),
                '^' => Color::Rgb(185, 195, 215),
                'x' => Color::Rgb(255, 115, 115),
                '=' => Color::Rgb(255, 155, 100),
                'O' => Color::Rgb(255, 135, 125),
                '~' => Color::Rgb(185, 245, 125),
                _ => {
                    if line_idx == 5 {
                        Color::Rgb(45, 52, 70)
                    } else {
                        match species {
                            PetSpecies::Ayam => match line_idx {
                                0 => Color::Rgb(235, 65, 65),  // Crest comb
                                1 => Color::Rgb(255, 205, 65), // Head
                                2 => Color::Rgb(255, 180, 50), // Upper torso
                                3 => Color::Rgb(240, 175, 45), // Body
                                _ => Color::Rgb(220, 140, 40), // Feet
                            },
                            PetSpecies::Singa => match line_idx {
                                0 => Color::Rgb(215, 105, 45), // Mane
                                1 => Color::Rgb(250, 195, 70), // Face
                                2 => Color::Rgb(245, 185, 60), // Muzzle
                                3 => Color::Rgb(230, 165, 50), // Torso
                                _ => Color::Rgb(220, 150, 50), // Paws
                            },
                            PetSpecies::Burung => match line_idx {
                                0 => Color::Rgb(50, 225, 255),  // Cyan crown
                                1 => Color::Rgb(245, 250, 255), // White face
                                2 => Color::Rgb(75, 150, 245),  // Azure wings
                                3 => Color::Rgb(235, 245, 255), // Chest
                                _ => Color::Rgb(250, 195, 50),  // Talons
                            },
                            PetSpecies::Ular => match line_idx {
                                0 => Color::Rgb(200, 130, 240), // Violet crown
                                1 => Color::Rgb(50, 215, 120),  // Emerald hood
                                2 => Color::Rgb(185, 245, 125), // Lime body
                                3 => Color::Rgb(40, 185, 105),  // Jade coils
                                _ => Color::Rgb(30, 155, 85),   // Forest tail
                            },
                            PetSpecies::Beruang => match line_idx {
                                0 => Color::Rgb(220, 175, 75),  // Bronze horns
                                1 => Color::Rgb(115, 145, 185), // Steel helm
                                2 => Color::Rgb(100, 125, 165), // Shoulders
                                3 => Color::Rgb(95, 120, 160),  // Golem plate
                                _ => Color::Rgb(80, 100, 135),  // Heavy treads
                            },
                        }
                    }
                }
            }
        };

        if let Some(cur) = current_color {
            if ch != ' ' && cur != ch_color {
                spans.push(Span::styled(
                    std::mem::take(&mut current_buf),
                    Style::default().fg(cur).add_modifier(Modifier::BOLD),
                ));
                current_color = Some(ch_color);
            }
        } else {
            current_color = Some(ch_color);
        }

        current_buf.push(ch);
    }

    if !current_buf.is_empty() {
        let col = current_color.unwrap_or(Color::Reset);
        spans.push(Span::styled(
            current_buf,
            Style::default().fg(col).add_modifier(Modifier::BOLD),
        ));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_colorize_retro_sprite_line() {
        let spans = colorize_retro_sprite_line(
            PetSpecies::Ayam,
            PetActionState::Idle,
            2,
            "      ███▌[◉ ◉]▐███     ",
            PetMood::Idle,
            false,
        );
        assert!(!spans.is_empty());
        // Should have colored spans for eye brackets/pupils and feathers
        assert!(spans.iter().any(|s| s.content.contains("◉")));
    }

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
    fn test_species_parsing() {
        assert_eq!(PetSpecies::parse("ayam"), PetSpecies::Ayam);
        assert_eq!(PetSpecies::parse("singa"), PetSpecies::Singa);
        assert_eq!(PetSpecies::parse("burung"), PetSpecies::Burung);
        assert_eq!(PetSpecies::parse("ular"), PetSpecies::Ular);
        assert_eq!(PetSpecies::parse("beruang"), PetSpecies::Beruang);
    }

    #[test]
    fn test_determine_mood() {
        assert_eq!(determine_mood(10.0, 50.0, 20, 10), PetMood::Idle);
        assert_eq!(determine_mood(90.0, 50.0, 20, 10), PetMood::Overheated);
        assert_eq!(determine_mood(20.0, 90.0, 20, 10), PetMood::Overheated);
        assert_eq!(determine_mood(10.0, 50.0, 90, 10), PetMood::Sick);
        assert_eq!(determine_mood(10.0, 50.0, 20, 95), PetMood::Sick);
        assert_eq!(determine_mood(75.0, 60.0, 20, 10), PetMood::Sprint);
    }

    #[test]
    fn test_isometric_8_states_frames() {
        let pet = PetStatus {
            species: PetSpecies::Ayam,
            name: "Ciko".to_string(),
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
            let frames = pet.frames_for_state(state);
            assert_eq!(frames.len(), 4);
            for f in frames {
                assert_eq!(f.len(), 6, "Each isometric frame must have 6 lines");
            }
        }
    }
}
