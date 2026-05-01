//! Cover art, taken from what the store already put on disk.
//!
//! Steam downloads box art for everything in your library and leaves it in
//! `appcache/librarycache`. There is no reason to go and fetch our own copy of
//! a picture that is already sitting there, and doing it this way means the
//! interface has art before the network is even awake.

use std::path::{Path, PathBuf};

use crate::{InstalledGame, Store};

/// The images Steam keeps per game. Any of them can be missing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Art {
    /// Portrait box art, 600x900. The one worth building a grid around.
    pub cover: Option<PathBuf>,
    /// Wide banner that sits behind the title on the store page.
    pub hero: Option<PathBuf>,
    /// Transparent wordmark meant to be drawn over the hero.
    pub logo: Option<PathBuf>,
}

impl Art {
    pub fn is_empty(&self) -> bool {
        self.cover.is_none() && self.hero.is_none() && self.logo.is_none()
    }
}

/// Older Steam clients wrote `<appid>_library_600x900.jpg` straight into the
/// cache folder. Newer ones give each game its own directory. Both still exist
/// on machines that have been upgraded rather than reinstalled.
fn pick(cache: &Path, app_id: &str, stem: &str) -> Option<PathBuf> {
    for ext in ["jpg", "png"] {
        let nested = cache.join(app_id).join(format!("{stem}.{ext}"));
        if nested.is_file() {
            return Some(nested);
        }
        let flat = cache.join(format!("{app_id}_{stem}.{ext}"));
        if flat.is_file() {
            return Some(flat);
        }
    }
    None
}

fn in_cache(cache: &Path, app_id: &str) -> Art {
    Art {
        cover: pick(cache, app_id, "library_600x900").or_else(|| pick(cache, app_id, "header")),
        hero: pick(cache, app_id, "library_hero"),
        logo: pick(cache, app_id, "logo"),
    }
}

/// Art for one Steam app id.
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
        };
        assert!(find(&game).is_empty());
    }
}
