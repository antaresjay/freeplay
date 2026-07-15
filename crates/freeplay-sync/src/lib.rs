//! Keeping the shipped cheat tables up to date.
//!
//! The point of this is that nobody has to go and find anything. Freeplay
//! fetches a small index from the project's own repository, downloads the
//! tables that are new or have changed, and puts them somewhere the app
//! already looks. A game gets support when somebody sends a pull request, and
//! everybody has it the next time they open the app.
//!
//! It only ever talks to one host, only over https, only reads, and sends
//! nothing about you. There is no account and no identifier. Turn it off in
//! settings and Freeplay works exactly as it did, on whatever is already on
//! disk.

pub mod community;
pub mod http;
pub mod rank;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use freeplay_table::Table;
use serde::{Deserialize, Serialize};

/// Where the published tables live. Raw files from the repository's main
/// branch, so a merged pull request is live immediately with nothing to
/// deploy.
pub const CATALOG: &str =
    "https://raw.githubusercontent.com/antaresjay/freeplay/main/tables/index.json";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Catalog {
    pub version: u32,
    #[serde(default)]
    pub tables: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Entry {
    /// Process this table is for, which is also how the app matches it.
    pub exe: String,
    pub game: String,
    /// File name under tables/ in the repository.
    pub file: String,
    /// Bumped by whoever changes the table. Nothing is downloaded twice.
    #[serde(default)]
    pub revision: u32,
    #[serde(default)]
    pub cheats: u32,
}

/// What we already have, so a run that changes nothing costs one small file.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
struct Held {
    #[serde(default)]
    revisions: HashMap<String, u32>,
}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: usize,
    pub failed: Vec<(String, String)>,
}

impl Report {
    pub fn changed(&self) -> bool {
        !self.added.is_empty() || !self.updated.is_empty()
    }

    pub fn summary(&self) -> String {
        if !self.changed() {
            return format!("Already up to date, {} tables", self.unchanged);
        }
        let mut parts = Vec::new();
        if !self.added.is_empty() {
            parts.push(format!("{} new", self.added.len()));
        }
        if !self.updated.is_empty() {
            parts.push(format!("{} updated", self.updated.len()));
        }
        format!("{} tables: {}", parts.join(", "), first_few(self))
    }
}

fn first_few(report: &Report) -> String {
    let mut names: Vec<&str> = report
        .added
        .iter()
        .chain(report.updated.iter())
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    if names.len() > 3 {
        let rest = names.len() - 3;
        format!("{}, and {rest} more", names[..3].join(", "))
    } else {
        names.join(", ")
    }
}

fn base_of(url: &str) -> String {
    match url.rfind('/') {
        Some(at) => url[..at + 1].to_string(),
        None => url.to_string(),
    }
}

/// Fetch the index and bring `into` up to date with it.
pub fn update(into: &Path) -> Result<Report, String> {
    update_from(CATALOG, into, &http::get)
}

/// Split out so the whole thing can be tested without a network.
pub fn update_from(
    catalog_url: &str,
    into: &Path,
    fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<Report, String> {
    let raw = fetch(catalog_url)?;
    let catalog: Catalog = serde_json::from_slice(&raw)
        .map_err(|e| format!("the table index does not make sense: {e}"))?;

    if catalog.version != 1 {
        return Err(format!(
            "the table index is version {}, this build understands 1. Update Freeplay.",
            catalog.version
        ));
    }

    std::fs::create_dir_all(into).map_err(|e| format!("could not make {}: {e}", into.display()))?;

    let held_path = into.join("held.json");
    let mut held: Held = std::fs::read_to_string(&held_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();

    let base = base_of(catalog_url);
    let mut report = Report::default();

    for entry in &catalog.tables {
        // a file name from the network is not allowed to be a path. the colon
        // counts, C:x has a drive on it and joining that onto a folder throws
        // the folder away
        if entry.file.contains(['/', '\\', ':']) || entry.file.contains("..") {
            report.failed.push((
                entry.game.clone(),
                format!("bad file name {:?}", entry.file),
            ));
            continue;
        }

        let destination = into.join(&entry.file);
        let known = held.revisions.get(&entry.file).copied();
        if known == Some(entry.revision) && destination.is_file() {
            report.unchanged += 1;
            continue;
        }

        match fetch_one(&format!("{base}{}", entry.file), fetch) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&destination, &text) {
                    report.failed.push((entry.game.clone(), e.to_string()));
                    continue;
                }
                held.revisions.insert(entry.file.clone(), entry.revision);
                if known.is_some() {
                    report.updated.push(entry.game.clone());
                } else {
                    report.added.push(entry.game.clone());
                }
            }
            Err(e) => report.failed.push((entry.game.clone(), e)),
        }
    }

    if let Ok(text) = serde_json::to_string_pretty(&held) {
        let _ = std::fs::write(&held_path, text);
    }

    for (game, why) in &report.failed {
        tracing::warn!("table for {game} not updated: {why}");
    }
    tracing::info!("{}", report.summary());
    Ok(report)
}

/// A downloaded table has to parse and validate before it is allowed to sit in
/// the folder the app loads from. A table describes writes into a game's
/// memory, so a broken one is not something to find out about later.
fn fetch_one(url: &str, fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> Result<String, String> {
    let bytes = fetch(url)?;
    let text = String::from_utf8(bytes).map_err(|_| "that table is not text".to_string())?;

    let table = Table::parse(&text).map_err(|e| format!("will not parse: {e}"))?;
    table.validate()?;
    Ok(text)
}

/// Where downloaded tables are kept, which is not the folder next to the
/// executable: that one belongs to whoever installed the app.
pub fn cache_dir(settings_file: &Path) -> PathBuf {
    settings_file
        .parent()
        .unwrap_or(Path::new("."))
        .join("tables")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const INDEX: &str = r#"{
        "version": 1,
        "tables": [
            {"exe":"witcher2.exe","game":"The Witcher 2","file":"witcher2.toml","revision":2,"cheats":4}
        ]
    }"#;

    const TABLE: &str = r#"
        [game]
        name = "The Witcher 2"
        exe = "witcher2.exe"

        [[cheat]]
        id = "vitality"
        name = "Infinite Vitality"
        type = "freeze"
        value_type = "f32"
        value = 1000

        [cheat.locator]
        find = "static"
        module = "witcher2.exe"
        offset = "0x1A2B3C"
    "#;

    fn server(
        pairs: Vec<(&'static str, &'static str)>,
    ) -> impl Fn(&str) -> Result<Vec<u8>, String> {
        let asked = RefCell::new(Vec::<String>::new());
        move |url: &str| {
            asked.borrow_mut().push(url.to_string());
            pairs
                .iter()
                .find(|(name, _)| url.ends_with(name))
                .map(|(_, body)| body.as_bytes().to_vec())
                .ok_or_else(|| format!("404 {url}"))
        }
    }

    /// One directory per test. These run in parallel in the same process, so a
    /// shared path means one test wiping another's downloads mid run.
    fn temp(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("freeplay-sync-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn downloads_a_table_it_does_not_have() {
        let dir = temp("downloads_a_table_it_does_not_have");
        let fetch = server(vec![("index.json", INDEX), ("witcher2.toml", TABLE)]);
        let report = update_from(CATALOG, &dir, &fetch).unwrap();

        assert_eq!(report.added, ["The Witcher 2"]);
        assert!(dir.join("witcher2.toml").is_file());
    }

    #[test]
    fn does_not_download_the_same_revision_twice() {
        let dir = temp("does_not_download_the_same_revision_twice");
        let fetch = server(vec![("index.json", INDEX), ("witcher2.toml", TABLE)]);
        update_from(CATALOG, &dir, &fetch).unwrap();

        let again = update_from(CATALOG, &dir, &fetch).unwrap();
        assert!(!again.changed());
        assert_eq!(again.unchanged, 1);
    }

    #[test]
    fn a_bumped_revision_is_fetched_again() {
        let dir = temp("a_bumped_revision_is_fetched_again");
        let fetch = server(vec![("index.json", INDEX), ("witcher2.toml", TABLE)]);
        update_from(CATALOG, &dir, &fetch).unwrap();

        let bumped = INDEX.replace("\"revision\":2", "\"revision\":3");
        let bumped: &'static str = Box::leak(bumped.into_boxed_str());
        let fetch = server(vec![("index.json", bumped), ("witcher2.toml", TABLE)]);
        let report = update_from(CATALOG, &dir, &fetch).unwrap();
        assert_eq!(report.updated, ["The Witcher 2"]);
    }

    /// A table describes writes into a game's memory. One that will not parse
    /// must never reach the folder the app loads from.
    #[test]
    fn a_broken_table_is_never_written() {
        let dir = temp("a_broken_table_is_never_written");
        let fetch = server(vec![
            ("index.json", INDEX),
            ("witcher2.toml", "not toml {{{"),
        ]);
        let report = update_from(CATALOG, &dir, &fetch).unwrap();

        assert!(report.added.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert!(!dir.join("witcher2.toml").exists());
    }

    /// The index comes off the network, so a file name in it is not allowed to
    /// climb out of the folder.
    #[test]
    fn a_file_name_cannot_be_a_path() {
        let dir = temp("a_file_name_cannot_be_a_path");
        let evil = r#"{"version":1,"tables":[
            {"exe":"a.exe","game":"Evil","file":"../../settings.json","revision":1}
        ]}"#;
        let evil: &'static str = Box::leak(evil.to_string().into_boxed_str());
        let fetch = server(vec![("index.json", evil), ("settings.json", TABLE)]);

        let report = update_from(CATALOG, &dir, &fetch).unwrap();
        assert!(report.added.is_empty());
        assert!(report.failed[0].1.contains("bad file name"));
    }

    // no slash in it, so the first check let it through. on windows a name
    // with a drive on it replaces whatever it is joined to, so this landed
    // wherever the process happened to be sitting on that drive
    #[test]
    fn nor_a_bare_drive_letter() {
        let dir = temp("nor_a_bare_drive_letter");
        let evil = r#"{"version":1,"tables":[
            {"exe":"a.exe","game":"Evil","file":"C:freeplay-escaped.toml","revision":1}
        ]}"#;
        let evil: &'static str = Box::leak(evil.to_string().into_boxed_str());
        let fetch = server(vec![("index.json", evil), ("freeplay-escaped.toml", TABLE)]);

        let report = update_from(CATALOG, &dir, &fetch).unwrap();
        assert!(report.added.is_empty());
        assert!(report.failed[0].1.contains("bad file name"));
    }

    #[test]
    fn an_index_from_the_future_says_so_rather_than_guessing() {
        let dir = temp("an_index_from_the_future_says_so_rather_than_guessing");
        let future = r#"{"version":9,"tables":[]}"#;
        let future: &'static str = Box::leak(future.to_string().into_boxed_str());
        let fetch = server(vec![("index.json", future)]);

        let e = update_from(CATALOG, &dir, &fetch).unwrap_err();
        assert!(e.contains("Update Freeplay"));
    }

    #[test]
    fn one_bad_table_does_not_stop_the_others() {
        let dir = temp("one_bad_table_does_not_stop_the_others");
        let two = r#"{"version":1,"tables":[
            {"exe":"a.exe","game":"Broken","file":"broken.toml","revision":1},
            {"exe":"witcher2.exe","game":"The Witcher 2","file":"witcher2.toml","revision":1}
        ]}"#;
        let two: &'static str = Box::leak(two.to_string().into_boxed_str());
        let fetch = server(vec![
            ("index.json", two),
            ("broken.toml", "nonsense {{"),
            ("witcher2.toml", TABLE),
        ]);

        let report = update_from(CATALOG, &dir, &fetch).unwrap();
        assert_eq!(report.added, ["The Witcher 2"]);
        assert_eq!(report.failed.len(), 1);
    }
}
