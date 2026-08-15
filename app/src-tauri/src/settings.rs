//! one small json file. a database would be overkill and you can open this
//! one in notepad, which suits an app whose whole point is being readable

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // system, dark or light
    pub theme: String,
    pub accent: String,
    // game keys, in the order they were starred
    pub favourites: Vec<String>,
    // pinning was a second way of doing what starring does, one shelf lower.
    // read so an older settings file still opens, folded into favourites by
    // tidy, and never written again
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<String>,
    // fetch published tables on start
    #[serde(default = "yes")]
    pub auto_update: bool,
    // ask the service what other people have shared. this one is separate
    // because it fires whenever a game page is opened rather than on a button,
    // and with both of these off freeplay opens no sockets at all
    #[serde(default = "yes")]
    pub community: bool,
    // attach on our own when a game with a table starts
    #[serde(default = "yes")]
    pub auto_attach: bool,
    // what is switched on, per exe. comes back next launch and engages once
    // the game is far enough in to allow it
    #[serde(default)]
    pub armed: HashMap<String, Vec<String>>,
    // numbers typed into a cheat, per exe then per cheat id. kept as text so a
    // float that was typed as 1.5 comes back as 1.5
    #[serde(default)]
    pub values: HashMap<String, HashMap<String, String>>,
    // the shared tables panel on the game page, open or folded away
    #[serde(default = "yes")]
    pub shared_open: bool,
    /* tables switched off, per exe, by tag. this used to be the other way
    round, a list of the ones to use, and anything installed afterwards was
    not in it and silently produced no cheats. off by exception means a new
    table works the moment it lands */
    #[serde(default)]
    pub off: HashMap<String, Vec<String>>,
    // categories folded away, per exe. open is the default, so this only ever
    // holds the ones somebody shut. the overlay reads the same list
    #[serde(default)]
    pub folded: HashMap<String, Vec<String>>,
    // a panel over the game, brought up by a key while you are playing
    #[serde(default)]
    pub overlay: bool,
    #[serde(default = "default_hotkey")]
    pub overlay_key: String,
    // keys rebound by hand, per exe then per cheat id. "" means the table's
    // own key was switched off
    #[serde(default)]
    pub keys: HashMap<String, HashMap<String, String>>,
    /* cheats that were switched on moments before the game went down, per exe
    then per cheat id, unix seconds of the crash. shown as a warning on the
    card. a cheat that later stays on without incident is taken off the list */
    #[serde(default)]
    pub crashed: HashMap<String, HashMap<String, i64>>,
    // one key that switches every cheat off at once
    #[serde(default = "default_panic")]
    pub panic: String,
    // a short beep when a cheat key lands, so you know without alt tabbing
    #[serde(default = "yes")]
    pub chirp: bool,
    // random, made once. it stops one person voting twice and is not tied to
    // the machine or to any name
    #[serde(default)]
    pub install_id: String,
    // tables pulled from the service, so the picker can show which you have
    #[serde(default)]
    pub grabbed: HashMap<String, i64>,
    // games already asked whether the table worked, so it is asked once
    #[serde(default)]
    pub rated: Vec<i64>,
    // tables that were actually in play, waiting to be asked about. asking
    // while the game is up means almost nobody sees it
    #[serde(default)]
    pub played: Vec<Played>,
    // skipped once, so leave them alone until this passes. unix seconds
    #[serde(default)]
    pub ask_again_at: i64,
    // where the window was when it was last closed
    #[serde(default)]
    pub window: Option<Spot>,
}

// physical pixels, which is what tauri hands back and takes, so a window put
// down on a scaled monitor comes back the same size it was
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    #[serde(default)]
    pub maximised: bool,
}

// one table, one sitting. what it takes to ask a question somebody can answer
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Played {
    pub id: i64,
    pub exe: String,
    pub game: String,
    pub by: String,
    // how long the game was attached, in seconds
    #[serde(default)]
    pub seconds: u64,
    #[serde(default)]
    pub cheats: usize,
    // when the sitting ended
    #[serde(default)]
    pub at: i64,
}

// two days, which is long enough not to nag and short enough that the answer
// still means something for a table people are downloading now
pub const SNOOZE: i64 = 60 * 60 * 24 * 2;
// a table has to have been in play this long before the question is worth
// asking. launching a game and quitting to the menu tells nobody anything
pub const ENOUGH: u64 = 120;
// how many to keep waiting. more than this and it is a chore, not a favour
pub const KEEP: usize = 5;

fn yes() -> bool {
    true
}

fn default_hotkey() -> String {
    crate::hotkey::DEFAULT.to_string()
}

// backspace with both hands on it is nobody's screenshot key, see `clash`
pub const PANIC: &str = "Ctrl+Shift+Backspace";

fn default_panic() -> String {
    PANIC.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            accent: "amber".into(),
            pinned: Vec::new(),
            favourites: Vec::new(),
            auto_update: true,
            community: true,
            auto_attach: true,
            armed: HashMap::new(),
            values: HashMap::new(),
            shared_open: true,
            off: HashMap::new(),
            folded: HashMap::new(),
            overlay: false,
            overlay_key: default_hotkey(),
            keys: HashMap::new(),
            crashed: HashMap::new(),
            panic: default_panic(),
            chirp: true,
            install_id: String::new(),
            grabbed: HashMap::new(),
            rated: Vec::new(),
            played: Vec::new(),
            ask_again_at: 0,
            window: None,
        }
    }
}

impl Settings {
    // keeps junk out, so a hand edit cannot leave the ui pointing at a theme
    // that does not exist
    pub fn tidy(&mut self) {
        if !["system", "dark", "light"].contains(&self.theme.as_str()) {
            self.theme = "system".into();
        }
        if !["amber", "violet", "cyan", "rose", "lime"].contains(&self.accent.as_str()) {
            self.accent = "amber".into();
        }
        // anything that was pinned becomes a favourite rather than vanishing
        for key in std::mem::take(&mut self.pinned) {
            if !self.favourites.contains(&key) {
                self.favourites.push(key);
            }
        }
        self.favourites.dedup();

        // never ask about one twice, and never let the queue become a chore
        self.played.retain(|p| !self.rated.contains(&p.id));
        self.played.dedup_by_key(|p| p.id);
        if self.played.len() > KEEP {
            let drop = self.played.len() - KEEP;
            self.played.drain(..drop);
        }

        // a hand edited hotkey that does not parse would leave the overlay
        // with no way to open it
        if crate::hotkey::parse(&self.overlay_key).is_err() {
            self.overlay_key = default_hotkey();
        }

        // "" is a key switched off on purpose and stays. gibberish goes
        if !self.panic.is_empty() && crate::hotkey::parse_loose(&self.panic).is_err() {
            self.panic = default_panic();
        }
        for bound in self.keys.values_mut() {
            bound.retain(|_, key| key.is_empty() || crate::hotkey::parse_loose(key).is_ok());
        }
        self.keys.retain(|_, bound| !bound.is_empty());

        if self.install_id.len() != 32 || !self.install_id.chars().all(|c| c.is_ascii_hexdigit()) {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(1)
                ^ (std::process::id() as u128) << 64;
            self.install_id = freeplay_sync::community::new_install_id(seed);
        }
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| PathBuf::from("."))
        });
    base.join("freeplay").join("settings.json")
}

pub fn load() -> Settings {
    let mut settings: Settings = std::fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    settings.tidy();
    settings
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let file = path();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&file, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn played(id: i64) -> Played {
        Played {
            id,
            exe: "witcher2.exe".into(),
            game: "The Witcher 2".into(),
            by: "aSwedishMagyar".into(),
            seconds: 3600,
            cheats: 3,
            at: 1_800_000_000,
        }
    }

    #[test]
    fn a_table_already_answered_for_is_dropped_from_the_queue() {
        let mut settings = Settings {
            played: vec![played(1), played(2)],
            rated: vec![1],
            ..Default::default()
        };
        settings.tidy();
        assert_eq!(settings.played.len(), 1);
        assert_eq!(settings.played[0].id, 2);
    }

    #[test]
    fn the_same_table_twice_is_asked_about_once() {
        let mut settings = Settings {
            played: vec![played(7), played(7)],
            ..Default::default()
        };
        settings.tidy();
        assert_eq!(settings.played.len(), 1);
    }

    #[test]
    fn the_queue_keeps_the_most_recent_and_lets_the_rest_go() {
        let mut settings = Settings {
            played: (1..=9).map(played).collect(),
            ..Default::default()
        };
        settings.tidy();
        assert_eq!(settings.played.len(), KEEP);
        assert_eq!(settings.played[0].id, 5, "the oldest go first");
        assert_eq!(settings.played[KEEP - 1].id, 9);
    }

    #[test]
    fn nothing_is_waiting_to_be_asked_about_on_a_fresh_install() {
        let settings = Settings::default();
        assert!(settings.played.is_empty());
        assert_eq!(settings.ask_again_at, 0);
    }

    #[test]
    fn an_install_id_is_made_once_and_then_kept() {
        let mut settings = Settings::default();
        settings.tidy();
        let first = settings.install_id.clone();

        assert_eq!(first.len(), 32);
        settings.tidy();
        assert_eq!(settings.install_id, first, "it must not change under you");
    }

    #[test]
    fn a_junk_install_id_is_replaced() {
        let mut settings = Settings {
            install_id: "not hex".into(),
            ..Default::default()
        };
        settings.tidy();
        assert_eq!(settings.install_id.len(), 32);
    }

    #[test]
    fn a_hotkey_nobody_can_press_is_replaced() {
        let mut settings = Settings {
            overlay_key: "Wibble+Wobble".into(),
            ..Default::default()
        };
        settings.tidy();
        assert_eq!(settings.overlay_key, crate::hotkey::DEFAULT);
    }

    #[test]
    fn a_hotkey_that_works_is_left_alone() {
        let mut settings = Settings {
            overlay_key: "Alt+F8".into(),
            ..Default::default()
        };
        settings.tidy();
        assert_eq!(settings.overlay_key, "Alt+F8");
    }

    #[test]
    fn a_scrambled_cheat_key_goes_and_an_off_one_stays() {
        let mut settings = Settings::default();
        let mut bound = HashMap::new();
        bound.insert("god-mode".to_string(), "F3".to_string());
        bound.insert("orens".to_string(), String::new());
        bound.insert("vigor".to_string(), "Wibble+Wobble".to_string());
        settings.keys.insert("witcher2.exe".into(), bound);
        settings.tidy();

        let kept = &settings.keys["witcher2.exe"];
        assert_eq!(kept.len(), 2);
        assert_eq!(kept["god-mode"], "F3");
        assert_eq!(kept["orens"], "");
    }

    #[test]
    fn the_panic_key_can_be_cleared_but_not_scrambled() {
        let mut settings = Settings {
            panic: String::new(),
            ..Default::default()
        };
        settings.tidy();
        assert_eq!(settings.panic, "", "cleared on purpose stays cleared");

        settings.panic = "Wobble".into();
        settings.tidy();
        assert_eq!(settings.panic, PANIC);
    }

    #[test]
    fn the_overlay_is_off_until_somebody_asks_for_it() {
        assert!(!Settings::default().overlay);
    }

    // the only two switches between freeplay and the network
    #[test]
    fn both_network_switches_start_on_and_can_both_go_off() {
        let mut settings = Settings::default();
        assert!(settings.auto_update && settings.community);

        settings.auto_update = false;
        settings.community = false;
        settings.tidy();
        assert!(!settings.auto_update && !settings.community);
    }

    #[test]
    fn defaults_are_sane() {
        let settings = Settings::default();
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.accent, "amber");
        assert!(settings.favourites.is_empty());
    }

    #[test]
    fn a_missing_field_falls_back_rather_than_failing() {
        let settings: Settings = serde_json::from_str(r#"{"accent":"cyan"}"#).unwrap();
        assert_eq!(settings.accent, "cyan");
        assert_eq!(settings.theme, "system");
    }

    #[test]
    fn nonsense_written_by_hand_is_corrected() {
        let mut settings: Settings =
            serde_json::from_str(r#"{"theme":"neon","accent":"puce"}"#).unwrap();
        settings.tidy();
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.accent, "amber");
    }

    #[test]
    fn survives_a_round_trip() {
        let before = Settings {
            theme: "light".into(),
            favourites: vec!["steam:20920".into()],
            ..Default::default()
        };

        let text = serde_json::to_string(&before).unwrap();
        let after: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(after.theme, "light");
        assert_eq!(after.favourites, vec!["steam:20920".to_string()]);
    }

    // pinning is gone, and somebody upgrading had games in it
    #[test]
    fn what_was_pinned_becomes_a_favourite() {
        let mut settings: Settings = serde_json::from_str(
            r#"{"pinned":["steam:20920","steam:1"],"favourites":["steam:1","gog:x"]}"#,
        )
        .unwrap();
        settings.tidy();

        assert!(settings.pinned.is_empty());
        assert_eq!(
            settings.favourites,
            vec!["steam:1".to_string(), "gog:x".into(), "steam:20920".into()]
        );

        // and the old key is not written back out
        let text = serde_json::to_string(&settings).unwrap();
        assert!(!text.contains("pinned"), "{text}");
    }
}
