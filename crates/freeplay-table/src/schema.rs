use freeplay_core::value::{Scalar, ValueKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Table {
    pub game: Game,
    #[serde(default)]
    pub meta: Meta,
    #[serde(default, rename = "cheat")]
    pub cheats: Vec<Cheat>,
}

// where a table came from and whether anybody has run it. a toggle that has
// been watched working in the game is worth more than one somebody typed out,
// and the interface should be able to say which is which
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Meta {
    pub source: Source,
    pub submitted_by: String,
    // date the check ran, and the game build it ran against
    pub checked_at: String,
    pub checked_build: String,
    pub checked: Option<Checked>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    // written or converted here and never run anywhere
    #[default]
    Unverified,
    // somebody ran it against the game and sent it in
    Community,
    // shipped with freeplay
    Bundled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Checked {
    pub worked: u32,
    pub tried: u32,
}

impl Meta {
    pub fn trust(&self) -> Trust {
        match (self.source, self.checked) {
            (_, Some(c)) if c.tried > 0 && c.worked == c.tried => Trust::Verified,
            (_, Some(c)) if c.tried > 0 && c.worked > 0 => Trust::Partial,
            (Source::Community, _) => Trust::Community,
            _ => Trust::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    Verified,
    Partial,
    Community,
    Unknown,
}

impl Trust {
    pub fn label(self) -> &'static str {
        match self {
            Trust::Verified => "verified",
            Trust::Partial => "partly working",
            Trust::Community => "community",
            Trust::Unknown => "unchecked",
        }
    }
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
    // keys the table author bound, straight out of the file
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hotkeys: Vec<Hotkey>,
    #[serde(flatten)]
    pub action: Action,
}

// what pressing it does. cheat engine also has increase and decrease by a
// step, which nothing here supports yet, so those are dropped on import
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tap {
    Toggle,
    On,
    Off,
    Set,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Hotkey {
    pub does: Tap,
    // virtual key codes, modifiers included, exactly as the table listed them
    pub keys: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
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
    // a number the player picks. weight limit, game speed, how much gold.
    // freeze is the special case where the number is always the same one
    Value {
        #[serde(rename = "value_type")]
        kind: TypeName,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Number>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<Number>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<Number>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        choices: Vec<Choice>,
        #[serde(default, skip_serializing_if = "is_false")]
        hex: bool,
        // hold it there against the game writing it back
        #[serde(default = "yes", skip_serializing_if = "is_yes")]
        lock: bool,
    },
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

fn yes() -> bool {
    true
}

fn is_yes(v: &bool) -> bool {
    *v
}

fn is_false(v: &bool) -> bool {
    !*v
}

// one line of a cheat engine dropdown, "0:Off". a bare number is allowed and
// just labels itself
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub value: Number,
    pub label: String,
}

impl Choice {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (number, label) = match text.split_once(':') {
            Some((number, label)) => (number.trim(), label.trim().to_string()),
            None => (text, text.to_string()),
        };
        Some(Self {
            value: Number::parse(number)?,
            label: if label.is_empty() {
                number.to_string()
            } else {
                label
            },
        })
    }
}

impl<'de> Deserialize<'de> for Choice {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let text = String::deserialize(d)?;
        Choice::parse(&text).ok_or_else(|| D::Error::custom(format!("bad choice {text:?}")))
    }
}

impl Serialize for Choice {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{}:{}", self.value, self.label))
    }
}

impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Action::Value { lock: true, .. } => "Value",
            Action::Value { .. } => "Set once",
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

    // the type held at the address, for anything that reads or writes one
    pub fn kind(&self) -> Option<ValueKind> {
        match self {
            Action::Value { kind, .. } | Action::Freeze { kind, .. } | Action::Set { kind, .. } => {
                Some(kind.0)
            }
            _ => None,
        }
    }

    // what goes in the box before anybody touches it
    pub fn default_value(&self) -> Option<Number> {
        match self {
            Action::Value { value, .. } => *value,
            Action::Freeze { value, .. } | Action::Set { value, .. } => Some(*value),
            _ => None,
        }
    }

    pub fn takes_a_number(&self) -> bool {
        matches!(self, Action::Value { .. })
    }

    pub fn choices(&self) -> &[Choice] {
        match self {
            Action::Value { choices, .. } => choices,
            _ => &[],
        }
    }

    pub fn shows_hex(&self) -> bool {
        matches!(self, Action::Value { hex: true, .. })
    }

    // whether it has to be written over and over, or once is enough
    pub fn holds(&self) -> bool {
        match self {
            Action::Value { lock, .. } => *lock,
            Action::Freeze { .. } => true,
            _ => false,
        }
    }

    pub fn limits(&self) -> (Option<Number>, Option<Number>) {
        match self {
            Action::Value { min, max, .. } => (*min, *max),
            _ => (None, None),
        }
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

impl std::fmt::Display for Number {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Number::Int(v) => write!(f, "{v}"),
            Number::Float(v) => write!(f, "{v}"),
        }
    }
}

impl Number {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(hex) = text
            .strip_prefix("0x")
            .or_else(|| text.strip_prefix("0X"))
            .or_else(|| text.strip_prefix('$'))
        {
            return i64::from_str_radix(hex, 16).ok().map(Number::Int);
        }
        if let Ok(v) = text.parse::<i64>() {
            return Some(Number::Int(v));
        }
        text.parse::<f64>().ok().map(Number::Float)
    }

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
