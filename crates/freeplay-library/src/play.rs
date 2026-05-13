//! How long you have played something and when you last did.
//!
//! Steam keeps both in `userdata/<user>/config/localconfig.vdf`, so this is
//! the same trick as the box art: read what is already on the disk rather than
//! asking anybody for it.
//!
//! Genre is deliberately missing. Steam only has it in `appinfo.vdf`, which is
//! a binary format, and everything else would mean calling the store API.

use std::collections::HashMap;
use std::path::Path;

use crate::vdf;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Play {
    /// Unix seconds, exactly as Steam writes it.
    pub last_played: Option<u64>,
    pub minutes: Option<u32>,
}

impl Play {
    pub fn is_empty(&self) -> bool {
        self.last_played.is_none() && self.minutes.is_none()
    }

    /// Merge two records for the same game. More than one Windows user can
    /// have played it, and the higher number is the interesting one.
    fn merge(self, other: Play) -> Play {
        Play {
            last_played: self.last_played.max(other.last_played),
            minutes: self.minutes.max(other.minutes),
        }
    }
}

fn from_config(text: &str) -> HashMap<String, Play> {
    let Ok(parsed) = vdf::parse(text) else {
        return HashMap::new();
    };

    let apps = parsed
        .get("UserLocalConfigStore")
        .and_then(|v| v.get("Software"))
        .and_then(|v| v.get("Valve"))
        .and_then(|v| v.get("Steam"))
        .and_then(|v| v.get("apps"));

    let Some(apps) = apps else {
        return HashMap::new();
    };

    apps.entries()
        .iter()
        .filter_map(|(app_id, entry)| {
            let play = Play {
                last_played: entry.string("LastPlayed").and_then(|v| v.parse().ok()),
                minutes: entry.string("Playtime").and_then(|v| v.parse().ok()),
            };
            (!play.is_empty()).then(|| (app_id.clone(), play))
        })
        .collect()
}

fn from_userdata(userdata: &Path) -> HashMap<String, Play> {
    let mut all: HashMap<String, Play> = HashMap::new();

    let Ok(users) = std::fs::read_dir(userdata) else {
        return all;
    };

    for user in users.flatten() {
        let config = user.path().join("config").join("localconfig.vdf");
        let Ok(text) = std::fs::read_to_string(&config) else {
            continue;
        };
        for (app_id, play) in from_config(&text) {
            all.entry(app_id)
                .and_modify(|held| *held = held.merge(play))
                .or_insert(play);
        }
    }
    all
}

/// Play records for every Steam game, keyed by app id.
#[cfg(windows)]
pub fn steam() -> HashMap<String, Play> {
    match crate::steam::root() {
        Some(root) => from_userdata(&root.join("userdata")),
        None => HashMap::new(),
    }
}

#[cfg(not(windows))]
pub fn steam() -> HashMap<String, Play> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
"UserLocalConfigStore"
{
    "Software"
    {
        "Valve"
        {
            "Steam"
            {
                "apps"
                {
                    "20920"
                    {
                        "LastPlayed"  "1785824227"
                        "Playtime"    "1674"
                    }
                    "1222140"
                    {
                        "LastPlayed"  "1770000000"
                    }
                    "9999"
                    {
                        "BadgeData"   "something"
                    }
                }
            }
        }
    }
}
"#;

    #[test]
    fn reads_playtime_and_last_played() {
        let all = from_config(SAMPLE);
        let witcher = all.get("20920").unwrap();
        assert_eq!(witcher.last_played, Some(1785824227));
        assert_eq!(witcher.minutes, Some(1674));
    }

    #[test]
    fn a_game_with_only_a_date_still_counts() {
        let all = from_config(SAMPLE);
        let detroit = all.get("1222140").unwrap();
        assert_eq!(detroit.last_played, Some(1770000000));
        assert_eq!(detroit.minutes, None);
    }

    #[test]
    fn entries_with_neither_are_left_out() {
        assert!(!from_config(SAMPLE).contains_key("9999"));
    }

    #[test]
    fn rubbish_input_is_empty_rather_than_a_panic() {
        assert!(from_config("not a vdf at all {{{").is_empty());
        assert!(from_config("").is_empty());
    }

    #[test]
    fn the_bigger_number_wins_when_two_users_played_it() {
        let a = Play {
            last_played: Some(100),
            minutes: Some(50),
        };
        let b = Play {
            last_played: Some(200),
            minutes: Some(20),
        };
        assert_eq!(
            a.merge(b),
            Play {
                last_played: Some(200),
                minutes: Some(50)
            }
        );
    }
}
