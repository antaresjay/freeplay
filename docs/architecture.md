# How it fits together

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

## Why there is an assembler in here

Almost every Cheat Engine table worth having is built the same way: a script
scans for an instruction, allocates a cave next to it, writes a jump over it,
and copies whatever register held the player into a slot. Every value entry then
hangs off that slot's name rather than off an address.

So importing those tables means running that assembly, which means assembling
it, allocating inside the target and hooking instructions. Without it plenty of
tables import nothing at all, because every entry in them depends on the script
that runs first.
