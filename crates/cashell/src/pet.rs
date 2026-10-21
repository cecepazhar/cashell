//! Virtual Pet (Tamagotchi) Engine for CAShell Cockpit.
//!
//! Provides pet lifecycle progression (Egg -> Hatch -> Juvenile -> Adult -> Mating -> Egg),
//! XP gain tracking, and a highly-reactive **Single-Core Emoji Animation Engine**.
//!
//! Instead of complex multi-line ASCII pixel art that risks terminal width fragmentation,
//! this engine renders a single, vibrant Core Emoji (`🐉`) equipped with 8 dynamic
//! movement offsets and reactive particle trails.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PetSpecies {
    Nagini,
}

impl PetSpecies {
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "nagini" | "dragon" | "ular" | "snake" => Self::Nagini,
            _ => Self::Nagini, // Default to Nagini
        }
    }

    pub fn display_name(&self) -> &'static str {
        "Nagini"
    }

    pub fn core_emoji(&self) -> &'static str {
        "🐉"
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

    /// The emoji used to represent the current lifecycle stage
    pub fn stage_emoji(&self) -> &'static str {
        match self {
            Self::Egg => "🥚",
            Self::Hatch => "🐣",
            Self::Juvenile => "🐲",
            Self::Adult => "🐉",
            Self::Mating => "💑",
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

/// 8-state dynamic reaction state system of the single-core emoji mascot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PetActionState {
    Idle,     // Calm resting, gentle sway
    Walk,     // Slight horizontal pacing
    Run,      // High-speed sprint
    Jump,     // Vertical jumping
    Attack,   // Forward lunge strike
    Dash,     // Speed-blur teleport
    Hit,      // Vibration / flinch impact
    Death,    // Dimmed sleeping / exhausted
}

impl PetActionState {
    pub fn from_mood(mood: PetMood) -> Self {
        match mood {
            PetMood::Idle => Self::Idle,
            PetMood::Overheated => Self::Hit,
            PetMood::Sick => Self::Death,
            PetMood::Sprint => Self::Run,
        }
    }
}

/// A pet action log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetActionLog {
    pub action_type: String,
    pub detail: String,
    pub timestamp: String,
}

/// Tamagotchi-style single-emoji animated pet model.
#[derive(Debug, Clone)]
pub struct PetStatus {
    pub species: PetSpecies,
    pub name: String,
    pub stage: PetStage,
    pub mood: PetMood,
    pub xp: i64,
}

impl PetStatus {
    /// The single reactive frame to render for a given action state.
    /// Returns a struct containing the Core Emoji, the Particle Trail string,
    /// and the X/Y offsets for dynamic positioning.
    pub fn build_frame(&self, state: PetActionState, tick: usize, _is_stopped: bool) -> EmojiFrame {
        let tick = tick % 120;
        let core = self.species.core_emoji();

        match state {
            PetActionState::Idle => {
                // Gentle breathing: 1px up/down shift, blinking every 4th cycle
                let y_offset = if tick % 8 < 4 { 0 } else { 1 };
                let particles = if tick % 64 >= 60 { "·" } else { "" };
                EmojiFrame {
                    core: core.to_string(),
                    particles: particles.to_string(),
                    x_offset: 0,
                    y_offset,
                    state_label: "IDLE".to_string(),
                }
            }
            PetActionState::Walk => {
                // Pacing side-to-side within a 4-slot arena
                let step = tick % 32;
                let x_offset = match step {
                    0..=7 => -2,
                    8..=15 => 0,
                    16..=23 => 2,
                    _ => 0,
                };
                let particles = " . ";
                EmojiFrame {
                    core: core.to_string(),
                    particles: particles.to_string(),
                    x_offset,
                    y_offset: 0,
                    state_label: "WALK".to_string(),
                }
            }
            PetActionState::Run => {
                // High-speed horizontal sprint with speed-line wind trails
                let step = tick % 16;
                let x_offset = match step {
                    0..=3 => -4,
                    4..=7 => 0,
                    8..=11 => 4,
                    _ => 0,
                };
                EmojiFrame {
                    core: core.to_string(),
                    particles: "彡彡彡".to_string(),
                    x_offset,
                    y_offset: 0,
                    state_label: "RUN >>".to_string(),
                }
            }
            PetActionState::Jump => {
                // Vertical jumping with motion blur
                let step = tick % 12;
                let y_offset = match step {
                    0..=2 => 0,
                    3..=5 => -1,
                    6..=8 => 0,
                    _ => 1,
                };
                let particles = if y_offset == -1 { "✨" } else { "" };
                EmojiFrame {
                    core: core.to_string(),
                    particles: particles.to_string(),
                    x_offset: 0,
                    y_offset,
                    state_label: "JUMP ▲".to_string(),
                }
            }
            PetActionState::Attack => {
                // Forward lunge with lightning energy strikes
                let step = tick % 8;
                let x_offset = if step < 4 { 1 } else { 0 };
                EmojiFrame {
                    core: core.to_string(),
                    particles: "⚡".to_string(),
                    x_offset,
                    y_offset: 0,
                    state_label: "ATTACK".to_string(),
                }
            }
            PetActionState::Dash => {
                // Teleport-style extreme blur movement
                let step = tick % 10;
                let x_offset = match step {
                    0..=2 => -5,
                    3..=4 => 0,
                    5..=7 => 5,
                    _ => 0,
                };
                EmojiFrame {
                    core: core.to_string(),
                    particles: "〰〰〰".to_string(),
                    x_offset,
                    y_offset: 0,
                    state_label: "DASH".to_string(),
                }
            }
            PetActionState::Hit => {
                // Vibration / flinch impact + red pulsing alerts
                let step = tick % 4;
                let x_offset = if step == 0 { -1 } else if step == 2 { 1 } else { 0 };
                EmojiFrame {
                    core: core.to_string(),
                    particles: "!!".to_string(),
                    x_offset,
                    y_offset: 0,
                    state_label: "ALERT".to_string(),
                }
            }
            PetActionState::Death => {
                // Dimmed sleep state with floating dreams
                let step = tick % 16;
                let y_offset = if step < 8 { 0 } else { 1 };
                let particles = "💤".to_string();
                EmojiFrame {
                    core: self.stage.stage_emoji().to_string(),
                    particles,
                    x_offset: 0,
                    y_offset,
                    state_label: "REST / ZZZ".to_string(),
                }
            }
        }
    }

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
            self.stage.stage_emoji(),
            self.name,
            level,
            mood_emoji
        )
    }
}

/// Output struct for the dynamic single-emoji frame.
#[derive(Debug, Clone, Default)]
pub struct EmojiFrame {
    pub core: String,
    pub particles: String,
    pub x_offset: i32,
    pub y_offset: i32,
    pub state_label: String,
}

/// Determines the PetMood based on hardware telemetry.
pub fn determine_mood(cpu_pct: f32, cpu_temp_c: f32, ram_pct: u8, swap_pct: u8) -> PetMood {
    if cpu_temp_c > 80.0 || cpu_pct > 85.0 {
        PetMood::Overheated
    } else if ram_pct >= 85 || swap_pct >= 90 {
        PetMood::Sick
    } else if cpu_pct > 65.0 {
        PetMood::Sprint
    } else {
        PetMood::Idle
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
            species: PetSpecies::Nagini,
            name: "Nagini".to_string(),
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
}
