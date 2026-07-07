//! Checks that the interface and its script agree about what exists.
//!
//! There is no bundler here and nothing type checks the front end, so a
//! renamed or forgotten `id` fails at runtime, inside a click handler, with no
//! sign anything is wrong. One missing `id="game-cover"` meant every game page
//! threw before it was shown, which looked like clicking a game did nothing.
//! These run with `cargo test` and cost nothing.

#![cfg(test)]

use std::collections::HashSet;
use std::path::PathBuf;

fn ui_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("ui")
}

fn read(name: &str) -> String {
    let path = ui_dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Everything between the delimiters, for every occurrence.
fn between(text: &str, open: &str, close: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        rest = &rest[start + open.len()..];
        match rest.find(close) {
            Some(end) => {
                found.push(rest[..end].to_string());
                rest = &rest[end..];
            }
            None => break,
        }
    }
    found
}

fn declared_ids(html: &str) -> HashSet<String> {
    between(html, "id=\"", "\"").into_iter().collect()
}

fn ids_line_up(page: &str, script: &str) {
    let html = read(page);
    let js = read(script);
    let have = declared_ids(&html);

    let missing: Vec<String> = between(&js, "$(\"", "\")")
        .into_iter()
        .filter(|id| !have.contains(id))
        .collect();

    assert!(
        missing.is_empty(),
        "{script} looks up ids that {page} does not define: {missing:?}. \
         $() returns null for these and the next property access throws."
    );
}

#[test]
fn every_id_the_script_asks_for_exists_in_the_page() {
    ids_line_up("index.html", "app.js");
}

// the overlay is a second window with a page and a script of its own, and
// nothing else in the suite would notice if it stopped loading
#[test]
fn the_overlay_page_and_its_script_agree_too() {
    ids_line_up("overlay.html", "overlay.js");
}

#[test]
fn the_overlay_loads_what_it_needs() {
    let html = read("overlay.html");
    for part in ["style.css", "overlay.css", "overlay.js"] {
        assert!(html.contains(part), "overlay.html should load {part}");
    }
}

#[test]
fn every_nav_button_points_at_a_section_that_exists() {
    let html = read("index.html");
    let have = declared_ids(&html);

    for view in between(&html, "data-view=\"", "\"") {
        assert!(
            have.contains(&format!("view-{view}")),
            "a nav button points at {view:?} but there is no id=\"view-{view}\""
        );
    }
    for target in between(&html, "data-goto=\"", "\"") {
        assert!(
            have.contains(&format!("view-{target}")),
            "something links to {target:?} but there is no id=\"view-{target}\""
        );
    }
}

/// showView() hides every section by name, so the list in the script and the
/// sections in the page have to be the same set.
#[test]
fn the_script_and_the_page_agree_on_which_views_exist() {
    let html = read("index.html");
    let js = read("app.js");

    let in_page: HashSet<String> = declared_ids(&html)
        .iter()
        .filter_map(|id| id.strip_prefix("view-").map(str::to_string))
        .collect();

    let line = js
        .lines()
        .find(|l| l.contains("for (const id of ["))
        .expect("showView should still list its views inline");
    // between() also returns what sits between one literal and the next, so
    // drop anything that is not a plain name.
    let in_script: HashSet<String> = between(line, "\"", "\"")
        .into_iter()
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
        .collect();

    assert_eq!(
        in_script, in_page,
        "showView() and index.html disagree about the set of views"
    );
}

#[test]
fn commands_the_script_calls_are_all_registered() {
    let js = read("app.js") + &read("overlay.js");
    let rust = include_str!("main.rs");

    let registered: HashSet<&str> = rust
        .split("generate_handler![")
        .nth(1)
        .expect("there should be a generate_handler! block")
        .split(']')
        .next()
        .unwrap()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let mut called: Vec<String> = between(&js, "invoke(\"", "\"");
    called.sort();
    called.dedup();

    let missing: Vec<&String> = called
        .iter()
        .filter(|c| !registered.contains(c.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "app.js invokes commands that are not in generate_handler!: {missing:?}"
    );
}
