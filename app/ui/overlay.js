const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

let attached = null;
let shape = null;
let cards = new Map();
let rows = [];

const many = (n, one) => `${n} ${n === 1 ? one : one + "s"}`;

/* ---------- drawing ---------- */

function whyFor(item) {
  if (item.live) return ["On", "live"];
  if (item.armed && item.state === "broken") {
    return [item.reason || "not in this version", "dead"];
  }
  if (item.armed && item.state === "wait") return [item.hint || item.reason, "wait"];
  if (item.armed) return ["Waiting for the game", "wait"];
  if (item.does === "Script") return ["Finds what the others need", ""];
  return [item.description || "", ""];
}

function card(item) {
  const row = document.createElement("div");
  row.className = "ov-cheat" + (item.armed ? " armed" : "") + (item.live ? " on" : "");

  const main = document.createElement("div");
  main.className = "ov-main";

  const name = document.createElement("div");
  name.className = "ov-name";
  name.textContent = item.name;

  const [line, tone] = whyFor(item);
  const why = document.createElement("div");
  why.className = "ov-why" + (tone ? " " + tone : "");
  why.textContent = line;

  main.append(name, why);
  if (item.editable) main.appendChild(valueBox(item));

  const toggle = document.createElement("button");
  toggle.className = "switch" + (item.armed ? " on" : "");
  toggle.title = item.armed ? "Turn it off" : "Turn it on";
  toggle.addEventListener("click", async () => {
    try {
      await invoke("set_cheat", { exe: attached.process, id: item.id, on: !item.armed });
      item.armed = !item.armed;
      patch(row, item);
      await refresh();
    } catch (e) {
      flash(String(e));
    }
  });

  row.append(main, toggle);
  return row;
}

function valueBox(item) {
  const wrap = document.createElement("div");
  wrap.className = "ov-value";

  const send = async (text, undo) => {
    try {
      await invoke("set_cheat_value", { exe: attached.process, id: item.id, value: text });
      item.value = text;
    } catch (e) {
      flash(String(e));
      undo();
    }
  };

  if (item.choices.length) {
    const pick = document.createElement("select");
    for (const choice of item.choices) {
      const option = document.createElement("option");
      option.value = choice.value;
      option.textContent = choice.label;
      pick.appendChild(option);
    }
    pick.value = item.value;
    pick.addEventListener("change", () => send(pick.value, () => (pick.value = item.value)));
    wrap.appendChild(pick);
  } else {
    const input = document.createElement("input");
    input.type = "text";
    input.spellcheck = false;
    input.value = item.value;
    input.placeholder = item.kind || "value";
    input.addEventListener("change", () => send(input.value, () => (input.value = item.value)));
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") input.blur();
    });
    wrap.appendChild(input);
  }

  const live = document.createElement("span");
  live.className = "ov-live";
  live.dataset.liveFor = item.id;
  live.textContent = item.current ? item.current : "";
  wrap.appendChild(live);
  return wrap;
}

/* patched rather than rebuilt, same as the main window. this refreshes while
   you are looking at it and rebuilding would fight whatever you are typing */
function patch(row, item) {
  const wanted = "ov-cheat" + (item.armed ? " armed" : "") + (item.live ? " on" : "");
  if (row.className !== wanted) row.className = wanted;

  const why = row.querySelector(".ov-why");
  const [line, tone] = whyFor(item);
  if (why.textContent !== line) why.textContent = line;
  const toned = "ov-why" + (tone ? " " + tone : "");
  if (why.className !== toned) why.className = toned;

  const toggle = row.querySelector(".switch");
  const on = "switch" + (item.armed ? " on" : "");
  if (toggle.className !== on) {
    toggle.className = on;
    toggle.title = item.armed ? "Turn it off" : "Turn it on";
  }

  const live = row.querySelector(".ov-live");
  if (live && live.textContent !== (item.current || "")) {
    live.textContent = item.current || "";
  }
}

function draw() {
  const host = $("ov-list");
  const next = attached ? rows.map((r) => r.id + r.category).join("|") : "";

  if (next !== shape) {
    shape = next;
    cards = new Map();
    host.innerHTML = "";

    const byCategory = new Map();
    for (const row of rows) {
      if (!byCategory.has(row.category)) byCategory.set(row.category, []);
      byCategory.get(row.category).push(row);
    }

    for (const [category, items] of byCategory) {
      const group = document.createElement("div");
      group.className = "ov-group";
      const heading = document.createElement("h3");
      heading.textContent = category;
      group.appendChild(heading);
      for (const item of items) {
        const built = card(item);
        cards.set(item.id, built);
        group.appendChild(built);
      }
      host.appendChild(group);
    }
    applyFilter();
  } else {
    for (const row of rows) patch(cards.get(row.id), row);
  }

  const armed = rows.filter((r) => r.armed).length;
  $("ov-count").textContent = rows.length
    ? `${armed} of ${many(rows.length, "cheat")} on`
    : "";
}

/* ---------- filter ---------- */

let timer = null;
$("ov-filter").addEventListener("input", () => {
  clearTimeout(timer);
  timer = setTimeout(applyFilter, 160);
});

function applyFilter() {
  const needle = $("ov-filter").value.trim().toLowerCase();
  let shown = 0;

  for (const [, row] of cards) {
    const name = row.querySelector(".ov-name").textContent.toLowerCase();
    const hit = !needle || name.includes(needle);
    row.hidden = !hit;
    if (hit) shown++;
  }
  for (const group of document.querySelectorAll(".ov-group")) {
    group.hidden = ![...group.querySelectorAll(".ov-cheat")].some((c) => !c.hidden);
  }
  if (needle && !shown && cards.size) say("Nothing matches that");
  else if (cards.size) say(null);
}

function say(text) {
  $("ov-empty").hidden = !text;
  $("ov-empty").textContent = text || "";
  $("ov-filter").hidden = !cards.size;
}

function flash(message) {
  say(message);
  clearTimeout(flash.timer);
  flash.timer = setTimeout(() => applyFilter(), 4000);
}

/* ---------- loop ---------- */

async function refresh() {
  let now = null;
  try {
    now = await invoke("overlay_game");
  } catch {
    return;
  }

  const changed = (now && now.process) !== (attached && attached.process);
  attached = now;

  if (!attached) {
    $("ov-game").textContent = "Nothing attached";
    $("ov-sub").textContent = "Start a game and Freeplay picks it up";
    rows = [];
    shape = null;
    cards = new Map();
    $("ov-list").innerHTML = "";
    say("Nothing to switch on yet");
    return;
  }

  $("ov-game").textContent = attached.game;
  $("ov-sub").textContent = `${attached.process} . pid ${attached.pid} . ${attached.arch}`;

  if (changed) {
    shape = null;
    $("ov-filter").value = "";
  }

  try {
    rows = await invoke("cheats", { exe: attached.process });
  } catch {
    return;
  }

  if (!rows.length) {
    shape = null;
    cards = new Map();
    $("ov-list").innerHTML = "";
    say("No cheat table for this game");
    return;
  }
  say(null);
  draw();
}

$("ov-close").addEventListener("click", () => invoke("hide_overlay"));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") invoke("hide_overlay");
});

async function start() {
  try {
    const state = await invoke("overlay_status");
    $("ov-key").textContent = state.key;
  } catch {
    // the footer just goes without it
  }
  await refresh();
  setInterval(refresh, 1000);
}

start();
