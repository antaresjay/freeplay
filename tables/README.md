# Tables

These are the tables Freeplay ships and updates. Add yours to `index.json` in
the same pull request and every copy of Freeplay picks it up on next launch,
with no release and nothing for anybody to download by hand.

```json
{"exe": "witcher2.exe", "game": "The Witcher 2", "file": "witcher2.toml", "revision": 1, "cheats": 6}
```

Bump `revision` whenever you change the file, otherwise nobody who already has
it will fetch it again.

One TOML file per game. Drop it in this folder and Freeplay picks it up on the
next attach. No Rust, no rebuild, no release.

If you work out where a game keeps its numbers, a pull request here helps
everybody who plays it.

## Cheat Engine tables

Freeplay reads `.CT` files out of this folder too, so if somebody has already
published one for your game you can drop it straight in. Name it after the
process, `witcher2.exe.CT`, so Freeplay knows what to attach to.

To see what you are getting before you trust it:

```
cargo run --release --bin freeplay -- import witcher2.exe.CT
```

It prints the table it would build and, on stderr, every entry it could not
take and why.

**Auto Assembler scripts come across as scripts.** A script becomes a cheat
whose toggle runs it: Freeplay scans for the signature, allocates a cave near
the match, assembles the body, writes the hook, and puts the original bytes
back when you switch it off. Value entries anchored to a name the script
registers become `find = "symbol"` locators, so they wait until that script is
on and then light up.

The one thing that never comes across is a **bare address**. An entry whose
address is a plain number like `1A2B3C4D5E` is wherever that value happened to
live on somebody else's machine on the day they scanned. Only addresses
anchored to a module, `game.exe+1A2B3C`, or to a script's symbol, mean anything
anywhere else.

Cheat Engine lists pointer offsets last hop first, the way its pointer editor
shows them. Freeplay reverses them on import, so the chain in the converted
table reads in the order it is actually walked.

Tables for 32-bit games work the same way. Freeplay reads the process's pointer
width off the process itself, so a chain written for a 32-bit build is walked
four bytes at a time without anything in the table saying so.

Every value entry comes across as a `freeze` with a guessed number, because a
`.CT` says what and where but never how much. Change the numbers.

## Symbol locators

```toml
[cheat.locator]
find = "symbol"
symbol = "baseWitcher"
hops = ["+0x14", "+0x8"]
```

The symbol is a name some script in the same table registers while it runs.
Until that script is switched on the cheat waits, and says so. This is how a
converted Cheat Engine table hangs together: one script finds the player, and
twenty entries read fields off what it found.

## Scripts

```toml
[[cheat]]
id = "get-witcher-base"
name = "Get Witcher Base"
type = "script"
source = """
[ENABLE]
aobscanmodule(getWitcher,witcher2.EXE,8B 10 8B C8 FF 92 34 02 00 00 84)
alloc(newgetWitcher,100,getWitcher)
label(baseWitcher)
registersymbol(baseWitcher)
newgetWitcher:
  mov [baseWitcher],eax
  mov edx,[eax]
  jmp returngetWitcher
baseWitcher:
  dd 0
getWitcher:
  jmp newgetWitcher
  nop 5
returngetWitcher:

[DISABLE]
getWitcher:
  db 8B 10 8B C8 FF 92 34 02 00 00
dealloc(newgetWitcher)
"""
```

A script has no `[cheat.locator]`, because it does not have one address.

Supported: `aobscanmodule`, `aobscan`, `alloc`, `globalalloc`, `label`,
`registersymbol`, `unregistersymbol`, `dealloc`, `define`, `assert`. Numbers are
hexadecimal without a prefix, `#` makes one decimal, the way Cheat Engine writes
them.

`createthread`, `loadlibrary` and `luacall` are read and ignored rather than
refused, so a script that only uses one for logging still works.

## The game block

```toml
[game]
name = "The Witcher 2: Assassins of Kings"
exe  = "witcher2.exe"          # matched case insensitively
author = "your name"           # optional
notes = "tested on the enhanced edition"   # optional
verified = ["3.5.0.1"]         # builds you actually checked, optional
```

## A cheat

```toml
[[cheat]]
id = "infinite-vigor"          # unique within the file
name = "Infinite Vigor"        # what the interface shows
category = "player"            # player, resources, combat, movement, game, misc
description = "Signs and dodging never run you dry"
hint = "Load a save first"     # shown when the cheat cannot resolve yet
type = "freeze"                # what to do
value_type = "f32"
value = 1000

[cheat.locator]                # how to find the address
find = "static"
module = "witcher2.exe"
offset = "0x1A2B3C"
```

## Finding the address

Two ways, and you almost always want the second.

**Static.** A fixed offset inside a module. Simple, and breaks the moment the
game is patched.

```toml
[cheat.locator]
find = "static"
module = "witcher2.exe"
offset = "0x1A2B3C"
hops = ["+0x28", "+0x1F0"]     # optional pointer chain
```

**Pattern.** Search for the instruction bytes instead. Addresses move on every
rebuild, the code around them usually does not, so this survives most updates.

```toml
[cheat.locator]
find = "pattern"
pattern = "48 8B 05 ?? ?? ?? ??"   # ?? is any byte
scope = "code"                     # code, data or all
module = "witcher2.exe"            # optional, restricts the search
offset = 3                         # bytes into the match to start from
hops = ["+0x28", "+0x1F0"]
```

Blank out anything that changes between builds, which mostly means offsets and
relative jumps. A pattern that matches more than once is rejected rather than
guessed at, so make it longer until it is unique.

**Rip relative operands.** On 64 bit, `mov rax, [rip+disp32]` stores a distance
from the end of the instruction rather than an address. Say where the
displacement sits and how long the instruction is, and Freeplay does the rest:

```toml
[cheat.locator]
find = "pattern"
pattern = "48 8B 05 ?? ?? ?? ??"
rip = { displacement_at = 3, instruction_length = 7 }
```

**Pointer chains.** `hops` is applied after each dereference, and the last one
lands on the value. `["+0x28", "+0x1F0"]` means read the pointer, add 0x28, read
that, add 0x1F0. Negative offsets are fine. This is what keeps a cheat working
after you reload a save, because the object gets allocated somewhere new every
time but the route to it does not.

## What a cheat does

**freeze** holds a value while the toggle is on.

```toml
type = "freeze"
value_type = "f32"     # i8 u8 i16 u16 i32 u32 i64 u64 f32 f64
value = 1000
```

**set** writes once and lets the game carry on. Right for money.

```toml
type = "set"
value_type = "i32"
value = 999999
```

**nop** replaces instructions with no-ops. Use this when the game rewrites the
value every frame, like a mission timer, where freezing gives you a stuttering
clock instead of a stopped one. Find the instruction that does the subtracting
and remove it.

```toml
type = "nop"
length = 5
```

`length` has to cover whole instructions. Half an instruction leaves the
processor decoding the tail of one thing as the start of another, and the game
crashes.

**bytes** replaces instructions with specific ones.

```toml
type = "bytes"
replacement = "31 C0 C3"   # xor eax, eax / ret
```

## States

Every cheat is checked whenever the list refreshes, and lands in one of three
states.

| State | Means | What to do |
| --- | --- | --- |
| ready | Address resolved | Use it |
| wait | Code found, pointer empty | Load into the game, it will light up |
| broken | Signature not found | The game patched, the table needs updating |

That split is the reason for the pattern and hop syntax above. A trainer that
shows one error for both cases is why trainers feel unreliable.

## Rules of thumb

- Prefer patterns to static offsets. They survive patches.
- Make patterns long enough to be unique, then wildcard the volatile bytes.
- Fill in `verified` so people know what you actually tested against.
- Write a `hint` for anything that only resolves during gameplay.
- Test that turning a cheat off puts things back.
