use freeplay_table::cheatengine::import;
use freeplay_table::schema::{Action, Locator};

const WITCHER2: &str = include_str!("data/witcher2.CT");

fn witcher() -> freeplay_table::cheatengine::Imported {
    import(WITCHER2, "witcher2.exe", "The Witcher 2").expect("should parse")
}

#[test]
fn a_published_table_comes_across_whole() {
    let imported = witcher();
    assert_eq!(imported.table.cheats.len(), 23);
    assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
}

#[test]
fn the_scripts_come_across_as_scripts() {
    let imported = witcher();
    let scripts: Vec<&str> = imported
        .table
        .cheats
        .iter()
        .filter(|c| c.action.is_script())
        .map(|c| c.name.as_str())
        .collect();

    assert_eq!(scripts.len(), 4);
    assert!(scripts.contains(&"Get Witcher Base"));
    assert!(scripts.contains(&"Inf Health"));
    assert!(scripts.contains(&"Inf Items"));
    assert!(scripts.contains(&"Modify Exp Gain"));
}

#[test]
fn a_script_keeps_the_assembly_it_needs_to_run() {
    let imported = witcher();
    let base = imported
        .table
        .cheats
        .iter()
        .find(|c| c.name == "Get Witcher Base")
        .unwrap();

    let Action::Script { source } = &base.action else {
        panic!("should be a script");
    };
    assert!(source.contains("[ENABLE]"));
    assert!(source.contains("[DISABLE]"));
    assert!(source.contains("aobscanmodule(getWitcher"));
    assert!(source.contains("mov [baseWitcher],eax"));
    assert!(!source.contains('\r'), "line endings should be normalised");
}

#[test]
fn a_script_has_no_address_of_its_own() {
    let imported = witcher();
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
    let imported = witcher();
    let health = imported
        .table
        .cheats
        .iter()
        .find(|c| c.name == "Current Health")
        .unwrap();

    let Some(Locator::Symbol { symbol, hops }) = &health.locator else {
        panic!("should be anchored to a symbol, got {:?}", health.locator);
    };
    assert_eq!(symbol, "baseWitcher");
    assert_eq!(
        hops.iter().map(|h| h.0).collect::<Vec<_>>(),
        vec![0x14, 0x8]
    );
}

#[test]
fn the_deep_chains_keep_all_their_hops() {
    let imported = witcher();
    let weight = imported
        .table
        .cheats
        .iter()
        .find(|c| c.name == "Weight Limit")
        .unwrap();

    let Some(Locator::Symbol { symbol, hops }) = &weight.locator else {
        panic!("should be anchored to a symbol");
    };
    assert_eq!(symbol, "baseWitcher");
    assert_eq!(
        hops.iter().map(|h| h.0).collect::<Vec<_>>(),
        vec![0xC4, 0x0, 0x1C, 0x50, 0x2C]
    );
}

#[test]
fn the_multiplier_hangs_off_the_slot_its_own_script_allocates() {
    let imported = witcher();
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

#[test]
fn a_group_heading_still_gives_its_children_a_category() {
    let imported = witcher();
    let health = imported
        .table
        .cheats
        .iter()
        .find(|c| c.name == "Current Health")
        .unwrap();
    assert_eq!(health.category.label(), "Player");
}

#[test]
fn what_it_produces_is_a_table_freeplay_will_load() {
    let imported = witcher();
    imported.table.validate().expect("should be valid");

    let text = toml::to_string_pretty(&imported.table).unwrap();
    let round = freeplay_table::Table::parse(&text).expect("should reparse");
    assert_eq!(round.cheats.len(), 23);
}

// this table is already published under this fingerprint. making values
// editable must not change what the table does, and so must not fork every
// copy people have already downloaded
#[test]
fn making_the_values_editable_did_not_fork_the_published_table() {
    let imported = witcher();
    let found = freeplay_table::fingerprint::fingerprint(&imported.table);
    assert!(
        found.starts_with("3ab6833698e5"),
        "the published copy is 3ab6833698e5, this one is {found}"
    );
}

// the point of the whole change: an experience multiplier is a number the
// player picks, not something to hold at 9999
#[test]
fn a_multiplier_is_left_for_the_player_and_a_maximum_is_not() {
    let imported = witcher();
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
    assert_eq!(by_name("Orens").action.default_value(), None);
    assert!(by_name("Max Vitality").action.default_value().is_some());
}
