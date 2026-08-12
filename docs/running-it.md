# Things you should know

**Windows will complain.** The binary is unsigned, so SmartScreen warns on first
run. Windows Defender may flag it too, because reading and writing another
process's memory is exactly what malware does. There is no way around that
short of a code signing certificate costing a few hundred pounds a year. Every
release is built by the workflow in this repository and the Actions tab shows
the run that produced it, so you can check the files came from this code. Or
[build it yourself](building.md), which takes one click on a fork.

**Administrator is usually not needed.** A game you started yourself runs as
you, and one program of yours can open another. You need it in one case: if the
game runs elevated, Freeplay has to be elevated too, because a normal process
cannot open a handle to one above it.

**A table can be out of date.** A table is written against one build of a game
and the addresses move when the game patches. Freeplay says which of the three
states each cheat is in rather than failing silently: ready, waiting for you to
load into the game, or not found in this build. That last one means the table
needs updating, not that anything is broken.

**Nothing here has been run against every game.** The library is converted from
tables other people wrote and published. They worked for the person who wrote
them, on the build they had. Whether one works for you is what the voting is
for, and Freeplay does not put its own name on any of it, because nothing in
the app checks a table functionally.

## Cheat Engine tables

Drop a `.CT` in `tables/` named after the process and Freeplay converts it:
addresses, pointer chains, types, groups, and Auto Assembler scripts.

The one thing that never comes across is a bare address. An entry whose address
is a plain number like `1A2B3C4D5E` is wherever that value happened to live on
somebody else's machine on the day they scanned. Only addresses anchored to a
module or to a script's symbol mean anything anywhere else.

Scripts that drop into Lua are refused too. Lua gets the whole machine rather
than the game's memory, and a table off the internet does not get that.

See [tables/README.md](../tables/README.md) for the format.

## Where things live

Settings live in `%APPDATA%\freeplay\settings.json`. Downloaded tables live
next to it in `%APPDATA%\freeplay\tables\`, and ones you converted from a
`.CT` yourself in `%APPDATA%\freeplay\mine\`. Whichever you picked most
recently is the one that wins.

## What it does not do

No genre, and no "installed on" beyond the store name. Steam only keeps genre
in `appinfo.vdf`, which is a binary format, and reading it properly is more
work than a line of text is worth.
