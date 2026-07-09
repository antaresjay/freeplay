//! the panel that sits over the game
//!
//! this is a borderless always on top window pinned to the right edge of the
//! game's own window. it is not drawn inside the game, which means it works
//! for borderless and windowed and does nothing at all for exclusive
//! fullscreen. drawing inside the game would mean injecting a dll and hooking
//! the swap chain, which is the thing every anti-cheat is looking for, and
//! this app refuses to go near games that have one.

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "overlay";
const WIDTH: f64 = 296.0;
// clear of the edge, so it does not sit on top of a health bar in the corner
const MARGIN: f64 = 24.0;

pub fn showing(app: &tauri::AppHandle) -> bool {
    app.get_webview_window(LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

pub fn hide(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

// built once, hidden, on the main thread. creating a window from the hotkey
// thread is asking for trouble on windows, and building it on the first press
// makes that press feel slow
pub fn prepare(app: &tauri::AppHandle) -> Result<(), String> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
        .title("Freeplay overlay")
        .inner_size(WIDTH, 560.0)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .transparent(true)
        .visible(false)
        .build()
        .map_err(|e| format!("could not open the overlay: {e}"))?;
    Ok(())
}

// it goes over the game window and nowhere else. not over freeplay, not over
// a browser, and not over a game with nothing to switch on
pub fn show(app: &tauri::AppHandle, pid: Option<u32>) -> Result<(), String> {
    let pid = pid.ok_or("attach to a game with a table first")?;
    if !game_in_front(pid) {
        return Err("bring the game to the front first".into());
    }

    prepare(app)?;
    let Some(window) = app.get_webview_window(LABEL) else {
        return Err("the overlay is not there".into());
    };

    pin_to_game(&window, pid);
    window.show().map_err(|e| e.to_string())?;
    let _ = window.set_always_on_top(true);
    let _ = window.set_focus();
    // it has been sitting hidden and polling slowly, so tell it to catch up
    // rather than showing whatever it last saw for a second
    let _ = window.emit("wake", ());
    Ok(())
}

// the game is what is in front, or the panel itself is because you clicked it
#[cfg(windows)]
pub fn belongs_here(app: &tauri::AppHandle, pid: u32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let front = unsafe { GetForegroundWindow() };
    if front.is_invalid() {
        return false;
    }
    if main_window(pid) == Some(front) {
        return true;
    }
    app.get_webview_window(LABEL)
        .and_then(|w| w.hwnd().ok())
        .is_some_and(|ours| ours == front)
}

#[cfg(windows)]
fn game_in_front(pid: u32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    main_window(pid) == Some(unsafe { GetForegroundWindow() })
}

#[cfg(not(windows))]
pub fn belongs_here(_app: &tauri::AppHandle, _pid: u32) -> bool {
    false
}

#[cfg(not(windows))]
fn game_in_front(_pid: u32) -> bool {
    false
}

// pixels all the way through. mixing logical and physical puts the panel in
// the wrong place the moment anybody runs windows at anything but 100 per cent
fn pin_to_game(window: &tauri::WebviewWindow, pid: u32) {
    let Some(rect) = game_rect(pid) else { return };
    let scale = window.scale_factor().unwrap_or(1.0);
    let width = (WIDTH * scale).round() as i32;
    let margin = (MARGIN * scale).round() as i32;

    let height = (rect.height - (margin * 2) as f64).clamp(240.0 * scale, 900.0 * scale);
    let _ = window.set_size(tauri::PhysicalSize::new(
        width as u32,
        height.round() as u32,
    ));
    let _ = window.set_position(tauri::PhysicalPosition::new(
        rect.right - width - margin,
        rect.top + margin,
    ));
}

// the game can be moved while the panel is up, and it should go with it
pub fn follow(app: &tauri::AppHandle, pid: u32) {
    if !showing(app) {
        return;
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        pin_to_game(&window, pid);
    }
}

pub fn toggle(app: &tauri::AppHandle, pid: Option<u32>) -> Result<bool, String> {
    if showing(app) {
        hide(app);
        Ok(false)
    } else {
        show(app, pid)?;
        Ok(true)
    }
}

pub struct Rect {
    pub right: i32,
    pub top: i32,
    pub height: f64,
}

// the game's main window, found by walking every top level window and asking
// which process owns it. a game usually has several, so take the biggest
// visible one rather than the first
#[cfg(windows)]
fn main_window(pid: u32) -> Option<windows::Win32::Foundation::HWND> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    };

    struct Hunt {
        pid: u32,
        best: Option<HWND>,
        area: i64,
    }

    unsafe extern "system" fn look(window: HWND, carried: LPARAM) -> BOOL {
        let hunt = unsafe { &mut *(carried.0 as *mut Hunt) };

        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
        if owner != hunt.pid || !unsafe { IsWindowVisible(window) }.as_bool() {
            return TRUE;
        }

        let mut rect = RECT::default();
        if unsafe { GetWindowRect(window, &mut rect) }.is_err() {
            return TRUE;
        }
        let area = (rect.right - rect.left) as i64 * (rect.bottom - rect.top) as i64;
        if area > hunt.area {
            hunt.area = area;
            hunt.best = Some(window);
        }
        TRUE
    }

    let mut hunt = Hunt {
        pid,
        best: None,
        area: 0,
    };
    unsafe {
        let _ = EnumWindows(Some(look), LPARAM(&mut hunt as *mut Hunt as isize));
    }
    hunt.best
}

#[cfg(windows)]
pub fn game_rect(pid: u32) -> Option<Rect> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let window = main_window(pid)?;
    let mut rect = RECT::default();
    unsafe { GetWindowRect(window, &mut rect) }.ok()?;

    Some(Rect {
        right: rect.right,
        top: rect.top,
        height: (rect.bottom - rect.top) as f64,
    })
}

// bring the game back to the front. clicking play on a game that is already
// running used to start a second copy, or nothing at all
#[cfg(windows)]
pub fn focus_game(pid: u32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    let Some(window) = main_window(pid) else {
        return false;
    };
    unsafe {
        // a minimised window will not come forward without this
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window).as_bool()
    }
}

#[cfg(not(windows))]
pub fn game_rect(_pid: u32) -> Option<Rect> {
    None
}

#[cfg(not(windows))]
pub fn focus_game(_pid: u32) -> bool {
    false
}
