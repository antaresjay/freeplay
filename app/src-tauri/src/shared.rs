// the community side of things: what other people have shared for a game,
// pulling one in, saying whether it worked, and who you publish as

use std::path::PathBuf;

use freeplay_id::Identity;
use freeplay_sync::community::{Community, Listing, Live, Sort, ENDPOINT};
use freeplay_table::Table;
use serde::Serialize;

use crate::settings;

pub fn endpoint() -> String {
    std::env::var("FREEPLAY_SERVICE").unwrap_or_else(|_| ENDPOINT.to_string())
}

pub fn identity_path() -> PathBuf {
    settings::path()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("identity.json")
}

pub fn me() -> Option<Identity> {
    Identity::load(&identity_path()).ok().flatten()
}

#[derive(Serialize)]
pub struct Shared {
    pub id: i64,
    pub game: String,
    pub by: String,
    pub cheats: u32,
    pub up: i64,
    pub down: i64,
    pub downloads: i64,
    pub built_for: String,
    pub added: i64,
    pub standing: String,
    // already sitting in the tables folder
    pub installed: bool,
}

impl Shared {
    fn from(row: Listing, installed: bool) -> Self {
        Self {
            standing: row.standing(),
            id: row.id,
            game: row.game,
            by: row.submitted_by,
            cheats: row.cheats,
            up: row.up,
            down: row.down,
            downloads: row.downloads,
            built_for: row.built_for,
            added: row.created_at,
            installed,
        }
    }
}

#[derive(Serialize)]
pub struct SortOption {
    pub key: String,
    pub label: String,
}

pub fn sorts() -> Vec<SortOption> {
    Sort::all()
        .into_iter()
        .map(|s| SortOption {
            key: s.key().to_string(),
            label: s.label().to_string(),
        })
        .collect()
}

pub fn list(exe: &str, build: &str, sort: &str, have: &[String]) -> Result<Vec<Shared>, String> {
    let sort: Sort = sort.parse()?;
    let wire = Live;
    let found = Community::new(&endpoint(), &wire).list_by(exe, build, sort)?;

    Ok(found
        .into_iter()
        .map(|row| {
            let installed = have.contains(&row.fingerprint);
            Shared::from(row, installed)
        })
        .collect())
}

// what comes back has already been parsed and validated by the client before
// it lands anywhere the app will load from
pub fn install(id: i64, install_id: &str, into: &PathBuf) -> Result<(String, Table), String> {
    let wire = Live;
    let (text, table) = Community::new(&endpoint(), &wire).fetch(id, install_id)?;

    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let name = format!(
        "{}.toml",
        table.game.exe.to_lowercase().trim_end_matches(".exe")
    );
    std::fs::write(into.join(name), &text).map_err(|e| e.to_string())?;
    Ok((text, table))
}

pub fn rate(id: i64, up: bool, install_id: &str, build: &str) -> Result<(), String> {
    let wire = Live;
    Community::new(&endpoint(), &wire).vote(id, install_id, up, build)
}

pub fn share(
    table: &Table,
    toml: &str,
    anonymous: bool,
    build: &str,
) -> Result<(i64, bool), String> {
    let who = if anonymous { None } else { me() };
    let wire = Live;
    let sent = Community::new(&endpoint(), &wire).submit(table, toml, who.as_ref(), build)?;
    Ok((sent.id, sent.already))
}

pub fn taken(name: &str) -> Result<bool, String> {
    freeplay_id::check_name(name)?;
    let wire = Live;
    Community::new(&endpoint(), &wire).taken(name)
}

pub fn claim(name: &str) -> Result<Vec<String>, String> {
    if me().is_some() {
        return Err("this machine already publishes under a name".into());
    }
    if taken(name)? {
        return Err(format!("{name} belongs to somebody else already"));
    }

    let who = Identity::create(name)?;
    who.save(&identity_path())?;
    Ok(who.phrase().words().to_vec())
}

pub fn recover(name: &str, phrase: &str) -> Result<String, String> {
    let who = Identity::recover(name, phrase)?;
    who.save(&identity_path())?;
    Ok(who.name)
}

pub fn forget() -> Result<(), String> {
    let path = identity_path();
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
