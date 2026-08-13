//! most people will use the desktop app. this exists because it is the fastest
//! way to try the engine against a real game, and because finding an address is
//! a conversation rather than one command

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use freeplay_core::target::Target;
use freeplay_core::value::{Scalar, ValueKind};
use freeplay_core::windows_target::{processes, WindowsTarget};
use freeplay_core::Error as CoreError;
use freeplay_library::discover;
use freeplay_session::Session;
use freeplay_table::resolve::State;
use freeplay_table::Table;

#[derive(Parser)]
#[command(name = "freeplay", version, about = "Open source game trainer")]
struct Cli {
    #[arg(long, global = true, help = "verbose logging")]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Games found on this machine.
    Games {
        #[arg(long, help = "show every candidate executable, best guess first")]
        all: bool,
    },
    /// Start a game the way its store expects.
    Play {
        #[arg(help = "part of the game's name")]
        name: String,
        #[arg(long, help = "say what it would do without doing it")]
        dry_run: bool,
    },
    /// Convert a Cheat Engine table into a Freeplay one.
    Import {
        #[arg(help = "path to a .CT file")]
        file: PathBuf,
        #[arg(long, help = "process to attach to, defaults to the file name")]
        exe: Option<String>,
        #[arg(long, help = "write the toml here instead of printing it")]
        out: Option<PathBuf>,
    },
    /// Running processes.
    Ps {
        #[arg(help = "only show names containing this")]
        filter: Option<String>,
    },
    /// Read a table over without the game running.
    Check {
        #[arg(help = "table files, or folders of them")]
        paths: Vec<PathBuf>,
        #[arg(long, help = "only say something when one is wrong")]
        quiet: bool,
        #[arg(long, help = "one json object per table, for piping somewhere")]
        json: bool,
    },
    /// Whether a table's signatures are in your copy of the game.
    Fit {
        #[arg(help = "table files, or folders of them")]
        paths: Vec<PathBuf>,
        #[arg(long, help = "the game executable to check them against")]
        exe: PathBuf,
        #[arg(long, help = "only say something when one does not fit")]
        quiet: bool,
    },
    /// What a table offers for a running game, and whether each part works.
    Cheats {
        #[arg(long)]
        table: PathBuf,
        #[arg(long, help = "override the executable name in the table")]
        process: Option<String>,
    },
    /// Turn cheats on and hold them until you press enter.
    On {
        #[arg(long)]
        table: PathBuf,
        #[arg(long)]
        process: Option<String>,
        #[arg(help = "cheat ids, or 'all'")]
        ids: Vec<String>,
    },
    /// Find an address by searching, narrowing, and searching again.
    Scan {
        #[arg(long)]
        process: String,
        #[arg(long, default_value = "i32")]
        r#type: String,
        #[arg(long, help = "starting value, omit if you cannot see the number")]
        value: Option<String>,
    },
    /// What other people have shared for a game.
    Browse {
        #[arg(help = "the game's executable, e.g. game.exe")]
        exe: String,
        #[arg(long, default_value = "", help = "prefer tables checked on this build")]
        build: String,
        #[arg(
            long,
            default_value = "best",
            help = "best, votes, downloads, new, old, cheats"
        )]
        sort: String,
    },
    /// Send a table so everybody else gets it.
    Share {
        #[arg(help = "path to a freeplay table")]
        table: PathBuf,
        #[arg(long, help = "share without a name on it")]
        anonymous: bool,
        #[arg(
            long,
            default_value = "",
            help = "the game build you checked it against"
        )]
        build: String,
    },
    /// The name your uploads go out under.
    Whoami,
    /// Claim a name nobody else can publish under.
    Claim {
        #[arg(help = "letters, numbers, dot, dash, underscore")]
        name: String,
    },
    /// Get a name back on another machine.
    Recover {
        #[arg(help = "the name")]
        name: String,
        #[arg(help = "the words you wrote down")]
        phrase: Vec<String>,
    },
    /// Say whether a shared table worked.
    Rate {
        #[arg(help = "the id browse printed")]
        id: i64,
        #[arg(long, help = "it did not work")]
        down: bool,
        #[arg(long, default_value = "", help = "your install id")]
        install: String,
    },
    /// Read a value at an address.
    Read {
        #[arg(long)]
        process: String,
        #[arg(help = "hex address, e.g. 0x7ff6a1b2c3d4")]
        address: String,
        #[arg(long, default_value = "i32")]
        r#type: String,
    },
}

fn main() {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_max_level(if cli.verbose {
            tracing::Level::DEBUG
        } else {
            tracing::Level::WARN
        })
        .without_time()
        .init();

    if let Err(message) = run(cli.command) {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Games { all } => games(all),
        Command::Play { name, dry_run } => play(&name, dry_run),
        Command::Import { file, exe, out } => import(&file, exe.as_deref(), out.as_deref()),
        Command::Ps { filter } => list_processes(filter.as_deref()),
        Command::Check { paths, quiet, json } => check(&paths, quiet, json),
        Command::Fit { paths, exe, quiet } => fit(&paths, &exe, quiet),
        Command::Cheats { table, process } => cheats(&table, process.as_deref()),
        Command::On {
            table,
            process,
            ids,
        } => turn_on(&table, process.as_deref(), &ids),
        Command::Scan {
            process,
            r#type,
            value,
        } => scan(&process, &r#type, value.as_deref()),
        Command::Browse { exe, build, sort } => browse(&exe, &build, &sort),
        Command::Share {
            table,
            anonymous,
            build,
        } => share(&table, anonymous, &build),
        Command::Whoami => whoami(),
        Command::Claim { name } => claim(&name),
        Command::Recover { name, phrase } => recover(&name, &phrase.join(" ")),
        Command::Rate { id, down, install } => rate(id, !down, &install),
        Command::Read {
            process,
            address,
            r#type,
        } => read(&process, &address, &r#type),
    }
}

fn service<'a>(
    wire: &'a freeplay_sync::community::Live,
) -> freeplay_sync::community::Community<'a> {
    let endpoint = std::env::var("FREEPLAY_SERVICE")
        .unwrap_or_else(|_| freeplay_sync::community::ENDPOINT.to_string());
    freeplay_sync::community::Community::new(&endpoint, wire)
}

fn browse(exe: &str, build: &str, sort: &str) -> Result<(), String> {
    let sort: freeplay_sync::community::Sort = sort.parse()?;
    let wire = freeplay_sync::community::Live;
    let found = service(&wire).list_by(exe, build, sort)?;

    if found.is_empty() {
        println!("nothing shared for {exe} yet");
        return Ok(());
    }

    println!("{} table(s) for {exe}\n", found.len());
    for row in &found {
        let mark = if row.built_for == build && !build.is_empty() {
            "*"
        } else {
            " "
        };
        println!("{mark} {:<5} {}", row.id, row.game);
        println!("        {}", row.standing());
        if !row.built_for.is_empty() {
            println!("        checked on {}", row.built_for);
        }
    }
    if !build.is_empty() {
        println!("\n* means somebody used it on {build}");
    }
    Ok(())
}

fn identity_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("freeplay").join("identity.json")
}

fn me() -> Result<Option<freeplay_id::Identity>, String> {
    freeplay_id::Identity::load(&identity_path())
}

fn whoami() -> Result<(), String> {
    match me()? {
        Some(who) => {
            println!("{}", who.name);
            println!("key {}", who.public());
            println!("kept in {}", identity_path().display());
        }
        None => {
            println!("no name yet, uploads go out anonymous. claim one with: freeplay claim <name>")
        }
    }
    Ok(())
}

fn claim(name: &str) -> Result<(), String> {
    if let Some(already) = me()? {
        return Err(format!(
            "this machine already publishes as {}. delete {} first if you meant to start again",
            already.name,
            identity_path().display()
        ));
    }

    let wire = freeplay_sync::community::Live;
    if service(&wire).taken(name)? {
        return Err(format!("{name} belongs to somebody else already"));
    }

    let who = freeplay_id::Identity::create(name)?;
    who.save(&identity_path())?;

    println!("{name} is yours, and it is not registered until your first upload.\n");
    println!("write these down. they are the only way back if this machine dies:\n");
    for (row, chunk) in who.phrase().words().chunks(6).enumerate() {
        println!("  {:>2}. {}", row * 6 + 1, chunk.join("  "));
    }
    println!("\nthere is no password and no reset. lose the words, lose the name.");
    Ok(())
}

fn recover(name: &str, phrase: &str) -> Result<(), String> {
    let who = freeplay_id::Identity::recover(name, phrase)?;
    who.save(&identity_path())?;
    println!("{} is back, key {}", who.name, who.public());
    Ok(())
}

fn share(path: &PathBuf, anonymous: bool, build: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("could not read it: {e}"))?;
    let table = Table::parse(&text).map_err(|e| e.to_string())?;
    table.validate()?;

    let who = if anonymous { None } else { me()? };
    let wire = freeplay_sync::community::Live;
    let sent = service(&wire).submit(&table, &text, who.as_ref(), build)?;

    let by = match &who {
        Some(who) => format!(" as {}", who.name),
        None => " anonymously".to_string(),
    };

    if sent.already {
        println!("already shared, it is number {}", sent.id);
    } else {
        println!("shared {}{by}, number {}", table.game.name, sent.id);
    }
    Ok(())
}

fn rate(id: i64, up: bool, install: &str) -> Result<(), String> {
    let install = if install.is_empty() {
        freeplay_sync::community::new_install_id(std::process::id() as u128)
    } else {
        install.to_string()
    };

    // the cli votes on a table by id without a game in front of it, so it has
    // nothing to say about which executable it was used on
    let wire = freeplay_sync::community::Live;
    service(&wire).vote(id, &install, up, "", "")?;
    println!(
        "{} table {id}",
        if up { "recommended" } else { "marked down" }
    );
    Ok(())
}

fn attach(name: &str) -> Result<WindowsTarget, String> {
    WindowsTarget::attach_by_name(name).map_err(|e| match e {
        CoreError::ProcessNotFound(_) => format!("{name} is not running"),
        CoreError::Protected { process, guard } => {
            format!("{process} is running {guard}. Freeplay is for single player games only")
        }
        CoreError::OpenFailed { .. } => {
            format!("could not open {name}. Try running freeplay as administrator")
        }
        other => other.to_string(),
    })
}

fn parse_type(text: &str) -> Result<ValueKind, String> {
    text.parse::<ValueKind>()
}

fn parse_address(text: &str) -> Result<usize, String> {
    let trimmed = text
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    usize::from_str_radix(trimmed, 16).map_err(|_| format!("bad address {text:?}"))
}

fn games(all: bool) -> Result<(), String> {
    let found = discover();
    if found.is_empty() {
        println!("No games found. Steam, Epic and GOG are checked.");
        return Ok(());
    }

    if all {
        for game in &found {
            println!("{} [{}]", game.name, game.store.label());
            println!("  {}", game.install_dir.display());
            for (index, exe) in game.executables.iter().enumerate() {
                let mark = if index == 0 { "*" } else { " " };
                let relative = exe.strip_prefix(&game.install_dir).unwrap_or(exe);
                println!("  {mark} {}", relative.display());
            }
            println!();
        }
        return Ok(());
    }

    println!("{:<44} {:<7} EXECUTABLE", "GAME", "STORE");
    for game in &found {
        println!(
            "{:<44} {:<7} {}",
            truncate(&game.name, 44),
            game.store.label(),
            game.main_exe().unwrap_or_else(|| "?".into())
        );
    }
    println!("\n{} games", found.len());
    Ok(())
}

fn play(name: &str, dry_run: bool) -> Result<(), String> {
    let needle = name.to_lowercase();
    let found = discover();
    let game = found
        .iter()
        .find(|g| g.name.to_lowercase().contains(&needle))
        .ok_or_else(|| format!("no installed game matches {name:?}"))?;

    let plan = freeplay_library::launch::plan(game)
        .ok_or_else(|| format!("nothing to start for {}", game.name))?;

    let what = match &plan {
        freeplay_library::launch::Launch::Url(url) => url.clone(),
        freeplay_library::launch::Launch::Exe(exe) => exe.display().to_string(),
    };

    if dry_run {
        println!(
            "{}
  would run {what}",
            game.name
        );
        return Ok(());
    }

    freeplay_library::launch::start(game)?;
    println!(
        "{}
  started via {what}",
        game.name
    );
    Ok(())
}

fn import(file: &PathBuf, exe: Option<&str>, out: Option<&Path>) -> Result<(), String> {
    let xml = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let exe = exe.map(str::to_string).unwrap_or_else(|| {
        if stem.to_lowercase().ends_with(".exe") {
            stem.clone()
        } else {
            format!("{stem}.exe")
        }
    });

    let title = stem.trim_end_matches(".exe").trim_end_matches(".EXE");
    let imported = freeplay_table::cheatengine::import(&xml, &exe, title)?;

    for skip in &imported.skipped {
        eprintln!("  skipped {:<34} {}", truncate(&skip.name, 34), skip.why);
    }
    eprintln!(
        "
{}",
        imported.summary()
    );

    if imported.table.cheats.is_empty() {
        return Err("nothing in that file could be imported".into());
    }

    let toml = toml::to_string_pretty(&imported.table).map_err(|e| e.to_string())?;
    match out {
        Some(path) => {
            std::fs::write(path, toml).map_err(|e| format!("{}: {e}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => println!("{toml}"),
    }
    Ok(())
}

// everything that can be said about a table with no game in front of you:
// does it parse, does it hold together, and would the scripts be refused
// actually run the assembler over a script rather than only parsing it.
//
// parsing catches a malformed script. it says nothing about whether the
// instructions inside can be encoded, and that is where cheat engine tables
// really break: a cast, a jump hint, a mnemonic nobody implemented. those only
// showed up when somebody launched the game and armed the cheat, which is a
// terrible place to find out.
//
// no process is needed for this. addresses are made up and every symbol the
// script never defines is stubbed, so what is left is genuine syntax the
// assembler cannot encode.
fn will_it_assemble(script: &freeplay_aa::Script) -> Vec<String> {
    use freeplay_asm::{assemble, AsmError, Bits};

    let mut found = Vec::new();
    for half in [&script.enable, &script.disable] {
        for section in &half.sections {
            // both, because a table is written for one architecture and
            // nothing here says which. only a failure under both is real
            // no process here, so readmem gets nops of the right length. the
            // point is whether the rest of the section encodes, not what the
            // bytes turn out to be
            let body = freeplay_aa::expand_readmem(&section.body, |_, _| None);

            let complained = [Bits::X64, Bits::X86].map(|bits| {
                let mut symbols: HashMap<String, u64> = HashMap::new();
                // feeding undefined symbols back in converges: every pass
                // either names one more or fails on something that is not a
                // symbol at all
                for _ in 0..64 {
                    match assemble(&body, 0x1000_0000, bits, &symbols) {
                        Ok(_) => return None,
                        Err(e) => match deepest(&e) {
                            AsmError::UndefinedSymbol(name) => {
                                symbols.insert(name.clone(), 0x2000_0000);
                            }
                            // a made up origin can put a jump out of reach,
                            // which says nothing about the script
                            AsmError::OutOfRange { .. } => return None,
                            _ => return Some(format!("{e}{}", blamed(&body, &e))),
                        },
                    }
                }
                None
            });

            if let [Some(a), Some(_)] = &complained {
                found.push(a.clone());
            }
        }
    }
    found
}

fn deepest(e: &freeplay_asm::AsmError) -> &freeplay_asm::AsmError {
    match e {
        freeplay_asm::AsmError::At { source, .. } => deepest(source),
        other => other,
    }
}

// a line number on its own means opening the table to find out what is on it
fn blamed(body: &str, e: &freeplay_asm::AsmError) -> String {
    let freeplay_asm::AsmError::At { line, .. } = e else {
        return String::new();
    };
    match body.lines().nth(line.saturating_sub(1)) {
        Some(text) => format!("   <- {}", text.trim()),
        None => String::new(),
    }
}

// every table file named, or every one in a folder
fn table_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
                })
                .collect();
            found.sort();
            files.extend(found);
        } else {
            files.push(path.clone());
        }
    }
    if files.is_empty() {
        return Err("name a table file or a folder with some in".into());
    }
    Ok(files)
}

fn fit(paths: &[PathBuf], exe: &Path, quiet: bool) -> Result<(), String> {
    let code = freeplay_library::pe::code(exe)
        .ok_or_else(|| format!("{} has no code section to read", exe.display()))?;
    println!(
        "{} {}-bit, {} bytes of code at {:#x}",
        exe.file_name().unwrap_or(exe.as_os_str()).to_string_lossy(),
        code.bits,
        code.bytes.len(),
        code.rva
    );

    let mut wrong = 0usize;
    for file in table_files(paths)? {
        let Ok(table) = Table::load(&file) else {
            continue;
        };
        let mut whole = freeplay_aa::fit::Fit::default();
        for cheat in &table.cheats {
            let freeplay_table::schema::Action::Script { source } = &cheat.action else {
                continue;
            };
            let part = freeplay_aa::fit::of_script(
                source,
                &freeplay_aa::fit::Code {
                    bytes: &code.bytes,
                    rva: code.rva,
                },
            );
            whole.signatures.extend(part.signatures);
            whole.stale.extend(part.stale);
        }
        if whole.is_empty() {
            if !quiet {
                println!("  {:<38} nothing to scan for", table.game.name);
            }
            continue;
        }

        let total = whole.signatures.len();
        let bad = whole.missing() > 0 || !whole.stale.is_empty();
        if bad {
            wrong += 1;
        }
        if quiet && !bad {
            continue;
        }
        println!(
            "  {:<38} {} of {} signatures{}",
            table.game.name,
            whole.found(),
            total,
            if whole.unknown() > 0 {
                format!(", {} search outside the exe", whole.unknown())
            } else {
                String::new()
            }
        );
        if whole.ambiguous() > 0 {
            println!("      {} match in more than one place", whole.ambiguous());
        }
        for stale in &whole.stale {
            println!(
                "      {} jumps to {:#x} but the branch it replaces goes to {:#x}",
                stale.symbol, stale.wants, stale.goes
            );
        }
    }

    if wrong > 0 {
        println!();
        println!("{wrong} do not fit this build");
    }
    Ok(())
}

fn check(paths: &[PathBuf], quiet: bool, json: bool) -> Result<(), String> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
                })
                .collect();
            found.sort();
            files.extend(found);
        } else {
            files.push(path.clone());
        }
    }
    if files.is_empty() {
        return Err("name a table file or a folder with some in".into());
    }

    let (mut bad, mut cheats, mut scripts) = (0usize, 0usize, 0usize);
    for file in &files {
        let short = file
            .file_name()
            .unwrap_or(file.as_os_str())
            .to_string_lossy();
        let table = match Table::load(file) {
            Ok(table) => table,
            Err(e) => {
                println!("{short}: {e}");
                bad += 1;
                continue;
            }
        };

        let mut trouble: Vec<String> = Vec::new();
        if let Err(e) = table.validate() {
            trouble.push(e);
        }
        for cheat in &table.cheats {
            if let freeplay_table::schema::Action::Script { source } = &cheat.action {
                scripts += 1;
                match freeplay_aa::parse(source) {
                    Ok(script) => {
                        trouble.extend(
                            freeplay_aa::safety::check(&script)
                                .iter()
                                .map(|r| format!("{}: {r}", cheat.id)),
                        );
                        trouble.extend(
                            will_it_assemble(&script)
                                .iter()
                                .map(|r| format!("{}: {r}", cheat.id)),
                        );
                    }
                    Err(e) => trouble.push(format!("{}: {e}", cheat.id)),
                }
            }
        }

        cheats += table.cheats.len();
        if !trouble.is_empty() {
            bad += 1;
            if !json {
                println!("{}", truncate(&short, 40));
                for why in &trouble {
                    println!("    {why}");
                }
            }
            continue;
        }

        if json {
            // one object a line, so the whole folder pipes into something else
            println!(
                r#"{{"file":{},"exe":{},"game":{},"author":{},"cheats":{},"scripts":{},"fingerprint":"{}"}}"#,
                quoted(&short),
                quoted(&table.game.exe),
                quoted(&table.game.name),
                quoted(&table.game.author),
                table.cheats.len(),
                table.cheats.iter().filter(|c| c.action.is_script()).count(),
                freeplay_table::fingerprint::fingerprint(&table),
            );
        } else if !quiet {
            println!(
                "{:<40} {:>4} cheats  {}",
                truncate(&short, 40),
                table.cheats.len(),
                table.game.exe
            );
        }
    }

    if json {
        if bad > 0 {
            return Err(format!("{bad} did not check out"));
        }
        return Ok(());
    }

    println!(
        "\n{} tables, {cheats} cheats, {scripts} scripts, {bad} with something wrong",
        files.len()
    );
    if bad > 0 {
        return Err(format!("{bad} did not check out"));
    }
    Ok(())
}

fn list_processes(filter: Option<&str>) -> Result<(), String> {
    let mut all = processes().map_err(|e| e.to_string())?;
    if let Some(needle) = filter {
        let needle = needle.to_lowercase();
        all.retain(|p| p.name.to_lowercase().contains(&needle));
    }
    all.sort_by_key(|a| a.name.to_lowercase());

    for process in &all {
        println!("{:>8}  {}", process.pid, process.name);
    }
    println!("\n{} processes", all.len());
    Ok(())
}

fn load(path: &PathBuf) -> Result<Table, String> {
    Table::load(path).map_err(|e| e.to_string())
}

fn cheats(path: &PathBuf, process: Option<&str>) -> Result<(), String> {
    let table = load(path)?;
    let exe = process.unwrap_or(&table.game.exe).to_string();
    let target = attach(&exe)?;

    println!("{} ({})\n", table.game.name, exe);
    let symbols = freeplay_table::resolve::Symbols::new();
    for cheat in &table.cheats {
        let state = match &cheat.locator {
            Some(locator) => freeplay_table::resolve::evaluate_with(&target, locator, &symbols),
            None => State::Ready { addr: 0 },
        };
        let (mark, note) = match &state {
            State::Ready { addr } => ("ready".to_string(), format!("{addr:#x}")),
            State::Unavailable { reason } => ("wait".to_string(), reason.clone()),
            State::Broken { reason } => ("broken".to_string(), reason.clone()),
        };
        println!("  [{mark:<6}] {:<28} {}", cheat.name, note);
        if !cheat.hint.is_empty() && !state.is_ready() {
            println!("            {}", cheat.hint);
        }
    }
    Ok(())
}

fn turn_on(path: &PathBuf, process: Option<&str>, ids: &[String]) -> Result<(), String> {
    let table = load(path)?;
    let exe = process.unwrap_or(&table.game.exe).to_string();
    let target: Arc<dyn Target> = Arc::new(attach(&exe)?);

    let wanted: Vec<String> = if ids.iter().any(|i| i == "all") {
        table.cheats.iter().map(|c| c.id.clone()).collect()
    } else {
        ids.to_vec()
    };
    if wanted.is_empty() {
        return Err("name at least one cheat, or 'all'".into());
    }

    let mut session = Session::new(target, table);
    for id in &wanted {
        match session.enable(id) {
            Ok(()) => println!("on  {id}"),
            Err(e) => println!("skip {id}: {e}"),
        }
    }

    if session.active_ids().is_empty() {
        return Err("nothing could be turned on".into());
    }

    session.start();
    println!(
        "\nHolding {} cheats. Press enter to stop.",
        session.active_ids().len()
    );
    let _ = io::stdin().lock().read_line(&mut String::new());

    session.stop();
    session.disable_all();
    println!("stopped, everything put back");
    Ok(())
}

fn scan(process: &str, type_name: &str, value: Option<&str>) -> Result<(), String> {
    use freeplay_core::search::{Filter, Search};

    let kind = parse_type(type_name)?;
    let target = attach(process)?;

    let first = match value {
        Some(text) => {
            let scalar = kind
                .parse(text)
                .ok_or_else(|| format!("bad {kind} value {text:?}"))?;
            Filter::Exact(scalar)
        }
        None => Filter::Unknown,
    };

    println!("scanning {process} for {kind} values...");
    let mut search = Search::first(&target, kind, first).map_err(|e| e.to_string())?;
    report(&search);

    let stdin = io::stdin();
    loop {
        print!("\nnext [value | changed | unchanged | up | down | list | quit] > ");
        io::stdout().flush().ok();

        let mut line = String::new();
        if stdin.lock().read_line(&mut line).is_err() {
            break;
        }
        let command = line.trim();

        let filter = match command {
            "" => continue,
            "quit" | "q" => break,
            "list" | "l" => {
                for candidate in search.results(20) {
                    println!("  {:#018x}  {}", candidate.addr, candidate.value);
                }
                continue;
            }
            "changed" | "c" => Filter::Changed,
            "unchanged" | "u" => Filter::Unchanged,
            "up" | "increased" => Filter::Increased,
            "down" | "decreased" => Filter::Decreased,
            other => match kind.parse(other) {
                Some(scalar) => Filter::Exact(scalar),
                None => {
                    println!("  did not understand {other:?}");
                    continue;
                }
            },
        };

        search.next(&target, filter).map_err(|e| e.to_string())?;
        report(&search);

        if search.len() <= 8 {
            for candidate in search.results(8) {
                println!("  {:#018x}  {}", candidate.addr, candidate.value);
            }
        }
        if search.is_empty() {
            println!("  nothing left, start again");
            break;
        }
    }
    Ok(())
}

fn report(search: &freeplay_core::search::Search) {
    let count = search.len();
    let noun = if count == 1 { "address" } else { "addresses" };
    println!("  round {}: {count} {noun}", search.rounds());
}

fn read(process: &str, address: &str, type_name: &str) -> Result<(), String> {
    let kind = parse_type(type_name)?;
    let addr = parse_address(address)?;
    let target = attach(process)?;

    let value: Scalar = target.read_scalar(addr, kind).map_err(|e| e.to_string())?;
    println!("{addr:#018x}  {kind}  {value}");
    Ok(())
}

fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        let cut: String = text.chars().take(width.saturating_sub(1)).collect();
        format!("{cut}~")
    }
}
