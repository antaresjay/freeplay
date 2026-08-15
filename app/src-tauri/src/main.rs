#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! desktop front end. all glue, the real work is in core and session

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use freeplay_core::search::{Filter, Search};
use freeplay_core::target::Target;
use freeplay_core::value::ValueKind;
use freeplay_core::windows_target::{processes, WindowsTarget};
use freeplay_core::Error as CoreError;
use freeplay_library::{discover, InstalledGame, Store};
use freeplay_session::Session;
use freeplay_table::resolve::State as CheatState;
use freeplay_table::Table;
use serde::Serialize;

mod community;
mod dialog;
mod hotkey;
mod log;
mod merge;
mod overlay;
mod place;
mod profile;
mod settings;
mod shared;
mod ui_contract;
use settings::Settings;
use tauri::{Emitter, Manager};

#[derive(Default)]
struct App {
    target: Mutex<Option<Arc<dyn Target>>>,
    session: Mutex<Option<Session>>,
    search: Mutex<Option<Search>>,
    // where each game's pictures are, keyed by app id. resolved once here
    // because working it out needs the store and the install dir, and the
    // protocol handler serving the bytes has neither
    art: Mutex<HashMap<String, freeplay_library::art::Art>>,
    // anti-cheat found in a game's folder, keyed by install dir
    guards: Mutex<HashMap<PathBuf, Option<Guard>>>,
    // walking every install dir takes seconds, so do it once and then only
    // when asked
    library: Mutex<Option<Vec<InstalledGame>>>,
    // detached by hand, so don't grab it again until the game restarts
    declined: Mutex<Option<String>>,
    // parsed tables. the library polls every few seconds and a .CT is xml, so
    // reparsing every time is real work for nothing
    tables: Mutex<Option<Vec<Table>>>,
    settings: Mutex<Settings>,
    // a profile that has been opened and shown to you, waiting on the yes
    pending: Mutex<Option<profile::Profile>>,
    // the overlay key, held open for as long as it is registered
    hotkey: Mutex<Option<hotkey::Listener>>,
    // every key the table binds, held while a game is attached
    bank: Mutex<Option<hotkey::Bank>>,
    // when each cheat was switched on this sitting, for working out what to
    // blame if the game goes down
    lit: Mutex<HashMap<String, std::time::Instant>>,
    // set once at startup, so a key landing on its own thread can reach the
    // rest of the app
    handle: std::sync::OnceLock<tauri::AppHandle>,
    // the shared table in play right now, so the question afterwards is about
    // the one that was actually running and not whatever is installed by then
    playing: Mutex<Option<Playing>>,
}

struct Playing {
    id: i64,
    exe: String,
    game: String,
    by: String,
    started: std::time::Instant,
}

#[derive(Serialize)]
struct GameRow {
    // stable across launches, pinning and favourites key off it. app id if
    // there is one, install path otherwise
    key: String,
    name: String,
    store: String,
    exe: Option<String>,
    dir: String,
    app_id: Option<String>,
    running: bool,
    has_table: bool,
    // anti-cheat shipped with the game, if any
    guard: Option<String>,
    // the file it was found in, kept only when the product could not be named,
    // so the page can show what it actually saw instead of a shrug
    guard_file: Option<String>,
    // minutes played and when. steam keeps both in a config file, gog only in
    // galaxy's database, so an offline gog install has neither
    minutes: Option<u32>,
    last_played: Option<u64>,
    // gog is the only one that tells us either of these
    version: Option<String>,
    genres: Vec<String>,
    favourite: bool,
}

fn key_for(game: &InstalledGame) -> String {
    let store = game.store.label().to_lowercase();
    match &game.app_id {
        Some(id) => format!("{store}:{id}"),
        None => format!("{store}:{}", game.install_dir.display()),
    }
}

#[derive(Serialize, Clone, Default)]
struct ArtUrls {
    cover: Option<String>,
    hero: Option<String>,
    logo: Option<String>,
}

#[derive(Serialize)]
struct ProcessRow {
    pid: u32,
    name: String,
}

#[derive(Serialize, Clone)]
struct Attached {
    process: String,
    pid: u32,
    game: String,
    table: bool,
    // 32-bit or 64-bit. decides whether a table written for the other build
    // resolves at all
    arch: String,
}

#[derive(Serialize)]
struct CheatRow {
    id: String,
    name: String,
    // which table it came from, when more than one is switched on
    from: String,
    category: String,
    description: String,
    hint: String,
    // ready, wait, broken or on
    state: String,
    reason: String,
    // what you want on, game running or not
    armed: bool,
    // actually doing something right now
    live: bool,
    // the key that flips it while you play, "" when there is none
    key: String,
    // the game went down right after this one was switched on, last time
    suspect: bool,
    does: String,
    // takes a number rather than being a plain switch
    editable: bool,
    // i32, f32 and so on, for the placeholder
    kind: String,
    // what is in the box
    value: String,
    // what the game is holding at this instant, only once attached
    current: String,
    // "0:Off" style options if the table author gave any
    choices: Vec<ChoiceRow>,
    hex: bool,
    // holds the number against the game writing it back
    holds: bool,
}

#[derive(Serialize)]
struct ChoiceRow {
    value: String,
    label: String,
}

#[derive(Serialize)]
struct ScanReport {
    round: usize,
    matches: usize,
    results: Vec<Hit>,
}

#[derive(Serialize)]
struct Hit {
    address: String,
    value: String,
}

fn tables_dir() -> PathBuf {
    // next to the exe once installed, in the repo while developing
    let beside = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("tables")));
    if let Some(dir) = beside.filter(|d| d.is_dir()) {
        return dir;
    }
    PathBuf::from("tables")
}

// downloaded tables end up here
fn synced_dir() -> PathBuf {
    freeplay_sync::cache_dir(&settings::path())
}

// tables converted from a .CT on this machine. next to the exe is where the
// bundled ones live and that folder is inside program files once installed,
// so writing an import there fails on anybody's machine but a developer's
fn mine_dir() -> PathBuf {
    settings::path()
        .parent()
        .unwrap_or(Path::new("."))
        .join("mine")
}

/* three folders can each hold a table for the same game, and now several can
sit in one folder too. newest first, because the one you picked most
recently is the one you meant, and ranking by folder instead meant an
imported .CT shadowed every table you downloaded afterwards. */
fn load_all() -> Vec<(String, Table)> {
    let mut found: Vec<(std::time::SystemTime, String, Table)> = Vec::new();

    for dir in [mine_dir(), synced_dir(), tables_dir()] {
        for (path, table) in Table::load_dir_with_paths(&dir) {
            let touched = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            found.push((touched, tag_of(&path, &table), table));
        }
    }

    found.sort_by_key(|(touched, _, _)| std::cmp::Reverse(*touched));

    // the same file reachable twice, or two copies of one table, is one table
    let mut seen = std::collections::HashSet::new();
    found
        .into_iter()
        .filter(|(_, tag, _)| seen.insert(tag.clone()))
        .map(|(_, tag, table)| (tag, table))
        .collect()
}

/* what a cheat's id gets filed under once tables are folded together, so it
has to survive a restart. the fingerprint is content based, which means
renaming a file or downloading the same table twice does not lose what you
had switched on. */
fn tag_of(path: &Path, table: &Table) -> String {
    let _ = path;
    let print = freeplay_table::fingerprint::fingerprint(table);
    print
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(8)
        .collect()
}

fn load_tables() -> Vec<Table> {
    load_all().into_iter().map(|(_, table)| table).collect()
}

fn tables(state: &tauri::State<'_, App>) -> Vec<Table> {
    if let Some(held) = state.tables.lock().unwrap().as_ref() {
        return held.clone();
    }
    let found = load_tables();
    *state.tables.lock().unwrap() = Some(found.clone());
    found
}

fn forget_tables(state: &tauri::State<'_, App>) {
    *state.tables.lock().unwrap() = None;
}

/* every table installed for this game, newest first. more than one is the
normal case now: no single table has health and ammo and speed in it. */
fn tables_for(exe: &str) -> Vec<(String, Table)> {
    load_all()
        .into_iter()
        .filter(|(_, t)| t.matches_process(exe))
        .collect()
}

// everything installed for the game except what was switched off, folded
fn with_tables(exe: &str, off: &[String]) -> Option<Table> {
    let mut parts = tables_for(exe);
    parts.retain(|(tag, _)| !off.contains(tag));
    merge::fold(parts)
}

// the version stamped into the game's exe. a table is written against one
// build and quietly stops working on the next, so every share and every vote
// carries the build it was judged on
fn build_of(state: &tauri::State<'_, App>, exe: &str) -> String {
    let wanted = exe.to_lowercase();
    let Some(game) = library(state, false).into_iter().find(|game| {
        game.main_exe()
            .is_some_and(|found| found.to_lowercase() == wanted)
    }) else {
        return String::new();
    };

    game.executables
        .first()
        .and_then(|path| freeplay_library::build::of(path))
        // plenty of games carry no version resource at all. gog writes one to
        // the registry, which is the only thing those have
        .or(game.version)
        .unwrap_or_default()
}

// what the library calls a game beats what the table calls it. a .CT names the
// game after its own file, so a table converted from one ends up called
// something like "skyrimse" instead of the name on the store page
fn nice_name(state: &tauri::State<'_, App>, exe: &str, fallback: &str) -> String {
    let wanted = exe.to_lowercase();
    library(state, false)
        .into_iter()
        .find(|game| {
            game.main_exe()
                .is_some_and(|found| found.to_lowercase() == wanted)
        })
        .map(|game| game.name)
        .unwrap_or_else(|| fallback.to_string())
}

// what is in a game's install folder, two deep. anti-cheats drop their loader
// next to the exe or one folder down, so that is far enough
fn folder_names(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 || out.len() > 4000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        out.push(entry.path());
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            folder_names(&entry.path(), depth - 1, out);
        }
    }
}

// version blocks say "Denuvo Anti-Cheat Installer" and the like. the installer
// is not the thing, the product is
fn trim_the_wrapper(text: &str) -> String {
    let mut name = text.trim();
    loop {
        let shorter = [
            "Installer",
            "Setup",
            "Launcher",
            "Service",
            "Client",
            "Bootstrapper",
        ]
        .iter()
        .find_map(|tail| name.strip_suffix(tail))
        .map(str::trim);
        match shorter {
            Some(cut) if !cut.is_empty() => name = cut,
            _ => break,
        }
    }
    name.trim_end_matches(',').trim().to_string()
}

// a file called AntiCheatInstaller.exe is obviously an anti-cheat and just as
// obviously not a name anybody can act on. ask the binary whose it is
fn name_from_the_file(path: &Path) -> Option<String> {
    let told = freeplay_library::build::describes_itself(path)?;
    if let Some(known) = freeplay_core::guard::product_for(&told.to_ascii_lowercase()) {
        if known != "an anti-cheat" {
            return Some(known.to_string());
        }
    }
    let cleaned = trim_the_wrapper(&told);
    let lowered = cleaned.to_ascii_lowercase();
    // only worth showing if it reads as an anti-cheat rather than as the game
    (lowered.contains("anti") || lowered.contains("cheat") || lowered.contains("guard"))
        .then_some(cleaned)
}

#[derive(Clone)]
struct Guard {
    name: String,
    // only set when the product could not be named
    file: Option<String>,
}

fn guard_for(state: &tauri::State<'_, App>, dir: &Path) -> Option<Guard> {
    if let Some(cached) = state.guards.lock().unwrap().get(dir) {
        return cached.clone();
    }

    /* a game folder with anti-cheat files in it is a strong hint. a windows
    folder is not: system32 holds the installed anti-cheat services for every
    game on the machine, and a hand added exe living there was reading as
    guarded by all of them. the check that counts still runs at attach */
    let system = std::env::var("WINDIR").is_ok_and(|windir| {
        let lowered = PathBuf::from(dir.display().to_string().to_lowercase());
        lowered.starts_with(PathBuf::from(windir.to_lowercase()))
    });
    if system {
        state.guards.lock().unwrap().insert(dir.to_path_buf(), None);
        return None;
    }

    let mut paths = Vec::new();
    folder_names(dir, 2, &mut paths);
    let names: Vec<String> = paths
        .iter()
        .map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .collect();

    let found = freeplay_core::guard::look(names.iter().map(String::as_str)).map(|spotted| {
        let named = spotted.product.map(str::to_string).or_else(|| {
            let at = names.iter().position(|n| *n == spotted.found_in)?;
            name_from_the_file(&paths[at])
        });
        match named {
            Some(name) => Guard { name, file: None },
            None => Guard {
                name: "an anti-cheat".into(),
                file: Some(spotted.found_in),
            },
        }
    });

    state
        .guards
        .lock()
        .unwrap()
        .insert(dir.to_path_buf(), found.clone());
    found
}

// scanning takes seconds and nothing changes while the app is open, so cache
// it until the refresh button says otherwise
fn library(state: &tauri::State<'_, App>, refresh: bool) -> Vec<InstalledGame> {
    if !refresh {
        if let Some(cached) = state.library.lock().unwrap().as_ref() {
            return cached.clone();
        }
    }

    let mut found = discover();
    hand_adds(state, &mut found);
    *state.library.lock().unwrap() = Some(found.clone());
    found
}

// the games pointed at by hand, folded in after the store scan. one a store
// already found is skipped rather than listed twice
fn hand_adds(state: &tauri::State<'_, App>, found: &mut Vec<InstalledGame>) {
    let added = state.settings.lock().unwrap().added.clone();
    for text in added {
        let exe = PathBuf::from(&text);
        if !exe.is_file() {
            continue;
        }
        let mine = text.to_lowercase();
        let twice = found.iter().any(|game| {
            game.executables
                .iter()
                .any(|e| e.display().to_string().to_lowercase() == mine)
        });
        if twice {
            continue;
        }
        found.push(InstalledGame {
            name: called(&exe),
            store: Store::Manual,
            install_dir: exe
                .parent()
                .map(|d| d.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
            app_id: None,
            version: freeplay_library::build::of(&exe),
            executables: vec![exe],
        });
    }
}

// what to call an exe somebody pointed at. its version block usually says
fn called(exe: &Path) -> String {
    if let Some(told) = freeplay_library::build::describes_itself(exe) {
        if !told.to_ascii_lowercase().contains("operating system") {
            return told;
        }
    }
    exe.file_stem()
        .map(|s| s.to_string_lossy().replace(['_', '-'], " "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Added game".into())
}

// point freeplay at any exe. no path means ask with the picker, and the
// empty string back means it was closed without choosing, worth no toast
#[tauri::command]
async fn add_game(
    app: tauri::AppHandle,
    state: tauri::State<'_, App>,
    path: Option<String>,
) -> Result<String, String> {
    let path = match path {
        Some(text) => {
            let given = PathBuf::from(&text);
            if !given.is_file() {
                return Err(format!("there is no file at {text}"));
            }
            given
        }
        None => match dialog::open(&dialog::Ask {
            owner: owner_window(&app),
            title: "Pick the game's exe",
            kinds: &[("Programs", "*.exe")],
            suggested: "",
            extension: "exe",
        }) {
            Some(picked) => picked,
            None => return Ok(String::new()),
        },
    };

    let text = path.display().to_string();
    {
        let mut settings = state.settings.lock().unwrap();
        if !settings
            .added
            .iter()
            .any(|held| held.eq_ignore_ascii_case(&text))
        {
            settings.added.push(text);
            settings::save(&settings)?;
        }
    }
    *state.library.lock().unwrap() = None;
    Ok(called(&path))
}

#[tauri::command]
fn remove_added(state: tauri::State<'_, App>, dir: String) -> Result<(), String> {
    let wanted = dir.to_lowercase();
    {
        let mut settings = state.settings.lock().unwrap();
        settings.added.retain(|held| {
            PathBuf::from(held)
                .parent()
                .map(|p| p.display().to_string().to_lowercase())
                != Some(wanted.clone())
        });
        settings::save(&settings)?;
    }
    *state.library.lock().unwrap() = None;
    Ok(())
}

// async so it lands on a worker. a sync command runs on the main thread and
// the window stops answering while it works
#[tauri::command]
async fn list_games(state: tauri::State<'_, App>, refresh: bool) -> Result<Vec<GameRow>, ()> {
    if refresh {
        forget_tables(&state);
    }
    let running: Vec<String> = processes()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.name.to_lowercase())
        .collect();
    let tables = tables(&state);
    let played = freeplay_library::play::steam();
    // epic only records when, never how long
    let epic_played = freeplay_library::play::epic();
    let galaxy = freeplay_library::galaxy::details();
    let steam_genres = freeplay_library::steam::root()
        .map(|root| freeplay_library::appinfo::genres(&root))
        .unwrap_or_default();
    let favourites = state.settings.lock().unwrap().favourites.clone();
    let switched_off = state.settings.lock().unwrap().off.clone();
    let sittings = state.settings.lock().unwrap().sittings.clone();

    Ok(library(&state, refresh)
        .into_iter()
        .map(|game| {
            let exe = game.main_exe();
            let lower = exe.as_deref().unwrap_or_default().to_lowercase();
            let key = key_for(&game);
            // a steam app id and a gog game id are both just numbers, so the
            // store has to pick which list to look in
            let mine = game
                .app_id
                .as_deref()
                .filter(|_| game.store == Store::Steam);
            let theirs = game.app_id.as_deref().filter(|_| game.store == Store::Gog);
            let epic = game.app_id.as_deref().filter(|_| game.store == Store::Epic);
            let play = mine
                .and_then(|id| played.get(id))
                .or_else(|| epic.and_then(|id| epic_played.get(id)))
                .copied()
                .or_else(|| theirs.and_then(|id| galaxy.get(id)).map(|d| d.play))
                .unwrap_or_default();
            let genres = theirs
                .and_then(|id| galaxy.get(id))
                .map(|d| d.genres.clone())
                .or_else(|| mine.and_then(|id| steam_genres.get(id)).cloned())
                .unwrap_or_default();

            let guard = guard_for(&state, &game.install_dir);

            GameRow {
                guard_file: guard.as_ref().and_then(|g| g.file.clone()),
                guard: guard.map(|g| g.name),
                running: !lower.is_empty() && running.iter().any(|p| p == &lower),
                /* a table installed and switched off is not a table this
                page will show anything for, and the page uses this to
                decide whether to put up cheat shaped placeholders */
                /* a table installed and switched off shows nothing, and the
                   page uses this to decide whether to put up placeholders */
                has_table: exe.as_deref().is_some_and(|e| {
                    let dropped = switched_off.get(&e.to_lowercase());
                    match dropped {
                        None => tables.iter().any(|t| t.matches_process(e)),
                        Some(d) => tables_for(e).iter().any(|(tag, _)| !d.contains(tag)),
                    }
                }),
                // the store's own count wins when it has one. ours fills in
                // for gog, manual installs and games started from the exe
                minutes: play.minutes.or_else(|| {
                    let ours = sittings.get(&lower).copied().unwrap_or_default();
                    (ours.seconds >= 60).then_some((ours.seconds / 60) as u32)
                }),
                last_played: {
                    let ours = sittings.get(&lower).map(|s| s.last).unwrap_or(0);
                    play.last_played.max((ours > 0).then_some(ours as u64))
                },
                version: game.version,
                genres,
                favourite: favourites.contains(&key),
                key,
                name: game.name,
                store: game.store.label().to_string(),
                dir: game.install_dir.display().to_string(),
                app_id: game.app_id,
                exe,
            }
        })
        .collect())
}

#[tauri::command]
fn settings(state: tauri::State<'_, App>) -> Settings {
    state.settings.lock().unwrap().clone()
}

// everything worth pasting into an issue, so nobody gets talked through
// finding a log file
#[tauri::command]
fn diagnostics(state: tauri::State<'_, App>) -> String {
    let attached = match state.target.lock().unwrap().as_ref() {
        Some(target) => format!("attached to {} (pid {})\n", target.name(), target.pid()),
        None => "not attached\n".to_string(),
    };
    let tables = tables(&state);
    let listed = tables
        .iter()
        .map(|t| {
            format!(
                "  {} ({}), {} cheats",
                t.game.name,
                t.game.exe,
                t.cheats.len()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let extra = format!(
        "{attached}tables dir: {}\ntables loaded: {}\n{listed}\n",
        tables_dir().display(),
        tables.len()
    );
    log::report(&extra)
}

// convert a .CT and keep it. dropping the file on the window is the whole
// flow, nobody should have to go find a folder
#[tauri::command]
fn import_table(
    state: tauri::State<'_, App>,
    path: String,
    exe: Option<String>,
) -> Result<String, String> {
    let source = PathBuf::from(&path);
    let extension = source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if extension != "ct" {
        return Err(format!("{path} is not a .CT file"));
    }

    let xml = std::fs::read_to_string(&source).map_err(|e| format!("could not read it: {e}"))?;
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // whatever we are attached to beats guessing from the file name
    let exe = exe.unwrap_or_else(|| {
        if stem.to_lowercase().ends_with(".exe") {
            stem.clone()
        } else {
            format!("{stem}.exe")
        }
    });
    let title = stem.trim_end_matches(".exe").trim_end_matches(".EXE");
    let title = nice_name(&state, &exe, title);

    let imported = freeplay_table::cheatengine::import(&xml, &exe, &title)?;
    if imported.table.cheats.is_empty() {
        for skip in &imported.skipped {
            tracing::info!("import skipped {:?}: {}", skip.name, skip.why);
        }
        let n = imported.skipped.len();
        return Err(format!(
            "None of the {n} entries in that table can work without injecting code into the game, \
             which Freeplay does not do: {}. The full list is in the log.",
            imported.breakdown()
        ));
    }

    let dir = mine_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let destination = dir.join(format!("{}.toml", exe.trim_end_matches(".exe")));

    let text = toml::to_string_pretty(&imported.table).map_err(|e| e.to_string())?;
    std::fs::write(&destination, text)
        .map_err(|e| format!("could not write {}: {e}", destination.display()))?;

    for skip in &imported.skipped {
        tracing::info!("import skipped {:?}: {}", skip.name, skip.why);
    }
    tracing::info!("imported {} into {}", path, destination.display());
    forget_tables(&state);
    reseat(&state, &exe);

    Ok(format!(
        "{} for {exe}. {}",
        imported.summary(),
        destination.display()
    ))
}

#[tauri::command]
async fn shared_tables(
    state: tauri::State<'_, App>,
    exe: String,
    sort: String,
) -> Result<Vec<shared::Shared>, String> {
    // this one runs on its own whenever a game page opens, so it is the one
    // that has to be switchable. refused here as well as hidden in the page
    online(&state)?;
    let have: Vec<String> = tables(&state)
        .iter()
        .map(freeplay_table::fingerprint::fingerprint)
        .collect();
    let build = build_of(&state, &exe);
    shared::list(&exe, &build, &sort, &have)
}

#[tauri::command]
fn sort_options() -> Vec<shared::SortOption> {
    shared::sorts()
}

// every table whose game name looks like this, whatever binary it is filed
// under. the way out when we guessed the wrong executable for a game
#[tauri::command]
async fn search_tables(
    state: tauri::State<'_, App>,
    query: String,
) -> Result<Vec<shared::Shared>, String> {
    online(&state)?;
    shared::search(&query)
}

// for_exe is set when the table was found by searching rather than offered
// for this game, so it is filed under a different binary and has to be
// pointed at this one or it will never show up
#[tauri::command]
async fn install_shared(
    state: tauri::State<'_, App>,
    id: i64,
    for_exe: Option<String>,
    replace: Option<bool>,
) -> Result<String, String> {
    online(&state)?;
    let install_id = state.settings.lock().unwrap().install_id.clone();
    let (_, table) = shared::install(id, &install_id, &synced_dir(), for_exe.as_deref())?;

    /* taking a table replaces what was there unless you asked to add it. the
    old behaviour was always to add, so picking a second table silently gave
    you both welded together */
    if replace.unwrap_or(true) {
        let keep = freeplay_table::fingerprint::fingerprint(&table);
        forget_tables(&state);
        for (tag, other) in tables_for(&table.game.exe) {
            if freeplay_table::fingerprint::fingerprint(&other) == keep {
                continue;
            }
            let _ = tag;
            delete_table_file(&other);
        }
        state
            .settings
            .lock()
            .unwrap()
            .off
            .remove(&table.game.exe.to_lowercase());
    }

    {
        let mut settings = state.settings.lock().unwrap();
        settings.grabbed.insert(table.game.exe.to_lowercase(), id);
        let _ = settings::save(&settings);
    }
    forget_tables(&state);
    reseat(&state, &table.game.exe);
    Ok(format!(
        "{} is ready, {} cheats",
        table.game.name,
        table.cheats.len()
    ))
}

// the file a table was loaded from, so one of several can be removed on its own
fn delete_table_file(wanted: &Table) {
    let print = freeplay_table::fingerprint::fingerprint(wanted);
    for dir in [mine_dir(), synced_dir()] {
        for (path, table) in Table::load_dir_with_paths(&dir) {
            if freeplay_table::fingerprint::fingerprint(&table) == print {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

// getting a table was one click, getting rid of it meant knowing which folder
// in appdata to go and delete out of
#[tauri::command]
fn remove_table(state: tauri::State<'_, App>, exe: String) -> Result<String, String> {
    let stem = exe.to_lowercase();
    let stem = stem.trim_end_matches(".exe");
    let mut gone = 0usize;

    for dir in [mine_dir(), synced_dir()] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            // match on what the table says it is for, not the file name, since
            // nothing forces the two to agree
            let matches = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| Table::parse(&text).ok())
                .is_some_and(|table| table.matches_process(&exe));

            if matches || path.file_stem().and_then(|s| s.to_str()) == Some(stem) {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("could not delete {}: {e}", path.display()))?;
                gone += 1;
            }
        }
    }

    if gone == 0 {
        // the only copy left is one that shipped with freeplay, and that one
        // is not ours to delete
        return Err("that table came bundled with Freeplay, so there is nothing to remove".into());
    }

    {
        let mut settings = state.settings.lock().unwrap();
        settings.grabbed.remove(&exe.to_lowercase());
        settings.armed.remove(&exe.to_lowercase());
        settings.values.remove(&exe.to_lowercase());
        let _ = settings::save(&settings);
    }
    forget_tables(&state);
    // there is no table left to sit on, so this puts back whatever was on and
    // leaves the game attached with nothing
    if let Some(mut session) = state.session.lock().unwrap().take() {
        remember_the_sitting(&state, &session);
        session.stop();
        session.disable_all();
    }
    tracing::info!("removed the table for {exe}");

    Ok("Removed. What you had switched on for it is forgotten too".to_string())
}

#[tauri::command]
async fn share_table(
    state: tauri::State<'_, App>,
    exe: String,
    anonymous: bool,
) -> Result<String, String> {
    online(&state)?;
    let table = tables(&state)
        .into_iter()
        .find(|t| t.matches_process(&exe))
        .ok_or("there is no table for that game to share")?;

    let toml = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    let build = build_of(&state, &exe);
    let (id, already) = shared::share(&table, &toml, anonymous, &build)?;

    Ok(if already {
        format!("somebody already shared that one, it is number {id}")
    } else {
        format!("shared, it is number {id}")
    })
}

#[derive(Serialize)]
struct Who {
    name: String,
}

#[tauri::command]
fn whoami() -> Option<Who> {
    shared::me().map(|who| Who { name: who.name })
}

#[tauri::command]
async fn claim_name(name: String) -> Result<Vec<String>, String> {
    shared::claim(&name)
}

#[tauri::command]
async fn recover_name(name: String, phrase: String) -> Result<String, String> {
    shared::recover(&name, &phrase)
}

#[tauri::command]
fn forget_name() -> Result<(), String> {
    shared::forget()
}

/* ---------- moving to another machine ---------- */

#[cfg(windows)]
fn owner_window(app: &tauri::AppHandle) -> isize {
    app.get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn owner_window(_app: &tauri::AppHandle) -> isize {
    0
}

#[derive(Serialize)]
struct ProfileGame {
    exe: String,
    name: String,
    cheats: usize,
    values: usize,
    shared: bool,
}

// what the export sheet lists. a game is worth offering if freeplay is holding
// anything for it, installed or not
#[tauri::command]
fn profile_games(state: tauri::State<'_, App>) -> Vec<ProfileGame> {
    let names = table_names(&state);
    let settings = state.settings.lock().unwrap();

    profile::known(&settings)
        .into_iter()
        .map(|exe| {
            let armed = settings.armed.get(&exe).map(Vec::len).unwrap_or_default();
            let values = settings
                .values
                .get(&exe)
                .map(HashMap::len)
                .unwrap_or_default();
            ProfileGame {
                name: names
                    .get(&exe)
                    .cloned()
                    .unwrap_or_else(|| exe.trim_end_matches(".exe").to_string()),
                cheats: armed,
                values,
                shared: settings.grabbed.contains_key(&exe),
                exe,
            }
        })
        .filter(|game| game.cheats > 0 || game.values > 0 || game.shared)
        .collect()
}

fn table_names(state: &tauri::State<'_, App>) -> HashMap<String, String> {
    tables(state)
        .into_iter()
        .map(|t| (t.game.exe.to_lowercase(), t.game.name))
        .collect()
}

fn now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    format!("{seconds}")
}

#[tauri::command]
fn export_profile(
    app: tauri::AppHandle,
    state: tauri::State<'_, App>,
    prefs: bool,
    account: bool,
    games: Vec<String>,
) -> Result<String, String> {
    let names = table_names(&state);
    // only the name. the words are typed on the far side, so the file is safe
    // to keep in a drive somebody else can read
    let who = account.then(|| shared::me().map(|w| w.name)).flatten();

    let made = {
        let settings = state.settings.lock().unwrap();
        profile::build(&settings, &games, prefs, who, &names, now())
    };

    if made.prefs.is_none() && made.games.is_empty() && made.account.is_none() {
        return Err("nothing was picked, so there is nothing to save".into());
    }

    let Some(path) = dialog::save(&dialog::Ask {
        owner: owner_window(&app),
        title: "Save your Freeplay profile",
        kinds: dialog::PROFILES,
        suggested: "freeplay-profile.freeplay",
        extension: "freeplay",
    }) else {
        return Err(String::new());
    };

    let text = serde_json::to_string_pretty(&made).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("could not write it: {e}"))?;

    let count = made.games.len();
    Ok(format!(
        "Saved {count} {} to {}",
        if count == 1 { "game" } else { "games" },
        path.display()
    ))
}

#[derive(Serialize)]
struct Peek {
    games: usize,
    prefs: bool,
    account: Option<String>,
    tables: usize,
}

// read it and describe it, but change nothing until the answer comes back. a
// path arrives when one was dropped on the window, otherwise ask for it
#[tauri::command]
fn open_profile(
    app: tauri::AppHandle,
    state: tauri::State<'_, App>,
    path: Option<String>,
) -> Result<Peek, String> {
    let picked = match path {
        Some(given) => Some(PathBuf::from(given)),
        None => dialog::open(&dialog::Ask {
            owner: owner_window(&app),
            title: "Open a Freeplay profile",
            kinds: dialog::PROFILES,
            suggested: "",
            extension: "freeplay",
        }),
    };
    let Some(path) = picked else {
        return Err(String::new());
    };

    let text = std::fs::read_to_string(&path).map_err(|e| format!("could not read it: {e}"))?;
    let found = profile::parse(&text)?;

    let peek = Peek {
        games: found.games.len(),
        prefs: found.prefs.is_some(),
        account: found.account.clone(),
        tables: found.games.iter().filter(|g| g.table.is_some()).count(),
    };
    *state.pending.lock().unwrap() = Some(found);
    Ok(peek)
}

#[tauri::command]
async fn apply_profile(
    state: tauri::State<'_, App>,
    phrase: Option<String>,
) -> Result<String, String> {
    let found = state
        .pending
        .lock()
        .unwrap()
        .take()
        .ok_or("open a profile first")?;

    let applied = {
        let mut settings = state.settings.lock().unwrap();
        let applied = profile::apply(&found, &mut settings);
        settings::save(&settings)?;
        applied
    };

    let mut notes = Vec::new();
    if applied.prefs {
        notes.push("preferences".to_string());
    }
    if applied.games > 0 {
        notes.push(format!(
            "{} {}",
            applied.games,
            if applied.games == 1 { "game" } else { "games" }
        ));
    }

    // the words rebuild the key, so a stolen profile is still only a list of
    // games
    match (found.account.as_deref(), phrase.as_deref()) {
        (Some(name), Some(words)) if !words.trim().is_empty() => {
            match shared::recover(name, words) {
                Ok(name) => notes.push(format!("signed in as {name}")),
                Err(e) => notes.push(format!("but the account did not come back: {e}")),
            }
        }
        (Some(_), _) => notes.push("account skipped".to_string()),
        _ => {}
    }

    let install_id = state.settings.lock().unwrap().install_id.clone();
    let mut pulled = 0usize;
    for id in applied.tables {
        match shared::install(id, &install_id, &synced_dir(), None) {
            Ok(_) => pulled += 1,
            Err(e) => tracing::warn!("could not fetch table {id}: {e}"),
        }
    }
    if pulled > 0 {
        notes.push(format!(
            "{pulled} {} downloaded",
            if pulled == 1 { "table" } else { "tables" }
        ));
    }

    forget_tables(&state);
    Ok(if notes.is_empty() {
        "Nothing in that file to import".to_string()
    } else {
        format!("Imported {}", notes.join(", "))
    })
}

// the recovery words, saved where the user says. copying them out of a box and
// into a text file by hand is how people end up not writing them down at all
#[tauri::command]
fn save_phrase(app: tauri::AppHandle, name: String, phrase: String) -> Result<String, String> {
    let Some(path) = dialog::save(&dialog::Ask {
        owner: owner_window(&app),
        title: "Save your recovery words",
        kinds: dialog::TEXT,
        suggested: &format!("freeplay-{}-recovery.txt", name.to_lowercase()),
        extension: "txt",
    }) else {
        return Err(String::new());
    };

    let text = format!(
        "Freeplay recovery words for {name}\r\n\r\n{phrase}\r\n\r\n\
         These words are the account. Anybody who has them can publish as {name},\r\n\
         and without them the name cannot be got back. There is no reset.\r\n"
    );
    std::fs::write(&path, text).map_err(|e| format!("could not write it: {e}"))?;
    Ok(format!("Saved to {}", path.display()))
}

// the file picker for a .CT, for anybody who does not think to drag one in
#[tauri::command]
fn pick_table(
    app: tauri::AppHandle,
    state: tauri::State<'_, App>,
    exe: Option<String>,
) -> Result<String, String> {
    let Some(path) = dialog::open(&dialog::Ask {
        owner: owner_window(&app),
        title: "Open a Cheat Engine table",
        kinds: dialog::TABLES,
        suggested: "",
        extension: "CT",
    }) else {
        return Err(String::new());
    };
    import_table(state, path.display().to_string(), exe)
}

/* ---------- the overlay ---------- */

#[derive(Serialize)]
struct OverlayState {
    on: bool,
    key: String,
    showing: bool,
    // whatever well known program already uses that combination
    clash: Option<String>,
    // the game the overlay would be about, if any
    game: Option<String>,
    // the overlay is its own window with its own document, so it has to be
    // told. it stayed on the default amber while the app was blue
    accent: String,
}

fn overlay_state(app: &tauri::AppHandle, state: &tauri::State<'_, App>) -> OverlayState {
    let settings = state.settings.lock().unwrap();
    OverlayState {
        on: settings.overlay,
        clash: hotkey::clash(&settings.overlay_key).map(str::to_string),
        key: settings.overlay_key.clone(),
        accent: settings.accent.clone(),
        showing: overlay::showing(app),
        game: state
            .target
            .lock()
            .unwrap()
            .as_ref()
            .map(|t| t.name().to_string()),
    }
}

#[tauri::command]
fn overlay_status(app: tauri::AppHandle, state: tauri::State<'_, App>) -> OverlayState {
    overlay_state(&app, &state)
}

#[tauri::command]
fn set_overlay(
    app: tauri::AppHandle,
    state: tauri::State<'_, App>,
    on: Option<bool>,
    key: Option<String>,
) -> Result<OverlayState, String> {
    if let Some(text) = &key {
        // refuse it here rather than finding out at the next launch that
        // nothing opens the overlay any more
        hotkey::parse(text)?;
    }

    {
        let mut settings = state.settings.lock().unwrap();
        if let Some(on) = on {
            settings.overlay = on;
        }
        if let Some(text) = key {
            settings.overlay_key = hotkey::spell(hotkey::parse(&text)?);
        }
        settings::save(&settings)?;
    }

    /* the window has to exist before the first press, not because of the
    delay but because building one steals the foreground. press the key in a
    game and windows would hand focus to whatever was behind it, drop the game
    out of fullscreen, and then the check would refuse because the game was no
    longer in front. only startup was building it, and only if the setting was
    already on. */
    if state.settings.lock().unwrap().overlay {
        overlay::prepare(&app)?;
    } else {
        overlay::hide(&app);
    }
    rebind_hotkey(&app)?;
    Ok(overlay_state(&app, &state))
}

#[tauri::command]
fn toggle_overlay(app: tauri::AppHandle, state: tauri::State<'_, App>) -> Result<bool, String> {
    overlay::toggle(&app, overlay_pid(&state))
}

// the pid only counts if there is a session on it, which means a table was
// found and loaded. an overlay with nothing to switch on is a floating box
fn overlay_pid(state: &tauri::State<'_, App>) -> Option<u32> {
    let pid = state.target.lock().unwrap().as_ref().map(|t| t.pid())?;
    state.session.lock().unwrap().as_ref().map(|_| pid)
}

#[tauri::command]
fn hide_overlay(app: tauri::AppHandle) {
    overlay::hide(&app);
}

// what the overlay is looking at, so it can draw something before anything is
// attached rather than a blank panel
#[tauri::command]
fn overlay_game(state: tauri::State<'_, App>) -> Option<Attached> {
    let guard = state.target.lock().unwrap();
    let target = guard.as_ref()?;
    let exe = target.name().to_string();
    let session = state.session.lock().unwrap();
    let table = session.as_ref().filter(|s| s.table().matches_process(&exe));

    Some(Attached {
        game: table
            .map(|s| s.table().game.name.clone())
            .unwrap_or_else(|| exe.clone()),
        table: table.is_some(),
        arch: target.arch().label().to_string(),
        pid: target.pid(),
        process: exe,
    })
}

// dropped and made again whenever the key or the on switch changes. holding
// the registration open while it is turned off would keep the combination
// away from whatever else wants it
fn rebind_hotkey(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<App>();
    let (wanted, text) = {
        let settings = state.settings.lock().unwrap();
        (settings.overlay, settings.overlay_key.clone())
    };

    *state.hotkey.lock().unwrap() = None;
    if !wanted {
        return Ok(());
    }

    let key = hotkey::parse(&text)?;
    let (tell, heard) = std::sync::mpsc::channel();
    let listener =
        hotkey::listen(key, tell).map_err(|e| format!("{text} could not be set: {e}"))?;
    *state.hotkey.lock().unwrap() = Some(listener);

    let handle = app.clone();
    std::thread::spawn(move || {
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);

        // ends on its own when the listener is dropped and the sender goes
        while heard.recv().is_ok() {
            // one press must never be two toggles. a global key hook is
            // exactly the sort of thing that delivers a stray second event,
            // and show then hide looks identical to the overlay being broken
            if last.elapsed() < std::time::Duration::from_millis(300) {
                continue;
            }
            last = std::time::Instant::now();

            let app = handle.clone();
            // windows are the main thread's business, and this is not it
            let _ = handle.run_on_main_thread(move || {
                let state = app.state::<App>();
                let pid = overlay_pid(&state);
                match overlay::toggle(&app, pid) {
                    Ok(shown) => tracing::debug!("overlay {}", if shown { "up" } else { "down" }),
                    Err(e) => {
                        tracing::info!("overlay: {e}");
                        let _ = app.emit("overlay-refused", e);
                    }
                }
            });
        }
    });
    Ok(())
}

// what one slot in the bank does when its key lands
#[derive(Clone)]
enum Strike {
    Cheat {
        id: String,
        does: freeplay_table::schema::Tap,
        value: Option<String>,
    },
    // everything off at once, for when the game starts misbehaving mid fight
    Panic,
}

#[derive(Clone, Serialize)]
struct Fired {
    exe: String,
    id: String,
    on: bool,
    panic: bool,
}

// every key that should work right now, in slot order. a rebind replaces all
// of a cheat's own keys, "" silences them
fn keyed_binds(settings: &Settings, exe: &str, table: &Table) -> Vec<(hotkey::Hotkey, Strike)> {
    let bound = settings.keys.get(&exe.to_lowercase());
    let mut out = Vec::new();
    for cheat in &table.cheats {
        match bound.and_then(|m| m.get(&cheat.id)) {
            Some(text) if text.is_empty() => {}
            Some(text) => {
                if let Ok(key) = hotkey::parse_loose(text) {
                    out.push((
                        key,
                        Strike::Cheat {
                            id: cheat.id.clone(),
                            does: freeplay_table::schema::Tap::Toggle,
                            value: None,
                        },
                    ));
                }
            }
            None => {
                for held in &cheat.hotkeys {
                    if let Some(key) = hotkey::from_vks(&held.keys) {
                        out.push((
                            key,
                            Strike::Cheat {
                                id: cheat.id.clone(),
                                does: held.does,
                                value: held.value.clone(),
                            },
                        ));
                    }
                }
            }
        }
    }
    if !settings.panic.is_empty() {
        if let Ok(key) = hotkey::parse_loose(&settings.panic) {
            out.push((key, Strike::Panic));
        }
    }
    out
}

// the table's keys, watched while the game is. dropped and rebuilt whenever
// the table, a bind or the game changes
fn arm_keys(state: &tauri::State<'_, App>) {
    *state.bank.lock().unwrap() = None;

    let Some(handle) = state.handle.get().cloned() else {
        return;
    };
    let (exe, pid) = {
        let held = state.target.lock().unwrap();
        match held.as_ref() {
            Some(t) => (t.name().to_lowercase(), t.pid()),
            None => return,
        }
    };
    let binds = {
        let held = state.session.lock().unwrap();
        let Some(session) = held.as_ref() else { return };
        let settings = state.settings.lock().unwrap();
        keyed_binds(&settings, &exe, session.table())
    };
    if binds.is_empty() {
        return;
    }

    let keys: Vec<hotkey::Hotkey> = binds.iter().map(|(k, _)| *k).collect();
    let strikes: Vec<Strike> = binds.into_iter().map(|(_, s)| s).collect();
    let (tell, heard) = std::sync::mpsc::channel();
    match hotkey::bank(&keys, pid, tell) {
        Ok(bank) => *state.bank.lock().unwrap() = Some(bank),
        Err(e) => {
            tracing::warn!("cheat keys: {e}");
            return;
        }
    }
    tracing::info!("{} cheat keys armed for {exe}", keys.len());

    // ends on its own when the bank is dropped and the sender goes with it
    std::thread::spawn(move || {
        let mut last: HashMap<usize, std::time::Instant> = HashMap::new();
        while let Ok(slot) = heard.recv() {
            // a hook can deliver a stray second event, and one press must
            // never be two toggles
            if last
                .get(&slot)
                .is_some_and(|t| t.elapsed().as_millis() < 250)
            {
                continue;
            }
            last.insert(slot, std::time::Instant::now());
            if let Some(strike) = strikes.get(slot).cloned() {
                strike_home(&handle, strike);
            }
        }
    });
}

fn strike_home(handle: &tauri::AppHandle, strike: Strike) {
    use freeplay_table::schema::Tap;

    let state = handle.state::<App>();
    let exe = {
        let held = state.target.lock().unwrap();
        match held.as_ref() {
            Some(t) => t.name().to_lowercase(),
            None => return,
        }
    };

    let fired = match strike {
        Strike::Panic => {
            if let Some(session) = state.session.lock().unwrap().as_ref() {
                session.disarm_all();
            }
            remember_armed(&state, &exe, Vec::new());
            tracing::info!("panic key, everything off");
            Fired {
                exe,
                id: String::new(),
                on: false,
                panic: true,
            }
        }
        Strike::Cheat { id, does, value } => {
            let on = match does {
                Tap::Toggle => {
                    let armed = state
                        .session
                        .lock()
                        .unwrap()
                        .as_ref()
                        .is_some_and(|s| s.is_armed(&id));
                    !armed
                }
                Tap::On => true,
                Tap::Off => false,
                Tap::Set => {
                    let text = value.unwrap_or_default();
                    match set_cheat_value(state.clone(), exe.clone(), id.clone(), text) {
                        Ok(_) => {
                            chirp(&state, false);
                            let _ = handle.emit(
                                "keys-fired",
                                Fired {
                                    exe,
                                    id,
                                    on: true,
                                    panic: false,
                                },
                            );
                        }
                        Err(e) => tracing::info!("{id} by key: {e}"),
                    }
                    return;
                }
            };
            if let Err(e) = set_cheat(state.clone(), exe.clone(), id.clone(), on) {
                tracing::info!("{id} by key: {e}");
                return;
            }
            Fired {
                exe,
                id,
                on,
                panic: false,
            }
        }
    };

    chirp(&state, fired.panic);
    let _ = handle.emit("keys-fired", fired);
}

// the classic trainer chirp, so you hear the key land without alt tabbing
fn chirp(state: &tauri::State<'_, App>, grim: bool) {
    if !state.settings.lock().unwrap().chirp {
        return;
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows::Win32::UI::WindowsAndMessaging::{MB_ICONHAND, MB_OK};
        let _ = MessageBeep(if grim { MB_ICONHAND } else { MB_OK });
    }
    #[cfg(not(windows))]
    let _ = grim;
}

// what the chip on the card says. a rebind wins, then the table's own first
// key, then nothing
fn shown_key(
    bound: Option<&HashMap<String, String>>,
    cheat: &freeplay_table::schema::Cheat,
) -> String {
    if let Some(text) = bound.and_then(|m| m.get(&cheat.id)) {
        return match hotkey::parse_loose(text) {
            Ok(key) => hotkey::spell(key),
            Err(_) => String::new(),
        };
    }
    cheat
        .hotkeys
        .iter()
        .find_map(|held| hotkey::from_vks(&held.keys))
        .map(hotkey::spell)
        .unwrap_or_default()
}

// "" switches a cheat's key off, no key at all hands it back to the table
#[tauri::command]
fn bind_key(
    state: tauri::State<'_, App>,
    exe: String,
    id: String,
    key: Option<String>,
) -> Result<String, String> {
    let stem = exe.to_lowercase();
    {
        let mut settings = state.settings.lock().unwrap();
        match key.as_deref().map(str::trim) {
            None => {
                if let Some(bound) = settings.keys.get_mut(&stem) {
                    bound.remove(&id);
                    if bound.is_empty() {
                        settings.keys.remove(&stem);
                    }
                }
            }
            Some("") => {
                settings
                    .keys
                    .entry(stem.clone())
                    .or_default()
                    .insert(id.clone(), String::new());
            }
            Some(text) => {
                let parsed = hotkey::parse_loose(text)?;
                if let Some(who) = hotkey::clash(text) {
                    return Err(format!(
                        "{} already belongs to {who}",
                        hotkey::spell(parsed)
                    ));
                }
                settings
                    .keys
                    .entry(stem.clone())
                    .or_default()
                    .insert(id.clone(), hotkey::spell(parsed));
            }
        }
        settings::save(&settings)?;
    }

    arm_keys(&state);

    let bound = state.settings.lock().unwrap().keys.get(&stem).cloned();
    Ok(with_tables(&exe, &off_for(&state, &exe))
        .and_then(|t| {
            t.cheats
                .iter()
                .find(|c| c.id == id)
                .map(|c| shown_key(bound.as_ref(), c))
        })
        .unwrap_or_default())
}

// the about page had a blank line where this was meant to go
#[tauri::command]
fn version() -> String {
    format!(
        "Version {} for {}",
        env!("CARGO_PKG_VERSION"),
        if cfg!(target_pointer_width = "64") {
            "64-bit Windows"
        } else {
            "32-bit Windows"
        }
    )
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("that is not a link".into());
    }
    freeplay_library::launch::show_url(&url)
}

// opens the install folder
#[tauri::command]
fn open_folder(dir: String) -> Result<(), String> {
    let path = PathBuf::from(dir);
    if !path.is_dir() {
        return Err("that folder is not there any more".into());
    }
    freeplay_library::launch::show(&path)
}

// opens a search in the browser. we make no request ourselves, just hand a url
// to the shell like a steam link. the tables belong to whoever wrote them, so
// point at them instead of shipping copies
#[tauri::command]
fn find_table(name: String) -> Result<(), String> {
    freeplay_library::launch::show_url(&community::search_url(&name))
}

// fetch published tables. also runs once at start unless turned off
#[tauri::command]
async fn update_tables(state: tauri::State<'_, App>) -> Result<String, String> {
    let report = freeplay_sync::update(&synced_dir())?;
    if report.changed() {
        forget_tables(&state);
    }
    Ok(report.summary())
}

// what is on disk, without going near the network. the check on start already
// runs on its own thread, and the settings page asking for a second one meant
// two requests every launch
#[tauri::command]
fn table_count(state: tauri::State<'_, App>) -> String {
    let held = tables(&state);
    match held.len() {
        0 => "None yet".into(),
        1 => "1 table".into(),
        n => format!("{n} tables"),
    }
}

#[tauri::command]
fn open_log() -> Result<(), String> {
    let file = log::path();
    if !file.is_file() {
        return Err("there is no log file yet".into());
    }
    freeplay_library::launch::show(&file)
}

// only the things the front end actually owns. it sends back a whole settings
// object built from whatever it last read, and everything armed, typed or
// downloaded since then lives in the same struct, so taking it wholesale threw
// all of that away the first time anybody clicked a theme swatch
#[tauri::command]
fn save_settings(state: tauri::State<'_, App>, next: Settings) -> Result<Settings, String> {
    let mut held = state.settings.lock().unwrap();

    held.theme = next.theme;
    held.accent = next.accent;
    held.favourites = next.favourites;
    held.auto_update = next.auto_update;
    held.community = next.community;
    held.auto_attach = next.auto_attach;
    held.shared_open = next.shared_open;
    let repanic = held.panic != next.panic;
    held.panic = next.panic;
    held.chirp = next.chirp;

    held.tidy();
    settings::save(&held)?;
    let done = held.clone();
    drop(held);

    // the bank holds the old panic key until it is remade
    if repanic {
        arm_keys(&state);
    }
    Ok(done)
}

// windows asks for two icons, a small one for the title bar and a big one for
// the taskbar and alt-tab. tauri only sets the small one, so windows stretched
// 16px up to 48 and it looked soft. build both at the size it actually asks
// for, which also picks up display scaling
#[cfg(windows)]
fn set_window_icons(window: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconFromResourceEx, GetSystemMetrics, SendMessageW, ICON_BIG, ICON_SMALL,
        LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, WM_SETICON,
    };

    const PNG: &[u8] = include_bytes!("../icons/256x256.png");
    // marks the buffer as a 3.0 icon resource, which is what lets it be a png
    const ICON_RESOURCE_V3: u32 = 0x0003_0000;

    let Ok(hwnd) = window.hwnd() else { return };

    for (which, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
        let size = unsafe { GetSystemMetrics(metric) };
        let icon = unsafe {
            CreateIconFromResourceEx(PNG, true, ICON_RESOURCE_V3, size, size, LR_DEFAULTCOLOR)
        };
        if let Ok(icon) = icon {
            unsafe {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(which as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
    }
}

#[cfg(not(windows))]
fn set_window_icons(_window: &tauri::WebviewWindow) {}

// the game is already up, so bring it back rather than starting a second copy
#[tauri::command]
fn focus_game(exe: String) -> Result<(), String> {
    let wanted = exe.to_lowercase();
    let pid = processes()
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.name.to_lowercase() == wanted)
        .map(|p| p.pid)
        .ok_or("that game is not running any more")?;

    if overlay::focus_game(pid) {
        Ok(())
    } else {
        Err("windows would not bring that window forward".into())
    }
}

#[tauri::command]
async fn launch_game(state: tauri::State<'_, App>, key: String) -> Result<(), String> {
    let game = library(&state, false)
        .into_iter()
        .find(|g| key_for(g) == key)
        .ok_or_else(|| {
            tracing::warn!("launch asked for {key}, which is not in the library");
            "that game is not in the library any more".to_string()
        })?;

    match freeplay_library::launch::start(&game) {
        Ok(what) => {
            tracing::info!("started {} via {what:?}", game.name);
            Ok(())
        }
        Err(e) => {
            tracing::error!("could not start {}: {e}", game.name);
            Err(e)
        }
    }
}

// art used to go over as base64 in the reply, which pushed megabytes of string
// through the bridge and made the webview decode the same picture on every
// redraw. as bytes it gets cached and decoded once like any other image
fn art_url(app_id: &str, kind: &str) -> String {
    if cfg!(windows) {
        format!("http://art.localhost/{app_id}/{kind}")
    } else {
        format!("art://localhost/{app_id}/{kind}")
    }
}

fn urls_for(app_id: &str, found: &freeplay_library::art::Art) -> ArtUrls {
    let url = |present: bool, kind: &str| present.then(|| art_url(app_id, kind));
    ArtUrls {
        cover: url(found.cover.is_some(), "cover"),
        hero: url(found.hero.is_some(), "hero"),
        logo: url(found.logo.is_some(), "logo"),
    }
}

// steam and gog art is already on the disk, epic's is a url in a catalog file
// and has to be fetched once. kept next to the settings so it survives a
// restart and nothing is downloaded twice
fn art_cache() -> PathBuf {
    settings::path()
        .parent()
        .unwrap_or(Path::new("."))
        .join("art")
}

fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xff, 0xd8, 0xff, ..] => Some("jpg"),
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        _ => None,
    }
}

fn epic_art(state: &tauri::State<'_, App>, app_id: &str) -> freeplay_library::art::Art {
    // the id is three hex words and two colons, and it is about to be a
    // filename
    let stem: String = app_id.replace(':', "-");
    if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Default::default();
    }

    let dir = art_cache();
    let saved = |kind: &str| {
        ["jpg", "png", "webp"]
            .iter()
            .map(|ext| dir.join(format!("{stem}-{kind}.{ext}")))
            .find(|file| file.is_file())
    };

    let mut art = freeplay_library::art::Art {
        cover: saved("cover"),
        hero: saved("hero"),
        logo: None,
    };
    if art.cover.is_some() && art.hero.is_some() {
        return art;
    }
    // turning the community off turns off every outbound call, this one too
    if online(state).is_err() {
        return art;
    }

    let remote = freeplay_library::art::epic(app_id);
    for (kind, url) in [("cover", remote.cover), ("hero", remote.hero)] {
        let (Some(url), true) = (url, saved(kind).is_none()) else {
            continue;
        };
        let Ok(bytes) = freeplay_sync::http::get(&url) else {
            continue;
        };
        // a cdn that answers with an error page is not a picture, and the
        // extension has to match what came back or the webview will not draw it
        let Some(ext) = image_kind(&bytes) else {
            continue;
        };
        let file = dir.join(format!("{stem}-{kind}.{ext}"));
        if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&file, &bytes).is_ok() {
            match kind {
                "cover" => art.cover = Some(file),
                _ => art.hero = Some(file),
            }
        }
    }
    art
}

#[tauri::command]
async fn game_art(state: tauri::State<'_, App>, app_id: String) -> Result<ArtUrls, ()> {
    if let Some(cached) = state.art.lock().unwrap().get(&app_id) {
        return Ok(urls_for(&app_id, cached));
    }

    // steam art needs the id, gog art needs the store and the folder too, so
    // go back to the game rather than guessing from the id alone
    let game = library(&state, false)
        .into_iter()
        .find(|g| g.app_id.as_deref() == Some(app_id.as_str()));

    let found = match &game {
        Some(g) if g.store == Store::Epic => epic_art(&state, &app_id),
        Some(g) => freeplay_library::art::find(g),
        None => Default::default(),
    };

    let urls = urls_for(&app_id, &found);
    state.art.lock().unwrap().insert(app_id, found);
    Ok(urls)
}

// serves what the store already cached. path is /<appid>/<kind>, and the
// pictures themselves were located when the page asked for them
fn serve_art(
    path: &str,
    found: Option<freeplay_library::art::Art>,
) -> tauri::http::Response<Vec<u8>> {
    let deny = || {
        tauri::http::Response::builder()
            .status(404)
            .body(Vec::new())
            .unwrap()
    };

    let mut parts = path.trim_matches('/').split('/');
    let (Some(app_id), Some(kind)) = (parts.next(), parts.next()) else {
        return deny();
    };
    // the app id goes into a path, so it had better be one. steam and gog are
    // numbers, epic is three hex words with colons between them
    let ordinary = |b: u8| b.is_ascii_alphanumeric() || b == b':';
    if app_id.is_empty() || !app_id.bytes().all(ordinary) {
        return deny();
    }

    let Some(found) = found else { return deny() };
    let file = match kind {
        "cover" => found.cover,
        "hero" => found.hero,
        "logo" => found.logo,
        _ => None,
    };

    let Some(file) = file else { return deny() };
    let Ok(bytes) = std::fs::read(&file) else {
        return deny();
    };
    let mime = match file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        // galaxy caches everything as webp, and the offline installer leaves
        // an ico. served as jpeg both render as a broken picture
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        _ => "image/jpeg",
    };

    tauri::http::Response::builder()
        .header("Content-Type", mime)
        .header("Cache-Control", "max-age=86400")
        .body(bytes)
        .unwrap()
}

#[tauri::command]
fn list_processes() -> Vec<ProcessRow> {
    let mut all: Vec<ProcessRow> = processes()
        .unwrap_or_default()
        .into_iter()
        .map(|p| ProcessRow {
            pid: p.pid,
            name: p.name,
        })
        .collect();
    all.sort_by_key(|a| a.name.to_lowercase());
    all
}

fn friendly(error: CoreError) -> String {
    match error {
        CoreError::ProcessNotFound(name) => format!("{name} is not running"),
        CoreError::Protected { process, guard } => format!(
            "{process} is running {guard}. Freeplay only works on single player games, \
             and attaching would risk your account."
        ),
        CoreError::OpenFailed { .. } => {
            "Could not open the game. Try running Freeplay as administrator.".to_string()
        }
        other => other.to_string(),
    }
}

#[tauri::command]
fn attach(
    app: tauri::AppHandle,
    state: tauri::State<'_, App>,
    exe: String,
) -> Result<Attached, String> {
    tear_down(&state);
    *state.declined.lock().unwrap() = None;

    let target = WindowsTarget::attach_by_name(&exe).map_err(friendly)?;
    let pid = target.pid();
    let arch = target.arch().label().to_string();
    let shared: Arc<dyn Target> = Arc::new(target);

    let table = with_tables(&exe, &off_for(&state, &exe));
    let has_table = table.is_some();
    let name = table
        .as_ref()
        .map(|t| t.game.name.clone())
        .unwrap_or_else(|| exe.clone());

    // taken now, because "use table" can change what is installed while this
    // one is still the one running
    *state.playing.lock().unwrap() = table.as_ref().and_then(|table| {
        let settings = state.settings.lock().unwrap();
        settings.grabbed.get(&exe.to_lowercase()).map(|id| Playing {
            id: *id,
            exe: exe.to_lowercase(),
            game: table.game.name.clone(),
            by: table.meta.submitted_by.clone(),
            started: std::time::Instant::now(),
        })
    });

    if let Some(table) = table {
        let mut session = Session::new(Arc::clone(&shared), table);
        session.start();
        // numbers before arming, or the first write goes out with the old one
        for (id, text) in values_for(&state, &exe) {
            if let Err(e) = session.choose(&id, &text) {
                tracing::warn!("dropping the saved value for {id}: {e}");
            }
        }
        session.arm_all(&armed_for(&state, &exe));
        relight(&state, &session);
        *state.session.lock().unwrap() = Some(session);
    }
    *state.target.lock().unwrap() = Some(shared);
    *state.search.lock().unwrap() = None;
    arm_keys(&state);

    // low level keyboard hooks are called newest first, and a game that eats
    // the windows key has one of its own. ours has to go on after theirs or
    // the shortcut never reaches us
    if state.settings.lock().unwrap().overlay {
        if let Err(e) = rebind_hotkey(&app) {
            tracing::warn!("overlay hotkey: {e}");
        }
    }

    Ok(Attached {
        process: exe,
        pid,
        game: name,
        table: has_table,
        arch,
    })
}

#[tauri::command]
fn detach(state: tauri::State<'_, App>) {
    let letting_go = state
        .target
        .lock()
        .unwrap()
        .as_ref()
        .map(|t| t.name().to_lowercase());
    *state.declined.lock().unwrap() = letting_go;
    tear_down(&state);
}

fn tear_down(state: &tauri::State<'_, App>) {
    // keys first, or one could land between the session going and the target
    *state.bank.lock().unwrap() = None;
    if let Some(exe) = state
        .target
        .lock()
        .unwrap()
        .as_ref()
        .map(|t| t.name().to_lowercase())
    {
        // a quiet sitting is what takes an old blame away
        write_verdicts(state, &exe, false);
    }
    if let Some(mut session) = state.session.lock().unwrap().take() {
        remember_the_sitting(state, &session);
        session.stop();
        session.disable_all();
    }
    *state.target.lock().unwrap() = None;
    *state.search.lock().unwrap() = None;
    state.lit.lock().unwrap().clear();
}

// how long each armed cheat has been on, for the blame below
fn ages_of(state: &tauri::State<'_, App>) -> Vec<(String, u64)> {
    let armed = state
        .session
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.armed())
        .unwrap_or_default();
    let lit = state.lit.lock().unwrap();
    armed
        .into_iter()
        .filter_map(|id| lit.get(&id).map(|t| (id, t.elapsed().as_secs())))
        .collect()
}

// switched on moments ago and the game died: suspect. on for ages without
// incident: let off. the stretch in between says nothing either way
const RECENT: u64 = 25;
const PROVEN: u64 = 90;

fn verdicts(ages: &[(String, u64)]) -> (Vec<String>, Vec<String>) {
    let mut blamed = Vec::new();
    let mut proven = Vec::new();
    for (id, seconds_on) in ages {
        if *seconds_on <= RECENT {
            blamed.push(id.clone());
        } else if *seconds_on >= PROVEN {
            proven.push(id.clone());
        }
    }
    (blamed, proven)
}

// died is true when the game went down on its own rather than being let go
fn write_verdicts(state: &tauri::State<'_, App>, exe: &str, died: bool) {
    let (blamed, proven) = verdicts(&ages_of(state));
    if (!died || blamed.is_empty()) && proven.is_empty() {
        return;
    }

    let mut settings = state.settings.lock().unwrap();
    let held = settings.crashed.entry(exe.to_string()).or_default();
    if died {
        for id in blamed {
            tracing::info!("{exe} went down moments after {id} was switched on");
            held.insert(id, now_seconds());
        }
    }
    for id in proven {
        held.remove(&id);
    }
    if held.is_empty() {
        settings.crashed.remove(exe);
    }
    let _ = settings::save(&settings);
}

// a session holds the table it was built with, so swapping tables on a game
// that is already attached changed nothing on screen until you detached and
// came back. rebuilt here against the same process
fn reseat(state: &tauri::State<'_, App>, exe: &str) {
    let wanted = exe.to_lowercase();
    let target = {
        let held = state.target.lock().unwrap();
        match held.as_ref() {
            Some(t) if t.name().to_lowercase() == wanted => Arc::clone(t),
            _ => return,
        }
    };

    // whatever is on belongs to the table on its way out, and nothing in the
    // new one knows how to put those bytes back
    if let Some(mut old) = state.session.lock().unwrap().take() {
        remember_the_sitting(state, &old);
        old.stop();
        old.disable_all();
    }
    // the old table's keys must not outlive it
    *state.bank.lock().unwrap() = None;

    let Some(table) = with_tables(exe, &off_for(state, exe)) else {
        return;
    };
    *state.playing.lock().unwrap() = {
        let settings = state.settings.lock().unwrap();
        settings.grabbed.get(&wanted).map(|id| Playing {
            id: *id,
            exe: wanted.clone(),
            game: table.game.name.clone(),
            by: table.meta.submitted_by.clone(),
            started: std::time::Instant::now(),
        })
    };

    let mut session = Session::new(target, table);
    session.start();
    for (id, text) in values_for(state, exe) {
        if let Err(e) = session.choose(&id, &text) {
            tracing::warn!("dropping the saved value for {id}: {e}");
        }
    }
    session.arm_all(&armed_for(state, exe));
    relight(state, &session);
    *state.session.lock().unwrap() = Some(session);
    arm_keys(state);
    tracing::info!("reseated {exe} on the table that just landed");
}

/* cheats brought back by arm_all count as switched on now. without this a
restored cheat had no timeline: it could never clear an old blame however
long it behaved, and a crash seconds into a sitting blamed nobody */
fn relight(state: &tauri::State<'_, App>, session: &Session) {
    let now = std::time::Instant::now();
    let mut lit = state.lit.lock().unwrap();
    lit.clear();
    for id in session.armed() {
        lit.insert(id, now);
    }
}

// everything except the four fields that need a live process, so the same
// shape comes back whether the game is running or not
fn cheat_row(cheat: &freeplay_table::schema::Cheat, typed: Option<&String>) -> CheatRow {
    CheatRow {
        id: cheat.id.clone(),
        name: cheat.name.clone(),
        from: String::new(),
        category: cheat.category.label().to_string(),
        description: cheat.description.clone(),
        hint: cheat.hint.clone(),
        state: "idle".into(),
        reason: String::new(),
        armed: false,
        live: false,
        key: String::new(),
        suspect: false,
        does: cheat.action.label().to_string(),
        editable: cheat.action.takes_a_number(),
        kind: cheat
            .action
            .kind()
            .map(|k| k.name().to_string())
            .unwrap_or_default(),
        value: typed
            .cloned()
            .or_else(|| cheat.action.default_value().map(|v| v.to_string()))
            .unwrap_or_default(),
        current: String::new(),
        choices: cheat
            .action
            .choices()
            .iter()
            .map(|c| ChoiceRow {
                value: c.value.to_string(),
                label: c.label.clone(),
            })
            .collect(),
        hex: cheat.action.shows_hex(),
        holds: cheat.action.holds(),
    }
}

#[derive(Serialize, Default)]
struct Credit {
    // whoever worked the addresses out, which is almost never whoever uploaded
    // it here
    author: String,
    // the thread it was converted from, if the table says
    source: String,
    notes: String,
}

// the first https link in a free text field
fn link_in(text: &str) -> String {
    text.split_whitespace()
        .find(|word| word.starts_with("https://"))
        .map(|word| word.trim_end_matches(['.', ',', ')']).to_string())
        .unwrap_or_default()
}

#[tauri::command]
fn credit(state: tauri::State<'_, App>, exe: String) -> Credit {
    let held = state.session.lock().unwrap();
    let open = held.as_ref().map(|s| s.table().clone());
    drop(held);

    let _ = open;
    /* with two tables folded together the list is both authors' work, so
    naming only the first one takes the credit off somebody */
    let parts: Vec<Table> = with_parts(&state, &exe);
    if parts.is_empty() {
        return Credit::default();
    }

    let mut authors: Vec<String> = Vec::new();
    for table in &parts {
        let who = table.game.author.trim();
        if !who.is_empty() && !authors.iter().any(|a| a == who) {
            authors.push(who.to_string());
        }
    }

    Credit {
        author: match authors.len() {
            0 => String::new(),
            1 => authors.remove(0),
            _ => {
                let last = authors.pop().unwrap_or_default();
                format!("{} and {last}", authors.join(", "))
            }
        },
        source: link_in(&parts[0].game.notes),
        notes: parts[0].game.notes.clone(),
    }
}

#[tauri::command]
fn cheats(state: tauri::State<'_, App>, exe: String) -> Vec<CheatRow> {
    let typed = values_for(&state, &exe);
    let named = credits(&exe);
    let (bound, scars) = {
        let settings = state.settings.lock().unwrap();
        (
            settings.keys.get(&exe.to_lowercase()).cloned(),
            settings.crashed.get(&exe.to_lowercase()).cloned(),
        )
    };
    let guard = state.session.lock().unwrap();
    let attached = guard.as_ref().filter(|s| s.table().matches_process(&exe));

    if let Some(session) = attached {
        session.reconcile();
        let symbols = session.symbols();
        return session
            .table()
            .cheats
            .iter()
            .map(|cheat| {
                let live = session.is_on(&cheat.id);
                let (label, reason) = match session.state_of(cheat, &symbols) {
                    CheatState::Ready { .. } => ("ready", String::new()),
                    CheatState::Unavailable { reason } => ("wait", reason),
                    CheatState::Broken { reason } => ("broken", reason),
                };

                let mut row = cheat_row(cheat, typed.get(&cheat.id));
                row.key = shown_key(bound.as_ref(), cheat);
                row.suspect = scars.as_ref().is_some_and(|s| s.contains_key(&cheat.id));
                row.from = whose(&named, &cheat.id);
                row.state = if live { "on".into() } else { label.to_string() };
                row.reason = reason;
                row.armed = session.is_armed(&cheat.id);
                // ready, armed and still not on means we tried and it did not
                // take. saying so beats leaving it looking like it is waiting
                if !live && row.armed && row.state == "ready" {
                    if let Some(why) = session.why_not(&cheat.id) {
                        row.state = "broken".into();
                        row.reason = why;
                    }
                }
                row.live = live;
                if row.editable {
                    row.current = session
                        .live_value(&cheat.id)
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    if row.value.is_empty() {
                        row.value = row.current.clone();
                    }
                }
                row
            })
            .collect();
    }
    drop(guard);

    let armed = armed_for(&state, &exe);
    // the same fold the session would build. going through tables() here meant
    // the list with the game shut only ever showed the first table
    let Some(table) = with_tables(&exe, &off_for(&state, &exe)) else {
        return Vec::new();
    };

    table
        .cheats
        .iter()
        .map(|cheat| {
            let mut row = cheat_row(cheat, typed.get(&cheat.id));
            row.key = shown_key(bound.as_ref(), cheat);
            row.suspect = scars.as_ref().is_some_and(|s| s.contains_key(&cheat.id));
            row.from = whose(&named, &cheat.id);
            row.armed = armed.contains(&cheat.id);
            row
        })
        .collect()
}

fn values_for(state: &tauri::State<'_, App>, exe: &str) -> HashMap<String, String> {
    state
        .settings
        .lock()
        .unwrap()
        .values
        .get(&exe.to_lowercase())
        .cloned()
        .unwrap_or_default()
}

// numbers are kept whether or not the game is up, same as what is armed. type
#[derive(Serialize)]
struct TableRow {
    tag: String,
    name: String,
    author: String,
    cheats: usize,
    // switched on and folded into the list, or sitting there unused
    using: bool,
}

// every table installed for this game, and which ones are switched on
#[tauri::command]
fn installed_tables(state: tauri::State<'_, App>, exe: String) -> Vec<TableRow> {
    let off = off_for(&state, &exe);
    tables_for(&exe)
        .into_iter()
        .map(|(tag, table)| TableRow {
            using: !off.contains(&tag),
            tag,
            name: table.game.name.clone(),
            author: if table.game.author.is_empty() {
                table.meta.submitted_by.clone()
            } else {
                table.game.author.clone()
            },
            cheats: table.cheats.len(),
        })
        .collect()
}

/* switch one on or off. everything that was on from the table going away has
to come off with it, or those bytes stay patched with nothing left that
knows how to put them back */
#[tauri::command]
fn use_table(
    state: tauri::State<'_, App>,
    exe: String,
    tag: String,
    on: bool,
) -> Result<(), String> {
    {
        let mut settings = state.settings.lock().unwrap();
        let held = settings.off.entry(exe.to_lowercase()).or_default();

        match on {
            false if !held.contains(&tag) => held.push(tag.clone()),
            true => held.retain(|t| t != &tag),
            _ => {}
        }
        // nothing switched off is the ordinary case, so it carries no entry
        if held.is_empty() {
            settings.off.remove(&exe.to_lowercase());
        }
        settings::save(&settings)?;
    }
    forget_tables(&state);
    reseat(&state, &exe);
    Ok(())
}

#[derive(Serialize, Default)]
struct FitRow {
    // signatures the table looks for that are in this copy of the game
    found: usize,
    total: usize,
    missing: usize,
    // aobscan searches the whole process, so a miss in the exe means nothing
    unknown: usize,
    ambiguous: usize,
    // scripts that will assemble, patch and then crash, because they jump to
    // an address that belongs to the build the author had
    stale: Vec<String>,
    // no scans at all, so there is nothing to measure
    silent: bool,
    // the exe is wrapped, so nothing can be read off it and a miss means
    // nothing. steam's drm does this to plenty of games
    sealed: bool,
}

/* whether the table matches this copy of the game, read off the exe with the
game shut. cheaper and far more direct than recording which version a table
was written for and hoping somebody votes */
#[tauri::command]
fn table_fit(state: tauri::State<'_, App>, exe: String) -> FitRow {
    let wanted = exe.to_lowercase();
    let Some(game) = library(&state, false).into_iter().find(|game| {
        game.main_exe()
            .is_some_and(|found| found.to_lowercase() == wanted)
    }) else {
        return FitRow::default();
    };
    let Some(path) = game.executables.first() else {
        return FitRow::default();
    };
    let Some(code) = freeplay_library::pe::code(path) else {
        return FitRow::default();
    };
    if code.packed {
        return FitRow {
            sealed: true,
            ..Default::default()
        };
    }

    let mut whole = freeplay_aa::fit::Fit::default();
    let mut broken: Vec<String> = Vec::new();
    for table in with_parts(&state, &exe).iter() {
        for cheat in &table.cheats {
            let freeplay_table::schema::Action::Script { source } = &cheat.action else {
                continue;
            };
            let part = freeplay_aa::fit::of_script(
                source,
                &freeplay_aa::fit::Code {
                    bytes: &code.bytes,
                    rva: code.rva,
                },
            );
            for stale in &part.stale {
                broken.push(format!(
                    "{} sends the game to {:#x}, but on your build that code is at {:#x}",
                    cheat.name.trim(),
                    stale.wants,
                    stale.goes
                ));
            }
            whole.signatures.extend(part.signatures);
            whole.stale.extend(part.stale);
        }
    }

    FitRow {
        found: whole.found(),
        total: whole.signatures.len(),
        missing: whole.missing(),
        unknown: whole.unknown(),
        ambiguous: whole.ambiguous(),
        stale: broken,
        sealed: false,
        silent: whole.is_empty(),
    }
}

// which categories are folded away for a game. the overlay shows the same
// groups over the same game, so it reads and writes the same list
#[tauri::command]
fn folded(state: tauri::State<'_, App>, exe: String) -> Vec<String> {
    state
        .settings
        .lock()
        .unwrap()
        .folded
        .get(&exe.to_lowercase())
        .cloned()
        .unwrap_or_default()
}

#[tauri::command]
fn fold(
    state: tauri::State<'_, App>,
    exe: String,
    category: String,
    shut: bool,
) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    let held = settings.folded.entry(exe.to_lowercase()).or_default();

    match shut {
        true if !held.contains(&category) => held.push(category),
        false => held.retain(|c| c != &category),
        _ => {}
    }
    // open is the default, so a game with nothing shut carries no entry
    if held.is_empty() {
        settings.folded.remove(&exe.to_lowercase());
    }
    settings::save(&settings)
}

// one in with the game closed and it is waiting when you launch
#[tauri::command]
fn set_cheat_value(
    state: tauri::State<'_, App>,
    exe: String,
    id: String,
    value: String,
) -> Result<String, String> {
    let text = value.trim().to_string();

    {
        let guard = state.session.lock().unwrap();
        if let Some(session) = guard.as_ref().filter(|s| s.table().matches_process(&exe)) {
            session.choose(&id, &text).map_err(|e| e.to_string())?;
        } else {
            // no session to check it against, so check it here rather than
            // finding out it was rubbish the next time the game starts
            let table = tables(&state)
                .into_iter()
                .find(|t| t.matches_process(&exe))
                .ok_or("there is no table for that game")?;
            let cheat = table
                .cheats
                .iter()
                .find(|c| c.id == id)
                .ok_or_else(|| format!("no cheat called {id}"))?;
            let kind = cheat
                .action
                .kind()
                .ok_or_else(|| format!("{} does not take a number", cheat.name))?;
            kind.parse(&text)
                .ok_or_else(|| format!("{text:?} is not a {kind}"))?;
        }
    }

    let mut settings = state.settings.lock().unwrap();
    settings
        .values
        .entry(exe.to_lowercase())
        .or_default()
        .insert(id, text.clone());
    let _ = settings::save(&settings);
    Ok(text)
}

// queued up to ask about later. asking while the game is up means almost
// nobody ever sees the question
fn remember_the_sitting(state: &tauri::State<'_, App>, session: &Session) {
    let Some(playing) = state.playing.lock().unwrap().take() else {
        return;
    };
    if !session.used() {
        // they never switched anything on, so there is nothing to say
        return;
    }

    let seconds = playing.started.elapsed().as_secs();
    if seconds < settings::ENOUGH {
        return;
    }

    let mut settings = state.settings.lock().unwrap();
    if settings.rated.contains(&playing.id) {
        return;
    }
    settings.played.retain(|p| p.id != playing.id);
    settings.played.push(settings::Played {
        id: playing.id,
        exe: playing.exe,
        game: playing.game,
        by: playing.by,
        seconds,
        cheats: session.armed().len(),
        at: now_seconds(),
    });
    settings.tidy();
    let _ = settings::save(&settings);
    tracing::info!("queued a question about table {}", playing.id);
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

#[derive(Serialize)]
struct Question {
    id: i64,
    game: String,
    by: String,
    // "an hour and a half", already worded
    played: String,
    cheats: usize,
}

// the one waiting to be asked about, if it is time to ask
#[tauri::command]
fn pending_question(state: tauri::State<'_, App>) -> Option<Question> {
    let settings = state.settings.lock().unwrap();
    if now_seconds() < settings.ask_again_at {
        return None;
    }
    // the most recent sitting, which is the one they remember
    let played = settings.played.last()?;
    Some(Question {
        id: played.id,
        game: played.game.clone(),
        by: played.by.clone(),
        played: how_long(played.seconds),
        cheats: played.cheats,
    })
}

fn how_long(seconds: u64) -> String {
    let minutes = seconds / 60;
    match minutes {
        0..=1 => "a minute".into(),
        2..=90 => format!("{minutes} minutes"),
        _ => {
            let hours = (minutes as f64 / 60.0 * 10.0).round() / 10.0;
            if (hours - hours.round()).abs() < 0.05 {
                format!("{} hours", hours.round())
            } else {
                format!("{hours} hours")
            }
        }
    }
}

#[tauri::command]
async fn answer_question(
    state: tauri::State<'_, App>,
    id: i64,
    up: bool,
) -> Result<String, String> {
    online(&state)?;
    let (install_id, exe) = {
        let settings = state.settings.lock().unwrap();
        let exe = settings
            .played
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.exe.clone())
            .unwrap_or_default();
        (settings.install_id.clone(), exe)
    };

    let build = build_of(&state, &exe);
    // the executable it was actually played as. when that is not the one the
    // table is filed under, saying so is what teaches the service the two go
    // together and spares the next person the search
    shared::rate(id, up, &install_id, &build, &exe)?;

    let mut settings = state.settings.lock().unwrap();
    if !settings.rated.contains(&id) {
        settings.rated.push(id);
    }
    settings.played.retain(|p| p.id != id);
    settings.ask_again_at = 0;
    settings::save(&settings)?;

    Ok(if up {
        "Thanks. That pushes it up the list for everybody else".into()
    } else {
        "Noted. It will sink down the list".into()
    })
}

// skipping keeps the question, it just stops us asking for a couple of days.
// nobody should have to answer to use the app
#[tauri::command]
fn skip_question(state: tauri::State<'_, App>) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    settings.ask_again_at = now_seconds() + settings::SNOOZE;
    settings::save(&settings)
}

// nothing that talks to the service should half work while it is turned off
fn online(state: &tauri::State<'_, App>) -> Result<(), String> {
    if state.settings.lock().unwrap().community {
        return Ok(());
    }
    Err("shared tables are turned off in settings".into())
}

/* none recorded is not the same as none chosen. a game nobody has picked for
uses everything it has, and unticking the last one leaves an empty list that
means exactly that */
// the tables actually in play for a game, in the order they are folded
fn with_parts(state: &tauri::State<'_, App>, exe: &str) -> Vec<Table> {
    let off = off_for(state, exe);
    tables_for(exe)
        .into_iter()
        .filter(|(tag, _)| !off.contains(tag))
        .map(|(_, table)| table)
        .collect()
}

// tag to table name, for saying which table a folded cheat came from
fn credits(exe: &str) -> std::collections::HashMap<String, String> {
    tables_for(exe)
        .into_iter()
        .map(|(tag, table)| (tag, table.game.name))
        .collect()
}

fn whose(named: &std::collections::HashMap<String, String>, id: &str) -> String {
    merge::source_of(id)
        .and_then(|tag| named.get(tag))
        .cloned()
        .unwrap_or_default()
}

// the tags switched off for this game, which is usually none of them
fn off_for(state: &tauri::State<'_, App>, exe: &str) -> Vec<String> {
    state
        .settings
        .lock()
        .unwrap()
        .off
        .get(&exe.to_lowercase())
        .cloned()
        .unwrap_or_default()
}

fn armed_for(state: &tauri::State<'_, App>, exe: &str) -> Vec<String> {
    state
        .settings
        .lock()
        .unwrap()
        .armed
        .get(&exe.to_lowercase())
        .cloned()
        .unwrap_or_default()
}

fn remember_armed(state: &tauri::State<'_, App>, exe: &str, ids: Vec<String>) {
    let mut settings = state.settings.lock().unwrap();
    if ids.is_empty() {
        settings.armed.remove(&exe.to_lowercase());
    } else {
        settings.armed.insert(exe.to_lowercase(), ids);
    }
    let _ = settings::save(&settings);
}

#[tauri::command]
fn set_cheat(
    state: tauri::State<'_, App>,
    exe: String,
    id: String,
    on: bool,
) -> Result<(), String> {
    let mut wanted = armed_for(&state, &exe);

    {
        let guard = state.session.lock().unwrap();
        if let Some(session) = guard.as_ref().filter(|s| s.table().matches_process(&exe)) {
            if on {
                session.arm(&id).map_err(|e| e.to_string())?;
                // written down so the crash, if one follows, has a timeline
                state
                    .lit
                    .lock()
                    .unwrap()
                    .insert(id.clone(), std::time::Instant::now());
            } else {
                session.disarm(&id).map_err(|e| e.to_string())?;
                state.lit.lock().unwrap().remove(&id);
            }
            wanted = session.armed();
        } else {
            wanted.retain(|held| held != &id);
            if on {
                wanted.push(id.clone());
                if let Some(script) = provider_for(&state, &exe, &id) {
                    if !wanted.contains(&script) {
                        wanted.push(script);
                    }
                }
            }
        }
    }

    remember_armed(&state, &exe, wanted);
    Ok(())
}

// which script writes down the symbol a cheat hangs off. switching on infinite
// health should switch on the thing that finds the player too
fn provider_for(state: &tauri::State<'_, App>, exe: &str, id: &str) -> Option<String> {
    use freeplay_table::schema::{Action, Locator};

    let table = tables(state).into_iter().find(|t| t.matches_process(exe))?;
    let cheat = table.cheats.iter().find(|c| c.id == id)?;
    let Some(Locator::Symbol { symbol, .. }) = &cheat.locator else {
        return None;
    };

    table
        .cheats
        .iter()
        .find(|other| match &other.action {
            Action::Script { source } => freeplay_aa::parse(source)
                .map(|script| freeplay_aa::symbols_defined(&script).contains(symbol))
                .unwrap_or(false),
            _ => false,
        })
        .map(|other| other.id.clone())
}

// grabs the game when it turns up and keeps retrying whatever is armed. the
// pointer most cheats hang off is null until you load a save, so trying once
// and giving up is what makes people alt-tab back to the app
fn watch_for_games(handle: tauri::AppHandle) {
    // when each library game was first seen running, and when the count was
    // last folded into settings
    let mut on_screen: HashMap<String, std::time::Instant> = HashMap::new();
    let mut folded_at = std::time::Instant::now();

    loop {
        std::thread::sleep(std::time::Duration::from_millis(1500));

        let state = handle.state::<App>();
        let running: Vec<String> = processes()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.name.to_lowercase())
            .collect();

        clock_games(&state, &running, &mut on_screen, &mut folded_at);

        {
            let mut declined = state.declined.lock().unwrap();
            if declined.as_ref().is_some_and(|exe| !running.contains(exe)) {
                *declined = None;
            }
        }

        let held = state.target.lock().unwrap().as_ref().map(|t| {
            let name = t.name().to_lowercase();
            (name, t.alive())
        });

        match held {
            Some((name, alive)) => {
                if !alive || !running.contains(&name) {
                    tracing::info!("{name} closed, letting go");
                    // gone without being asked to go, so note what was just
                    // switched on before the timeline is thrown away
                    write_verdicts(&state, &name, true);
                    tear_down(&state);
                    // the panel was pinned over a window that is gone
                    overlay::hide(&handle);
                    let _ = handle.emit("detached", name);
                    continue;
                }
                if let Some(session) = state.session.lock().unwrap().as_ref() {
                    session.reconcile();
                }
                if let Some(pid) = overlay_pid(&state) {
                    let app = handle.clone();
                    let _ = handle.run_on_main_thread(move || overlay::follow(&app, pid));
                }
            }
            None => {
                if !state.settings.lock().unwrap().auto_attach {
                    continue;
                }
                let declined = state.declined.lock().unwrap().clone();
                let candidate = tables(&state).into_iter().find(|table| {
                    let exe = table.game.exe.to_lowercase();
                    running.contains(&exe) && declined.as_ref() != Some(&exe)
                });

                if let Some(table) = candidate {
                    let exe = table.game.exe.clone();
                    match attach(handle.clone(), state.clone(), exe.clone()) {
                        Ok(what) => {
                            tracing::info!("attached to {exe} on its own");
                            let _ = handle.emit("attached", what);
                        }
                        Err(e) => tracing::debug!("could not attach to {exe}: {e}"),
                    }
                }
            }
        }
    }
}

/* steam counts its own launches, epic writes a line in an ini, gog only
counts inside galaxy, and a bare exe counts nowhere. this counts everything
the same way: the game was running, so it was played. folded into settings a
minute at a time so closing freeplay mid session loses next to nothing */
fn clock_games(
    state: &tauri::State<'_, App>,
    running: &[String],
    on_screen: &mut HashMap<String, std::time::Instant>,
    folded_at: &mut std::time::Instant,
) {
    let watched: Vec<String> = {
        let held = state.library.lock().unwrap();
        match held.as_ref() {
            Some(games) => games
                .iter()
                .filter_map(|g| g.main_exe())
                .map(|e| e.to_lowercase())
                .collect(),
            // the first scan has not finished, so there is nothing to clock
            None => return,
        }
    };

    let mut fold: Vec<(String, u64)> = Vec::new();
    for exe in &watched {
        let up = running.contains(exe);
        if up && !on_screen.contains_key(exe) {
            on_screen.insert(exe.clone(), std::time::Instant::now());
        } else if !up {
            if let Some(began) = on_screen.remove(exe) {
                fold.push((exe.clone(), began.elapsed().as_secs()));
            }
        }
    }

    if folded_at.elapsed().as_secs() >= 60 && !on_screen.is_empty() {
        *folded_at = std::time::Instant::now();
        for (exe, began) in on_screen.iter_mut() {
            fold.push((exe.clone(), began.elapsed().as_secs()));
            *began = std::time::Instant::now();
        }
    }

    if fold.is_empty() {
        return;
    }
    let mut settings = state.settings.lock().unwrap();
    for (exe, seconds) in fold {
        let sitting = settings.sittings.entry(exe).or_default();
        sitting.seconds += seconds;
        sitting.last = now_seconds();
    }
    let _ = settings::save(&settings);
}

// alt tab away from the game and the panel goes with it. it is pinned over
// one window, so leaving it up over whatever you switched to is just litter
fn watch_the_front(handle: tauri::AppHandle) {
    // one bad reading is not an answer. windows reports odd things mid focus
    // change, and taking the panel away on the strength of a single poll is
    // what made it vanish the instant it appeared
    const STRIKES: u32 = 3;
    let mut wrong = 0u32;

    loop {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if !overlay::showing(&handle) {
            wrong = 0;
            continue;
        }

        let state = handle.state::<App>();
        let still = overlay_pid(&state).is_some_and(|pid| overlay::belongs_here(&handle, pid));
        if still {
            wrong = 0;
            continue;
        }

        wrong += 1;
        if wrong < STRIKES {
            continue;
        }
        wrong = 0;

        tracing::info!("overlay down, {} has focus", overlay::whats_in_front());
        let app = handle.clone();
        let _ = handle.run_on_main_thread(move || overlay::hide(&app));
    }
}

fn parse_kind(name: &str) -> Result<ValueKind, String> {
    name.parse::<ValueKind>()
}

fn report(search: &Search) -> ScanReport {
    ScanReport {
        round: search.rounds(),
        matches: search.len(),
        results: search
            .results(200)
            .into_iter()
            .map(|c| Hit {
                address: format!("{:#018x}", c.addr),
                value: c.value.to_string(),
            })
            .collect(),
    }
}

#[tauri::command]
fn scan_start(
    state: tauri::State<'_, App>,
    kind: String,
    value: Option<String>,
) -> Result<ScanReport, String> {
    let kind = parse_kind(&kind)?;
    let guard = state.target.lock().unwrap();
    let target = guard.as_ref().ok_or("nothing is attached")?;

    let filter = match value.as_deref().filter(|v| !v.trim().is_empty()) {
        Some(text) => Filter::Exact(
            kind.parse(text)
                .ok_or_else(|| format!("{text:?} is not a {kind}"))?,
        ),
        None => Filter::Unknown,
    };

    let search = Search::first(target.as_ref(), kind, filter).map_err(|e| e.to_string())?;
    let out = report(&search);
    *state.search.lock().unwrap() = Some(search);
    Ok(out)
}

#[tauri::command]
fn scan_next(
    state: tauri::State<'_, App>,
    filter: String,
    value: Option<String>,
) -> Result<ScanReport, String> {
    let target_guard = state.target.lock().unwrap();
    let target = target_guard.as_ref().ok_or("nothing is attached")?;

    let mut guard = state.search.lock().unwrap();
    let search = guard.as_mut().ok_or("start a scan first")?;

    let chosen = match filter.as_str() {
        "changed" => Filter::Changed,
        "unchanged" => Filter::Unchanged,
        "increased" => Filter::Increased,
        "decreased" => Filter::Decreased,
        "exact" => {
            let text = value.ok_or("give a value to search for")?;
            let kind = search.kind;
            Filter::Exact(
                kind.parse(&text)
                    .ok_or_else(|| format!("{text:?} is not a {kind}"))?,
            )
        }
        other => return Err(format!("unknown filter {other}")),
    };

    search
        .next(target.as_ref(), chosen)
        .map_err(|e| e.to_string())?;
    Ok(report(search))
}

#[tauri::command]
fn write_value(
    state: tauri::State<'_, App>,
    address: String,
    kind: String,
    value: String,
) -> Result<(), String> {
    let kind = parse_kind(&kind)?;
    let scalar = kind
        .parse(&value)
        .ok_or_else(|| format!("{value:?} is not a {kind}"))?;
    let raw = address.trim().trim_start_matches("0x");
    let addr = usize::from_str_radix(raw, 16).map_err(|_| "bad address".to_string())?;

    let guard = state.target.lock().unwrap();
    let target = guard.as_ref().ok_or("nothing is attached")?;
    target.write_scalar(addr, scalar).map_err(|e| e.to_string())
}

fn main() {
    // a windowed build has no console, so this went nowhere
    log::start(std::env::var("FREEPLAY_VERBOSE").is_ok());

    tauri::Builder::default()
        .manage(App {
            settings: Mutex::new(settings::load()),
            ..Default::default()
        })
        .register_asynchronous_uri_scheme_protocol("art", |ctx, request, responder| {
            // an epic id has colons in it and some webviews hand the path back
            // with them escaped, which would miss the cache every time
            let path = request.uri().path().replace("%3A", ":").replace("%3a", ":");
            let handle = ctx.app_handle().clone();
            std::thread::spawn(move || {
                let id = path.trim_matches('/').split('/').next().unwrap_or_default();
                let found = handle.state::<App>().art.lock().unwrap().get(id).cloned();
                responder.respond(serve_art(&path, found))
            });
        })
        .setup(|app| {
            let _ = app.state::<App>().handle.set(app.handle().clone());

            // the window is built hidden, so this is what puts it on screen.
            // sizing it after it was already up meant it appeared at the size
            // in tauri.conf and then jumped
            if let Some(window) = app.get_webview_window("main") {
                set_window_icons(&window);
                let spot = app.state::<App>().settings.lock().unwrap().window;
                place::restore(&window, spot);
            }

            // own thread. a slow or blocked network must never be something
            // you wait on before the window shows up
            if app.state::<App>().settings.lock().unwrap().auto_update {
                let handle = app.handle().clone();
                std::thread::spawn(move || match freeplay_sync::update(&synced_dir()) {
                    Ok(report) if report.changed() => {
                        *handle.state::<App>().tables.lock().unwrap() = None;
                        let _ = handle.emit("tables-updated", report.summary());
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("could not fetch tables: {e}"),
                });
            }

            if app.state::<App>().settings.lock().unwrap().overlay {
                if let Err(e) = overlay::prepare(app.handle()) {
                    tracing::warn!("overlay: {e}");
                }
            }
            if let Err(e) = rebind_hotkey(app.handle()) {
                tracing::warn!("overlay hotkey: {e}");
            }

            let handle = app.handle().clone();
            std::thread::spawn(move || watch_for_games(handle));

            let handle = app.handle().clone();
            std::thread::spawn(move || watch_the_front(handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_games,
            game_art,
            settings,
            save_settings,
            folded,
            fold,
            table_fit,
            installed_tables,
            use_table,
            diagnostics,
            open_log,
            table_count,
            credit,
            import_table,
            find_table,
            shared_tables,
            sort_options,
            search_tables,
            install_shared,
            remove_table,
            pending_question,
            answer_question,
            skip_question,
            share_table,
            whoami,
            claim_name,
            recover_name,
            forget_name,
            open_folder,
            open_url,
            version,
            overlay_status,
            set_overlay,
            toggle_overlay,
            hide_overlay,
            overlay_game,
            pick_table,
            profile_games,
            export_profile,
            open_profile,
            apply_profile,
            save_phrase,
            update_tables,
            launch_game,
            focus_game,
            list_processes,
            attach,
            detach,
            cheats,
            set_cheat,
            set_cheat_value,
            bind_key,
            add_game,
            remove_added,
            scan_start,
            scan_next,
            write_value,
        ])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let Some(window) = window.get_webview_window("main") else {
                    return;
                };
                let state = window.state::<App>();
                let mut held = state.settings.lock().unwrap();
                held.window = place::at_close(&window);
                if let Err(e) = settings::save(&held) {
                    tracing::warn!("could not remember the window: {e}");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("freeplay failed to start");
}

#[cfg(test)]
mod wording {
    use super::{how_long, link_in, verdicts};

    #[test]
    fn a_cheat_lit_moments_before_the_crash_is_blamed() {
        let ages = [("god-mode".to_string(), 5u64), ("orens".to_string(), 50)];
        let (blamed, proven) = verdicts(&ages);
        assert_eq!(blamed, ["god-mode"]);
        assert!(proven.is_empty(), "50 seconds says nothing either way");
    }

    #[test]
    fn a_cheat_on_for_ages_without_incident_is_let_off() {
        let ages = [("god-mode".to_string(), 300u64)];
        let (blamed, proven) = verdicts(&ages);
        assert!(blamed.is_empty());
        assert_eq!(proven, ["god-mode"]);
    }

    #[test]
    fn nothing_lit_means_nothing_to_say() {
        let (blamed, proven) = verdicts(&[]);
        assert!(blamed.is_empty() && proven.is_empty());
    }

    #[test]
    fn how_long_reads_like_somebody_said_it() {
        assert_eq!(how_long(30), "a minute");
        assert_eq!(how_long(60), "a minute");
        assert_eq!(how_long(300), "5 minutes");
        assert_eq!(how_long(3600), "60 minutes");
        assert_eq!(how_long(5400), "90 minutes");
        assert_eq!(how_long(5460), "1.5 hours");
        assert_eq!(how_long(7200), "2 hours");
        assert_eq!(how_long(9000), "2.5 hours");
    }

    #[test]
    fn a_long_session_does_not_come_out_as_a_fraction_of_a_fraction() {
        assert_eq!(how_long(36_000), "10 hours");
    }

    // the source link is pulled out of a free text notes field, so it has to
    // cope with the note being a sentence rather than just a url
    #[test]
    fn the_source_link_comes_out_of_the_notes() {
        assert_eq!(
            link_in("Converted from a Cheat Engine table by X. Source: https://example.com/t=1"),
            "https://example.com/t=1"
        );
    }

    #[test]
    fn a_full_stop_after_the_link_is_not_part_of_it() {
        assert_eq!(
            link_in("see https://example.com/thread."),
            "https://example.com/thread"
        );
    }

    #[test]
    fn notes_with_no_link_give_nothing() {
        assert_eq!(link_in("tested on the enhanced edition"), "");
        assert_eq!(link_in(""), "");
    }

    // http would be handed to the shell and open a browser on a plain socket
    #[test]
    fn only_https_counts_as_a_link() {
        assert_eq!(link_in("http://example.com/thread"), "");
    }
}
