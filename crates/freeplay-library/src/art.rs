//! cover art, out of whatever the store already put on disk
//!
//! steam downloads box art for your whole library into appcache/librarycache.
//! no reason to fetch our own copy of a picture already sitting there, and it
//! means the grid has art before the network is even awake.

use std::path::{Path, PathBuf};

use crate::{InstalledGame, Store};

// what steam keeps per game. any of them can be missing
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Art {
    // portrait box art, the one worth building a grid around
    pub cover: Option<PathBuf>,
    // wide banner that sits behind the title
    pub hero: Option<PathBuf>,
    // transparent wordmark, drawn over the hero
    pub logo: Option<PathBuf>,
}

impl Art {
    pub fn is_empty(&self) -> bool {
        self.cover.is_none() && self.hero.is_none() && self.logo.is_none()
    }
}

// steam has changed where this lives twice. oldest wrote
// `<appid>_library_600x900.jpg` flat into the cache folder, then each game got
// its own directory, and now every asset sits in a directory named after its
// content hash with the real name inside. a machine that has been upgraded
// rather than reinstalled has all three at once
fn pick(cache: &Path, app_id: &str, stems: &[&str]) -> Option<PathBuf> {
    for stem in stems {
        for ext in ["jpg", "png"] {
            let name = format!("{stem}.{ext}");

            let flat = cache.join(format!("{app_id}_{name}"));
            if flat.is_file() {
                return Some(flat);
            }
            let nested = cache.join(app_id).join(&name);
            if nested.is_file() {
                return Some(nested);
            }
            if let Some(found) = under_a_hash(&cache.join(app_id), &name) {
                return Some(found);
            }
        }
    }
    None
}

// one level down, in whatever the folder happens to be called
fn under_a_hash(dir: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let candidate = entry.path().join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn in_cache(cache: &Path, app_id: &str) -> Art {
    Art {
        // library_capsule is what the portrait is called these days
        cover: pick(
            cache,
            app_id,
            &[
                "library_600x900",
                "library_capsule",
                "header",
                "library_header",
            ],
        ),
        hero: pick(cache, app_id, &["library_hero"]),
        logo: pick(cache, app_id, &["logo"]),
    }
}

// art for one steam app id
#[cfg(windows)]
pub fn steam(app_id: &str) -> Art {
    let Some(root) = crate::steam::root() else {
        return Art::default();
    };
    in_cache(&root.join("appcache").join("librarycache"), app_id)
}

#[cfg(not(windows))]
pub fn steam(_app_id: &str) -> Art {
    Art::default()
}

pub fn find(game: &InstalledGame) -> Art {
    match (game.store, game.app_id.as_deref()) {
        (Store::Steam, Some(id)) => steam(id),
        _ => Art::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("freeplay-art-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"not really a jpeg").unwrap();
    }

    #[test]
    fn reads_the_per_game_folder_layout() {
        let cache = scratch("nested");
        touch(&cache.join("20920").join("library_600x900.jpg"));
        touch(&cache.join("20920").join("library_hero.jpg"));
        touch(&cache.join("20920").join("logo.png"));

        let art = in_cache(&cache, "20920");
        assert!(art.cover.unwrap().ends_with("library_600x900.jpg"));
        assert!(art.hero.is_some());
        assert!(art.logo.unwrap().ends_with("logo.png"));

        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn reads_the_older_flat_layout() {
        let cache = scratch("flat");
        touch(&cache.join("20920_library_600x900.jpg"));

        let art = in_cache(&cache, "20920");
        assert!(art.cover.unwrap().ends_with("20920_library_600x900.jpg"));
        assert!(art.hero.is_none());

        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn falls_back_to_the_header_when_there_is_no_box_art() {
        let cache = scratch("header");
        touch(&cache.join("400").join("header.jpg"));

        assert!(in_cache(&cache, "400")
            .cover
            .unwrap()
            .ends_with("header.jpg"));

        std::fs::remove_dir_all(&cache).unwrap();
    }

    // the layout that made THE FINALS and NBA 2K26 show two grey letters
    // instead of a cover: every asset in a folder named after its content hash
    #[test]
    fn reads_the_content_hash_layout() {
        let cache = scratch("hashed");
        touch(
            &cache
                .join("2073850")
                .join("6d9cfdeaea9f822d8f4af42d8f0dbd08")
                .join("library_capsule.jpg"),
        );
        touch(
            &cache
                .join("2073850")
                .join("f145688cf8e5b9e7b17cc4430b489a27")
                .join("library_hero.jpg"),
        );
        touch(&cache.join("2073850").join("logo.png"));

        let art = in_cache(&cache, "2073850");
        assert!(
            art.cover.is_some(),
            "the capsule is what the portrait is called now"
        );
        assert!(art.hero.is_some());
        assert!(art.logo.is_some());

        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn the_blurred_hero_is_not_the_hero() {
        let cache = scratch("blur");
        touch(
            &cache
                .join("2073850")
                .join("f145688c")
                .join("library_hero_blur.jpg"),
        );

        assert_eq!(in_cache(&cache, "2073850").hero, None);

        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn missing_game_gives_nothing() {
        let cache = scratch("empty");
        assert!(in_cache(&cache, "999999").is_empty());
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn only_steam_has_art_for_now() {
        let game = InstalledGame {
            name: "Some GOG Game".into(),
            store: Store::Gog,
            install_dir: PathBuf::from("C:\\g"),
            app_id: Some("1234".into()),
            executables: vec![],
            version: None,
        };
        assert!(find(&game).is_empty());
    }
}
