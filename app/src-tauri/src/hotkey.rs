//! the key that brings the overlay up
//!
//! RegisterHotKey was the obvious answer and it is not enough. plenty of games
//! install a low level keyboard hook and swallow everything, which is why the
//! windows key stops working in some of them. hooks run before hotkeys are
//! looked at, so ours never fired. alt tab keeps working because it is handled
//! in the kernel before any of this, and there is no way to register into
//! that.
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
    read(text, false)
}

// same words, but a bare F1 is allowed. cheat keys only count while the game
// is in front, so a key on its own cannot fire while you type elsewhere
pub fn parse_loose(text: &str) -> Result<Hotkey, String> {
    read(text, true)
}

fn read(text: &str, bare_ok: bool) -> Result<Hotkey, String> {
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
    if modifiers == 0 && !bare_ok {
        // a bare letter would fire every time you typed it, in every program
        return Err("hold ctrl, alt or shift as well, or it fires while you type".into());
    }
    Ok(Hotkey { modifiers, key })
}

// a cheat engine key list is virtual key codes with the modifiers mixed in.
// pull those out, and what is left had better be exactly one key
pub fn from_vks(codes: &[u32]) -> Option<Hotkey> {
    let mut modifiers = 0u32;
    let mut key = None;
    for &code in codes {
        match code {
            0x10 | 0xA0 | 0xA1 => modifiers |= SHIFT,
            0x11 | 0xA2 | 0xA3 => modifiers |= CONTROL,
            0x12 | 0xA4 | 0xA5 => modifiers |= ALT,
            0x5B | 0x5C => modifiers |= WIN,
            other => {
                if key.replace(other).is_some() {
                    // a two key chord, which nothing here plays
                    return None;
                }
            }
        }
    }
    Some(Hotkey {
        modifiers,
        key: key?,
    })
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
    if let Some(number) = name.strip_prefix("num") {
        if let Ok(n) = number.parse::<u32>() {
            if n <= 9 {
                return Some(0x60 + n);
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
        "pause" => 0x13,
        "num*" => 0x6A,
        // spelled without the sign, or parse eats it as a separator
        "numplus" => 0x6B,
        "num-" => 0x6D,
        "num." => 0x6E,
        "num/" => 0x6F,
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
        0x60..=0x69 => format!("Num{}", code - 0x60),
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
        0x13 => "Pause".into(),
        0x6A => "Num*".into(),
        0x6B => "NumPlus".into(),
        0x6D => "Num-".into(),
        0x6E => "Num.".into(),
        0x6F => "Num/".into(),
        0xBD => "-".into(),
        0xBB => "=".into(),
        0xDB => "[".into(),
        0xDD => "]".into(),
        0xDC => "\\".into(),
        0xBA => ";".into(),
        0xDE => "'".into(),
        0xBC => ",".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
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
    // loose, so a bare F12 still gets called what it is: the steam screenshot
    let Ok(key) = parse_loose(text) else {
        return None;
    };
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
    // dropping a join handle does not stop the thread behind it, and this one
    // polls a counter shared with every other, so a leaked one fires the
    // shortcut a second time
    #[cfg(windows)]
    halt: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(windows)]
    stop: u32,
}

// every key a table binds, watched at once. same hook trick as the overlay
// key, but a row of slots instead of one combination. there is one bank at a
// time, made when a game is grabbed and dropped when it goes
pub const SLOTS: usize = 128;

pub struct Bank {
    #[cfg(windows)]
    thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    watcher: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    halt: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(windows)]
    stop: u32,
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::Arc;

    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN,
        WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_USER,
    };

    const QUIT: u32 = WM_USER + 1;

    // the modifiers in the low half, the key in the high half. the callback
    // must not take a lock, so what it needs to know lives in atomics
    static WANTED: AtomicU64 = AtomicU64::new(0);
    static HITS: AtomicU32 = AtomicU32::new(0);
    // holding a key repeats keydown, and every repeat used to count as another
    // press
    static DOWN: AtomicBool = AtomicBool::new(false);

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
        if code >= 0 {
            let message = what.0 as u32;
            let event = unsafe { &*(carried.0 as *const KBDLLHOOKSTRUCT) };
            let wanted = WANTED.load(Ordering::Relaxed);

            if event.vkCode == (wanted >> 32) as u32 {
                if message == WM_KEYUP || message == WM_SYSKEYUP {
                    DOWN.store(false, Ordering::Relaxed);
                } else if message == WM_KEYDOWN || message == WM_SYSKEYDOWN {
                    let ours = held() == wanted as u32;
                    // one press is one toggle, however long it is held
                    let repeat = DOWN.swap(true, Ordering::Relaxed);
                    if ours && !repeat {
                        HITS.fetch_add(1, Ordering::Relaxed);
                    }
                    if ours {
                        // ours, so the game never sees it
                        return LRESULT(1);
                    }
                }
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
        let halt = Arc::new(AtomicBool::new(false));
        let mine = Arc::clone(&halt);
        let watcher = std::thread::spawn(move || {
            let mut last = seen;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(25));
                if mine.load(Ordering::Relaxed) {
                    break;
                }
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
            halt,
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
        // and this one has to be told. dropping its handle leaves it running,
        // polling the same counter the next one polls, so every rebind used to
        // add another toggle to a single press
        listener.halt.store(true, Ordering::Relaxed);
        if let Some(watcher) = listener.watcher.take() {
            let _ = watcher.join();
        }
    }

    // the bank. combos packed the same way, one atomic per slot so the hook
    // never takes a lock
    static COMBOS: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
    static TAPS: [AtomicU32; SLOTS] = [const { AtomicU32::new(0) }; SLOTS];
    static HELDS: [AtomicBool; SLOTS] = [const { AtomicBool::new(false) }; SLOTS];
    // cheat keys only count with the game or freeplay in front. F1 in a
    // browser must never toggle god mode
    static GATE: AtomicU32 = AtomicU32::new(0);

    fn in_front(gate: u32) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
        pid != 0 && (pid == gate || pid == std::process::id())
    }

    unsafe extern "system" fn watch_bank(code: i32, what: WPARAM, carried: LPARAM) -> LRESULT {
        if code >= 0 {
            let message = what.0 as u32;
            let event = unsafe { &*(carried.0 as *const KBDLLHOOKSTRUCT) };
            let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
            let up = message == WM_KEYUP || message == WM_SYSKEYUP;
            let vk = event.vkCode;
            // a modifier is never the key half of a combination
            if (down || up) && !matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5) {
                let mods = held();
                for (i, slot) in COMBOS.iter().enumerate() {
                    let packed = slot.load(Ordering::Relaxed);
                    if packed == 0 || (packed >> 32) as u32 != vk {
                        continue;
                    }
                    if up {
                        HELDS[i].store(false, Ordering::Relaxed);
                    } else if packed as u32 == mods {
                        let repeat = HELDS[i].swap(true, Ordering::Relaxed);
                        if !repeat && in_front(GATE.load(Ordering::Relaxed)) {
                            TAPS[i].fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        }
        // the game sees the key either way. eating F1 would steal a key the
        // player may have bound in the game itself
        unsafe { CallNextHookEx(None, code, what, carried) }
    }

    pub fn bank(keys: &[Hotkey], gate: u32, tell: Sender<usize>) -> Result<Bank, String> {
        for (i, slot) in COMBOS.iter().enumerate() {
            HELDS[i].store(false, Ordering::Relaxed);
            let packed = keys
                .get(i)
                .map(|k| ((k.key as u64) << 32) | k.modifiers as u64)
                .unwrap_or(0);
            slot.store(packed, Ordering::Relaxed);
        }
        GATE.store(gate, Ordering::Relaxed);

        let (ready, done) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(watch_bank), None, 0) };
            let Ok(hook) = hook else {
                let _ = ready.send(Err("windows would not let us watch the keys".to_string()));
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

        let halt = Arc::new(AtomicBool::new(false));
        let mine = Arc::clone(&halt);
        let watcher = std::thread::spawn(move || {
            let mut last: Vec<u32> = TAPS.iter().map(|t| t.load(Ordering::Relaxed)).collect();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(25));
                if mine.load(Ordering::Relaxed) {
                    break;
                }
                for (i, tap) in TAPS.iter().enumerate() {
                    let now = tap.load(Ordering::Relaxed);
                    if now == last[i] {
                        continue;
                    }
                    last[i] = now;
                    if tell.send(i).is_err() {
                        return;
                    }
                }
            }
        });

        Ok(Bank {
            thread: Some(thread),
            watcher: Some(watcher),
            halt,
            stop: id,
        })
    }

    pub fn quit_bank(bank: &mut Bank) {
        // slots first, so a keystroke between here and the unhook hits nothing
        for slot in COMBOS.iter() {
            slot.store(0, Ordering::Relaxed);
        }
        GATE.store(0, Ordering::Relaxed);

        if bank.stop != 0 {
            unsafe {
                let _ = PostThreadMessageW(bank.stop, QUIT, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(thread) = bank.thread.take() {
            let _ = thread.join();
        }
        bank.halt.store(true, Ordering::Relaxed);
        if let Some(watcher) = bank.watcher.take() {
            let _ = watcher.join();
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

// which slot fired comes back over the channel as its index in `keys`
#[cfg(windows)]
pub fn bank(keys: &[Hotkey], gate: u32, tell: Sender<usize>) -> Result<Bank, String> {
    win::bank(keys, gate, tell)
}

#[cfg(windows)]
impl Drop for Bank {
    fn drop(&mut self) {
        win::quit_bank(self);
    }
}

#[cfg(not(windows))]
pub fn bank(_keys: &[Hotkey], _gate: u32, _tell: Sender<usize>) -> Result<Bank, String> {
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

    // the shape a cheat engine table writes: modifiers as codes in the list
    #[test]
    fn a_key_list_folds_its_modifiers() {
        let key = from_vks(&[162, 112]).unwrap();
        assert_eq!(key.modifiers, CONTROL);
        assert_eq!(key.key, 0x70);
        assert_eq!(spell(key), "Ctrl+F1");
    }

    #[test]
    fn a_bare_f_key_is_fine_for_a_cheat() {
        let key = from_vks(&[112]).unwrap();
        assert_eq!(key.modifiers, 0);
        assert_eq!(spell(key), "F1");
        assert_eq!(parse_loose("F1").unwrap(), key);
    }

    #[test]
    fn a_two_key_chord_is_not_pretended_at() {
        assert_eq!(from_vks(&[49, 50]), None);
        assert_eq!(from_vks(&[]), None);
        assert_eq!(from_vks(&[162]), None, "only modifiers is not a key");
    }

    #[test]
    fn the_overlay_key_still_refuses_a_bare_letter() {
        assert!(parse("O").is_err());
        assert!(parse_loose("O").is_ok());
    }

    #[test]
    fn numpad_keys_spell_and_read_back() {
        for code in [0x60, 0x69, 0x6A, 0x6B, 0x6D] {
            let key = Hotkey {
                modifiers: 0,
                key: code,
            };
            assert_eq!(parse_loose(&spell(key)).unwrap(), key, "{code:#x}");
        }
    }
}
