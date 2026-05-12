# Freeplay

A game trainer for Windows that is free, open source, and does not want your
email address.

Named after the arcade cabinet setting that gives you unlimited credits without
feeding coins into the slot.

## Why I built this

I play a lot of single player games and I use trainers. Infinite money in a
Witcher game, skipping a mission timer I have already failed six times, giving
myself the gear I cannot be bothered to grind for. Nobody else is affected. It
is my save file and my evening.

Every way of doing that is bad in a different way.

WeMod works well and looks good, but it is closed source, it wants an account,
and the parts you actually want sit behind a subscription. Paying monthly to
give myself infinite arrows in a game I already bought feels ridiculous.

The free trainers are single exe files from sites buried in ads, uploaded by
someone you have never heard of, and the instructions tell you to disable your
antivirus and run them as administrator so they can write into another
program's memory. I am not doing that.

Cheat Engine is open source and can do anything, which is also the problem. It
is a tool for people who already know what a pointer chain is. Most people just
want to tick a box that says infinite health.

So there is a gap. Something as easy as WeMod, as open as Cheat Engine, free,
and not asking you to log in.

Freeplay is open source specifically because of what it does. A program that
attaches to your games and writes into their memory is a program you should be
able to read before you trust it. That is not a marketing line, it is the whole
reason the source is here.

## What it does

- Finds games installed through Steam, Epic and GOG, and shows which are running
- Shows your library with real cover art, read from what Steam already cached on
  your own disk. Freeplay makes no network requests at all
- Marks games that ship an anti-cheat before you click them, rather than letting
  you find out at the point of refusal
- Attaches to a game and lists the cheats available for it
- Only offers cheats that work right now. Anything that cannot resolve is greyed
  out with the reason, instead of being a toggle that silently does nothing
- Finds values yourself: search for 100, take damage, search again, keep going
  until one address is left
- Freezes values, sets them once, or patches the instruction that changes them
- Saves what you found as a small readable table file you can share

## Single player only

Freeplay refuses to attach to any process running an anti-cheat, and that check
is in the code rather than in a disclaimer nobody reads. EasyAntiCheat,
BattlEye, Vanguard, GameGuard, XIGNCODE, PunkBuster, Denuvo Anti-Cheat and
others are all refused before the process handle is used for anything.

Two reasons. Cheating in multiplayer ruins the game for people who did nothing
to deserve it. And it gets accounts banned, which is a miserable thing to happen
to somebody because a tool let them.

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

## Layout

| Crate | What it does |
| --- | --- |
| `freeplay-core` | Process access, scanning, pointer chains, patching |
| `freeplay-table` | Table format and turning a locator into an address |
| `freeplay-session` | An attached game with cheats held on |
| `freeplay-library` | Finding installed games |
| `freeplay-cli` | Command line driver |
| `app` | Tauri desktop application |

Every Windows API call lives in one file, `freeplay-core/src/windows_target.rs`,
behind a trait. Reading another process on Linux is `process_vm_readv`, so
Steam Deck support later is a new module rather than a rewrite.

```
cargo test
```

## License

MIT.
