const MAX_BYTES = 256 * 1024;
const MAX_POSTS_PER_HOUR = 10;

/* an install id is made by the client, so anybody can mint as many as they
   like and vote once from each. the answer is not to fingerprint the machine:
   the client is open source and would just be patched, and a program that
   promises to send nothing about you cannot start reading disk serials.

   what actually costs an attacker something is addresses. votes are counted
   per hour against a salted hash of one, and a single address only gets to be
   so many different people in a day */
const MAX_VOTES_PER_HOUR = 40;
const MAX_VOTERS_PER_DAY = 6;
const MAX_NAMES_PER_DAY = 3;

const HOUR = 3600;
const DAY = 86400;

// same list the rust guard refuses. the client is what actually protects you,
// this is here so obvious junk never reaches the database in the first place
const BANNED = [
  "loadlibrary",
  "createthread",
  "createthreadandwait",
  "luacall",
  "luacode",
  "shellexecute",
  "winexec",
];

const json = (body, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store" },
  });

const bad = (why, status = 400) => json({ error: why }, status);

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const path = url.pathname.replace(/\/+$/, "");

    try {
      if (request.method === "GET" && path === "/tables") return list(url, env);
      if (request.method === "GET" && path.startsWith("/table/")) return one(path, url, env);
      if (request.method === "POST" && path === "/submit") return submit(request, env);
      if (request.method === "POST" && path === "/vote") return vote(request, env);
      if (request.method === "GET" && path.startsWith("/who/")) return who(path, env);
      return bad("no such endpoint", 404);
    } catch (e) {
      return bad(String(e && e.message ? e.message : e), 500);
    }
  },

  async scheduled(event, env, ctx) {
    ctx.waitUntil(mirror(env));
  },
};

// best puts a table somebody already used on your build first, because tables
// die when a game patches and twenty votes from two patches ago are worth less
// than one from the version you are running
const ORDER = {
  best: "(built_for = ?2) desc, (up - down) desc, downloads desc, created_at desc",
  votes: "(up - down) desc, up desc, created_at desc",
  downloads: "downloads desc, (up - down) desc, created_at desc",
  new: "created_at desc",
  old: "created_at asc",
  cheats: "cheats desc, (up - down) desc",
};

async function list(url, env) {
  const exe = (url.searchParams.get("exe") || "").toLowerCase();
  if (!exe) return bad("which game");
  const build = url.searchParams.get("build") || "";

  // own property, because ORDER["constructor"] and ORDER["toString"] both come
  // back truthy off the prototype and end up interpolated into the sql below
  const sort = url.searchParams.get("sort") || "best";
  if (!Object.prototype.hasOwnProperty.call(ORDER, sort)) {
    return bad(`sort by one of ${Object.keys(ORDER).join(", ")}`);
  }
  const order = ORDER[sort];

  const statement = env.DB.prepare(
    `select id, exe, game, fingerprint, cheats, bytes, submitted_by, author,
            built_for, up, down, downloads, created_at
       from tables
      where exe = ?1 and blocked = 0
      order by ${order}
      limit 50`
  );

  // only best looks at the build, and d1 refuses a spare bound parameter
  const bound = order.includes("?2") ? statement.bind(exe, build) : statement.bind(exe);
  const rows = await bound.all();

  return json({ tables: rows.results || [], sort });
}

async function one(path, url, env) {
  const id = Number(path.slice("/table/".length));
  if (!Number.isInteger(id)) return bad("that is not an id");

  const row = await env.DB.prepare(
    "select id, exe, game, toml, fingerprint, downloads from tables where id = ?1 and blocked = 0"
  )
    .bind(id)
    .first();

  if (!row) return bad("not here", 404);

  // counted per install, so opening the same table twice is still one download
  const install = url.searchParams.get("install") || "";
  if (/^[0-9a-f]{16,64}$/.test(install)) {
    const grabbed = await env.DB.prepare(
      "insert or ignore into grabs (install, table_id, created_at) values (?1, ?2, ?3)"
    )
      .bind(install, id, Math.floor(Date.now() / 1000))
      .run();

    if (grabbed.meta && grabbed.meta.changes) {
      await env.DB.prepare("update tables set downloads = downloads + 1 where id = ?1")
        .bind(id)
        .run();
    }
  }

  return json(row);
}

// a name belongs to whoever registered it, and staying that person means
// signing. no password to lose, and nothing here worth stealing since all we
// keep is the public half
async function verifySignature(pubkey, message, signature) {
  const bytes = (hex) =>
    new Uint8Array(hex.match(/../g).map((pair) => parseInt(pair, 16)));

  for (const algorithm of ["Ed25519", "NODE-ED25519"]) {
    try {
      const key = await crypto.subtle.importKey(
        "raw",
        bytes(pubkey),
        { name: algorithm, namedCurve: algorithm },
        false,
        ["verify"]
      );
      return await crypto.subtle.verify(
        algorithm,
        key,
        bytes(signature),
        new TextEncoder().encode(message)
      );
    } catch (e) {
      // the runtime only takes one of the two names, try the other
    }
  }
  throw new Error("this runtime cannot check signatures");
}

async function who(path, env) {
  const name = decodeURIComponent(path.slice("/who/".length)).toLowerCase();
  const row = await env.DB.prepare(
    "select name, pubkey, created_at from accounts where name = ?1"
  )
    .bind(name)
    .first();

  return row ? json(row) : json({ taken: false }, 404);
}

async function claim(env, ip, name, pubkey, signature, fingerprint) {
  if (!/^[0-9a-f]{64}$/.test(pubkey || "")) return "that is not a key";
  if (!/^[0-9a-f]{128}$/.test(signature || "")) return "that is not a signature";
  if (!/^[\w.-]{2,32}$/.test(name)) return "that name has odd characters in it";

  const message = `freeplay/1\n${name.toLowerCase()}\n${fingerprint}`;
  if (!(await verifySignature(pubkey, message, signature))) {
    return "that signature does not match the key";
  }

  const held = await env.DB.prepare("select pubkey from accounts where name = ?1")
    .bind(name.toLowerCase())
    .first();

  if (held) {
    return held.pubkey === pubkey ? null : `${name} belongs to somebody else`;
  }

  // publishing under a name you already hold is not a claim. this only counts
  // the ones that register something new, so sitting on a pile of names has to
  // be spread over days
  const today = await lately(env, ip, "claim", DAY);
  if (today.count >= MAX_NAMES_PER_DAY) {
    return `that is ${MAX_NAMES_PER_DAY} new names today, try tomorrow`;
  }

  await env.DB.prepare(
    "insert into accounts (name, pubkey, created_at) values (?1, ?2, ?3)"
  )
    .bind(name.toLowerCase(), pubkey, Math.floor(Date.now() / 1000))
    .run();
  await noteHit(env, ip, "claim", name.toLowerCase());
  return null;
}

async function submit(request, env) {
  const now = Math.floor(Date.now() / 1000);
  // rate limiting needs to tell two submitters apart, not know who they are.
  // a salted hash does that, and there is no address sat in the database
  const ip = await stamp(request.headers.get("cf-connecting-ip") || "", env);

  await env.DB.prepare("delete from posts where at < ?1").bind(now - 3600).run();

  const recent = await env.DB.prepare(
    "select count(*) as n from posts where ip = ?1 and at > ?2"
  )
    .bind(ip, now - 3600)
    .first();
  if (recent && recent.n >= MAX_POSTS_PER_HOUR) return bad("slow down", 429);

  const body = await request.json().catch(() => null);
  if (!body) return bad("send json");

  const { fingerprint, exe, game, toml, submitted_by, built_for, cheats } = body;
  // free text, and it stays free text. it says where a table came from, not
  // who is uploading it, and there is nothing to check it against
  const author = String(body.author || "").slice(0, 80);

  if (!fingerprint || !/^[0-9a-f]{64}$/.test(fingerprint)) return bad("bad fingerprint");
  if (!exe || typeof exe !== "string" || exe.length > 128) return bad("bad exe");
  if (!game || typeof game !== "string" || game.length > 200) return bad("bad game name");
  if (typeof toml !== "string" || !toml.length) return bad("nothing to store");
  if (toml.length > MAX_BYTES) return bad("that table is far too big");

  // a name only counts if it is signed for. anything else is anonymous
  let handle = "";
  if (submitted_by) {
    const wrong = await claim(
      env, ip, String(submitted_by), body.pubkey, body.signature, fingerprint
    );
    if (wrong) return bad(wrong, 403);
    handle = String(submitted_by);
  }

  const lowered = toml.toLowerCase();
  const hit = BANNED.find((word) => lowered.includes(word + "("));
  if (hit) return bad(`tables that call ${hit} are not accepted`);

  if (!lowered.includes("[game]")) return bad("that is not a freeplay table");

  const already = await env.DB.prepare("select id from tables where fingerprint = ?1")
    .bind(fingerprint)
    .first();
  if (already) return json({ id: already.id, already: true });

  const inserted = await env.DB.prepare(
    `insert into tables (fingerprint, exe, game, toml, bytes, cheats, submitted_by,
                         author, built_for, created_at)
     values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
     returning id`
  )
    .bind(
      fingerprint,
      exe.toLowerCase(),
      game,
      toml,
      toml.length,
      Number(cheats) || 0,
      handle,
      author,
      String(built_for || "").slice(0, 60),
      now
    )
    .first();

  await env.DB.prepare("insert into posts (ip, at) values (?1, ?2)").bind(ip, now).run();

  return json({ id: inserted.id, already: false });
}

// one vote per install per table, and changing your mind just replaces it
async function vote(request, env) {
  const body = await request.json().catch(() => null);
  if (!body) return bad("send json");

  const id = Number(body.id);
  const install = String(body.install || "");
  const up = body.up ? 1 : -1;

  if (!Number.isInteger(id)) return bad("that is not an id");
  if (!/^[0-9a-f]{16,64}$/.test(install)) return bad("bad install id");

  const ip = await stamp(request.headers.get("cf-connecting-ip") || "", env);
  const seen = await lately(env, ip, "vote", HOUR);
  if (seen.count >= MAX_VOTES_PER_HOUR) return bad("slow down", 429);

  // changing your own mind is not a new voter, so this only bites when one
  // address turns up as several people
  const day = await lately(env, ip, "vote", DAY);
  const known = await env.DB.prepare(
    "select 1 as yes from hits where kind = 'vote' and ip = ?1 and what = ?2 limit 1"
  )
    .bind(ip, install)
    .first();
  if (!known && day.distinct >= MAX_VOTERS_PER_DAY) return bad("slow down", 429);

  const before = await env.DB.prepare(
    "select vote from votes where install = ?1 and table_id = ?2"
  )
    .bind(install, id)
    .first();

  if (before && before.vote === up) return json({ ok: true, unchanged: true });

  await env.DB.batch([
    env.DB.prepare(
      `insert into votes (install, table_id, vote, built_for, created_at)
       values (?1, ?2, ?3, ?4, ?5)
       on conflict (install, table_id) do update set vote = ?3, created_at = ?5`
    ).bind(install, id, up, String(body.built_for || "").slice(0, 60), Math.floor(Date.now() / 1000)),
    env.DB.prepare(
      `update tables
          set up = (select count(*) from votes where table_id = ?1 and vote = 1),
              down = (select count(*) from votes where table_id = ?1 and vote = -1)
        where id = ?1`
    ).bind(id),
  ]);

  await noteHit(env, ip, "vote", install);
  return json({ ok: true });
}

// every few minutes, anything new goes into the tables repo in one commit and
// index.json gets rebuilt. d1 is the fast lane, the repo is the one that
// outlives whatever happens to this worker
async function mirror(env) {
  const pending = await env.DB.prepare(
    "select id, exe, game, toml, fingerprint, cheats from tables where mirrored = 0 and blocked = 0 limit 40"
  ).all();

  const rows = pending.results || [];
  if (!rows.length) return;

  const api = (path, init = {}) =>
    fetch(`https://api.github.com/repos/${env.TABLES_REPO}${path}`, {
      ...init,
      headers: {
        authorization: `Bearer ${env.GITHUB_TOKEN}`,
        accept: "application/vnd.github+json",
        "user-agent": "freeplay-mirror",
        ...(init.headers || {}),
      },
    }).then(async (r) => {
      if (!r.ok) throw new Error(`github ${path}: ${r.status} ${await r.text()}`);
      return r.json();
    });

  const branch = env.TABLES_BRANCH || "main";
  const ref = await api(`/git/ref/heads/${branch}`);
  const head = await api(`/git/commits/${ref.object.sha}`);

  const tree = [];
  for (const row of rows) {
    const blob = await api("/git/blobs", {
      method: "POST",
      body: JSON.stringify({ content: row.toml, encoding: "utf-8" }),
    });
    tree.push({
      path: `tables/${slug(row.exe)}/${row.fingerprint.slice(0, 12)}.toml`,
      mode: "100644",
      type: "blob",
      sha: blob.sha,
    });
  }

  const everything = await env.DB.prepare(
    `select exe, game, fingerprint, cheats, up, down, downloads, submitted_by, author, created_at
       from tables where blocked = 0 order by exe, (up - down) desc`
  ).all();

  const index = {
    version: 1,
    tables: (everything.results || []).map((row) => ({
      exe: row.exe,
      game: row.game,
      file: `${slug(row.exe)}/${row.fingerprint.slice(0, 12)}.toml`,
      revision: 1,
      cheats: row.cheats,
      score: row.up - row.down,
      up: row.up,
      down: row.down,
      downloads: row.downloads,
      by: row.submitted_by,
      author: row.author,
      added: row.created_at,
    })),
  };

  const indexBlob = await api("/git/blobs", {
    method: "POST",
    body: JSON.stringify({ content: JSON.stringify(index, null, 2), encoding: "utf-8" }),
  });
  tree.push({ path: "tables/index.json", mode: "100644", type: "blob", sha: indexBlob.sha });

  const built = await api("/git/trees", {
    method: "POST",
    body: JSON.stringify({ base_tree: head.tree.sha, tree }),
  });

  // say who the commit is from. left out, github stamps it with whatever the
  // token's account uses, which can be a real address
  const who = {
    name: env.MIRROR_NAME || "freeplay",
    email: env.MIRROR_EMAIL || "freeplay@users.noreply.github.com",
    date: new Date().toISOString(),
  };

  const commit = await api("/git/commits", {
    method: "POST",
    body: JSON.stringify({
      message: `add ${rows.length} table${rows.length === 1 ? "" : "s"}`,
      tree: built.sha,
      parents: [ref.object.sha],
      author: who,
      committer: who,
    }),
  });

  await api(`/git/refs/heads/${branch}`, {
    method: "PATCH",
    body: JSON.stringify({ sha: commit.sha }),
  });

  await env.DB.prepare(
    `update tables set mirrored = 1 where id in (${rows.map((r) => r.id).join(",")})`
  ).run();
}

// how many of `kind` this address has done lately, and how many different
// things it did them as. nothing is written until the caller says it went
// ahead, so a refused request does not count against the next one
async function lately(env, ip, kind, since) {
  const now = Math.floor(Date.now() / 1000);
  await env.DB.prepare("delete from hits where at < ?1")
    .bind(now - DAY)
    .run();

  const row = await env.DB.prepare(
    `select count(*) as n, count(distinct what) as who
       from hits where kind = ?1 and ip = ?2 and at > ?3`
  )
    .bind(kind, ip, now - since)
    .first();

  return { count: (row && row.n) || 0, distinct: (row && row.who) || 0 };
}

const noteHit = (env, ip, kind, what = "") =>
  env.DB.prepare("insert into hits (ip, kind, what, at) values (?1, ?2, ?3, ?4)")
    .bind(ip, kind, what, Math.floor(Date.now() / 1000))
    .run();

async function stamp(value, env) {
  const salt = env.IP_SALT || "freeplay";
  const bytes = new TextEncoder().encode(salt + "|" + value);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)]
    .slice(0, 12)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

function slug(exe) {
  return exe.toLowerCase().replace(/\.exe$/, "").replace(/[^a-z0-9._-]/g, "-");
}
