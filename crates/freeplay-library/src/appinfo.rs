//! Genres out of Steam's appinfo cache.
//!
//! `appcache/appinfo.vdf` is Valve's binary key-value format, not the text one
//! in `vdf.rs`. Version 29 moved every key into a string table at the end of
//! the file, so a key is a number and the table has to be read first.
//!
//! Only `common/genres` is kept. Everything else is walked past, which is why
//! this reads the whole tree rather than seeking: the offsets are not written
//! down anywhere, so finding a field means passing over the ones before it.
//!
//! Valve changes this file without telling anybody. Every read is checked and
//! anything unexpected ends the parse and returns what was already found, so a
//! new version costs the genre row and nothing else.

use std::collections::HashMap;
use std::path::Path;

// steam's own list. the ids are stable, the names are what the store shows
const NAMES: &[(u32, &str)] = &[
    (1, "Action"),
    (2, "Strategy"),
    (3, "RPG"),
    (4, "Casual"),
    (9, "Racing"),
    (18, "Sports"),
    (23, "Indie"),
    (25, "Adventure"),
    (28, "Simulation"),
    (29, "Massively Multiplayer"),
    (37, "Free to Play"),
    (50, "Accounting"),
    (51, "Animation & Modeling"),
    (52, "Audio Production"),
    (53, "Design & Illustration"),
    (54, "Education"),
    (55, "Photo Editing"),
    (56, "Software Training"),
    (57, "Utilities"),
    (58, "Video Production"),
    (59, "Web Publishing"),
    (60, "Game Development"),
    (70, "Early Access"),
    (71, "Sexual Content"),
    (72, "Nudity"),
    (73, "Violent"),
    (74, "Gore"),
    (81, "Documentary"),
    (84, "Tutorial"),
];

fn name_of(id: u32) -> Option<&'static str> {
    NAMES.iter().find(|(n, _)| *n == id).map(|(_, name)| *name)
}

pub fn genres(steam_root: &Path) -> HashMap<String, Vec<String>> {
    let file = steam_root.join("appcache").join("appinfo.vdf");
    let Ok(raw) = std::fs::read(&file) else {
        return HashMap::new();
    };
    read(&raw).unwrap_or_default()
}

struct Reader<'a> {
    raw: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Option<u8> {
        let found = *self.raw.get(self.at)?;
        self.at += 1;
        Some(found)
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let found = self.raw.get(self.at..self.at + n)?;
        self.at += n;
        Some(found)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn text(&mut self) -> Option<String> {
        let end = self.raw[self.at..].iter().position(|b| *b == 0)? + self.at;
        let found = String::from_utf8_lossy(&self.raw[self.at..end]).into_owned();
        self.at = end + 1;
        Some(found)
    }
}

const MAP: u8 = 0x00;
const TEXT: u8 = 0x01;
const INT: u8 = 0x02;
const BIG: u8 = 0x07;
const END: u8 = 0x08;
const SIGNED: u8 = 0x0B;

fn read(raw: &[u8]) -> Option<HashMap<String, Vec<String>>> {
    let mut head = Reader { raw, at: 0 };
    let magic = head.u32()?;
    if magic != 0x0756_4429 {
        // 27 and 28 kept their keys inline and nobody has run one for years
        return None;
    }
    let _universe = head.u32()?;
    let table = strings(raw, head.i64()? as usize)?;

    let mut out = HashMap::new();
    let mut at = head.at;
    loop {
        let mut app = Reader { raw, at };
        let id = app.u32()?;
        if id == 0 {
            return Some(out);
        }
        let size = app.u32()? as usize;
        // the fixed header: state, updated, token, two hashes and a change number
        app.take(4 + 4 + 8 + 20 + 4 + 20)?;

        let mut found = Vec::new();
        // a broken app section is not a reason to lose the rest of the file
        let _ = read_map(&mut app, &table, "", "", &mut found);
        if !found.is_empty() {
            out.insert(id.to_string(), found);
        }

        at = at.checked_add(8)?.checked_add(size)?;
        if at >= raw.len() {
            return Some(out);
        }
    }
}

fn strings(raw: &[u8], at: usize) -> Option<Vec<String>> {
    let mut r = Reader { raw, at };
    let count = r.u32()? as usize;
    // a length field that has gone wrong should not ask for a gigabyte
    if count > raw.len() {
        return None;
    }
    (0..count).map(|_| r.text()).collect()
}

// `under` and `parent` are the two keys above this map, which is as much as it
// takes to know that a number is a genre and not some other list of ids
fn read_map(
    r: &mut Reader,
    table: &[String],
    under: &str,
    parent: &str,
    out: &mut Vec<String>,
) -> Option<()> {
    let wanted = under == "common" && parent == "genres";
    loop {
        let kind = r.byte()?;
        if kind == END {
            return Some(());
        }
        let key = table.get(r.u32()? as usize)?.as_str();
        match kind {
            MAP => read_map(r, table, parent, key, out)?,
            TEXT => {
                let value = r.text()?;
                if wanted {
                    keep(value.parse().ok()?, out);
                }
            }
            INT => {
                let value = r.u32()?;
                if wanted {
                    keep(value, out);
                }
            }
            BIG | SIGNED => {
                r.take(8)?;
            }
            _ => return None,
        }
    }
}

fn keep(id: u32, out: &mut Vec<String>) {
    if let Some(name) = name_of(id) {
        let name = name.to_string();
        if !out.contains(&name) {
            out.push(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // the smallest file the reader will accept, built the way steam writes one
    fn built(apps: &[(u32, &[u32])]) -> Vec<u8> {
        let table: Vec<&str> = vec!["appinfo", "common", "genres", "0", "1", "2", "name"];
        let index = |want: &str| table.iter().position(|s| *s == want).unwrap() as u32;

        let mut body = Vec::new();
        for (id, genres) in apps {
            let mut kv = Vec::new();
            kv.push(MAP);
            kv.extend(index("appinfo").to_le_bytes());
            kv.push(MAP);
            kv.extend(index("common").to_le_bytes());
            kv.push(TEXT);
            kv.extend(index("name").to_le_bytes());
            kv.extend(b"a game\0");
            kv.push(MAP);
            kv.extend(index("genres").to_le_bytes());
            for (n, genre) in genres.iter().enumerate() {
                kv.push(INT);
                kv.extend(index(&n.to_string()).to_le_bytes());
                kv.extend(genre.to_le_bytes());
            }
            kv.extend([END, END, END]);

            let header = 4 + 4 + 8 + 20 + 4 + 20;
            body.extend(id.to_le_bytes());
            body.extend(((header + kv.len()) as u32).to_le_bytes());
            body.extend(std::iter::repeat_n(0u8, header));
            body.extend(kv);
        }
        body.extend(0u32.to_le_bytes());

        let mut out = Vec::new();
        out.extend(0x0756_4429u32.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(((16 + body.len()) as i64).to_le_bytes());
        out.extend(body);
        out.extend((table.len() as u32).to_le_bytes());
        for word in &table {
            out.extend(word.as_bytes());
            out.push(0);
        }
        out
    }

    #[test]
    fn reads_the_genres_off_an_app() {
        let found = read(&built(&[(824270, &[1, 23, 28])])).unwrap();
        assert_eq!(
            found.get("824270").unwrap(),
            &["Action", "Indie", "Simulation"]
        );
    }

    #[test]
    fn every_app_in_the_file_gets_read() {
        let found = read(&built(&[(1, &[1]), (2, &[3]), (3, &[25])])).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(found.get("2").unwrap(), &["RPG"]);
    }

    #[test]
    fn an_app_with_no_genres_is_left_out_rather_than_kept_empty() {
        let found = read(&built(&[(1, &[]), (2, &[1])])).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found.contains_key("2"));
    }

    #[test]
    fn an_id_valve_has_not_published_a_name_for_is_dropped() {
        let found = read(&built(&[(1, &[1, 9999])])).unwrap();
        assert_eq!(found.get("1").unwrap(), &["Action"]);
    }

    // valve has changed this format four times. a fifth should cost the genre
    // row and nothing else
    #[test]
    fn a_version_this_does_not_know_is_not_a_panic() {
        let mut raw = built(&[(1, &[1])]);
        raw[3] = 0x30;
        assert!(read(&raw).is_none());
    }

    #[test]
    fn a_file_that_stops_halfway_is_not_a_panic() {
        let raw = built(&[(1, &[1]), (2, &[3])]);
        for cut in [17, 40, 80, raw.len() - 4] {
            let _ = read(&raw[..cut]);
        }
    }
}

// reads the real file if steam is installed, and says nothing if it is not.
// there is no fixture for this one: the point is that whatever valve shipped
// this week still parses
#[cfg(test)]
mod against_the_real_thing {
    #[test]
    fn the_installed_cache_still_reads() {
        let Some(root) = crate::steam::root() else {
            return;
        };
        let found = super::genres(&root);
        if !root.join("appcache").join("appinfo.vdf").is_file() {
            return;
        }
        assert!(!found.is_empty(), "steam is here but nothing came back");
        assert!(found.values().all(|list| !list.is_empty()));
    }
}
