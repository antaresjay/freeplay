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
    pub exe: String,
    #[serde(default)]
    pub notes: String,
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
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub locator: Option<Locator>,
    #[serde(flatten)]
    pub action: Action,
}

fn default_category() -> Category {
    Category::Misc
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "find", rename_all = "lowercase")]
pub enum Locator {
    Static {
        module: String,
        #[serde(deserialize_with = "hex_or_int", serialize_with = "as_hex")]
        offset: usize,
        #[serde(default)]
        hops: Vec<Hop>,
    },
    Pattern {
        pattern: String,
        #[serde(default)]
        scope: Scope,
        #[serde(default)]
        module: Option<String>,
        #[serde(default)]
        offset: i64,
        #[serde(default)]
        rip: Option<Rip>,
        #[serde(default)]
        hops: Vec<Hop>,
    },
    Symbol {
        symbol: String,
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
    pub displacement_at: usize,
    pub instruction_length: usize,
}

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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Action {
    Freeze {
        #[serde(rename = "value_type")]
        kind: TypeName,
        value: Number,
    },
    Set {
        #[serde(rename = "value_type")]
        kind: TypeName,
        value: Number,
    },
    Nop {
        length: usize,
    },
    Bytes {
        replacement: String,
    },
    Script {
        source: String,
    },
}

impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Action::Freeze { .. } => "Freeze",
            Action::Set { .. } => "Set once",
            Action::Nop { .. } => "Patch out",
            Action::Bytes { .. } => "Patch",
            Action::Script { .. } => "Script",
        }
    }

    pub fn is_script(&self) -> bool {
        matches!(self, Action::Script { .. })
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Number {
    Int(i64),
    Float(f64),
}

impl Number {
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
