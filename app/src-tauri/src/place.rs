//! putting the window back where it was

use tauri::{LogicalSize, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::settings::Spot;

// enough of the bar has to land on a screen to grab it with the mouse
const GRABBABLE: i32 = 90;

// always the outer rectangle, the one with the frame, because that is what
// windows hands back for a maximised window and mixing the two is how nine
// pixels went missing off the bottom every time one was closed maximised
pub fn read(window: &WebviewWindow) -> Option<Spot> {
    let at = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(Spot {
        x: at.x,
        y: at.y,
        w: size.width,
        h: size.height,
        maximised: window.is_maximized().unwrap_or(false),
    })
}

// a monitor that was there last time may not be there now, and a window put
// back onto one that has gone is a window you cannot reach
fn on_a_screen(window: &WebviewWindow, spot: &Spot) -> bool {
    let Ok(screens) = window.available_monitors() else {
        return false;
    };
    screens.iter().any(|screen| {
        let at = screen.position();
        let size = screen.size();
        let (left, top) = (at.x, at.y);
        let (right, bottom) = (at.x + size.width as i32, at.y + size.height as i32);

        // the title bar, not the whole window. dragged half off the side is a
        // thing people do on purpose
        let bar_left = spot.x;
        let bar_right = spot.x + spot.w as i32;
        bar_right - GRABBABLE > left
            && bar_left + GRABBABLE < right
            && spot.y + 34 > top
            && spot.y < bottom
    })
}

pub fn restore(window: &WebviewWindow, spot: Option<Spot>) {
    if let Some(spot) = spot.filter(|s| s.w > 0 && s.h > 0 && on_a_screen(window, s)) {
        let _ = window.set_position(PhysicalPosition::new(spot.x, spot.y));
        // set_size takes the inner size, and the frame is only worth measuring
        // here, where the window is new and certainly not maximised yet
        let frame = window
            .outer_size()
            .ok()
            .zip(window.inner_size().ok())
            .map(|(outer, inner)| (outer.width - inner.width, outer.height - inner.height))
            .unwrap_or((0, 0));
        let _ = window.set_size(PhysicalSize::new(
            spot.w.saturating_sub(frame.0),
            spot.h.saturating_sub(frame.1),
        ));
        if spot.maximised {
            let _ = window.maximize();
        }
    } else {
        // whatever tauri.conf asked for, in the middle of the main screen
        let _ = window.set_size(LogicalSize::new(1180.0, 760.0));
        let _ = window.center();
    }
    let _ = window.show();
}

// closing while maximised has to come back maximised, and unmaximising after
// that has to give back the size it had before rather than the size of the
// screen. following resize events to keep that size does not work, the one
// from a maximise arrives before is_maximized starts saying yes and the big
// size overwrites the very thing being kept. windows already holds the restore
// rectangle, so ask it
#[cfg(windows)]
fn before_it_was_maximised(window: &WebviewWindow) -> Option<Spot> {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowPlacement, WINDOWPLACEMENT};

    let hwnd = window.hwnd().ok()?;
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    unsafe { GetWindowPlacement(hwnd, &mut placement) }.ok()?;

    let box_ = placement.rcNormalPosition;
    Some(Spot {
        x: box_.left,
        y: box_.top,
        w: (box_.right - box_.left) as u32,
        h: (box_.bottom - box_.top) as u32,
        maximised: true,
    })
}

#[cfg(not(windows))]
fn before_it_was_maximised(_window: &WebviewWindow) -> Option<Spot> {
    None
}

pub fn at_close(window: &WebviewWindow) -> Option<Spot> {
    let now = read(window)?;
    if !now.maximised {
        return Some(now);
    }
    before_it_was_maximised(window).or(Some(now))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(x: i32, y: i32, w: u32, h: u32) -> Spot {
        Spot {
            x,
            y,
            w,
            h,
            maximised: false,
        }
    }

    // the screen test needs a window, so this covers the arithmetic on its own
    fn reachable(spot: &Spot, screen: (i32, i32, i32, i32)) -> bool {
        let (left, top, right, bottom) = screen;
        spot.x + spot.w as i32 - GRABBABLE > left
            && spot.x + GRABBABLE < right
            && spot.y + 34 > top
            && spot.y < bottom
    }

    #[test]
    fn a_window_in_the_middle_of_the_screen_is_reachable() {
        assert!(reachable(&spot(200, 150, 1180, 760), (0, 0, 1920, 1080)));
    }

    #[test]
    fn one_hanging_off_the_side_is_still_reachable() {
        assert!(reachable(&spot(-400, 100, 1180, 760), (0, 0, 1920, 1080)));
        assert!(reachable(&spot(1800, 100, 1180, 760), (0, 0, 1920, 1080)));
    }

    #[test]
    fn one_left_on_a_monitor_that_has_gone_is_not() {
        assert!(!reachable(&spot(2600, 200, 1180, 760), (0, 0, 1920, 1080)));
        assert!(!reachable(&spot(-1400, 200, 1180, 760), (0, 0, 1920, 1080)));
    }

    #[test]
    fn one_dropped_below_the_bottom_is_not() {
        assert!(!reachable(&spot(300, 1400, 1180, 760), (0, 0, 1920, 1080)));
    }

    #[test]
    fn the_title_bar_only_has_to_clear_the_top_edge() {
        assert!(reachable(&spot(300, -20, 1180, 760), (0, 0, 1920, 1080)));
        assert!(!reachable(&spot(300, -60, 1180, 760), (0, 0, 1920, 1080)));
    }
}
