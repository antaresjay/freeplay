//! Finding games that are already installed.
//!
//! Every store keeps its own record of what it put where, in its own format,
//! in its own place. None of them agree, so this reads all of them and returns
//! one list.

pub mod art;
pub mod build;
pub mod galaxy;
pub mod launch;
pub mod play;
pub mod vdf;

use rayon::prelude::*;
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
    "crashpad",
    "breakpad",
    "easyanticheat",
    "beservice",
    "battleye",
    "touchup",
    "setup",
    "redist",
    "installscript",
    "cleanup",
    "activation",
    "helper",
    "webhelper",
    "epicwebhelper",
    "subprocess",
    "handler",
    "updater",
    "patcher",
    "diagnostic",
    "benchmark",
    "config",
    "settings",
    "server",
];

/// Steam entries that are tooling rather than something you play.
const NOT_A_TITLE: &[&str] = &[
    "steamworks common redistributables",
    "steam linux runtime",
    "proton",
    "steamvr",
    "steam controller",
];

fn is_a_title(name: &str) -> bool {
    let lower = name.to_lowercase();
    !NOT_A_TITLE.iter().any(|skip| lower.contains(skip))
}

fn looks_like_a_game(path: &Path) -> bool {
    let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else {
        return false;
    };
    !NOT_A_GAME.iter().any(|bad| stem.contains(bad))
}

/// Folders that never hold the game binary and can be large.
const SKIP_DIRS: &[&str] = &[
    "_commonredist",
    "redist",
    "directx",
    "vcredist",
    "dotnet",
    "content",
    "data",
    "movies",
    "sound",
    "audio",
    "textures",
    "docs",
    "manual",
    "saves",
    "mods",
];

/// Executables under `dir`, no deeper than `depth` levels.
///
/// Depth has to be generous because engines bury the real binary a long way
/// down. Mass Effect Legendary Edition keeps it at Game/ME1/Binaries/Win64,
/// which is five levels in. Skipping the asset folders keeps that affordable.
///
/// Subdirectories are walked in parallel. Mass Effect alone is thirty six
/// thousand files and doing that one at a time is most of the wait before the
/// library appears. Rayon's flat_map keeps the order, so the ranking below
/// still gets the same list every run.
fn find_executables(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut subdirs = Vec::new();

    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                && looks_like_a_game(&path)
            {
                found.push(path);
            }
        } else if depth > 0 {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !SKIP_DIRS.iter().any(|skip| name == *skip) {
                subdirs.push(path);
            }
        }
    }

    found.par_extend(
        subdirs
            .into_par_iter()
            .flat_map_iter(|sub| find_executables(&sub, depth - 1)),
    );
    found
}

// how much two squashed names look like each other, bigger is better
//
// containment wins outright, which covers a short executable name sitting
// inside a long store title. otherwise a shared prefix counts, which is what
// links "MassEffect1" to "Mass Effect Legendary Edition" where neither one
// contains the other
fn similarity(title: &str, stem: &str) -> usize {
    if title.is_empty() || stem.is_empty() {
        return 0;
    }
    // Both sides have to be long enough to mean anything. A folder called "ME"
    // otherwise matches inside "Something" and wins outright.
    if title.len() >= 4 && stem.len() >= 4 && (title.contains(stem) || stem.contains(title)) {
        return 1000;
    }
    let shared = title
        .chars()
        .zip(stem.chars())
        .take_while(|(a, b)| a == b)
        .count();
    if shared >= 4 {
        shared
    } else {
        0
    }
}

/// Strip everything but letters and digits so "Mass Effect™ Legendary Edition"
/// and "MassEffectLauncher" can be compared.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Best guess at the real game binary, first.
///
/// Name is the strongest signal by far. After that, size: the game itself is
/// nearly always the largest executable in its own folder, while the launchers
/// and helpers around it are small.
fn rank(dir: &Path, game_name: &str, mut exes: Vec<PathBuf>) -> Vec<PathBuf> {
    let title = squash(game_name);
    let folder = squash(
        &dir.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
    );

    let stem_of = |path: &Path| {
        squash(
            &path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
        )
    };
    let score = |path: &Path| {
        let stem = stem_of(path);
        similarity(&title, &stem).max(similarity(&folder, &stem))
    };

    // Launchers score deceptively well on name alone: "MassEffectLauncher"
    // shares more letters with "Mass Effect Legendary Edition" than
    // "MassEffect1" does. Push them below anything else that looks related,
    // but only when there is something else, since plenty of games really are
    // launched through one.
    let has_alternative = exes
        .iter()
        .any(|p| !stem_of(p).contains("launch") && score(p) > 0);

    exes.sort_by_key(|path| {
        let demoted = usize::from(has_alternative && stem_of(path).contains("launch"));
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        (
            demoted,
            std::cmp::Reverse(score(path)),
            std::cmp::Reverse(size),
        )
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

    games.sort_by_key(|a| a.name.to_lowercase());
    games.dedup_by(|a, b| a.install_dir == b.install_dir);
    games
}

fn build(
    name: String,
    store: Store,
    dir: PathBuf,
    app_id: Option<String>,
) -> Option<InstalledGame> {
    if !dir.is_dir() || !is_a_title(&name) {
        return None;
    }
    let executables = rank(&dir, &name, find_executables(&dir, 5));
    if executables.is_empty() {
        return None;
    }
    Some(InstalledGame {
        name,
        store,
        install_dir: dir,
        app_id,
        executables,
    })
}

#[cfg(windows)]
pub mod steam {
    use super::*;
    use crate::vdf;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    /// Where the client itself is installed, which is also where it caches
    /// library art. Games can live on other drives, this cannot.
    pub fn root() -> Option<PathBuf> {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Valve\Steam")
            .ok()?;
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
        let Some(root) = root() else {
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
            if path.extension().is_none_or(|e| e != "item") {
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
        let Ok(root) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(r"SOFTWARE\GOG.com\Games", KEY_READ | KEY_WOW64_32KEY)
        else {
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
        assert!(!looks_like_a_game(Path::new(
            r"C:\g\UnityCrashHandler64.exe"
        )));
        assert!(!looks_like_a_game(Path::new(r"C:\g\vcredist_x64.exe")));
        assert!(!looks_like_a_game(Path::new(r"C:\g\unins000.exe")));
        assert!(!looks_like_a_game(Path::new(
            r"C:\g\EasyAntiCheat_Setup.exe"
        )));
        assert!(!looks_like_a_game(Path::new(r"C:\g\DXSETUP.exe")));
    }

    #[test]
    fn keeps_actual_games() {
        assert!(looks_like_a_game(Path::new(r"C:\g\MassEffect1.exe")));
        assert!(looks_like_a_game(Path::new(r"C:\g\witcher2.exe")));
        assert!(looks_like_a_game(Path::new(r"C:\g\BatmanAK.exe")));
    }

    #[test]
    fn ignores_the_helpers_that_sit_next_to_games() {
        assert!(!looks_like_a_game(Path::new(r"C:\g\breakpad_server.exe")));
        assert!(!looks_like_a_game(Path::new(r"C:\g\Cleanup.exe")));
        assert!(!looks_like_a_game(Path::new(r"C:\g\EpicWebHelper.exe")));
        assert!(!looks_like_a_game(Path::new(r"C:\g\Discovery_Server.exe")));
    }

    #[test]
    fn ranks_a_binary_named_after_the_game_first() {
        let dir = Path::new(r"C:\Games\ME");
        let exes = vec![
            PathBuf::from(r"C:\Games\ME\Something.exe"),
            PathBuf::from(r"C:\Games\ME\MassEffect1.exe"),
        ];
        let ranked = rank(dir, "Mass Effect\u{2122} Legendary Edition", exes);
        assert!(ranked[0].ends_with("MassEffect1.exe"));
    }

    #[test]
    fn punctuation_and_case_do_not_matter_when_matching() {
        let dir = Path::new(r"C:\Games\W2");
        let exes = vec![
            PathBuf::from(r"C:\Games\W2\other.exe"),
            PathBuf::from(r"C:\Games\W2\witcher2.exe"),
        ];
        let ranked = rank(dir, "The Witcher 2: Assassins of Kings", exes);
        assert!(ranked[0].ends_with("witcher2.exe"));
    }

    #[test]
    fn a_launcher_loses_to_the_game_itself() {
        let dir = Path::new(r"C:\Games\Thing");
        let exes = vec![
            PathBuf::from(r"C:\Games\Thing\ThingLauncher.exe"),
            PathBuf::from(r"C:\Games\Thing\Thing.exe"),
        ];
        assert!(rank(dir, "Thing", exes)[0].ends_with("Thing.exe"));
    }

    #[test]
    fn a_launcher_that_scores_higher_still_loses() {
        // The real case: MassEffectLauncher shares more letters with the title
        // than MassEffect1 does, purely by accident.
        let dir = Path::new(r"C:\Games\Mass Effect Legendary Edition");
        let exes = vec![
            PathBuf::from(
                r"C:\Games\Mass Effect Legendary Edition\Game\Launcher\MassEffectLauncher.exe",
            ),
            PathBuf::from(r"C:\Games\Mass Effect Legendary Edition\Game\ME1\MassEffect1.exe"),
        ];
        let ranked = rank(dir, "Mass Effect\u{2122} Legendary Edition", exes);
        assert!(
            ranked[0].ends_with("MassEffect1.exe"),
            "got {:?}",
            ranked[0]
        );
    }

    #[test]
    fn a_launcher_wins_when_it_is_the_only_thing_that_matches() {
        let dir = Path::new(r"C:\Games\Thing");
        let exes = vec![
            PathBuf::from(r"C:\Games\Thing\unrelated.exe"),
            PathBuf::from(r"C:\Games\Thing\ThingLauncher.exe"),
        ];
        assert!(rank(dir, "Thing", exes)[0].ends_with("ThingLauncher.exe"));
    }

    #[test]
    fn redistributables_are_not_games() {
        assert!(!is_a_title("Steamworks Common Redistributables"));
        assert!(!is_a_title("Steam Linux Runtime 3.0"));
        assert!(is_a_title("Mass Effect Legendary Edition"));
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
