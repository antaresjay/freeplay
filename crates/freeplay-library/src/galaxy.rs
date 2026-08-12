//! what gog galaxy knows and the installer does not.
//!
//! the registry says where a game is and which binary to run. how long you
//! played it, when you last did, and what genre it is only exist in galaxy's
//! own database, so this reads that when it is there and shrugs when it is not.
//! buying from gog and running the offline installer is a supported way to
//! live and none of this applies to it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::play::Play;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Details {
    pub play: Play,
    pub genres: Vec<String>,
}

impl Details {
    pub fn is_empty(&self) -> bool {
        self.play.is_empty() && self.genres.is_empty()
    }
}

// galaxy writes `gog_1438861093`, we hold `1438861093`
fn game_id(release_key: &str) -> Option<&str> {
    release_key.strip_prefix("gog_").filter(|id| !id.is_empty())
}

// nothing before 2000 is a date anybody played a game on, and galaxy has been
// seen writing zeroes into this column for games never launched
fn a_real_date(seconds: i64) -> Option<u64> {
    (seconds > 946_684_800).then_some(seconds as u64)
}

// a copy of the database that deletes itself
struct Snapshot {
    dir: PathBuf,
    db: PathBuf,
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// galaxy keeps this open in wal mode the whole time it runs. a read only
// connection cannot replay a wal without being allowed to write the -shm
// beside it, and the wal here is bigger than the database, so reading the
// db file alone gives stale rubbish. copying both and reading our own copy
// is the only version of this that neither lies nor touches galaxy's files.
//
// a torn copy is fine: sqlite checksums wal frames and ignores the ones that
// do not add up, which costs at worst the last few minutes of playtime.
fn snapshot(storage: &Path) -> Option<Snapshot> {
    let source = storage.join("galaxy-2.0.db");
    if !source.is_file() {
        return None;
    }

    let dir = std::env::temp_dir().join(format!("freeplay-galaxy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;

    let taken = Snapshot {
        db: dir.join("galaxy-2.0.db"),
        dir,
    };
    std::fs::copy(&source, &taken.db).ok()?;

    // no -wal is normal, it means galaxy checkpointed and shut down cleanly
    let wal = storage.join("galaxy-2.0.db-wal");
    if wal.is_file() {
        let mut beside = taken.db.clone().into_os_string();
        beside.push("-wal");
        let _ = std::fs::copy(&wal, PathBuf::from(beside));
    }
    Some(taken)
}

#[cfg(windows)]
fn storage() -> Option<PathBuf> {
    let program_data = std::env::var("PROGRAMDATA").ok()?;
    Some(
        PathBuf::from(program_data)
            .join("GOG.com")
            .join("Galaxy")
            .join("storage"),
    )
}

#[cfg(not(windows))]
fn storage() -> Option<PathBuf> {
    None
}

// everything galaxy has to say, by gog game id. empty if it is not installed
#[cfg(windows)]
pub fn details() -> HashMap<String, Details> {
    storage()
        .and_then(|dir| snapshot(&dir))
        .and_then(|taken| read(&taken.db).ok())
        .unwrap_or_default()
}

#[cfg(not(windows))]
pub fn details() -> HashMap<String, Details> {
    HashMap::new()
}

#[cfg(windows)]
fn read(path: &Path) -> rusqlite::Result<HashMap<String, Details>> {
    let db = rusqlite::Connection::open(path)?;
    let mut out: HashMap<String, Details> = HashMap::new();

    // more than one windows account can have played the same game, same as
    // steam. the bigger number is the interesting one
    let mut minutes = db.prepare("select releaseKey, minutesInGame from GameTimes")?;
    for row in minutes.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (key, played) = row?;
        let (Some(id), true) = (game_id(&key), played > 0) else {
            continue;
        };
        let held = &mut out.entry(id.to_string()).or_default().play.minutes;
        *held = Some((*held).unwrap_or(0).max(played as u32));
    }

    let mut dates = db.prepare("select gameReleaseKey, lastPlayedDate from LastPlayedDates")?;
    for row in dates.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
    })? {
        let (key, when) = row?;
        let (Some(id), Some(at)) = (game_id(&key), when.and_then(a_real_date)) else {
            continue;
        };
        let held = &mut out.entry(id.to_string()).or_default().play.last_played;
        *held = Some((*held).unwrap_or(0).max(at));
    }

    // genre sits in a json blob under a type id that is not stable across
    // galaxy versions, so it gets looked up by name
    let mut meta = db.prepare(
        "select p.releaseKey, p.value from GamePieces p \
         join GamePieceTypes t on t.id = p.gamePieceTypeId where t.type = 'meta'",
    )?;
    for row in meta.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (key, json) = row?;
        let Some(id) = game_id(&key) else { continue };
        let genres = genres_in(&json);
        if !genres.is_empty() {
            out.entry(id.to_string()).or_default().genres = genres;
        }
    }

    out.retain(|_, d| !d.is_empty());
    Ok(out)
}

fn genres_in(json: &str) -> Vec<String> {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    parsed
        .get("genres")
        .and_then(|g| g.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|g| g.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_prefix_galaxy_puts_on_everything() {
        assert_eq!(game_id("gog_1438861093"), Some("1438861093"));
        assert_eq!(game_id("steam_20920"), None);
        assert_eq!(game_id("gog_"), None);
    }

    #[test]
    fn zero_is_not_a_date() {
        assert_eq!(a_real_date(0), None);
        assert_eq!(a_real_date(-1), None);
        assert_eq!(a_real_date(1_786_000_000), Some(1_786_000_000));
    }

    #[test]
    fn pulls_genres_out_of_the_meta_blob() {
        let json = r#"{"criticsScore":null,"developers":["Orangepixel"],
            "genres":["Adventure","Indie","Platform"],"releaseDate":1490745600}"#;
        assert_eq!(genres_in(json), ["Adventure", "Indie", "Platform"]);
    }

    #[test]
    fn a_blob_with_no_genres_is_not_a_panic() {
        assert!(genres_in("{}").is_empty());
        assert!(genres_in("not json at all").is_empty());
        assert!(genres_in(r#"{"genres":"nonsense"}"#).is_empty());
    }

    #[test]
    fn no_galaxy_is_no_snapshot() {
        let nowhere = std::env::temp_dir().join("freeplay-galaxy-does-not-exist");
        assert!(snapshot(&nowhere).is_none());
    }
}
