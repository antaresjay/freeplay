// several tables for one game, shown as one list
//
// no single table has everything. one has health, another has ammo, a third
// has speed, and picking one meant giving up the other two. this folds the
// ones you have switched on into a single table, so the rest of the app
// carries on believing there is only ever one.

use std::collections::{HashMap, HashSet};

use freeplay_aa::script;
use freeplay_table::schema::{Action, Cheat, Locator, Table};

// where a cheat came from, so a card can say so and the ids stay apart
pub const MARK: char = '\u{1}';

/// The table a cheat came from, if it came out of a fold.
pub fn source_of(id: &str) -> Option<&str> {
    id.split_once(MARK).map(|(tag, _)| tag)
}

/// Several tables for the same game as one.
///
/// Tags name the source of each and have to be stable, because they end up in
/// the ids that saved values and armed flags are filed under.
pub fn fold(parts: Vec<(String, Table)>) -> Option<Table> {
    let mut parts = parts;
    if parts.is_empty() {
        return None;
    }
    if parts.len() == 1 {
        return Some(parts.remove(0).1);
    }

    let clashing = clashes(&parts);
    let mut whole = parts[0].1.clone();
    whole.cheats.clear();
    whole.game.name = parts[0].1.game.name.clone();

    let mut seen: HashSet<String> = HashSet::new();
    for (tag, table) in &parts {
        for cheat in &table.cheats {
            let mut cheat = cheat.clone();
            cheat.id = format!("{tag}{MARK}{}", cheat.id);
            // two tables really can offer the same thing twice, and both get
            // to be here. only an exact id repeat inside one tag is a mistake
            if !seen.insert(cheat.id.clone()) {
                continue;
            }
            rename(&mut cheat, tag, &clashing);
            whole.cheats.push(cheat);
        }
    }
    Some(whole)
}

/// Symbol names more than one of these tables declares.
///
/// Only these get renamed. A fold that changes nothing textually cannot break
/// a script, so the common case stays exactly as the author wrote it.
fn clashes(parts: &[(String, Table)]) -> HashSet<String> {
    let mut owners: HashMap<String, HashSet<&str>> = HashMap::new();

    for (tag, table) in parts {
        for name in declared(table) {
            owners.entry(name).or_default().insert(tag.as_str());
        }
    }
    owners
        .into_iter()
        .filter(|(_, tags)| tags.len() > 1)
        .map(|(name, _)| name)
        .collect()
}

// every symbol a table's scripts bring into being
fn declared(table: &Table) -> HashSet<String> {
    let mut names = HashSet::new();
    for cheat in &table.cheats {
        let Action::Script { source } = &cheat.action else {
            continue;
        };
        let Ok(halves) = script::parse(source) else {
            continue;
        };
        for half in [&halves.enable, &halves.disable] {
            for directive in &half.directives {
                let name = match directive {
                    script::Directive::AobScanModule { symbol, .. } => symbol,
                    script::Directive::AobScan { symbol, .. } => symbol,
                    script::Directive::Alloc { symbol, .. } => symbol,
                    script::Directive::Label(name) => name,
                    script::Directive::RegisterSymbol(name) => name,
                    script::Directive::Define { name, .. } => name,
                    _ => continue,
                };
                names.insert(name.trim().to_string());
            }
        }
    }
    names
}

fn rename(cheat: &mut Cheat, tag: &str, clashing: &HashSet<String>) {
    if clashing.is_empty() {
        return;
    }
    if let Action::Script { source } = &mut cheat.action {
        *source = swap(source, tag, clashing);
    }
    // a value cheat reaches its address through a symbol its own table's
    // script registered, so it has to follow the rename
    if let Some(Locator::Symbol { symbol, .. }) = &mut cheat.locator {
        if clashing.contains(symbol.trim()) {
            *symbol = tagged(tag, symbol.trim());
        }
    }
}

fn tagged(tag: &str, name: &str) -> String {
    format!("{tag}_{name}")
}

/* whole words only. a blind replace of "health" would also hit "healthMax"
and the middle of a comment, and a script that no longer parses is worse
than two tables that cannot be used together */
fn swap(source: &str, tag: &str, clashing: &HashSet<String>) -> String {
    let mut out = String::with_capacity(source.len());
    let mut word = String::new();

    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            if clashing.contains(word.as_str()) {
                out.push_str(&tagged(tag, word));
            } else {
                out.push_str(word);
            }
            word.clear();
        }
    };

    for ch in source.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            word.push(ch);
        } else {
            flush(&mut word, &mut out);
            out.push(ch);
        }
    }
    flush(&mut word, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use freeplay_core::value::ValueKind;
    use freeplay_table::schema::{Category, Game, TypeName};

    fn script(id: &str, source: &str) -> Cheat {
        Cheat {
            id: id.into(),
            name: id.into(),
            category: Category::Misc,
            description: String::new(),
            hint: String::new(),
            locator: None,
            hotkeys: vec![],
            action: Action::Script {
                source: source.into(),
            },
        }
    }

    fn value(id: &str, symbol: &str) -> Cheat {
        Cheat {
            id: id.into(),
            name: id.into(),
            category: Category::Misc,
            description: String::new(),
            hint: String::new(),
            locator: Some(Locator::Symbol {
                symbol: symbol.into(),
                hops: vec![],
            }),
            hotkeys: vec![],
            action: Action::Value {
                kind: TypeName(ValueKind::I32),
                value: None,
                min: None,
                max: None,
                choices: vec![],
                hex: false,
                lock: false,
            },
        }
    }

    fn table(exe: &str, cheats: Vec<Cheat>) -> Table {
        Table {
            game: Game {
                name: exe.into(),
                exe: exe.into(),
                notes: String::new(),
                verified: vec![],
                author: String::new(),
            },
            meta: Default::default(),
            cheats,
        }
    }

    const HEALTH: &str = "[ENABLE]\nalloc(health,4)\nregistersymbol(health)\n\
                          newmem:\n  mov [health],esi\n[DISABLE]\n";

    #[test]
    fn one_table_comes_back_untouched() {
        let only = table("g.exe", vec![script("a", HEALTH)]);
        let folded = fold(vec![("x".into(), only.clone())]).unwrap();
        assert_eq!(folded.cheats.len(), 1);
        // no tag on the id and not a character changed in the script
        assert_eq!(folded.cheats[0].id, "a");
        match (&folded.cheats[0].action, &only.cheats[0].action) {
            (Action::Script { source: got }, Action::Script { source: want }) => {
                assert_eq!(got, want)
            }
            _ => panic!("not a script"),
        }
    }

    #[test]
    fn nothing_to_fold_is_nothing() {
        assert!(fold(vec![]).is_none());
    }

    #[test]
    fn both_tables_bring_their_cheats() {
        let a = table("g.exe", vec![script("one", "[ENABLE]\n[DISABLE]\n")]);
        let b = table("g.exe", vec![script("two", "[ENABLE]\n[DISABLE]\n")]);
        let folded = fold(vec![("a".into(), a), ("b".into(), b)]).unwrap();
        assert_eq!(folded.cheats.len(), 2);
        assert_eq!(folded.cheats[0].id, format!("a{MARK}one"));
        assert_eq!(folded.cheats[1].id, format!("b{MARK}two"));
    }

    #[test]
    fn the_id_says_which_table_it_came_from() {
        let id = format!("2188{MARK}health");
        assert_eq!(source_of(&id), Some("2188"));
        assert_eq!(source_of("plain"), None);
    }

    /* the one that would corrupt values without a word on screen. both tables
    allocate their own four bytes and call them health, and the session
    keeps one symbol table, so the second overwrites the first */
    #[test]
    fn a_symbol_two_tables_both_declare_is_pulled_apart() {
        let a = table("g.exe", vec![script("s", HEALTH), value("v", "health")]);
        let b = table("g.exe", vec![script("s", HEALTH), value("v", "health")]);
        let folded = fold(vec![("a".into(), a), ("b".into(), b)]).unwrap();

        let sources: Vec<&str> = folded
            .cheats
            .iter()
            .filter_map(|c| match &c.action {
                Action::Script { source } => Some(source.as_str()),
                _ => None,
            })
            .collect();
        assert!(sources[0].contains("alloc(a_health,4)"), "{}", sources[0]);
        assert!(sources[1].contains("alloc(b_health,4)"), "{}", sources[1]);

        let symbols: Vec<String> = folded
            .cheats
            .iter()
            .filter_map(|c| match &c.locator {
                Some(Locator::Symbol { symbol, .. }) => Some(symbol.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(symbols, vec!["a_health", "b_health"]);
    }

    #[test]
    fn a_symbol_only_one_table_has_is_left_exactly_as_written() {
        let a = table("g.exe", vec![script("s", HEALTH), value("v", "health")]);
        let b = table(
            "g.exe",
            vec![script("t", "[ENABLE]\nalloc(ammo,4)\n[DISABLE]\n")],
        );
        let folded = fold(vec![("a".into(), a), ("b".into(), b)]).unwrap();

        for cheat in &folded.cheats {
            if let Action::Script { source } = &cheat.action {
                assert!(!source.contains("a_"), "{source}");
                assert!(!source.contains("b_"), "{source}");
            }
        }
    }

    #[test]
    fn a_longer_name_that_starts_the_same_is_not_touched() {
        let clashing: HashSet<String> = ["health".to_string()].into_iter().collect();
        let source = "mov [health],esi\nmov [healthMax],eax\n// health here too";
        let out = swap(source, "a", &clashing);
        assert!(out.contains("[a_health]"));
        assert!(out.contains("[healthMax]"), "{out}");
        assert!(out.contains("// a_health here too"), "{out}");
    }

    #[test]
    fn two_tables_can_both_offer_the_same_cheat() {
        let a = table("g.exe", vec![script("god", "[ENABLE]\n[DISABLE]\n")]);
        let b = table("g.exe", vec![script("god", "[ENABLE]\n[DISABLE]\n")]);
        let folded = fold(vec![("a".into(), a), ("b".into(), b)]).unwrap();
        assert_eq!(folded.cheats.len(), 2, "both survive, under different tags");
    }
}
