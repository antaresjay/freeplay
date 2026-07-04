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
use freeplay_library::{discover, InstalledGame};
use freeplay_session::Session;
use freeplay_table::resolve::State as CheatState;
use freeplay_table::Table;
use serde::Serialize;

mod community;
mod dialog;
mod log;
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
    // which art each game has, keyed by app id. the bytes go over the art
    // protocol, this is just the three exists checks
    art: Mutex<HashMap<String, ArtUrls>>,
    // anti-cheat found in a game's folder, keyed by install dir
    guards: Mutex<HashMap<PathBuf, Option<String>>>,
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
    // minutes played and when, straight out of steam
    minutes: Option<u32>,
    last_played: Option<u64>,
    pinned: bool,
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

// three folders can each hold a table for the same game, and the one you
// picked most recently is the one you meant. ranking by folder instead meant
// an imported .CT shadowed every table you downloaded afterwards, so pressing
// "use this" said it worked and changed nothing
fn load_tables() -> Vec<Table> {
    let mut found: Vec<(std::time::SystemTime, Table)> = Vec::new();

    for dir in [mine_dir(), synced_dir(), tables_dir()] {
        for (path, table) in Table::load_dir_with_paths(&dir) {
            let touched = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            found.push((touched, table));
        }
    }

    found.sort_by_key(|(touched, _)| std::cmp::Reverse(*touched));

    let mut tables: Vec<Table> = Vec::new();
    for (_, table) in found {
        if !tables.iter().any(|t| t.matches_process(&table.game.exe)) {
            tables.push(table);
        }
    }
    tables
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

fn table_for(exe: &str) -> Option<Table> {
    load_tables().into_iter().find(|t| t.matches_process(exe))
}

// the version stamped into the game's exe. a table is written against one
// build and quietly stops working on the next, so every share and every vote
// carries the build it was judged on
fn build_of(state: &tauri::State<'_, App>, exe: &str) -> String {
    let wanted = exe.to_lowercase();
    library(state, false)
        .into_iter()
        .find(|game| {
            game.main_exe()
                .is_some_and(|found| found.to_lowercase() == wanted)
        })
        .and_then(|game| game.executables.first().cloned())
        .and_then(|path| freeplay_library::build::of(&path))
        .unwrap_or_default()
}

// what this game is called in the library beats what the table calls itself. a
// .CT converted from witcher2.CT calls the game "witcher2"
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

// names in a game's install folder, two deep. anti-cheats drop their loader
// next to the exe or one folder down, so that is far enough
fn folder_names(dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth == 0 || out.len() > 4000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        out.push(entry.file_name().to_string_lossy().to_string());
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            folder_names(&entry.path(), depth - 1, out);
        }
    }
}

fn guard_for(state: &tauri::State<'_, App>, dir: &Path) -> Option<String> {
    if let Some(cached) = state.guards.lock().unwrap().get(dir) {
        return cached.clone();
    }

    let mut names = Vec::new();
    folder_names(dir, 2, &mut names);
    let found =
        freeplay_core::guard::inspect_names(names.iter().map(String::as_str)).map(str::to_string);

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

    let found = discover();
    *state.library.lock().unwrap() = Some(found.clone());
    found
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
    let (pinned, favourites) = {
        let settings = state.settings.lock().unwrap();
        (settings.pinned.clone(), settings.favourites.clone())
    };

    Ok(library(&state, refresh)
        .into_iter()
        .map(|game| {
            let exe = game.main_exe();
            let lower = exe.as_deref().unwrap_or_default().to_lowercase();
            let key = key_for(&game);
            let play = game
                .app_id
                .as_deref()
                .and_then(|id| played.get(id))
                .copied()
                .unwrap_or_default();

            GameRow {
                guard: guard_for(&state, &game.install_dir),
                running: !lower.is_empty() && running.iter().any(|p| p == &lower),
                has_table: exe
                    .as_deref()
                    .is_some_and(|e| tables.iter().any(|t| t.matches_process(e))),
                minutes: play.minutes,
                last_played: play.last_played,
                pinned: pinned.contains(&key),
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

#[tauri::command]
async fn install_shared(state: tauri::State<'_, App>, id: i64) -> Result<String, String> {
    let install_id = state.settings.lock().unwrap().install_id.clone();
    let (_, table) = shared::install(id, &install_id, &synced_dir())?;

    {
        let mut settings = state.settings.lock().unwrap();
        settings.grabbed.insert(table.game.exe.to_lowercase(), id);
        let _ = settings::save(&settings);
    }
    forget_tables(&state);
    Ok(format!(
        "{} is ready, {} cheats",
        table.game.name,
        table.cheats.len()
    ))
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
    tracing::info!("removed the table for {exe}");

    Ok("Removed. What you had switched on for it is forgotten too".to_string())
}

#[tauri::command]
async fn rate_shared(
    state: tauri::State<'_, App>,
    id: i64,
    up: bool,
    exe: String,
) -> Result<(), String> {
    let install_id = state.settings.lock().unwrap().install_id.clone();
    let build = build_of(&state, &exe);
    shared::rate(id, up, &install_id, &build)?;

    let mut settings = state.settings.lock().unwrap();
    if !settings.rated.contains(&id) {
        settings.rated.push(id);
    }
    let _ = settings::save(&settings);
    Ok(())
}

// which shared table this game is using, and whether it has been rated yet
#[tauri::command]
fn using(state: tauri::State<'_, App>, exe: String) -> Option<(i64, bool)> {
    let settings = state.settings.lock().unwrap();
    settings
        .grabbed
        .get(&exe.to_lowercase())
        .map(|id| (*id, settings.rated.contains(id)))
}

#[tauri::command]
async fn share_table(
    state: tauri::State<'_, App>,
    exe: String,
    anonymous: bool,
) -> Result<String, String> {
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
fn open_profile(state: tauri::State<'_, App>, path: Option<String>) -> Result<Peek, String> {
    let picked = match path {
        Some(given) => Some(PathBuf::from(given)),
        None => dialog::open(&dialog::Ask {
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
        match shared::install(id, &install_id, &synced_dir()) {
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
fn save_phrase(name: String, phrase: String) -> Result<String, String> {
    let Some(path) = dialog::save(&dialog::Ask {
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
fn pick_table(state: tauri::State<'_, App>, exe: Option<String>) -> Result<String, String> {
    let Some(path) = dialog::open(&dialog::Ask {
        title: "Open a Cheat Engine table",
        kinds: dialog::TABLES,
        suggested: "",
        extension: "CT",
    }) else {
        return Err(String::new());
    };
    import_table(state, path.display().to_string(), exe)
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

#[tauri::command]
fn save_settings(state: tauri::State<'_, App>, next: Settings) -> Result<Settings, String> {
    let mut next = next;
    next.tidy();
    settings::save(&next)?;
    *state.settings.lock().unwrap() = next.clone();
    Ok(next)
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

#[tauri::command]
fn game_art(state: tauri::State<'_, App>, app_id: String) -> ArtUrls {
    if let Some(cached) = state.art.lock().unwrap().get(&app_id) {
        return cached.clone();
    }

    let found = freeplay_library::art::steam(&app_id);
    let url = |present: bool, kind: &str| present.then(|| art_url(&app_id, kind));
    let urls = ArtUrls {
        cover: url(found.cover.is_some(), "cover"),
        hero: url(found.hero.is_some(), "hero"),
        logo: url(found.logo.is_some(), "logo"),
    };

    state.art.lock().unwrap().insert(app_id, urls.clone());
    urls
}

// serves what steam already cached. path is /<appid>/<kind>
fn serve_art(path: &str) -> tauri::http::Response<Vec<u8>> {
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
    // the app id goes into a path, so it had better be one
    if app_id.is_empty() || !app_id.bytes().all(|b| b.is_ascii_digit()) {
        return deny();
    }

    let found = freeplay_library::art::steam(app_id);
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
    let mime = match file.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
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
fn attach(state: tauri::State<'_, App>, exe: String) -> Result<Attached, String> {
    tear_down(&state);
    *state.declined.lock().unwrap() = None;

    let target = WindowsTarget::attach_by_name(&exe).map_err(friendly)?;
    let pid = target.pid();
    let arch = target.arch().label().to_string();
    let shared: Arc<dyn Target> = Arc::new(target);

    let table = table_for(&exe);
    let has_table = table.is_some();
    let name = table
        .as_ref()
        .map(|t| t.game.name.clone())
        .unwrap_or_else(|| exe.clone());

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
        *state.session.lock().unwrap() = Some(session);
    }
    *state.target.lock().unwrap() = Some(shared);
    *state.search.lock().unwrap() = None;

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
    if let Some(mut session) = state.session.lock().unwrap().take() {
        session.stop();
        session.disable_all();
    }
    *state.target.lock().unwrap() = None;
    *state.search.lock().unwrap() = None;
}

// everything except the four fields that need a live process, so the same
// shape comes back whether the game is running or not
fn cheat_row(cheat: &freeplay_table::schema::Cheat, typed: Option<&String>) -> CheatRow {
    CheatRow {
        id: cheat.id.clone(),
        name: cheat.name.clone(),
        category: cheat.category.label().to_string(),
        description: cheat.description.clone(),
        hint: cheat.hint.clone(),
        state: "idle".into(),
        reason: String::new(),
        armed: false,
        live: false,
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

#[tauri::command]
fn cheats(state: tauri::State<'_, App>, exe: String) -> Vec<CheatRow> {
    let typed = values_for(&state, &exe);
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
                row.state = if live { "on".into() } else { label.to_string() };
                row.reason = reason;
                row.armed = session.is_armed(&cheat.id);
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
    let Some(table) = tables(&state).into_iter().find(|t| t.matches_process(&exe)) else {
        return Vec::new();
    };

    table
        .cheats
        .iter()
        .map(|cheat| {
            let mut row = cheat_row(cheat, typed.get(&cheat.id));
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
            } else {
                session.disarm(&id).map_err(|e| e.to_string())?;
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
    loop {
        std::thread::sleep(std::time::Duration::from_millis(1500));

        let state = handle.state::<App>();
        let running: Vec<String> = processes()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.name.to_lowercase())
            .collect();

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
                    tear_down(&state);
                    let _ = handle.emit("detached", name);
                    continue;
                }
                if let Some(session) = state.session.lock().unwrap().as_ref() {
                    session.reconcile();
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
                    match attach(state.clone(), exe.clone()) {
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
        .register_asynchronous_uri_scheme_protocol("art", |_ctx, request, responder| {
            let path = request.uri().path().to_string();
            std::thread::spawn(move || responder.respond(serve_art(&path)));
        })
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                set_window_icons(&window);
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

            let handle = app.handle().clone();
            std::thread::spawn(move || watch_for_games(handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_games,
            game_art,
            settings,
            save_settings,
            diagnostics,
            open_log,
            table_count,
            import_table,
            find_table,
            shared_tables,
            sort_options,
            install_shared,
            remove_table,
            rate_shared,
            using,
            share_table,
            whoami,
            claim_name,
            recover_name,
            forget_name,
            open_folder,
            open_url,
            version,
            pick_table,
            profile_games,
            export_profile,
            open_profile,
            apply_profile,
            save_phrase,
            update_tables,
            launch_game,
            list_processes,
            attach,
            detach,
            cheats,
            set_cheat,
            set_cheat_value,
            scan_start,
            scan_next,
            write_value,
        ])
        .run(tauri::generate_context!())
        .expect("freeplay failed to start");
}
