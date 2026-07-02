//! everything worth carrying to another machine, in one json file
//!
//! tables themselves are not copied. a published table is already reachable by
//! its number, so the file carries the number and the import pulls the same
//! bytes back down. the account is the same idea: only the name travels, and
//! the recovery words are typed on the other side, so the file on its own
//! cannot publish as anybody

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub freeplay: u32,
    #[serde(default)]
    pub made: String,
    #[serde(default)]
    pub prefs: Option<Prefs>,
    #[serde(default)]
    pub games: Vec<GameState>,
    #[serde(default)]
    pub account: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    pub theme: String,
    pub accent: String,
    #[serde(default)]
    pub pinned: Vec<String>,
    #[serde(default)]
    pub favourites: Vec<String>,
    #[serde(default)]
    pub auto_update: bool,
    #[serde(default)]
    pub auto_attach: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub exe: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub armed: Vec<String>,
    #[serde(default)]
    pub values: HashMap<String, String>,
    // the published table it came from, if any
    #[serde(default)]
    pub table: Option<i64>,
}

// every game freeplay is holding anything for, whether or not it is installed
// on this machine right now
pub fn known(settings: &Settings) -> Vec<String> {
    let mut exes: Vec<String> = settings
        .armed
        .keys()
        .chain(settings.values.keys())
        .chain(settings.grabbed.keys())
        .cloned()
        .collect();
    exes.sort();
    exes.dedup();
    exes
}

pub fn build(
    settings: &Settings,
    exes: &[String],
    prefs: bool,
    account: Option<String>,
    names: &HashMap<String, String>,
    stamp: String,
) -> Profile {
    let games = exes
        .iter()
        .map(|exe| {
            let exe = exe.to_lowercase();
            GameState {
                name: names.get(&exe).cloned().unwrap_or_default(),
                armed: settings.armed.get(&exe).cloned().unwrap_or_default(),
                values: settings.values.get(&exe).cloned().unwrap_or_default(),
                table: settings.grabbed.get(&exe).copied(),
                exe,
            }
        })
        .filter(|game| !game.armed.is_empty() || !game.values.is_empty() || game.table.is_some())
        .collect();

    Profile {
        freeplay: VERSION,
        made: stamp,
        prefs: prefs.then(|| Prefs {
            theme: settings.theme.clone(),
            accent: settings.accent.clone(),
            pinned: settings.pinned.clone(),
            favourites: settings.favourites.clone(),
            auto_update: settings.auto_update,
            auto_attach: settings.auto_attach,
        }),
        games,
        account,
    }
}

pub fn parse(text: &str) -> Result<Profile, String> {
    let profile: Profile =
        serde_json::from_str(text).map_err(|e| format!("that is not a Freeplay profile: {e}"))?;
    if profile.freeplay == 0 || profile.freeplay > VERSION {
        return Err(format!(
            "that profile was written by a newer Freeplay (format {})",
            profile.freeplay
        ));
    }
    Ok(profile)
}

// what came in wins over what is here, but a game the file says nothing about
// is left exactly as it was
pub fn apply(profile: &Profile, settings: &mut Settings) -> Applied {
    let mut applied = Applied::default();

    if let Some(prefs) = &profile.prefs {
        settings.theme = prefs.theme.clone();
        settings.accent = prefs.accent.clone();
        settings.pinned = prefs.pinned.clone();
        settings.favourites = prefs.favourites.clone();
        settings.auto_update = prefs.auto_update;
        settings.auto_attach = prefs.auto_attach;
        applied.prefs = true;
    }

    for game in &profile.games {
        let exe = game.exe.to_lowercase();
        if !game.armed.is_empty() {
            settings.armed.insert(exe.clone(), game.armed.clone());
        }
        if !game.values.is_empty() {
            settings.values.insert(exe.clone(), game.values.clone());
        }
        if let Some(id) = game.table {
            settings.grabbed.insert(exe.clone(), id);
            applied.tables.push(id);
        }
        applied.games += 1;
    }

    settings.tidy();
    applied
}

#[derive(Debug, Default)]
pub struct Applied {
    pub prefs: bool,
    pub games: usize,
    // published tables to go and fetch
    pub tables: Vec<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let mut settings = Settings {
            theme: "light".into(),
            pinned: vec!["steam:20920".into()],
            ..Default::default()
        };
        settings
            .armed
            .insert("witcher2.exe".into(), vec!["orens".into()]);
        settings.values.insert(
            "witcher2.exe".into(),
            HashMap::from([("orens".to_string(), "5000".to_string())]),
        );
        settings.grabbed.insert("witcher2.exe".into(), 3);
        settings
            .armed
            .insert("other.exe".into(), vec!["something".into()]);
        settings
    }

    fn stamp() -> String {
        "2026-08-09T00:00:00Z".into()
    }

    #[test]
    fn every_game_it_holds_anything_for_is_offered() {
        assert_eq!(known(&settings()), ["other.exe", "witcher2.exe"]);
    }

    #[test]
    fn only_the_games_that_were_picked_go_in() {
        let profile = build(
            &settings(),
            &["witcher2.exe".into()],
            true,
            None,
            &HashMap::new(),
            stamp(),
        );
        assert_eq!(profile.games.len(), 1);
        assert_eq!(profile.games[0].exe, "witcher2.exe");
        assert_eq!(profile.games[0].values["orens"], "5000");
        assert_eq!(profile.games[0].table, Some(3));
    }

    #[test]
    fn leaving_preferences_out_leaves_them_out() {
        let profile = build(
            &settings(),
            &["witcher2.exe".into()],
            false,
            None,
            &HashMap::new(),
            stamp(),
        );
        assert!(profile.prefs.is_none());
    }

    // the file is meant to be safe to email to yourself
    #[test]
    fn the_account_travels_as_a_name_and_never_as_a_key() {
        let profile = build(
            &settings(),
            &[],
            false,
            Some("aSwedishMagyar".into()),
            &HashMap::new(),
            stamp(),
        );
        let text = serde_json::to_string(&profile).unwrap();
        assert!(text.contains("aSwedishMagyar"));
        assert!(!text.to_lowercase().contains("secret"));
        assert!(!text.to_lowercase().contains("private"));
    }

    #[test]
    fn a_game_with_nothing_set_is_not_written_out() {
        let mut bare = Settings::default();
        bare.armed.insert("empty.exe".into(), Vec::new());
        let profile = build(
            &bare,
            &["empty.exe".into()],
            false,
            None,
            &HashMap::new(),
            stamp(),
        );
        assert!(profile.games.is_empty());
    }

    #[test]
    fn importing_puts_everything_back() {
        let profile = build(
            &settings(),
            &["witcher2.exe".into()],
            true,
            None,
            &HashMap::new(),
            stamp(),
        );
        let text = serde_json::to_string(&profile).unwrap();

        let mut fresh = Settings::default();
        let applied = apply(&parse(&text).unwrap(), &mut fresh);

        assert!(applied.prefs);
        assert_eq!(applied.games, 1);
        assert_eq!(applied.tables, vec![3]);
        assert_eq!(fresh.theme, "light");
        assert_eq!(fresh.armed["witcher2.exe"], vec!["orens".to_string()]);
        assert_eq!(fresh.values["witcher2.exe"]["orens"], "5000");
    }

    #[test]
    fn a_game_the_file_says_nothing_about_is_left_alone() {
        let profile = build(
            &settings(),
            &["witcher2.exe".into()],
            false,
            None,
            &HashMap::new(),
            stamp(),
        );

        let mut mine = Settings::default();
        mine.armed.insert("mine.exe".into(), vec!["keep".into()]);
        apply(&profile, &mut mine);

        assert_eq!(mine.armed["mine.exe"], vec!["keep".to_string()]);
        assert!(mine.armed.contains_key("witcher2.exe"));
    }

    #[test]
    fn rubbish_is_refused_with_a_reason() {
        assert!(parse("{}").is_err());
        assert!(parse("not json").is_err());
        let newer = parse(r#"{"freeplay":99}"#).unwrap_err();
        assert!(newer.contains("newer Freeplay"));
    }

    #[test]
    fn a_profile_from_a_future_field_still_loads() {
        let text = r#"{"freeplay":1,"games":[],"somethingNew":true}"#;
        assert!(parse(text).is_ok());
    }
}
