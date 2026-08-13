//! Starting a game the way its store expects.
//!
//! Going through `steam://` rather than the executable matters. Plenty of
//! games will not run unless the client has set them up first, and the ones
//! that do run will not count play time or see your cloud saves.

use std::path::PathBuf;

use crate::{InstalledGame, Store};

/// What starting a game will actually do. Worked out separately from doing it
/// so the decision can be tested without launching anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// Hand a url to the shell and let the client take it from there.
    Url(String),
    /// Run the executable directly, for stores with no url scheme worth using.
    Exe(PathBuf),
}

pub fn plan(game: &InstalledGame) -> Option<Launch> {
    match (game.store, game.app_id.as_deref()) {
        // This ends up going to the shell, so it has to be what it claims.
        (Store::Steam, Some(id)) if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => {
            Some(Launch::Url(format!("steam://rungameid/{id}")))
        }
        (Store::Epic, Some(id)) => epic_url(id).map(Launch::Url),
        _ => None,
    }
    .or_else(|| game.executables.first().cloned().map(Launch::Exe))
}

// namespace:catalogitem:appname, with the colons escaped again on the way
// out. silent stops the launcher throwing its own window in front of the game
fn epic_url(id: &str) -> Option<String> {
    let parts: Vec<&str> = id.split(':').collect();
    let [ns, item, app] = parts[..] else {
        return None;
    };
    let sane = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric());
    if !sane(ns) || !sane(item) || !sane(app) {
        return None;
    }
    Some(format!(
        "com.epicgames.launcher://apps/{ns}%3A{item}%3A{app}?action=launch&silent=true"
    ))
}

/// Starts the game and returns what it did, so a caller can say so.
pub fn start(game: &InstalledGame) -> Result<Launch, String> {
    let plan = plan(game).ok_or("there is no executable to start for this one")?;

    match &plan {
        Launch::Url(url) => open(url)?,
        Launch::Exe(exe) => {
            std::process::Command::new(exe)
                .current_dir(exe.parent().unwrap_or(&game.install_dir))
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
        }
    }
    Ok(plan)
}

/// `cmd /C start` used to do this, which meant a console window flashing up
/// and no way to tell a failure from a success. ShellExecute is the call the
/// shell actually uses and it says what went wrong.
///
/// It runs on a thread of its own that initialises COM as a single threaded
/// apartment first. ShellExecute goes through the shell, the shell wants an
/// STA, and the thread this gets called on is whatever the caller happened to
/// be using: in the desktop app that is a tokio worker with no COM at all,
/// where the same call that works from the command line does nothing.
#[cfg(windows)]
fn open(url: &str) -> Result<(), String> {
    let url = url.to_string();
    std::thread::spawn(move || open_here(&url))
        .join()
        .map_err(|_| "the thread that starts games panicked".to_string())?
}

#[cfg(windows)]
fn open_here(url: &str) -> Result<(), String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    // Already initialised is fine, it just means somebody got here first.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let verb = HSTRING::from("open");
    let target = HSTRING::from(url);
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    let code = result.0 as usize;
    let scheme = url.split_once(':').map(|(s, _)| s).unwrap_or("these");

    if com.is_ok() {
        unsafe { CoUninitialize() };
    }

    // Anything at or below 32 is an error code rather than a handle.
    match code {
        c if c > 32 => Ok(()),
        2 | 3 => Err(format!(
            "the client that handles {scheme} urls is not installed"
        )),
        31 => Err(format!(
            "Windows has nothing registered to open {scheme} urls"
        )),
        other => Err(format!(
            "Windows would not open it, ShellExecute gave {other}"
        )),
    }
}

#[cfg(not(windows))]
fn open(url: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Hand a file to whatever normally opens it.
pub fn show(path: &std::path::Path) -> Result<(), String> {
    open(&path.display().to_string())
}

/// Hand a url to the browser. Nothing here fetches anything, the shell does.
pub fn show_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("only https links are opened".into());
    }
    open(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(store: Store, app_id: Option<&str>, exes: &[&str]) -> InstalledGame {
        InstalledGame {
            name: "A Game".into(),
            store,
            install_dir: PathBuf::from(r"C:\games\a game"),
            app_id: app_id.map(str::to_string),
            executables: exes.iter().map(PathBuf::from).collect(),
            version: None,
        }
    }

    #[test]
    fn a_steam_game_goes_through_the_client() {
        let plan = plan(&game(Store::Steam, Some("20920"), &[r"C:\g\witcher2.exe"]));
        assert_eq!(plan, Some(Launch::Url("steam://rungameid/20920".into())));
    }

    #[test]
    fn other_stores_run_the_executable() {
        let plan = plan(&game(Store::Gog, Some("1207658930"), &[r"C:\g\game.exe"]));
        assert_eq!(plan, Some(Launch::Exe(PathBuf::from(r"C:\g\game.exe"))));
    }

    // straight off the manifest for tomb raider goty
    #[test]
    fn an_epic_game_goes_through_the_launcher() {
        let id = "caca23a0954f4c1aba1fdd7e277b81e2:\
                  ff45e0eabd0c48d6950e369c79c26823:\
                  d6264d56f5ba434e91d4b0a0b056c83a";
        let plan = plan(&game(Store::Epic, Some(id), &[r"C:\g\TombRaider.exe"]));
        assert_eq!(
            plan,
            Some(Launch::Url(
                "com.epicgames.launcher://apps/\
                 caca23a0954f4c1aba1fdd7e277b81e2%3A\
                 ff45e0eabd0c48d6950e369c79c26823%3A\
                 d6264d56f5ba434e91d4b0a0b056c83a?action=launch&silent=true"
                    .into()
            ))
        );
    }

    #[test]
    fn a_half_written_epic_manifest_runs_the_executable() {
        for bad in [
            "",
            "onlyanappname",
            "ns:item",
            "ns:item:app:extra",
            "ns::app",
            "ns:item:app name",
            "ns:item:app&calc",
            "../..:x:y",
        ] {
            let plan = plan(&game(Store::Epic, Some(bad), &[r"C:\g\game.exe"]));
            assert_eq!(
                plan,
                Some(Launch::Exe(PathBuf::from(r"C:\g\game.exe"))),
                "{bad} should not have reached the shell"
            );
        }
    }

    /// An app id is pasted straight into a url handed to the shell, so a junk
    /// one falls back to the executable rather than being trusted.
    #[test]
    fn a_dodgy_app_id_never_reaches_the_shell() {
        for bad in ["", "20920 & calc", "../../evil", "abc"] {
            let plan = plan(&game(Store::Steam, Some(bad), &[r"C:\g\game.exe"]));
            assert_eq!(plan, Some(Launch::Exe(PathBuf::from(r"C:\g\game.exe"))));
        }
    }

    #[test]
    fn a_steam_game_with_no_app_id_still_runs() {
        let plan = plan(&game(Store::Steam, None, &[r"C:\g\game.exe"]));
        assert_eq!(plan, Some(Launch::Exe(PathBuf::from(r"C:\g\game.exe"))));
    }

    #[test]
    fn nothing_to_run_is_nothing_to_plan() {
        assert_eq!(plan(&game(Store::Gog, None, &[])), None);
    }
}
