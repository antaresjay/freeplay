# The table service

A Cloudflare Worker and a D1 database. It takes tables people submit, hands
them back out to everyone else, counts votes, and every few minutes pushes
whatever is new into the `freeplay-tables` repository.

Two lanes, on purpose:

```
client -> worker -> D1                  live in seconds
                     |
                  cron, 5 min
                     v
              freeplay-tables repo      durable, public, reviewable
```

D1 is the fast lane. The repository is the one that outlives this service, and
it is what Freeplay falls back to if the Worker ever stops answering, since the
app already reads `index.json` over https and needs no code to do it.

## Why a Worker and not a database the app talks to directly

Anything shipped in a desktop binary is public. A database credential in the
client is a database anyone can write to. The Worker holds the credentials and
the app holds nothing, which is the whole reason it exists.

## Deploying

```
npm install -g wrangler
wrangler login

wrangler d1 create freeplay-tables
# put the id it prints into wrangler.toml

wrangler d1 execute freeplay-tables --remote --file schema.sql
wrangler secret put GITHUB_TOKEN
wrangler deploy
```

`GITHUB_TOKEN` is a fine grained token with contents write on the
`freeplay-tables` repository and nothing else. It never leaves Cloudflare.

## Endpoints

| Method | Path | What |
| --- | --- | --- |
| GET | `/tables?exe=witcher2.exe&build=3.5` | Tables for a game, best first |
| GET | `/table/:id` | One table's toml |
| POST | `/submit` | Send a converted table |
| POST | `/vote` | One vote per install per table |

Listing prefers tables somebody has already used on your build of the game.
Tables break when a game patches, so twenty votes from two patches ago is worth
less than one from the version you are actually running.

## What it refuses

Submissions are capped at 256KB, ten per IP per hour, and anything calling
`loadlibrary`, `createthread`, `luacall` and friends is rejected outright.

That check is here so obvious junk never reaches the database. It is not what
protects you. The client validates and sandboxes every downloaded table before
anything runs, in `freeplay-aa/src/safety.rs`, and that is the check that
matters, because it is the one running on your machine.

## Cost

Free tier. 100k requests a day, 5GB in D1, 5M row reads and 100k row writes a
day. A table is a few kilobytes, so five thousand of them is about fifty
megabytes. Storage was never going to be the problem.
