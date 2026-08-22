# Freeplay

Freeplay is an open source Windows game trainer written in Rust. It finds the
games you have installed, attaches to single player ones, scans memory, walks
pointer chains and holds values where you want them, from either a desktop app
or a command line.

A game is described by a TOML file saying where its numbers live and what to do
with them, so adding one needs no Rust and no release. Cheat Engine `.CT` files
convert straight into that format, Auto Assembler scripts and all: Freeplay
ships its own x86 and x64 assembler, allocates code caves in the target and
hooks instructions, which is what those scripts need to run.

It is free, it does not want your email address, and it refuses to attach to
anything running an anti-cheat.

Named after the arcade cabinet setting that gives you unlimited credits without
feeding coins into the slot.

## The library

**8,896 tables covering 7,034 games**, with 258,041 cheats between them.

They were converted from Cheat Engine tables people published on Fearless
Revolution, one thread per game. Whoever worked the addresses out is named on
the game page with a link back to their thread, and all 1,182 of them are
listed in [CREDITS.md](https://github.com/antaresjay/freeplay-tables/blob/main/CREDITS.md)
in the tables repository. If you wrote one and would rather it was not there,
say so and it goes.

Nothing in that library has been run against every game by anybody. A table was
written against one build and the addresses move when a game patches, which is
what the voting is for: people say whether it worked, and the ones that did
rise. Freeplay does not put its own name on any of it, because nothing in the
app checks a table functionally.

You do not download the library. Opening a game asks what exists for that one
game, and you install the table you pick.

## Get it

[Releases](https://github.com/antaresjay/freeplay/releases) has three files.

| File | What it is |
| --- | --- |
| `Freeplay_x.y.z_x64-setup.exe` | Installer. Goes in your own user folder, no admin prompt, uninstalls from Settings like anything else |
| `Freeplay.exe` | The same program, nothing to install. Put it where you like |
| `freeplay-cli.exe` | The command line one |

None of them is signed, so SmartScreen stops you the first time: **More info**,
then **Run anyway**. See [things you should know](docs/running-it.md).

Or [build it yourself](docs/building.md), which is one click on a fork and
needs no Rust toolchain.

## What it does

- Finds games installed through Steam, Epic and GOG, and shows which are running
- Shows your library with real cover art, play time and when you last played,
  all read from what Steam already cached on your own disk
- Shows what other people have shared for a game, sorted by what worked for
  them, and asks afterwards whether it worked for you
- Marks games that ship an anti-cheat before you click them, rather than letting
  you find out at the point of refusal
- Attaches to a game and lists its cheats, 32-bit or 64-bit, since plenty of the
  games worth cheating in are still 32-bit
- Switch cheats on with the game closed. Freeplay attaches on its own when the
  game starts and holds them on as soon as you are far enough in for them to
  work, so you never alt-tab back to the app
- Cheats that take a number take a number. Carry weight, game speed and how
  much gold are not switches, and a table that freezes them at 999999 is a
  wrecked save rather than a cheat
- Says what each cheat is doing rather than pretending. On, waiting for the
  game, or not found in this build
- An [overlay](docs/overlay.md) over the game, on a shortcut, for turning things
  on without alt-tabbing at all
- Finds values yourself: search for 100, take damage, search again, keep going
  until one address is left
- Freezes values, sets them once, patches the instruction that changes them, or
  runs a Cheat Engine script and puts everything back when you switch it off
- Remembers what you had on and what you typed, per game, across launches
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

## Tables

A table is a TOML file describing one game. Nothing in the code knows about any
specific game.

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

`freeplay check mytable.toml` reads one over without the game running. See
[tables/README.md](tables/README.md) for the full format, including code
patching and rip-relative operands.

## The rest

| | |
| --- | --- |
| [Things you should know](docs/running-it.md) | SmartScreen, administrator, out of date tables, where files live |
| [Building it yourself](docs/building.md) | From a fork or locally, the tests, the command line tool |
| [How it fits together](docs/architecture.md) | The crates, and why there is an assembler in here |
| [Sharing tables](docs/sharing.md) | Publishing, voting, names, and every request Freeplay makes |
| [The overlay](docs/overlay.md) | The panel over the game, and the keyboard hook it needs |
| [The table format](tables/README.md) | Writing one by hand |

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

## Buy me a coffee

Your support helps me continue developing and maintaining these projects,
ensuring they stay useful and up-to-date.

If you appreciate my work and want to help me keep going, buying me a coffee is
a great way to show your support. Thank you! :coffee:

<a href="https://www.buymeacoffee.com/antaresjeet" target="_blank"><img src="https://cdn.buymeacoffee.com/buttons/v2/default-yellow.png" alt="Buy Me A Coffee" style="height: 60px !important;width: 217px !important;" ></a>

## License

MIT.
