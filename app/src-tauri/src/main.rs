#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Desktop front end.
//!
//! Everything here is glue. The interesting parts live in freeplay-core and
//! freeplay-session, which is deliberate: the app should be replaceable
//! without touching anything that knows how memory works.

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

mod settings;
use settings::Settings;
use tauri::Manager;

#[derive(Default)]
struct App {
    target: Mutex<Option<Arc<dyn Target>>>,
    session: Mutex<Option<Session>>,
    search: Mutex<Option<Search>>,
    /// Which art a game actually has, keyed by app id. The bytes go over the
    /// art protocol, this is only the three existence checks.
    art: Mutex<HashMap<String, ArtUrls>>,
    /// Anti-cheat found in a game's folder, keyed by install directory.
    guards: Mutex<HashMap<PathBuf, Option<String>>>,
    /// Installed games. Finding them means walking every install directory,
    /// which takes seconds, so it happens once and then only when asked.
    library: Mutex<Option<Vec<InstalledGame>>>,
    settings: Mutex<Settings>,
}

#[derive(Serialize)]
struct GameRow {
    /// Stable across launches, which is what pinning and favourites are keyed
    /// on. An app id where there is one, the install path otherwise.
    key: String,
    name: String,
    store: String,
    exe: Option<String>,
    dir: String,
    app_id: Option<String>,
    running: bool,
    has_table: bool,
    /// Name of the anti-cheat shipped alongside the game, if there is one.
    guard: Option<String>,
    /// Minutes played and when, straight out of Steam's own record.
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

#[derive(Serialize)]
struct Attached {
    process: String,
    pid: u32,
    game: String,
    table: bool,
}

#[derive(Serialize)]
struct CheatRow {
    id: String,
    name: String,
    category: String,
    description: String,
    hint: String,
    /// "ready", "wait" or "broken".
    state: String,
    reason: String,
    on: bool,
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
    // Next to the executable when installed, in the repo while developing.
    let beside = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("tables")));
    if let Some(dir) = beside.filter(|d| d.is_dir()) {
        return dir;
    }
    PathBuf::from("tables")
}

fn load_tables() -> Vec<Table> {
    Table::load_dir(tables_dir())
}

fn table_for(exe: &str) -> Option<Table> {
    load_tables().into_iter().find(|t| t.matches_process(exe))
}

/// Names sitting in a game's install folder, two levels down. Anti-cheats
/// either drop their loader next to the executable or in a folder of their
/// own, so that is deep enough and keeps this off the slow path.
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

/// Walking every install directory takes seconds, and nothing about an
/// installed game changes while the app is open. Scan once, then only when the
/// refresh button is pressed.
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

/// Async so the scan lands on a worker thread. A synchronous command runs on
/// the main thread, which means the window stops answering while it works.
#[tauri::command]
async fn list_games(state: tauri::State<'_, App>, refresh: bool) -> Result<Vec<GameRow>, ()> {
    let running: Vec<String> = processes()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.name.to_lowercase())
        .collect();
    let tables = load_tables();
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

#[tauri::command]
fn save_settings(state: tauri::State<'_, App>, next: Settings) -> Result<Settings, String> {
    let mut next = next;
    next.tidy();
    settings::save(&next)?;
    *state.settings.lock().unwrap() = next.clone();
    Ok(next)
}

#[tauri::command]
async fn launch_game(state: tauri::State<'_, App>, key: String) -> Result<(), String> {
    let game = library(&state, false)
        .into_iter()
        .find(|g| key_for(g) == key)
        .ok_or("that game is not in the library any more")?;

    freeplay_library::launch::start(&game).map(|_| ())
}

/// Box art used to go over as base64 in the command reply. That put megabytes
/// of string through the bridge and made the webview decode the same picture
/// again on every redraw. It is served as bytes now, so it gets cached and
/// decoded once like any other image.
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

/// Serves the files Steam already cached. Path is `/<appid>/<kind>`.
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
    // The app id goes into a path, so it has to actually be one.
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
    detach(state.clone());

    let target = WindowsTarget::attach_by_name(&exe).map_err(friendly)?;
    let pid = target.pid();
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
        *state.session.lock().unwrap() = Some(session);
    }
    *state.target.lock().unwrap() = Some(shared);
    *state.search.lock().unwrap() = None;

    Ok(Attached {
        process: exe,
        pid,
        game: name,
        table: has_table,
    })
}

#[tauri::command]
fn detach(state: tauri::State<'_, App>) {
    if let Some(mut session) = state.session.lock().unwrap().take() {
        session.stop();
        session.disable_all();
    }
    *state.target.lock().unwrap() = None;
    *state.search.lock().unwrap() = None;
}

#[tauri::command]
fn cheats(state: tauri::State<'_, App>) -> Vec<CheatRow> {
    let guard = state.session.lock().unwrap();
    let Some(session) = guard.as_ref() else {
        return Vec::new();
    };

    session
        .table()
        .cheats
        .iter()
        .map(|cheat| {
            let state = freeplay_table::evaluate(session.target().as_ref(), &cheat.locator);
            let (label, reason) = match &state {
                CheatState::Ready { .. } => ("ready", String::new()),
                CheatState::Unavailable { reason } => ("wait", reason.clone()),
                CheatState::Broken { reason } => ("broken", reason.clone()),
            };
            CheatRow {
                id: cheat.id.clone(),
                name: cheat.name.clone(),
                category: cheat.category.label().to_string(),
                description: cheat.description.clone(),
                hint: cheat.hint.clone(),
                state: label.to_string(),
                reason,
                on: session.is_on(&cheat.id),
            }
        })
        .collect()
}

#[tauri::command]
fn set_cheat(state: tauri::State<'_, App>, id: String, on: bool) -> Result<(), String> {
    let guard = state.session.lock().unwrap();
    let session = guard.as_ref().ok_or("nothing is attached")?;

    if on {
        session.enable(&id).map_err(|e| e.to_string())
    } else {
        session.disable(&id).map_err(|e| e.to_string())
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
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

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
            let _ = app.get_webview_window("main");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_games,
            game_art,
            settings,
            save_settings,
            launch_game,
            list_processes,
            attach,
            detach,
            cheats,
            set_cheat,
            scan_start,
            scan_next,
            write_value,
        ])
        .run(tauri::generate_context!())
        .expect("freeplay failed to start");
}
