//! Virtual Pet Engine for CAShell Cockpit — Sovereign Cyberpunk Token.
//!
//! Replaces legacy multi-animal sprites with the **Cyberpunk Isometric Sovereign Token**:
//!   • 3D Extruded Cylinder Token with glowing neon phosphor aesthetic (#00FF44 / #39FF14)
//!   • Stable sine-wave hovering / floating bobbing (melayang stabil)
//!   • 100 discrete condition-responsive expressions mapped directly to CAShell telemetry
//!   • Dynamic Matrix Digital Rain background (toggleable in Settings)
//!   • Sovereign-themed action states ([⚡ Overclock], [🛡 Audit], [🌀 Purge], [🪙 Spin])

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde::{Deserialize, Serialize};

// ── Species ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PetSpecies {
    Sovereign,
}

impl PetSpecies {
    pub fn parse(_s: &str) -> Self {
        Self::Sovereign
    }
    pub fn display_name(&self) -> &'static str {
        "Sovereign"
    }
}

// ── Stage / Progression ──────────────────────────────────────────────────────

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
            Self::Egg => "Genesis Egg",
            Self::Hatch => "Booting Node",
            Self::Juvenile => "Sovereign Miner",
            Self::Adult => "Consensus Leader",
            Self::Mating => "Armada Overseer",
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
    pub fn stage_emoji(&self) -> &'static str {
        match self {
            Self::Egg => "🪙",
            Self::Hatch => "⚡",
            Self::Juvenile => "💎",
            Self::Adult => "🕶",
            Self::Mating => "👑",
        }
    }
}

// ── Mood & Hardware Condition ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PetMood {
    Idle,
    Overheated,
    Sick,
    Sprint,
}

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

// ── Action State ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PetActionState {
    Idle,
    Walk,
    Run,
    Jump,
    Attack,
    Dash,
    Hit,
    Death,
}

impl PetActionState {
    pub fn from_mood(m: PetMood) -> Self {
        match m {
            PetMood::Idle => Self::Idle,
            PetMood::Overheated => Self::Hit,
            PetMood::Sick => Self::Death,
            PetMood::Sprint => Self::Run,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetActionLog {
    pub action_type: String,
    pub detail: String,
    pub timestamp: String,
}

#[derive(Debug, Clone)]
pub struct PetStatus {
    pub species: PetSpecies,
    pub name: String,
    pub stage: PetStage,
    pub mood: PetMood,
    pub xp: i64,
}

#[derive(Debug, Clone)]
pub struct EmojiFrame {
    pub core: String,
    pub particles: String,
    pub x_offset: i32,
    pub y_offset: i32,
    pub state_label: String,
}

// ── Mascot Telemetry Input for Expression Selection ──────────────────────────

#[derive(Debug, Clone, Default)]
pub struct MascotTelemetry {
    pub cpu_pct: f32,
    pub cpu_temp_c: f32,
    pub ram_pct: u8,
    pub swap_pct: u8,
    pub containers_down: usize,
    pub fleet_nodes_down: usize,
    pub uptime_monitors_down: usize,
    pub git_dirty_count: usize,
    pub turbo_active: bool,
    pub transient_action: Option<PetActionState>,
    pub xp_level: usize,
    pub tick: usize,
}

// ── 100 Discrete Sovereign Expressions ───────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct MascotExpression {
    pub id: usize,
    pub archetype: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub has_glasses: bool,
    pub mouth_type: u8,   // 0=smile, 1=smirk_left, 2=smirk_right, 3=grin, 4=laugh, 5=tongue, 6=frown, 7=open_o, 8=flat, 9=zigzag
    pub eye_type: u8,     // 0=shades, 1=wink_left, 2=wink_right, 3=normal, 4=squint, 5=angry, 6=surprised, 7=sleep, 8=matrix, 9=crosshair
    pub brow_type: u8,    // 0=none, 1=raised, 2=angry, 3=skeptical, 4=worried
}

/// Static array of 100 distinct Sovereign expressions.
/// 10 Archetypes × 10 Intensity Tiers = 100 Unique Visual Configurations.
pub const SOVEREIGN_EXPRESSIONS: [MascotExpression; 100] = [
    // 00..09: SOVEREIGN SHADES (Cool with Cyber Sunglasses)
    MascotExpression { id: 0, archetype: "SHADES", label: "Classic Shades", desc: "Balanced steady operations", has_glasses: true, mouth_type: 0, eye_type: 0, brow_type: 0 },
    MascotExpression { id: 1, archetype: "SHADES", label: "Smirk Shades", desc: "All systems nominal & secure", has_glasses: true, mouth_type: 1, eye_type: 0, brow_type: 0 },
    MascotExpression { id: 2, archetype: "SHADES", label: "Wide Shades", desc: "Fleet heartbeat synchronized", has_glasses: true, mouth_type: 3, eye_type: 0, brow_type: 0 },
    MascotExpression { id: 3, archetype: "SHADES", label: "Right Smirk Shades", desc: "Zero packet loss on mesh", has_glasses: true, mouth_type: 2, eye_type: 0, brow_type: 0 },
    MascotExpression { id: 4, archetype: "SHADES", label: "Stoic Shades", desc: "Quiescent background watcher", has_glasses: true, mouth_type: 8, eye_type: 0, brow_type: 0 },
    MascotExpression { id: 5, archetype: "SHADES", label: "Gleam Shades", desc: "Fast RPC responses <5ms", has_glasses: true, mouth_type: 0, eye_type: 0, brow_type: 1 },
    MascotExpression { id: 6, archetype: "SHADES", label: "Laughing Shades", desc: "Zero compiler warnings achieved", has_glasses: true, mouth_type: 4, eye_type: 0, brow_type: 0 },
    MascotExpression { id: 7, archetype: "SHADES", label: "Tongue Shades", desc: "Playful overclock burst", has_glasses: true, mouth_type: 5, eye_type: 0, brow_type: 0 },
    MascotExpression { id: 8, archetype: "SHADES", label: "Firm Shades", desc: "Heavy database transactions clean", has_glasses: true, mouth_type: 8, eye_type: 0, brow_type: 2 },
    MascotExpression { id: 9, archetype: "SHADES", label: "Overdrive Shades", desc: "Consensus leader operating at peak", has_glasses: true, mouth_type: 3, eye_type: 0, brow_type: 1 },

    // 10..19: SOVEREIGN WINK (Confident & Playful)
    MascotExpression { id: 10, archetype: "WINK", label: "Left Wink", desc: "Confident node handshake", has_glasses: false, mouth_type: 0, eye_type: 1, brow_type: 0 },
    MascotExpression { id: 11, archetype: "WINK", label: "Right Wink", desc: "Tailscale mesh tunnel established", has_glasses: false, mouth_type: 0, eye_type: 2, brow_type: 0 },
    MascotExpression { id: 12, archetype: "WINK", label: "Left Wink Smirk", desc: "Smooth SSL cert auto-renewal", has_glasses: false, mouth_type: 1, eye_type: 1, brow_type: 0 },
    MascotExpression { id: 13, archetype: "WINK", label: "Right Wink Smirk", desc: "Local SQLite WAL committed", has_glasses: false, mouth_type: 2, eye_type: 2, brow_type: 0 },
    MascotExpression { id: 14, archetype: "WINK", label: "Wink Tongue", desc: "Developer treat action received", has_glasses: false, mouth_type: 5, eye_type: 1, brow_type: 0 },
    MascotExpression { id: 15, archetype: "WINK", label: "Wink Open Mouth", desc: "Fast build completion signal", has_glasses: false, mouth_type: 4, eye_type: 2, brow_type: 1 },
    MascotExpression { id: 16, archetype: "WINK", label: "Wink Brow Raised", desc: "Curious telemetry check", has_glasses: false, mouth_type: 1, eye_type: 1, brow_type: 1 },
    MascotExpression { id: 17, archetype: "WINK", label: "Wink Grin", desc: "All 11 cockpit tabs nominal", has_glasses: false, mouth_type: 3, eye_type: 2, brow_type: 0 },
    MascotExpression { id: 18, archetype: "WINK", label: "Wink Fast Pulse", desc: "Mesh packet ping incoming", has_glasses: false, mouth_type: 0, eye_type: 1, brow_type: 3 },
    MascotExpression { id: 19, archetype: "WINK", label: "Sovereign Wink Master", desc: "Decentralized auth confirmed", has_glasses: false, mouth_type: 3, eye_type: 2, brow_type: 1 },

    // 20..29: SOVEREIGN GRIN (Cheerful Nominal)
    MascotExpression { id: 20, archetype: "GRIN", label: "Cheerful Smile", desc: "System running smoothly", has_glasses: false, mouth_type: 0, eye_type: 3, brow_type: 0 },
    MascotExpression { id: 21, archetype: "GRIN", label: "Wide Grin", desc: "All armada nodes responding", has_glasses: false, mouth_type: 3, eye_type: 3, brow_type: 0 },
    MascotExpression { id: 22, archetype: "GRIN", label: "High Brow Grin", desc: "Uptime monitor at 100.0%", has_glasses: false, mouth_type: 3, eye_type: 3, brow_type: 1 },
    MascotExpression { id: 23, archetype: "GRIN", label: "Gentle Beam", desc: "Git repository clean & synced", has_glasses: false, mouth_type: 0, eye_type: 3, brow_type: 0 },
    MascotExpression { id: 24, archetype: "GRIN", label: "Contented Glow", desc: "Coolify deployment healthy", has_glasses: false, mouth_type: 0, eye_type: 3, brow_type: 1 },
    MascotExpression { id: 25, archetype: "GRIN", label: "Bright Grin", desc: "Zero deadlocks on SQLite", has_glasses: false, mouth_type: 3, eye_type: 3, brow_type: 0 },
    MascotExpression { id: 26, archetype: "GRIN", label: "Broad Smile", desc: "Fast NVMe disk write speeds", has_glasses: false, mouth_type: 3, eye_type: 3, brow_type: 1 },
    MascotExpression { id: 27, archetype: "GRIN", label: "Subtle Grin", desc: "Autonomous daemon loop nominal", has_glasses: false, mouth_type: 1, eye_type: 3, brow_type: 0 },
    MascotExpression { id: 28, archetype: "GRIN", label: "Radiant Grin", desc: "Hashrate XP milestone reached", has_glasses: false, mouth_type: 3, eye_type: 3, brow_type: 1 },
    MascotExpression { id: 29, archetype: "GRIN", label: "Sovereign Apex Grin", desc: "Complete ecosystem sovereignty", has_glasses: false, mouth_type: 3, eye_type: 3, brow_type: 1 },

    // 30..39: SOVEREIGN LAUGH (High XP / Ecstatic)
    MascotExpression { id: 30, archetype: "LAUGH", label: "Squint Laugh", desc: "Audit score 100/100 perfect", has_glasses: false, mouth_type: 4, eye_type: 4, brow_type: 0 },
    MascotExpression { id: 31, archetype: "LAUGH", label: "Triumphant Laugh", desc: "Release binary successfully deployed", has_glasses: false, mouth_type: 4, eye_type: 4, brow_type: 1 },
    MascotExpression { id: 32, archetype: "LAUGH", label: "Joyous Chortle", desc: "Multi-tenant sync completed", has_glasses: false, mouth_type: 4, eye_type: 4, brow_type: 0 },
    MascotExpression { id: 33, archetype: "LAUGH", label: "Beaming Laugh", desc: "Pro E2EE state fully synchronized", has_glasses: false, mouth_type: 4, eye_type: 3, brow_type: 1 },
    MascotExpression { id: 34, archetype: "LAUGH", label: "Guffaw Joy", desc: "Zero runtime panics detected", has_glasses: false, mouth_type: 4, eye_type: 4, brow_type: 1 },
    MascotExpression { id: 35, archetype: "LAUGH", label: "Sparkle Laugh", desc: "Analytics visitors traffic spike", has_glasses: false, mouth_type: 4, eye_type: 4, brow_type: 0 },
    MascotExpression { id: 36, archetype: "LAUGH", label: "Belly Laugh", desc: "Fastest benchmark test recorded", has_glasses: false, mouth_type: 4, eye_type: 4, brow_type: 1 },
    MascotExpression { id: 37, archetype: "LAUGH", label: "Ecstatic Beam", desc: "Memory cache cleared & freed", has_glasses: false, mouth_type: 4, eye_type: 3, brow_type: 1 },
    MascotExpression { id: 38, archetype: "LAUGH", label: "Consensus Laugh", desc: "All fleet daemons validated", has_glasses: false, mouth_type: 4, eye_type: 4, brow_type: 1 },
    MascotExpression { id: 39, archetype: "LAUGH", label: "Sovereign Jubilation", desc: "Sovereign developer supreme state", has_glasses: false, mouth_type: 4, eye_type: 4, brow_type: 1 },

    // 40..49: SOVEREIGN TONGUE (Playful Blep)
    MascotExpression { id: 40, archetype: "TONGUE", label: "Center Blep", desc: "Overclock treat consumed", has_glasses: false, mouth_type: 5, eye_type: 3, brow_type: 0 },
    MascotExpression { id: 41, archetype: "TONGUE", label: "Wink Blep Left", desc: "Playful cache purge triggered", has_glasses: false, mouth_type: 5, eye_type: 1, brow_type: 0 },
    MascotExpression { id: 42, archetype: "TONGUE", label: "Wink Blep Right", desc: "Interactive button click detected", has_glasses: false, mouth_type: 5, eye_type: 2, brow_type: 0 },
    MascotExpression { id: 43, archetype: "TONGUE", label: "Squint Blep", desc: "Spin token action executed", has_glasses: false, mouth_type: 5, eye_type: 4, brow_type: 0 },
    MascotExpression { id: 44, archetype: "TONGUE", label: "Cheeky Tongue", desc: "Testing Docker container restart", has_glasses: false, mouth_type: 5, eye_type: 3, brow_type: 1 },
    MascotExpression { id: 45, archetype: "TONGUE", label: "Derp Blep", desc: "Temporary dev port test", has_glasses: false, mouth_type: 5, eye_type: 3, brow_type: 3 },
    MascotExpression { id: 46, archetype: "TONGUE", label: "Happy Tongue", desc: "Bonus XP points accrued", has_glasses: false, mouth_type: 5, eye_type: 3, brow_type: 1 },
    MascotExpression { id: 47, archetype: "TONGUE", label: "Surprised Blep", desc: "Unexpected fast response", has_glasses: false, mouth_type: 5, eye_type: 6, brow_type: 1 },
    MascotExpression { id: 48, archetype: "TONGUE", label: "Matrix Blep", desc: "Digital token playful mood", has_glasses: false, mouth_type: 5, eye_type: 8, brow_type: 0 },
    MascotExpression { id: 49, archetype: "TONGUE", label: "Sovereign Cheeky Blep", desc: "Mascot sovereign energy high", has_glasses: false, mouth_type: 5, eye_type: 3, brow_type: 1 },

    // 50..59: SOVEREIGN ALERT (Vigilant & Focused)
    MascotExpression { id: 50, archetype: "ALERT", label: "Watchful Gaze", desc: "Network latency elevation >60ms", has_glasses: false, mouth_type: 8, eye_type: 3, brow_type: 3 },
    MascotExpression { id: 51, archetype: "ALERT", label: "Narrow Eye Focus", desc: "Uncommitted git changes pending", has_glasses: false, mouth_type: 8, eye_type: 4, brow_type: 3 },
    MascotExpression { id: 52, archetype: "ALERT", label: "Scanning Eyes", desc: "Fleet heartbeat telemetry probe", has_glasses: false, mouth_type: 8, eye_type: 3, brow_type: 1 },
    MascotExpression { id: 53, archetype: "ALERT", label: "Skeptical Brow", desc: "Single container restart detected", has_glasses: false, mouth_type: 1, eye_type: 3, brow_type: 3 },
    MascotExpression { id: 54, archetype: "ALERT", label: "Crosshair Scan", desc: "Audit vulnerability scan running", has_glasses: false, mouth_type: 8, eye_type: 9, brow_type: 2 },
    MascotExpression { id: 55, archetype: "ALERT", label: "Guarded Stance", desc: "SSL cert expiration approaching", has_glasses: false, mouth_type: 6, eye_type: 3, brow_type: 2 },
    MascotExpression { id: 56, archetype: "ALERT", label: "Vigilant Guard", desc: "Foreign SSH handshake blocked", has_glasses: false, mouth_type: 8, eye_type: 3, brow_type: 2 },
    MascotExpression { id: 57, archetype: "ALERT", label: "Intense Audit", desc: "Codebase review in progress", has_glasses: false, mouth_type: 8, eye_type: 9, brow_type: 3 },
    MascotExpression { id: 58, archetype: "ALERT", label: "Perimeter Alert", desc: "Elevated network traffic detected", has_glasses: false, mouth_type: 6, eye_type: 3, brow_type: 3 },
    MascotExpression { id: 59, archetype: "ALERT", label: "Sovereign Sentry", desc: "24/7 autonomous cockpit vigilance", has_glasses: false, mouth_type: 8, eye_type: 9, brow_type: 2 },

    // 60..69: SOVEREIGN ANGRY / OVERHEATED (High Load & Temperature)
    MascotExpression { id: 60, archetype: "ANGRY", label: "Annoyed Frown", desc: "CPU temp reaching 75°C", has_glasses: false, mouth_type: 6, eye_type: 5, brow_type: 2 },
    MascotExpression { id: 61, archetype: "ANGRY", label: "Furious Glare", desc: "CPU load exceeds 85%", has_glasses: false, mouth_type: 6, eye_type: 5, brow_type: 2 },
    MascotExpression { id: 62, archetype: "ANGRY", label: "Thermal Throttle", desc: "Core temperature >80°C threshold", has_glasses: false, mouth_type: 9, eye_type: 5, brow_type: 2 },
    MascotExpression { id: 63, archetype: "ANGRY", label: "RAM Pressure", desc: "System memory utilized >85%", has_glasses: false, mouth_type: 6, eye_type: 5, brow_type: 2 },
    MascotExpression { id: 64, archetype: "ANGRY", label: "Redline Overheat", desc: "Heavy multi-core compilation burn", has_glasses: false, mouth_type: 9, eye_type: 5, brow_type: 2 },
    MascotExpression { id: 65, archetype: "ANGRY", label: "Steaming Core", desc: "High fan RPM on ThinkPad X1", has_glasses: false, mouth_type: 9, eye_type: 5, brow_type: 2 },
    MascotExpression { id: 66, archetype: "ANGRY", label: "Swap Stress", desc: "Linux ZRAM swap active >80%", has_glasses: false, mouth_type: 6, eye_type: 5, brow_type: 2 },
    MascotExpression { id: 67, archetype: "ANGRY", label: "Intense Scowl", desc: "Docker I/O bottleneck encountered", has_glasses: false, mouth_type: 6, eye_type: 5, brow_type: 2 },
    MascotExpression { id: 68, archetype: "ANGRY", label: "Overclock Blaze", desc: "Max TDP PL2 burst sustained", has_glasses: false, mouth_type: 9, eye_type: 5, brow_type: 2 },
    MascotExpression { id: 69, archetype: "ANGRY", label: "Sovereign Fury", desc: "Hardware boundary stress test", has_glasses: false, mouth_type: 9, eye_type: 5, brow_type: 2 },

    // 70..79: SOVEREIGN SURPRISED (Node Timeout / Anomaly)
    MascotExpression { id: 70, archetype: "SURPRISED", label: "Wide Eyes O", desc: "Uptime monitor returned 5xx", has_glasses: false, mouth_type: 7, eye_type: 6, brow_type: 1 },
    MascotExpression { id: 71, archetype: "SURPRISED", label: "Gasp Shock", desc: "Remote node unreachable timeout", has_glasses: false, mouth_type: 7, eye_type: 6, brow_type: 1 },
    MascotExpression { id: 72, archetype: "SURPRISED", label: "Dropped Jaw", desc: "Docker container unexpectedly exited", has_glasses: false, mouth_type: 7, eye_type: 6, brow_type: 1 },
    MascotExpression { id: 73, archetype: "SURPRISED", label: "Asymmetric Shock", desc: "Disk partition capacity >90%", has_glasses: false, mouth_type: 7, eye_type: 6, brow_type: 3 },
    MascotExpression { id: 74, archetype: "SURPRISED", label: "Blink Gasp", desc: "Network interface reconnected", has_glasses: false, mouth_type: 7, eye_type: 1, brow_type: 1 },
    MascotExpression { id: 75, archetype: "SURPRISED", label: "Sudden Alert", desc: "Memory drop cache command issued", has_glasses: false, mouth_type: 7, eye_type: 6, brow_type: 1 },
    MascotExpression { id: 76, archetype: "SURPRISED", label: "Bewildered O", desc: "Database lock contention resolved", has_glasses: false, mouth_type: 7, eye_type: 6, brow_type: 3 },
    MascotExpression { id: 77, archetype: "SURPRISED", label: "Wide Stare", desc: "Rapid spike in web analytics", has_glasses: false, mouth_type: 7, eye_type: 6, brow_type: 1 },
    MascotExpression { id: 78, archetype: "SURPRISED", label: "High Eyebrows O", desc: "Zero-day vulnerability patch alert", has_glasses: false, mouth_type: 7, eye_type: 6, brow_type: 1 },
    MascotExpression { id: 79, archetype: "SURPRISED", label: "Sovereign Anomaly", desc: "Unexpected edge case captured", has_glasses: false, mouth_type: 7, eye_type: 6, brow_type: 1 },

    // 80..89: SOVEREIGN ZEN / SLEEP (Quiescent Night / Idle)
    MascotExpression { id: 80, archetype: "ZEN", label: "Slumber Slits", desc: "System quiescent & idle", has_glasses: false, mouth_type: 8, eye_type: 7, brow_type: 0 },
    MascotExpression { id: 81, archetype: "ZEN", label: "Zen Smile", desc: "Peaceful background operation", has_glasses: false, mouth_type: 0, eye_type: 7, brow_type: 0 },
    MascotExpression { id: 82, archetype: "ZEN", label: "Deep Meditation", desc: "Zero processes hogging CPU", has_glasses: false, mouth_type: 8, eye_type: 7, brow_type: 0 },
    MascotExpression { id: 83, archetype: "ZEN", label: "Sleepy Zzz", desc: "Overnight maintenance cycle", has_glasses: false, mouth_type: 8, eye_type: 7, brow_type: 0 },
    MascotExpression { id: 84, archetype: "ZEN", label: "Gentle Respiration", desc: "Battery conservation mode active", has_glasses: false, mouth_type: 0, eye_type: 7, brow_type: 0 },
    MascotExpression { id: 85, archetype: "ZEN", label: "Quiescent Slumber", desc: "Low thermal footprint <45°C", has_glasses: false, mouth_type: 8, eye_type: 7, brow_type: 0 },
    MascotExpression { id: 86, archetype: "ZEN", label: "Trance State", desc: "Autonomous daemon resting", has_glasses: false, mouth_type: 0, eye_type: 7, brow_type: 0 },
    MascotExpression { id: 87, archetype: "ZEN", label: "Calm Serenity", desc: "All background queues empty", has_glasses: false, mouth_type: 0, eye_type: 7, brow_type: 0 },
    MascotExpression { id: 88, archetype: "ZEN", label: "Half Awake Blink", desc: "Telemetry tick awakening", has_glasses: false, mouth_type: 0, eye_type: 1, brow_type: 0 },
    MascotExpression { id: 89, archetype: "ZEN", label: "Sovereign Quiescence", desc: "Harmonious sovereign harmony", has_glasses: false, mouth_type: 0, eye_type: 7, brow_type: 0 },

    // 90..99: SOVEREIGN CYBER / MATRIX OVERDRIVE (Turbo Overclock)
    MascotExpression { id: 90, archetype: "CYBER", label: "Matrix Code Eyes", desc: "Turbo mode engaged / stream active", has_glasses: false, mouth_type: 3, eye_type: 8, brow_type: 1 },
    MascotExpression { id: 91, archetype: "CYBER", label: "Crosshair HUD", desc: "Targeted performance optimization", has_glasses: false, mouth_type: 0, eye_type: 9, brow_type: 2 },
    MascotExpression { id: 92, archetype: "CYBER", label: "Binary Visor", desc: "BBR congestion control accelerating", has_glasses: true, mouth_type: 3, eye_type: 8, brow_type: 1 },
    MascotExpression { id: 93, archetype: "CYBER", label: "Neon Glitch", desc: "Kernel drop caches execution", has_glasses: false, mouth_type: 9, eye_type: 8, brow_type: 3 },
    MascotExpression { id: 94, archetype: "CYBER", label: "Turbo Overdrive", desc: "Max governor performance lock", has_glasses: true, mouth_type: 4, eye_type: 0, brow_type: 1 },
    MascotExpression { id: 95, archetype: "CYBER", label: "Warp Acceleration", desc: "Parallel cargo compilation burst", has_glasses: false, mouth_type: 4, eye_type: 8, brow_type: 1 },
    MascotExpression { id: 96, archetype: "CYBER", label: "Crypto Proof Gaze", desc: "Hardware SHA-256 / Ed25519 loop", has_glasses: false, mouth_type: 3, eye_type: 9, brow_type: 1 },
    MascotExpression { id: 97, archetype: "CYBER", label: "Quantum HUD", desc: "Multi-cluster mesh synchronization", has_glasses: false, mouth_type: 8, eye_type: 9, brow_type: 1 },
    MascotExpression { id: 98, archetype: "CYBER", label: "Root Awakening", desc: "Maximum sovereign permissions", has_glasses: true, mouth_type: 3, eye_type: 8, brow_type: 1 },
    MascotExpression { id: 99, archetype: "CYBER", label: "Sovereign Singularity", desc: "Peak sovereign developer singularity", has_glasses: true, mouth_type: 4, eye_type: 8, brow_type: 1 },
];

/// Deterministically selects the exact expression from the 100 expressions
/// based on real-time CAShell telemetry.
pub fn select_expression(telem: &MascotTelemetry) -> &'static MascotExpression {
    // 1. High Priority: Explicit Turbo mode engaged
    if telem.turbo_active {
        let v = (telem.tick / 12) % 10;
        return &SOVEREIGN_EXPRESSIONS[90 + v];
    }

    // 2. High Priority: Thermal Overheat or CPU Overload (>80°C or >85%)
    if telem.cpu_temp_c > 80.0 || telem.cpu_pct > 85.0 {
        let v = (telem.tick / 10) % 10;
        return &SOVEREIGN_EXPRESSIONS[60 + v];
    }

    // 3. High Priority: RAM / Swap pressure
    if telem.ram_pct >= 85 || telem.swap_pct >= 80 {
        let v = (telem.tick / 14) % 10;
        return &SOVEREIGN_EXPRESSIONS[60 + v];
    }

    // 4. Critical: Node or Container Down (Surprise / Anomaly)
    if telem.fleet_nodes_down > 0 || telem.containers_down > 0 || telem.uptime_monitors_down > 0 {
        let v = (telem.tick / 16) % 10;
        return &SOVEREIGN_EXPRESSIONS[70 + v];
    }

    // 5. User Interaction Actions
    if let Some(action) = telem.transient_action {
        match action {
            PetActionState::Attack | PetActionState::Dash => {
                let v = (telem.tick / 6) % 10;
                return &SOVEREIGN_EXPRESSIONS[90 + v]; // Turbo / Cyber
            }
            PetActionState::Jump => {
                let v = (telem.tick / 8) % 10;
                return &SOVEREIGN_EXPRESSIONS[40 + v]; // Tongue Blep
            }
            PetActionState::Run => {
                let v = (telem.tick / 10) % 10;
                return &SOVEREIGN_EXPRESSIONS[10 + v]; // Wink
            }
            PetActionState::Death => {
                let v = (telem.tick / 16) % 10;
                return &SOVEREIGN_EXPRESSIONS[80 + v]; // Zen Sleep
            }
            _ => {}
        }
    }

    // 6. Alert state: Uncommitted git or medium CPU (50..80%)
    if telem.git_dirty_count > 0 || telem.cpu_pct > 50.0 {
        let v = (telem.tick / 14) % 10;
        return &SOVEREIGN_EXPRESSIONS[50 + v];
    }

    // 7. Idle / Night / Very low CPU
    if telem.cpu_pct < 8.0 && (telem.tick / 24) % 5 == 4 {
        let v = (telem.tick / 20) % 10;
        return &SOVEREIGN_EXPRESSIONS[80 + v];
    }

    // 8. High XP Celebration
    if telem.xp_level >= 3 && (telem.tick / 30) % 4 == 3 {
        let v = (telem.tick / 12) % 10;
        return &SOVEREIGN_EXPRESSIONS[30 + v];
    }

    // 9. Standard nominal: Cycle between Shades (0..9), Wink (10..19), Grin (20..29)
    let cycle = (telem.tick / 32) % 3;
    let v = (telem.tick / 8) % 10;
    match cycle {
        0 => &SOVEREIGN_EXPRESSIONS[v],        // Shades
        1 => &SOVEREIGN_EXPRESSIONS[10 + v],   // Wink
        _ => &SOVEREIGN_EXPRESSIONS[20 + v],   // Grin
    }
}

// ── 3D Extruded Isometric Sovereign Token Renderer ───────────────────────────

/// Renders the complete Cyberpunk Sovereign Mascot:
/// - Falling Matrix digital code streams in the background (if `matrix_rain` is enabled)
/// - 3D Extruded cylinder token floating with smooth sine-wave bobbing
/// - Neon phosphor green aesthetic with block character depth
/// - Custom facial features corresponding to the selected expression from the 100-expression matrix
pub fn render_sovereign_token_lines(
    width: usize,
    height: usize,
    telem: &MascotTelemetry,
    matrix_rain: bool,
) -> (Vec<Line<'static>>, &'static MascotExpression) {
    let expr = select_expression(telem);

    let w = width.max(12);
    let h = height.max(6);

    // Grid buffers for character, foreground color, and modifier
    let mut char_grid = vec![vec![' '; w]; h];
    let mut fg_grid = vec![vec![Color::Reset; w]; h];
    let mut mod_grid = vec![vec![Modifier::empty(); w]; h];

    // 1. Matrix Digital Rain Background
    if matrix_rain {
        const RAIN_CHARS: &[char] = &[
            '0', '1', 'A', 'F', 'C', 'X', '8', '9', '7', 'E', '#', '*', '~', ':', ';', '·',
        ];
        let t = telem.tick;
        for col in 0..w {
            let col_speed = 1 + (col % 3);
            let head_y = ((t * col_speed + col * 7) % (h + 12)) as i32 - 4;
            for r in 0..h {
                let dist = head_y - (r as i32);
                if (0..=5).contains(&dist) {
                    let char_idx = (col * 17 + r * 5 + t) % RAIN_CHARS.len();
                    char_grid[r][col] = RAIN_CHARS[char_idx];
                    if dist == 0 {
                        fg_grid[r][col] = Color::Rgb(120, 255, 160);
                        mod_grid[r][col] = Modifier::BOLD;
                    } else if dist <= 2 {
                        fg_grid[r][col] = Color::Rgb(0, 180, 50);
                    } else {
                        fg_grid[r][col] = Color::Rgb(0, 75, 20);
                        mod_grid[r][col] = Modifier::DIM;
                    }
                } else if (col + r + t / 4) % 19 == 0 {
                    // Faint ambient digital static
                    char_grid[r][col] = '·';
                    fg_grid[r][col] = Color::Rgb(0, 45, 15);
                    mod_grid[r][col] = Modifier::DIM;
                }
            }
        }
    }

    // 2. Stable Hovering Bobbing (melayang stabil)
    // Smooth sine wave: ±1 row vertical floating, no horizontal jitter
    let bob_sine = (telem.tick as f64 * 0.12).sin();
    let bob = if bob_sine > 0.4 { 1i32 } else if bob_sine < -0.4 { -1i32 } else { 0i32 };

    // 3. Coin Geometry & 3D Extrusion
    let cx = (w as f64) / 2.0;
    let cy = ((h as f64) / 2.0 - 1.2 + (bob as f64)).max(2.0);
    let rx = ((w as f64) / 2.0 - 2.8).max(4.0);
    let ry = ((h as f64) / 3.4).max(1.8);
    let thickness = (h / 4).clamp(2, 4) as i32;

    // Mood-specific neon phosphor hue
    let neon_bright = match telem.cpu_temp_c {
        t if t > 80.0 => Color::Rgb(255, 60, 60),  // Overheated Red
        _ if telem.turbo_active => Color::Rgb(0, 255, 240), // Turbo Cyan
        _ => Color::Rgb(57, 255, 20), // Sovereign Phosphor Neon Green
    };
    let neon_mid = match telem.cpu_temp_c {
        t if t > 80.0 => Color::Rgb(180, 30, 30),
        _ if telem.turbo_active => Color::Rgb(0, 180, 200),
        _ => Color::Rgb(0, 190, 55),
    };
    let neon_dim = match telem.cpu_temp_c {
        t if t > 80.0 => Color::Rgb(90, 15, 15),
        _ if telem.turbo_active => Color::Rgb(0, 80, 100),
        _ => Color::Rgb(0, 90, 25),
    };

    // Draw 3D Extrusion Rim (bottom curved layers)
    for t_step in (1..=thickness).rev() {
        let cy_ext = cy + (t_step as f64);
        for x in 0..w {
            let dx = (x as f64 - cx) / rx;
            if dx.abs() <= 1.0 {
                let dy = (1.0 - dx * dx).max(0.0).sqrt() * ry;
                let y_bot = (cy_ext + dy).round() as i32;
                if y_bot >= 0 && (y_bot as usize) < h {
                    let y_idx = y_bot as usize;
                    char_grid[y_idx][x] = if t_step == thickness { '█' } else { '▓' };
                    fg_grid[y_idx][x] = if t_step == thickness { neon_bright } else { neon_mid };
                    mod_grid[y_idx][x] = Modifier::BOLD;
                }
                // Extruded side walls
                let y_mid = (cy + dy).round() as i32;
                if y_mid <= y_bot {
                    for yw in y_mid..=y_bot {
                        if yw >= 0 && (yw as usize) < h {
                            let yw_idx = yw as usize;
                            if char_grid[yw_idx][x] != '█' {
                                char_grid[yw_idx][x] = '▒';
                                fg_grid[yw_idx][x] = neon_dim;
                            }
                        }
                    }
                }
            }
        }
    }

    // Draw Top Face Disc (Isometric Ellipse)
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f64 - cx) / rx;
            let dy = (y as f64 - cy) / ry;
            let val = dx * dx + dy * dy;
            if val <= 1.0 {
                if val >= 0.72 {
                    // Outer rim of the top disc
                    char_grid[y][x] = '█';
                    fg_grid[y][x] = neon_bright;
                    mod_grid[y][x] = Modifier::BOLD;
                } else {
                    // Interior face of the token
                    char_grid[y][x] = ' ';
                    fg_grid[y][x] = Color::Reset;
                    mod_grid[y][x] = Modifier::empty();
                }
            }
        }
    }

    // 4. Render Facial Features inside Top Face Disc
    let fcx = cx.round() as i32;
    let fcy = cy.round() as i32;

    if expr.has_glasses {
        // Pixel Cyber Sunglasses
        // Top brow line of shades
        if fcy - 1 >= 0 && ((fcy - 1) as usize) < h {
            let r = (fcy - 1) as usize;
            for dx in -4..=4 {
                let x = fcx + dx;
                if x >= 0 && (x as usize) < w {
                    char_grid[r][x as usize] = '▄';
                    fg_grid[r][x as usize] = neon_bright;
                    mod_grid[r][x as usize] = Modifier::BOLD;
                }
            }
        }
        // Main lenses & bridge
        if fcy >= 0 && (fcy as usize) < h {
            let r = fcy as usize;
            for dx in -4..=4 {
                let x = fcx + dx;
                if x >= 0 && (x as usize) < w {
                    if dx == 0 {
                        char_grid[r][x as usize] = '▀'; // bridge
                    } else {
                        char_grid[r][x as usize] = '█'; // solid dark neon lenses
                    }
                    fg_grid[r][x as usize] = neon_bright;
                    mod_grid[r][x as usize] = Modifier::BOLD;
                }
            }
        }
    } else {
        // Non-glasses expressions: Render Eyebrows & Eyes
        // Eyebrows
        if fcy - 1 >= 0 && ((fcy - 1) as usize) < h {
            let r = (fcy - 1) as usize;
            match expr.brow_type {
                1 => { // Raised brows
                    let lx = fcx - 3;
                    let rx_b = fcx + 3;
                    if lx >= 0 && (lx as usize) < w { char_grid[r][lx as usize] = '▀'; fg_grid[r][lx as usize] = neon_bright; }
                    if rx_b >= 0 && (rx_b as usize) < w { char_grid[r][rx_b as usize] = '▀'; fg_grid[r][rx_b as usize] = neon_bright; }
                }
                2 => { // Angry brows \ /
                    let lx = fcx - 3;
                    let rx_b = fcx + 3;
                    if lx >= 0 && (lx as usize) < w { char_grid[r][lx as usize] = '╲'; fg_grid[r][lx as usize] = neon_bright; }
                    if rx_b >= 0 && (rx_b as usize) < w { char_grid[r][rx_b as usize] = '╱'; fg_grid[r][rx_b as usize] = neon_bright; }
                }
                3 => { // Skeptical (one raised, one flat)
                    let lx = fcx - 3;
                    let rx_b = fcx + 3;
                    if lx >= 0 && (lx as usize) < w { char_grid[r][lx as usize] = '▀'; fg_grid[r][lx as usize] = neon_bright; }
                    if rx_b >= 0 && (rx_b as usize) < w { char_grid[r][rx_b as usize] = '─'; fg_grid[r][rx_b as usize] = neon_bright; }
                }
                _ => {}
            }
        }

        // Eyes
        if fcy >= 0 && (fcy as usize) < h {
            let r = fcy as usize;
            let lx = fcx - 3;
            let rx_e = fcx + 3;

            let (l_ch, r_ch) = match expr.eye_type {
                1 => ('─', '█'), // Wink Left
                2 => ('█', '─'), // Wink Right
                3 => ('█', '█'), // Normal open eyes
                4 => ('^', '^'), // Squint happy eyes
                5 => ('▲', '▲'), // Angry eyes
                6 => ('O', 'O'), // Surprised open eyes
                7 => ('─', '─'), // Sleep slits
                8 => ('1', '0'), // Matrix binary eyes
                9 => ('+', '+'), // Crosshair HUD eyes
                _ => ('█', '█'),
            };

            if lx >= 0 && (lx as usize) < w {
                char_grid[r][lx as usize] = l_ch;
                fg_grid[r][lx as usize] = neon_bright;
                mod_grid[r][lx as usize] = Modifier::BOLD;
            }
            if rx_e >= 0 && (rx_e as usize) < w {
                char_grid[r][rx_e as usize] = r_ch;
                fg_grid[r][rx_e as usize] = neon_bright;
                mod_grid[r][rx_e as usize] = Modifier::BOLD;
            }
        }
    }

    // Mouth rendering
    if fcy + 2 >= 0 && ((fcy + 2) as usize) < h {
        let r = (fcy + 2) as usize;
        match expr.mouth_type {
            0 => { // Standard Smile
                for dx in -2..=2 {
                    let x = fcx + dx;
                    if x >= 0 && (x as usize) < w {
                        char_grid[r][x as usize] = '▄';
                        fg_grid[r][x as usize] = neon_bright;
                        mod_grid[r][x as usize] = Modifier::BOLD;
                    }
                }
            }
            1 => { // Smirk Left
                for dx in -3..=0 {
                    let x = fcx + dx;
                    if x >= 0 && (x as usize) < w {
                        char_grid[r][x as usize] = '▄';
                        fg_grid[r][x as usize] = neon_bright;
                    }
                }
            }
            2 => { // Smirk Right
                for dx in 0..=3 {
                    let x = fcx + dx;
                    if x >= 0 && (x as usize) < w {
                        char_grid[r][x as usize] = '▄';
                        fg_grid[r][x as usize] = neon_bright;
                    }
                }
            }
            3 => { // Wide Grin
                for dx in -3..=3 {
                    let x = fcx + dx;
                    if x >= 0 && (x as usize) < w {
                        char_grid[r][x as usize] = '█';
                        fg_grid[r][x as usize] = neon_bright;
                    }
                }
            }
            4 => { // Open Laughing Mouth
                for dx in -2..=2 {
                    let x = fcx + dx;
                    if x >= 0 && (x as usize) < w {
                        char_grid[r][x as usize] = '▀';
                        fg_grid[r][x as usize] = neon_bright;
                        mod_grid[r][x as usize] = Modifier::BOLD;
                    }
                }
            }
            5 => { // Tongue Blep
                for dx in -2..=2 {
                    let x = fcx + dx;
                    if x >= 0 && (x as usize) < w {
                        char_grid[r][x as usize] = '▄';
                        fg_grid[r][x as usize] = neon_bright;
                    }
                }
                // Tongue hanging below mouth
                if (r + 1) < h {
                    let tx = fcx;
                    if tx >= 0 && (tx as usize) < w {
                        char_grid[r + 1][tx as usize] = '█';
                        fg_grid[r + 1][tx as usize] = Color::Rgb(255, 100, 150); // pink neon tongue
                        mod_grid[r + 1][tx as usize] = Modifier::BOLD;
                    }
                }
            }
            6 => { // Frown
                for dx in -2..=2 {
                    let x = fcx + dx;
                    if x >= 0 && (x as usize) < w {
                        char_grid[r][x as usize] = '▀';
                        fg_grid[r][x as usize] = neon_bright;
                        mod_grid[r][x as usize] = Modifier::BOLD;
                    }
                }
            }
            7 => { // Open "O" mouth
                for dx in -1..=1 {
                    let x = fcx + dx;
                    if x >= 0 && (x as usize) < w {
                        char_grid[r][x as usize] = '○';
                        fg_grid[r][x as usize] = neon_bright;
                        mod_grid[r][x as usize] = Modifier::BOLD;
                    }
                }
            }
            8 => { // Flat mouth
                for dx in -2..=2 {
                    let x = fcx + dx;
                    if x >= 0 && (x as usize) < w {
                        char_grid[r][x as usize] = '─';
                        fg_grid[r][x as usize] = neon_bright;
                    }
                }
            }
            9 => { // Zigzag teeth / grimace
                for dx in -2..=2 {
                    let x = fcx + dx;
                    if x >= 0 && (x as usize) < w {
                        char_grid[r][x as usize] = '░';
                        fg_grid[r][x as usize] = neon_bright;
                    }
                }
            }
            _ => {}
        }
    }

    // Assemble the grid into Ratatui Lines
    let mut lines = Vec::with_capacity(h);
    for r in 0..h {
        let mut spans = Vec::new();
        let mut current_str = String::new();
        let mut current_fg = fg_grid[r][0];
        let mut current_mod = mod_grid[r][0];

        for c in 0..w {
            let ch = char_grid[r][c];
            let fg = fg_grid[r][c];
            let md = mod_grid[r][c];

            if fg == current_fg && md == current_mod {
                current_str.push(ch);
            } else {
                if !current_str.is_empty() {
                    spans.push(Span::styled(
                        current_str,
                        Style::default().fg(current_fg).add_modifier(current_mod),
                    ));
                    current_str = String::new();
                }
                current_fg = fg;
                current_mod = md;
                current_str.push(ch);
            }
        }
        if !current_str.is_empty() {
            spans.push(Span::styled(
                current_str,
                Style::default().fg(current_fg).add_modifier(current_mod),
            ));
        }
        lines.push(Line::from(spans));
    }

    (lines, expr)
}

// ── Backward-compatible Isometric Head Helper ────────────────────────────────

/// Backward compatibility stub: maps old caller to the new Sovereign token.
pub fn render_isometric_head(
    tick: usize,
    mood: PetMood,
    cols: usize,
    rows: usize,
) -> Vec<String> {
    let telem = MascotTelemetry {
        cpu_pct: match mood {
            PetMood::Sprint => 75.0,
            PetMood::Overheated => 90.0,
            _ => 15.0,
        },
        cpu_temp_c: match mood {
            PetMood::Overheated => 88.0,
            _ => 50.0,
        },
        tick,
        ..Default::default()
    };

    let (lines, _) = render_sovereign_token_lines(cols, rows, &telem, false);
    lines
        .into_iter()
        .map(|l| {
            l.spans
                .into_iter()
                .map(|s| s.content.to_string())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect()
}

// ── PetStatus Frame Builder ──────────────────────────────────────────────────

impl PetStatus {
    pub fn build_frame(&self, state: PetActionState, tick: usize, _is_stopped: bool) -> EmojiFrame {
        let telem = MascotTelemetry {
            cpu_pct: match self.mood {
                PetMood::Sprint => 75.0,
                PetMood::Overheated => 92.0,
                _ => 20.0,
            },
            cpu_temp_c: match self.mood {
                PetMood::Overheated => 85.0,
                _ => 52.0,
            },
            transient_action: Some(state),
            xp_level: match self.stage {
                PetStage::Egg => 0,
                PetStage::Hatch => 1,
                PetStage::Juvenile => 2,
                PetStage::Adult => 3,
                PetStage::Mating => 4,
            },
            tick,
            ..Default::default()
        };

        let expr = select_expression(&telem);

        let label = match state {
            PetActionState::Idle => format!("[#{:02} {}]", expr.id, expr.label),
            PetActionState::Walk => format!("[#{:02} MESH DRIFT]", expr.id),
            PetActionState::Run => format!("[#{:02} RPC BURST]", expr.id),
            PetActionState::Jump => format!("[#{:02} OVERCLOCK]", expr.id),
            PetActionState::Attack => format!("[#{:02} SECURITY SCAN]", expr.id),
            PetActionState::Dash => format!("[#{:02} TURBO DASH]", expr.id),
            PetActionState::Hit => format!("[#{:02} THERMAL ALERT]", expr.id),
            PetActionState::Death => format!("[#{:02} QUIESCENT / ZZZ]", expr.id),
        };

        EmojiFrame {
            core: "🪙".to_string(),
            particles: format!("⚡ {}", expr.desc),
            x_offset: 0,
            y_offset: 0,
            state_label: label,
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
        let mood_badge = match self.mood {
            PetMood::Idle => "☺ NOMINAL",
            PetMood::Overheated => "🔥 HOT",
            PetMood::Sick => "💀 UNHEALTHY",
            PetMood::Sprint => "⚡ TURBO",
        };
        format!(
            "{} {} [Lv.{} | {}]",
            self.stage.stage_emoji(),
            self.name,
            level,
            mood_badge
        )
    }
}

// ── Unit Tests ───────────────────────────────────────────────────────────────

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
    fn test_all_100_expressions_exist() {
        assert_eq!(SOVEREIGN_EXPRESSIONS.len(), 100);
        for (i, expr) in SOVEREIGN_EXPRESSIONS.iter().enumerate() {
            assert_eq!(expr.id, i);
            assert!(!expr.label.is_empty());
            assert!(!expr.desc.is_empty());
        }
    }

    #[test]
    fn test_expression_selection_responsiveness() {
        // 1. Turbo mode active -> returns CYBER archetype (90..99)
        let turbo_telem = MascotTelemetry {
            turbo_active: true,
            tick: 0,
            ..Default::default()
        };
        let expr = select_expression(&turbo_telem);
        assert_eq!(expr.archetype, "CYBER");
        assert!((90..=99).contains(&expr.id));

        // 2. Overheated CPU -> returns ANGRY archetype (60..69)
        let hot_telem = MascotTelemetry {
            cpu_temp_c: 88.0,
            cpu_pct: 92.0,
            tick: 0,
            ..Default::default()
        };
        let expr_hot = select_expression(&hot_telem);
        assert_eq!(expr_hot.archetype, "ANGRY");
        assert!((60..=69).contains(&expr_hot.id));

        // 3. Node down -> returns SURPRISED archetype (70..79)
        let node_down_telem = MascotTelemetry {
            fleet_nodes_down: 1,
            tick: 0,
            ..Default::default()
        };
        let expr_down = select_expression(&node_down_telem);
        assert_eq!(expr_down.archetype, "SURPRISED");
        assert!((70..=79).contains(&expr_down.id));
    }

    #[test]
    fn test_token_rendering_output() {
        let telem = MascotTelemetry {
            cpu_pct: 20.0,
            cpu_temp_c: 55.0,
            tick: 5,
            ..Default::default()
        };
        let (lines, expr) = render_sovereign_token_lines(30, 12, &telem, true);
        assert_eq!(lines.len(), 12);
        assert_eq!(expr.archetype, "SHADES");

        // Test without matrix rain
        let (clean_lines, _) = render_sovereign_token_lines(30, 12, &telem, false);
        assert_eq!(clean_lines.len(), 12);
    }
}
