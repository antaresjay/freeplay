//! Reading Cheat Engine tables.
//!
//! There are thousands of `.CT` files in the wild for games nobody here is
//! ever going to sit down and reverse. Most of what they contain is a name, a
//! type, an address written as `game.exe+1A2B3C`, and a list of offsets, which
//! is exactly what a Freeplay locator is. So import that part.
//!
//! What cannot come across is the Auto Assembler: `.CT` scripts are assembly
//! with code caves and allocations, and running them means an assembler and
//! injecting code. Those entries are reported as skipped, by name, rather than
//! being quietly dropped so the table looks more complete than it is.

use std::collections::HashSet;

use freeplay_core::value::ValueKind;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::schema::{Action, Category, Cheat, Game, Hop, Locator, Number, Table, TypeName};

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

    /// Why nothing came across, counted by reason.
    ///
    /// Naming the first skipped entry and stopping is useless on a table where
    /// every entry failed for one of two reasons, which is the usual shape:
    /// the scripts do the work and the values hang off what the scripts found.
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

/// Why an entry could not come across, kept separate from the wording so the
/// reasons can be counted rather than only printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    /// Auto Assembler. Needs an assembler and code injection.
    Script,
    /// Anchored to a name a script registers at runtime.
    Symbol(String),
    Other,
}

/// One `<CheatEntry>`, with its children if it is a group.
#[derive(Debug, Default, Clone)]
struct Entry {
    description: String,
    variable_type: String,
    address: String,
    offsets: Vec<i64>,
    script: bool,
    children: Vec<Entry>,
}

pub fn import(xml: &str, exe: &str, game_name: &str) -> Result<Imported, String> {
    let roots = parse(xml)?;
    let mut cheats = Vec::new();
    let mut skipped = Vec::new();
    let mut used: HashSet<String> = HashSet::new();

    for entry in &roots {
        convert(entry, None, &mut cheats, &mut skipped, &mut used);
    }

    Ok(Imported {
        table: Table {
            game: Game {
                name: game_name.to_string(),
                exe: exe.to_string(),
                notes: "Imported from a Cheat Engine table. Check it before trusting it.".into(),
                verified: Vec::new(),
                author: String::new(),
            },
            cheats,
        },
        skipped,
    })
}

fn parse(xml: &str) -> Result<Vec<Entry>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    // One frame per open <CheatEntry>. Entries nest, so a stack is the whole
    // structure: a group is just an entry whose children got closed first.
    let mut stack: Vec<Entry> = Vec::new();
    let mut roots: Vec<Entry> = Vec::new();
    let mut field = String::new();
    let mut depth_of_offsets = 0usize;
    let mut depth = 0i32;

    loop {
        match reader.read_event() {
            Err(e) => return Err(format!("line {}: {e}", reader.buffer_position())),
            Ok(Event::Eof) => {
                // Running out of file inside an entry means the file is cut
                // short. Reading it leniently would import half a table.
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
                    other => field = other.to_string(),
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
                let Some(entry) = stack.last_mut() else {
                    continue;
                };

                match field.as_str() {
                    "Description" => entry.description = unquote(&value),
                    "VariableType" => entry.variable_type = value,
                    "Address" => entry.address = value,
                    "AssemblerScript" => entry.script = true,
                    "Offset" if depth_of_offsets > 0 => {
                        if let Ok(v) = i64::from_str_radix(value.trim_start_matches("0x"), 16) {
                            entry.offsets.push(v);
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

    Ok(roots)
}

fn unquote(text: &str) -> String {
    text.trim().trim_matches('"').to_string()
}

fn convert(
    entry: &Entry,
    group: Option<&str>,
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

    // A node with children is a heading. Its name is the best category hint
    // the file has, so carry it down. It can be a script as well as a heading,
    // which is the usual shape: the script finds the player and everything
    // underneath it reads fields off what it found. Report the script in that
    // case rather than treating the entry as nothing but a label.
    if !entry.children.is_empty() {
        if is_script {
            skipped.push(Skipped {
                name: name.clone(),
                why: "an auto assembler script, which needs code injection".into(),
                blocker: Blocker::Script,
            });
        }
        for child in &entry.children {
            convert(child, Some(&name), out, skipped, used);
        }
        return;
    }

    if is_script {
        skipped.push(Skipped {
            name,
            why: "an auto assembler script, which needs code injection".into(),
            blocker: Blocker::Script,
        });
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

    let Some((module, offset)) = split_address(&entry.address) else {
        let (why, blocker) = if entry.address.is_empty() {
            ("no address".to_string(), Blocker::Other)
        } else if let Some(symbol) = symbol_in(&entry.address) {
            (
                format!(
                    "anchored to {symbol}, which is not an address in the game but a name one of \
                     the scripts writes down while it runs"
                ),
                Blocker::Symbol(symbol),
            )
        } else {
            (
                format!(
                    "address {:?} is not anchored to a module, so it means nothing on another machine",
                    entry.address
                ),
                Blocker::Other,
            )
        };
        skipped.push(Skipped { name, why, blocker });
        return;
    };

    // Cheat Engine lists offsets last hop first, the way its pointer editor
    // shows them. Applying them in that order walks the chain backwards.
    let hops: Vec<Hop> = entry
        .offsets
        .iter()
        .rev()
        .map(|v| Hop(*v as isize))
        .collect();

    out.push(Cheat {
        id: unique_id(&name, used),
        name: name.clone(),
        category: guess_category(&name, group),
        description: String::new(),
        hint: String::new(),
        locator: Locator::Static {
            module,
            offset,
            hops,
        },
        action: Action::Freeze {
            kind: TypeName(kind),
            value: freeze_value(kind),
        },
    });
}

/// A name an Auto Assembler script registered, rather than an address.
///
/// This is how nearly every table worth having is built. The script scans for
/// an instruction, hooks it, and writes whatever register held the player into
/// a slot it allocated. Every value entry then hangs off that slot's name. The
/// name is meaningless until the script has run inside the game, so there is
/// nothing for Freeplay to point at, and saying "that is not a module" about
/// it explains nothing.
fn symbol_in(address: &str) -> Option<String> {
    let head = address.trim().split(['+', '-']).next()?.trim();

    // A module is a file name and a bare number is an address. Neither is this.
    if head.is_empty() || head.contains('.') || head.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    if head.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Some(head.to_string());
    }
    None
}

/// Cheat Engine writes `game.exe+1A2B3C`, sometimes quoted, sometimes with the
/// module in `"..."` because the name has a space in it.
fn split_address(address: &str) -> Option<(String, usize)> {
    let text = address.trim();
    let (module, rest) = text.split_once('+')?;

    let module = module.trim().trim_matches('"').to_string();
    // A bare hex address has no module and is worthless in a shared table.
    if module.is_empty() || !module.contains('.') {
        return None;
    }

    let rest = rest.trim().trim_start_matches("0x");
    let offset = usize::from_str_radix(rest, 16).ok()?;
    Some((module, offset))
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

/// A freeze needs something to hold. There is no right answer without knowing
/// the game, so hold what a "plenty of this" value usually looks like and let
/// the number be edited.
fn freeze_value(kind: ValueKind) -> Number {
    match kind {
        ValueKind::F32 | ValueKind::F64 => Number::Float(9999.0),
        ValueKind::U8 => Number::Int(255),
        ValueKind::I16 | ValueKind::U16 => Number::Int(9999),
        _ => Number::Int(999999),
    }
}

/// Cheat Engine tables have no categories, only whatever the author called
/// their groups. The group name is the stronger signal when it matches one of
/// ours outright, otherwise fall back to reading the cheat's own name.
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

    fn imported() -> Imported {
        import(SAMPLE, "witcher2.exe", "The Witcher 2").unwrap()
    }

    #[test]
    fn pulls_the_declarative_entries_across() {
        let out = imported();
        let names: Vec<&str> = out.table.cheats.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Infinite Health", "Infinite Vigor", "Orens"]);
    }

    #[test]
    fn a_group_becomes_a_category_rather_than_a_cheat() {
        let out = imported();
        assert!(!out.table.cheats.iter().any(|c| c.name == "Player"));
        let health = &out.table.cheats[0];
        assert_eq!(health.category, Category::Player);
    }

    /// Cheat Engine shows the last hop first, so importing them in file order
    /// would walk the chain backwards and read somewhere random.
    #[test]
    fn offsets_come_out_in_the_order_they_are_applied() {
        let out = imported();
        match &out.table.cheats[0].locator {
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
            .map(|c| match &c.action {
                Action::Freeze { kind, .. } => kind.0,
                other => panic!("expected a freeze, got {other:?}"),
            })
            .collect();
        assert_eq!(kinds, [ValueKind::F32, ValueKind::I32, ValueKind::I32]);
    }

    #[test]
    fn scripts_are_reported_rather_than_dropped() {
        let out = imported();
        let script = out
            .skipped
            .iter()
            .find(|s| s.name == "God Mode Script")
            .expect("the script entry should be reported");
        assert!(script.why.contains("auto assembler"));
    }

    /// An address with no module is wherever that machine happened to put it
    /// that day, so it is useless in a table meant to be shared.
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
        assert_eq!(ids, ["infinite-health", "infinite-health-2", "orens"]);
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
}
