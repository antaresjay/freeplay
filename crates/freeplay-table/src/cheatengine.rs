use std::collections::HashSet;

use freeplay_core::value::ValueKind;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::schema::{
    Action, Category, Cheat, Choice, Game, Hop, Hotkey, Locator, Meta, Number, Table, Tap, TypeName,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub name: String,
    pub why: String,
    pub blocker: Blocker,
}

#[derive(Debug, Clone)]
pub struct Imported {
    pub table: Table,
    pub skipped: Vec<Skipped>,
}

impl Imported {
    pub fn summary(&self) -> String {
        format!(
            "{} cheats imported, {} skipped",
            self.table.cheats.len(),
            self.skipped.len()
        )
    }

    pub fn breakdown(&self) -> String {
        let mut scripts = 0usize;
        let mut symbols: Vec<&str> = Vec::new();
        let mut other = 0usize;

        for skip in &self.skipped {
            match &skip.blocker {
                Blocker::Script => scripts += 1,
                Blocker::Symbol(name) => {
                    if !symbols.contains(&name.as_str()) {
                        symbols.push(name);
                    }
                }
                Blocker::Other => other += 1,
            }
        }

        let anchored = self
            .skipped
            .iter()
            .filter(|s| matches!(s.blocker, Blocker::Symbol(_)))
            .count();

        let mut parts = Vec::new();
        if scripts > 0 {
            parts.push(format!(
                "{scripts} {} assembly that has to run inside the game",
                if scripts == 1 { "is" } else { "are" }
            ));
        }
        if anchored > 0 {
            symbols.sort_unstable();
            parts.push(format!(
                "{anchored} hang off {}, {}",
                symbols.join(" and "),
                if symbols.len() == 1 {
                    "a name one of those scripts writes down while it runs"
                } else {
                    "names those scripts write down while they run"
                }
            ));
        }
        if other > 0 {
            parts.push(format!("{other} could not be read"));
        }
        parts.join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    Script,
    Symbol(String),
    Other,
}

#[derive(Debug, Default, Clone)]
struct Entry {
    description: String,
    variable_type: String,
    address: String,
    offsets: Vec<i64>,
    script: bool,
    assembler: String,
    // "0:Off" lines out of a dropdown, if the author gave one
    dropdown: Vec<String>,
    hex: bool,
    // whatever cheat engine last saw at that address. the only honest starting
    // number we have, so it beats making one up
    last_seen: String,
    hotkeys: Vec<RawKey>,
    children: Vec<Entry>,
}

// a <Hotkey> block as written, sorted out into a schema hotkey later
#[derive(Debug, Default, Clone)]
struct RawKey {
    action: String,
    keys: Vec<u32>,
    value: String,
}

pub fn import(xml: &str, exe: &str, game_name: &str) -> Result<Imported, String> {
    let (roots, comments) = parse(xml)?;
    let mut cheats = Vec::new();
    let mut skipped = Vec::new();
    let mut used: HashSet<String> = HashSet::new();

    for entry in &roots {
        convert(entry, None, None, &mut cheats, &mut skipped, &mut used);
    }

    Ok(Imported {
        table: Table {
            meta: Meta::default(),
            game: Game {
                name: game_name.to_string(),
                exe: exe.to_string(),
                // the author's own words when the table carries any. "enable
                // in the main menu" style instructions live here
                notes: if comments.trim().is_empty() {
                    "Imported from a Cheat Engine table. Check it before trusting it.".into()
                } else {
                    comments.trim().to_string()
                },
                verified: Vec::new(),
                author: String::new(),
            },
            cheats,
        },
        skipped,
    })
}

fn parse(xml: &str) -> Result<(Vec<Entry>, String), String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut stack: Vec<Entry> = Vec::new();
    let mut roots: Vec<Entry> = Vec::new();
    let mut comments = String::new();
    let mut field = String::new();
    let mut depth_of_offsets = 0usize;
    let mut depth_of_hotkeys = 0usize;
    let mut depth = 0i32;

    loop {
        match reader.read_event() {
            Err(e) => return Err(format!("line {}: {e}", reader.buffer_position())),
            Ok(Event::Eof) => {
                if depth != 0 {
                    return Err(format!(
                        "the file ends with {depth} tags still open, so it is cut short"
                    ));
                }
                break;
            }

            Ok(Event::Start(tag)) => {
                depth += 1;
                let name = String::from_utf8_lossy(tag.name().as_ref()).to_string();
                match name.as_str() {
                    "CheatEntry" => stack.push(Entry::default()),
                    "Offsets" => depth_of_offsets += 1,
                    "Hotkeys" => depth_of_hotkeys += 1,
                    "Hotkey" if depth_of_hotkeys > 0 => {
                        if let Some(entry) = stack.last_mut() {
                            entry.hotkeys.push(RawKey::default());
                        }
                    }
                    other => field = other.to_string(),
                }
            }

            // <LastState Value="100" .../> carries no text, only attributes
            Ok(Event::Empty(tag)) => {
                if String::from_utf8_lossy(tag.name().as_ref()) == "LastState" {
                    if let (Some(entry), Some(seen)) = (stack.last_mut(), attribute(&tag, "Value"))
                    {
                        entry.last_seen = seen;
                    }
                }
            }

            Ok(Event::Text(text)) => {
                let value = text
                    .unescape()
                    .map_err(|e| e.to_string())?
                    .trim()
                    .to_string();
                if value.is_empty() {
                    continue;
                }
                // table level comments sit outside every entry, and they are
                // where authors write "enable this one in the main menu"
                if field == "Comments" && stack.is_empty() {
                    if !comments.is_empty() {
                        comments.push('\n');
                    }
                    comments.push_str(&value);
                    continue;
                }
                let Some(entry) = stack.last_mut() else {
                    continue;
                };

                match field.as_str() {
                    "Description" => entry.description = unquote(&value),
                    "VariableType" => entry.variable_type = value,
                    "Address" => entry.address = value,
                    "AssemblerScript" => {
                        entry.script = true;
                        if !entry.assembler.is_empty() {
                            entry.assembler.push('\n');
                        }
                        entry.assembler.push_str(&value);
                    }
                    "Offset" if depth_of_offsets > 0 => {
                        if let Ok(v) = i64::from_str_radix(value.trim_start_matches("0x"), 16) {
                            entry.offsets.push(v);
                        }
                    }
                    "DropDownList" => {
                        entry.dropdown = value
                            .lines()
                            .map(str::trim)
                            .filter(|l| !l.is_empty())
                            .map(str::to_string)
                            .collect();
                    }
                    "ShowAsHex" => entry.hex = value == "1",
                    "Action" if depth_of_hotkeys > 0 => {
                        if let Some(raw) = entry.hotkeys.last_mut() {
                            raw.action = value;
                        }
                    }
                    "Key" if depth_of_hotkeys > 0 => {
                        if let (Some(raw), Ok(code)) = (entry.hotkeys.last_mut(), value.parse()) {
                            raw.keys.push(code);
                        }
                    }
                    "Value" if depth_of_hotkeys > 0 => {
                        if let Some(raw) = entry.hotkeys.last_mut() {
                            raw.value = value;
                        }
                    }
                    _ => {}
                }
            }

            Ok(Event::End(tag)) => {
                depth -= 1;
                let name = String::from_utf8_lossy(tag.name().as_ref()).to_string();
                match name.as_str() {
                    "Offsets" => depth_of_offsets = depth_of_offsets.saturating_sub(1),
                    "Hotkeys" => depth_of_hotkeys = depth_of_hotkeys.saturating_sub(1),
                    "CheatEntry" => {
                        if let Some(done) = stack.pop() {
                            match stack.last_mut() {
                                Some(parent) => parent.children.push(done),
                                None => roots.push(done),
                            }
                        }
                    }
                    _ => {}
                }
                field.clear();
            }

            _ => {}
        }
    }

    Ok((roots, comments))
}

fn unquote(text: &str) -> String {
    text.trim().trim_matches('"').to_string()
}

fn attribute(tag: &quick_xml::events::BytesStart<'_>, want: &str) -> Option<String> {
    tag.attributes().flatten().find_map(|a| {
        (a.key.as_ref() == want.as_bytes())
            .then(|| String::from_utf8_lossy(a.value.as_ref()).to_string())
    })
}

fn convert(
    entry: &Entry,
    group: Option<&str>,
    parent: Option<&Locator>,
    out: &mut Vec<Cheat>,
    skipped: &mut Vec<Skipped>,
    used: &mut HashSet<String>,
) {
    let name = if entry.description.is_empty() {
        "unnamed".to_string()
    } else {
        entry.description.clone()
    };

    let is_script = entry.script
        || entry
            .variable_type
            .eq_ignore_ascii_case("Auto Assembler Script");

    if is_script {
        match script_cheat(entry, &name, group, used) {
            Ok(cheat) => out.push(cheat),
            Err(why) => skipped.push(Skipped {
                name: name.clone(),
                why,
                blocker: Blocker::Script,
            }),
        }
    }

    // worked out even for a heading that never becomes a cheat itself, because
    // everything nested under it is written as an offset from wherever it lands
    let here = locate(entry, parent);

    if !entry.children.is_empty() {
        for child in &entry.children {
            convert(child, Some(&name), here.as_ref(), out, skipped, used);
        }
        return;
    }

    if is_script {
        return;
    }

    let Some(kind) = value_kind(&entry.variable_type) else {
        skipped.push(Skipped {
            name,
            why: format!(
                "value type {:?} does not map onto anything",
                entry.variable_type
            ),
            blocker: Blocker::Other,
        });
        return;
    };

    let Some(locator) = here else {
        let why = if entry.address.is_empty() {
            "no address".to_string()
        } else if entry.address.trim_start().starts_with('+') {
            format!(
                "address {:?} is an offset from the group above it, and that group has no address we can use",
                entry.address
            )
        } else {
            format!(
                "address {:?} is not anchored to a module, so it means nothing on another machine",
                entry.address
            )
        };
        skipped.push(Skipped {
            name,
            why,
            blocker: Blocker::Other,
        });
        return;
    };

    let choices: Vec<Choice> = entry
        .dropdown
        .iter()
        .filter_map(|l| Choice::parse(l))
        .collect();

    out.push(Cheat {
        id: unique_id(&name, used),
        name: name.clone(),
        category: guess_category(&name, group),
        description: String::new(),
        hint: String::new(),
        locator: Some(locator),
        hotkeys: keys_of(entry),
        action: Action::Value {
            kind: TypeName(kind),
            value: starting_value(&name, kind, &entry.last_seen, &choices),
            min: None,
            max: None,
            choices,
            hex: entry.hex,
            lock: true,
        },
    });
}

// where an entry points, if anywhere we can write down. cheat engine nests
// entries under a group and writes the nested ones as a bare "+4C", meaning
// that far past whatever the group resolved to, so those need the parent
fn locate(entry: &Entry, parent: Option<&Locator>) -> Option<Locator> {
    let own: Vec<Hop> = entry
        .offsets
        .iter()
        .rev()
        .map(|v| Hop(*v as isize))
        .collect();

    let address = entry.address.trim();
    if let Some(rest) = address.strip_prefix('+') {
        // "+godmode" is an offset by a name only cheat engine knows, not a number
        let extra = i64::from_str_radix(rest.trim(), 16).ok()?;
        let mut base = under(parent?, extra as isize)?;
        if !own.is_empty() {
            hops_of(&mut base).extend(own);
        }
        return Some(base);
    }

    if let Some((module, offset)) = split_address(address) {
        return Some(Locator::Static {
            module,
            offset,
            hops: own,
        });
    }
    symbol_in(address).map(|symbol| Locator::Symbol { symbol, hops: own })
}

// one more offset onto the end of the parent's chain. it lands on the last hop
// rather than becoming a hop of its own, because a hop is a dereference and
// this is only ever arithmetic on the address the parent already found
fn under(parent: &Locator, extra: isize) -> Option<Locator> {
    let mut child = parent.clone();
    match &mut child {
        Locator::Static { offset, hops, .. } if hops.is_empty() => {
            *offset = offset.wrapping_add_signed(extra);
        }
        Locator::Pattern { offset, hops, .. } if hops.is_empty() => {
            *offset = offset.wrapping_add(extra as i64);
        }
        // a symbol on its own is an address, and there is nowhere in the schema
        // to say "that address plus four" without inventing a dereference
        Locator::Symbol { hops, .. } if hops.is_empty() => return None,
        other => hops_of(other).last_mut()?.0 += extra,
    }
    Some(child)
}

fn hops_of(locator: &mut Locator) -> &mut Vec<Hop> {
    match locator {
        Locator::Static { hops, .. }
        | Locator::Symbol { hops, .. }
        | Locator::Pattern { hops, .. } => hops,
    }
}

// cheat engine gives no hint of what a good number is, so this is a guess and
// the box is editable precisely because it is one. "infinite health" wants the
// ceiling, "carry weight" wants whatever the player types
fn starting_value(
    name: &str,
    kind: ValueKind,
    last_seen: &str,
    choices: &[Choice],
) -> Option<Number> {
    if let Some(first) = choices.first() {
        return Some(first.value);
    }
    if wants_the_ceiling(name) {
        return Some(freeze_value(kind));
    }
    Number::parse(last_seen)
}

fn wants_the_ceiling(name: &str) -> bool {
    let name = name.to_lowercase();
    [
        "infinite",
        "unlimited",
        "inf ",
        "max ",
        "no reload",
        "godmode",
        "god mode",
    ]
    .iter()
    .any(|w| name.contains(w))
        || name.starts_with("max")
}

fn script_cheat(
    entry: &Entry,
    name: &str,
    group: Option<&str>,
    used: &mut HashSet<String>,
) -> std::result::Result<Cheat, String> {
    let source = entry.assembler.replace("\r\n", "\n").replace('\r', "\n");
    let source = source.trim();
    if source.is_empty() {
        return Err("the script is empty".into());
    }
    if !source.to_ascii_uppercase().contains("[ENABLE]") {
        return Err("the script has no [ENABLE] section".into());
    }
    // one that will not parse can never run, so publishing it only wastes the
    // reader's time working out why the toggle does nothing
    freeplay_aa::parse(source).map_err(|e| e.to_string())?;

    Ok(Cheat {
        id: unique_id(name, used),
        name: name.to_string(),
        category: guess_category(name, group),
        description: String::new(),
        hint: String::new(),
        locator: None,
        hotkeys: keys_of(entry),
        action: Action::Script {
            source: source.to_string(),
        },
    })
}

// the hotkey blocks an entry carried, minus the ones nothing here can do
fn keys_of(entry: &Entry) -> Vec<Hotkey> {
    entry
        .hotkeys
        .iter()
        .filter_map(|raw| {
            let does = match raw.action.as_str() {
                // cheat engine writes an empty action for plain toggle
                ""
                | "Toggle Activation"
                | "Toggle Activation Allow Increase"
                | "Toggle Activation Allow Decrease" => Tap::Toggle,
                "Activate" => Tap::On,
                "Deactivate" => Tap::Off,
                "Set Value" => Tap::Set,
                _ => return None,
            };
            if raw.keys.is_empty() {
                return None;
            }
            if does == Tap::Set && raw.value.trim().is_empty() {
                return None;
            }
            Some(Hotkey {
                does,
                keys: raw.keys.clone(),
                value: (does == Tap::Set).then(|| raw.value.trim().to_string()),
            })
        })
        .collect()
}

fn symbol_in(address: &str) -> Option<String> {
    let head = address.trim().split(['+', '-']).next()?.trim();

    if head.is_empty() || head.contains('.') || head.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    if head.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Some(head.to_string());
    }
    None
}

fn split_address(address: &str) -> Option<(String, usize)> {
    let text = address.trim();
    let (module, rest) = text.split_once('+')?;

    let module = module.trim().trim_matches('"').to_string();
    if module.is_empty() || !module.contains('.') {
        return None;
    }

    let rest = rest.trim().trim_start_matches("0x");
    let offset = usize::from_str_radix(rest, 16).ok()?;
    Some((module, offset))
}

/* the other direction: what freeplay holds, written back out as a .CT that
cheat engine opens. pattern anchored values and raw byte patches cannot be
said in that file, so they are left out and counted */
pub fn export(table: &Table) -> (String, usize) {
    let mut out = String::new();
    let mut id = 0usize;
    let mut dropped = 0usize;

    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str("<CheatTable CheatEngineTableVersion=\"45\">\n  <CheatEntries>\n");

    // grouped the way the page shows them
    let mut groups: Vec<(&str, Vec<&Cheat>)> = Vec::new();
    for cheat in &table.cheats {
        let label = cheat.category.label();
        match groups.iter_mut().find(|(name, _)| *name == label) {
            Some((_, held)) => held.push(cheat),
            None => groups.push((label, vec![cheat])),
        }
    }

    for (label, cheats) in groups {
        id += 1;
        out.push_str(&format!(
            "    <CheatEntry>\n      <ID>{id}</ID>\n      \
             <Description>\"{}\"</Description>\n      <GroupHeader>1</GroupHeader>\n      \
             <CheatEntries>\n",
            esc(label)
        ));
        for cheat in cheats {
            match entry(cheat, &mut id) {
                Some(text) => out.push_str(&text),
                None => dropped += 1,
            }
        }
        out.push_str("      </CheatEntries>\n    </CheatEntry>\n");
    }

    out.push_str("  </CheatEntries>\n");
    let notes = table.game.notes.trim();
    if !notes.is_empty() && !notes.starts_with("Imported from a Cheat Engine table") {
        out.push_str(&format!("  <Comments>{}</Comments>\n", esc(notes)));
    }
    out.push_str("</CheatTable>\n");
    (out, dropped)
}

fn entry(cheat: &Cheat, id: &mut usize) -> Option<String> {
    let mut body = String::new();
    match &cheat.action {
        Action::Script { source } => {
            body.push_str(&format!(
                "          <VariableType>Auto Assembler Script</VariableType>\n          \
                 <AssemblerScript>{}</AssemblerScript>\n",
                esc(source)
            ));
        }
        Action::Value { kind, .. } | Action::Freeze { kind, .. } | Action::Set { kind, .. } => {
            let (address, hops) = spoken_address(cheat.locator.as_ref()?)?;
            body.push_str(&format!(
                "          <VariableType>{}</VariableType>\n          <Address>{}</Address>\n",
                variable_type(kind.0),
                esc(&address)
            ));
            let choices = cheat.action.choices();
            if !choices.is_empty() {
                let lines: Vec<String> = choices
                    .iter()
                    .map(|c| format!("{}:{}", c.value, c.label))
                    .collect();
                body.push_str(&format!(
                    "          <DropDownList DescriptionOnly=\"0\">{}</DropDownList>\n",
                    esc(&lines.join("\n"))
                ));
            }
            if cheat.action.shows_hex() {
                body.push_str("          <ShowAsHex>1</ShowAsHex>\n");
            }
            if !hops.is_empty() {
                body.push_str("          <Offsets>\n");
                // written deepest last, the order cheat engine keeps them
                for hop in hops.iter().rev() {
                    let offset = if hop.0 < 0 {
                        format!("-{:X}", -hop.0)
                    } else {
                        format!("{:X}", hop.0)
                    };
                    body.push_str(&format!("            <Offset>{offset}</Offset>\n"));
                }
                body.push_str("          </Offsets>\n");
            }
        }
        Action::Nop { .. } | Action::Bytes { .. } => return None,
    }

    *id += 1;
    Some(format!(
        "        <CheatEntry>\n          <ID>{}</ID>\n          \
         <Description>\"{}\"</Description>\n{body}{}        </CheatEntry>\n",
        id,
        esc(&cheat.name),
        keys_out(cheat)
    ))
}

fn spoken_address(locator: &Locator) -> Option<(String, Vec<Hop>)> {
    match locator {
        Locator::Static {
            module,
            offset,
            hops,
        } => {
            // a module with a space in its name goes back in quotes, the way
            // cheat engine writes it
            let held = if module.contains(' ') {
                format!("\"{module}\"+{offset:X}")
            } else {
                format!("{module}+{offset:X}")
            };
            Some((held, hops.clone()))
        }
        Locator::Symbol { symbol, hops } => Some((symbol.clone(), hops.clone())),
        Locator::Pattern { .. } => None,
    }
}

fn keys_out(cheat: &Cheat) -> String {
    if cheat.hotkeys.is_empty() {
        return String::new();
    }
    let mut out = String::from("          <Hotkeys>\n");
    for (n, held) in cheat.hotkeys.iter().enumerate() {
        let action = match held.does {
            Tap::Toggle => "Toggle Activation",
            Tap::On => "Activate",
            Tap::Off => "Deactivate",
            Tap::Set => "Set Value",
        };
        out.push_str(&format!(
            "            <Hotkey>\n              <Action>{action}</Action>\n              <Keys>\n"
        ));
        for key in &held.keys {
            out.push_str(&format!("                <Key>{key}</Key>\n"));
        }
        out.push_str("              </Keys>\n");
        if let Some(value) = &held.value {
            out.push_str(&format!("              <Value>{}</Value>\n", esc(value)));
        }
        out.push_str(&format!(
            "              <ID>{n}</ID>\n            </Hotkey>\n"
        ));
    }
    out.push_str("          </Hotkeys>\n");
    out
}

fn variable_type(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::F32 => "Float",
        ValueKind::F64 => "Double",
        other => match other.size() {
            1 => "Byte",
            2 => "2 Bytes",
            8 => "8 Bytes",
            _ => "4 Bytes",
        },
    }
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn value_kind(variable_type: &str) -> Option<ValueKind> {
    match variable_type.trim().to_ascii_lowercase().as_str() {
        "byte" => Some(ValueKind::U8),
        "2 bytes" => Some(ValueKind::I16),
        "4 bytes" => Some(ValueKind::I32),
        "8 bytes" => Some(ValueKind::I64),
        "float" => Some(ValueKind::F32),
        "double" => Some(ValueKind::F64),
        _ => None,
    }
}

fn freeze_value(kind: ValueKind) -> Number {
    match kind {
        ValueKind::F32 | ValueKind::F64 => Number::Float(9999.0),
        ValueKind::U8 => Number::Int(255),
        ValueKind::I16 | ValueKind::U16 => Number::Int(9999),
        _ => Number::Int(999999),
    }
}

fn guess_category(name: &str, group: Option<&str>) -> Category {
    if let Some(group) = group {
        if let Some(category) = category_named(group) {
            return category;
        }
    }
    category_named(name).unwrap_or(Category::Misc)
}

fn category_named(text: &str) -> Option<Category> {
    let text = text.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));

    if has(&[
        "health",
        "hp",
        "vitality",
        "life",
        "lives",
        "shield",
        "armor",
        "armour",
        "player",
        "vigor",
        "vigour",
        "mana",
        "focus",
        "adrenaline",
        "toxicity",
        "hunger",
        "thirst",
    ]) {
        Some(Category::Player)
    } else if has(&[
        "money",
        "gold",
        "cash",
        "credit",
        "coin",
        "oren",
        "ammo",
        "resource",
        "material",
        "gem",
        "inventory",
        "item",
        "crafting",
        "currency",
        "skill point",
        "perk",
    ]) {
        Some(Category::Resources)
    } else if has(&[
        "damage", "kill", "attack", "weapon", "reload", "recoil", "spread", "combat", "enemy",
    ]) {
        Some(Category::Combat)
    } else if has(&[
        "speed", "jump", "stamina", "energy", "sprint", "fly", "noclip", "movement", "teleport",
    ]) {
        Some(Category::Movement)
    } else if has(&[
        "time", "timer", "day", "night", "weather", "score", "wave", "level", "quest", "mission",
    ]) {
        Some(Category::Game)
    } else {
        None
    }
}

fn unique_id(name: &str, used: &mut HashSet<String>) -> String {
    let base: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    let base = if base.is_empty() {
        "cheat".to_string()
    } else {
        base
    };
    if used.insert(base.clone()) {
        return base;
    }
    for n in 2.. {
        let candidate = format!("{base}-{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<CheatTable CheatEngineTableVersion="42">
  <CheatEntries>
    <CheatEntry>
      <ID>1</ID>
      <Description>"Player"</Description>
      <GroupHeader>1</GroupHeader>
      <CheatEntries>
        <CheatEntry>
          <ID>2</ID>
          <Description>"Infinite Health"</Description>
          <VariableType>Float</VariableType>
          <Address>witcher2.exe+1A2B3C</Address>
          <Offsets>
            <Offset>1F0</Offset>
            <Offset>28</Offset>
          </Offsets>
        </CheatEntry>
        <CheatEntry>
          <ID>3</ID>
          <Description>"Infinite Vigor"</Description>
          <VariableType>4 Bytes</VariableType>
          <Address>witcher2.exe+2B3C4D</Address>
        </CheatEntry>
      </CheatEntries>
    </CheatEntry>
    <CheatEntry>
      <ID>4</ID>
      <Description>"Orens"</Description>
      <VariableType>4 Bytes</VariableType>
      <Address>witcher2.exe+3C4D5E</Address>
    </CheatEntry>
    <CheatEntry>
      <ID>5</ID>
      <Description>"God Mode Script"</Description>
      <VariableType>Auto Assembler Script</VariableType>
      <AssemblerScript>[ENABLE]
aobscanmodule(inj,witcher2.exe,89 41 04)
alloc(newmem,$1000)
</AssemblerScript>
    </CheatEntry>
    <CheatEntry>
      <ID>6</ID>
      <Description>"Scanned value"</Description>
      <VariableType>4 Bytes</VariableType>
      <Address>7FF6A1B2C3D4</Address>
    </CheatEntry>
  </CheatEntries>
</CheatTable>
"#;

    // the shape half of every real table uses: one script finds an object, a
    // heading points at what it wrote down, and the fields hang off the heading
    const NESTED: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<CheatTable CheatEngineTableVersion="42">
  <CheatEntries>
    <CheatEntry>
      <ID>1</ID>
      <Description>"Health"</Description>
      <GroupHeader>1</GroupHeader>
      <Address>healthPtr</Address>
      <Offsets>
        <Offset>0</Offset>
      </Offsets>
      <CheatEntries>
        <CheatEntry>
          <ID>2</ID>
          <Description>"Current"</Description>
          <VariableType>4 Bytes</VariableType>
          <Address>+4C</Address>
        </CheatEntry>
        <CheatEntry>
          <ID>3</ID>
          <Description>"Stats"</Description>
          <GroupHeader>1</GroupHeader>
          <Address>+68</Address>
          <CheatEntries>
            <CheatEntry>
              <ID>4</ID>
              <Description>"Strength"</Description>
              <VariableType>4 Bytes</VariableType>
              <Address>+8</Address>
            </CheatEntry>
          </CheatEntries>
        </CheatEntry>
        <CheatEntry>
          <ID>5</ID>
          <Description>"Named offset"</Description>
          <VariableType>Byte</VariableType>
          <Address>+godmode</Address>
        </CheatEntry>
      </CheatEntries>
    </CheatEntry>
    <CheatEntry>
      <ID>6</ID>
      <Description>"Settings"</Description>
      <GroupHeader>1</GroupHeader>
      <Address>game.exe+1000</Address>
      <CheatEntries>
        <CheatEntry>
          <ID>7</ID>
          <Description>"Game Speed"</Description>
          <VariableType>Float</VariableType>
          <Address>+58</Address>
        </CheatEntry>
      </CheatEntries>
    </CheatEntry>
  </CheatEntries>
</CheatTable>
"#;

    fn imported() -> Imported {
        import(SAMPLE, "witcher2.exe", "The Witcher 2").unwrap()
    }

    fn nested() -> Imported {
        import(NESTED, "game.exe", "Game").unwrap()
    }

    fn locator_for<'a>(out: &'a Imported, name: &str) -> &'a Locator {
        out.table
            .cheats
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} was not imported"))
            .locator
            .as_ref()
            .unwrap()
    }

    #[test]
    fn a_field_nested_under_a_pointer_lands_past_it() {
        match locator_for(&nested(), "Current") {
            Locator::Symbol { symbol, hops } => {
                assert_eq!(symbol, "healthPtr");
                // the group's own +0 and the child's +4C are one dereference,
                // not two, so they add up rather than stacking
                assert_eq!(hops, &[Hop(0x4c)]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nesting_two_deep_keeps_adding_up() {
        match locator_for(&nested(), "Strength") {
            Locator::Symbol { symbol, hops } => {
                assert_eq!(symbol, "healthPtr");
                assert_eq!(hops, &[Hop(0x68 + 0x8)]);
            }
            other => panic!("{other:?}"),
        }
    }

    // no pointer in the way, so the offset belongs on the module address
    #[test]
    fn a_field_under_a_plain_module_address_just_moves_it_along() {
        match locator_for(&nested(), "Game Speed") {
            Locator::Static {
                module,
                offset,
                hops,
            } => {
                assert_eq!(module, "game.exe");
                assert_eq!(*offset, 0x1058);
                assert!(hops.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_offset_by_a_name_rather_than_a_number_is_left_alone() {
        let out = nested();
        assert!(!out.table.cheats.iter().any(|c| c.name == "Named offset"));
        assert!(out
            .skipped
            .iter()
            .any(|s| s.name == "Named offset" && s.why.contains("group above")));
    }

    #[test]
    fn pulls_every_entry_across() {
        let out = imported();
        let names: Vec<&str> = out.table.cheats.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Infinite Health",
                "Infinite Vigor",
                "Orens",
                "God Mode Script"
            ]
        );
    }

    #[test]
    fn a_group_becomes_a_category_rather_than_a_cheat() {
        let out = imported();
        assert!(!out.table.cheats.iter().any(|c| c.name == "Player"));
        let health = &out.table.cheats[0];
        assert_eq!(health.category, Category::Player);
    }

    #[test]
    fn offsets_come_out_in_the_order_they_are_applied() {
        let out = imported();
        match out.table.cheats[0].locator.as_ref().unwrap() {
            Locator::Static {
                module,
                offset,
                hops,
            } => {
                assert_eq!(module, "witcher2.exe");
                assert_eq!(*offset, 0x1A2B3C);
                assert_eq!(hops, &[Hop(0x28), Hop(0x1F0)]);
            }
            other => panic!("expected a static locator, got {other:?}"),
        }
    }

    #[test]
    fn types_map_onto_ours() {
        let out = imported();
        let kinds: Vec<ValueKind> = out
            .table
            .cheats
            .iter()
            .filter_map(|c| c.action.kind())
            .collect();
        assert_eq!(kinds, [ValueKind::F32, ValueKind::I32, ValueKind::I32]);
    }

    #[test]
    fn a_plain_value_can_be_typed_into_rather_than_just_switched_on() {
        let out = imported();
        let orens = out.table.cheats.iter().find(|c| c.name == "Orens").unwrap();
        assert!(orens.action.takes_a_number());
        assert!(orens.action.holds());
    }

    // this is the bug the whole change is about. "multiplier" got frozen at
    // 9999 because nothing else was on offer
    #[test]
    fn a_number_nobody_can_guess_is_left_for_the_player() {
        let xml = SAMPLE.replace("Orens", "Carry Weight");
        let out = import(&xml, "witcher2.exe", "The Witcher 2").unwrap();
        let weight = out
            .table
            .cheats
            .iter()
            .find(|c| c.name == "Carry Weight")
            .unwrap();
        assert_eq!(weight.action.default_value(), None);
    }

    #[test]
    fn infinite_anything_still_gets_the_ceiling() {
        let out = imported();
        let health = &out.table.cheats[0];
        assert_eq!(health.name, "Infinite Health");
        assert_eq!(health.action.default_value(), Some(Number::Float(9999.0)));
    }

    #[test]
    fn the_last_number_cheat_engine_saw_is_the_starting_point() {
        let xml = SAMPLE.replace(
            "<Address>witcher2.exe+3C4D5E</Address>",
            "<Address>witcher2.exe+3C4D5E</Address><LastState Value=\"742\" RealAddress=\"0\"/>",
        );
        let out = import(&xml, "witcher2.exe", "The Witcher 2").unwrap();
        let orens = out.table.cheats.iter().find(|c| c.name == "Orens").unwrap();
        assert_eq!(orens.action.default_value(), Some(Number::Int(742)));
    }

    #[test]
    fn a_dropdown_comes_across_as_the_options_it_lists() {
        let xml = SAMPLE.replace(
            "<Address>witcher2.exe+3C4D5E</Address>",
            "<Address>witcher2.exe+3C4D5E</Address>\
             <DropDownList ReadOnly=\"1\">0:Easy\n1:Normal\n2:Hard</DropDownList>\
             <ShowAsHex>1</ShowAsHex>",
        );
        let out = import(&xml, "witcher2.exe", "The Witcher 2").unwrap();
        let orens = out.table.cheats.iter().find(|c| c.name == "Orens").unwrap();

        let labels: Vec<&str> = orens
            .action
            .choices()
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(labels, ["Easy", "Normal", "Hard"]);
        assert!(orens.action.shows_hex());
        assert_eq!(orens.action.default_value(), Some(Number::Int(0)));
    }

    #[test]
    fn scripts_come_across_as_scripts() {
        let out = imported();
        let script = out
            .table
            .cheats
            .iter()
            .find(|c| c.name == "God Mode Script")
            .expect("the script entry should be imported");
        assert!(script.action.is_script());
        assert!(script.locator.is_none());
    }

    #[test]
    fn bare_addresses_are_refused() {
        let out = imported();
        let bare = out
            .skipped
            .iter()
            .find(|s| s.name == "Scanned value")
            .expect("the bare address should be reported");
        assert!(bare.why.contains("not anchored"));
    }

    #[test]
    fn ids_are_slugs_and_never_collide() {
        let xml = SAMPLE.replace("Infinite Vigor", "Infinite Health");
        let out = import(&xml, "witcher2.exe", "The Witcher 2").unwrap();
        let ids: Vec<&str> = out.table.cheats.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "infinite-health",
                "infinite-health-2",
                "orens",
                "god-mode-script"
            ]
        );
    }

    #[test]
    fn a_quoted_module_name_still_splits() {
        assert_eq!(
            split_address(r#""Mass Effect.exe"+1234"#),
            Some(("Mass Effect.exe".to_string(), 0x1234))
        );
    }

    #[test]
    fn rubbish_is_an_error_not_a_panic() {
        assert!(import("<CheatTable><CheatEntries>", "a.exe", "A").is_err());
        let empty = import("<CheatTable/>", "a.exe", "A").unwrap();
        assert!(empty.table.cheats.is_empty());
    }

    #[test]
    fn the_imported_table_round_trips_through_toml() {
        let out = imported();
        let text = toml::to_string_pretty(&out.table).unwrap();
        let back = crate::Table::parse(&text).expect("what we write we must be able to read");
        assert_eq!(back.cheats.len(), out.table.cheats.len());
        assert_eq!(back.cheats[0].name, "Infinite Health");
    }

    fn keyed() -> Imported {
        let xml = SAMPLE.replace(
            "<Address>witcher2.exe+3C4D5E</Address>",
            "<Address>witcher2.exe+3C4D5E</Address>\
             <Hotkeys>\
               <Hotkey><Action>Toggle Activation</Action>\
                 <Keys><Key>112</Key></Keys><ID>0</ID></Hotkey>\
               <Hotkey><Action>Set Value</Action>\
                 <Keys><Key>17</Key><Key>113</Key></Keys>\
                 <Value>9999</Value><ID>1</ID></Hotkey>\
               <Hotkey><Action>Increase Value</Action>\
                 <Keys><Key>114</Key></Keys><Value>10</Value><ID>2</ID></Hotkey>\
             </Hotkeys>",
        );
        import(&xml, "witcher2.exe", "The Witcher 2").unwrap()
    }

    #[test]
    fn the_keys_the_author_bound_come_across() {
        let out = keyed();
        let orens = out.table.cheats.iter().find(|c| c.name == "Orens").unwrap();

        assert_eq!(
            orens.hotkeys.len(),
            2,
            "increase by a step is not a thing here"
        );
        assert_eq!(orens.hotkeys[0].does, Tap::Toggle);
        assert_eq!(orens.hotkeys[0].keys, [112]);
        assert_eq!(orens.hotkeys[1].does, Tap::Set);
        assert_eq!(orens.hotkeys[1].keys, [17, 113]);
        assert_eq!(orens.hotkeys[1].value.as_deref(), Some("9999"));
    }

    #[test]
    fn a_key_on_one_entry_does_not_leak_onto_the_next() {
        let out = keyed();
        for cheat in out.table.cheats.iter().filter(|c| c.name != "Orens") {
            assert!(
                cheat.hotkeys.is_empty(),
                "{} got keys from nowhere",
                cheat.name
            );
        }
    }

    #[test]
    fn bound_keys_survive_the_toml_round_trip() {
        let out = keyed();
        let text = toml::to_string_pretty(&out.table).unwrap();
        let back = crate::Table::parse(&text).expect("keys must not break the file");
        let orens = back.cheats.iter().find(|c| c.name == "Orens").unwrap();
        assert_eq!(orens.hotkeys.len(), 2);
        assert_eq!(orens.hotkeys[1].value.as_deref(), Some("9999"));
    }

    #[test]
    fn the_authors_notes_come_across_as_the_tables_notes() {
        let xml = SAMPLE.replacen(
            "<CheatEntries>",
            "<Comments>Enable the base script in the main menu first.\n\
             https://fearlessrevolution.com/viewtopic.php?t=1</Comments><CheatEntries>",
            1,
        );
        let out = import(&xml, "witcher2.exe", "The Witcher 2").unwrap();
        assert!(
            out.table.game.notes.starts_with("Enable the base script"),
            "{}",
            out.table.game.notes
        );
        assert!(out.table.game.notes.contains("fearlessrevolution.com"));
    }

    #[test]
    fn no_comments_still_says_where_the_table_came_from() {
        let out = imported();
        assert!(out
            .table
            .game
            .notes
            .starts_with("Imported from a Cheat Engine table"));
    }

    // what goes out has to come back in whole
    #[test]
    fn a_table_round_trips_through_export_and_import() {
        let out = keyed();
        let (xml, dropped) = export(&out.table);
        assert_eq!(dropped, 0, "nothing in the sample needs dropping");

        let back = import(&xml, "witcher2.exe", "The Witcher 2").unwrap();
        assert_eq!(back.table.cheats.len(), out.table.cheats.len());
        assert!(back.skipped.is_empty(), "{:?}", back.skipped);

        let orens = back
            .table
            .cheats
            .iter()
            .find(|c| c.name == "Orens")
            .unwrap();
        assert_eq!(orens.hotkeys.len(), 2, "the bound keys survive the trip");
        assert_eq!(orens.hotkeys[1].value.as_deref(), Some("9999"));

        let script = back
            .table
            .cheats
            .iter()
            .find(|c| c.name == "God Mode Script")
            .unwrap();
        assert!(
            matches!(&script.action, Action::Script { source } if source.contains("aobscanmodule"))
        );
    }

    #[test]
    fn deep_offset_chains_survive_the_round_trip() {
        let out = imported();
        let (xml, _) = export(&out.table);
        let back = import(&xml, "witcher2.exe", "The Witcher 2").unwrap();
        let health = back
            .table
            .cheats
            .iter()
            .find(|c| c.name == "Infinite Health")
            .unwrap();
        match health.locator.as_ref().unwrap() {
            Locator::Static {
                module,
                offset,
                hops,
            } => {
                assert_eq!(module, "witcher2.exe");
                assert_eq!(*offset, 0x1A2B3C);
                assert_eq!(hops, &[Hop(0x28), Hop(0x1F0)]);
            }
            other => panic!("expected the static chain back, got {other:?}"),
        }
    }
}
