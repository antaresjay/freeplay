//! the panel that sits over the game
//!
//! this is a borderless always on top window pinned to the right edge of the
//! game's own window. it is not drawn inside the game, which means it works
//! for borderless and windowed and does nothing at all for exclusive
//! fullscreen. drawing inside the game would mean injecting a dll and hooking
//! the swap chain, which is the thing every anti-cheat is looking for, and
//! this app refuses to go near games that have one.

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "overlay";
const WIDTH: f64 = 340.0;
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

pub fn show(app: &tauri::AppHandle, pid: Option<u32>) -> Result<(), String> {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
            .title("Freeplay overlay")
            .inner_size(WIDTH, 620.0)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .transparent(true)
            .visible(false)
            .build()
            .map_err(|e| format!("could not open the overlay: {e}"))?,
    };

    if let Some(rect) = pid.and_then(game_rect) {
        let _ = window.set_size(tauri::LogicalSize::new(WIDTH, rect.height.min(760.0)));
        let _ = window.set_position(tauri::PhysicalPosition::new(
            rect.right - (WIDTH as i32) - MARGIN as i32,
            rect.top + MARGIN as i32,
        ));
    }

    window.show().map_err(|e| e.to_string())?;
    let _ = window.set_always_on_top(true);
    let _ = window.set_focus();
    Ok(())
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
pub fn game_rect(pid: u32) -> Option<Rect> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    };

    struct Hunt {
        pid: u32,
        best: Option<RECT>,
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
            hunt.best = Some(rect);
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

    hunt.best.map(|rect| Rect {
        right: rect.right,
        top: rect.top,
        height: (rect.bottom - rect.top) as f64,
    })
}

#[cfg(not(windows))]
pub fn game_rect(_pid: u32) -> Option<Rect> {
    None
}
