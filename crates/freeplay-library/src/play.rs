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

// epic writes `LastPlayedGame=<namespace>:<item>:<app>,<iso timestamp>` once
// per game, all under a section named after the account. no minutes anywhere,
// the launcher asks its own servers for those
fn from_launcher_ini(text: &str) -> HashMap<String, Play> {
    let mut found: HashMap<String, Play> = HashMap::new();

    for line in text.lines() {
        let Some(value) = line.trim().strip_prefix("LastPlayedGame=") else {
            continue;
        };
        let Some((id, stamp)) = value.rsplit_once(',') else {
            continue;
        };
        // three parts, or it is not an id we would ever look up
        if id.split(':').count() != 3 {
            continue;
        }
        let Some(at) = iso_seconds(stamp) else {
            continue;
        };
        let held = &mut found.entry(id.to_string()).or_default().last_played;
        *held = Some((*held).unwrap_or(0).max(at));
    }
    found
}

// 2026-08-13T09:08:17.534Z, always utc. a whole date crate to read one field
// of one ini file is not worth it
fn iso_seconds(stamp: &str) -> Option<u64> {
    let stamp = stamp.trim();
    let (date, rest) = stamp.split_once('T')?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;

    let clock = rest.split(['.', 'Z', '+']).next()?;
    let mut units = clock.split(':');
    let hour: i64 = units.next()?.parse().ok()?;
    let minute: i64 = units.next()?.parse().ok()?;
    let second: i64 = units.next().unwrap_or("0").parse().ok()?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }

    // days from the civil epoch, shifting the year to start in march so the
    // leap day lands at the end of it
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    (seconds > 946_684_800).then_some(seconds as u64)
}

#[cfg(windows)]
fn launcher_ini() -> Option<std::path::PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    Some(
        std::path::PathBuf::from(local)
            .join("EpicGamesLauncher")
            .join("Saved")
            .join("Config")
            .join("WindowsEditor")
            .join("GameUserSettings.ini"),
    )
}

// keyed by the same namespace:item:app triple discovery hands back
#[cfg(windows)]
pub fn epic() -> HashMap<String, Play> {
    launcher_ini()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| from_launcher_ini(&text))
        .unwrap_or_default()
}

#[cfg(not(windows))]
pub fn epic() -> HashMap<String, Play> {
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

    // cut from the real GameUserSettings.ini
    const LAUNCHER: &str = "\
[dd7993131d61415f8c19537776913dbd_Launcher]
LastPlayedGame=caca23a0954f4c1aba1fdd7e277b81e2:ff45e0eabd0c48d6950e369c79c26823:d6264d56f5ba434e91d4b0a0b056c83a,2026-08-13T09:08:17.534Z
LastPlayedGame=a14a02aa3c8143729605eaf7c93d7501:232e505971824de9ba841ba35d4b08f5:glacrdemo,2025-08-16T19:43:27.027Z
LastPlayedGame=nonsense,2025-08-16T19:43:27.027Z
LastPlayedGame=a:b:c,not a date
SomethingElse=a:b:c,2025-08-16T19:43:27.027Z
";

    #[test]
    fn epic_says_when_but_never_how_long() {
        let found = from_launcher_ini(LAUNCHER);
        assert_eq!(found.len(), 2);
        let tr = found["caca23a0954f4c1aba1fdd7e277b81e2:\
                        ff45e0eabd0c48d6950e369c79c26823:\
                        d6264d56f5ba434e91d4b0a0b056c83a"];
        assert_eq!(tr.last_played, Some(1_786_612_097));
        assert_eq!(tr.minutes, None);
    }

    #[test]
    fn a_game_played_twice_keeps_the_later_one() {
        let text = "LastPlayedGame=a:b:c,2024-01-01T00:00:00.000Z\n\
                    LastPlayedGame=a:b:c,2026-03-04T05:06:07.000Z\n";
        assert_eq!(
            from_launcher_ini(text)["a:b:c"].last_played,
            Some(1_772_600_767)
        );
    }

    #[test]
    fn the_epic_timestamps_are_utc() {
        assert_eq!(iso_seconds("2026-08-13T09:08:17.534Z"), Some(1_786_612_097));
        // a leap day, and one right after it
        assert_eq!(iso_seconds("2024-02-29T00:00:00Z"), Some(1_709_164_800));
        assert_eq!(iso_seconds("2024-03-01T00:00:00Z"), Some(1_709_251_200));
        assert_eq!(iso_seconds("2000-01-01T00:00:01Z"), Some(946_684_801));
    }

    #[test]
    fn a_date_that_is_not_one_is_dropped() {
        for bad in [
            "",
            "2026-08-13",
            "not a date",
            "2026-13-01T00:00:00Z",
            "2026-08-32T00:00:00Z",
            "2026-08-13T99:00:00Z",
            "1970-01-01T00:00:00Z",
        ] {
            assert_eq!(iso_seconds(bad), None, "{bad} should not parse");
        }
    }
}
