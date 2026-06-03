//! Cheat tables: what a game offers and how to find it.
//!
//! A table is data, not code. Nothing here knows anything about a specific
//! game, which is what lets somebody add support for one without touching
//! Rust or waiting for a release.

pub mod cheatengine;
pub mod resolve;
pub mod schema;

use std::path::{Path, PathBuf};

pub use resolve::{evaluate, State};
pub use schema::{Action, Category, Cheat, Game, Locator, Table};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path} is not a valid table: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("{0}")]
    Invalid(String),
}

impl Table {
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.into(),
            source,
        })?;
        let table: Table = toml::from_str(&text).map_err(|source| Error::Parse {
            path: path.into(),
            source,
        })?;
        table.validate().map_err(Error::Invalid)?;
        Ok(table)
    }

    /// A Cheat Engine table, converted on the way in. The exe and the game
    /// name come from the file name, since a `.CT` does not reliably say
    /// which game it belongs to.
    pub fn load_ct(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let xml = std::fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.into(),
            source,
        })?;

        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // "witcher2.exe.CT" names the process outright, otherwise assume the
        // stem is the process name.
        let exe = if stem.to_lowercase().ends_with(".exe") {
            stem.clone()
        } else {
            format!("{stem}.exe")
        };
        let title = stem
            .trim_end_matches(".exe")
            .trim_end_matches(".EXE")
            .to_string();

        let imported = cheatengine::import(&xml, &exe, &title).map_err(Error::Invalid)?;
        for skip in &imported.skipped {
            tracing::info!("{}: skipped {:?}, {}", path.display(), skip.name, skip.why);
        }
        tracing::info!("{}: {}", path.display(), imported.summary());

        imported.table.validate().map_err(Error::Invalid)?;
        Ok(imported.table)
    }

    /// Every table in a directory, skipping anything that will not parse so one
    /// bad file does not hide the rest. Cheat Engine tables are read too, so
    /// dropping a `.CT` in the folder is all it takes.
    pub fn load_dir(dir: impl AsRef<Path>) -> Vec<Self> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };

        entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter_map(|path| {
                let extension = path.extension()?.to_string_lossy().to_lowercase();
                let loaded = match extension.as_str() {
                    "toml" => Table::load(&path),
                    "ct" => Table::load_ct(&path),
                    _ => return None,
                };
                match loaded {
                    Ok(table) => Some(table),
                    Err(e) => {
                        tracing::warn!("skipping table: {e}");
                        None
                    }
                }
            })
            .collect()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.game.exe.trim().is_empty() {
            return Err("game.exe is empty".into());
        }

        let mut seen = std::collections::HashSet::new();
        for cheat in &self.cheats {
            if cheat.id.trim().is_empty() {
                return Err(format!("cheat {:?} has no id", cheat.name));
            }
            if !seen.insert(&cheat.id) {
                return Err(format!("two cheats share the id {:?}", cheat.id));
            }
            if let Action::Bytes { replacement } = &cheat.action {
                resolve::parse_bytes(replacement)
                    .map_err(|e| format!("cheat {:?}: {e}", cheat.id))?;
            }
            if let Action::Nop { length } = &cheat.action {
                if *length == 0 {
                    return Err(format!("cheat {:?} nops zero bytes", cheat.id));
                }
            }
        }
        Ok(())
    }

    /// Matches the executable name, case insensitively.
    pub fn matches_process(&self, process: &str) -> bool {
        self.game.exe.eq_ignore_ascii_case(process)
    }

    pub fn cheat(&self, id: &str) -> Option<&Cheat> {
        self.cheats.iter().find(|c| c.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freeplay_core::value::{Scalar, ValueKind};

    const SAMPLE: &str = r#"
        [game]
        name = "Mass Effect"
        exe = "MassEffect1.exe"
        verified = ["1.0.0"]

        [[cheat]]
        id = "infinite-health"
        name = "Infinite Health"
        category = "player"
        find = "pattern"
        type = "freeze"
        value_type = "f32"
        value = 1000

        [cheat.locator]
        find = "pattern"
        pattern = "48 8B 05 ?? ?? ?? ??"
        offset = 3
        hops = ["+0x28", "-0x8"]

        [[cheat]]
        id = "freeze-timer"
        name = "Freeze Mission Timer"
        category = "game"
        type = "nop"
        length = 5

        [cheat.locator]
        find = "static"
        module = "MassEffect1.exe"
        offset = "0x1A2B3C"
    "#;

    #[test]
    fn parses_a_table() {
        let table = Table::parse(SAMPLE).expect("parse");
        assert_eq!(table.game.name, "Mass Effect");
        assert_eq!(table.cheats.len(), 2);
        assert!(table.validate().is_ok());
    }

    #[test]
    fn matches_the_executable_case_insensitively() {
        let table = Table::parse(SAMPLE).unwrap();
        assert!(table.matches_process("masseffect1.exe"));
        assert!(!table.matches_process("MassEffect2.exe"));
    }

    #[test]
    fn reads_hops_written_as_hex_strings() {
        let table = Table::parse(SAMPLE).unwrap();
        let Locator::Pattern { hops, offset, .. } =
            &table.cheat("infinite-health").unwrap().locator
        else {
            panic!("expected a pattern locator");
        };
        assert_eq!(offset, &3);
        assert_eq!(
            hops.iter().map(|h| h.0).collect::<Vec<_>>(),
            vec![0x28, -0x8]
        );
    }

    #[test]
    fn reads_static_offsets_written_as_hex() {
        let table = Table::parse(SAMPLE).unwrap();
        let Locator::Static { offset, .. } = &table.cheat("freeze-timer").unwrap().locator else {
            panic!("expected a static locator");
        };
        assert_eq!(*offset, 0x1A2B3C);
    }

    #[test]
    fn an_integer_literal_works_for_a_float_field() {
        let table = Table::parse(SAMPLE).unwrap();
        let Action::Freeze { kind, value } = &table.cheat("infinite-health").unwrap().action else {
            panic!("expected a freeze");
        };
        assert_eq!(kind.0, ValueKind::F32);
        assert_eq!(value.to_scalar(kind.0), Scalar::F32(1000.0));
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let text = r#"
            [game]
            name = "X"
            exe = "x.exe"

            [[cheat]]
            id = "same"
            name = "One"
            type = "nop"
            length = 1
            [cheat.locator]
            find = "static"
            module = "x.exe"
            offset = 0

            [[cheat]]
            id = "same"
            name = "Two"
            type = "nop"
            length = 1
            [cheat.locator]
            find = "static"
            module = "x.exe"
            offset = 0
        "#;
        let table = Table::parse(text).unwrap();
        assert!(table.validate().unwrap_err().contains("share the id"));
    }

    #[test]
    fn a_nop_of_zero_bytes_is_rejected() {
        let text = r#"
            [game]
            name = "X"
            exe = "x.exe"

            [[cheat]]
            id = "bad"
            name = "Bad"
            type = "nop"
            length = 0
            [cheat.locator]
            find = "static"
            module = "x.exe"
            offset = 0
        "#;
        assert!(Table::parse(text).unwrap().validate().is_err());
    }

    #[test]
    fn a_table_with_no_cheats_is_still_valid() {
        let table = Table::parse("[game]\nname = \"X\"\nexe = \"x.exe\"").unwrap();
        assert!(table.cheats.is_empty());
        assert!(table.validate().is_ok());
    }
}
