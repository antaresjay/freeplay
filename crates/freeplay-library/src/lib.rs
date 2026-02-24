//! Finding games that are already installed.
//!
//! Every store keeps its own record of what it put where, in its own format,
//! in its own place. None of them agree, so this reads all of them and returns
//! one list.

pub mod vdf;

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    Steam,
    Epic,
    Gog,
    Xbox,
    Manual,
}

impl Store {
    pub fn label(self) -> &'static str {
        match self {
            Store::Steam => "Steam",
            Store::Epic => "Epic",
            Store::Gog => "GOG",
            Store::Xbox => "Xbox",
            Store::Manual => "Manual",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledGame {
    pub name: String,
    pub store: Store,
    pub install_dir: PathBuf,
    pub app_id: Option<String>,
    /// Candidate executables, best guess first.
    pub executables: Vec<PathBuf>,
}

impl InstalledGame {
    /// File name of the most likely executable, which is what a table's
    /// `exe` field is matched against.
    pub fn main_exe(&self) -> Option<String> {
        self.executables
            .first()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
    }
}

/// Installers, crash handlers and redistributables that live next to games and
/// are never the thing you want to attach to.
const NOT_A_GAME: &[&str] = &[
    "unins",
    "uninstall",
    "vcredist",
    "vc_redist",
    "dxsetup",
    "dxwebsetup",
    "directx",
    "oalinst",
    "dotnetfx",
    "crashhandler",
    "crashreport",
    "crashsender",
    "easyanticheat",
    "beservice",
    "touchup",
    "setup",
    "redist",
    "installscript",
];

fn looks_like_a_game(path: &Path) -> bool {
    let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else {
        return false;
    };
    !NOT_A_GAME.iter().any(|bad| stem.contains(bad))
}

/// Executables under `dir`, no deeper than `depth` levels.
///
/// Games bury the real binary in Binaries/Win64 or similar, but walking the
/// whole tree on a 100GB install is slow and turns up mod tools and editors.
fn find_executables(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut here = Vec::new();

    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                && looks_like_a_game(&path)
            {
                found.push(path);
            }
        } else if depth > 0 {
            here.push(path);
        }
    }

    for sub in here {
        found.extend(find_executables(&sub, depth - 1));
    }
    found
}

/// Shallower executables first, since a game's launcher usually sits at the
/// root and the deeper ones tend to be tools.
fn rank(dir: &Path, mut exes: Vec<PathBuf>) -> Vec<PathBuf> {
    exes.sort_by_key(|p| {
        let depth = p.strip_prefix(dir).map(|r| r.components().count()).unwrap_or(9);
        let name = p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        // A binary named after its folder is almost always the game.
        let folder = dir.file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let matches_folder = if folder.contains(&name) || name.contains(&folder) { 0 } else { 1 };
        (matches_folder, depth)
    });
    exes
}

pub fn discover() -> Vec<InstalledGame> {
    let mut games = Vec::new();

    #[cfg(windows)]
    {
        games.extend(steam::discover());
        games.extend(epic::discover());
        games.extend(gog::discover());
    }

    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    games.dedup_by(|a, b| a.install_dir == b.install_dir);
    games
}

fn build(name: String, store: Store, dir: PathBuf, app_id: Option<String>) -> Option<InstalledGame> {
    if !dir.is_dir() {
        return None;
    }
    let executables = rank(&dir, find_executables(&dir, 3));
    Some(InstalledGame { name, store, install_dir: dir, app_id, executables })
}

#[cfg(windows)]
pub mod steam {
    use super::*;
    use crate::vdf;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    fn steam_root() -> Option<PathBuf> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Valve\Steam").ok()?;
        let path: String = key.get_value("SteamPath").ok()?;
        Some(PathBuf::from(path.replace('/', "\\")))
    }

    fn library_paths(root: &Path) -> Vec<PathBuf> {
        let mut paths = vec![root.to_path_buf()];

        let manifest = root.join("steamapps").join("libraryfolders.vdf");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            return paths;
        };
        let Ok(parsed) = vdf::parse(&text) else {
            return paths;
        };
        let Some(folders) = parsed.get("libraryfolders") else {
            return paths;
        };

        for (_, entry) in folders.entries() {
            if let Some(path) = entry.string("path") {
                paths.push(PathBuf::from(path));
            }
        }
        paths.sort();
        paths.dedup();
        paths
    }

    pub fn discover() -> Vec<InstalledGame> {
        let Some(root) = steam_root() else {
            return Vec::new();
        };

        let mut games = Vec::new();
        for library in library_paths(&root) {
            let steamapps = library.join("steamapps");
            let Ok(entries) = std::fs::read_dir(&steamapps) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                let is_manifest = path
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with("appmanifest_"))
                    .unwrap_or(false);
                if !is_manifest {
                    continue;
                }

                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(parsed) = vdf::parse(&text) else {
                    continue;
                };
                let Some(state) = parsed.get("AppState") else {
                    continue;
                };

                let (Some(name), Some(dir)) = (state.string("name"), state.string("installdir"))
                else {
                    continue;
                };

                let install = steamapps.join("common").join(dir);
                if let Some(game) = build(
                    name.to_string(),
                    Store::Steam,
                    install,
                    state.string("appid").map(str::to_string),
                ) {
                    games.push(game);
                }
            }
        }
        games
    }
}

#[cfg(windows)]
pub mod epic {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Manifest {
        display_name: Option<String>,
        install_location: Option<String>,
        launch_executable: Option<String>,
    }

    pub fn discover() -> Vec<InstalledGame> {
        let program_data =
            std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".to_string());
        let dir = PathBuf::from(program_data)
            .join("Epic")
            .join("EpicGamesLauncher")
            .join("Data")
            .join("Manifests");

        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };

        let mut games = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.extension().is_some_and(|e| e == "item") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
                continue;
            };
            let (Some(name), Some(location)) = (manifest.display_name, manifest.install_location)
            else {
                continue;
            };

            let install = PathBuf::from(location);
            if let Some(mut game) = build(name, Store::Epic, install.clone(), None) {
                // Epic records the real binary, so trust it over our guess.
                if let Some(exe) = manifest.launch_executable {
                    let full = install.join(exe.replace('/', "\\"));
                    if full.is_file() {
                        game.executables.retain(|p| p != &full);
                        game.executables.insert(0, full);
                    }
                }
                games.push(game);
            }
        }
        games
    }
}

#[cfg(windows)]
pub mod gog {
    use super::*;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY};
    use winreg::RegKey;

    pub fn discover() -> Vec<InstalledGame> {
        let Ok(root) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(
            r"SOFTWARE\GOG.com\Games",
            KEY_READ | KEY_WOW64_32KEY,
        ) else {
            return Vec::new();
        };

        let mut games = Vec::new();
        for id in root.enum_keys().flatten() {
            let Ok(key) = root.open_subkey(&id) else {
                continue;
            };
            let name: String = key.get_value("gameName").unwrap_or_default();
            let path: String = key.get_value("path").unwrap_or_default();
            if name.is_empty() || path.is_empty() {
                continue;
            }
            if let Some(game) = build(name, Store::Gog, PathBuf::from(path), Some(id)) {
                games.push(game);
            }
        }
        games
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_installers_and_crash_handlers() {
        assert!(!looks_like_a_game(Path::new(r"C:\g\UnityCrashHandler64.exe")));
        assert!(!looks_like_a_game(Path::new(r"C:\g\vcredist_x64.exe")));
        assert!(!looks_like_a_game(Path::new(r"C:\g\unins000.exe")));
        assert!(!looks_like_a_game(Path::new(r"C:\g\EasyAntiCheat_Setup.exe")));
        assert!(!looks_like_a_game(Path::new(r"C:\g\DXSETUP.exe")));
    }

    #[test]
    fn keeps_actual_games() {
        assert!(looks_like_a_game(Path::new(r"C:\g\MassEffect1.exe")));
        assert!(looks_like_a_game(Path::new(r"C:\g\witcher2.exe")));
        assert!(looks_like_a_game(Path::new(r"C:\g\BatmanAK.exe")));
    }

    #[test]
    fn ranks_a_binary_named_after_its_folder_first() {
        let dir = Path::new(r"C:\Games\Witcher2");
        let exes = vec![
            PathBuf::from(r"C:\Games\Witcher2\bin\tools\editor.exe"),
            PathBuf::from(r"C:\Games\Witcher2\witcher2.exe"),
        ];
        let ranked = rank(dir, exes);
        assert!(ranked[0].ends_with("witcher2.exe"));
    }

    #[test]
    fn ranks_shallower_binaries_first_otherwise() {
        let dir = Path::new(r"C:\Games\Thing");
        let exes = vec![
            PathBuf::from(r"C:\Games\Thing\a\b\deep.exe"),
            PathBuf::from(r"C:\Games\Thing\shallow.exe"),
        ];
        assert!(rank(dir, exes)[0].ends_with("shallow.exe"));
    }

    #[test]
    fn main_exe_is_the_file_name() {
        let game = InstalledGame {
            name: "Test".into(),
            store: Store::Steam,
            install_dir: PathBuf::from(r"C:\g"),
            app_id: None,
            executables: vec![PathBuf::from(r"C:\g\Bin\test.exe")],
        };
        assert_eq!(game.main_exe().as_deref(), Some("test.exe"));
    }

    #[test]
    fn a_game_with_no_executables_reports_none() {
        let game = InstalledGame {
            name: "Test".into(),
            store: Store::Gog,
            install_dir: PathBuf::from(r"C:\g"),
            app_id: None,
            executables: vec![],
        };
        assert_eq!(game.main_exe(), None);
    }
}
