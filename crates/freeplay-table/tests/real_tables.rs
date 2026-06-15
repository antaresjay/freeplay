//! Import against a table somebody actually published, rather than one written
//! to make the importer look good.
//!
//! aSwedishMagyar's Witcher 2 table is the normal shape for anything decent: a
//! script finds the player by hooking an instruction and copying a register
//! into a slot it allocated, and every value entry hangs off that slot's name.
//! None of it can come across without running code inside the game, and the
//! point of this test is that Freeplay says so precisely instead of blaming
//! the first entry it tripped over.

use freeplay_table::cheatengine::{import, Blocker};

const WITCHER2: &str = include_str!("data/witcher2.CT");

/// 25 entries: 4 scripts, 19 values, and 2 pure group headings that are labels
/// rather than cheats. So 23 have to be reported, and none can come across.
#[test]
fn reads_a_published_table_without_falling_over() {
    let imported = import(WITCHER2, "witcher2.exe", "The Witcher 2").expect("should parse");
    assert_eq!(imported.table.cheats.len(), 0);
    assert_eq!(imported.skipped.len(), 23);
}

/// Four scripts, and two of them are also group headings. Those two used to be
/// treated as nothing but labels and vanished from the report entirely.
#[test]
fn counts_scripts_that_are_also_headings() {
    let imported = import(WITCHER2, "witcher2.exe", "The Witcher 2").unwrap();
    let scripts: Vec<&str> = imported
        .skipped
        .iter()
        .filter(|s| s.blocker == Blocker::Script)
        .map(|s| s.name.as_str())
        .collect();

    assert_eq!(scripts.len(), 4);
    assert!(scripts.contains(&"Get Witcher Base"), "{scripts:?}");
    assert!(scripts.contains(&"Modify Exp Gain"), "{scripts:?}");
}

/// The 19 value entries are anchored to `baseWitcher` and `expMultVal`, names
/// the scripts register at runtime. Calling those "not anchored to a module"
/// was true and useless.
#[test]
fn names_the_symbol_the_values_depend_on() {
    let imported = import(WITCHER2, "witcher2.exe", "The Witcher 2").unwrap();

    let health = imported
        .skipped
        .iter()
        .find(|s| s.name == "Current Health")
        .expect("Current Health should be listed");

    assert_eq!(health.blocker, Blocker::Symbol("baseWitcher".into()));
    assert!(health.why.contains("baseWitcher"), "{}", health.why);
    assert!(!health.why.contains("another machine"), "{}", health.why);
}

#[test]
fn the_breakdown_explains_the_whole_table_in_one_line() {
    let imported = import(WITCHER2, "witcher2.exe", "The Witcher 2").unwrap();
    let text = imported.breakdown();

    assert!(text.contains("4 are assembly"), "{text}");
    assert!(text.contains("19 hang off"), "{text}");
    assert!(text.contains("baseWitcher"), "{text}");
    assert!(text.contains("expMultVal"), "{text}");
}

/// Groups give their children a category, and that has to survive the entry
/// also being a script.
#[test]
fn group_headings_still_reach_their_children() {
    let imported = import(WITCHER2, "witcher2.exe", "The Witcher 2").unwrap();
    let names: Vec<&str> = imported.skipped.iter().map(|s| s.name.as_str()).collect();

    assert!(names.contains(&"Weight Limit"), "{names:?}");
    assert!(names.contains(&"Multiplier"), "{names:?}");
    assert!(names.contains(&"Orens"), "{names:?}");
}
