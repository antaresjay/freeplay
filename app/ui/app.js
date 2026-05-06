const { invoke } = window.__TAURI__.core;
const appWindow = window.__TAURI__.window.getCurrentWindow();
const $ = (id) => document.getElementById(id);

let games = [];
let attached = null;
let scanning = false;
let cheatTimer = null;
let view = "library";
let drawn = "";

/* art is read off disk and base64'd, so ask once and keep it */
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

/* ---------- art ---------- */

function artFor(game) {
  return (game.app_id && art.get(game.app_id)) || null;
}

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

/* ---------- library ---------- */

async function loadGames() {
  try {
    games = await invoke("list_games");
  } catch (e) {
    games = [];
    toast(String(e), true);
  }
  draw();
  loadArt();
}

/* Playable and running first, anything with an anti-cheat last. */
function ordered() {
  const rank = (g) => (g.guard ? 2 : 0) - (g.running ? 1 : 0);
  return [...games].sort(
    (a, b) => rank(a) - rank(b) || b.has_table - a.has_table || a.name.localeCompare(b.name)
  );
}

/* Redrawing wipes hover and restarts image decoding, and the list is polled
   every few seconds, so only rebuild when something actually moved. */
function signature(list) {
  return list
    .map((g) => `${g.name}${g.running}${g.has_table}${g.guard}${art.has(g.app_id)}`)
    .join("|") + `#${$("filter").value}#${attached ? attached.process : ""}`;
}

function draw() {
  const list = ordered();
  const stamp = signature(list);
  if (stamp === drawn) return;
  drawn = stamp;

  drawRail(list);
  drawSpotlight(list);
  drawGrid(list);

  const live = list.filter((g) => g.running).length;
  $("library-count").textContent = `${list.length} games, ${live} running`;
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
    button.className = "rail-game" + (attached && attached.process === game.exe ? " active" : "");

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
    button.addEventListener("click", () => pick(game));
    host.appendChild(button);
  }
}

function drawSpotlight(list) {
  const running = list.find((g) => g.running && !g.guard);
  const box = $("spotlight");
  $("idle-banner").hidden = !!running || !!attached || !list.length;

  if (!running || attached) {
    box.hidden = true;
    return;
  }

  const images = artFor(running) || {};
  const hero = $("spotlight-hero");
  hero.src = images.hero || images.cover || "";
  hero.hidden = !hero.src;

  const logo = $("spotlight-logo");
  logo.src = images.logo || "";
  logo.hidden = !images.logo;

  $("spotlight-name").textContent = running.name;
  $("spotlight-sub").textContent = running.has_table
    ? "There is a table for this one. Attach and the cheats are ready."
    : "No table yet, but you can find values yourself once attached.";
  $("spotlight-attach").onclick = () => pick(running);
  box.hidden = false;
}

function drawGrid(list) {
  const host = $("grid");
  const needle = $("filter").value.trim().toLowerCase();
  const shown = list.filter((g) => !needle || g.name.toLowerCase().includes(needle));

  host.innerHTML = "";
  $("library-empty").hidden = games.length > 0;

  for (const game of shown) {
    const card = document.createElement("button");
    card.className = "card" + (game.guard ? " guarded" : game.running ? "" : " idle");

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
    label.textContent = game.guard ? "Off limits" : game.running ? "Attach" : "Not running";
    overlay.appendChild(label);
    box.appendChild(overlay);

    const title = document.createElement("span");
    title.className = "card-title";
    title.textContent = game.name;

    const sub = document.createElement("span");
    sub.className = "card-sub";
    sub.textContent = `${game.store} · ${game.exe || "no executable found"}`;

    card.append(box, title, sub);
    card.addEventListener("click", () => pick(game));
    host.appendChild(card);
  }
}

function badge(text, extra) {
  const span = document.createElement("span");
  span.className = `badge ${extra}`;
  span.textContent = text;
  return span;
}

function pick(game) {
  if (game.guard) {
    return toast(
      `${game.name} ships ${game.guard}. Freeplay is for single player games, and attaching would risk your account.`,
      true
    );
  }
  if (!game.exe) return toast("Could not work out which file to attach to.", true);
  if (!game.running) return toast(`${game.name} is not running. Launch it first.`, true);
  doAttach(game.exe, game);
}

/* ---------- attaching ---------- */

async function doAttach(exe, game) {
  try {
    attached = await invoke("attach", { exe });
  } catch (e) {
    toast(String(e), true);
    return;
  }

  const known = game || games.find((g) => g.exe === exe);
  const images = (known && artFor(known)) || {};

  const hero = $("game-hero-img");
  hero.src = images.hero || images.cover || "";
  hero.hidden = !hero.src;
  document.querySelector(".game-hero").classList.toggle("no-art", !hero.src);

  const logo = $("game-logo");
  logo.src = images.logo || "";
  logo.hidden = !images.logo;

  $("game-name").textContent = known ? known.name : attached.game;
  $("game-process").textContent = `${attached.process} · pid ${attached.pid}`;

  showView("game");
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
  showView("library");
  drawn = "";
  draw();
}

/* ---------- cheats ---------- */

async function refreshCheats() {
  if (!attached) return;
  let rows = [];
  try {
    rows = await invoke("cheats");
  } catch {
    return;
  }

  $("no-table").hidden = rows.length > 0;
  const ready = rows.filter((r) => r.state === "ready").length;
  $("game-ready").textContent = rows.length
    ? `${ready} of ${rows.length} ready`
    : "no table";

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
  // The game page only exists while something is attached.
  if (name === "game" && !attached) name = "library";
  view = name;

  for (const id of ["library", "game", "finder", "about"]) {
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
}

/* ---------- wiring ---------- */

document.querySelectorAll(".nav-item").forEach((item) => {
  item.addEventListener("click", () => {
    const target = item.dataset.view;
    showView(target === "library" && attached ? "game" : target);
  });
});
document.querySelectorAll("[data-goto]").forEach((button) => {
  button.addEventListener("click", () => showView(button.dataset.goto));
});
document.querySelectorAll(".chip").forEach((chip) => {
  chip.addEventListener("click", () => narrow(chip.dataset.filter));
});

$("win-min").addEventListener("click", () => appWindow.minimize());
$("win-max").addEventListener("click", () => appWindow.toggleMaximize());
$("win-close").addEventListener("click", () => appWindow.close());

$("filter").addEventListener("input", draw);
$("refresh").addEventListener("click", loadGames);
$("detach").addEventListener("click", doDetach);
$("scan-start").addEventListener("click", startScan);
$("scan-reset").addEventListener("click", resetScan);
$("show-processes").addEventListener("click", openProcesses);
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

loadGames();
setInterval(loadGames, 5000);
