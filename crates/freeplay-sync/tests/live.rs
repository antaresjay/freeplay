// hits the real worker, so it is off by default:
//   cargo test -p freeplay-sync --test live -- --ignored --nocapture
use freeplay_sync::community::{Community, Live, ENDPOINT};
use freeplay_table::Table;

const TABLE: &str = r#"
[game]
name = "Live Check"
exe = "freeplay-live-check.exe"

[[cheat]]
id = "one"
name = "A Cheat"
type = "freeze"
value_type = "i32"
value = 1

[cheat.locator]
find = "static"
module = "freeplay-live-check.exe"
offset = "0x1000"
"#;

#[test]
#[ignore]
fn talks_to_the_worker() {
    let wire = Live;
    let community = Community::new(ENDPOINT, &wire);
    let table = Table::parse(TABLE).unwrap();

    let sent = community
        .submit(&table, TABLE, None, "1.0")
        .expect("submit should work");
    println!("submitted id {} already {}", sent.id, sent.already);

    let again = community.submit(&table, TABLE, None, "1.0").unwrap();
    assert_eq!(again.id, sent.id);
    assert!(again.already, "the second one should be recognised");

    let listed = community
        .list("freeplay-live-check.exe", "1.0")
        .expect("list should work");
    assert!(listed.iter().any(|row| row.id == sent.id));
    println!(
        "listed {} table(s), first: {}",
        listed.len(),
        listed[0].standing()
    );

    let (text, fetched) = community
        .fetch(sent.id, "0123456789abcdef")
        .expect("fetch should work");
    assert_eq!(fetched.game.exe, "freeplay-live-check.exe");
    assert_eq!(text.trim(), TABLE.trim());

    community
        .vote(
            sent.id,
            "0123456789abcdef",
            true,
            "1.0",
            "freeplay-live-check.exe",
        )
        .expect("vote should work");

    let after = community.list("freeplay-live-check.exe", "1.0").unwrap();
    let row = after.iter().find(|r| r.id == sent.id).unwrap();
    assert!(row.up >= 1, "the vote should have counted");
    println!("after voting: {}", row.standing());
}

#[test]
#[ignore]
fn the_worker_refuses_what_it_should() {
    let wire = Live;
    let community = Community::new(ENDPOINT, &wire);

    let nasty = TABLE.replace(
        "[[cheat]]",
        "[[cheat]]\nid = \"s\"\nname = \"S\"\ntype = \"script\"\nsource = \"\"\"\n[ENABLE]\nloadlibrary(evil.dll)\n[DISABLE]\n\"\"\"\n\n[[cheat]]",
    );
    let table = Table::parse(&nasty).expect("should still parse");

    let outcome = community.submit(&table, &nasty, None, "1.0");
    let why = outcome.unwrap_err();
    println!("refused with: {why}");
    assert!(why.contains("loadlibrary"), "{why}");
}

#[test]
#[ignore]
fn a_claimed_name_cannot_be_taken_by_somebody_else() {
    let wire = Live;
    let community = Community::new(ENDPOINT, &wire);

    let thief = freeplay_id::Identity::create("aSwedishMagyar").unwrap();
    let table = Table::parse(&TABLE.replace("0x1000", "0x2000")).unwrap();

    let why = community
        .submit(&table, TABLE, Some(&thief), "1.0")
        .expect_err("a different key must not publish under a taken name");

    println!("refused with: {why}");
    assert!(why.contains("somebody else"), "{why}");
}

#[test]
#[ignore]
fn an_anonymous_submission_still_works() {
    let wire = Live;
    let community = Community::new(ENDPOINT, &wire);
    let table = Table::parse(&TABLE.replace("0x1000", "0x3000")).unwrap();

    let sent = community.submit(&table, TABLE, None, "1.0").unwrap();
    println!("anonymous submission got id {}", sent.id);

    let listed = community.list("freeplay-live-check.exe", "1.0").unwrap();
    let row = listed.iter().find(|r| r.id == sent.id).unwrap();
    assert!(row.submitted_by.is_empty(), "no name should be on it");
}
