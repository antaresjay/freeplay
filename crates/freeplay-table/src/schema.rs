//! What a table file looks like on disk.

use freeplay_core::value::{Scalar, ValueKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Table {
    pub game: Game,
    #[serde(default, rename = "cheat")]
    pub cheats: Vec<Cheat>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Game {
    pub name: String,
    /// Executable to attach to, matched case insensitively.
    pub exe: String,
    #[serde(default)]
    pub notes: String,
    /// Game builds this table was actually checked against. A mismatch is a
    /// warning rather than a refusal, since signatures often survive a patch.
    #[serde(default)]
    pub verified: Vec<String>,
    #[serde(default)]
    pub author: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Player,
    Resources,
    Combat,
    Movement,
    Game,
    Misc,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Player => "Player",
            Category::Resources => "Resources",
            Category::Combat => "Combat",
            Category::Movement => "Movement",
            Category::Game => "Game",
            Category::Misc => "Misc",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Cheat {
    pub id: String,
    pub name: String,
    #[serde(default = "default_category")]
    pub category: Category,
    #[serde(default)]
    pub description: String,
    /// Shown in the interface before the toggle is flipped. Use it for things
    /// like "only works once you are in a mission".
    #[serde(default)]
    pub hint: String,
    pub locator: Locator,
    #[serde(flatten)]
    pub action: Action,
}

fn default_category() -> Category {
    Category::Misc
}

/// How to find the address.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "find", rename_all = "lowercase")]
pub enum Locator {
    /// A fixed offset inside a module. Survives ASLR, breaks on a game patch.
    Static {
        module: String,
        #[serde(deserialize_with = "hex_or_int", serialize_with = "as_hex")]
        offset: usize,
        #[serde(default)]
        hops: Vec<Hop>,
    },
    /// Search for instruction bytes. Survives most patches.
    Pattern {
        pattern: String,
        #[serde(default)]
        scope: Scope,
        #[serde(default)]
        module: Option<String>,
        /// Bytes into the match where the interesting part starts.
        #[serde(default)]
        offset: i64,
        /// For `mov rax, [rip+disp32]` style operands, where the address is
        /// relative to the end of the instruction rather than absolute.
        #[serde(default)]
        rip: Option<Rip>,
        #[serde(default)]
        hops: Vec<Hop>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[default]
    Code,
    Data,
    All,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Rip {
    /// Offset from the match to the 4 byte displacement.
    pub displacement_at: usize,
    /// Total instruction length, since rip points at the next instruction.
    pub instruction_length: usize,
}

/// One step of a pointer chain. Accepts `"+0x18"`, `"-0x8"` or a plain number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hop(pub isize);

impl<'de> Deserialize<'de> for Hop {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(i64),
            Text(String),
        }

        match Raw::deserialize(d)? {
            Raw::Int(v) => Ok(Hop(v as isize)),
            Raw::Text(s) => parse_hop(&s).map(Hop).map_err(D::Error::custom),
        }
    }
}

impl Serialize for Hop {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let text = if self.0 < 0 {
            format!("-{:#x}", -self.0)
        } else {
            format!("+{:#x}", self.0)
        };
        s.serialize_str(&text)
    }
}

fn parse_hop(text: &str) -> Result<isize, String> {
    let trimmed = text.trim();
    let (sign, rest) = match trimmed.strip_prefix('-') {
        Some(rest) => (-1isize, rest),
        None => (1isize, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let magnitude = match rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        Some(hex) => isize::from_str_radix(hex, 16),
        None => rest.parse::<isize>(),
    }
    .map_err(|_| format!("bad offset {text:?}"))?;
    Ok(sign * magnitude)
}

/// Written back as hex. A table is meant to be read and edited by hand, and
/// nobody recognises a module offset in decimal.
fn as_hex<S: serde::Serializer>(value: &usize, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format!("{value:#x}"))
}

fn hex_or_int<'de, D: serde::Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
    use serde::de::Error as _;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Int(i64),
        Text(String),
    }
    match Raw::deserialize(d)? {
        Raw::Int(v) => Ok(v as usize),
        Raw::Text(s) => parse_hop(&s).map(|v| v as usize).map_err(D::Error::custom),
    }
}

/// What to do once the address is known.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Action {
    /// Hold a value in place while the cheat is on.
    Freeze {
        #[serde(rename = "value_type")]
        kind: TypeName,
        value: Number,
    },
    /// Write once and let the game carry on.
    Set {
        #[serde(rename = "value_type")]
        kind: TypeName,
        value: Number,
    },
    /// Replace instructions with no-ops.
    Nop { length: usize },
    /// Replace instructions with specific bytes.
    Bytes { replacement: String },
}

/// Wrapper so a value type can be written as `"f32"` in TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeName(pub ValueKind);

impl<'de> Deserialize<'de> for TypeName {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let text = String::deserialize(d)?;
        text.parse::<ValueKind>()
            .map(TypeName)
            .map_err(D::Error::custom)
    }
}

impl Serialize for TypeName {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0.name())
    }
}

/// A number written in TOML as either an integer or a float.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Number {
    Int(i64),
    Float(f64),
}

impl Number {
    /// Coerce to whatever type the cheat declared, so `value = 1000` works for
    /// an f32 field without anyone having to write `1000.0`.
    pub fn to_scalar(self, kind: ValueKind) -> Scalar {
        let text = match self {
            Number::Int(v) => v.to_string(),
            Number::Float(v) => v.to_string(),
        };
        kind.parse(&text).unwrap_or_else(|| match self {
            Number::Int(v) => kind
                .parse(&(v as f64).to_string())
                .unwrap_or(Scalar::I64(v)),
            Number::Float(v) => Scalar::F64(v),
        })
    }
}
