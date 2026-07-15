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

`IP_SALT` is optional and worth setting:

```
wrangler secret put IP_SALT
```

## What is public

The `workers.dev` subdomain, because it is compiled into the app. Pick a neutral
one, it has nothing to do with the account behind it.

Not public: the Cloudflare account or its email. Nothing in a response header or
a URL carries it.

Mirror commits are stamped with `MIRROR_NAME` and `MIRROR_EMAIL` on purpose. Left
unset, GitHub uses whatever address the token's account has, which may be a real
one.

Submitter handles are optional, free text, and whatever somebody types.

Addresses are never stored. Rate limiting hashes them with `IP_SALT`, keeps
twelve bytes of that, and drops rows older than a day. Nothing in the tables
repository has ever seen one.

`schema.sql` only creates what is missing, so run it again after a pull to pick
up new tables on a service that is already live.

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

Votes are forty an hour per address, and one address only gets to be six
different voters a day. New names are three a day.

An install id is made by the client, so anybody can mint as many as they like
and vote once from each. The fix is not to fingerprint the machine: the client
is open source and would just be patched, and an app that promises to send
nothing about you cannot start reading disk serials to enforce it. Addresses
are the thing an attacker has to pay for, so that is what the limits count.
None of this makes stuffing impossible. It makes it cost something, which for
ordering a list of cheat tables is the right amount of effort to spend.

That check is here so obvious junk never reaches the database. It is not what
protects you. The client validates and sandboxes every downloaded table before
anything runs, in `freeplay-aa/src/safety.rs`, and that is the check that
matters, because it is the one running on your machine.

## Cost

Free tier. 100k requests a day, 5GB in D1, 5M row reads and 100k row writes a
day. A table is a few kilobytes, so five thousand of them is about fifty
megabytes. Storage was never going to be the problem.
