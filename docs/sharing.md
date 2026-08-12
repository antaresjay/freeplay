# Sharing tables, and what Freeplay sends

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

Two different claims are kept apart on purpose. **Name verified** means the
name is registered to a key and nobody else can publish under it. It says
nothing about whether the table works, and nothing about who wrote it either.
Whether it works is what the votes and the tested-on version are for.

Whoever worked the addresses out is named separately, on the game page, with a
link back to where the table came from. For a converted table that is almost
never the person who uploaded it.

Names are optional and anonymous is the default. If you do want your name on
what you share, Freeplay makes a key and registers the name against it, so
nobody can publish under your name by typing it. There is no password and no
email, just seventeen words you write down once, which is also how the name
moves to another machine. Lose the words and the name is gone, and there is no
way around that: a secret cannot be recovered from nothing.

Everything shared is our own table format, never a raw `.CT`, so it has been
parsed and validated before it can reach anybody. A downloaded script may only
touch the game's own modules, and anything calling `loadlibrary` or spawning a
thread is refused outright. See `crates/freeplay-aa/src/safety.rs`.

Tables land in a Cloudflare D1 database within seconds and are mirrored into
[freeplay-tables](https://github.com/antaresjay/freeplay-tables), so the
repository outlives the service and you can read every table on GitHub without
installing anything.

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
