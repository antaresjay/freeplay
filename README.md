# Freeplay

Freeplay is an open source Windows game modification framework written in Rust.
It discovers installed games, attaches to single player processes, scans memory,
resolves pointer chains and applies configurable runtime modifications, through
either a native desktop application or a command line tool.

A game is described by a TOML table saying where its values live and what to do
with them, so adding a game needs no Rust and no release. Cheat Engine `.CT`
files convert straight into that format, including their Auto Assembler
scripts: Freeplay ships its own x86 and x64 assembler, allocates code caves in
the target and hooks instructions, which is what those scripts need to run.

It is free, it does not want your email address, and it refuses to attach to
anything running an anti-cheat.

Named after the arcade cabinet setting that gives you unlimited credits without
feeding coins into the slot.

## Architecture

```
              app (Tauri desktop)          freeplay-cli
                       └─────────┬──────────────┘
      ┌────────────────┬─────────┴─────┬─────────────────┐
      │                │               │                 │
freeplay-sync   freeplay-session   freeplay-library   freeplay-table
      │                │                                 │
      │           freeplay-aa ──────── freeplay-asm      │
      │                │                                 │
      └────────────────┴────────┬────────────────────────┘
                                │
                          freeplay-core
                                │
                         windows_target.rs
```

Every Windows API call lives in one file behind a trait, so nothing above
`freeplay-core` knows it is running on Windows. Reading another process on Linux
is `process_vm_readv`, so Steam Deck support later is a new module rather than a
rewrite.

`freeplay-library` sits apart on purpose. Finding installed games reads store
metadata off your own disk and never touches a process, so it has no reason to
depend on the memory engine and does not.

| Crate | What it does |
| --- | --- |
| `freeplay-core` | Process access, scanning, pointer chains, patching, allocation |
| `freeplay-asm` | x86 and x64 assembler |
| `freeplay-aa` | Cheat Engine Auto Assembler: scans, code caves, hooks |
| `freeplay-table` | Table format, `.CT` import, turning a locator into an address |
| `freeplay-session` | An attached game with cheats held on |
| `freeplay-library` | Finding installed games |
| `freeplay-sync` | Fetching published tables |
| `freeplay-cli` | Command line driver |
| `app` | Tauri desktop application |

## What it does

- Finds games installed through Steam, Epic and GOG, and shows which are running
- Shows your library with real cover art, play time and when you last played,
  all read from what Steam already cached on your own disk
- Downloads new cheat tables by itself, so a game somebody adds today works for
  everybody tomorrow without anyone hunting for a file
- A page per game: art, play time, launch it, pin it to the top, favourite it
- Marks games that ship an anti-cheat before you click them, rather than letting
  you find out at the point of refusal
- Light and dark, follows Windows by default, and five accents if you dislike
  the one I picked
- Attaches to a game and lists the cheats available for it, 32-bit or 64-bit,
  since plenty of the games worth cheating in are still 32-bit
- Shows what a game's table holds before you attach, so you can see whether it
  is worth starting
- Only offers cheats that work right now. Anything that cannot resolve is greyed
  out with the reason, instead of being a toggle that silently does nothing
- Finds values yourself: search for 100, take damage, search again, keep going
  until one address is left
- Freezes values, sets them once, patches the instruction that changes them, or
  runs a Cheat Engine script and puts everything back when you switch it off
- Saves what you found as a small readable table file you can share

## Single player only

Freeplay refuses to attach to any process running an anti-cheat, and that check
is in the code rather than in a disclaimer nobody reads. EasyAntiCheat,
BattlEye, Vanguard, GameGuard, XIGNCODE, PunkBuster, Denuvo Anti-Cheat and
others are all refused before the process handle is used for anything.

Two reasons. Cheating in multiplayer ruins the game for people who did nothing
to deserve it. And it gets accounts banned, which is a miserable thing to happen
to somebody because a tool let them.

## Why I built this

I mostly use trainers on a second playthrough. I like finishing a game properly
first, then on the way back through I would rather skip the grind I have already
done, or get past the one section that beat me six times.

The existing options are all decent, they just did not fit what I wanted. WeMod
is polished, but it wants an account and the useful parts are behind a
subscription. The free trainers are single exe files from sites I do not know,
and I am wary of giving one of those permission to write into another program's
memory. Cheat Engine does all of this and far more, but it assumes you already
know what a pointer chain is.

So I wanted something in between: as easy as WeMod, as open as Cheat Engine, no
login.

Being open source matters more than usual here. A program that attaches to your
games and writes into their memory is one you should be able to read first.

## Running it

```
git clone https://github.com/antaresjay/freeplay
cd freeplay
cargo run --release -p freeplay-app
```

Build it in release. Finding your games means walking every install directory,
and scanning a running game means reading gigabytes of its memory. Both are
several times slower in a debug build.

There is also a command line tool, which is the quickest way to try the engine:

```
cargo run --release --bin freeplay -- games
cargo run --release --bin freeplay -- ps witcher
cargo run --release --bin freeplay -- scan --process witcher2.exe --type i32 --value 100
```

`scan` is interactive. Search for what you can see on screen, change it in game,
then type the new value or one of `up`, `down`, `changed`, `unchanged` to narrow
the list until one address is left.

## Things you should know

**Windows will complain.** The binary is unsigned, so SmartScreen warns on first
run. Windows Defender may flag it too, because reading and writing another
process's memory is exactly what malware does. There is no way around that
short of a code signing certificate costing a few hundred pounds a year. Build
it yourself if you would rather not trust a download.

**You will probably need to run it as administrator.** Opening a handle to
another process usually requires it.

**Freeplay does not ship cheats for many games.** It ships the engine and the
format. Somebody has to sit with a debugger and work out where a game keeps its
numbers, and that has to be redone when the game patches. The value finder is
there so you can do that yourself in a few minutes rather than an afternoon.

I am not going to pretend otherwise by shipping a pile of tables I have never
run. A toggle that silently does nothing is worse than no toggle.

**Cheat Engine tables work, including the scripts.** Drop a `.CT` in `tables/`
named after the process and Freeplay converts it: addresses, pointer chains,
types, groups, and Auto Assembler.

That last part is most of the work. Almost every table worth having is built the
same way: a script scans for an instruction, allocates a cave next to it, writes
a jump over it, and copies whatever register held the player into a slot. Every
value entry then hangs off that slot's name rather than off an address. So
Freeplay has an assembler, allocates inside the target, and hooks instructions.
Without that, a table like aSwedishMagyar's Witcher 2 one imports nothing at
all, because all 23 of its entries depend on it.

See [tables/README.md](tables/README.md).

## What it sends

Nothing about you. There is no account, no identifier, no telemetry, and no
crash reporting.

The one thing Freeplay does over the network is fetch cheat tables: a GET of
`tables/index.json` from this repository, then a GET of any table file that is
new or has changed. That is it, one host, read only, over https, using the
certificate store Windows already has. Turn it off in settings and Freeplay
never opens a socket, and runs on whatever is on your disk.

I built it without any network at first and it was the wrong call. It meant
every new game was somebody manually finding a file and putting it in a folder,
which is not a trainer, it is homework.

## What it does not do

No genre, and no "installed on" beyond the store name. Steam only keeps genre
in `appinfo.vdf`, which is a binary format, and reading it properly is more
work than a line of text is worth.

Settings live in `%APPDATA%\freeplay\settings.json`. Downloaded tables live
next to it in `%APPDATA%\freeplay\tables\`.

## Tables

A table is a TOML file describing one game. Nothing in the code knows about any
specific game, so adding one needs no Rust and no release.

```toml
[game]
name = "Some Game"
exe  = "somegame.exe"

[[cheat]]
id = "infinite-health"
name = "Infinite Health"
category = "player"
type = "freeze"
value_type = "f32"
value = 1000

[cheat.locator]
find = "pattern"
pattern = "48 8B 05 ?? ?? ?? ??"
offset = 3
hops = ["+0x28", "+0x1F0"]
```

See [tables/README.md](tables/README.md) for the full format, including code
patching and rip-relative operands.

## Tests

```
cargo test
```

## License

MIT.
