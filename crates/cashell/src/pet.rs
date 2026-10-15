//! Virtual Pet (Tamagotchi) Engine for cashell cockpit.
//!
//! Provides pet lifecycle progression (Egg -> Hatch -> Juvenile -> Adult -> Mating -> Egg),
//! XP gain tracking, and ASCII art sprites for 5 species:
//! - Ayam (Chicken)
//! - Singa (Lion)
//! - Burung (Bird)
//! - Ular (Snake)
//! - Beruang (Bear)

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
            Self::Egg => "Telur",
            Self::Hatch => "Menetas",
            Self::Juvenile => "Kecil",
            Self::Adult => "Dewasa",
            Self::Mating => "Menikah",
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

pub struct PetStatus {
    pub species: PetSpecies,
    pub name: String,
    pub stage: PetStage,
    pub xp: i64,
}

impl PetStatus {
    pub fn format_badge(&self) -> String {
        let emoji = self.species.emoji(&self.stage);
        let next = self.stage.next_threshold();
        let level = match self.stage {
            PetStage::Egg => 0,
            PetStage::Hatch => 1,
            PetStage::Juvenile => 2,
            PetStage::Adult => 3,
            PetStage::Mating => 4,
        };
        format!("{} {} Lv.{} [{}/{}]", emoji, self.name, level, self.xp, next)
    }

    pub fn ascii_art(&self) -> &'static [&'static str] {
        match (self.species, self.stage) {
            (PetSpecies::Ayam, PetStage::Egg) => &[
                "   .---.   ",
                "  /     \\  ",
                " |  Egg  | ",
                "  \\_____/  ",
            ],
            (PetSpecies::Ayam, PetStage::Hatch) => &[
                "   .---.   ",
                "  /(o.o)\\  ",
                "   /   \\   ",
                "  ''---''  ",
            ],
            (PetSpecies::Ayam, PetStage::Juvenile) => &[
                "   (\\__/)  ",
                "   (•ㅅ•)  ",
                "   / 　 づ ",
                "  ''   ''  ",
            ],
            (PetSpecies::Ayam, PetStage::Adult) => &[
                "   _//\\_   ",
                "  ( >.< )  ",
                "  /|   |\\  ",
                "   /   \\   ",
            ],
            (PetSpecies::Singa, PetStage::Adult) => &[
                "  /\\_//\\   ",
                " ( ='.'= ) ",
                " /*\"--\"*\\  ",
                "   \"   \"   ",
            ],
            (PetSpecies::Burung, PetStage::Adult) => &[
                "   /\\_/\\   ",
                "  ( >.< )  ",
                "  /\"\"-\"\"\\  ",
                "    \" \"    ",
            ],
            _ => &[
                "   .---.   ",
                "  | ~ ~ |  ",
                "   '---'   ",
            ],
        }
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
    fn test_species_parsing() {
        assert_eq!(PetSpecies::parse("ayam"), PetSpecies::Ayam);
        assert_eq!(PetSpecies::parse("singa"), PetSpecies::Singa);
        assert_eq!(PetSpecies::parse("burung"), PetSpecies::Burung);
        assert_eq!(PetSpecies::parse("ular"), PetSpecies::Ular);
        assert_eq!(PetSpecies::parse("beruang"), PetSpecies::Beruang);
    }
}
