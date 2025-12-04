# Freeplay

A game trainer for Windows that is free, open source, and does not want your
email address.

Named after the arcade cabinet setting that gives you unlimited credits without
feeding coins into the slot.

> Early development. Nothing to install yet.

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

- Lists the games you have installed and the ones currently running
- Attaches to a running game and shows the cheats available for it
- Finds values yourself with a built in scanner: search for 100, take damage,
  search again, keep going until one address is left
- Saves what you found as a small readable table file you can share
- Only offers cheats that are actually working right now, instead of toggles
  that silently do nothing

## Single player only

Freeplay refuses to attach to any process running an anti-cheat. That check is
in the code, not in a disclaimer at the bottom of a page. EasyAntiCheat,
BattlEye, Vanguard, GameGuard, XIGNCODE, PunkBuster and others are all in the
refusal list.

Two reasons. Cheating in multiplayer ruins the game for people who did nothing
to deserve it. And it gets accounts banned, which is a miserable thing to
happen to somebody because a tool let them. Single player is the whole point.

## Status

The memory engine works and is tested against a live process. Still to come:
the value scanner, pointer chains, code patching, the table format, game
detection and the interface.

```
cargo test
```

## License

MIT.
