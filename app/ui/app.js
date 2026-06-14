const { invoke } = window.__TAURI__.core;
const appWindow = window.__TAURI__.window.getCurrentWindow();
const $ = (id) => document.getElementById(id);

let games = [];
let attached = null;
let scanning = false;
let cheatTimer = null;
let open = null; // key of the game whose page is showing
let drawn = "";
let config = { theme: "system", accent: "amber", pinned: [], favourites: [] };

/* art is read off disk and served over its own protocol, so ask once */
const art = new Map();
const pending = new Set();

function toast(message, bad = false) {
  const el = $("toast");
  el.textContent = message;
  el.classList.toggle("bad", bad);
  el.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (el.hidden = true), bad ? 6000 : 3000);
}

const gameFor = (key) => games.find((g) => g.key === key);

/* ---------- settings ---------- */

function applyTheme() {
  document.documentElement.dataset.theme = config.theme;
  document.documentElement.dataset.accent = config.accent;

  for (const button of document.querySelectorAll("#theme-pick button")) {
    button.classList.toggle("on", button.dataset.theme === config.theme);
  }
  for (const button of document.querySelectorAll("#accent-pick button")) {
    button.classList.toggle("on", button.dataset.accent === config.accent);
  }
  $("pinned-count").textContent = config.pinned.length
    ? `${config.pinned.length} pinned`
    : "Nothing pinned yet";
  $("auto-update").classList.toggle("on", config.auto_update !== false);
}

async function checkForTables(manual) {
  const label = $("tables-state");
  label.textContent = "Checking";
  try {
    const note = await invoke("update_tables");
    label.textContent = note;
    if (manual) toast(note);
    await loadGames(false);
    await refreshCheats();
  } catch (e) {
    label.textContent = String(e);
    if (manual) toast(String(e), true);
  }
}

async function saveConfig(changes) {
  const next = { ...config, ...changes };
  try {
    config = await invoke("save_settings", { next });
  } catch (e) {
    return toast(String(e), true);
  }
  applyTheme();
  drawn = "";
  draw();
}

function toggleIn(list, key) {
  return list.includes(key) ? list.filter((k) => k !== key) : [...list, key];
}

/* ---------- art ---------- */

const artFor = (game) => (game && game.app_id && art.get(game.app_id)) || null;

async function fetchArt(game) {
  const id = game.app_id;
  if (!id || art.has(id) || pending.has(id)) return false;
  pending.add(id);
  try {
    art.set(id, await invoke("game_art", { appId: id }));
    return true;
  } catch {
    art.set(id, {});
    return false;
  } finally {
    pending.delete(id);
  }
}

async function loadArt() {
  const results = await Promise.all(games.map(fetchArt));
  if (results.some(Boolean)) {
    drawn = "";
    draw();
    if (open) drawGamePage();
  }
}

function initials(name) {
  const words = name.replace(/[^\w\s]/g, " ").split(/\s+/).filter(Boolean);
  if (!words.length) return "?";
  return (words.length > 1 ? words[0][0] + words[1][0] : words[0].slice(0, 2)).toUpperCase();
}

function hue(name) {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) % 360;
  return h;
}

function coverInto(box, game) {
  const url = artFor(game)?.cover;
  if (url) {
    const img = document.createElement("img");
    img.src = url;
    img.alt = "";
    box.appendChild(img);
    return;
  }
  box.classList.add("blankart");
  box.style.setProperty("--h", hue(game.name));
  const span = document.createElement("span");
  span.className = "initials";
  span.textContent = initials(game.name);
  box.appendChild(span);
}

/* ---------- formatting ---------- */

function playedFor(minutes) {
  if (!minutes) return null;
  if (minutes < 60) return `${minutes} min`;
  return `${(minutes / 60).toFixed(1)} hrs`;
}

function lastPlayed(seconds) {
  if (!seconds) return null;
  const then = new Date(seconds * 1000);
  const days = Math.floor((Date.now() - then) / 86400000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 30) return `${days} days ago`;
  return then.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

/* ---------- library ---------- */

async function loadGames(refresh = false) {
  try {
    games = await invoke("list_games", { refresh });
  } catch (e) {
    games = [];
    toast(String(e), true);
  }
  draw();
  if (open) drawGamePage();
  loadArt();
}

/* Running first, then anything with a table, anti-cheat last. */
function ordered(list) {
  const rank = (g) => (g.guard ? 2 : 0) - (g.running ? 1 : 0);
  return [...list].sort(
    (a, b) => rank(a) - rank(b) || b.has_table - a.has_table || a.name.localeCompare(b.name)
  );
}

/* Redrawing wipes hover and restarts image decoding, and the list is polled
   every few seconds, so only rebuild when something actually moved. */
function signature(list) {
  return (
    list
      .map((g) => `${g.key}${g.running}${g.has_table}${g.pinned}${art.has(g.app_id)}`)
      .join("|") + `#${$("filter").value}#${attached ? attached.process : ""}`
  );
}

function draw() {
  const list = ordered(games);
  const stamp = signature(list);
  if (stamp === drawn) return;
  drawn = stamp;

  drawRail(list);
  drawGrids(list);

  const live = list.filter((g) => g.running).length;
  $("library-count").textContent = `${list.length} games, ${live} running`;
  $("idle-banner").hidden = live > 0 || !list.length;
}

function drawRail(list) {
  const host = $("library-rail");
  host.innerHTML = "";

  if (!list.length) {
    host.innerHTML = `<div class="placeholder">Nothing found yet</div>`;
    return;
  }

  for (const game of list) {
    const button = document.createElement("button");
    button.className = "rail-game" + (open === game.key ? " active" : "");

    const thumb = document.createElement("span");
    thumb.className = "thumb";
    const url = artFor(game)?.cover;
    if (url) {
      const img = document.createElement("img");
      img.src = url;
      img.alt = "";
      thumb.appendChild(img);
    } else {
      thumb.textContent = initials(game.name);
    }

    const name = document.createElement("span");
    name.className = "name";
    name.textContent = game.name;

    const pip = document.createElement("span");
    pip.className = "pip" + (game.running ? " live" : "");

    button.append(thumb, name, pip);
    button.addEventListener("click", () => showGame(game.key));
    host.appendChild(button);
  }
}

function drawGrids(list) {
  const needle = $("filter").value.trim().toLowerCase();
  const shown = list.filter((g) => !needle || g.name.toLowerCase().includes(needle));

  const pinned = shown.filter((g) => g.pinned);
  const rest = shown.filter((g) => !g.pinned);

  $("pinned-wrap").hidden = !pinned.length;
  fill($("pinned-grid"), pinned);
  fill($("grid"), rest);
  $("library-empty").hidden = games.length > 0;
}

function fill(host, list) {
  host.innerHTML = "";
  for (const game of list) host.appendChild(card(game));
}

function card(game) {
  const button = document.createElement("button");
  button.className = "card" + (game.guard ? " guarded" : game.running ? "" : " idle");

  const box = document.createElement("div");
  box.className = "art";
  coverInto(box, game);

  const badges = document.createElement("div");
  badges.className = "badges";
  if (game.guard) badges.appendChild(badge("Anti-cheat", "guarded"));
  else if (game.running) badges.appendChild(badge("Running", "live"));
  if (game.has_table) badges.appendChild(badge("Table", "spare"));
  box.appendChild(badges);

  const overlay = document.createElement("div");
  overlay.className = "card-overlay";
  const label = document.createElement("b");
  // Clicking always opens the game's page. Saying "Attach" here promised
  // something the click does not do.
  label.textContent = game.guard ? "Off limits" : "Open";
  overlay.appendChild(label);
  box.appendChild(overlay);

  const title = document.createElement("span");
  title.className = "card-title";
  title.textContent = game.name;

  const sub = document.createElement("span");
  sub.className = "card-sub";
  const played = playedFor(game.minutes);
  sub.textContent = played ? `${game.store} · ${played}` : game.store;

  button.append(box, title, sub);
  button.addEventListener("click", () => showGame(game.key));
  return button;
}

function badge(text, extra) {
  const span = document.createElement("span");
  span.className = `badge ${extra}`;
  span.textContent = text;
  return span;
}

/* ---------- one game ---------- */

function showGame(key) {
  open = key;
  drawGamePage();
  showView("game");
  drawn = "";
  draw();
}

function fact(label, value, live = false) {
  const box = document.createElement("div");
  box.className = "fact" + (live ? " live" : "");
  const b = document.createElement("b");
  b.textContent = value;
  const span = document.createElement("span");
  span.textContent = label;
  box.append(b, span);
  return box;
}

function drawGamePage() {
  const game = gameFor(open);
  if (!game) return;
  const images = artFor(game) || {};

  const hero = $("game-hero-img");
  hero.src = images.hero || "";
  hero.hidden = !hero.src;
  document.querySelector(".game-hero").classList.toggle("no-art", !hero.src);

  const cover = $("game-cover-img");
  cover.src = images.cover || "";
  cover.hidden = !images.cover;
  const blank = $("game-cover-initials");
  blank.hidden = !!images.cover;
  blank.textContent = initials(game.name);
  $("game-cover").classList.toggle("blankart", !images.cover);
  $("game-cover").style.setProperty("--h", hue(game.name));

  const logo = $("game-logo");
  logo.src = images.logo || "";
  logo.hidden = !images.logo;
  $("game-name").textContent = game.name;

  const facts = $("game-facts");
  facts.innerHTML = "";
  facts.appendChild(fact("Store", game.store));
  const played = playedFor(game.minutes);
  if (played) facts.appendChild(fact("Play time", played));
  const seen = lastPlayed(game.last_played);
  if (seen) facts.appendChild(fact("Last played", seen));
  if (attached && attached.process === game.exe) {
    facts.appendChild(fact("Status", `Attached, pid ${attached.pid}`, true));
    if (attached.arch) facts.appendChild(fact("Build", attached.arch));
  } else if (game.running) {
    facts.appendChild(fact("Status", "Running", true));
  }

  $("detail-exe").textContent = game.exe || "not found";
  $("detail-dir").textContent = game.dir;
  $("detail-dir").title = game.dir;
  $("detail-id-row").hidden = !game.app_id;
  $("detail-id").textContent = game.app_id || "";
  $("detail-guard-row").hidden = !game.guard;
  $("detail-guard").textContent = game.guard || "";
  $("game-folder").disabled = !game.dir;

  drawTableCard(game);

  $("game-fav").classList.toggle("on", game.favourite);
  $("game-pin").classList.toggle("on", game.pinned);

  const isAttached = attached && attached.process === game.exe;
  const attach = $("game-attach");
  attach.textContent = isAttached ? "Detach" : "Attach";
  attach.disabled = !!game.guard || (!game.running && !isAttached);
  $("game-play").disabled = !!game.guard;

  const note = $("attach-note");
  if (game.guard) {
    note.hidden = false;
    $("attach-note-title").textContent = "Off limits";
    $("attach-note-body").textContent = `${game.name} ships ${game.guard}. Freeplay is for single player games, and attaching would risk your account.`;
  } else if (!game.running) {
    note.hidden = false;
    $("attach-note-title").textContent = "Not running";
    $("attach-note-body").textContent = "Press Play to start it, then attach. Freeplay reads memory from a live process.";
  } else if (!isAttached) {
    note.hidden = false;
    $("attach-note-title").textContent = "Running, not attached";
    $("attach-note-body").textContent = "Attach to see what this game offers.";
  } else {
    note.hidden = true;
  }

  if (!isAttached) {
    $("cheat-groups").innerHTML = "";
    $("no-table").hidden = true;
  }
}

/* What the table holds, drawn before anything is attached. Reading it is the
   point: you want to know whether a game is worth starting. */
async function drawTableCard(game) {
  const card = $("table-card");
  const isAttached = attached && attached.process === game.exe;

  // Once attached the live list below says all of this, with working toggles.
  if (isAttached || !game.exe) {
    card.hidden = true;
    return;
  }

  let table = null;
  try {
    table = await invoke("table_preview", { exe: game.exe });
  } catch {
    card.hidden = true;
    return;
  }

  // The page may have moved on while that was in flight.
  if (open !== game.key) return;

  if (!table) {
    card.hidden = true;
    $("no-table").hidden = false;
    return;
  }

  $("no-table").hidden = true;
  card.hidden = false;
  $("table-title").textContent = table.game;

  const by = [];
  if (table.author) by.push(`by ${table.author}`);
  if (table.verified.length) by.push(`checked against ${table.verified.join(", ")}`);
  $("table-by").textContent = by.join(", ");
  $("table-by").hidden = !by.length;

  $("table-notes").textContent = table.notes;
  $("table-notes").hidden = !table.notes;

  const n = table.cheats.length;
  $("table-count").textContent = `${n} ${n === 1 ? "cheat" : "cheats"}`;

  const host = $("table-preview");
  host.innerHTML = "";
  const byCategory = new Map();
  for (const cheat of table.cheats) {
    if (!byCategory.has(cheat.category)) byCategory.set(cheat.category, []);
    byCategory.get(cheat.category).push(cheat);
  }

  for (const [category, items] of byCategory) {
    const group = document.createElement("div");
    group.className = "preview-group";

    const heading = document.createElement("h4");
    heading.textContent = category;
    group.appendChild(heading);

    for (const item of items) {
      const row = document.createElement("div");
      row.className = "preview-row";

      const name = document.createElement("span");
      name.className = "preview-name";
      name.textContent = item.name;

      const does = document.createElement("span");
      does.className = "preview-does";
      does.textContent = item.does;

      row.append(name, does);
      if (item.description) row.title = item.description;
      group.appendChild(row);
    }
    host.appendChild(group);
  }

  $("table-locked").textContent = game.guard
    ? "Off limits while this game ships an anti-cheat."
    : game.running
      ? "Attach to switch these on."
      : "Press Play, then attach to switch these on.";
}

/* ---------- attaching ---------- */

async function doAttach(exe) {
  try {
    attached = await invoke("attach", { exe });
  } catch (e) {
    toast(String(e), true);
    return;
  }

  const known = games.find((g) => g.exe === exe);
  if (known) open = known.key;

  if (open) {
    drawGamePage();
    showView("game");
  }
  drawn = "";
  draw();
  await refreshCheats();

  // States change as you move between menus and gameplay, so keep checking.
  clearInterval(cheatTimer);
  cheatTimer = setInterval(refreshCheats, 1500);
}

async function doDetach() {
  clearInterval(cheatTimer);
  await invoke("detach");
  attached = null;
  scanning = false;
  resetScan();
  if (open) drawGamePage();
  drawn = "";
  draw();
}

/* ---------- cheats ---------- */

async function refreshCheats() {
  // Resolving a locator can mean scanning the game's memory. Not worth doing
  // for a page nobody is looking at.
  if (!attached || $("view-game").hidden) return;

  let rows = [];
  try {
    rows = await invoke("cheats");
  } catch {
    return;
  }

  $("no-table").hidden = rows.length > 0;

  const host = $("cheat-groups");
  const byCategory = new Map();
  for (const row of rows) {
    if (!byCategory.has(row.category)) byCategory.set(row.category, []);
    byCategory.get(row.category).push(row);
  }

  host.innerHTML = "";
  for (const [category, items] of byCategory) {
    const group = document.createElement("div");
    group.className = "group";

    const heading = document.createElement("h3");
    heading.textContent = category;

    const grid = document.createElement("div");
    grid.className = "cheats";
    for (const item of items) grid.appendChild(cheatCard(item));

    group.append(heading, grid);
    host.appendChild(group);
  }
}

function cheatCard(item) {
  const card = document.createElement("div");
  card.className = "cheat" + (item.on ? " on" : "") + (item.state === "ready" ? "" : " off-limits");

  const main = document.createElement("div");
  main.className = "cheat-main";

  const name = document.createElement("div");
  name.className = "cheat-name";
  name.textContent = item.name;

  const why = document.createElement("div");
  why.className = "cheat-why";
  if (item.state === "broken") {
    why.classList.add("dead");
    why.textContent = item.reason || "not found in this build";
  } else if (item.state === "wait") {
    why.classList.add("wait");
    why.textContent = item.hint || item.reason;
  } else {
    why.textContent = item.description;
  }

  main.append(name, why);

  const toggle = document.createElement("button");
  toggle.className = "switch" + (item.on ? " on" : "");
  toggle.disabled = item.state !== "ready" && !item.on;
  toggle.addEventListener("click", async () => {
    try {
      await invoke("set_cheat", { id: item.id, on: !item.on });
      await refreshCheats();
    } catch (e) {
      toast(String(e), true);
    }
  });

  card.append(main, toggle);
  return card;
}

/* ---------- finder ---------- */

function setScanStatus(report) {
  const noun = report.matches === 1 ? "address" : "addresses";
  $("scan-status").innerHTML = `Round ${report.round}: <b>${report.matches.toLocaleString()}</b> ${noun} left`;
  $("finder-filters").hidden = false;
  drawResults(report.results);
}

function drawResults(results) {
  const host = $("results");
  host.innerHTML = "";
  if (!results.length) return;

  if (results.length > 60) {
    host.innerHTML = `<div class="placeholder">Too many to list. Change the value in game and narrow it down.</div>`;
    return;
  }

  for (const hit of results) {
    const row = document.createElement("div");
    row.className = "row";

    const addr = document.createElement("span");
    addr.className = "addr";
    addr.textContent = hit.address;

    const val = document.createElement("span");
    val.className = "val";
    val.textContent = hit.value;

    const input = document.createElement("input");
    input.type = "text";
    input.placeholder = "new value";

    const write = document.createElement("button");
    write.className = "ghost";
    write.textContent = "Write";
    write.addEventListener("click", async () => {
      try {
        await invoke("write_value", {
          address: hit.address,
          kind: $("scan-type").value,
          value: input.value,
        });
        toast(`Wrote ${input.value} to ${hit.address}`);
      } catch (e) {
        toast(String(e), true);
      }
    });

    row.append(addr, val, input, write);
    host.appendChild(row);
  }
}

async function startScan() {
  if (!attached) return toast("Attach to a game first.", true);
  $("scan-start").disabled = true;
  $("scan-status").textContent = "Scanning";
  try {
    const report = await invoke("scan_start", {
      kind: $("scan-type").value,
      value: $("scan-value").value || null,
    });
    scanning = true;
    setScanStatus(report);
  } catch (e) {
    toast(String(e), true);
    $("scan-status").textContent = "";
  } finally {
    $("scan-start").disabled = false;
  }
}

async function narrow(filter) {
  if (!scanning) return toast("Run a first scan before narrowing.", true);

  let value = null;
  if (filter === "exact") {
    value = $("scan-value").value.trim();
    if (!value) return toast("Type the value it shows now, then press it again.", true);
  }

  $("scan-status").textContent = "Scanning";
  try {
    setScanStatus(await invoke("scan_next", { filter, value }));
  } catch (e) {
    toast(String(e), true);
  }
}

function resetScan() {
  scanning = false;
  $("finder-filters").hidden = true;
  $("scan-status").textContent = "";
  $("results").innerHTML = "";
  $("scan-value").value = "";
}

/* ---------- process sheet ---------- */

async function openProcesses() {
  const list = await invoke("list_processes");
  const host = $("process-list");

  const render = () => {
    const needle = $("process-filter").value.trim().toLowerCase();
    host.innerHTML = "";
    for (const p of list.filter((p) => !needle || p.name.toLowerCase().includes(needle))) {
      const button = document.createElement("button");
      const name = document.createElement("span");
      name.textContent = p.name;
      const pid = document.createElement("span");
      pid.className = "pid";
      pid.textContent = p.pid;
      button.append(name, pid);
      button.addEventListener("click", () => {
        $("sheet").hidden = true;
        doAttach(p.name);
      });
      host.appendChild(button);
    }
  };

  $("process-filter").oninput = render;
  render();
  $("sheet").hidden = false;
  $("process-filter").focus();
}

/* ---------- views ---------- */

function showView(name) {
  if (name === "game" && !open) name = "library";

  for (const id of ["library", "game", "finder", "settings", "about"]) {
    $(`view-${id}`).hidden = id !== name;
  }
  for (const item of document.querySelectorAll(".nav-item")) {
    const target = item.dataset.view;
    item.classList.toggle("active", target === name || (name === "game" && target === "library"));
  }

  if (name === "finder") {
    $("finder-blocked").hidden = !!attached;
    $("finder-panel").hidden = !attached;
    $("finder-target").textContent = attached ? `in ${attached.process}` : "";
  }
  if (name === "game") refreshCheats();
}

/* ---------- wiring ---------- */

document.querySelectorAll(".nav-item").forEach((item) => {
  item.addEventListener("click", () => {
    if (item.dataset.view === "library") open = null;
    showView(item.dataset.view);
    drawn = "";
    draw();
  });
});
document.querySelectorAll("[data-goto]").forEach((button) => {
  button.addEventListener("click", () => showView(button.dataset.goto));
});
document.querySelectorAll(".chip").forEach((chip) => {
  chip.addEventListener("click", () => narrow(chip.dataset.filter));
});
document.querySelectorAll("#theme-pick button").forEach((button) => {
  button.addEventListener("click", () => saveConfig({ theme: button.dataset.theme }));
});
document.querySelectorAll("#accent-pick button").forEach((button) => {
  button.addEventListener("click", () => saveConfig({ accent: button.dataset.accent }));
});

$("back").addEventListener("click", () => {
  open = null;
  showView("library");
  drawn = "";
  draw();
});

$("game-play").addEventListener("click", async () => {
  const game = gameFor(open);
  if (!game) return;
  try {
    await invoke("launch_game", { key: game.key });
    toast(`Starting ${game.name}`);
  } catch (e) {
    toast(String(e), true);
  }
});

$("game-attach").addEventListener("click", () => {
  const game = gameFor(open);
  if (!game) return;
  if (attached && attached.process === game.exe) doDetach();
  else if (game.exe) doAttach(game.exe);
  else toast("Could not work out which file to attach to.", true);
});

$("game-fav").addEventListener("click", () => {
  if (open) saveConfig({ favourites: toggleIn(config.favourites, open) });
});
$("game-pin").addEventListener("click", () => {
  if (open) saveConfig({ pinned: toggleIn(config.pinned, open) });
});
$("clear-pins").addEventListener("click", () => saveConfig({ pinned: [] }));
$("auto-update").addEventListener("click", () =>
  saveConfig({ auto_update: config.auto_update === false })
);
$("update-now").addEventListener("click", () => checkForTables(true));

$("copy-report").addEventListener("click", async () => {
  try {
    const text = await invoke("diagnostics");
    await navigator.clipboard.writeText(text);
    toast("Report copied, paste it into the issue");
  } catch (e) {
    toast(String(e), true);
  }
});
async function importTable(path) {
  const game = gameFor(open);
  try {
    const note = await invoke("import_table", { path, exe: game ? game.exe : null });
    toast(note);
    await loadGames(false);
    await refreshCheats();
  } catch (e) {
    toast(String(e), true);
  }
}

function searchForTable() {
  const game = gameFor(open);
  if (!game) return;
  invoke("find_table", { name: game.name }).catch((e) => toast(String(e), true));
}

$("game-find-table").addEventListener("click", searchForTable);
$("no-table-find").addEventListener("click", searchForTable);

$("game-folder").addEventListener("click", () => {
  const game = gameFor(open);
  if (!game) return;
  invoke("open_folder", { dir: game.dir }).catch((e) => toast(String(e), true));
});

/* Dropping a .CT anywhere on the window imports it. Tauri reports the drop
   itself, the webview never sees the file. */
if (window.__TAURI__.event) {
  const { listen } = window.__TAURI__.event;
  listen("tables-updated", async (e) => {
    toast(String(e.payload));
    $("tables-state").textContent = String(e.payload);
    await loadGames(false);
    await refreshCheats();
  });
  listen("tauri://drag-enter", () => document.body.classList.add("dropping"));
  listen("tauri://drag-leave", () => document.body.classList.remove("dropping"));
  listen("tauri://drag-drop", (e) => {
    document.body.classList.remove("dropping");
    const paths = (e.payload && e.payload.paths) || [];
    const table = paths.find((p) => p.toLowerCase().endsWith(".ct"));
    if (!table) return toast("Drop a Cheat Engine .CT file", true);
    importTable(table);
  });
}

$("open-log").addEventListener("click", async () => {
  try {
    await invoke("open_log");
  } catch (e) {
    toast(String(e), true);
  }
});

$("win-min").addEventListener("click", () => appWindow.minimize());
$("win-max").addEventListener("click", () => appWindow.toggleMaximize());
$("win-close").addEventListener("click", () => appWindow.close());

$("filter").addEventListener("input", draw);
$("refresh").addEventListener("click", () => loadGames(true));
$("scan-start").addEventListener("click", startScan);
$("scan-reset").addEventListener("click", resetScan);
$("show-processes").addEventListener("click", openProcesses);
$("empty-processes").addEventListener("click", openProcesses);
$("sheet-close").addEventListener("click", () => ($("sheet").hidden = true));
$("sheet").addEventListener("click", (e) => {
  if (e.target === $("sheet")) $("sheet").hidden = true;
});
$("scan-value").addEventListener("keydown", (e) => {
  if (e.key === "Enter") (scanning ? narrow("exact") : startScan());
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") $("sheet").hidden = true;
});

async function start() {
  try {
    config = await invoke("settings");
  } catch {
    // defaults are already in place
  }
  applyTheme();
  $("settings-path").textContent = "%APPDATA%\\freeplay\\settings.json";
  await loadGames();
  setInterval(loadGames, 5000);
}

start();
