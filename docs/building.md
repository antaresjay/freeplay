# Building it yourself

Nothing here is worth trusting a stranger's binary for, and you do not need a
Rust toolchain to avoid one. Fork the repository, open **Actions**, pick
**release**, press **Run workflow**. About ten minutes later the run has a
`freeplay-windows-x64` artifact with the same three files in it, built from the
code you are looking at, on a machine that is not mine.

Locally, if you have Rust:

```
cargo run --release -p freeplay-app       # just run it

cargo install tauri-cli --version "^2"    # once, if you want the installer
cd app && cargo tauri build
```

The installer lands in `target/release/bundle/nsis/`. The first bundle
downloads NSIS, which is a few megabytes and can time out on a slow line: run
it again and it picks up where it left off.

Build in release either way. Finding your games walks every install directory
and scanning a game reads gigabytes of its memory, and both are several times
slower in a debug build.

## Tests

```
cargo test
python app/uitest/clickthrough.py
python app/uitest/overlay.py
python app/uitest/layout.py
```

Nothing type checks the front end, so the Python scripts drive it in headless
Edge behind a fake Tauri bridge and fail if a control does nothing or anything
throws. They have caught more real bugs than the unit tests have.

The last one measures rather than clicks. It records the position and size of
every element before and after switching game and prints whatever moved, which
is how the layout stays still instead of sliding around by a scrollbar width.

## The command line tool

The quickest way to try the engine without the desktop app:

```
cargo run --release --bin freeplay -- games
cargo run --release --bin freeplay -- ps
cargo run --release --bin freeplay -- scan --process game.exe --type i32 --value 100
```

`scan` is interactive. Search for what you can see on screen, change it in game,
then type the new value or one of `up`, `down`, `changed`, `unchanged` to narrow
the list until one address is left.

`check` reads a table over without the game running, which is the fastest way
to find out whether one you wrote or converted actually holds together. It
parses every script, refuses the ones that reach outside the process, and takes
a folder as happily as a file.

```
cargo run --release --bin freeplay -- check tables/
cargo run --release --bin freeplay -- check mytable.toml --json
```
