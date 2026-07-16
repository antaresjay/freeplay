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

## Get it

[Releases](https://github.com/antaresjay/freeplay/releases) has three files.

| File | What it is |
| --- | --- |
| `Freeplay_x.y.z_x64-setup.exe` | Installer. Goes in your own user folder, no admin prompt, uninstalls from Settings like anything else |
| `Freeplay.exe` | The same program, nothing to install. Put it where you like |
| `freeplay-cli.exe` | The command line one |

Neither is signed, so SmartScreen stops you the first time: **More info**, then
**Run anyway**. See [Things you should know](#things-you-should-know).

### Or build it yourself

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
| `freeplay-sync` | Fetching published tables, and the community service |
| `freeplay-id` | Signing, so a name cannot be taken by typing it |
| `freeplay-cli` | Command line driver |
| `app` | Tauri desktop application |

## What it does

- Finds games installed through Steam, Epic and GOG, and shows which are running
- Shows your library with real cover art, play time and when you last played,
  all read from what Steam already cached on your own disk
- Downloads new cheat tables by itself, so a game somebody adds today works for
  everybody tomorrow without anyone hunting for a file
- Shows what other people have shared for a game, sorted by what worked for
  them, and asks afterwards whether it worked for you
- A page per game: art, play time, launch it, pin it to the top, favourite it
- Marks games that ship an anti-cheat before you click them, rather than letting
  you find out at the point of refusal
- Light and dark, follows Windows by default, and five accents if you dislike
  the one I picked
- Attaches to a game and lists the cheats available for it, 32-bit or 64-bit,
  since plenty of the games worth cheating in are still 32-bit
- Shows what a game's table holds before you attach, so you can see whether it
  is worth starting
- Switch cheats on with the game closed. Freeplay attaches on its own when the
  game starts and holds them on as soon as you are far enough in for them to
  work, so you never alt-tab back to the app
- Cheats that take a number take a number. Carry weight, game speed and how
  much gold are not switches, and a table that freezes them at 999999 is a
  wrecked save rather than a cheat
- An overlay over the game, on a shortcut, for turning things on without
  alt-tabbing at all
- Remembers what you had on and what you typed, per game, across launches
- Says what each cheat is doing rather than pretending. On, waiting for the
  game, or not found in this build
- Finds values yourself: search for 100, take damage, search again, keep going
  until one address is left
- Freezes values, sets them once, patches the instruction that changes them, or
  runs a Cheat Engine script and puts everything back when you switch it off
- Saves what you found as a small readable table file you can share
- Carries everything to another machine in one file: your games, what you had
  on, the numbers you set, and which tables to pull back down

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

## Things you should know

**Windows will complain.** The binary is unsigned, so SmartScreen warns on first
run. Windows Defender may flag it too, because reading and writing another
process's memory is exactly what malware does. There is no way around that
short of a code signing certificate costing a few hundred pounds a year. Every
release is built by the workflow in this repository and the Actions tab shows
the run that produced it, so you can check the files came from this code. Or
build it yourself, which takes one click on a fork.

**Administrator is usually not needed.** A game you started yourself runs as
you, and one program of yours can open another. You need it in one case: if the
game runs elevated, Freeplay has to be elevated too, because a normal process
cannot open a handle to one above it.

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
Without that, plenty of tables import nothing at all, because every entry in
them depends on it.

See [tables/README.md](tables/README.md).

## The overlay

A panel pinned to the right edge of the game, on a shortcut, so switching a
cheat on does not mean alt-tabbing out of what you are doing. Same toggles and
number boxes as the main window.

It only ever appears over the game itself and goes away the moment you switch to
something else, so it cannot end up floating over a browser. It needs a game
attached with a table loaded, which also means it can never appear over a
multiplayer game, since Freeplay refuses to attach to one in the first place.

It is a borderless window on top of the game, not something drawn inside it.
That works for windowed and borderless and does nothing for exclusive
fullscreen. Drawing inside the game means injecting a DLL and hooking the swap
chain, which is the thing every anti-cheat looks for, so it is not worth it for
a program that will not go near those games anyway.

Turning it on lets Freeplay watch the keyboard, and that is worth being straight
about. `RegisterHotKey` is not enough: plenty of games install a low level
keyboard hook and swallow every key, which is why the Windows key stops working
inside them, and hooks run before hotkeys are looked at. So Freeplay installs
one of its own, and reinstalls it whenever it attaches to a game, because hooks
are called newest first. The callback compares the key against one combination
and returns. Nothing is recorded, stored or sent, it is only installed while the
overlay is turned on, and it is a few lines in `app/src-tauri/src/hotkey.rs`.

The default is `Ctrl+Shift+O`, because everything obvious is taken: `Alt+Z` and
`Alt+R` are NVIDIA, `Shift+Tab` is Steam, `Win+G` is the Xbox Game Bar, `F12` is
a Steam screenshot. Pick your own and Freeplay says if it knows who owns it.

## Sharing tables

If you work out a table that works, one button sends it and everybody else gets
it. They vote on whether it worked for them, which is what decides the order the
next person sees.

The question is asked after you close the game rather than over it, since the
middle of playing is the one moment nobody is looking at this window. It only
asks about a table that was actually running, and it notes which one that was
when the game starts, so swapping tables halfway through cannot confuse it. It
does not ask if you never switched anything on, and "not now" is a real answer
that buys two days of quiet.

Which table is offered first is worked out on your machine rather than by the
service, because only your machine knows which build of the game is installed.
A table checked against your build beats a popular one from two patches ago,
which usually resolves nothing at all. Votes go through a Wilson lower bound so
two out of two does not outrank ninety out of a hundred, and a table people say
is broken is pushed down rather than merely not lifted.

Two different claims are kept apart on purpose. **Author verified** means the
name is registered to a key and nobody else can publish under it. It says
nothing about whether the table works. Whether it works is what the votes and
the tested-on version are for, and Freeplay does not put its own name on any of
that, because nothing in the app checks a table functionally.

Names are optional and anonymous is the default. If you do want your name on
what you share, Freeplay makes a key and registers the name against it, so
nobody can publish under your name by typing it. There is no password and no
email, just seventeen words you write down once, which is also how the name
moves to another machine. Lose the words and the name is gone, and there is no
way around that: a secret cannot be recovered from nothing.

Everything shared is our own table format, never a raw `.CT`, so it has been
parsed and validated before it can reach anybody. A downloaded script may only
touch the game's own modules, and anything calling `loadlibrary` or spawning a
thread is refused outright. See `freeplay-aa/src/safety.rs`.

Tables land in a Cloudflare D1 database within seconds and are mirrored into
[freeplay-tables](https://github.com/antaresjay/freeplay-tables) every few
minutes, so the repository outlives the service and you can read every table on
GitHub without installing anything.

## What it sends

No email address, no password, no telemetry, no crash reporting, and nothing
about you or your machine. Claiming a name is optional, is a key rather than an
account, and nothing else changes if you never do it.

Every request Freeplay can make is in this list. Two hosts: GitHub for the
published tables, and the sharing service for the rest.

| Request | When | What goes with it |
| --- | --- | --- |
| The published table list, then any table that changed | Once, at startup | Nothing |
| What people have shared for a game | Opening that game's page | The game's file name and version |
| One shared table | You press Use table | The table id, and a random id so downloads can be counted |
| A table you are publishing | You press Share yours | The table, and your name and signature unless you shared it anonymously |
| A vote | You answer whether a table worked | The table id, the same random id, and the game version |
| A name | You claim one | The name |

The random id is made once on first run, is not derived from anything about the
machine, and exists so one person cannot vote twice.

Two switches in settings cover all of it. **Download new tables automatically**
is the first row. **Shared tables from other people** is the other five. That
second one is separate on purpose: opening a game page asks the service about
that game without you pressing anything, so saying "only when you use it" would
not have been true. Turn both off and Freeplay opens no connections at all and
runs on whatever is on your disk.

I built it without any network at first and it was the wrong call. It meant
every new game was somebody manually finding a file and putting it in a folder,
which is not a trainer, it is homework.

## What it does not do

No genre, and no "installed on" beyond the store name. Steam only keeps genre
in `appinfo.vdf`, which is a binary format, and reading it properly is more
work than a line of text is worth.

Settings live in `%APPDATA%\freeplay\settings.json`. Downloaded tables live
next to it in `%APPDATA%\freeplay\tables\`, and ones you converted from a
`.CT` yourself in `%APPDATA%\freeplay\mine\`. Whichever you picked most
recently is the one that wins.

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
type = "value"
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

## License

MIT.
