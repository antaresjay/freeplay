//! the importer against a whole table rather than a snippet
//!
//! the unit tests in cheatengine.rs each poke one feature. this one takes a
//! table the shape of something somebody would actually publish, twenty
//! entries with four scripts and pointer chains hanging off the symbols those
//! scripts register, and checks the whole thing survives the trip.

use freeplay_table::cheatengine::import;
use freeplay_table::schema::{Action, Locator};

const EMBERFALL: &str = include_str!("data/emberfall.CT");

fn table() -> freeplay_table::cheatengine::Imported {
    import(EMBERFALL, "emberfall.exe", "Emberfall").expect("should parse")
}

#[test]
fn a_whole_table_comes_across_whole() {
    let imported = table();
    assert_eq!(imported.table.cheats.len(), 20);
    assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
}

#[test]
fn the_scripts_come_across_as_scripts() {
    let imported = table();
    let scripts: Vec<&str> = imported
        .table
        .cheats
        .iter()
        .filter(|c| c.action.is_script())
        .map(|c| c.name.as_str())
        .collect();

    assert_eq!(scripts.len(), 4);
    assert!(scripts.contains(&"Get Player Base"));
    assert!(scripts.contains(&"Infinite Health"));
    assert!(scripts.contains(&"Infinite Items"));
    assert!(scripts.contains(&"Modify Exp Gain"));
}

#[test]
fn a_script_keeps_the_assembly_it_needs_to_run() {
    let imported = table();
    let base = imported
        .table
        .cheats
        .iter()
        .find(|c| c.name == "Get Player Base")
        .unwrap();

    let Action::Script { source } = &base.action else {
        panic!("should be a script");
    };
    assert!(source.contains("[ENABLE]"));
    assert!(source.contains("[DISABLE]"));
    assert!(source.contains("aobscanmodule(getPlayer"));
    assert!(source.contains("mov [basePlayer],eax"));
}

// the file on disk is crlf, the way cheat engine writes it, and .gitattributes
// keeps it that way so this can fail
#[test]
fn the_line_endings_are_normalised_on_the_way_in() {
    assert!(EMBERFALL.contains('\r'), "the fixture should still be crlf");

    for cheat in table().table.cheats {
        if let Action::Script { source } = &cheat.action {
            assert!(!source.contains('\r'), "{}", cheat.name);
        }
    }
}

#[test]
fn a_script_has_no_address_of_its_own() {
    let imported = table();
    for cheat in imported
        .table
        .cheats
        .iter()
        .filter(|c| c.action.is_script())
    {
        assert!(cheat.locator.is_none(), "{}", cheat.name);
    }
}

#[test]
fn the_values_hang_off_the_symbol_their_script_writes() {
    let imported = table();
    let health = imported
        .table
        .cheats
        .iter()
        .find(|c| c.name == "Current Health")
        .unwrap();

    let Some(Locator::Symbol { symbol, hops }) = &health.locator else {
        panic!("should be anchored to a symbol, got {:?}", health.locator);
    };
    assert_eq!(symbol, "basePlayer");
    assert_eq!(
        hops.iter().map(|h| h.0).collect::<Vec<_>>(),
        vec![0x14, 0x8]
    );
}

// cheat engine lists offsets in the reverse of the order they are walked
#[test]
fn the_deep_chains_keep_all_their_hops() {
    let imported = table();
    let crowns = imported
        .table
        .cheats
        .iter()
        .find(|c| c.name == "Crowns")
        .unwrap();

    let Some(Locator::Symbol { symbol, hops }) = &crowns.locator else {
        panic!("should be anchored to a symbol");
    };
    assert_eq!(symbol, "basePlayer");
    assert_eq!(
        hops.iter().map(|h| h.0).collect::<Vec<_>>(),
        vec![0xC4, 0x0, 0x1C, 0x50, 0x2C]
    );
}

#[test]
fn the_multiplier_hangs_off_the_slot_its_own_script_allocates() {
    let imported = table();
    let multiplier = imported
        .table
        .cheats
        .iter()
        .find(|c| c.name == "Multiplier")
        .unwrap();

    let Some(Locator::Symbol { symbol, .. }) = &multiplier.locator else {
        panic!("should be anchored to a symbol");
    };
    assert_eq!(symbol, "expMultVal");
}

// one that is not anchored to a script at all
#[test]
fn a_plain_module_address_stays_a_module_address() {
    let imported = table();
    let speed = imported
        .table
        .cheats
        .iter()
        .find(|c| c.name == "Game Speed")
        .unwrap();

    let Some(Locator::Static { module, offset, .. }) = &speed.locator else {
        panic!("should be a module offset, got {:?}", speed.locator);
    };
    assert_eq!(module, "emberfall.exe");
    assert_eq!(*offset, 0x1A2B3C);
}

#[test]
fn a_group_heading_still_gives_its_children_a_category() {
    let imported = table();
    let by_name = |want: &str| {
        imported
            .table
            .cheats
            .iter()
            .find(|c| c.name == want)
            .unwrap_or_else(|| panic!("no cheat called {want}"))
    };

    // the heading is mapped onto one of ours, so Inventory lands on Resources
    assert_eq!(by_name("Current Health").category.label(), "Player");
    assert_eq!(by_name("Crowns").category.label(), "Resources");

    // and a heading that means nothing to us leaves each child to its own
    // name, which is why one group can come out split across two categories
    assert_eq!(by_name("Game Speed").category.label(), "Movement");
    assert_eq!(by_name("Time Of Day").category.label(), "Game");
}

#[test]
fn what_it_produces_is_a_table_freeplay_will_load() {
    let imported = table();
    imported.table.validate().expect("should be valid");

    let text = toml::to_string_pretty(&imported.table).unwrap();
    let round = freeplay_table::Table::parse(&text).expect("should reparse");
    assert_eq!(round.cheats.len(), 20);
}

// a table is published under its fingerprint. change what the importer does
// and the same .CT converts to something the service treats as a different
// table, forking every copy people have already downloaded
#[test]
fn converting_the_same_table_twice_gives_the_same_fingerprint() {
    let once = freeplay_table::fingerprint::fingerprint(&table().table);
    let twice = freeplay_table::fingerprint::fingerprint(&table().table);
    assert_eq!(once, twice);
    assert!(
        once.starts_with("e6e7762cddf3"),
        "the importer changed what it produces, this is now {once}"
    );
}

// a multiplier is a number the player picks. a maximum is not
#[test]
fn a_multiplier_is_left_for_the_player_and_a_maximum_is_not() {
    let imported = table();
    let by_name = |want: &str| {
        imported
            .table
            .cheats
            .iter()
            .find(|c| c.name == want)
            .unwrap_or_else(|| panic!("no cheat called {want}"))
    };

    assert!(by_name("Multiplier").action.takes_a_number());
    assert_eq!(by_name("Multiplier").action.default_value(), None);
    assert_eq!(by_name("Crowns").action.default_value(), None);
    assert!(by_name("Max Health").action.default_value().is_some());
}

// the last number cheat engine had on screen is a better starting point than
// nothing, and a dropdown is better than either
#[test]
fn what_cheat_engine_last_saw_carries_over() {
    let imported = table();
    let by_name = |want: &str| {
        imported
            .table
            .cheats
            .iter()
            .find(|c| c.name == want)
            .unwrap_or_else(|| panic!("no cheat called {want}"))
    };

    assert!(by_name("Weight Limit").action.default_value().is_some());
    assert!(by_name("Arrows").action.shows_hex());

    let labels: Vec<&str> = by_name("Time Of Day")
        .action
        .choices()
        .iter()
        .map(|c| c.label.as_str())
        .collect();
    assert_eq!(labels, ["Dawn", "Noon", "Dusk", "Night"]);
}
