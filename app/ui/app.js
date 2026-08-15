const { invoke } = window.__TAURI__.core;
const appWindow = window.__TAURI__.window.getCurrentWindow();
const $ = (id) => document.getElementById(id);

let games = [];
let attached = null;
let scanning = false;
let open = null; // key of the game whose page is showing
let drawn = "";
let config = { theme: "system", accent: "amber", favourites: [] };
let me = null; // the name we publish under, if any
let sharedFor = null; // the exe the shared list on screen belongs to
let folded = new Set(); // cheat categories shut on the game page, by name

/* art is read off disk and served over its own protocol, so ask once */
const art = new Map();
const pending = new Set();

/* error strings come up from rust, where they start lowercase by convention,
   and they land in front of somebody exactly as written. fixing the first
   letter here beats fixing it in fifty places and forgetting the next one.
   a first word with a dot in it is a file name, and Game.exe is wrong */
function sentence(text) {
  const first = String(text).split(" ")[0];
  if (!text || first.includes(".") || first.includes("_")) return text;
  return text[0].toUpperCase() + text.slice(1);
}

function toast(message, bad = false) {
  const el = $("toast");
  el.textContent = sentence(message);
  el.classList.toggle("bad", bad);
  el.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (el.hidden = true), bad ? 6000 : 3000);
}

const gameFor = (key) => games.find((g) => g.key === key);

/* ---------- opening screen ---------- */

const bootedAt = Date.now();

const bootStep = (what) => {
  const step = $("splash-step");
  if (step) step.textContent = what;
};

/* held for a moment even when everything is instant, because something that
   appears and vanishes inside one frame reads as a glitch rather than a screen */
async function stopBooting() {
  const splash = $("splash");
  if (!splash || splash.classList.contains("gone")) return;

  const waited = Date.now() - bootedAt;
  if (waited < 1200) await new Promise((r) => setTimeout(r, 1200 - waited));

  document.body.classList.remove("booting");
  splash.classList.add("gone");
  setTimeout(() => splash.remove(), 600);
}

// nothing below is allowed to leave it sitting there
setTimeout(stopBooting, 12000);

/* ---------- dropdowns ---------- */

/* a native select opens an operating system menu that knows nothing about the
   theme: grey highlight, square corners, its own font. this wraps one in a
   list we draw ourselves and leaves the select in the page, so everything that
   reads .value or listens for change carries on working */

let openPicker = null;

function enhanceSelect(select) {
  if (select.picker) return select.picker.sync();

  // the class on the select styles a text input, and putting it on the
  // wrapper too would draw a second border round the face
  const wrap = document.createElement("div");
  wrap.className = "picker";

  const button = document.createElement("button");
  button.type = "button";
  button.className = "picker-face";
  const label = document.createElement("span");
  const chevron = document.createElement("svg");
  chevron.innerHTML = '<path d="M3.5 5.5L7 9l3.5-3.5"/>';
  chevron.setAttribute("viewBox", "0 0 14 14");
  chevron.setAttribute("class", "picker-chevron");
  button.append(label, chevron);

  const menu = document.createElement("div");
  menu.className = "picker-menu";
  menu.hidden = true;
  // the menu is fixed to the viewport so a panel with its own scrollbar
  // cannot clip it
  document.body.appendChild(menu);

  select.parentNode.insertBefore(wrap, select);
  wrap.append(button, select);

  const api = {
    sync() {
      const chosen = select.options[select.selectedIndex];
      label.textContent = chosen ? chosen.textContent : "";
    },
    close() {
      menu.hidden = true;
      button.classList.remove("open");
      if (openPicker === api) openPicker = null;
    },
    open() {
      if (openPicker && openPicker !== api) openPicker.close();
      openPicker = api;

      menu.innerHTML = "";
      [...select.options].forEach((option, at) => {
        const item = document.createElement("button");
        item.type = "button";
        item.className = "picker-item" + (at === select.selectedIndex ? " on" : "");
        item.textContent = option.textContent;
        item.addEventListener("click", () => {
          select.selectedIndex = at;
          api.sync();
          api.close();
          button.focus();
          select.dispatchEvent(new Event("change", { bubbles: true }));
        });
        menu.appendChild(item);
      });

      menu.hidden = false;
      button.classList.add("open");
      place();
      const on = menu.querySelector(".picker-item.on") || menu.firstChild;
      if (on) on.focus();
    },
  };

  function place() {
    const box = button.getBoundingClientRect();
    const room = window.innerHeight - box.bottom;
    const height = menu.offsetHeight;

    menu.style.minWidth = `${box.width}px`;
    menu.style.left = `${Math.min(box.left, window.innerWidth - menu.offsetWidth - 8)}px`;
    // flip above when there is no room below, rather than running off screen
    menu.style.top =
      room < height + 12 && box.top > height + 12
        ? `${box.top - height - 6}px`
        : `${box.bottom + 6}px`;
  }

  button.addEventListener("click", () => (menu.hidden ? api.open() : api.close()));
  button.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      api.open();
    }
  });

  menu.addEventListener("keydown", (e) => {
    const items = [...menu.querySelectorAll(".picker-item")];
    const at = items.indexOf(document.activeElement);

    if (e.key === "Escape") {
      e.preventDefault();
      api.close();
      button.focus();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      items[Math.min(at + 1, items.length - 1)].focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      items[Math.max(at - 1, 0)].focus();
    } else if (e.key === "Home") {
      e.preventDefault();
      items[0].focus();
    } else if (e.key === "End") {
      e.preventDefault();
      items[items.length - 1].focus();
    } else if (e.key === "Tab") {
      api.close();
    } else if (e.key.length === 1) {
      // type the first letter to jump, the way the native one does
      const found = items.find((i) =>
        i.textContent.toLowerCase().startsWith(e.key.toLowerCase())
      );
      if (found) found.focus();
    }
  });

  select.picker = api;
  api.sync();
  return api;
}

document.addEventListener("pointerdown", (e) => {
  if (!openPicker) return;
  if (e.target.closest(".picker") || e.target.closest(".picker-menu")) return;
  openPicker.close();
});
window.addEventListener("resize", () => openPicker && openPicker.close());
// scrolling the page underneath would leave the menu floating on its own
document.addEventListener("scroll", () => openPicker && openPicker.close(), true);

/* right clicking anywhere offered Back, Refresh, Save as, Print and Send tab
   to your devices, which is the webview underneath saying out loud that it is
   a browser. text boxes keep theirs, that one is cut and paste */
/* the target of a key or click is not always an element. it can be the
   document itself, which has no closest() */
const inABox = (target) =>
  !!target && typeof target.closest === "function" && !!target.closest("input, textarea");

document.addEventListener("contextmenu", (e) => {
  if (!inABox(e.target)) e.preventDefault();
});

/* the rest of what the webview brings with it. ctrl+f opened a find bar over
   the window, ctrl+p offered to print the game page, f5 reloaded the whole
   app back to the library. none of them belong in a desktop program and every
   one of them is somebody's bug report.

   editing keys are deliberately not here: ctrl+c, ctrl+v, ctrl+x, ctrl+a,
   ctrl+z and ctrl+y all still work, and the text boxes need them. */
const BROWSER_KEYS = new Set([
  "f", // find bar
  "g", // find next
  "p", // print
  "r", // reload
  "s", // save page
  "o", // open a file into the webview
  "u", // view source
  "j", // downloads
  "h", // history
  "d", // bookmark
  "n", // new window
  "t", // new tab
  "=", // zoom in, and the unshifted +
  "+",
  "-",
  "_",
  "0", // zoom back to normal
]);

const BROWSER_FKEYS = new Set(["F3", "F5", "F6", "F7", "F11", "F12"]);

document.addEventListener(
  "keydown",
  (e) => {
    const typing = inABox(e.target);

    if (BROWSER_FKEYS.has(e.key)) {
      e.preventDefault();
      return;
    }
    // devtools, and the reload that ignores the cache
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && "IJCR".includes(e.key.toUpperCase())) {
      e.preventDefault();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && BROWSER_KEYS.has(e.key.toLowerCase())) {
      e.preventDefault();
      return;
    }
    // alt+arrow is back and forward, and there is nowhere to go back to
    if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      e.preventDefault();
      return;
    }
    // backspace outside a text box used to mean back
    if (e.key === "Backspace" && !typing) e.preventDefault();
  },
  true
);

// ctrl and the wheel zooms the whole window, which throws every layout out
document.addEventListener(
  "wheel",
  (e) => {
    if (e.ctrlKey) e.preventDefault();
  },
  { passive: false, capture: true }
);

/* windows does not care about case in a file name and neither does anything
   that matches a table to a game. attaching reports whatever the table calls
   the executable, which for everything converted is lowercase, while the
   library reports the real file name. comparing those two exactly is what had
   an attached game insisting it was not attached */
const sameExe = (a, b) =>
  !!a && !!b && String(a).toLowerCase() === String(b).toLowerCase();

const many = (n, one, more) => `${n} ${n === 1 ? one : more || one + "s"}`;

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
  $("fav-total").textContent = config.favourites.length
    ? `${config.favourites.length} starred`
    : "Nothing starred yet";
  $("auto-update").classList.toggle("on", config.auto_update !== false);
  $("community-on").classList.toggle("on", config.community !== false);
  $("auto-attach").classList.toggle("on", config.auto_attach !== false);
  $("chirp-on").classList.toggle("on", config.chirp !== false);
  $("panic-key").textContent = config.panic || "None";
  $("panic-off").hidden = !config.panic;
  const tucked = (config.hidden || []).length;
  $("hidden-note").textContent = tucked
    ? many(tucked, "game") + " hidden from the library"
    : "Nothing is hidden. Hide soundtracks and tools from a game's page.";
  $("unhide-all").hidden = !tucked;
  drawDock();
}

async function drawVersion() {
  try {
    $("about-version").textContent = await invoke("version");
  } catch {
    // the about page just goes without it
  }
}

/* it used to say "Checking" until you pressed the button yourself, which read
   as though something was stuck */
async function countTables() {
  try {
    const held = await invoke("table_count");
    $("tables-state").textContent =
      config.auto_update === false ? `${held}, checking turned off` : held;
  } catch (e) {
    $("tables-state").textContent = String(e);
  }
}

async function checkForTables(manual) {
  const label = $("tables-state");
  label.textContent = "Checking";
  try {
    const note = await invoke("update_tables");
    if (manual) toast(note);
    await loadGames(false);
    await refreshCheats();
    await countTables();
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
  /* the favourite flag is worked out back there and comes down with the games
     list, which only refreshes every few seconds. without this you have to
     leave the page and come back to see the star fill in */
  for (const game of games) {
    game.favourite = config.favourites.includes(game.key);
  }
  drawn = "";
  draw();
  if (open) drawGamePage();
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

/* running first, then anything with a table, anti-cheat last */
function ordered(list) {
  const rank = (g) => (g.guard ? 2 : 0) - (g.running ? 1 : 0);
  return [...list].sort(
    (a, b) => rank(a) - rank(b) || b.has_table - a.has_table || a.name.localeCompare(b.name)
  );
}

/* redrawing wipes hover and restarts image decoding, and this polls every few
   seconds, so only rebuild when something actually moved */
function signature(list) {
  return (
    list
      .map((g) => `${g.key}${g.running}${g.has_table}${g.favourite}${art.has(g.app_id)}`)
      .join("|") + `#${$("filter").value}#${attached ? attached.process : ""}`
  );
}

function draw() {
  const hidden = config.hidden || [];
  const list = ordered(games.filter((g) => !hidden.includes(g.key)));
  const stamp = signature(list) + "#" + hidden.length;
  if (stamp === drawn) return;
  drawn = stamp;

  // the filter narrows the rail too. with a hundred games installed, the
  // strip is where you actually look for one
  const needle = $("filter").value.trim().toLowerCase();
  drawRail(needle ? list.filter((g) => g.name.toLowerCase().includes(needle)) : list);
  drawGrids(list);

  const live = list.filter((g) => g.running).length;
  $("library-count").textContent = `${many(list.length, "game")}, ${live} running`;
  $("idle-banner").hidden = live > 0 || !list.length;
}

function drawRail(list) {
  const host = $("library-rail");
  host.innerHTML = "";

  if (!list.length) {
    host.innerHTML = `<div class="placeholder">Nothing found yet</div>`;
    return;
  }

  let split = false;
  for (const game of list) {
    /* they are last in the order already, so one line here is enough to say
       why the ones under it look different */
    if (game.guard && !split) {
      split = true;
      const head = document.createElement("div");
      head.className = "rail-split";
      head.textContent = "Not supported";
      host.appendChild(head);
    }

    const button = document.createElement("button");
    button.className =
      "rail-game" + (open === game.key ? " active" : "") + (game.guard ? " barred" : "");

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

    let mark;
    if (game.guard) {
      mark = document.createElement("span");
      mark.className = "rail-barred";
      mark.title = game.name + " runs " + game.guard;
      mark.innerHTML =
        '<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="5.6"/><path d="M4 4l8 8"/></svg>';
    } else {
      mark = document.createElement("span");
      mark.className = "pip" + (game.running ? " live" : "");
    }

    button.append(thumb, name, mark);
    button.addEventListener("click", () => showGame(game.key));
    host.appendChild(button);
  }
}

function drawGrids(list) {
  const needle = $("filter").value.trim().toLowerCase();
  const shown = list.filter((g) => !needle || g.name.toLowerCase().includes(needle));

  /* a game with an anti-cheat cannot be attached to whatever else is true of
     it, so it goes to the end on its own rather than sitting in favourites
     next to something that works */
  const barred = shown.filter((g) => g.guard);
  const usable = shown.filter((g) => !g.guard);

  /* starring a game filled in the star and did nothing else, which is not a
     feature, it is a button that lies */
  const favourite = usable.filter((g) => g.favourite);
  const rest = usable.filter((g) => !g.favourite);

  $("fav-wrap").hidden = !favourite.length;
  $("blocked-wrap").hidden = !barred.length;
  /* "everything else" only means anything as a contrast to a shelf above it.
     with nothing starred there is nothing above, and it ends up calling your
     whole library the leftovers. the anti-cheat shelf does not count, that
     sits below and has its own heading */
  $("rest-shelf").hidden = !rest.length || !favourite.length;
  $("fav-count").textContent = favourite.length;
  $("rest-count").textContent = rest.length;
  $("blocked-count").textContent = barred.length;
  fill($("fav-grid"), favourite);
  fill($("grid"), rest);
  fill($("blocked-grid"), barred);

  const blank = $("library-empty");
  blank.hidden = shown.length > 0;
  const filtered = needle && games.length > 0;
  blank.querySelector("h3").textContent = filtered
    ? "Nothing matches that"
    : "No games found";
  blank.querySelector("p").textContent = filtered
    ? `No installed game has "${$("filter").value.trim()}" in its name.`
    : "Steam, Epic and GOG are all checked. If yours is installed somewhere unusual, attach to it by process instead.";
  $("empty-processes").hidden = !!filtered;
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
  if (game.favourite) badges.appendChild(badge("Favourite", "fav"));
  box.appendChild(badges);

  const overlay = document.createElement("div");
  overlay.className = "card-overlay";
  const label = document.createElement("b");
  // clicking always opens the game page. saying "attach" here promised
  // something the click does not do
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

  /* a different game means everything below is about to be replaced. wipe it
     now and put skeletons up, or the old table sits there looking current
     until the next answer lands */
  if (drawGamePage.showing !== open) {
    drawGamePage.showing = open;
    refreshCheats.shape = null;
    paintFit.done = null;
    paintTables.done = null;
    sharedFor = null;
    // the count the empty state quotes belongs to the game you just left
    sharedRows = [];
    ownRows = null;
    $("cheat-filter").value = "";
    $("shared-search").value = "";
    // a search is about the game you were on, not the one you just opened
    $("table-search").value = "";
    searchFor = "";
    ownRows = null;
    $("search-note").hidden = true;
    if (!$("cheat-groups").children.length) skeletons(game.has_table);
  }
  const images = artFor(game) || {};

  const hero = $("game-hero-img");
  hero.src = images.hero || "";
  // an empty src reads back as this page's own url, so test the value we set
  hero.hidden = !images.hero;
  document.querySelector(".game-hero").classList.toggle("no-art", !images.hero);

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
  if (game.version) facts.appendChild(fact("Version", game.version));
  const played = playedFor(game.minutes);
  if (played) facts.appendChild(fact("Play time", played));
  const seen = lastPlayed(game.last_played);
  if (seen) facts.appendChild(fact("Last played", seen));
  // four of them is a paragraph, and the row has the title sitting next to it
  const kinds = game.genres || [];
  if (kinds.length) {
    const genre = fact("Genre", kinds.slice(0, 2).join(", "));
    genre.title = kinds.join(", ");
    facts.appendChild(genre);
  }
  if (attached && sameExe(attached.process, game.exe)) {
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
  $("detail-id").title = game.app_id || "";
  $("detail-exe").title = game.exe || "";
  $("detail-guard-row").hidden = !game.guard;
  // the product when we can name it, the file we saw when we cannot, because
  // "Anti-cheat: an anti-cheat" tells nobody anything
  $("detail-guard").textContent = game.guard_file
    ? "unrecognised, found " + game.guard_file
    : game.guard || "";
  $("game-folder").disabled = !game.dir;
  // only for games that were pointed at by hand. the stores put theirs back
  // on the next scan, so offering to remove those would be a lie
  $("game-remove").hidden = game.store !== "Manual";


  const fav = $("game-fav");
  fav.classList.toggle("on", game.favourite);
  fav.title = game.favourite ? "Remove from favourites" : "Add to favourites";

  const isAttached = attached && sameExe(attached.process, game.exe);
  const attach = $("game-attach");
  attach.textContent = isAttached ? "Detach" : "Attach";
  attach.disabled = !!game.guard || (!game.running && !isAttached);

  // it said Play with the game already running, which either starts a second
  // copy or does nothing at all
  const play = $("game-play");
  play.textContent = game.running ? "Switch to game" : "Play";
  play.title = game.running
    ? "Bring the game back to the front"
    : "Start it the way its store expects";
  play.disabled = !!game.guard;

  /* offering to import a table for a game we refuse to attach to is telling
     somebody to go and get banned */
  const guarded = !!game.guard;
  $("guarded-note").hidden = !guarded;
  $("guarded-name").textContent = game.name;
  $("guarded-which").textContent = game.guard || "";
  $("shared").hidden = guarded || config.shared_open === false;
  $("dock-open").hidden = guarded || config.shared_open !== false;
  document.querySelector(".game-layout").classList.toggle("alone", guarded);
  $("game-import").hidden = guarded;
  $("game-find-table").hidden = guarded;
  if (guarded) {
    $("cheats-panel").hidden = true;
    $("no-table").hidden = true;
  }

  const note = $("attach-note");
  if (game.guard) {
    note.hidden = true;
  } else if (!game.running) {
    note.hidden = false;
    $("attach-note-title").textContent = "The game is not running";
    $("attach-note-body").textContent = "Set up whatever you want now. Freeplay attaches on its own once the game is open and turns everything on for you.";
  } else if (!isAttached) {
    note.hidden = false;
    $("attach-note-title").textContent = "Running, not attached yet";
    $("attach-note-body").textContent = "Attach and the cheats below start working.";
  } else {
    note.hidden = true;
  }

  refreshCheats();
  loadShared();
}

/* ---------- attaching ---------- */

async function doAttach(exe) {
  try {
    attached = await invoke("attach", { exe });
  } catch (e) {
    const text = String(e);
    /* access denied means the game is elevated and freeplay is not. that is
       fixable with one click, so offer the fix rather than just the words */
    if (text.includes("administrator")) {
      $("elevate-why").textContent = text;
      $("elevate-sheet").hidden = false;
    } else {
      toast(text, true);
    }
    return;
  }
  // attaching starts a new process with a new address space, and the backend
  // drops the old search, so nothing already on the finder still means anything
  resetScan();

  const known = games.find((g) => g.exe === exe);
  if (known) open = known.key;

  if (open) {
    drawGamePage();
    showView("game");
  }
  drawn = "";
  draw();
  await refreshCheats();
}

async function doDetach() {
  try {
    await invoke("detach");
  } catch (e) {
    return toast(String(e), true);
  }
  attached = null;
  scanning = false;
  resetScan();
  if (open) drawGamePage();
  drawn = "";
  draw();
}

/* ---------- cheats ---------- */

/* whoever found these addresses is almost never whoever uploaded the table
   here, and the person who did the work is the one worth naming */
async function loadCredit(exe) {
  try {
    return await invoke("credit", { exe });
  } catch {
    return {};
  }
}

function paintCredit(credit) {
  const line = $("table-credit");
  const link = $("credit-source");
  credit = credit || {};

  /* what the author wrote inside the table. folded away by default, since
     it can run to a screenful, but "turn this on in the main menu" has to be
     findable or the cheat gets blamed for it */
  const notes = (credit.notes || "").trim();
  $("table-notes").hidden = !notes;
  if ($("notes-body").textContent !== notes) {
    $("notes-body").textContent = notes;
    $("notes-body").hidden = true;
  }

  const author = (credit.author || "").trim();
  line.hidden = !author;
  if (!author) return;

  $("credit-author").textContent = author;
  const source = (credit.source || "").trim();
  link.hidden = !source;
  if (source) {
    link.dataset.open = source;
    link.title = source;
    try {
      link.textContent = new URL(source).hostname.replace(/^www\./, "");
    } catch {
      link.textContent = "where it came from";
    }
  }
}

async function refreshCheats() {
  const game = gameFor(open);
  if (!game || $("view-game").hidden) return;

  // nothing is on offer for a game we refuse to attach to, and the empty
  // state would otherwise reappear here after the page hid it
  if (game.guard) return;

  // no executable means nothing to look a table up by. say so rather than
  // leaving the placeholders pulsing for ever
  if (!game.exe) {
    refreshCheats.shape = null;
    $("cheat-groups").innerHTML = "";
    $("cheats-panel").hidden = true;
    $("table-picker").hidden = true;
    $("table-fit").hidden = true;
    $("no-table").hidden = false;
    paintNoTable();
    return;
  }

  /* this polls on a timer and is also called when you open a page, so two of
     them are in flight the moment you switch games. without this the slower
     one lands last and paints the game you just left */
  const asked = open;

  /* the cheats, the picker and the fit notice all used to be fetched one after
     another and drawn as each landed, so opening a game resized the page three
     times. they go out together and are drawn in one go */
  const fresh = paintTables.done !== game.exe || paintFit.done !== game.exe;
  let rows = [];
  let tables = null;
  let fit = null;
  let credit = null;
  let shut = null;
  let kept = [];
  try {
    [rows, tables, fit, credit, shut, kept] = await Promise.all([
      invoke("cheats", { exe: game.exe }),
      fresh ? loadTables(game.exe) : null,
      fresh ? loadFit(game.exe) : null,
      fresh ? loadCredit(game.exe) : null,
      fresh ? invoke("folded", { exe: game.exe }).catch(() => []) : null,
      invoke("last_loadout", { exe: game.exe }).catch(() => []),
    ]);
  } catch {
    return;
  }
  if (open !== asked) return;

  $("no-table").hidden = rows.length > 0;
  if (fresh) {
    folded = new Set(shut || []);
    paintTables(game.exe, tables || []);
    paintFit(game.exe, rows.length ? fit : null);
    paintCredit(rows.length ? credit : null);
  }
  if (!rows.length) paintNoTable();
  $("cheats-panel").hidden = rows.length === 0;
  // keeps its space either way, or the search box beside it jumps sideways
  $("remove-table").classList.toggle("away", rows.length === 0);
  $("game-export").hidden = rows.length === 0;
  $("cheat-count").textContent = rows.length ? `${many(rows.length, "cheat")}` : "";
  // said once up here rather than on all forty cards, where it was the only
  // thing most of them had to say and read as filler
  $("cheat-typing").hidden = !rows.some((row) => row.editable);

  /* the one press that brings the last set back, offered only when nothing is
     on and there was a last set. after a panic key this is the way home */
  const offer = $("rearm-last");
  const allOff = rows.length > 0 && rows.every((row) => !row.armed);
  offer.hidden = !(allOff && kept.length);
  if (!offer.hidden) {
    offer.textContent = "Turn on what you had (" + kept.length + ")";
  }

  const byCategory = new Map();
  const pinned = rows.filter((row) => row.starred);
  if (pinned.length) byCategory.set("Pinned", pinned);
  for (const row of rows) {
    if (row.starred) continue;
    if (!byCategory.has(row.category)) byCategory.set(row.category, []);
    byCategory.get(row.category).push(row);
  }

  /* this runs every second and a half. throwing the list away and building it
     again wiped hover, killed focus and made the whole panel blink, so the
     cards are only built when the table itself changes and patched otherwise.
     the value box is never touched here, or it would eat what you are typing */
  const shape =
    game.exe + rows.map((r) => r.id + r.category + (r.starred ? "*" : "")).join("|");
  if (shape !== refreshCheats.shape) {
    refreshCheats.shape = shape;
    refreshCheats.cards = new Map();
    // only when the table itself changes. this function runs on a timer and
    // the credit does not move between ticks

    const host = $("cheat-groups");
    host.innerHTML = "";
    for (const [category, items] of byCategory) {
      const group = document.createElement("div");
      const shut = folded.has(category);
      group.className = "group" + (shut ? " shut" : "");

      // a heading you can click has to be a button, or it is invisible to the
      // keyboard and to anything reading the page aloud
      const heading = document.createElement("button");
      heading.className = "group-head";
      heading.type = "button";
      heading.setAttribute("aria-expanded", String(!shut));

      const caret = document.createElement("span");
      caret.className = "caret";
      const label = document.createElement("h3");
      label.textContent = category;
      const count = document.createElement("span");
      count.className = "group-count";
      count.textContent = String(items.length);
      heading.append(caret, label, count);

      const grid = document.createElement("div");
      grid.className = "cheats";
      for (const item of items) {
        const card = cheatCard(item, game.exe);
        refreshCheats.cards.set(item.id, card);
        grid.appendChild(card);
      }

      heading.addEventListener("click", () => {
        const now = !group.classList.contains("shut");
        group.classList.toggle("shut", now);
        heading.setAttribute("aria-expanded", String(!now));
        if (now) folded.add(category);
        else folded.delete(category);
        invoke("fold", { exe: game.exe, category, shut: now }).catch(() => {});
      });

      group.append(heading, grid);
      host.appendChild(group);
    }
    applyCheatFilter();
    return;
  }

  for (const row of rows) patchCard(refreshCheats.cards.get(row.id), row);
}

/* more than one table can be installed for a game and they are shown as one
   list, so this is where you say which of them count. only appears when there
   is actually a choice to make */
async function loadTables(exe) {
  try {
    return await invoke("installed_tables", { exe });
  } catch {
    return [];
  }
}

function paintTables(exe, rows) {
  const box = $("table-picker");
  paintTables.done = exe;

  const host = $("table-list");
  host.innerHTML = "";
  // cleared first. going from two tables to one used to leave both rows in the
  // hidden box, and the empty state counts them to say how many you have
  box.hidden = rows.length < 2;
  if (box.hidden) return;
  for (const row of rows) {
    const label = document.createElement("label");
    label.className = "picker-table" + (row.using ? " on" : "");

    const tick = document.createElement("input");
    tick.type = "checkbox";
    tick.checked = row.using;
    tick.addEventListener("change", async () => {
      tick.disabled = true;
      try {
        await invoke("use_table", { exe, tag: row.tag, on: tick.checked });
        paintTables.done = null;
        paintFit.done = null;
        refreshCheats.shape = null;
        await refreshCheats();
        if (!$("no-table").hidden) paintNoTable();
      } catch (e) {
        toast(String(e), true);
        tick.checked = !tick.checked;
      } finally {
        tick.disabled = false;
      }
    });

    const name = document.createElement("span");
    name.className = "picker-table-name";
    name.textContent = row.name;
    const by = document.createElement("span");
    by.className = "picker-table-by";
    by.textContent = row.author
      ? `${many(row.cheats, "cheat")} by ${row.author}`
      : many(row.cheats, "cheat");

    label.append(tick, name, by);
    host.appendChild(label);
  }
}

/* whether the table's signatures are in this copy of the game, read off the
   exe with the game shut. asked once per game rather than on every tick of the
   timer refreshCheats sits on */
async function loadFit(exe) {
  try {
    return await invoke("table_fit", { exe });
  } catch {
    return null;
  }
}

function paintFit(exe, fit) {
  const box = $("table-fit");
  paintFit.done = exe;

  if (!fit || fit.silent) {
    box.hidden = true;
    return;
  }

  /* the game's code is encrypted on disk, which steam does to a lot of them.
     nothing can be read off the file, so saying the table does not fit would
     be an accusation we cannot support */
  if (fit.sealed) {
    box.hidden = false;
    box.className = "fit";
    $("fit-headline").textContent = "Cannot check this one against the game";
    $("fit-detail").textContent =
      "The executable is wrapped, so its code only exists once the game is " +
      "running. Whether the table fits can only be found out by trying it.";
    $("fit-stale").hidden = true;
    $("fit-stale").innerHTML = "";
    return;
  }

  /* three different things, and they used to share one sentence. a table can
     have every address right and still hold one cheat that crashes the game,
     which is not the same as the table being for another build */
  const missing = fit.missing > 0;
  const crashes = fit.stale.length > 0;
  box.hidden = false;
  box.className =
    "fit" + (missing || crashes ? " bad" : fit.ambiguous ? " iffy" : " good");

  if (missing) {
    $("fit-headline").textContent = "Some of this table is for a different build";
  } else if (crashes) {
    $("fit-headline").textContent =
      many(fit.stale.length, "cheat") + " here would crash the game";
  } else {
    $("fit-headline").textContent = `${many(fit.found, "signature")} found in your copy`;
  }

  const notes = [];
  if (missing) notes.push(`${fit.missing} of ${fit.total} addresses are not in it`);
  if (fit.ambiguous) notes.push(`${fit.ambiguous} match in more than one place`);
  // aobscan looks at the whole process, so the exe alone cannot answer it
  if (fit.unknown) notes.push(`${fit.unknown} search outside the executable`);
  if (!notes.length && crashes) {
    notes.push(
      `Everything else in it lines up, so the rest is fine to use`
    );
  }
  $("fit-detail").textContent = notes.length
    ? notes.join(". ") + "."
    : "Every address it looks for is where it expects.";

  const list = $("fit-stale");
  list.innerHTML = "";
  list.hidden = !fit.stale.length;
  for (const line of fit.stale) {
    const item = document.createElement("li");
    item.textContent = line;
    list.appendChild(item);
  }
}

function patchCard(card, item) {
  if (!card) return;

  const wanted = "cheat" + (item.armed ? " armed" : "") + (item.live ? " on" : "");
  if (card.className !== wanted) card.className = wanted;

  const why = card.querySelector(".cheat-why");
  const [text, tone] = whyFor(item);
  if (why.textContent !== text) why.textContent = text;
  const toned = "cheat-why" + (tone ? " " + tone : "");
  if (why.className !== toned) why.className = toned;

  const toggle = card.querySelector(".switch");
  const on = "switch" + (item.armed ? " on" : "");
  if (toggle.className !== on) {
    toggle.className = on;
    toggle.title = item.armed ? "Turn it off" : "Turn it on";
  }

  // left alone while it is listening for a press
  const chip = card.querySelector(".cheat-key");
  if (chip && !chip.classList.contains("listening")) {
    const text = item.key || "key";
    if (chip.textContent !== text) chip.textContent = text;
    chip.classList.toggle("empty", !item.key);
  }

  const live = card.querySelector(".cheat-live");
  if (live) {
    const now = item.current ? `now ${item.current}` : "";
    if (live.textContent !== now) live.textContent = now;
  }
}

/* the line under a cheat's name, and what colour it is */
function whyFor(item) {
  if (item.live) return ["On", "live"];
  if (item.armed && item.state === "broken") {
    return [item.reason || "not in this version of the game", "dead"];
  }
  if (item.armed && item.state === "wait") return [item.hint || item.reason, "wait"];
  /* worth more than the description: last time this one went on, the game
     went down within seconds. it clears itself after a quiet sitting */
  if (item.suspect) {
    return ["The game went down right after this was switched on last time", "dead"];
  }
  if (item.armed) return ["Waiting for the game to get there", "wait"];
  if (item.description) return [item.description, ""];
  if (item.does === "Script") return ["Finds the addresses the other cheats here need.", ""];
  return ["", ""];
}

/* placeholder cards while the real ones are on their way. cheaper than a
   spinner, and the page does not jump when the answer lands */
/* the placeholder while the answer is on its way. we already know whether the
   game has a table at all, so a game with none goes straight to the empty
   state. showing six fake cheat cards first and then taking them away was the
   page resizing twice, which is exactly what reads as a flicker */
function skeletons(hasTable) {
  const host = $("cheat-groups");
  host.innerHTML = "";
  $("cheat-none").hidden = true;
  $("cheat-typing").hidden = true;
  $("cheat-count").textContent = "";
  $("remove-table").classList.add("away");
  $("game-export").hidden = true;
  $("table-fit").hidden = true;

  if (!hasTable) {
    $("cheats-panel").hidden = true;
    $("no-table").hidden = false;
    paintNoTable();
    return;
  }
  $("cheats-panel").hidden = false;
  $("no-table").hidden = true;

  const grid = document.createElement("div");
  grid.className = "cheats";
  for (let n = 0; n < 6; n++) {
    const bone = document.createElement("div");
    bone.className = "cheat bone";
    const main = document.createElement("div");
    main.className = "cheat-main";
    const wide = document.createElement("span");
    wide.className = "bar wide";
    const thin = document.createElement("span");
    thin.className = "bar";
    main.append(wide, thin);
    bone.appendChild(main);
    grid.appendChild(bone);
  }
  host.appendChild(grid);
}

/* typing filters what is already on screen. debounced, or every keystroke
   walks the whole list */
let filterTimer = null;
function filterCheats() {
  clearTimeout(filterTimer);
  filterTimer = setTimeout(applyCheatFilter, 180);
}

function applyCheatFilter() {
  const needle = $("cheat-filter").value.trim().toLowerCase();
  let shown = 0;

  for (const card of document.querySelectorAll("#cheat-groups .cheat")) {
    const name = (card.querySelector(".cheat-name") || {}).textContent || "";
    const hit = !needle || name.toLowerCase().includes(needle);
    card.hidden = !hit;
    if (hit) shown++;
  }
  // a category with nothing left in it should not keep its heading
  for (const group of document.querySelectorAll("#cheat-groups .group")) {
    group.hidden = ![...group.querySelectorAll(".cheat")].some((c) => !c.hidden);
  }
  /* a folded group still has to give up its matches while you are searching,
     or the search says nothing found when it did find something */
  $("cheat-groups").classList.toggle("searching", !!needle);
  $("cheat-none").hidden = shown > 0 || !needle;
}

$("cheat-filter").addEventListener("input", filterCheats);

$("notes-toggle").addEventListener("click", () => {
  $("notes-body").hidden = !$("notes-body").hidden;
});

$("rearm-last").addEventListener("click", async () => {
  const game = gameFor(open);
  if (!game) return;
  try {
    const n = await invoke("rearm", { exe: game.exe });
    toast(many(n, "cheat") + " back on");
    await refreshCheats();
  } catch (e) {
    toast(String(e), true);
  }
});

/* you can switch a cheat on whenever. whether it is actually doing anything is
   a separate thing the card says underneath, since the pointer most of them
   hang off is null until you load a save */
/* a name out of a cheat engine table is one long shout with underscores where
   the spaces would be. left alone it breaks in the middle of a word, so it
   gets a break offered after each underscore instead.

   `wbr` rather than a zero width space because it adds nothing to textContent,
   which is what the search box reads */
function spellOut(into, text) {
  const parts = text.split("_");
  parts.forEach((part, n) => {
    const last = n === parts.length - 1;
    // the underscore stays on the line it ends, the way a hyphen would
    into.appendChild(document.createTextNode(last ? part : part + "_"));
    if (!last) into.appendChild(document.createElement("wbr"));
  });
}

function cheatCard(item, exe) {
  const card = document.createElement("div");
  card.className = "cheat" + (item.armed ? " armed" : "") + (item.live ? " on" : "");

  const main = document.createElement("div");
  main.className = "cheat-main";

  const name = document.createElement("div");
  name.className = "cheat-name";
  spellOut(name, item.name);

  const tag = document.createElement("span");
  tag.className = "cheat-does";
  tag.textContent = item.does || "";
  name.appendChild(tag);

  name.appendChild(keyChip(item, exe));
  name.appendChild(pinButton(item, exe));

  // only filled in when more than one table is folded into the list
  if (item.from) {
    const from = document.createElement("span");
    from.className = "cheat-from";
    from.textContent = item.from;
    from.title = "from " + item.from;
    name.appendChild(from);
  }

  const [line, tone] = whyFor(item);
  const why = document.createElement("div");
  why.className = "cheat-why" + (tone ? " " + tone : "");
  why.textContent = line;

  main.append(name, why);
  if (item.editable) main.appendChild(valueBox(item, exe));

  const toggle = document.createElement("button");
  toggle.className = "switch" + (item.armed ? " on" : "");
  toggle.title = item.armed ? "Turn it off" : "Turn it on";
  toggle.addEventListener("click", async () => {
    try {
      await invoke("set_cheat", { exe, id: item.id, on: !item.armed });
      item.armed = !item.armed;
      patchCard(card, item);
      await refreshCheats();
    } catch (e) {
      toast(String(e), true);
    }
  });

  card.append(main, toggle);
  return card;
}

/* a merged table runs to fifty rows and most people use three. the star
   gathers those three on a shelf of their own at the top */
function pinButton(item, exe) {
  const pin = document.createElement("button");
  pin.className = "cheat-pin" + (item.starred ? " on" : "");
  pin.innerHTML =
    '<svg viewBox="0 0 20 20"><path d="M10 2.8l2.3 4.6 5.1.7-3.7 3.6.9 5.1L10 14.4l-4.6 2.4.9-5.1L2.6 8.1l5.1-.7z"/></svg>';
  pin.title = item.starred ? "Take it off the pinned shelf" : "Pin it to the top";
  pin.addEventListener("click", async (e) => {
    e.stopPropagation();
    try {
      await invoke("star_cheat", { exe, id: item.id, on: !item.starred });
      item.starred = !item.starred;
      await refreshCheats();
    } catch (err) {
      toast(String(err), true);
    }
  });
  return pin;
}

/* the key that flips this cheat while you play. tables bring their own, the
   chip rebinds them, right click takes one away */
function keyChip(item, exe) {
  const chip = document.createElement("button");
  chip.className = "cheat-key" + (item.key ? "" : " empty");
  chip.textContent = item.key || "key";
  chip.title = item.key
    ? "Works in the game. Click to change it, right click to remove it"
    : "Bind a key you can press in the game";
  chip.addEventListener("click", (e) => {
    e.stopPropagation();
    captureKey(chip, item, exe);
  });
  chip.addEventListener("contextmenu", async (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (!item.key) return;
    try {
      const spelled = await invoke("bind_key", { exe, id: item.id, key: "" });
      item.key = spelled;
      chip.textContent = spelled || "key";
      chip.classList.toggle("empty", !spelled);
    } catch (err) {
      toast(String(err), true);
    }
  });
  return chip;
}

function captureKey(chip, item, exe) {
  if (chip.classList.contains("listening")) return;
  chip.classList.add("listening");
  const was = chip.textContent;
  chip.textContent = "press a key";

  const done = () => {
    window.removeEventListener("keydown", take, true);
    window.removeEventListener("blur", quit);
    chip.classList.remove("listening");
  };
  const quit = () => {
    done();
    chip.textContent = was;
  };
  const take = async (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") return quit();
    const combo = comboFrom(e);
    if (!combo) return; // a modifier on its own, keep listening
    try {
      const spelled = await invoke("bind_key", { exe, id: item.id, key: combo });
      done();
      item.key = spelled;
      chip.textContent = spelled || "key";
      chip.classList.toggle("empty", !spelled);
    } catch (err) {
      toast(String(err), true);
      quit();
    }
  };
  window.addEventListener("keydown", take, true);
  window.addEventListener("blur", quit);
}

/* spelled the way the backend reads it. built off e.code so the letter is the
   key itself and not what shift turns it into */
function comboFrom(e) {
  const name = keyNameOf(e);
  if (!name) return null;
  const parts = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  parts.push(name);
  return parts.join("+");
}

function keyNameOf(e) {
  if (/^F\d+$/.test(e.key)) return e.key;
  const code = e.code || "";
  let m;
  if ((m = code.match(/^Key([A-Z])$/))) return m[1];
  if ((m = code.match(/^Digit(\d)$/))) return m[1];
  if ((m = code.match(/^Numpad(\d)$/))) return "Num" + m[1];
  const names = {
    Space: "Space",
    Tab: "Tab",
    Enter: "Enter",
    Backspace: "Backspace",
    Insert: "Insert",
    Delete: "Delete",
    Home: "Home",
    End: "End",
    PageUp: "PageUp",
    PageDown: "PageDown",
    ArrowUp: "Up",
    ArrowDown: "Down",
    ArrowLeft: "Left",
    ArrowRight: "Right",
    Pause: "Pause",
    Backquote: "`",
    Minus: "-",
    Equal: "=",
    BracketLeft: "[",
    BracketRight: "]",
    Backslash: "\\",
    Semicolon: ";",
    Quote: "'",
    Comma: ",",
    Period: ".",
    Slash: "/",
    NumpadMultiply: "Num*",
    NumpadAdd: "NumPlus",
    NumpadSubtract: "Num-",
    NumpadDecimal: "Num.",
    NumpadDivide: "Num/",
  };
  return names[code] || null;
}

/* a dropdown if the table author listed the options, a plain box otherwise.
   plenty of cheats are a number, not a switch: carry weight, game speed, how
   much gold. freezing those at 999999 breaks the save */
function valueBox(item, exe) {
  const wrap = document.createElement("div");
  wrap.className = "cheat-value";

  const send = async (text, revert) => {
    try {
      await invoke("set_cheat_value", { exe, id: item.id, value: text });
      item.value = text;
    } catch (e) {
      toast(String(e), true);
      revert();
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
    pick.addEventListener("change", () => send(pick.value, () => reset(pick)));
    wrap.appendChild(pick);
    // built and given its value first, so the face reads right straight away
    setTimeout(() => enhanceSelect(pick), 0);

    function reset(box) {
      box.value = item.value;
      if (box.picker) box.picker.sync();
    }
  } else {
    const input = document.createElement("input");
    input.type = "text";
    input.spellcheck = false;
    input.value = item.value;
    // "i32" is the table's word for it. nobody typing in a health box wants it
    input.placeholder = item.hex ? "hex" : item.kind.startsWith("f") ? "decimal" : "number";
    if (item.kind) input.title = `the table stores this as ${item.kind}`;
    input.addEventListener("change", () => send(input.value, () => (input.value = item.value)));
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") input.blur();
    });
    wrap.appendChild(input);
  }

  const live = document.createElement("span");
  live.className = "cheat-live";
  live.dataset.liveFor = item.id;
  live.textContent = item.current ? `now ${item.current}` : "";
  wrap.appendChild(live);

  if (!item.holds) {
    const once = document.createElement("span");
    once.className = "cheat-once";
    once.textContent = "written once";
    once.title = "the game is free to change it back";
    wrap.appendChild(once);
  }

  return wrap;
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
    host.innerHTML = `<div class="placeholder">Too many to show. Change the value in the game, then narrow it down.</div>`;
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
    // the type the scan actually ran under, not whatever the dropdown says
    // now. changing it after a scan used to write the wrong width
    const kind = $("scan-type").value;
    write.addEventListener("click", async () => {
      try {
        await invoke("write_value", {
          address: hit.address,
          kind,
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
  if (!attached) return toast("Attach to a game first", true);
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

let narrowing = false;

async function narrow(filter) {
  if (!scanning) return toast("Run the first scan before narrowing it down", true);
  // a double click was quietly running two rounds and throwing away addresses
  // the user never meant to discard
  if (narrowing) return;

  let value = null;
  if (filter === "exact") {
    value = $("scan-value").value.trim();
    if (!value) return toast("Type what the game shows now, then press it again", true);
  }

  const was = $("scan-status").innerHTML;
  narrowing = true;
  chipsEnabled(false);
  $("scan-status").textContent = "Scanning";
  try {
    setScanStatus(await invoke("scan_next", { filter, value }));
  } catch (e) {
    toast(String(e), true);
    $("scan-status").innerHTML = was;
  } finally {
    narrowing = false;
    chipsEnabled(true);
  }
}

function chipsEnabled(on) {
  for (const chip of document.querySelectorAll(".chip")) chip.disabled = !on;
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
  let list = [];
  try {
    list = await invoke("list_processes");
  } catch (e) {
    return toast(String(e), true);
  }
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

function currentView() {
  return ["library", "game", "finder", "settings", "about"].find(
    (id) => !$(`view-${id}`).hidden
  ) || "library";
}

function showView(name) {
  if (name === "game" && !open) name = "library";

  for (const id of ["library", "game", "finder", "settings", "about"]) {
    $(`view-${id}`).hidden = id !== name;
  }
  for (const item of document.querySelectorAll(".nav-item")) {
    const target = item.dataset.view;
    item.classList.toggle("active", target === name || (name === "game" && target === "library"));
  }

  // coming back to the library is exactly when somebody has just finished
  // playing, so this is the moment to ask
  if (name === "library") drawQuestion();

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
    leaveGame();
    showView(item.dataset.view);
    drawn = "";
    draw();
  });
});

/* nothing outside the game page belongs to a game. leaving it set meant a .CT
   imported from Settings was filed under the last game you looked at, and the
   five second tick kept asking the service about it from every other view */
function leaveGame() {
  open = null;
  drawGamePage.showing = null;
  sharedFor = null;
}
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
  leaveGame();
  showView("library");
  drawn = "";
  draw();
});

$("game-play").addEventListener("click", async () => {
  const game = gameFor(open);
  if (!game) return;
  try {
    if (game.running) {
      await invoke("focus_game", { exe: game.exe });
    } else {
      await invoke("launch_game", { key: game.key });
      toast(`Starting ${game.name}`);
    }
  } catch (e) {
    toast(String(e), true);
  }
});

$("game-attach").addEventListener("click", () => {
  const game = gameFor(open);
  if (!game) return;
  if (attached && sameExe(attached.process, game.exe)) doDetach();
  else if (game.exe) doAttach(game.exe);
  else toast("Could not work out which file to attach to", true);
});

$("game-fav").addEventListener("click", () => {
  if (open) saveConfig({ favourites: toggleIn(config.favourites, open) });
});
$("clear-favs").addEventListener("click", () => saveConfig({ favourites: [] }));
$("auto-attach").addEventListener("click", () =>
  saveConfig({ auto_attach: config.auto_attach === false })
);

$("auto-update").addEventListener("click", () =>
  saveConfig({ auto_update: config.auto_update === false })
);

$("chirp-on").addEventListener("click", () =>
  saveConfig({ chirp: config.chirp === false })
);
$("panic-off").addEventListener("click", () => saveConfig({ panic: "" }));

/* same catch-a-real-press box as the overlay shortcut */
let catchingPanic = false;

$("panic-key").addEventListener("click", () => {
  catchingPanic = true;
  $("panic-key").classList.add("catching");
  $("panic-key").textContent = "Press a key";
  $("panic-key").focus();
});

$("panic-key").addEventListener("blur", () => {
  if (!catchingPanic) return;
  catchingPanic = false;
  $("panic-key").classList.remove("catching");
  applyTheme();
});

$("panic-key").addEventListener("keydown", async (e) => {
  if (!catchingPanic) return;
  e.preventDefault();
  if (e.key === "Escape") return $("panic-key").blur();
  const combo = comboFrom(e);
  if (!combo) return;
  catchingPanic = false;
  $("panic-key").classList.remove("catching");
  await saveConfig({ panic: combo });
});

$("community-on").addEventListener("click", async () => {
  await saveConfig({ community: config.community === false });
  if (open) await loadShared(true);
});
$("update-now").addEventListener("click", () => checkForTables(true));

$("copy-report").addEventListener("click", async () => {
  try {
    const text = await invoke("diagnostics");
    await navigator.clipboard.writeText(text);
    toast("Copied. Paste it into the issue");
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

/* dragging a file in is the quicker way, but nobody discovers it, so there is
   a button for it too */
async function pickTable() {
  const game = gameFor(open);
  try {
    const note = await invoke("pick_table", { exe: game ? game.exe : null });
    toast(note);
    await loadGames(false);
    await refreshCheats();
  } catch (e) {
    if (String(e)) toast(String(e), true);
  }
}

$("game-find-table").addEventListener("click", searchForTable);
$("no-table-find").addEventListener("click", searchForTable);
$("game-import").addEventListener("click", pickTable);
$("no-table-import").addEventListener("click", pickTable);
$("import-table").addEventListener("click", pickTable);

document.querySelectorAll("[data-open]").forEach((button) => {
  button.addEventListener("click", () =>
    invoke("open_url", { url: button.dataset.open }).catch((e) => toast(String(e), true))
  );
});

// this one gets its address when a table loads, so it is not there to be found
// in the sweep above
$("credit-source").addEventListener("click", (e) => {
  e.preventDefault();
  const url = $("credit-source").dataset.open;
  if (url) invoke("open_url", { url }).catch((err) => toast(String(err), true));
});

/* ---------- the cheat dock ---------- */

function drawDock() {
  const shut = config.shared_open === false;
  $("shared").hidden = shut;
  $("dock-open").hidden = !shut;
  // the empty state points at the dock, so it has to know whether it is there
  if (!$("no-table").hidden) paintNoTable();
}

$("dock-close").addEventListener("click", () => saveConfig({ shared_open: false }));
$("dock-open").addEventListener("click", () => saveConfig({ shared_open: true }));

$("no-table-pick").addEventListener("click", () => saveConfig({ shared_open: true }));

$("game-folder").addEventListener("click", () => {
  const game = gameFor(open);
  if (!game) return;
  invoke("open_folder", { dir: game.dir }).catch((e) => toast(String(e), true));
});

$("add-game").addEventListener("click", async () => {
  try {
    const name = await invoke("add_game");
    // nothing back means the picker was closed, which needs no toast
    if (!name) return;
    toast(name + " added");
    await loadGames(true);
  } catch (e) {
    toast(String(e), true);
  }
});


async function dropGame(path) {
  try {
    const name = await invoke("add_game", { path });
    toast(name + " added");
    await loadGames(true);
  } catch (e) {
    toast(String(e), true);
  }
}

$("game-export").addEventListener("click", async () => {
  const game = gameFor(open);
  if (!game) return;
  try {
    const said = await invoke("export_table", { exe: game.exe });
    // nothing back means the picker was closed
    if (said) toast(said);
  } catch (e) {
    toast(String(e), true);
  }
});

$("game-hide").addEventListener("click", async () => {
  const game = gameFor(open);
  if (!game) return;
  await saveConfig({ hidden: [...(config.hidden || []), game.key] });
  toast(game.name + " hidden. Settings brings it back");
  open = null;
  showView("library");
});

$("unhide-all").addEventListener("click", () => saveConfig({ hidden: [] }));

$("elevate-no").addEventListener("click", () => ($("elevate-sheet").hidden = true));
$("elevate-go").addEventListener("click", async () => {
  try {
    await invoke("relaunch_admin");
  } catch (e) {
    $("elevate-sheet").hidden = true;
    toast(String(e), true);
  }
});

$("game-remove").addEventListener("click", async () => {
  const game = gameFor(open);
  if (!game) return;
  try {
    await invoke("remove_added", { dir: game.dir });
    toast(game.name + " removed");
    open = null;
    showView("library");
    await loadGames(true);
  } catch (e) {
    toast(String(e), true);
  }
});

/* dropping a .CT anywhere on the window imports it. tauri reports the drop, the
   webview never sees the file */
if (window.__TAURI__.event) {
  const { listen } = window.__TAURI__.event;
  listen("tables-updated", async (e) => {
    toast(String(e.payload));
    await loadGames(false);
    await refreshCheats();
    await countTables();
    await loadShared(true);
  });
  listen("tauri://drag-enter", () => document.body.classList.add("dropping"));
  listen("tauri://drag-leave", () => document.body.classList.remove("dropping"));
  listen("tauri://drag-drop", (e) => {
    document.body.classList.remove("dropping");
    const paths = (e.payload && e.payload.paths) || [];
    const table = paths.find((p) => p.toLowerCase().endsWith(".ct"));
    if (table) return importTable(table);
    const carried = paths.find((p) => p.toLowerCase().endsWith(".freeplay"));
    if (carried) return dropProfile(carried);
    // an exe is a game somebody wants in the library
    const program = paths.find((p) => p.toLowerCase().endsWith(".exe"));
    if (program) return dropGame(program);
    toast("Drop a Cheat Engine .CT file, a game's exe, or a Freeplay profile", true);
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
$("win-max").addEventListener("click", async () => {
  await appWindow.toggleMaximize();
  drawWindowState();
});
$("win-close").addEventListener("click", () => appWindow.close());

/* the button showed a maximise square whether or not the window was already
   maximised, so it looked like nothing happened */
async function drawWindowState() {
  let big = false;
  try {
    big = await appWindow.isMaximized();
  } catch {
    return;
  }
  document.body.classList.toggle("maximised", big);
  $("win-max").title = big ? "Restore" : "Maximise";
}

/* dragging the title bar to the top edge maximises without going through the
   button, so watch for it as well */
window.addEventListener("resize", drawWindowState);

$("filter").addEventListener("input", draw);
$("refresh").addEventListener("click", () => {
  // rescanning should give the art another go too
  art.clear();
  loadGames(true);
});
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
/* the recovery words are the only copy of an account that has already been
   written to disk by the time they are on screen, so this one sheet does not
   close by being dismissed */
const showingPhrase = () => !$("name-sheet").hidden && !$("name-phrase").hidden;

document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  $("sheet").hidden = true;
  $("export-sheet").hidden = true;
  $("import-sheet").hidden = true;
  if (!showingPhrase()) $("name-sheet").hidden = true;
});

/* clicking the dimmed area behind a sheet closes it, same as escape */
["name-sheet", "export-sheet", "import-sheet"].forEach((id) => {
  $(id).addEventListener("click", (e) => {
    if (e.target !== $(id)) return;
    if (id === "name-sheet" && showingPhrase()) return;
    $(id).hidden = true;
  });
});


/* ---------- shared tables ---------- */

let sharedSorts = [];

async function loadSortOptions() {
  if (sharedSorts.length) return;
  try {
    sharedSorts = await invoke("sort_options");
  } catch {
    sharedSorts = [{ key: "best", label: "Best match" }];
  }
  const box = $("shared-sort");
  box.innerHTML = "";
  for (const option of sharedSorts) {
    const el = document.createElement("option");
    el.value = option.key;
    el.textContent = option.label;
    box.appendChild(el);
  }
  enhanceSelect(box);
}

function when(seconds) {
  if (!seconds) return "";
  const days = Math.floor((Date.now() / 1000 - seconds) / 86400);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 30) return many(days, "day") + " ago";
  if (days < 365) return many(Math.floor(days / 30), "month") + " ago";
  return many(Math.floor(days / 365), "year") + " ago";
}

async function loadShared(force = false) {
  const game = gameFor(open);
  if (!game || !game.exe) return;

  // a search owns the list while there is one. this also runs on a timer, and
  // without this it wipes the results out from under whoever is reading them
  if (searchFor.length >= 2) return;

  /* turned off means turned off, so this does not fire and then report the
     refusal as though something had gone wrong */
  if (config.community === false) {
    sharedFor = null;
    sharedRows = [];
    ownRows = null;
    $("shared-list").innerHTML = "";
    $("dock-open-count").textContent = "";
    $("shared-empty").hidden = false;
    $("shared-empty").textContent =
      "Shared tables are turned off, so Freeplay is not asking the service about this game.";
    paintNoTable();
    return;
  }

  if (!force && sharedFor === game.exe) return;
  sharedFor = game.exe;

  await loadSortOptions();
  /* whatever is up stays up while the next answer is on its way. emptying it
     first is what made the panel collapse and spring back, and holding it open
     at a fixed height just moved the problem to a panel full of nothing */
  const host = $("shared-list");
  host.classList.add("waiting");

  let rows = [];
  try {
    rows = await invoke("shared_tables", {
      exe: game.exe,
      sort: $("shared-sort").value || "best",
    });
  } catch (e) {
    if (sharedFor !== game.exe) return;
    host.classList.remove("waiting");
    // let go of the game, or one bad reply means this never asks again
    sharedFor = null;
    host.innerHTML = "";
    $("shared-empty").hidden = false;
    $("shared-empty").textContent = String(e);
    return;
  }

  if (sharedFor !== game.exe) return;

  // one change, when there is something to change to
  host.classList.remove("waiting");
  sharedRows = rows;
  host.innerHTML = "";
  $("shared-empty").hidden = rows.length > 0;
  $("shared-empty").textContent =
    "Nobody has shared a table for this game yet. If yours works, share it and you will be the first.";
  $("dock-open-count").textContent = rows.length ? String(rows.length) : "";

  for (const row of rows) host.appendChild(sharedRow(row));
  applySharedFilter();
  paintNoTable();
}

let sharedRows = [];
/* the empty state used to say there was nothing for this game while seven
   shared tables sat in the dock next to it. what it says now depends on
   whether any of them turned up, and both halves of the page can finish in
   either order, so each one calls this when it does */
function paintNoTable() {
  const rows = ownRows || sharedRows;
  const some = rows.length > 0;
  const shut = $("shared").hidden;

  /* you have tables, you just switched them all off. saying "nobody has
     shared one" here sent people looking for a table they already had */
  const off = [...document.querySelectorAll("#table-list input")];
  if (off.length && off.every((t) => !t.checked)) {
    $("no-table-title").textContent = `None of your ${off.length} tables are switched on`;
    $("no-table-lead").textContent =
      "Tick one above to get its cheats back.";
    $("no-table-body").textContent =
      "They are still on this machine. Nothing was deleted.";
    $("no-table-pick").hidden = true;
    $("no-table-import").className = "ghost";
    return;
  }

  $("no-table-title").textContent = some
    ? `${many(rows.length, "shared table")} for this game`
    : "No cheat table for this game";
  $("no-table-lead").textContent = some
    ? shut
      ? "Somebody has already worked this game out. Open the shared tables and pick one."
      : "Somebody has already worked this game out. Pick one from the list on the right."
    : "A table says where the game keeps its numbers. There are three ways to get one.";
  $("no-table-body").textContent = some
    ? "None of them fit? Search for another, or import a Cheat Engine .CT file of your own. You can also drop one anywhere on this window."
    : "Import a Cheat Engine .CT file and Freeplay converts it, scripts included. You can also drop one anywhere on this window. Or find the value yourself while the game runs.";

  // pointless when the list is already sitting there
  $("no-table-pick").hidden = !some || !shut;
  $("no-table-import").className = some ? "ghost" : "primary";
}

let sharedTimer = null;
let searchTimer = null;
let searchFor = "";
// this game's own tables, held while a search is covering them up
let ownRows = null;

function fillDock(rows) {
  const host = $("shared-list");
  host.classList.remove("waiting");
  host.innerHTML = "";
  sharedRows = rows;
  for (const row of rows) host.appendChild(sharedRow(row));
}

/* the way out when we picked the wrong binary for a game. matching a table to
   a game is an exact compare on the executable name, and no rule gets that
   right every time, so this ignores executables and looks by name instead */
async function searchEveryGame() {
  const wanted = $("table-search").value.trim();
  const was = searchFor;
  searchFor = wanted;

  if (wanted.length < 2) {
    $("search-note").hidden = true;
    /* put back what the search covered rather than asking the service for it
       again. going back to the network left somebody else's game sitting in
       the panel for half a second after the box was already empty */
    if (was.length >= 2 && ownRows) {
      fillDock(ownRows);
      $("shared-empty").hidden = ownRows.length > 0;
      $("shared-empty").textContent =
        "Nobody has shared a table for this game yet. If yours works, share it and you will be the first.";
      ownRows = null;
      applySharedFilter();
      paintNoTable();
      return;
    }
    await loadShared(true);
    return;
  }

  // the keystroke that starts a search is the one that has to remember what
  // was on screen before it
  if (was.length < 2) ownRows = sharedRows;

  const host = $("shared-list");
  host.classList.add("waiting");
  $("search-note").hidden = false;
  $("search-note").textContent = "Looking through every table";

  let rows = [];
  try {
    rows = await invoke("search_tables", { query: wanted });
  } catch (e) {
    if (searchFor !== wanted) return;
    host.classList.remove("waiting");
    // leaving this game's tables sitting under the error reads as though the
    // search returned them
    host.innerHTML = "";
    sharedRows = [];
    $("shared-empty").hidden = true;
    $("search-note").textContent = String(e);
    return;
  }
  // typed on while that was in the air
  if (searchFor !== wanted) return;

  $("shared-empty").hidden = true;
  fillDock(rows.map((row) => ({ ...row, found_by_search: true })));

  const game = gameFor(open);
  $("search-note").textContent = rows.length
    ? `${many(rows.length, "table")} matching "${wanted}". Using one points it at ` +
      `${(game && game.exe) || "this game"}.`
    : `Nothing matches "${wanted}".`;
}

function filterShared() {
  clearTimeout(sharedTimer);
  sharedTimer = setTimeout(applySharedFilter, 180);
}

function applySharedFilter() {
  const needle = $("shared-search").value.trim().toLowerCase();
  const cards = [...document.querySelectorAll("#shared-list .shared-row")];
  let shown = 0;

  cards.forEach((card, at) => {
    const row = sharedRows[at];
    /* the author counts as much as the uploader. searching for the person
       who worked a game out should find their tables, and for anything
       converted they are not the one who uploaded it */
    const hit =
      !needle ||
      (row && ((row.by || "").toLowerCase().includes(needle) ||
               (row.author || "").toLowerCase().includes(needle) ||
               (row.built_for || "").toLowerCase().includes(needle)));
    card.hidden = !hit;
    if (hit) shown++;
  });

  if (needle && !shown && cards.length) {
    $("shared-empty").hidden = false;
    $("shared-empty").textContent = "Nobody matching that has shared one.";
  } else if (cards.length) {
    $("shared-empty").hidden = true;
  }
}

$("shared-search").addEventListener("input", filterShared);
$("table-search").addEventListener("input", () => {
  clearTimeout(searchTimer);
  // waiting is for not firing a request per keystroke. emptying the box sends
  // nothing, so there is nothing to wait for and the panel snaps straight back
  const wait = $("table-search").value.trim().length < 2 ? 0 : 260;
  searchTimer = setTimeout(searchEveryGame, wait);
});

// whether this game already has a table, so "add to mine" has something to
// add to. the picker only exists once there are two, so read the game instead
const haveATable = () => {
  const game = gameFor(open);
  return !!(game && game.has_table);
};

function sharedRow(row) {
  const card = document.createElement("div");
  card.className =
    "shared-row" + (row.installed ? " have" : "") + (row.recommended ? " pick" : "");

  const main = document.createElement("div");
  main.className = "shared-main";

  if (row.recommended) {
    const star = document.createElement("div");
    star.className = "pick-flag";
    star.textContent = "Recommended";
    main.appendChild(star);
  }

  // the copy you grabbed is not the copy up there any more
  if (row.stale) {
    const fresh = document.createElement("div");
    fresh.className = "stale-flag";
    fresh.textContent = "Updated since you got it";
    main.appendChild(fresh);
  }

  /* the name the table calls its game, not the uploader. plenty of games ship
     as game.exe and everything converted from one place goes up under one
     account, so the uploader is the one thing that does not tell them apart */
  const title = document.createElement("div");
  title.className = "shared-name";
  title.textContent = row.game || (row.by ? row.by : "shared anonymously");

  /* two different people. the author worked the addresses out, the uploader
     put the table here, and for anything converted they are never the same.
     the author is the one worth reading, so it leads */
  const byline = document.createElement("div");
  byline.className = "shared-by";
  const uploader = row.by ? "uploaded by " + row.by : "uploaded anonymously";
  byline.textContent = row.author ? `by ${row.author}, ${uploader}` : uploader;

  /* your own upload coming back at you looks like a stranger's otherwise */
  if (row.by && me && row.by.toLowerCase() === me.toLowerCase()) {
    const yours = document.createElement("span");
    yours.className = "mine";
    yours.textContent = " (you)";
    byline.appendChild(yours);
  }
  if (row.by) {
    /* this says the name is registered to a key and nobody else can publish
       under it. it says nothing at all about whether the table works, and the
       old wording, "signed", read like it did. it does not say "author"
       either, because whoever uploaded a converted table is usually not the
       person who found the addresses in it */
    const tick = document.createElement("span");
    tick.className = "verified";
    tick.textContent = "name verified";
    tick.title =
      "This name is registered to a key, so only its owner can publish under it. " +
      "It is not a claim that the table works, or that they wrote it.";
    byline.appendChild(tick);
  }

  /* the version is what decides whether a table does anything at all, so it
     goes above the counts rather than at the end of a run of full stops */
  const fit = document.createElement("div");
  fit.className = "shared-fit " + row.fit;
  const mark = document.createElement("b");
  mark.textContent = { same: "✓", older: "⚠", newer: "⚠" }[row.fit] || "?";
  const said = document.createElement("span");
  said.textContent = row.fit_note;
  fit.append(mark, said);

  const facts = document.createElement("div");
  facts.className = "shared-facts";
  const bits = [many(row.cheats, "cheat")];
  if (row.downloads) bits.push(many(row.downloads, "download"));
  if (row.up || row.down) bits.push(row.up + " up, " + row.down + " down");
  const added = when(row.added);
  if (added) bits.push(added);
  facts.textContent = bits.join("  .  ");
  if (row.standing) card.title = row.standing;

  main.append(title, byline, fit, facts);

  const actions = document.createElement("div");
  actions.className = "row-actions";

  /* taking a table used to quietly add it to whatever was already there, so
     picking a second one gave you both lists welded together whether or not
     that is what you wanted. use replaces, add is the one that merges, and
     add only appears when there is something to add to */
  const grab = async (button, replace) => {
    const was = button.textContent;
    button.disabled = true;
    button.textContent = "Getting it";
    try {
      /* a table found by searching is filed under some other game's binary,
         so it has to be pointed at this one or it lands on disk and never
         shows up anywhere */
      const game = gameFor(open);
      const borrowed = row.found_by_search && game && game.exe ? game.exe : null;
      toast(
        await invoke("install_shared", {
          id: row.id,
          forExe: borrowed,
          replace,
        })
      );
      if (borrowed) {
        $("table-search").value = "";
        $("search-note").hidden = true;
      }
      paintFit.done = null;
      paintTables.done = null;
      await loadGames(false);
      await refreshCheats();
      await loadShared(true);
    } catch (e) {
      toast(String(e), true);
      button.disabled = false;
      button.textContent = was;
    }
  };

  const button = document.createElement("button");
  button.className = row.installed ? "ghost" : "primary";
  button.textContent = row.installed ? "Installed" : row.stale ? "Update" : "Use table";
  button.title = row.installed
    ? "This one is already on this machine"
    : row.stale
      ? "The shared copy changed since you grabbed it. This replaces yours with the new one"
      : "Use this one on its own, replacing what you have";
  button.disabled = row.installed;
  button.addEventListener("click", () => grab(button, true));
  actions.appendChild(button);

  if (!row.installed && haveATable()) {
    const also = document.createElement("button");
    also.className = "ghost";
    also.textContent = "Add to mine";
    also.title = "Keep what you have and show this one's cheats as well";
    also.addEventListener("click", () => grab(also, false));
    actions.appendChild(also);
  }

  if (row.installed) {
    const drop = document.createElement("button");
    drop.className = "ghost";
    drop.textContent = "Remove";
    drop.title = "Delete it from this machine";
    drop.addEventListener("click", () => removeTable());
    actions.appendChild(drop);
  }

  card.append(main, actions);
  return card;
}

/* downloading a table was one click and getting rid of it was editing a folder
   by hand, which is not a thing anybody should have to know about */
async function removeTable() {
  const game = gameFor(open);
  if (!game || !game.exe) return;
  try {
    toast(await invoke("remove_table", { exe: game.exe }));
    await loadGames(false);
    await refreshCheats();
    await loadShared(true);
  } catch (e) {
    toast(String(e), true);
  }
}

$("remove-table").addEventListener("click", removeTable);

/* the question used to appear only while the game was attached, which is the
   one moment nobody is looking at this window. it waits now, and it is only
   ever about a table that was actually running: which one that was is noted
   when the game starts, so switching tables afterwards cannot confuse it */
async function drawQuestion() {
  let question = null;
  try {
    question = await invoke("pending_question");
  } catch {
    return;
  }

  $("ask").hidden = !question;
  if (!question) return;

  $("ask").dataset.id = question.id;
  $("ask-title").textContent = `Did the table for ${question.game} work?`;

  const by = question.by ? `${question.by}'s table` : "The table you downloaded";
  const on = question.cheats
    ? `, ${many(question.cheats, "cheat")} switched on`
    : "";
  $("ask-detail").textContent = `${by}. You played for ${question.played}${on}.`;
}

async function answerQuestion(up) {
  const id = Number($("ask").dataset.id);
  $("ask").hidden = true;
  try {
    toast(await invoke("answer_question", { id, up }));
  } catch (e) {
    toast(String(e), true);
  }
  await drawQuestion();
  if (open) await loadShared(true);
}

$("ask-yes").addEventListener("click", () => answerQuestion(true));
$("ask-no").addEventListener("click", () => answerQuestion(false));

/* nobody has to answer to use the app. skipping keeps the question and just
   stops us asking for a couple of days */
$("ask-skip").addEventListener("click", async () => {
  $("ask").hidden = true;
  try {
    await invoke("skip_question");
  } catch (e) {
    toast(String(e), true);
  }
});
$("shared-refresh").addEventListener("click", () => loadShared(true));
$("shared-sort").addEventListener("change", () => loadShared(true));

$("shared-share").addEventListener("click", async () => {
  const game = gameFor(open);
  if (!game) return;
  const button = $("shared-share");
  button.disabled = true;
  try {
    toast(
      await invoke("share_table", {
        exe: game.exe,
        anonymous: !!me && $("share-anon").checked,
      })
    );
    await loadShared(true);
  } catch (e) {
    toast(String(e), true);
  } finally {
    button.disabled = false;
  }
});

/* ---------- who you publish as ---------- */

async function drawWhoami() {
  let who = null;
  try {
    who = await invoke("whoami");
  } catch {
    // stays anonymous
  }
  const changed = me !== (who ? who.name : null);
  me = who ? who.name : null;
  $("share-anon-row").hidden = !me;
  if (!me) $("share-anon").checked = false;
  if (changed && open) loadShared(true);
  $("whoami-state").textContent = who
    ? `What you share is signed as ${who.name}, and nobody else can use that name.`
    : "Nothing you share carries a name. Claim one and it is registered to a key only you hold.";
  $("claim-name").hidden = !!who;
  $("forget-name").hidden = !who;
}

function showNameSheet(step) {
  $("name-sheet").hidden = false;
  $("name-pick").hidden = step !== "pick";
  $("name-recover").hidden = step !== "recover";
  $("name-phrase").hidden = step !== "phrase";

  const words = {
    pick: ["Claim a name", "Nobody else can publish under it once it is yours. No password, no email."],
    recover: ["Get your name back", "Type the words you wrote down when you claimed it."],
    phrase: ["Write these down", "This is the only copy. There is no reset and no way to see them again."],
  }[step];
  $("name-title").textContent = words[0];
  $("name-blurb").textContent = words[1];

  const focus = { pick: "name-input", recover: "recover-name", phrase: "phrase-save" }[step];
  setTimeout(() => $(focus).focus(), 0);
}

$("claim-name").addEventListener("click", () => {
  $("name-input").value = "";
  $("name-why").textContent = "";
  showNameSheet("pick");
  $("name-input").focus();
});

/* every one of these looks like a form and none of them took the return key */
function submitOn(field, button) {
  $(field).addEventListener("keydown", (e) => {
    if (e.key !== "Enter" || e.shiftKey) return;
    e.preventDefault();
    $(button).click();
  });
}

submitOn("name-input", "name-go");
submitOn("recover-name", "recover-go");
submitOn("recover-phrase", "recover-go");
submitOn("import-phrase", "import-go");

$("name-cancel").addEventListener("click", () => ($("name-sheet").hidden = true));
$("name-recover-go").addEventListener("click", () => showNameSheet("recover"));
$("recover-back").addEventListener("click", () => showNameSheet("pick"));

$("name-go").addEventListener("click", async () => {
  const name = $("name-input").value.trim();
  $("name-why").textContent = "Checking";
  try {
    const words = await invoke("claim_name", { name });
    const host = $("phrase-words");
    host.innerHTML = "";
    words.forEach((word, at) => {
      const el = document.createElement("span");
      const number = document.createElement("b");
      number.textContent = at + 1;
      el.append(number, document.createTextNode(" " + word));
      host.appendChild(el);
    });
    host.dataset.phrase = words.join(" ");
    host.dataset.name = name;
    $("phrase-saved").textContent = "";
    showNameSheet("phrase");
  } catch (e) {
    $("name-why").textContent = String(e);
  }
});

$("recover-go").addEventListener("click", async () => {
  $("recover-why").textContent = "Checking";
  try {
    const name = await invoke("recover_name", {
      name: $("recover-name").value.trim(),
      phrase: $("recover-phrase").value.trim(),
    });
    $("name-sheet").hidden = true;
    toast("Welcome back, " + name);
    await drawWhoami();
  } catch (e) {
    $("recover-why").textContent = String(e);
  }
});

$("phrase-copy").addEventListener("click", () => {
  navigator.clipboard.writeText($("phrase-words").dataset.phrase || "");
  toast("Copied. Keep it somewhere that is not this machine");
});

$("phrase-save").addEventListener("click", async () => {
  try {
    const where = await invoke("save_phrase", {
      name: $("phrase-words").dataset.name || "",
      phrase: $("phrase-words").dataset.phrase || "",
    });
    $("phrase-saved").textContent = where;
    toast("Saved. Keep a copy somewhere that is not this machine");
  } catch (e) {
    if (String(e)) toast(String(e), true);
  }
});

$("phrase-done").addEventListener("click", async () => {
  $("name-sheet").hidden = true;
  await drawWhoami();
});

/* ---------- the overlay ---------- */

async function drawOverlay() {
  let state = null;
  try {
    state = await invoke("overlay_status");
  } catch (e) {
    return toast(String(e), true);
  }

  $("overlay-on").classList.toggle("on", state.on);
  $("overlay-key").textContent = state.key;
  $("overlay-key-row").hidden = !state.on;
  $("overlay-quiet-row").hidden = !state.on;
  $("overlay-quiet").classList.toggle("on", !!state.quiet);

  const why = $("overlay-key-why");
  if (state.clash) {
    why.textContent = `${state.key} is already used by ${state.clash}. Pick another one.`;
    why.className = "warn";
  } else {
    why.textContent = "Click the box, then press the keys you want.";
    why.className = "";
  }
}

$("overlay-on").addEventListener("click", async () => {
  const on = !$("overlay-on").classList.contains("on");
  try {
    await invoke("set_overlay", { on, key: null });
    toast(on ? "Overlay on. Press the shortcut while you play" : "Overlay off");
  } catch (e) {
    toast(String(e), true);
  }
  await drawOverlay();
});

$("overlay-quiet").addEventListener("click", async () => {
  const quiet = !$("overlay-quiet").classList.contains("on");
  try {
    await invoke("set_overlay", { on: null, key: null, quiet });
    toast(
      quiet
        ? "The panel will leave the game's focus alone"
        : "The panel takes focus again, so you can type into it"
    );
  } catch (e) {
    toast(String(e), true);
  }
  await drawOverlay();
});

/* the box listens for a real key press rather than asking anybody to type
   "Ctrl+Shift+O" into a text field and spell it the way we happen to parse */
let catching = false;

$("overlay-key").addEventListener("click", () => {
  catching = true;
  $("overlay-key").classList.add("catching");
  $("overlay-key").textContent = "Press the keys";
  $("overlay-key").focus();
});

$("overlay-key").addEventListener("blur", () => {
  if (!catching) return;
  catching = false;
  $("overlay-key").classList.remove("catching");
  drawOverlay();
});

$("overlay-key").addEventListener("keydown", async (e) => {
  if (!catching) return;
  e.preventDefault();

  if (e.key === "Escape") return $("overlay-key").blur();

  // wait for the key itself, a modifier on its own is not a shortcut
  const held = ["Control", "Shift", "Alt", "Meta"];
  if (held.includes(e.key)) return;

  const parts = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  parts.push(e.key.length === 1 ? e.key.toUpperCase() : e.key);

  catching = false;
  $("overlay-key").classList.remove("catching");

  try {
    await invoke("set_overlay", { on: null, key: parts.join("+") });
    toast("Shortcut set to " + parts.join("+"));
  } catch (err) {
    toast(String(err), true);
  }
  await drawOverlay();
});

$("overlay-key-reset").addEventListener("click", async () => {
  try {
    await invoke("set_overlay", { on: null, key: "Ctrl+Shift+O" });
  } catch (e) {
    toast(String(e), true);
  }
  await drawOverlay();
});

/* ---------- moving to another machine ---------- */

let exportPicks = new Set();

async function openExport() {
  let list = [];
  try {
    list = await invoke("profile_games");
  } catch (e) {
    return toast(String(e), true);
  }

  let who = null;
  try {
    who = await invoke("whoami");
  } catch {
    // no name, so there is nothing to carry over
  }
  const box = $("export-account");
  box.disabled = !who;
  box.checked = false;
  box.closest(".check").querySelector("i").textContent = who
    ? `Carries the name ${who.name}. The import asks for your recovery words, so the file on its own cannot publish as you.`
    : "You have not claimed a name yet.";

  exportPicks = new Set(list.map((g) => g.exe));
  const host = $("export-games");
  host.innerHTML = "";

  if (!list.length) {
    host.innerHTML = '<p class="dim">No games have anything set on them yet.</p>';
  }

  for (const game of list) {
    const row = document.createElement("label");
    row.className = "check";

    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = true;
    box.addEventListener("change", () => {
      if (box.checked) exportPicks.add(game.exe);
      else exportPicks.delete(game.exe);
    });
    box.dataset.exe = game.exe;

    const label = document.createElement("span");
    const title = document.createElement("b");
    title.textContent = game.name;
    const sub = document.createElement("i");
    const bits = [];
    if (game.cheats) bits.push(`${game.cheats} on`);
    if (game.values) bits.push(`${game.values} set`);
    if (game.shared) bits.push("shared table");
    sub.textContent = bits.join(", ");
    label.append(title, sub);

    row.append(box, label);
    host.appendChild(row);
  }

  $("export-why").textContent = "";
  $("export-sheet").hidden = false;
  $("export-go").focus();
}

function setAllGames(on) {
  exportPicks = new Set();
  for (const box of document.querySelectorAll("#export-games input")) {
    box.checked = on;
    if (on) exportPicks.add(box.dataset.exe);
  }
}

$("export-profile").addEventListener("click", openExport);
$("export-close").addEventListener("click", () => ($("export-sheet").hidden = true));
$("export-all").addEventListener("click", () => setAllGames(true));
$("export-none").addEventListener("click", () => setAllGames(false));

$("export-go").addEventListener("click", async () => {
  $("export-why").textContent = "";
  $("export-go").disabled = true;
  try {
    const note = await invoke("export_profile", {
      prefs: $("export-prefs").checked,
      account: $("export-account").checked,
      games: [...exportPicks],
    });
    $("export-sheet").hidden = true;
    toast(note);
  } catch (e) {
    if (String(e)) $("export-why").textContent = String(e);
  } finally {
    $("export-go").disabled = false;
  }
});

async function openImport(path) {
  let peek = null;
  try {
    peek = await invoke("open_profile", { path: path || null });
  } catch (e) {
    if (String(e)) toast(String(e), true);
    return;
  }

  const bits = [];
  if (peek.prefs) bits.push("your preferences");
  if (peek.games) bits.push(many(peek.games, "game"));
  if (peek.tables) bits.push(many(peek.tables, "table") + " to download");
  $("import-summary").textContent = bits.length
    ? `That file has ${bits.join(", ")}.`
    : "That file is empty.";

  $("import-needs-phrase").hidden = !peek.account;
  $("import-phrase").value = "";
  $("import-why").textContent = peek.account
    ? `It was exported by ${peek.account}.`
    : "";
  $("import-sheet").hidden = false;
  (peek.account ? $("import-phrase") : $("import-go")).focus();
}

const dropProfile = (path) => openImport(path);
$("import-profile").addEventListener("click", () => openImport());

$("import-close").addEventListener("click", () => ($("import-sheet").hidden = true));
$("import-cancel").addEventListener("click", () => ($("import-sheet").hidden = true));

$("import-go").addEventListener("click", async () => {
  $("import-why").textContent = "Working";
  $("import-go").disabled = true;
  try {
    const note = await invoke("apply_profile", { phrase: $("import-phrase").value || null });
    $("import-sheet").hidden = true;
    toast(note);
    config = await invoke("settings");
    applyTheme();
    await loadGames(true);
    await drawWhoami();
  } catch (e) {
    $("import-why").textContent = String(e);
  } finally {
    $("import-go").disabled = false;
  }
});

$("forget-name").addEventListener("click", async () => {
  try {
    await invoke("forget_name");
    toast("Signed out. Uploads go out anonymously now");
    await drawWhoami();
  } catch (e) {
    toast(String(e), true);
  }
});

async function start() {
  bootStep("Reading your settings");
  try {
    config = await invoke("settings");
  } catch (e) {
    toast("Could not read your settings: " + e, true);
  }
  applyTheme();
  enhanceSelect($("scan-type"));
  await drawWhoami();
  drawWindowState();
  drawVersion();
  countTables();
  drawOverlay();
  drawQuestion();
  $("settings-path").textContent = "%APPDATA%\\freeplay\\settings.json";

  bootStep("Looking through Steam, Epic and GOG");
  await loadGames();
  stopBooting();

  setInterval(loadGames, 5000);
  // a sitting can end while this window is closed, and the snooze runs out on
  // its own, so this is not only driven by the detach event
  setInterval(drawQuestion, 60000);
  setInterval(refreshCheats, 1500);

  if (window.__TAURI__.event) {
    const { listen } = window.__TAURI__.event;
    listen("attached", (e) => {
      attached = e.payload;
      drawn = "";
      draw();
      if (open) drawGamePage();
      toast(`Attached to ${attached.game}`);
    });
    // the overlay goes over a game, so pressing the key with nothing attached
    // has to say something rather than looking broken
    listen("overlay-refused", (e) => toast(String(e.payload), true));
    // a cheat key landed in the game, so the cards are out of date
    listen("keys-fired", (e) => {
      const hit = e.payload || {};
      if (hit.panic) toast("Panic key. Everything switched off");
      const game = gameFor(open);
      if (game && sameExe(game.exe, hit.exe)) refreshCheats();
    });
    listen("detached", () => {
      attached = null;
      // this is the moment a sitting ends, and the moment there is something
      // to ask about
      drawQuestion();
      // the backend threw the search away with the process, so the addresses
      // on the finder are pointing at memory that no longer exists
      resetScan();
      showView(currentView());
      drawn = "";
      draw();
      if (open) drawGamePage();
    });
  }
}

start();
