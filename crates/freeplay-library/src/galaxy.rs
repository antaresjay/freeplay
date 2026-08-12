//! what gog galaxy knows and the installer does not.
//!
//! the registry says where a game is and which binary to run. how long you
//! played it, when you last did, and what genre it is only exist in galaxy's
//! own database, so this reads that when it is there and shrugs when it is not.
//! buying from gog and running the offline installer is a supported way to
//! live and none of this applies to it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::play::Play;

static TAKEN: AtomicU64 = AtomicU64::new(0);

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

    // the library page asks for this on a timer and again whenever it is
    // drawn, so two of these are in flight constantly. one directory shared
    // between them means the second call deletes the copy the first is still
    // reading and both come back with nothing
    let dir = std::env::temp_dir().join(format!(
        "freeplay-galaxy-{}-{}",
        std::process::id(),
        TAKEN.fetch_add(1, Ordering::Relaxed)
    ));
    // a pid gets reused after a crash, and the count starts again with it
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
type Found = HashMap<String, Details>;

// galaxy has changed column types between versions and will again. these are
// three separate reads on purpose: a surprise in one table costs that one
// field, not the other two. reading the date as a number instead of a string
// once took every game's genre down with it
#[cfg(windows)]
fn read(path: &Path) -> rusqlite::Result<Found> {
    let db = rusqlite::Connection::open(path)?;
    let mut out = Found::new();

    let _ = minutes(&db, &mut out);
    let _ = last_played(&db, &mut out);
    let _ = genres(&db, &mut out);

    out.retain(|_, d| !d.is_empty());
    Ok(out)
}

// more than one windows account can have played the same game, same as steam.
// the bigger number is the interesting one
#[cfg(windows)]
fn minutes(db: &rusqlite::Connection, out: &mut Found) -> rusqlite::Result<()> {
    let mut stmt = db.prepare("select releaseKey, minutesInGame from GameTimes")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    for (key, played) in rows.flatten() {
        let (Some(id), true) = (game_id(&key), played > 0) else {
            continue;
        };
        let held = &mut out.entry(id.to_string()).or_default().play.minutes;
        *held = Some((*held).unwrap_or(0).max(played as u32));
    }
    Ok(())
}

// stored as 'YYYY-MM-DD HH:MM:SS' in utc, not as seconds, whatever the older
// galaxy builds did. sqlite already knows how to read both, so it does the
// conversion rather than this growing a calendar
#[cfg(windows)]
fn last_played(db: &rusqlite::Connection, out: &mut Found) -> rusqlite::Result<()> {
    let mut stmt = db.prepare(
        "select gameReleaseKey, case typeof(lastPlayedDate) \
           when 'integer' then lastPlayedDate \
           else cast(strftime('%s', lastPlayedDate) as integer) \
         end from LastPlayedDates",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
    })?;
    for (key, when) in rows.flatten() {
        let (Some(id), Some(at)) = (game_id(&key), when.and_then(a_real_date)) else {
            continue;
        };
        let held = &mut out.entry(id.to_string()).or_default().play.last_played;
        *held = Some((*held).unwrap_or(0).max(at));
    }
    Ok(())
}

// genre sits in a json blob under a type id that is not stable across galaxy
// versions, so it gets looked up by name
#[cfg(windows)]
fn genres(db: &rusqlite::Connection, out: &mut Found) -> rusqlite::Result<()> {
    let mut stmt = db.prepare(
        "select p.releaseKey, p.value from GamePieces p \
         join GamePieceTypes t on t.id = p.gamePieceTypeId where t.type = 'meta'",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for (key, json) in rows.flatten() {
        let Some(id) = game_id(&key) else { continue };
        let found = genres_in(&json);
        if !found.is_empty() {
            out.entry(id.to_string()).or_default().genres = found;
        }
    }
    Ok(())
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

    // two of these overlapping used to share one directory, and the second
    // deleted the copy the first was reading. every caller got nothing
    #[test]
    fn overlapping_snapshots_do_not_delete_each_other() {
        let storage = std::env::temp_dir().join(format!("freeplay-fake-galaxy-{}", id()));
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(storage.join("galaxy-2.0.db"), b"pretend this is sqlite").unwrap();

        let held: Vec<_> = (0..4).filter_map(|_| snapshot(&storage)).collect();
        assert_eq!(held.len(), 4, "every call got a snapshot");

        for taken in &held {
            assert!(
                taken.db.is_file(),
                "{} was deleted underneath us",
                taken.db.display()
            );
        }
        let places: std::collections::HashSet<_> = held.iter().map(|t| &t.dir).collect();
        assert_eq!(places.len(), 4, "each one got its own directory");

        drop(held);
        let _ = std::fs::remove_dir_all(&storage);
    }

    fn id() -> u32 {
        std::process::id()
    }

    // galaxy declares lastPlayedDate TEXT NOT NULL and writes
    // '2026-08-12 03:10:10'. reading it as a number failed the statement, and
    // because that error came back out of read() every game lost its genre
    // and its playtime too. the column was empty when this was written, so
    // nothing caught it until a game had actually been launched
    #[cfg(windows)]
    #[test]
    fn a_date_stored_as_text_costs_nothing_else() {
        let dir = std::env::temp_dir().join(format!("freeplay-galaxy-text-{}", id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("galaxy-2.0.db");

        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "create table GameTimes (userId int64, releaseKey text, minutesInGame int64);
             create table LastPlayedDates (userId int64, gameReleaseKey text,
                 lastPlayedDate text not null);
             create table GamePieceTypes (id int64, type text);
             create table GamePieces (releaseKey text, gamePieceTypeId int64, value text);
             insert into GameTimes values (1, 'gog_1438861093', 3);
             insert into LastPlayedDates values (1, 'gog_1438861093', '2026-08-12 03:10:10');
             insert into GamePieceTypes values (71, 'meta');
             insert into GamePieces values ('gog_1438861093', 71,
                 '{\"genres\":[\"Adventure\",\"Indie\"]}');",
        )
        .unwrap();
        drop(db);

        let found = read(&path).unwrap();
        let one = found.get("1438861093").expect("the game came back");
        assert_eq!(one.play.minutes, Some(3));
        assert_eq!(one.play.last_played, Some(1_786_504_210), "utc, not local");
        assert_eq!(
            one.genres,
            ["Adventure", "Indie"],
            "a date did not eat these"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // if a galaxy build ever declares that column a number instead. it takes
    // integer affinity to reach that branch at all: a number written into a
    // text column comes back as the string, which is the case below
    #[cfg(windows)]
    #[test]
    fn a_date_in_a_number_column_still_reads() {
        let dir = std::env::temp_dir().join(format!("freeplay-galaxy-int-{}", id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("galaxy-2.0.db");

        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "create table LastPlayedDates (userId int64, gameReleaseKey text,
                 lastPlayedDate int64);
             insert into LastPlayedDates values (1, 'gog_55', 1786504210);",
        )
        .unwrap();
        drop(db);

        let found = read(&path).unwrap();
        assert_eq!(
            found.get("55").map(|d| d.play.last_played),
            Some(Some(1_786_504_210))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // anything it cannot read as a date comes back blank, including a number
    // dropped into the text column. a wrong "last played" is worse than none
    #[cfg(windows)]
    #[test]
    fn a_date_it_cannot_read_is_left_blank() {
        let dir = std::env::temp_dir().join(format!("freeplay-galaxy-junk-{}", id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("galaxy-2.0.db");

        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "create table LastPlayedDates (userId int64, gameReleaseKey text,
                 lastPlayedDate text not null);
             insert into LastPlayedDates values (1, 'gog_56', 'not a date at all');
             insert into LastPlayedDates values (1, 'gog_57', 1786504210);
             insert into LastPlayedDates values (1, 'gog_58', '');",
        )
        .unwrap();
        drop(db);

        let found = read(&path).unwrap();
        for key in ["56", "57", "58"] {
            assert!(!found.contains_key(key), "{key} was guessed at");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // a table galaxy renamed or removed should cost that field only
    #[cfg(windows)]
    #[test]
    fn a_missing_table_does_not_take_the_others_with_it() {
        let dir = std::env::temp_dir().join(format!("freeplay-galaxy-gone-{}", id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("galaxy-2.0.db");

        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "create table GamePieceTypes (id int64, type text);
             create table GamePieces (releaseKey text, gamePieceTypeId int64, value text);
             insert into GamePieceTypes values (71, 'meta');
             insert into GamePieces values ('gog_99', 71, '{\"genres\":[\"Puzzle\"]}');",
        )
        .unwrap();
        drop(db);

        let found = read(&path).unwrap();
        assert_eq!(
            found.get("99").map(|d| d.genres.clone()),
            Some(vec!["Puzzle".to_string()])
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
