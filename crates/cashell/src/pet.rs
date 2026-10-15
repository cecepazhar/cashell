//! Virtual Pet (Tamagotchi) Engine for cashell cockpit.
//!
//! Provides pet lifecycle progression (Egg -> Hatch -> Juvenile -> Adult -> Mating -> Egg),
//! XP gain tracking, ASCII art sprites, and a frame-by-frame animation engine
//! for 5 species: Ayam, Singa, Burung, Ular, Beruang.

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
            Self::Ayam => "Ayam",
            Self::Singa => "Singa",
            Self::Burung => "Burung",
            Self::Ular => "Ular",
            Self::Beruang => "Beruang",
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
                Self::Ayam => "🐔",
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

/// The current physical/mental state of the pet, which dictates its animation frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PetMood {
    Idle,       // Normal, comfortable
    Overheated, // CPU temp > 80C or CPU > 85%
    Sick,       // RAM > 85% or Swap > 90%
    Sprint,     // High CPU load
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
            PetMood::Overheated => "🥵",
            PetMood::Sick => "💀",
            PetMood::Sprint => "🏃",
        };
        format!(
            "{} {} Lv.{} [{}]",
            self.species.emoji(&self.stage),
            self.name,
            level,
            mood_emoji
        )
    }

    /// Returns a reference to the frame array for the current mood and species.
    pub fn frames(&self) -> &'static [&'static [&'static str]] {
        match (self.species, self.mood) {
            // === AYAM (CHICKEN) ANIMATIONS ===
            (PetSpecies::Ayam, PetMood::Idle) => &AYAM_IDLE,
            (PetSpecies::Ayam, PetMood::Overheated) => &AYAM_OVERHEATED,
            (PetSpecies::Ayam, PetMood::Sick) => &AYAM_SICK,
            (PetSpecies::Ayam, PetMood::Sprint) => &AYAM_SPRINT,

            // === SINGA (LION) ANIMATIONS ===
            (PetSpecies::Singa, PetMood::Idle) => &SINGA_IDLE,
            (PetSpecies::Singa, PetMood::Overheated) => &SINGA_OVERHEATED,
            (PetSpecies::Singa, PetMood::Sick) => &SINGA_SICK,
            (PetSpecies::Singa, PetMood::Sprint) => &SINGA_SPRINT,

            // === BURUNG (BIRD) ANIMATIONS ===
            (PetSpecies::Burung, PetMood::Idle) => &BURUNG_IDLE,
            (PetSpecies::Burung, PetMood::Overheated) => &BURUNG_OVERHEATED,
            (PetSpecies::Burung, PetMood::Sick) => &BURUNG_SICK,
            (PetSpecies::Burung, PetMood::Sprint) => &BURUNG_SPRINT,

            // === ULAR (SNAKE) ANIMATIONS ===
            (PetSpecies::Ular, PetMood::Idle) => &ULAR_IDLE,
            (PetSpecies::Ular, PetMood::Overheated) => &ULAR_OVERHEATED,
            (PetSpecies::Ular, PetMood::Sick) => &ULAR_SICK,
            (PetSpecies::Ular, PetMood::Sprint) => &ULAR_SPRINT,

            // === BERUANG (BEAR) ANIMATIONS ===
            (PetSpecies::Beruang, PetMood::Idle) => &BERUANG_IDLE,
            (PetSpecies::Beruang, PetMood::Overheated) => &BERUANG_OVERHEATED,
            (PetSpecies::Beruang, PetMood::Sick) => &BERUANG_SICK,
            (PetSpecies::Beruang, PetMood::Sprint) => &BERUANG_SPRINT,
        }
    }
}

// ── AYAM (CHICKEN) HIGH-DEFINITION RETRO PIXEL ART ─────────────────────────
const AYAM_IDLE: [&[&str]; 4] = [
    &[
        "      ▄▄████▄▄       ",
        "     ██( ◕ ‿ ◕ )██   ",
        "    ╭███(  ▼  )███╮  ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
    &[
        "      ▄▄████▄▄       ",
        "     ██( ─ ‿ ─ )██   ",
        "    ╭███(  ▼  )███╮  ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
    &[
        "      ▄▄████▄▄       ",
        "    ╭██( ◕ ‿ ◕ )██╮  ",
        "     ███(  ▼  )███   ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
    &[
        "      ▄▄████▄▄       ",
        "     ██( ◕ ‿ ◕ )██   ",
        "    ╭███(  ▼  )███╮  ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
];

const AYAM_OVERHEATED: [&[&str]; 4] = [
    &[
        "   (✇) ▄▄████▄▄  💦 ",
        "    |\\██( > ﹏ < )██ ",
        "     ███(  ▼  )███   ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
    &[
        "       ▄▄████▄▄  💦💦",
        "   (✇)██( > ﹏ < )██ ",
        "    |\\██(  ▼  )███   ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
    &[
        "   (✇) ▄▄████▄▄  💦 ",
        "    |\\██( x ﹏ x )██ ",
        "     ███(  ▼  )███   ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
    &[
        "       ▄▄████▄▄  💦💦",
        "   (✇)██( > ﹏ < )██ ",
        "    |\\██(  ▼  )███   ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
];

const AYAM_SICK: [&[&str]; 4] = [
    &[
        "     [+]▄▄████▄▄  ~  ",
        "     ██( ◎ ﹏ ◎ )██  ",
        "     ███( 🤢 )███   ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
    &[
        "     [+]▄▄████▄▄  ~  ",
        "     ██( ─ ﹏ ─ )██  ",
        "     ███( 🤢 )███   ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
    &[
        "     [+]▄▄████▄▄  ~  ",
        "     ██( ◎ ﹏ ◎ )██  ",
        "     ███( 🤢 )███   ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
    &[
        "     [+]▄▄████▄▄  ~  ",
        "     ██( ─ ﹏ ─ )██  ",
        "     ███( 🤢 )███   ",
        "     ▀██████████▀    ",
        "       ▄▀     ▀▄     ",
    ],
];

const AYAM_SPRINT: [&[&str]; 4] = [
    &[
        "       ▄▄████▄▄ >>   ",
        "     ╭██( ◕ ‿ ◕ )██  ",
        "     ███(  ▼  )███   ",
        "      ▀██████████▀   ",
        "        ▄▀    ▀▄     ",
    ],
    &[
        "   >>  ▄▄████▄▄      ",
        "      ██( ◕ ‿ ◕ )██╮ ",
        "      ███(  ▼  )███  ",
        "      ▀██████████▀   ",
        "        ▀▄    ▄▀     ",
    ],
    &[
        "       ▄▄████▄▄ >>   ",
        "     ╭██( ◕ ‿ ◕ )██  ",
        "     ███(  ▼  )███   ",
        "      ▀██████████▀   ",
        "        ▄▀    ▀▄     ",
    ],
    &[
        "   >>  ▄▄████▄▄      ",
        "      ██( ◕ ‿ ◕ )██╮ ",
        "      ███(  ▼  )███  ",
        "      ▀██████████▀   ",
        "        ▀▄    ▄▀     ",
    ],
];

// ── SINGA (LION) HIGH-DEFINITION RETRO PIXEL ART ───────────────────────────
const SINGA_IDLE: [&[&str]; 4] = [
    &[
        "    ╭▓▓▓▓▓▓▓▓▓╮     ",
        "  ▓╭▓( ◕ ‿ ◕ )▓╮▓   ",
        "  ▓ ▀(   ▼   )▀ ▓  ",
        "    ╰▓▓▓▓▓▓▓▓▓╯     ",
    ],
    &[
        "    ╭▓▓▓▓▓▓▓▓▓╮     ",
        "  ▓╭▓( ─ ‿ ─ )▓╮▓   ",
        "  ▓ ▀(   ▼   )▀ ▓  ",
        "    ╰▓▓▓▓▓▓▓▓▓╯     ",
    ],
    &[
        "    ╭▓▓▓▓▓▓▓▓▓╮     ",
        "  ▓╭▓( ◕ ‿ ◕ )▓╮▓   ",
        "  ▓ ▀(  ▼▼  )▀ ▓   ",
        "    ╰▓▓▓▓▓▓▓▓▓╯     ",
    ],
    &[
        "    ╭▓▓▓▓▓▓▓▓▓╮     ",
        "  ▓╭▓( ◕ ‿ ◕ )▓╮▓   ",
        "  ▓ ▀(   ▼   )▀ ▓  ",
        "    ╰▓▓▓▓▓▓▓▓▓╯     ",
    ],
];

const SINGA_OVERHEATED: [&[&str]; 4] = [
    &[
        "  (✇) ╭▓▓▓▓▓▓▓▓▓╮ 💦 ",
        "  |\\ ▓╭▓( > ﹏ < )▓╮",
        "     ▓ ▀(   ▼   )▀ ",
        "     ╰▓▓▓▓▓▓▓▓▓╯   ",
    ],
    &[
        "     ╭▓▓▓▓▓▓▓▓▓╮ 💦💦",
        "  (✇)▓╭▓( > ﹏ < )▓╮",
        "   |\\▓ ▀(   ▼   )▀ ",
        "     ╰▓▓▓▓▓▓▓▓▓╯   ",
    ],
    &[
        "  (✇) ╭▓▓▓▓▓▓▓▓▓╮ 💦 ",
        "  |\\ ▓╭▓( x ﹏ x )▓╮",
        "     ▓ ▀(   ▼   )▀ ",
        "     ╰▓▓▓▓▓▓▓▓▓╯   ",
    ],
    &[
        "     ╭▓▓▓▓▓▓▓▓▓╮ 💦💦",
        "  (✇)▓╭▓( > ﹏ < )▓╮",
        "   |\\▓ ▀(   ▼   )▀ ",
        "     ╰▓▓▓▓▓▓▓▓▓╯   ",
    ],
];

const SINGA_SICK: [&[&str]; 4] = [
    &[
        "   [+]╭▓▓▓▓▓▓▓▓▓╮  ~ ",
        "     ▓╭▓( ◎ ﹏ ◎ )▓╮ ",
        "     ▓ ▀( 🤢 )▀ ▓   ",
        "     ╰▓▓▓▓▓▓▓▓▓╯    ",
    ],
    &[
        "   [+]╭▓▓▓▓▓▓▓▓▓╮  ~ ",
        "     ▓╭▓( ─ ﹏ ─ )▓╮ ",
        "     ▓ ▀( 🤢 )▀ ▓   ",
        "     ╰▓▓▓▓▓▓▓▓▓╯    ",
    ],
    &[
        "   [+]╭▓▓▓▓▓▓▓▓▓╮  ~ ",
        "     ▓╭▓( ◎ ﹏ ◎ )▓╮ ",
        "     ▓ ▀( 🤢 )▀ ▓   ",
        "     ╰▓▓▓▓▓▓▓▓▓╯    ",
    ],
    &[
        "   [+]╭▓▓▓▓▓▓▓▓▓╮  ~ ",
        "     ▓╭▓( ─ ﹏ ─ )▓╮ ",
        "     ▓ ▀( 🤢 )▀ ▓   ",
        "     ╰▓▓▓▓▓▓▓▓▓╯    ",
    ],
];

const SINGA_SPRINT: [&[&str]; 4] = [
    &[
        "     ╭▓▓▓▓▓▓▓▓▓╮ >> ",
        "   ▓╭▓( ◕ ‿ ◕ )▓╮▓  ",
        "   ▓ ▀(   ▼   )▀ ▓ ",
        "     ╰▓▓▓▓▓▓▓▓▓╯    ",
    ],
    &[
        " >>   ╭▓▓▓▓▓▓▓▓▓╮   ",
        "    ▓╭▓( ◕ ‿ ◕ )▓╮▓ ",
        "    ▓ ▀(   ▼   )▀ ▓",
        "      ╰▓▓▓▓▓▓▓▓▓╯  ",
    ],
    &[
        "     ╭▓▓▓▓▓▓▓▓▓╮ >> ",
        "   ▓╭▓( ◕ ‿ ◕ )▓╮▓  ",
        "   ▓ ▀(   ▼   )▀ ▓ ",
        "     ╰▓▓▓▓▓▓▓▓▓╯    ",
    ],
    &[
        " >>   ╭▓▓▓▓▓▓▓▓▓╮   ",
        "    ▓╭▓( ◕ ‿ ◕ )▓╮▓ ",
        "    ▓ ▀(   ▼   )▀ ▓",
        "      ╰▓▓▓▓▓▓▓▓▓╯  ",
    ],
];

// ── BURUNG (BIRD) HIGH-DEFINITION RETRO PIXEL ART ──────────────────────────
const BURUNG_IDLE: [&[&str]; 4] = [
    &[
        "      ╭▄▄▄▄▄╮       ",
        "    ◄( ◕ ‿ ◕ )►     ",
        "     ▀(  ▼  )▀      ",
        "      ▀▄▄▄▄▄▀       ",
    ],
    &[
        "      ╭▄▄▄▄▄╮       ",
        "    ◄( ─ ‿ ─ )►     ",
        "     ▀(  ▼  )▀      ",
        "      ▀▄▄▄▄▄▀       ",
    ],
    &[
        "     ▲╭▄▄▄▄▄╮▲      ",
        "    ◄( ◕ ‿ ◕ )►     ",
        "     ▀(  ▼  )▀      ",
        "      ▀▄▄▄▄▄▀       ",
    ],
    &[
        "      ╭▄▄▄▄▄╮       ",
        "    ◄( ◕ ‿ ◕ )►     ",
        "     ▀(  ▼  )▀      ",
        "      ▀▄▄▄▄▄▀       ",
    ],
];

const BURUNG_OVERHEATED: [&[&str]; 4] = [
    &[
        "  (✇) ╭▄▄▄▄▄╮ 💦    ",
        "  |\\◄( > ﹏ < )►    ",
        "    ▀(  ▼  )▀       ",
        "     ▀▄▄▄▄▄▀        ",
    ],
    &[
        "     ╭▄▄▄▄▄╮ 💦💦   ",
        "  (✇)◄( > ﹏ < )►   ",
        "  |\\ ▀(  ▼  )▀      ",
        "     ▀▄▄▄▄▄▀        ",
    ],
    &[
        "  (✇) ╭▄▄▄▄▄╮ 💦    ",
        "  |\\◄( x ﹏ x )►    ",
        "    ▀(  ▼  )▀       ",
        "     ▀▄▄▄▄▄▀        ",
    ],
    &[
        "     ╭▄▄▄▄▄╮ 💦💦   ",
        "  (✇)◄( > ﹏ < )►   ",
        "  |\\ ▀(  ▼  )▀      ",
        "     ▀▄▄▄▄▄▀        ",
    ],
];

const BURUNG_SICK: [&[&str]; 4] = [
    &[
        "    [+]╭▄▄▄▄▄╮ ~    ",
        "     ◄( ◎ ﹏ ◎ )►   ",
        "      ▀( 🤢 )▀      ",
        "       ▀▄▄▄▄▄▀      ",
    ],
    &[
        "    [+]╭▄▄▄▄▄╮ ~    ",
        "     ◄( ─ ﹏ ─ )►   ",
        "      ▀( 🤢 )▀      ",
        "       ▀▄▄▄▄▄▀      ",
    ],
    &[
        "    [+]╭▄▄▄▄▄╮ ~    ",
        "     ◄( ◎ ﹏ ◎ )►   ",
        "      ▀( 🤢 )▀      ",
        "       ▀▄▄▄▄▄▀      ",
    ],
    &[
        "    [+]╭▄▄▄▄▄╮ ~    ",
        "     ◄( ─ ﹏ ─ )►   ",
        "      ▀( 🤢 )▀      ",
        "       ▀▄▄▄▄▄▀      ",
    ],
];

const BURUNG_SPRINT: [&[&str]; 4] = [
    &[
        "     ▲╭▄▄▄▄▄╮▲ >>   ",
        "    ◄( ◕ ‿ ◕ )►     ",
        "     ▀(  ▼  )▀      ",
        "      ▀▄▄▄▄▄▀       ",
    ],
    &[
        " >>   ▲╭▄▄▄▄▄╮▲     ",
        "     ◄( ◕ ‿ ◕ )►    ",
        "      ▀(  ▼  )▀     ",
        "       ▀▄▄▄▄▄▀      ",
    ],
    &[
        "     ▲╭▄▄▄▄▄╮▲ >>   ",
        "    ◄( ◕ ‿ ◕ )►     ",
        "     ▀(  ▼  )▀      ",
        "      ▀▄▄▄▄▄▀       ",
    ],
    &[
        " >>   ▲╭▄▄▄▄▄╮▲     ",
        "     ◄( ◕ ‿ ◕ )►    ",
        "      ▀(  ▼  )▀     ",
        "       ▀▄▄▄▄▄▀      ",
    ],
];

// ── ULAR (SNAKE) HIGH-DEFINITION RETRO PIXEL ART ────────────────────────────
const ULAR_IDLE: [&[&str]; 4] = [
    &[
        "       ▄▄████▄▄      ",
        "      ██( ◕ ‿ ◕ )██  ",
        "     ╭████( ω )████╮ ",
        "    ╰██████████████╯~",
    ],
    &[
        "       ▄▄████▄▄      ",
        "      ██( ─ ‿ ─ )██  ",
        "     ╭████( ω )████╮ ",
        "    ╰██████████████╯~",
    ],
    &[
        "       ▄▄████▄▄      ",
        "      ██( ◕ ‿ ◕ )██  ",
        "     ╭████( ᴥ )████╮ ",
        "   ~~╰██████████████╯",
    ],
    &[
        "       ▄▄████▄▄      ",
        "      ██( ◕ ‿ ◕ )██  ",
        "     ╭████( ω )████╮ ",
        "    ╰██████████████╯~",
    ],
];

const ULAR_OVERHEATED: [&[&str]; 4] = [
    &[
        "   (✇) ▄▄████▄▄  💦  ",
        "   |\\ ██( > ﹏ < )██ ",
        "     ╭████( ═ )████╮ ",
        "    ╰██████████████╯~",
    ],
    &[
        "       ▄▄████▄▄  💦💦",
        "   (✇)██( > ﹏ < )██ ",
        "   |\\╭████( ═ )████╮ ",
        "    ╰██████████████╯~",
    ],
    &[
        "   (✇) ▄▄████▄▄  💦  ",
        "   |\\ ██( x ﹏ x )██ ",
        "     ╭████( ═ )████╮ ",
        "    ╰██████████████╯~",
    ],
    &[
        "       ▄▄████▄▄  💦💦",
        "   (✇)██( > ﹏ < )██ ",
        "   |\\╭████( ═ )████╮ ",
        "    ╰██████████████╯~",
    ],
];

const ULAR_SICK: [&[&str]; 4] = [
    &[
        "    [+]▄▄████▄▄  ~   ",
        "      ██( ◎ ﹏ ◎ )██ ",
        "     ╭████( 🤢 )████╮",
        "    ╰██████████████╯ ",
    ],
    &[
        "    [+]▄▄████▄▄  ~   ",
        "      ██( ─ ﹏ ─ )██ ",
        "     ╭████( 🤢 )████╮",
        "    ╰██████████████╯ ",
    ],
    &[
        "    [+]▄▄████▄▄  ~   ",
        "      ██( ◎ ﹏ ◎ )██ ",
        "     ╭████( 🤢 )████╮",
        "    ╰██████████████╯ ",
    ],
    &[
        "    [+]▄▄████▄▄  ~   ",
        "      ██( ─ ﹏ ─ )██ ",
        "     ╭████( 🤢 )████╮",
        "    ╰██████████████╯ ",
    ],
];

const ULAR_SPRINT: [&[&str]; 4] = [
    &[
        "       ▄▄████▄▄ >>   ",
        "      ██( ◕ ‿ ◕ )██  ",
        "     ╭████( ω )████╮ ",
        "   >╰██████████████╯>",
    ],
    &[
        "   >>  ▄▄████▄▄      ",
        "      ██( ◕ ‿ ◕ )██  ",
        "     ╭████( ω )████╮ ",
        "   >╰██████████████╯>",
    ],
    &[
        "       ▄▄████▄▄ >>   ",
        "      ██( ◕ ‿ ◕ )██  ",
        "     ╭████( ω )████╮ ",
        "   >╰██████████████╯>",
    ],
    &[
        "   >>  ▄▄████▄▄      ",
        "      ██( ◕ ‿ ◕ )██  ",
        "     ╭████( ω )████╮ ",
        "   >╰██████████████╯>",
    ],
];

// ── BERUANG (BEAR) HIGH-DEFINITION RETRO PIXEL ART ──────────────────────────
const BERUANG_IDLE: [&[&str]; 4] = [
    &[
        "     ╭(o)━━━━(o)╮    ",
        "    ███( ◕ ‿ ◕ )███  ",
        "   ╭████(  ●  )████╮ ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        "     ╭(o)━━━━(o)╮    ",
        "    ███( ─ ‿ ─ )███  ",
        "   ╭████(  ●  )████╮ ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        "     ╭(o)━━━━(o)╮    ",
        "    ███( ◕ ‿ ◕ )███  ",
        "   ╭████(  ᴥ  )████╮ ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        "     ╭(o)━━━━(o)╮    ",
        "    ███( ◕ ‿ ◕ )███  ",
        "   ╭████(  ●  )████╮ ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
];

const BERUANG_OVERHEATED: [&[&str]; 4] = [
    &[
        "  (✇)╭(o)━━━━(o)╮ 💦 ",
        "  |\\███( > ﹏ < )███ ",
        "   ╭████(  ●  )████╮ ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        "     ╭(o)━━━━(o)╮ 💦💦",
        "  (✇)███( > ﹏ < )███ ",
        "  |\\╭████(  ●  )████╮",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        "  (✇)╭(o)━━━━(o)╮ 💦 ",
        "  |\\███( x ﹏ x )███ ",
        "   ╭████(  ●  )████╮ ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        "     ╭(o)━━━━(o)╮ 💦💦",
        "  (✇)███( > ﹏ < )███ ",
        "  |\\╭████(  ●  )████╮",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
];

const BERUANG_SICK: [&[&str]; 4] = [
    &[
        "   [+]╭(o)━━━━(o)╮ ~ ",
        "    ███( ◎ ﹏ ◎ )███ ",
        "   ╭████( 🤢 )████╮  ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        "   [+]╭(o)━━━━(o)╮ ~ ",
        "    ███( ─ ﹏ ─ )███ ",
        "   ╭████( 🤢 )████╮  ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        "   [+]╭(o)━━━━(o)╮ ~ ",
        "    ███( ◎ ﹏ ◎ )███ ",
        "   ╭████( 🤢 )████╮  ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        "   [+]╭(o)━━━━(o)╮ ~ ",
        "    ███( ─ ﹏ ─ )███ ",
        "   ╭████( 🤢 )████╮  ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
];

const BERUANG_SPRINT: [&[&str]; 4] = [
    &[
        "     ╭(o)━━━━(o)╮ >> ",
        "    ███( ◕ ‿ ◕ )███  ",
        "   ╭████(  ●  )████╮ ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        " >>  ╭(o)━━━━(o)╮    ",
        "    ███( ◕ ‿ ◕ )███  ",
        "   ╭████(  ●  )████╮ ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        "     ╭(o)━━━━(o)╮ >> ",
        "    ███( ◕ ‿ ◕ )███  ",
        "   ╭████(  ●  )████╮ ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
    ],
    &[
        " >>  ╭(o)━━━━(o)╮    ",
        "    ███( ◕ ‿ ◕ )███  ",
        "   ╭████(  ●  )████╮ ",
        "    ▀████████████▀   ",
        "      ▀▄▄    ▄▄▀     ",
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
}
