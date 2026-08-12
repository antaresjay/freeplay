# The overlay

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

## The keyboard hook

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
