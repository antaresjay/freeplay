#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Desktop front end.
//!
//! Everything here is glue. The interesting parts live in freeplay-core and
//! freeplay-session, which is deliberate: the app should be replaceable
//! without touching anything that knows how memory works.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use freeplay_core::search::{Filter, Search};
use freeplay_core::target::Target;
use freeplay_core::value::ValueKind;
use freeplay_core::windows_target::{processes, WindowsTarget};
use freeplay_core::Error as CoreError;
use freeplay_library::discover;
use freeplay_session::Session;
use freeplay_table::resolve::State as CheatState;
use freeplay_table::Table;
use serde::Serialize;
use tauri::Manager;

#[derive(Default)]
struct App {
    target: Mutex<Option<Arc<dyn Target>>>,
    session: Mutex<Option<Session>>,
    search: Mutex<Option<Search>>,
}

#[derive(Serialize)]
struct GameRow {
    name: String,
    store: String,
    exe: Option<String>,
    dir: String,
    running: bool,
    has_table: bool,
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

#[tauri::command]
fn list_games() -> Vec<GameRow> {
    let running: Vec<String> = processes()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.name.to_lowercase())
        .collect();
    let tables = load_tables();

    discover()
        .into_iter()
        .map(|game| {
            let exe = game.main_exe();
            let lower = exe.as_deref().unwrap_or_default().to_lowercase();
            GameRow {
                running: !lower.is_empty() && running.iter().any(|p| p == &lower),
                has_table: exe
                    .as_deref()
                    .is_some_and(|e| tables.iter().any(|t| t.matches_process(e))),
                name: game.name,
                store: game.store.label().to_string(),
                dir: game.install_dir.display().to_string(),
                exe,
            }
        })
        .collect()
}

#[tauri::command]
fn list_processes() -> Vec<ProcessRow> {
    let mut all: Vec<ProcessRow> = processes()
        .unwrap_or_default()
        .into_iter()
        .map(|p| ProcessRow { pid: p.pid, name: p.name })
        .collect();
    all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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

    Ok(Attached { process: exe, pid, game: name, table: has_table })
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
            .map(|c| Hit { address: format!("{:#018x}", c.addr), value: c.value.to_string() })
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
        Some(text) => {
            Filter::Exact(kind.parse(text).ok_or_else(|| format!("{text:?} is not a {kind}"))?)
        }
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
            Filter::Exact(kind.parse(&text).ok_or_else(|| format!("{text:?} is not a {kind}"))?)
        }
        other => return Err(format!("unknown filter {other}")),
    };

    search.next(target.as_ref(), chosen).map_err(|e| e.to_string())?;
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
    let scalar = kind.parse(&value).ok_or_else(|| format!("{value:?} is not a {kind}"))?;
    let raw = address.trim().trim_start_matches("0x");
    let addr = usize::from_str_radix(raw, 16).map_err(|_| "bad address".to_string())?;

    let guard = state.target.lock().unwrap();
    let target = guard.as_ref().ok_or("nothing is attached")?;
    target.write_scalar(addr, scalar).map_err(|e| e.to_string())
}

fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    tauri::Builder::default()
        .manage(App::default())
        .setup(|app| {
            let _ = app.get_webview_window("main");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_games,
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
