//! the key that brings the overlay up
//!
//! RegisterHotKey was the obvious answer and it is not enough. plenty of games
//! install a low level keyboard hook and swallow everything, which is why the
//! windows key stops working inside the witcher 2. hooks run before hotkeys,
//! so ours never fired. alt tab keeps working because it is handled in the
//! kernel before any of this, and there is no way to register into that.
//!
//! so this installs a low level hook of its own. they are called newest first,
//! which is the whole trick: reinstall ours after the game has installed its
//! one and we see the key before it does.
//!
//! a hook like this is handed every keystroke on the machine. this one
//! compares the key against one combination and returns. it keeps nothing,
//! writes nothing down and sends nothing anywhere, it is only installed while
//! the overlay is turned on, and it is fourteen lines you can read below.

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

// holds the hook for as long as it is alive
pub struct Listener {
    #[cfg(windows)]
    thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    watcher: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    stop: u32,
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN,
        WM_SYSKEYDOWN, WM_USER,
    };

    const QUIT: u32 = WM_USER + 1;

    // the modifiers in the low half, the key in the high half. the callback
    // must not take a lock, so what it needs to know lives in atomics
    static WANTED: AtomicU64 = AtomicU64::new(0);
    static HITS: AtomicU32 = AtomicU32::new(0);

    fn held() -> u32 {
        let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0;
        let mut modifiers = 0;
        if down(VK_CONTROL.0) {
            modifiers |= CONTROL;
        }
        if down(VK_SHIFT.0) {
            modifiers |= SHIFT;
        }
        if down(VK_MENU.0) {
            modifiers |= ALT;
        }
        if down(VK_LWIN.0) || down(VK_RWIN.0) {
            modifiers |= WIN;
        }
        modifiers
    }

    // windows drops a hook that dawdles, so this counts and returns. anything
    // that is not the one combination goes straight on to whoever is next,
    // untouched and unrecorded
    unsafe extern "system" fn watch(code: i32, what: WPARAM, carried: LPARAM) -> LRESULT {
        if code >= 0 && (what.0 as u32 == WM_KEYDOWN || what.0 as u32 == WM_SYSKEYDOWN) {
            let event = unsafe { &*(carried.0 as *const KBDLLHOOKSTRUCT) };
            let wanted = WANTED.load(Ordering::Relaxed);
            let key = (wanted >> 32) as u32;
            let modifiers = wanted as u32;

            if event.vkCode == key && held() == modifiers {
                HITS.fetch_add(1, Ordering::Relaxed);
                // ours, so the game does not see it as well
                return LRESULT(1);
            }
        }
        unsafe { CallNextHookEx(None, code, what, carried) }
    }

    pub fn listen(key: Hotkey, tell: Sender<()>) -> Result<Listener, String> {
        WANTED.store(
            ((key.key as u64) << 32) | key.modifiers as u64,
            Ordering::Relaxed,
        );
        let seen = HITS.load(Ordering::Relaxed);
        let (ready, done) = std::sync::mpsc::channel();

        // the hook is owned by the thread that installs it and is called on
        // that thread, which therefore needs a message loop of its own
        let thread = std::thread::spawn(move || {
            let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(watch), None, 0) };
            let Ok(hook) = hook else {
                let _ = ready.send(Err("windows would not let us watch for the key".to_string()));
                return;
            };

            let id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
            let _ = ready.send(Ok(id));

            let mut message = MSG::default();
            loop {
                let got = unsafe { GetMessageW(&mut message, None, 0, 0) };
                if got.0 <= 0 || message.message == QUIT {
                    break;
                }
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            unsafe {
                let _ = UnhookWindowsHookEx(hook);
            }
        });

        let id = match done.recv() {
            Ok(Ok(id)) => id,
            Ok(Err(why)) => return Err(why),
            Err(_) => return Err("could not start the key watcher".into()),
        };

        // the counter is turned back into something to wait on here, so the
        // hook itself never allocates or takes a lock
        let watcher = std::thread::spawn(move || {
            let mut last = seen;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(25));
                let now = HITS.load(Ordering::Relaxed);
                if now == last {
                    continue;
                }
                last = now;
                if tell.send(()).is_err() {
                    break;
                }
            }
        });

        Ok(Listener {
            thread: Some(thread),
            watcher: Some(watcher),
            stop: id,
        })
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
        // the watcher ends on its own when the sender goes with the listener
        listener.watcher.take();
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
