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

// the padding number on the background has changed before, so that one is
// matched on its stem. there is no wordmark among these: the square icon is
// the game's icon, and dropping it in the logo slot draws a second little
// cover on top of the background
const GOG_COVER: &str = "_glx_vertical_cover";
const GOG_HERO: &str = "_glx_bg_top_padding";
const GOG_ICON: &str = "_glx_square_icon";

// webcache/<user>/gog/<gameid>/<hash>_glx_<kind>.webp. the user folder is a
// number nobody knows in advance, so every one of them gets looked in
fn in_webcache(root: &Path, game_id: &str) -> Art {
    let Ok(users) = std::fs::read_dir(root) else {
        return Art::default();
    };

    for user in users.flatten() {
        let dir = user.path().join("gog").join(game_id);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        let mut art = Art::default();
        let mut icon = None;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();

            if name.contains(GOG_COVER) {
                art.cover.get_or_insert(path);
            } else if name.contains(GOG_HERO) {
                art.hero.get_or_insert(path);
            } else if name.contains(GOG_ICON) {
                icon.get_or_insert(path);
            }
        }

        // square in a portrait slot is not ideal, but it is the game rather
        // than a grey tile with two letters on it
        if art.cover.is_none() {
            art.cover = icon;
        }
        if !art.is_empty() {
            return art;
        }
    }
    Art::default()
}

#[cfg(windows)]
fn webcache() -> Option<PathBuf> {
    let program_data = std::env::var("PROGRAMDATA").ok()?;
    Some(
        PathBuf::from(program_data)
            .join("GOG.com")
            .join("Galaxy")
            .join("webcache"),
    )
}

#[cfg(not(windows))]
fn webcache() -> Option<PathBuf> {
    None
}

// galaxy has proper artwork but only if it is installed. buy from gog and run
// the offline installer and none of it exists, so the icon the installer drops
// in the game folder is the fallback. it is square and 256 across, which is
// not a cover, but it beats two letters on a grey tile
pub fn gog(game_id: &str, install_dir: &Path) -> Art {
    let mut art = webcache()
        .map(|root| in_webcache(&root, game_id))
        .unwrap_or_default();

    if art.cover.is_none() {
        let ico = install_dir.join(format!("goggame-{game_id}.ico"));
        if ico.is_file() {
            art.cover = Some(ico);
        }
    }
    art
}

pub fn find(game: &InstalledGame) -> Art {
    match (game.store, game.app_id.as_deref()) {
        (Store::Steam, Some(id)) => steam(id),
        (Store::Gog, Some(id)) => gog(id, &game.install_dir),
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
    fn reads_what_galaxy_cached() {
        let root = scratch("webcache");
        let dir = root
            .join("56125266018961573")
            .join("gog")
            .join("1438861093");
        touch(&dir.join("cfade37f_glx_vertical_cover.webp"));
        touch(&dir.join("d4af52eb_glx_bg_top_padding_7.webp"));
        touch(&dir.join("7b5c518c_glx_square_icon_v2.webp"));

        let art = in_webcache(&root, "1438861093");
        assert!(art
            .cover
            .unwrap()
            .ends_with("cfade37f_glx_vertical_cover.webp"));
        assert!(art
            .hero
            .unwrap()
            .ends_with("d4af52eb_glx_bg_top_padding_7.webp"));
        // gog has no wordmark. the square icon in this slot gets drawn over
        // the background as a second tiny cover
        assert!(art.logo.is_none(), "the square icon is not a logo");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_square_icon_stands_in_when_there_is_no_cover() {
        let root = scratch("iconly");
        let dir = root.join("1").join("gog").join("77");
        touch(&dir.join("aa_glx_square_icon_v2.webp"));
        touch(&dir.join("bb_glx_bg_top_padding_7.webp"));

        let art = in_webcache(&root, "77");
        assert!(art.cover.unwrap().ends_with("aa_glx_square_icon_v2.webp"));
        assert!(art.logo.is_none());

        std::fs::remove_dir_all(&root).unwrap();
    }

    // the padding number is part of the file name and has moved before
    #[test]
    fn the_background_matches_whatever_padding_it_was_saved_with() {
        let root = scratch("padding");
        let dir = root.join("1").join("gog").join("55");
        touch(&dir.join("aa_glx_bg_top_padding_12.webp"));

        assert!(in_webcache(&root, "55").hero.is_some());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_game_galaxy_never_saw_is_not_art() {
        let root = scratch("unseen");
        std::fs::create_dir_all(root.join("1").join("gog").join("999")).unwrap();

        assert!(in_webcache(&root, "999").is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    // bought from gog, installed with the offline installer, galaxy never
    // involved. the id is made up so the real webcache cannot match it
    #[test]
    fn falls_back_to_the_icon_the_installer_left() {
        let dir = scratch("offline");
        touch(&dir.join("goggame-4040404040.ico"));

        let art = gog("4040404040", &dir);
        assert!(art.cover.unwrap().ends_with("goggame-4040404040.ico"));
        assert!(art.hero.is_none(), "an icon is not a background");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_on_disk_is_no_art_rather_than_a_panic() {
        let dir = scratch("bare");
        assert!(gog("4040404041", &dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
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
    fn a_folder_with_nothing_in_it_has_no_art() {
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

    #[test]
    fn a_store_we_do_not_read_art_for_is_not_a_panic() {
        let game = InstalledGame {
            name: "Something".into(),
            store: Store::Epic,
            install_dir: PathBuf::from("C:\\g"),
            app_id: None,
            executables: vec![],
            version: None,
        };
        assert!(find(&game).is_empty());
    }
}
