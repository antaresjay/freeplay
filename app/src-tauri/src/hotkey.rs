//! the key that brings the overlay up, registered with windows itself
//!
//! RegisterHotKey is system wide, so it fires while a game has focus, which is
//! the whole point. it also refuses when something else already holds the
//! combination, and that refusal is the only honest way to know a hotkey is
//! taken.

use std::sync::mpsc::Sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    // MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN
    pub modifiers: u32,
    pub key: u32,
}

pub const ALT: u32 = 0x0001;
pub const CONTROL: u32 = 0x0002;
pub const SHIFT: u32 = 0x0004;
pub const WIN: u32 = 0x0008;
const NO_REPEAT: u32 = 0x4000;

// nothing else on a gaming machine wants this one. the obvious picks are all
// spoken for, see `clash`
pub const DEFAULT: &str = "Ctrl+Shift+O";

pub fn parse(text: &str) -> Result<Hotkey, String> {
    let mut modifiers = 0u32;
    let mut key = None;

    for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= CONTROL,
            "shift" => modifiers |= SHIFT,
            "alt" => modifiers |= ALT,
            "win" | "super" | "meta" => modifiers |= WIN,
            other => {
                if key.is_some() {
                    return Err(format!("{text:?} names more than one key"));
                }
                key = Some(code_for(other).ok_or_else(|| format!("no key called {other:?}"))?);
            }
        }
    }

    let key = key.ok_or_else(|| format!("{text:?} is only modifiers"))?;
    if modifiers == 0 {
        // a bare letter would fire every time you typed it, in every program
        return Err("hold ctrl, alt or shift as well, or it fires while you type".into());
    }
    Ok(Hotkey { modifiers, key })
}

fn code_for(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    if bytes.len() == 1 {
        let c = bytes[0].to_ascii_uppercase();
        if c.is_ascii_alphanumeric() {
            return Some(c as u32);
        }
    }
    if let Some(number) = name.strip_prefix('f') {
        if let Ok(n) = number.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(0x6F + n);
            }
        }
    }
    Some(match name {
        "space" => 0x20,
        "tab" => 0x09,
        "enter" | "return" => 0x0D,
        "backspace" => 0x08,
        "insert" => 0x2D,
        "delete" => 0x2E,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "up" => 0x26,
        "down" => 0x28,
        "left" => 0x25,
        "right" => 0x27,
        "`" | "backquote" => 0xC0,
        "-" | "minus" => 0xBD,
        "=" | "equal" => 0xBB,
        "[" => 0xDB,
        "]" => 0xDD,
        "\\" => 0xDC,
        ";" => 0xBA,
        "'" => 0xDE,
        "," => 0xBC,
        "." => 0xBE,
        "/" => 0xBF,
        _ => return None,
    })
}

pub fn spell(key: Hotkey) -> String {
    let mut parts = Vec::new();
    if key.modifiers & CONTROL != 0 {
        parts.push("Ctrl".to_string());
    }
    if key.modifiers & ALT != 0 {
        parts.push("Alt".to_string());
    }
    if key.modifiers & SHIFT != 0 {
        parts.push("Shift".to_string());
    }
    if key.modifiers & WIN != 0 {
        parts.push("Win".to_string());
    }
    parts.push(name_of(key.key));
    parts.join("+")
}

fn name_of(code: u32) -> String {
    match code {
        0x30..=0x39 | 0x41..=0x5A => ((code as u8) as char).to_string(),
        0x70..=0x87 => format!("F{}", code - 0x6F),
        0x20 => "Space".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x08 => "Backspace".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x26 => "Up".into(),
        0x28 => "Down".into(),
        0x25 => "Left".into(),
        0x27 => "Right".into(),
        0xC0 => "`".into(),
        other => format!("key {other:#x}"),
    }
}

// whatever well known thing already uses this combination
//
// the graphics vendors hook the keyboard below the level RegisterHotKey works
// at, so they take a key without ever showing up as a clash. knowing them by
// name is the only way to warn about those
pub fn clash(text: &str) -> Option<&'static str> {
    let Ok(key) = parse(text) else { return None };
    let spelled = spell(key).to_ascii_lowercase();

    let known: &[(&str, &str)] = &[
        ("alt+z", "the NVIDIA overlay"),
        ("alt+r", "NVIDIA and AMD instant replay"),
        ("alt+f1", "NVIDIA screenshots"),
        ("alt+f9", "NVIDIA recording"),
        ("alt+f10", "NVIDIA instant replay"),
        ("alt+f12", "the NVIDIA frame counter"),
        ("ctrl+shift+f1", "Intel Arc Control"),
        ("shift+tab", "the Steam overlay"),
        ("f12", "Steam screenshots"),
        ("win+g", "the Xbox Game Bar"),
        ("win+alt+r", "Xbox Game Bar recording"),
        ("shift+`", "the Discord overlay"),
        ("ctrl+f2", "MSI Afterburner"),
    ];

    known
        .iter()
        .find(|(combination, _)| *combination == spelled)
        .map(|(_, who)| *who)
}

// holds the registration for as long as it is alive
pub struct Listener {
    #[cfg(windows)]
    thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    stop: u32,
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, HOT_KEY_MODIFIERS};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, MSG, WM_HOTKEY,
        WM_USER,
    };

    const ID: i32 = 0xF7E9;
    const QUIT: u32 = WM_USER + 1;

    // the hotkey belongs to whichever thread registered it, and its messages
    // land in that thread's queue rather than any window, so this needs a
    // thread of its own with a message loop
    pub fn listen(key: Hotkey, tell: Sender<()>) -> Result<Listener, String> {
        let (ready, done) = std::sync::mpsc::channel();

        let thread = std::thread::spawn(move || {
            let registered = unsafe {
                RegisterHotKey(
                    Some(HWND::default()),
                    ID,
                    HOT_KEY_MODIFIERS(key.modifiers | NO_REPEAT),
                    key.key,
                )
            };

            if registered.is_err() {
                let _ = ready.send(Err(
                    "another program is already using that combination".to_string()
                ));
                return;
            }

            // the loop below only ends when this thread is posted to, so its
            // id has to travel back out
            let id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
            let _ = ready.send(Ok(id));

            let mut message = MSG::default();
            loop {
                let got = unsafe { GetMessageW(&mut message, None, 0, 0) };
                if got.0 <= 0 {
                    break;
                }
                if message.message == QUIT {
                    break;
                }
                if message.message == WM_HOTKEY && tell.send(()).is_err() {
                    break;
                }
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        });

        match done.recv() {
            Ok(Ok(id)) => Ok(Listener {
                thread: Some(thread),
                stop: id,
            }),
            Ok(Err(why)) => Err(why),
            Err(_) => Err("could not start the hotkey watcher".into()),
        }
    }

    pub fn quit(listener: &mut Listener) {
        if listener.stop != 0 {
            unsafe {
                let _ = PostThreadMessageW(listener.stop, QUIT, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(thread) = listener.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(windows)]
pub fn listen(key: Hotkey, tell: Sender<()>) -> Result<Listener, String> {
    win::listen(key, tell)
}

#[cfg(windows)]
impl Drop for Listener {
    fn drop(&mut self) {
        win::quit(self);
    }
}

#[cfg(not(windows))]
pub fn listen(_key: Hotkey, _tell: Sender<()>) -> Result<Listener, String> {
    Err("hotkeys are windows only for now".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_default() {
        let key = parse(DEFAULT).unwrap();
        assert_eq!(key.modifiers, CONTROL | SHIFT);
        assert_eq!(key.key, b'O' as u32);
    }

    #[test]
    fn the_order_of_the_parts_does_not_matter() {
        assert_eq!(parse("Ctrl+Shift+O"), parse("shift + ctrl + o"));
    }

    #[test]
    fn function_keys_and_named_keys_work() {
        assert_eq!(parse("Alt+F4").unwrap().key, 0x73);
        assert_eq!(parse("Ctrl+F12").unwrap().key, 0x7B);
        assert_eq!(parse("Ctrl+Space").unwrap().key, 0x20);
        assert_eq!(parse("Ctrl+`").unwrap().key, 0xC0);
    }

    // a hotkey with no modifier fires while you are typing your name into a
    // text box, in every program at once
    #[test]
    fn a_bare_key_is_refused() {
        let why = parse("O").unwrap_err();
        assert!(why.contains("ctrl, alt or shift"), "{why}");
    }

    #[test]
    fn modifiers_on_their_own_are_refused() {
        assert!(parse("Ctrl+Shift").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn two_keys_are_refused() {
        assert!(parse("Ctrl+O+P").is_err());
    }

    #[test]
    fn a_key_nobody_has_is_refused() {
        let why = parse("Ctrl+Wibble").unwrap_err();
        assert!(why.contains("wibble"), "{why}");
    }

    #[test]
    fn spelling_it_out_round_trips() {
        for text in [
            "Ctrl+Shift+O",
            "Alt+F4",
            "Ctrl+Alt+Shift+Space",
            "Win+Ctrl+`",
        ] {
            let key = parse(text).unwrap();
            assert_eq!(parse(&spell(key)).unwrap(), key, "{text}");
        }
    }

    #[test]
    fn it_is_always_spelled_the_same_way() {
        assert_eq!(spell(parse("shift+ctrl+o").unwrap()), "Ctrl+Shift+O");
    }

    // the point of the default
    #[test]
    fn the_default_treads_on_nobody() {
        assert_eq!(clash(DEFAULT), None);
    }

    #[test]
    fn the_obvious_choices_are_all_taken() {
        assert_eq!(clash("Alt+Z"), Some("the NVIDIA overlay"));
        assert_eq!(clash("Shift+Tab"), Some("the Steam overlay"));
        assert_eq!(clash("Win+G"), Some("the Xbox Game Bar"));
        assert_eq!(clash("Alt+R"), Some("NVIDIA and AMD instant replay"));
    }

    #[test]
    fn a_clash_is_found_however_it_was_typed() {
        assert_eq!(clash("z+alt"), Some("the NVIDIA overlay"));
    }

    #[test]
    fn rubbish_is_not_a_clash_it_is_just_rubbish() {
        assert_eq!(clash("not a hotkey"), None);
    }
}
