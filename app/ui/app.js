const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

let games = [];
let attached = null;
let scanning = false;
let cheatTimer = null;

function toast(message, bad = false) {
  const el = $("toast");
  el.textContent = message;
  el.classList.toggle("bad", bad);
  el.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (el.hidden = true), bad ? 6000 : 3000);
}

/* ---------- library ---------- */

async function loadGames() {
  try {
    games = await invoke("list_games");
  } catch (e) {
    games = [];
    toast(String(e), true);
  }
  drawLibrary();
}

function drawLibrary() {
  const needle = $("filter").value.trim().toLowerCase();
  const shown = games.filter((g) => !needle || g.name.toLowerCase().includes(needle));
  const list = $("library");
  list.innerHTML = "";

  if (!shown.length) {
    list.innerHTML = `<div class="placeholder">${
      games.length ? "Nothing matches that." : "No games found. Steam, Epic and GOG are checked."
    }</div>`;
  }

  // Running games first, then ones we have a table for.
  shown.sort((a, b) => b.running - a.running || b.has_table - a.has_table || a.name.localeCompare(b.name));

  for (const game of shown) {
    const button = document.createElement("button");
    button.className = "game" + (attached && attached.process === game.exe ? " active" : "");
    button.innerHTML = `
      <span class="title"></span>
      <span class="pip ${game.running ? "live" : ""}"></span>
      <span class="meta"></span>`;
    button.querySelector(".title").textContent = game.name;

    const meta = button.querySelector(".meta");
    meta.textContent = `${game.store} · ${game.exe || "no executable found"}`;
    if (game.has_table) {
      const tag = document.createElement("span");
      tag.className = "tag";
      tag.textContent = "table";
      meta.appendChild(tag);
    }

    button.addEventListener("click", () => {
      if (!game.exe) return toast("Could not work out which file to attach to.", true);
      if (!game.running) return toast(`${game.name} is not running. Launch it first.`, true);
      doAttach(game.exe);
    });
    list.appendChild(button);
  }

  $("library-count").textContent = `${games.length} games, ${games.filter((g) => g.running).length} running`;
}

/* ---------- attaching ---------- */

async function doAttach(exe) {
  try {
    attached = await invoke("attach", { exe });
  } catch (e) {
    toast(String(e), true);
    return;
  }

  $("status-dot").classList.add("on");
  $("attached-name").textContent = attached.game;
  $("attached-sub").textContent = `${attached.process} · pid ${attached.pid}`;
  $("detach").hidden = false;
  $("tabs").hidden = false;
  $("view-empty").hidden = true;

  showTab("cheats");
  drawLibrary();
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

  $("status-dot").classList.remove("on");
  $("attached-name").textContent = "Nothing attached";
  $("attached-sub").textContent = "Pick a running game on the left";
  $("detach").hidden = true;
  $("tabs").hidden = true;
  for (const id of ["view-cheats", "view-finder"]) $(id).hidden = true;
  $("view-empty").hidden = false;
  drawLibrary();
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
    group.appendChild(heading);

    for (const item of items) {
      group.appendChild(cheatRow(item));
    }
    host.appendChild(group);
  }
}

function cheatRow(item) {
  const row = document.createElement("div");
  row.className = "cheat" + (item.state === "ready" ? "" : " off-limits");

  const name = document.createElement("div");
  name.className = "name";
  name.textContent = item.name;

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

  const why = document.createElement("div");
  why.className = "why";
  if (item.state === "broken") {
    why.classList.add("dead");
    why.textContent = item.reason || "not found in this build";
  } else if (item.state === "wait") {
    why.classList.add("bad");
    why.textContent = item.hint || item.reason;
  } else {
    why.textContent = item.description;
  }

  row.append(name, toggle, why);
  return row;
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
    row.innerHTML = `<span class="addr"></span><span class="val"></span>`;
    row.querySelector(".addr").textContent = hit.address;
    row.querySelector(".val").textContent = hit.value;

    const input = document.createElement("input");
    input.placeholder = "new value";
    const write = document.createElement("button");
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

    const cell = document.createElement("span");
    cell.style.display = "flex";
    cell.style.gap = "6px";
    cell.append(input, write);
    row.appendChild(cell);
    host.appendChild(row);
  }
}

async function startScan() {
  if (!attached) return toast("Attach to a game first.", true);
  $("scan-start").disabled = true;
  $("scan-status").textContent = "Scanning…";
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

  $("scan-status").textContent = "Scanning…";
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

  const draw = () => {
    const needle = $("process-filter").value.trim().toLowerCase();
    host.innerHTML = "";
    for (const p of list.filter((p) => !needle || p.name.toLowerCase().includes(needle))) {
      const button = document.createElement("button");
      button.innerHTML = `<span></span><span class="pid"></span>`;
      button.children[0].textContent = p.name;
      button.children[1].textContent = p.pid;
      button.addEventListener("click", () => {
        $("sheet").hidden = true;
        doAttach(p.name);
      });
      host.appendChild(button);
    }
  };

  $("process-filter").oninput = draw;
  draw();
  $("sheet").hidden = false;
  $("process-filter").focus();
}

/* ---------- tabs and wiring ---------- */

function showTab(name) {
  for (const tab of document.querySelectorAll(".tab")) {
    tab.classList.toggle("active", tab.dataset.tab === name);
  }
  $("view-cheats").hidden = name !== "cheats";
  $("view-finder").hidden = name !== "finder";
}

document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => showTab(tab.dataset.tab));
});
document.querySelectorAll(".chip").forEach((chip) => {
  chip.addEventListener("click", () => narrow(chip.dataset.filter));
});

$("filter").addEventListener("input", drawLibrary);
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

loadGames();
setInterval(loadGames, 5000);
