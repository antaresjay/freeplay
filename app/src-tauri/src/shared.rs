// the community side of things: what other people have shared for a game,
// pulling one in, saying whether it worked, and who you publish as

use std::path::PathBuf;

use freeplay_id::Identity;
use freeplay_sync::community::{Community, Live, Sort, ENDPOINT};
use freeplay_sync::rank::Fit;
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
    // who uploaded it
    pub by: String,
    // who worked the addresses out, if the table says
    pub author: String,
    pub cheats: u32,
    pub up: i64,
    pub down: i64,
    pub downloads: i64,
    pub built_for: String,
    pub added: i64,
    pub standing: String,
    // already sitting in the tables folder
    pub installed: bool,
    // how the build it was checked against lines up with the one installed
    // here: same, older, newer or unknown
    pub fit: String,
    pub fit_note: String,
    // the one worth pointing at, if any
    pub recommended: bool,
}

impl Shared {
    fn from(row: freeplay_sync::rank::Scored, installed: bool, mine: &str) -> Self {
        let fit = row.fit;
        let listing = row.listing;

        Self {
            standing: listing.standing(),
            fit_note: fit_note(fit, &listing.built_for, mine),
            fit: fit.key().to_string(),
            recommended: row.recommended,
            id: listing.id,
            game: listing.game,
            by: listing.submitted_by,
            author: listing.author,
            cheats: listing.cheats,
            up: listing.up,
            down: listing.down,
            downloads: listing.downloads,
            built_for: listing.built_for,
            added: listing.created_at,
            installed,
        }
    }
}

// said in full, because "checked on 3.5.0.1" only means something if you
// happen to know which build you are running
fn fit_note(fit: Fit, theirs: &str, mine: &str) -> String {
    match fit {
        Fit::Same => format!("Tested on your version, {theirs}"),
        Fit::Older => format!("Tested on {theirs}, you have {mine}"),
        Fit::Newer => format!("Tested on {theirs}, newer than your {mine}"),
        Fit::Unknown if !theirs.is_empty() => format!("Tested on {theirs}"),
        Fit::Unknown => "Nobody recorded which version this was tested on".into(),
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

    // best match is worked out here rather than by the service, because this
    // is the only machine that knows which build of the game is installed.
    // the other orders are what the person asked for and are left alone
    let ordered = if sort == Sort::Best {
        freeplay_sync::rank::rank(found, build, now())
    } else {
        found
            .into_iter()
            .map(|listing| freeplay_sync::rank::Scored {
                fit: freeplay_sync::rank::fit_of(&listing.built_for, build),
                listing,
                score: 0.0,
                recommended: false,
            })
            .collect()
    };

    Ok(ordered
        .into_iter()
        .map(|row| {
            let installed = have.contains(&row.listing.fingerprint);
            Shared::from(row, installed, build)
        })
        .collect())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

// point a table at a different executable.
//
// somebody searched by name, found the table filed under falloutw.exe and
// asked for it on a game running falloutwHR.exe. matching is an exact compare
// on the file name, so without this the table downloads perfectly and then
// never appears anywhere, which looks far more broken than an error would.
//
// only the one line under [game] moves. rewriting the whole file from the
// parsed table would drop anything the parser does not keep
fn point_at(toml: &str, exe: &str) -> String {
    let mut out = String::with_capacity(toml.len() + exe.len());
    let mut in_game = false;
    let mut done = false;

    for line in toml.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_game = trimmed.starts_with("[game]");
        }
        if !done && in_game && trimmed.starts_with("exe") {
            if let Some(at) = line.find("exe") {
                out.push_str(&line[..at]);
                out.push_str(&format!("exe = {exe:?}"));
                out.push('\n');
                done = true;
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

// what comes back has already been parsed and validated by the client before
// it lands anywhere the app will load from
pub fn install(
    id: i64,
    install_id: &str,
    into: &PathBuf,
    for_exe: Option<&str>,
) -> Result<(String, Table), String> {
    let wire = Live;
    let (mut text, mut table) = Community::new(&endpoint(), &wire).fetch(id, install_id)?;

    // asked for on a game whose binary is called something else
    if let Some(wanted) = for_exe.filter(|w| !w.eq_ignore_ascii_case(&table.game.exe)) {
        let moved = point_at(&text, wanted);
        // parse it again rather than trusting the edit. a table that no longer
        // reads is worse than one filed under the wrong name
        table = Table::parse(&moved)
            .map_err(|e| format!("could not point that table at {wanted}: {e}"))?;
        text = moved;
    }

    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    // the id is in the name because several tables for one game is normal now.
    // it used to be just the exe, so the second one you fetched quietly
    // replaced the first
    let name = format!(
        "{}-{id}.toml",
        table.game.exe.to_lowercase().trim_end_matches(".exe")
    );
    std::fs::write(into.join(name), &text).map_err(|e| e.to_string())?;
    Ok((text, table))
}

// for_exe is what the game was really running. an upvote from somebody who
// took a table filed under one name and used it under another is the only
// thing that tells the service the two go together, which is what saves the
// next person the search
pub fn rate(id: i64, up: bool, install_id: &str, build: &str, for_exe: &str) -> Result<(), String> {
    let wire = Live;
    Community::new(&endpoint(), &wire).vote(id, install_id, up, build, for_exe)
}

pub fn search(query: &str) -> Result<Vec<Shared>, String> {
    let wire = Live;
    let found = Community::new(&endpoint(), &wire).search(query)?;
    Ok(found
        .into_iter()
        .map(|listing| {
            Shared::from(
                freeplay_sync::rank::Scored {
                    fit: freeplay_sync::rank::Fit::Unknown,
                    listing,
                    score: 0.0,
                    recommended: false,
                },
                false,
                "",
            )
        })
        .collect())
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

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = r#"[game]
name = "Fallout"
exe = "falloutw.exe"
verified = ["1.2"]
notes = "exe = \"decoy.exe\" in the notes"

[[cheat]]
id = "caps"
name = "Caps"
category = "resources"
type = "freeze"
value_type = "i32"
value = 9999

[cheat.locator]
find = "pattern"
pattern = "48 8B 05 ?? ?? ?? ??"
offset = 3
"#;

    #[test]
    fn pointing_a_table_at_another_binary_moves_one_line() {
        let moved = point_at(TABLE, "falloutwHR.exe");
        let table = Table::parse(&moved).expect("still reads");

        assert_eq!(table.game.exe, "falloutwHR.exe");
        assert_eq!(table.game.name, "Fallout");
        assert_eq!(table.cheats.len(), 1, "the cheat survived");
    }

    // only the one under [game]. a cheat with its own exe key, or the word in
    // a note, is none of its business
    // only the one under [game]. the word turning up in a note is none of
    // its business, and neither is anything in a [[cheat]]
    #[test]
    fn it_leaves_everything_outside_the_game_block_alone() {
        let moved = point_at(TABLE, "other.exe");

        assert!(moved.contains(r#"exe = \"decoy.exe\" in the notes"#));
        assert!(moved.contains(r#"pattern = "48 8B 05 ?? ?? ?? ??""#));
        assert_eq!(moved.matches("other.exe").count(), 1);
    }

    #[test]
    fn a_name_with_awkward_characters_is_quoted_properly() {
        let moved = point_at(TABLE, r#"od d"name.exe"#);
        let table = Table::parse(&moved).expect("still reads");
        assert_eq!(table.game.exe, r#"od d"name.exe"#);
    }

    #[test]
    fn a_table_with_no_game_exe_is_left_as_it_was() {
        let bare = "[game]\nname = \"Thing\"\n";
        assert_eq!(point_at(bare, "thing.exe"), bare);
    }
}
