//! A log file next to the settings.
//!
//! A window with no console cannot tell you anything when something quietly
//! fails, and "it did nothing" is the least useful bug report there is. This
//! writes what the app tried and what came back, in plain text, somewhere you
//! can open it.

use std::fs::OpenOptions;
use std::path::PathBuf;

/// Anything past this and the file gets started again, so it cannot quietly
/// grow forever on a machine nobody looks at.
const MAX_BYTES: u64 = 512 * 1024;

pub fn path() -> PathBuf {
    crate::settings::path().with_file_name("freeplay.log")
}

pub fn start(verbose: bool) {
    let file = path();
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&file).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::remove_file(&file);
    }

    let level = if verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };

    // Reopened per line rather than held. A handful of lines a session is not
    // worth a background thread, and an Arc<File> quietly wrote nowhere.
    let sink = file.clone();
    let made = tracing_subscriber::fmt()
        .with_writer(
            move || match OpenOptions::new().create(true).append(true).open(&sink) {
                Ok(handle) => Box::new(handle) as Box<dyn std::io::Write>,
                Err(_) => Box::new(std::io::sink()),
            },
        )
        .with_ansi(false)
        .with_target(false)
        .with_max_level(level)
        .try_init();

    if made.is_ok() {
        tracing::info!(
            "freeplay {} starting, logging to {}",
            env!("CARGO_PKG_VERSION"),
            file.display()
        );
    }
}

/// Everything that would go in a bug report, as text.
pub fn report(extra: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("freeplay {}\n", env!("CARGO_PKG_VERSION")));
    out.push_str(extra);
    out.push_str("\n--- log ---\n");

    match std::fs::read_to_string(path()) {
        Ok(text) => {
            // The tail is the part anybody cares about.
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(200);
            out.push_str(&lines[start..].join("\n"));
        }
        Err(e) => out.push_str(&format!("(no log: {e})")),
    }
    out
}
