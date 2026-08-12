use freeplay_id::Identity;
use freeplay_table::Table;
use serde::{Deserialize, Serialize};

use crate::http;

pub const ENDPOINT: &str = "https://freeplay-tables.antaresjay.workers.dev";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Listing {
    pub id: i64,
    pub exe: String,
    pub game: String,
    pub fingerprint: String,
    #[serde(default)]
    pub cheats: u32,
    #[serde(default)]
    pub bytes: u64,
    // who uploaded it
    #[serde(default)]
    pub submitted_by: String,
    // who worked the addresses out, which is usually somebody else
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub built_for: String,
    #[serde(default)]
    pub up: i64,
    #[serde(default)]
    pub down: i64,
    #[serde(default)]
    pub downloads: i64,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    // a table somebody used on your build first, then the ones people liked
    #[default]
    Best,
    Votes,
    Downloads,
    Newest,
    Oldest,
    Most,
}

impl Sort {
    pub fn key(self) -> &'static str {
        match self {
            Sort::Best => "best",
            Sort::Votes => "votes",
            Sort::Downloads => "downloads",
            Sort::Newest => "new",
            Sort::Oldest => "old",
            Sort::Most => "cheats",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Best => "Best match",
            Sort::Votes => "Most liked",
            Sort::Downloads => "Most used",
            Sort::Newest => "Newest",
            Sort::Oldest => "Oldest",
            Sort::Most => "Most cheats",
        }
    }

    pub fn all() -> [Sort; 6] {
        [
            Sort::Best,
            Sort::Votes,
            Sort::Downloads,
            Sort::Newest,
            Sort::Oldest,
            Sort::Most,
        ]
    }
}

impl std::str::FromStr for Sort {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        Sort::all()
            .into_iter()
            .find(|s| s.key() == text.trim().to_lowercase())
            .ok_or_else(|| {
                let names: Vec<&str> = Sort::all().iter().map(|s| s.key()).collect();
                format!("sort by one of {}", names.join(", "))
            })
    }
}

impl Listing {
    pub fn score(&self) -> i64 {
        self.up - self.down
    }

    // what the picker puts under the name. a table nobody has voted on yet
    // should say so rather than showing a smug zero
    pub fn standing(&self) -> String {
        let mut parts = Vec::new();

        if !self.submitted_by.is_empty() {
            parts.push(format!("by {}", self.submitted_by));
        }
        parts.push(format!("{} cheats", self.cheats));

        if self.up == 0 && self.down == 0 {
            parts.push("nobody has voted yet".into());
        } else {
            parts.push(format!("{} up and {} down", self.up, self.down));
        }

        match self.downloads {
            0 => {}
            1 => parts.push("used once".into()),
            n => parts.push(format!("used {n} times")),
        }

        parts.join(", ")
    }
}

#[derive(Debug, Deserialize)]
struct Listings {
    #[serde(default)]
    tables: Vec<Listing>,
}

#[derive(Debug, Deserialize)]
struct Fetched {
    toml: String,
}

#[derive(Debug, Deserialize)]
pub struct Submitted {
    pub id: i64,
    #[serde(default)]
    pub already: bool,
}

// swapped out in tests so none of this needs a network
pub trait Wire: Send + Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
    fn post(&self, url: &str, body: &[u8]) -> Result<Vec<u8>, String>;
}

pub struct Live;

impl Wire for Live {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        http::get(url)
    }
    fn post(&self, url: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        http::post(url, body)
    }
}

pub struct Community<'a> {
    pub endpoint: String,
    wire: &'a dyn Wire,
}

impl<'a> Community<'a> {
    pub fn new(endpoint: &str, wire: &'a dyn Wire) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            wire,
        }
    }

    pub fn list(&self, exe: &str, build: &str) -> Result<Vec<Listing>, String> {
        self.list_by(exe, build, Sort::Best)
    }

    pub fn list_by(&self, exe: &str, build: &str, sort: Sort) -> Result<Vec<Listing>, String> {
        let url = format!(
            "{}/tables?exe={}&build={}&sort={}",
            self.endpoint,
            encode(&exe.to_lowercase()),
            encode(build),
            sort.key()
        );
        let raw = self.wire.get(&url)?;
        let listings: Listings =
            serde_json::from_slice(&raw).map_err(|e| format!("could not read the list: {e}"))?;
        Ok(listings.tables)
    }

    // by game name rather than executable, for when we picked the wrong
    // binary or the table was written against another edition of the game
    pub fn search(&self, query: &str) -> Result<Vec<Listing>, String> {
        let wanted = query.trim();
        if wanted.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let raw = self
            .wire
            .get(&format!("{}/search?q={}", self.endpoint, encode(wanted)))?;
        let listings: Listings =
            serde_json::from_slice(&raw).map_err(|e| format!("could not read the list: {e}"))?;
        Ok(listings.tables)
    }

    // whatever comes back has to parse and validate before it is a table we
    // will hand to anything else
    pub fn fetch(&self, id: i64, install: &str) -> Result<(String, Table), String> {
        let raw = self.wire.get(&format!(
            "{}/table/{id}?install={}",
            self.endpoint,
            encode(install)
        ))?;
        let fetched: Fetched =
            serde_json::from_slice(&raw).map_err(|e| format!("could not read that table: {e}"))?;

        let table = Table::parse(&fetched.toml).map_err(|e| format!("will not parse: {e}"))?;
        table.validate()?;
        Ok((fetched.toml, table))
    }

    // no identity means anonymous, which is the default and needs nothing.
    // with one, the name is signed for so nobody else can publish under it
    pub fn submit(
        &self,
        table: &Table,
        toml: &str,
        who: Option<&Identity>,
        built_for: &str,
    ) -> Result<Submitted, String> {
        let fingerprint = freeplay_table::fingerprint::fingerprint(table);

        let mut body = serde_json::json!({
            "fingerprint": fingerprint,
            "exe": table.game.exe.to_lowercase(),
            "game": table.game.name,
            "toml": toml,
            "built_for": built_for,
            "cheats": table.cheats.len(),
            // whoever worked the addresses out, which for anything converted
            // is not whoever is uploading it. goes up alongside so the list
            // can say who wrote a table before you download it
            "author": table.game.author,
        });

        if let Some(me) = who {
            let signed = freeplay_id::message(&me.name, &fingerprint);
            body["submitted_by"] = me.name.clone().into();
            body["pubkey"] = me.public().into();
            body["signature"] = me.sign(&signed).into();
        }

        let raw = self.wire.post(
            &format!("{}/submit", self.endpoint),
            body.to_string().as_bytes(),
        )?;
        serde_json::from_slice(&raw).map_err(|e| format!("odd answer to a submission: {e}"))
    }

    // whether a name is already spoken for, so the interface can say so before
    // somebody picks one and gets refused
    pub fn taken(&self, name: &str) -> Result<bool, String> {
        match self
            .wire
            .get(&format!("{}/who/{}", self.endpoint, encode(name)))
        {
            Ok(raw) => Ok(!String::from_utf8_lossy(&raw).contains("\"taken\":false")),
            Err(why) if why.contains("404") => Ok(false),
            Err(_) => Ok(false),
        }
    }

    // for_exe is what the game was actually running, which is not always what
    // the table is filed under. an upvote from somebody using a table on a
    // different binary is the only evidence there is that the two belong
    // together, so it goes up with the vote
    pub fn vote(
        &self,
        id: i64,
        install: &str,
        up: bool,
        built_for: &str,
        for_exe: &str,
    ) -> Result<(), String> {
        let body = serde_json::json!({
            "id": id,
            "install": install,
            "up": up,
            "built_for": built_for,
            "for_exe": for_exe.to_lowercase(),
        });
        self.wire.post(
            &format!("{}/vote", self.endpoint),
            body.to_string().as_bytes(),
        )?;
        Ok(())
    }
}

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// not tied to the machine on purpose. it exists to stop one person voting
// twice, not to know who they are
pub fn new_install_id(seed: u128) -> String {
    freeplay_table::fingerprint::sha256(&seed.to_le_bytes())[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const TABLE: &str = r#"
        [game]
        name = "The Witcher 2"
        exe = "witcher2.exe"

        [[cheat]]
        id = "vitality"
        name = "Infinite Vitality"
        type = "freeze"
        value_type = "f32"
        value = 1000

        [cheat.locator]
        find = "static"
        module = "witcher2.exe"
        offset = "0x1A2B3C"
    "#;

    #[derive(Default)]
    struct Fake {
        answers: Vec<(String, String)>,
        seen: RefCell<Vec<String>>,
    }

    impl Fake {
        fn with(mut self, contains: &str, answer: &str) -> Self {
            self.answers.push((contains.into(), answer.into()));
            self
        }
        fn answer(&self, url: &str, body: Option<&str>) -> Result<Vec<u8>, String> {
            self.seen
                .borrow_mut()
                .push(format!("{url} {}", body.unwrap_or("")));
            self.answers
                .iter()
                .find(|(needle, _)| url.contains(needle.as_str()))
                .map(|(_, answer)| answer.as_bytes().to_vec())
                .ok_or_else(|| format!("nothing canned for {url}"))
        }
    }

    unsafe impl Sync for Fake {}
    unsafe impl Send for Fake {}

    impl Wire for Fake {
        fn get(&self, url: &str) -> Result<Vec<u8>, String> {
            self.answer(url, None)
        }
        fn post(&self, url: &str, body: &[u8]) -> Result<Vec<u8>, String> {
            self.answer(url, Some(&String::from_utf8_lossy(body)))
        }
    }

    #[test]
    fn reads_a_listing() {
        let wire = Fake::default().with(
            "/tables",
            r#"{"tables":[{"id":3,"exe":"witcher2.exe","game":"The Witcher 2",
                "fingerprint":"ab","cheats":23,"submitted_by":"someone",
                "built_for":"3.5","up":9,"down":2}]}"#,
        );
        let out = Community::new("https://x", &wire)
            .list("witcher2.exe", "3.5")
            .unwrap();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, 3);
        assert_eq!(out[0].score(), 7);
    }

    #[test]
    fn asks_for_the_right_game_and_build() {
        let wire = Fake::default().with("/tables", r#"{"tables":[]}"#);
        Community::new("https://x", &wire)
            .list("The Witcher 2.exe", "3.5.0.1")
            .unwrap();

        let seen = wire.seen.borrow()[0].clone();
        assert!(seen.contains("exe=the%20witcher%202.exe"), "{seen}");
        assert!(seen.contains("build=3.5.0.1"), "{seen}");
    }

    #[test]
    fn an_empty_list_is_not_an_error() {
        let wire = Fake::default().with("/tables", r#"{"tables":[]}"#);
        assert!(Community::new("https://x", &wire)
            .list("nothing.exe", "")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_fetched_table_has_to_parse() {
        let answer = serde_json::json!({ "toml": TABLE }).to_string();
        let wire = Fake::default().with("/table/", &answer);

        let (text, table) = Community::new("https://x", &wire)
            .fetch(7, "0123456789abcdef")
            .unwrap();
        assert_eq!(table.cheats.len(), 1);
        assert!(text.contains("witcher2.exe"));
    }

    // the worker counts a download per install, so it has to be told which one
    #[test]
    fn fetching_says_who_is_asking_so_it_can_be_counted() {
        let answer = serde_json::json!({ "toml": TABLE }).to_string();
        let wire = Fake::default().with("/table/", &answer);
        Community::new("https://x", &wire)
            .fetch(7, "0123456789abcdef")
            .unwrap();

        assert!(wire.seen.borrow()[0].contains("install=0123456789abcdef"));
    }

    #[test]
    fn sorting_is_passed_along() {
        let wire = Fake::default().with("/tables", r#"{"tables":[]}"#);
        Community::new("https://x", &wire)
            .list_by("a.exe", "", Sort::Downloads)
            .unwrap();
        assert!(wire.seen.borrow()[0].contains("sort=downloads"));
    }

    #[test]
    fn sort_names_round_trip() {
        for sort in Sort::all() {
            assert_eq!(sort.key().parse::<Sort>().unwrap(), sort);
        }
        assert!("nonsense".parse::<Sort>().is_err());
    }

    #[test]
    fn standing_counts_downloads_too() {
        let one = Listing {
            cheats: 3,
            downloads: 1,
            ..Default::default()
        };
        assert!(one.standing().contains("used once"), "{}", one.standing());

        let many = Listing {
            cheats: 3,
            downloads: 42,
            ..Default::default()
        };
        assert!(many.standing().contains("used 42 times"));

        let never = Listing {
            cheats: 3,
            ..Default::default()
        };
        assert!(!never.standing().contains("used"));
    }

    #[test]
    fn rubbish_off_the_network_never_becomes_a_table() {
        let answer = serde_json::json!({ "toml": "this is not toml {{{" }).to_string();
        let wire = Fake::default().with("/table/", &answer);

        assert!(Community::new("https://x", &wire)
            .fetch(7, "0123456789abcdef")
            .is_err());
    }

    #[test]
    fn submitting_sends_the_fingerprint_and_the_counts() {
        let table = Table::parse(TABLE).unwrap();
        let wire = Fake::default().with("/submit", r#"{"id":4,"already":false}"#);

        let out = Community::new("https://x", &wire)
            .submit(&table, TABLE, None, "3.5")
            .unwrap();
        assert_eq!(out.id, 4);
        assert!(!out.already);

        let sent = wire.seen.borrow()[0].clone();
        let expected = freeplay_table::fingerprint::fingerprint(&table);
        assert!(sent.contains(&expected), "{sent}");
        assert!(sent.contains("\"cheats\":1"), "{sent}");
        assert!(sent.contains("witcher2.exe"), "{sent}");
    }

    #[test]
    fn an_anonymous_submission_carries_no_name_or_key() {
        let table = Table::parse(TABLE).unwrap();
        let wire = Fake::default().with("/submit", r#"{"id":4,"already":false}"#);
        Community::new("https://x", &wire)
            .submit(&table, TABLE, None, "")
            .unwrap();

        let sent = wire.seen.borrow()[0].clone();
        assert!(!sent.contains("submitted_by"), "{sent}");
        assert!(!sent.contains("pubkey"), "{sent}");
        assert!(!sent.contains("signature"), "{sent}");
    }

    #[test]
    fn a_named_submission_is_signed_for() {
        let table = Table::parse(TABLE).unwrap();
        let me = Identity::create("someone").unwrap();
        let wire = Fake::default().with("/submit", r#"{"id":4,"already":false}"#);

        Community::new("https://x", &wire)
            .submit(&table, TABLE, Some(&me), "")
            .unwrap();

        let sent = wire.seen.borrow()[0].clone();
        let body: serde_json::Value =
            serde_json::from_str(sent.split_once(' ').unwrap().1.trim()).unwrap();

        assert_eq!(body["submitted_by"], "someone");
        assert_eq!(body["pubkey"], me.public());

        let signed = freeplay_id::message("someone", body["fingerprint"].as_str().unwrap());
        assert!(freeplay_id::verify(
            &me.public(),
            &signed,
            body["signature"].as_str().unwrap()
        ));
    }

    #[test]
    fn a_table_already_there_says_so_rather_than_failing() {
        let table = Table::parse(TABLE).unwrap();
        let wire = Fake::default().with("/submit", r#"{"id":4,"already":true}"#);

        let out = Community::new("https://x", &wire)
            .submit(&table, TABLE, None, "")
            .unwrap();
        assert!(out.already);
    }

    #[test]
    fn voting_sends_the_install_and_which_way() {
        let wire = Fake::default().with("/vote", r#"{"ok":true}"#);
        Community::new("https://x", &wire)
            .vote(4, "deadbeefdeadbeef", false, "3.5", "witcher2.exe")
            .unwrap();

        let sent = wire.seen.borrow()[0].clone();
        assert!(sent.contains("\"install\":\"deadbeefdeadbeef\""), "{sent}");
        assert!(sent.contains("\"up\":false"), "{sent}");
    }

    // the service only learns that two executables go together if the vote
    // says which one the game was actually running
    #[test]
    fn voting_says_which_executable_it_was_used_on() {
        let wire = Fake::default().with("/vote", r#"{"ok":true}"#);
        Community::new("https://x", &wire)
            .vote(4, "deadbeefdeadbeef", true, "1.2", "FalloutwHR.exe")
            .unwrap();

        let sent = wire.seen.borrow()[0].clone();
        assert!(sent.contains("\"for_exe\":\"falloutwhr.exe\""), "{sent}");
    }

    #[test]
    fn searching_asks_by_name_and_not_by_executable() {
        let wire = Fake::default().with(
            "/search",
            r#"{"tables":[{"id":1,"exe":"falloutw.exe","game":"Fallout","fingerprint":"a"}]}"#,
        );
        let found = Community::new("https://x", &wire)
            .search("fall out")
            .unwrap();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].game, "Fallout");
        let asked = wire.seen.borrow()[0].clone();
        assert!(asked.contains("/search?q=fall%20out"), "{asked}");
    }

    // two letters is the floor the service enforces, so do not spend a
    // request finding that out
    #[test]
    fn one_letter_is_not_a_search() {
        let wire = Fake::default();
        assert!(Community::new("https://x", &wire)
            .search(" f ")
            .unwrap()
            .is_empty());
        assert!(wire.seen.borrow().is_empty(), "it should not have asked");
    }

    #[test]
    fn a_trailing_slash_on_the_endpoint_does_not_double_up() {
        let wire = Fake::default().with("/tables", r#"{"tables":[]}"#);
        Community::new("https://x/", &wire)
            .list("a.exe", "")
            .unwrap();
        assert!(!wire.seen.borrow()[0].contains("//tables"));
    }

    #[test]
    fn an_install_id_is_the_right_shape_for_the_worker() {
        let id = new_install_id(12345);
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(id, new_install_id(12346));
    }

    #[test]
    fn standing_says_when_nobody_has_voted() {
        let fresh = Listing {
            cheats: 23,
            submitted_by: "someone".into(),
            ..Default::default()
        };
        assert!(fresh.standing().contains("nobody has voted"));

        let used = Listing {
            cheats: 23,
            up: 9,
            down: 2,
            ..Default::default()
        };
        assert!(used.standing().contains("9 up and 2 down"));
    }
}
