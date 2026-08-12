"""click through the ui with a fake tauri bridge behind it.

nothing type checks the front end, so the only way to know a click still works
is to do the click. copies app/ui somewhere temporary, swaps window.__TAURI__
for canned answers, drives it in headless edge or chrome, and fails if anything
throws or a view will not open.

a missing id="game-cover" once made every game page throw before it was shown,
so clicking a game did nothing at all and cargo test could not see it.

    python app/uitest/clickthrough.py
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
UI = os.path.join(HERE, "..", "ui")

STUB = r"""
<script>
window.__errors = [];
window.addEventListener("error", e => window.__errors.push(String(e.message)));
window.addEventListener("unhandledrejection", e => window.__errors.push("promise: " + e.reason));

/* this runs where app.js is loaded, after the markup and before any of its
   timers, so it is the last moment the opening screen is guaranteed to be
   untouched. recorded rather than looked at later, because a slow machine
   gets to the probe after app.js has already taken the screen away and that
   is not the same thing as it never having been there */
window.__atStart = {
  splash: !!document.getElementById("splash"),
  booting: document.body.classList.contains("booting")
};

const GAMES = [
  {key:"steam:20920", name:"The Witcher 2", store:"Steam", exe:"witcher2.exe",
   dir:"D:/games/witcher2", app_id:"20920", running:true, has_table:false,
   guard:null, minutes:1801, last_played:1785824227, favourite:false},
  {key:"steam:1222140", name:"Detroit Become Human", store:"Steam", exe:"Detroit.exe",
   dir:"D:/games/detroit", app_id:"1222140", running:false, has_table:false,
   guard:null, minutes:1065, last_played:1783896512, favourite:false},
  {key:"gog:noexe", name:"Some GOG Game", store:"GOG", exe:null,
   dir:"D:/games/gog", app_id:null, running:false, has_table:false,
   guard:null, minutes:null, last_played:null, favourite:false},
  {key:"steam:2073850", name:"THE FINALS", store:"Steam", exe:"Discovery.exe",
   dir:"D:/games/finals", app_id:"2073850", running:false, has_table:false,
   guard:"easyanticheat", minutes:73451, last_played:1786142235, favourite:false}
];

const plain = {editable:false, kind:"", value:"", current:"", choices:[], hex:false, holds:true};
const CHEATS = [
  {id:"base", name:"Get Witcher Base", category:"Misc", description:"", hint:"",
   state:"idle", reason:"", armed:false, live:false, does:"Script", ...plain},
  {id:"vitality", name:"Infinite Vitality", category:"Player", description:"never die", hint:"",
   state:"idle", reason:"", armed:true, live:false, does:"Freeze", ...plain,
   editable:true, kind:"f32", value:"9999", current:"312"},
  {id:"orens", name:"Orens", category:"Resources", description:"money", hint:"",
   state:"idle", reason:"", armed:false, live:false, does:"Value", ...plain,
   editable:true, kind:"i32", value:"5000", current:"120"},
  {id:"difficulty", name:"Difficulty", category:"Game", description:"", hint:"",
   state:"idle", reason:"", armed:false, live:false, does:"Set once", ...plain,
   editable:true, kind:"i32", value:"1", holds:false,
   choices:[{value:"0",label:"Easy"},{value:"1",label:"Normal"},{value:"2",label:"Hard"}]}
];

const OTHER_CHEATS = [
  {id:"chapters", name:"Unlock Chapters", category:"Game", description:"", hint:"",
   state:"idle", reason:"", armed:false, live:false, does:"Value", ...plain,
   editable:true, kind:"i32", value:"12"}
];

const SORTS = [
  {key:"best", label:"Best match"},
  {key:"votes", label:"Most liked"},
  {key:"downloads", label:"Most used"},
  {key:"new", label:"Newest"}
];

const SHARED = [
  {id:7, game:"The Witcher 2", by:"aSwedishMagyar", cheats:23, up:9, down:1,
   downloads:140, built_for:"3.5.0.1", added:1786142235, standing:"",
   installed:false, fit:"same", fit_note:"Tested on your version, 3.5.0.1",
   recommended:true},
  {id:8, game:"The Witcher 2", by:"aSwedishMagyar", cheats:11, up:0, down:0,
   downloads:0, built_for:"", added:1786142235, standing:"",
   installed:true, fit:"unknown",
   fit_note:"Nobody recorded which version this was tested on",
   recommended:false},
  // a different game that happens to ship the same executable name, uploaded
  // by the account everything converted goes up under
  {id:9, game:"Some Other Game", by:"SomeoneElse", cheats:31, up:2, down:0,
   downloads:89, built_for:"3.4.0.2", added:1786142235, standing:"",
   installed:false, fit:"older", fit_note:"Tested on 3.4.0.2, you have 3.5.0.1",
   recommended:false}
];

const PHRASE = ["cold","burst","cash","camel","cargo","bread","cloth","clerk",
  "adopt","camel","bear","candy","chalk","bank","alloy","boot","column"];

let OVERLAY = {on: false, key: "Ctrl+Shift+O"};
let QUESTION = {id: 7, game: "The Witcher 2", by: "aSwedishMagyar",
                played: "1.5 hours", cheats: 3};
window.__calls = [];
/* save_settings hands back what it was given, same as the real one. a stub
   that answered with a fixed object hid the bug where pinning a game did not
   show up until you left the page */
let SETTINGS = {theme:"dark", accent:"amber", favourites:[], shared_open:true, community:true,
                armed:{"witcher2.exe":["vitality"]},
                values:{"witcher2.exe":{"orens":"5000"}},
                grabbed:{"witcher2.exe":7}};
window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      window.__calls.push(cmd);
      switch (cmd) {
        case "settings": return SETTINGS;
        case "save_settings":
          // the real one keeps everything the front end does not own, so it
          // has to be spelled out here rather than merged blindly
          SETTINGS = {
            ...SETTINGS,
            theme: args.next.theme, accent: args.next.accent,
            favourites: args.next.favourites,
            auto_update: args.next.auto_update, auto_attach: args.next.auto_attach,
            shared_open: args.next.shared_open,
            community: args.next.community,
          };
          return SETTINGS;
        case "list_games": return GAMES;
        case "game_art": return {cover:null, hero:null, logo:null};
        case "cheats":
          // the witcher answers slowly on purpose. switching away mid flight
          // used to paint the game you had just left
          if (args.exe === "witcher2.exe") {
            await new Promise(r => setTimeout(r, 400));
            return CHEATS;
          }
          if (args.exe === "Detroit.exe") {
            await new Promise(r => setTimeout(r, 300));
            return OTHER_CHEATS;
          }
          return [];
        case "set_cheat": return null;
        case "credit":
          if (args.exe === "witcher2.exe") {
            return {author: "aSwedishMagyar",
                    source: "https://fearlessrevolution.com/viewtopic.php?t=14844",
                    notes: "Converted from a Cheat Engine table by aSwedishMagyar."};
          }
          // a table nobody put a name on
          return {author: "", source: "", notes: ""};
        case "sort_options": return SORTS;
        case "shared_tables":
          if (!SETTINGS.community) throw "shared tables are turned off";
          window.__askedAbout = args.exe;
          await new Promise(r => setTimeout(r, 350));
          return args.exe === "witcher2.exe" ? SHARED : [];
        case "install_shared": return "The Witcher 2 is ready, 23 cheats";
        case "pending_question": return QUESTION;
        case "answer_question":
          window.__answered = {id: args.id, up: args.up};
          QUESTION = null;
          return args.up ? "Thanks" : "Noted";
        case "skip_question": window.__skipped = true; QUESTION = null; return null;
        case "share_table": return "shared, it is number 9";
        case "whoami": return window.__claimed ? {name: window.__claimed} : null;
        case "claim_name": window.__claimed = args.name; return PHRASE;
        case "list_processes": return [{pid:1234, name:"witcher2.exe"}];
        case "set_cheat_value": return "5000";
        case "profile_games": return [
          {exe:"witcher2.exe", name:"The Witcher 2", cheats:3, values:2, shared:true},
          {exe:"detroit.exe", name:"Detroit Become Human", cheats:1, values:0, shared:false}];
        case "export_profile": return "Saved 2 games to D:/profile.freeplay";
        case "open_profile": return {games:2, prefs:true, account:"aSwedishMagyar", tables:1};
        case "apply_profile": return "Imported preferences, 2 games";
        case "save_phrase": return "Saved to D:/words.txt";
        case "pick_table": return "23 cheats imported, 0 skipped";
        case "open_url": return null;
        case "focus_game": window.__focused = args.exe; return null;
        case "overlay_status":
          return {on: OVERLAY.on, key: OVERLAY.key, showing: false,
                  clash: OVERLAY.key === "Alt+Z" ? "the NVIDIA overlay" : null,
                  game: "witcher2.exe"};
        case "set_overlay":
          if (args.on !== null && args.on !== undefined) OVERLAY.on = args.on;
          if (args.key) OVERLAY.key = args.key;
          return null;
        case "toggle_overlay":
          if (!window.__attachedNow) throw "attach to a game first, the overlay goes over the game";
          return true;
        case "hide_overlay": window.__hidden = true; return null;
        case "overlay_game":
          return {process:"witcher2.exe", pid:1234, game:"The Witcher 2",
                  table:true, arch:"32-bit"};
        case "remove_table": return "Removed. What you had switched on for it is forgotten too";
        case "version": return "Version 0.1.0 for 64-bit Windows";
        case "table_count": return "3 tables";
        case "update_tables": return "3 tables, up to date";
        case "attach":
          return {process:"witcher2.exe", pid:1234, game:"The Witcher 2",
                  table:false, arch:"32-bit"};
        default: return null;
      }
    }
  },
  window: {
    getCurrentWindow: () => ({
      minimize(){}, close(){},
      async toggleMaximize(){ window.__big = !window.__big; },
      async isMaximized(){ return !!window.__big; }
    })
  }
};
</script>
"""

PROBE = r"""
<script>
(async () => {
  const out = [];
  const finish = () => {
    const pre = document.createElement("pre");
    pre.id = "probe-results";
    pre.textContent = out.join(String.fromCharCode(10));
    document.body.appendChild(pre);
  };
  try {
  const visible = id => { const el = document.getElementById(id); return !!el && !el.hidden; };
  const note = (ok, label) => out.push((ok ? "PASS " : "FAIL ") + label);
  const settle = ms => new Promise(r => setTimeout(r, ms));

  /* the opening screen covers the whole window, so the way it fails is by
     staying there. it also has to outlast a fast start rather than blinking.

     app.js holds it 1200ms then fades it for 600ms. how much of that is left
     by the time this runs depends on how quickly the browser got here, and a
     ci runner is a lot slower than this machine, so everything below is timed
     off the page's own clock rather than assuming we arrived at zero */
  const HOLD = 1200, FADE = 600;
  const left = ms => Math.max(0, ms - performance.now());

  note(window.__atStart.splash, "the opening screen is up to begin with");
  note(window.__atStart.booting, "and the window knows it is still starting");
  // the class and the screen have to agree. which of the two states we catch
  // is a matter of timing, the two disagreeing never is
  const splash = document.getElementById("splash");
  const stillUp = !!splash && !splash.classList.contains("gone");
  note(document.body.classList.contains("booting") === stillUp,
       "and the booting class says the same as the screen does");
  const winButtons = document.getElementById("win-close").getBoundingClientRect();
  note(document.elementFromPoint(winButtons.x + winButtons.width / 2,
                                 winButtons.y + winButtons.height / 2)
         ?.closest("#win-close") !== null,
       "the close button is still the thing under the cursor while it is up");

  // still there a good way into the hold, so it cannot blink past on a fast
  // start. only worth asserting if we got here early enough to watch it
  const earlyEnough = performance.now() < HOLD - 400;
  await settle(Math.min(500, left(HOLD - 100)));
  note(!earlyEnough || !!document.getElementById("splash"),
       "it holds for a moment on a fast start");

  await settle(left(HOLD) + 250);
  note(!document.body.classList.contains("booting"), "then it lets go");
  await settle(left(HOLD + FADE) + 250);
  note(!document.getElementById("splash"), "and takes itself out of the page");

  /* a game with an anti-cheat cannot be used at all, so it goes to the end
     under its own heading rather than sitting in the middle of the ones that
     work with a small badge as the only clue */
  note(visible("blocked-wrap"), "games with an anti-cheat get their own shelf");
  note(document.getElementById("blocked-count").textContent === "1",
       "and it is counted (" + document.getElementById("blocked-count").textContent + ")");
  note(document.querySelector("#blocked-grid .card") !== null,
       "with the game in it");
  note([...document.querySelectorAll("#grid .card")]
         .every(c => !c.className.includes("guarded")),
       "and it is not in the main grid as well");
  const barredHead = document.querySelector("#blocked-wrap h3").textContent.toLowerCase();
  note(barredHead.includes("not supported"),
       "the heading says what it means (" + barredHead + ")");
  note(document.querySelector("#blocked-wrap .shelf-note").textContent.toLowerCase()
         .includes("anti-cheat"),
       "and the line under it says why");

  const lastRail = [...document.querySelectorAll("#library-rail .rail-game")].pop();
  note(lastRail.className.includes("barred"),
       "the sidebar keeps them at the bottom and marks them");
  note(!!document.querySelector("#library-rail .rail-split"),
       "with a line above saying not supported");
  note(document.querySelectorAll("#library-rail .rail-split").length === 1,
       "drawn once, not once per game");

  const cards = document.querySelectorAll("#grid .card, #fav-grid .card");
  note(cards.length >= 1, "library rendered cards (" + cards.length + ")");
  note(visible("view-library"), "library view is showing");

  // the question waits for the game to close rather than appearing over it,
  // which is the one moment nobody is looking at this window
  note(visible("ask"), "a table that was played gets asked about afterwards");
  note(document.getElementById("ask-title").textContent
         .includes("Did the table for The Witcher 2 work?"),
       "and it names the game");
  const detail = document.getElementById("ask-detail").textContent;
  note(detail.includes("aSwedishMagyar"), "and who wrote the table");
  note(detail.includes("1.5 hours"), "and how long you played (" + detail + ")");
  note(detail.includes("3 cheats"), "and how many cheats you had on");
  note(document.querySelector(".ask-why").textContent.toLowerCase()
         .includes("next person"),
       "and says why answering is worth anything");

  // nobody has to answer
  note(!!document.getElementById("ask-skip"), "there is a way out of answering");
  document.getElementById("ask-skip").click();
  await settle(300);
  note(window.__skipped === true, "skipping is remembered");
  note(!visible("ask"), "and the question goes away");

  if (cards.length) {
    // pinned games are drawn first, so the first card in the document is not
    // the first game in the list
    const witcher = [...cards].find(c => c.textContent.includes("Witcher")) || cards[0];
    witcher.click();
    await settle(700);
    note(visible("view-game"), "clicking a card opens the game page");
    note(!visible("view-library"), "library is hidden once the game page opens");
    const name = (document.getElementById("game-name") || {}).textContent || "";
    note(name.length > 0, "game page shows a name (" + JSON.stringify(name) + ")");
    const facts = document.querySelectorAll("#game-facts .fact").length;
    note(facts >= 2, "game page shows facts (" + facts + ")");
    note(!!document.getElementById("game-play"), "game page has a play button");

    note(!!document.getElementById("detail-exe").textContent,
         "detail rows name the executable");
    note(document.getElementById("detail-dir").textContent.length > 0,
         "detail rows show the install folder");

    // cheats list and switch on with nothing attached and the game closed
    const listed = document.querySelectorAll("#cheat-groups .cheat").length;
    note(listed === 4, "cheats list without attaching (" + listed + ")");
    const groups = document.querySelectorAll("#cheat-groups .group").length;
    note(groups === 4, "cheats are grouped by category (" + groups + ")");
    note(!visible("no-table"), "no-table notice stays hidden when there is a table");

    // whoever found the addresses is not whoever uploaded the table, and the
    // person who did the work is the one worth naming on the page
    note(visible("table-credit"), "the table credits whoever worked it out");
    note(document.getElementById("credit-author").textContent === "aSwedishMagyar",
         "and names them");
    const src = document.getElementById("credit-source");
    note(visible("credit-source"), "with a link back to where it came from");
    note(src.textContent === "fearlessrevolution.com",
         "shown as the site, not the whole url (" + src.textContent + ")");
    note(src.dataset.open === "https://fearlessrevolution.com/viewtopic.php?t=14844",
         "and the link goes to the thread");

    const switches = document.querySelectorAll("#cheat-groups .switch");
    note([...switches].every((s) => !s.disabled),
         "every toggle is usable with the game closed");
    note(document.querySelectorAll("#cheat-groups .cheat.armed").length === 1,
         "an armed cheat is drawn as armed");

    switches[0].click();
    await settle(250);
    note(window.__calls.includes("set_cheat"), "toggling arms the cheat");

    // a cheat that needs a number, which plenty of them are
    const boxes = document.querySelectorAll("#cheat-groups .cheat-value input");
    note(boxes.length === 2, "value cheats get a box to type in (" + boxes.length + ")");
    note(boxes[0].value === "9999", "the box starts on what the table suggests");
    const drops = document.querySelectorAll("#cheat-groups .cheat-value select");
    note(drops.length === 1, "a cheat with listed options gets a dropdown");
    note(drops[0].options.length === 3, "the dropdown lists every option");
    note(drops[0].value === "1", "the dropdown starts on the right option");
    const live = document.querySelector("[data-live-for=orens]");
    note(live && live.textContent === "now 120",
         "the box says what the game is holding now");
    note(document.querySelectorAll(".cheat-once").length === 1,
         "a cheat that does not hold its value says so");

    boxes[1].value = "12345";
    boxes[1].dispatchEvent(new Event("change"));
    await settle(250);
    note(window.__calls.includes("set_cheat_value"), "typing a number saves it");

    // cheats own the wide column, the shared list folds away at the side
    note(visible("cheats-panel"), "cheats sit in the main column");
    note(document.querySelectorAll("#cheat-groups .cheat").length === 4,
         "and every one of them is there");
    note(visible("shared"), "the shared tables sit in their own panel");
    note(!visible("dock-open"), "the reopen tab is hidden while the panel is open");
    document.getElementById("dock-close").click();
    await settle(300);
    note(!visible("shared"), "the shared panel folds away");
    note(visible("dock-open"), "and leaves a tab to bring it back");
    note(document.getElementById("dock-open-count").textContent === "3",
         "the tab says how many are on offer");
    document.getElementById("dock-open").click();
    await settle(300);
    note(visible("shared"), "the tab brings it back");
    note(visible("cheats-panel"), "and the cheats never moved");

    // nothing should be rebuilt when nothing changed
    const first = document.querySelector("#cheat-groups .cheat");
    await settle(1800);
    note(document.querySelector("#cheat-groups .cheat") === first,
         "polling does not rebuild the cards underneath you");

    // flipping a switch patches the one card instead of the whole list
    const before = [...document.querySelectorAll("#cheat-groups .cheat")];
    document.querySelectorAll("#cheat-groups .switch")[2].click();
    await settle(400);
    const after = [...document.querySelectorAll("#cheat-groups .cheat")];
    note(before.every((card, at) => card === after[at]),
         "and neither does flipping a switch");

    // searching the shared list
    document.getElementById("shared-search").value = "aSwedishMagyar";
    document.getElementById("shared-search").dispatchEvent(new Event("input"));
    await settle(300);
    const matching = [...document.querySelectorAll("#shared-list .shared-row")]
      .filter(r => !r.hidden);
    note(matching.length === 2, "searching by who shared it keeps their tables");
    note([...document.querySelectorAll("#shared-list .shared-row")].some(r => r.hidden),
         "and drops everybody else");
    document.getElementById("shared-search").value = "nobody";
    document.getElementById("shared-search").dispatchEvent(new Event("input"));
    await settle(300);
    note([...document.querySelectorAll("#shared-list .shared-row")].every(r => r.hidden),
         "and hides the rest");
    document.getElementById("shared-search").value = "";
    document.getElementById("shared-search").dispatchEvent(new Event("input"));
    await settle(300);

    // getting rid of a downloaded table
    const remove = [...document.querySelectorAll("#shared-list button")]
      .find(b => b.textContent === "Remove");
    note(!!remove, "an installed table can be removed");
    if (remove) {
      remove.click();
      await settle(400);
      note(window.__calls.includes("remove_table"), "removing it deletes the file");
    }
    note(visible("remove-table"), "and there is a way to remove one that was never shared");

    // starring without having to leave the page and come back
    const fav = document.getElementById("game-fav");
    const wasFav = fav.classList.contains("on");
    fav.click();
    await settle(300);
    note(fav.classList.contains("on") !== wasFav, "the star fills in straight away");
    note(fav.title.toLowerCase().includes(wasFav ? "add to" : "remove"),
         "and the tooltip says what clicking it again does");
    document.getElementById("back").click();
    await settle(300);
    note(visible("fav-wrap"), "and the game gets a favourites shelf of its own");
    note(document.querySelectorAll("#fav-grid .card").length === 1,
         "with the game in it");
    document.querySelector("#fav-grid .card").click();
    await settle(700);

    document.getElementById("game-import").click();
    await settle(250);
    note(window.__calls.includes("pick_table"), "the game page can import a .CT");

    // shared tables
    const rows = document.querySelectorAll("#shared-list .shared-row");
    note(rows.length === 3, "shared tables listed (" + rows.length + ")");

    /* two games can ship the same executable, and everything converted from
       one place goes up under one account. the game the table is for is the
       only thing that tells those rows apart, so it is the heading */
    const names = [...rows].map(r => r.querySelector(".shared-name").textContent);
    note(names[0] === "The Witcher 2",
         "a shared row is headed by the game, not the uploader (" + names[0] + ")");
    note(names.includes("Some Other Game"),
         "so two games sharing an exe are told apart (" + JSON.stringify(names) + ")");
    const byline = rows[0].querySelector(".shared-by");
    note(byline && byline.textContent.startsWith("by aSwedishMagyar"),
         "and the uploader moves under it (" + (byline && byline.textContent) + ")");
    note(document.querySelectorAll("#shared-sort option").length === 4,
         "sort options filled in");

    // the windows popup ignores the theme entirely, so we draw our own
    const face = document.querySelector("#shared-sort").closest(".picker")
      .querySelector(".picker-face");
    note(!!face, "the sort list is drawn by us, not by windows");
    note(face.textContent.includes("Best match"), "and shows what is chosen");
    note(document.querySelectorAll(".picker-menu:not([hidden])").length === 0,
         "with nothing open to start with");

    face.click();
    await settle(200);
    const menu = document.querySelector(".picker-menu:not([hidden])");
    note(!!menu, "clicking it opens the list");
    note(menu.querySelectorAll(".picker-item").length === 4,
         "with every option in it");
    note(menu.querySelector(".picker-item.on").textContent === "Best match",
         "and the current one marked");

    document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true}));
    menu.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true}));
    await settle(200);
    note(document.querySelectorAll(".picker-menu:not([hidden])").length === 0,
         "escape closes it");

    face.click();
    await settle(200);
    const pickItems = document.querySelectorAll(".picker-menu:not([hidden]) .picker-item");
    pickItems[2].click();
    await settle(400);
    note(document.getElementById("shared-sort").value === "downloads",
         "choosing one sets the value behind it");
    note(face.textContent.includes("Most used"), "and the face follows");
    note(document.querySelectorAll(".picker-menu:not([hidden])").length === 0,
         "and it closes on its own");

    // put it back so the rest of the run is on best match
    face.click();
    await settle(150);
    document.querySelectorAll(".picker-menu:not([hidden]) .picker-item")[0].click();
    await settle(400);
    // authenticity and "does it work" are different claims and the badge
    // must not blur them
    const badge = document.querySelector("#shared-list .shared-row .verified");
    note(badge !== null, "a registered name is marked");
    note(badge.textContent === "name verified",
         "the badge is about the name, not the table (" + badge.textContent + ")");
    note(badge.title.toLowerCase().includes("not a claim that the table works"),
         "and says so outright");
    // whoever uploaded a converted table is not whoever wrote it, and the
    // badge used to say "author verified" over the uploader's name
    note(!badge.textContent.includes("author"),
         "and does not call the uploader the author");

    // which build it was tested on is what decides whether it does anything
    const fits = [...document.querySelectorAll("#shared-list .shared-fit")];
    note(fits.length === 3, "every row leads with its version compatibility");
    note(fits[0].textContent.includes("Tested on your version"),
         "a table for your build says so plainly");
    note(fits[0].classList.contains("same"), "and is marked as a match");
    note(fits[2].textContent.includes("you have 3.5.0.1"),
         "one for another build says which build you are on");
    note(fits[2].classList.contains("older"), "and is marked as a mismatch");

    // the recommendation
    const picks = document.querySelectorAll("#shared-list .shared-row.pick");
    note(picks.length === 1, "exactly one table is recommended");
    note(picks[0].querySelector(".pick-flag").textContent === "Recommended",
         "and it says why it is at the top");
    note(rows[0].classList.contains("pick"),
         "the recommended one is the one you see first");
    note(document.querySelectorAll("#shared-list .shared-row.have").length === 1,
         "one is already installed");

    const blurb = document.querySelector("#shared-list .shared-facts").textContent;
    note(blurb.includes("23 cheats"), "row shows the cheat count");
    note(blurb.includes("9 up"), "row shows votes");
    note(blurb.includes("140 downloads"), "row shows downloads");
    note(!blurb.includes("3.5.0.1"),
         "the version is not buried at the end of the counts any more");

    const use = [...document.querySelectorAll("#shared-list button")].find(b => !b.disabled);
    note(use.textContent === "Use table", "the button says what it does");
    note([...document.querySelectorAll("#shared-list button")]
           .some(b => b.textContent === "Installed"),
         "and one already on disk says Installed");
    use.click();
    await settle(400);
    note(window.__calls.includes("install_shared"), "using a shared table installs it");

    // the witcher is running in the stub, so this is not a play button
    const play = document.getElementById("game-play");
    note(play.textContent === "Switch to game",
         "a running game does not offer to be started again");
    play.click();
    await settle(300);
    note(window.__focused === "witcher2.exe",
         "it brings the game forward instead");
  }

  const back = document.getElementById("back");
  if (back) {
    back.click();
    await settle(250);
    note(visible("view-library"), "back returns to the library");
  }

  const rail = document.querySelectorAll("#library-rail .rail-game");
  note(rail.length >= 1, "side rail rendered (" + rail.length + ")");

  // switching straight from one game to another must not leave the first
  // one's cheats on screen. the witcher stub answers slowly to force it
  rail[0].click();
  await settle(600);
  const witcherCheats = document.querySelectorAll("#cheat-groups .cheat:not(.bone)").length;
  note(witcherCheats === 4, "the first game's cheats are up (" + witcherCheats + ")");

  const detroit = [...rail].find(r => r.textContent.includes("Detroit"));
  detroit.click();
  await settle(150);
  note(document.getElementById("game-play").textContent === "Play",
       "a game that is not running still says play");
  note(document.querySelectorAll("#cheat-groups .bone").length > 0,
       "switching puts placeholders up straight away");
  await settle(900);
  const names = [...document.querySelectorAll("#cheat-groups .cheat-name")]
    .map(n => n.textContent);
  note(names.length === 1 && names[0].startsWith("Unlock Chapters"),
       "and the new game's cheats replace them (" + JSON.stringify(names) + ")");
  note(!visible("table-credit"),
       "a table with nobody's name on it credits nobody");

  // searching within a table
  document.getElementById("cheat-filter").value = "zzz";
  document.getElementById("cheat-filter").dispatchEvent(new Event("input"));
  await settle(120);
  note(!document.querySelector("#cheat-groups .cheat").hidden,
       "the search waits before it runs");
  await settle(300);
  note(document.querySelector("#cheat-groups .cheat").hidden, "then it filters");
  note(visible("cheat-none"), "and says when nothing matches");
  document.getElementById("cheat-filter").value = "unlock";
  document.getElementById("cheat-filter").dispatchEvent(new Event("input"));
  await settle(300);
  note(!document.querySelector("#cheat-groups .cheat").hidden, "a match comes back");

  // switching games used to drop the panel to a single line saying Looking
  // and spring back, which reads as a flicker
  const detroitAgain = [...rail].find(r => r.textContent.includes("Detroit"));
  rail[0].click();
  await settle(900);
  const dockWas = document.querySelector(".shared-dock").offsetHeight;
  note(dockWas > 0, "the shared panel has a height to begin with");

  detroitAgain.click();
  await settle(120);
  const dockMid = document.querySelector(".shared-dock").offsetHeight;
  note(dockMid === dockWas,
       "the panel does not resize while it waits (" +
       dockWas + " then " + dockMid + ")");
  note(document.getElementById("shared-list").classList.contains("waiting"),
       "what is on screen is held there, dimmed, rather than thrown away");
  note(document.querySelectorAll("#shared-list .shared-row").length === 3,
       "so the rows are still up");

  await settle(700);
  note(!document.getElementById("shared-list").classList.contains("waiting"),
       "and the dimming lifts once the answer lands");
  const dockAfter = document.querySelector(".shared-dock").offsetHeight;
  note(dockAfter < dockWas,
       "a game with nothing shared gets a small panel, not a tall empty one (" +
       dockAfter + " against " + dockWas + ")");
  note(dockAfter < 400,
       "and it is genuinely small (" + dockAfter + ")");
  note(!document.getElementById("shared-list").textContent.includes("Looking"),
       "no word appears and vanishes in the middle of it");

  // a game we cannot find an executable for
  const gog = [...rail].find(r => r.textContent.includes("GOG"));
  gog.click();
  await settle(600);
  note(document.querySelectorAll("#cheat-groups .bone").length === 0,
       "a game with no executable does not sit on placeholders");
  note(visible("no-table"), "it says there is no table instead");

  // a game with an anti-cheat is refused outright
  const finals = [...rail].find(r => r.textContent.includes("FINALS"));
  finals.click();
  await settle(500);
  note(visible("guarded-note"), "an anti-cheat game says Freeplay will not do it");
  note(!visible("no-table"), "and does not offer to import a table");
  note(!visible("cheats-panel"), "and shows no cheats panel");
  note(!visible("shared"), "and no shared tables");
  note(!visible("dock-open"), "and no tab to bring them back");
  note(document.getElementById("game-import").hidden, "the import button is gone");
  note(document.getElementById("game-find-table").hidden, "so is search online");
  note(document.getElementById("guarded-note").textContent.toLowerCase().includes("multiplayer"),
       "and it says why");

  if (rail.length) {
    rail[rail.length - 1].click();
    await settle(300);
    note(visible("view-game"), "clicking the side rail opens the game page");
  }

  // claiming a name
  const settingsNav = [...document.querySelectorAll(".nav-item")]
    .find(i => i.dataset.view === "settings");
  settingsNav.click();
  await settle(200);
  note(visible("claim-name"), "settings offers to claim a name");

  // opening a game page asks the service about that game, so it needs a
  // switch of its own rather than being covered by the downloads one
  note(document.getElementById("community-on").classList.contains("on"),
       "shared tables start on");
  document.getElementById("community-on").click();
  await settle(400);
  note(!document.getElementById("community-on").classList.contains("on"),
       "and can be turned off separately from automatic downloads");

  window.__askedAbout = null;
  [...document.querySelectorAll(".nav-item")].find(i => i.dataset.view === "library").click();
  await settle(200);
  document.querySelectorAll("#library-rail .rail-game")[0].click();
  await settle(800);
  note(window.__askedAbout === null,
       "with it off, opening a game page asks the service nothing");
  note(document.getElementById("shared-empty").textContent.includes("turned off"),
       "and the panel says why rather than showing an error");

  [...document.querySelectorAll(".nav-item")].find(i => i.dataset.view === "settings").click();
  await settle(200);
  document.getElementById("community-on").click();
  await settle(400);
  note(document.getElementById("community-on").classList.contains("on"),
       "turning it back on sticks");

  // the overlay, off until asked for
  note(!document.getElementById("overlay-on").classList.contains("on"),
       "the overlay starts off");
  note(!visible("overlay-key-row"), "and its shortcut is not offered until it is on");

  document.getElementById("overlay-on").click();
  await settle(400);
  note(document.getElementById("overlay-on").classList.contains("on"),
       "turning it on sticks");
  note(visible("overlay-key-row"), "and the shortcut appears");
  // the shortcut is part of the overlay, not a setting of its own, and two
  // cards of wildly different heights side by side left a hole
  note(document.getElementById("overlay-key-row").closest(".setting")
         === document.getElementById("overlay-on").closest(".setting"),
       "the shortcut lives inside the overlay setting");

  // the one card with a lot to say takes the whole row, so nothing short sits
  // beside it with a hole in the middle
  const tall = document.getElementById("overlay-on").closest(".setting");
  const short = document.getElementById("auto-update").closest(".setting");
  note(tall.offsetWidth > short.offsetWidth * 1.5,
       "the overlay setting spans the row rather than pairing with a short one");

  // every card sharing a row is the same height as the others on it
  const rows = new Map();
  for (const card of document.querySelectorAll(".settings-grid .setting")) {
    const top = Math.round(card.offsetTop);
    if (!rows.has(top)) rows.set(top, []);
    rows.get(top).push(Math.round(card.offsetHeight));
  }
  const ragged = [...rows.values()].filter(hs => new Set(hs).size > 1);
  note(ragged.length === 0,
       "cards sharing a row are the same height" +
       (ragged.length ? ": " + JSON.stringify(ragged) : ""));
  note(document.getElementById("overlay-key").textContent === "Ctrl+Shift+O",
       "with a default that treads on nothing");

  // pressing keys rather than typing the name of a key
  const cap = document.getElementById("overlay-key");
  cap.click();
  await settle(150);
  note(cap.classList.contains("catching"), "clicking it waits for a key press");
  cap.dispatchEvent(new KeyboardEvent("keydown", {key: "Shift", shiftKey: true, bubbles: true}));
  await settle(100);
  note(cap.classList.contains("catching"),
       "a modifier on its own is not a shortcut, so it keeps waiting");

  cap.dispatchEvent(new KeyboardEvent("keydown",
    {key: "j", ctrlKey: true, altKey: true, bubbles: true}));
  await settle(400);
  note(cap.textContent === "Ctrl+Alt+J", "the combination you pressed is what it takes");

  // a combination somebody else already owns is allowed but called out,
  // because not everybody runs the program that owns it
  cap.click();
  await settle(150);
  cap.dispatchEvent(new KeyboardEvent("keydown", {key: "z", altKey: true, bubbles: true}));
  await settle(400);
  note(document.getElementById("overlay-key-why").textContent.includes("NVIDIA"),
       "a shortcut another program owns is called out");
  note(document.getElementById("overlay-key-why").className === "warn",
       "and marked as a problem");

  document.getElementById("overlay-key-reset").click();
  await settle(400);
  note(document.getElementById("overlay-key").textContent === "Ctrl+Shift+O",
       "reset puts the safe one back");
  note(document.getElementById("overlay-key-why").className !== "warn",
       "and the warning goes");

  // changing a preference must not take anything else with it
  document.querySelector("#accent-pick button[data-accent=cyan]").click();
  await settle(300);
  const kept = window.__TAURI__.core;
  note(!!SETTINGS.armed && SETTINGS.armed["witcher2.exe"].length === 1,
       "changing a preference keeps what you had armed");
  note(!!SETTINGS.values && SETTINGS.values["witcher2.exe"].orens === "5000",
       "and the numbers you typed");
  note(!!SETTINGS.grabbed && SETTINGS.grabbed["witcher2.exe"] === 7,
       "and which table you installed");
  note(document.querySelectorAll("#view-settings .settings-grid").length >= 3,
       "settings is grouped rather than one long column");
  note(document.getElementById("tables-state").textContent.includes("3 tables"),
       "the table count settles instead of sitting on Checking");

  // filtering everything away used to leave a blank page
  const libraryNav = [...document.querySelectorAll(".nav-item")].find(i => i.dataset.view === "library");
  libraryNav.click();
  document.getElementById("filter").value = "zzzznothing";
  document.getElementById("filter").dispatchEvent(new Event("input"));
  await settle(250);
  note(visible("library-empty"), "filtering to nothing says so");
  note(document.querySelector("#library-empty h3").textContent.includes("Nothing matches"),
       "and says it is the filter, not a missing library");
  /* nothing pinned or starred means nothing above the first grid, and a
     heading calling it "everything else" is calling the whole library the
     leftovers. filtering the pinned one away is the same situation */
  // the gog one, because the pages above have starred the witcher
  document.getElementById("filter").value = "gog";
  document.getElementById("filter").dispatchEvent(new Event("input"));
  await settle(250);
  note(document.querySelectorAll("#grid .card").length > 0,
       "there are games in the main grid");
  note(!visible("fav-wrap"), "with nothing starred");
  note(!visible("rest-shelf"),
       "so the grid gets no Everything else heading over it");

  document.getElementById("filter").value = "";
  document.getElementById("filter").dispatchEvent(new Event("input"));
  await settle(250);
  note(!visible("library-empty"), "clearing the filter brings them back");
  note(visible("rest-shelf"),
       "and the heading comes back once there is a shelf above it");

  const settingsAgain = [...document.querySelectorAll(".nav-item")].find(i => i.dataset.view === "settings");
  settingsAgain.click();
  await settle(200);

  document.getElementById("claim-name").click();
  await settle(200);
  note(visible("name-sheet"), "the name sheet opens");

  document.getElementById("name-input").value = "aSwedishMagyar";
  document.getElementById("name-go").click();
  await settle(400);
  const words = document.querySelectorAll("#phrase-words span").length;
  note(words === 17, "the recovery phrase is shown, all " + words + " words");

  document.getElementById("phrase-done").click();
  await settle(200);
  note(!visible("name-sheet"), "the sheet closes once written down");

  // nothing outside the game page belongs to a game
  const before = window.__calls.length;
  await settle(1700);
  const asked = window.__calls.slice(before);
  note(!asked.includes("shared_tables"),
       "the shared table service is left alone from other views");

  // saving the words rather than copying them by hand
  document.getElementById("claim-name").click();
  await settle(200);
  document.getElementById("name-input").value = "aSwedishMagyar";
  document.getElementById("name-go").click();
  await settle(400);
  document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape"}));
  await settle(200);
  note(visible("name-sheet"), "escape does not throw the recovery words away");
  document.getElementById("name-sheet").click();
  await settle(200);
  note(visible("name-sheet"), "and neither does clicking outside it");
  note(document.getElementById("name-title").textContent.includes("Write these down"),
       "the sheet is titled for the step it is on");

  document.getElementById("phrase-save").click();
  await settle(300);
  note(window.__calls.includes("save_phrase"), "the phrase can be saved to a file");
  note(document.getElementById("phrase-saved").textContent.length > 0,
       "and it says where it went");
  document.getElementById("phrase-done").click();
  await settle(200);

  // exporting a profile
  document.getElementById("export-profile").click();
  await settle(300);
  note(visible("export-sheet"), "the export sheet opens");
  const picks = document.querySelectorAll("#export-games input");
  note(picks.length === 2, "every game with something set is offered (" + picks.length + ")");
  note([...picks].every(p => p.checked), "all games start ticked");
  document.getElementById("export-none").click();
  note([...document.querySelectorAll("#export-games input")].every(p => !p.checked),
       "none clears every tick");
  document.getElementById("export-all").click();
  document.getElementById("export-go").click();
  await settle(300);
  note(window.__calls.includes("export_profile"), "export writes a file");
  note(!visible("export-sheet"), "the export sheet closes after saving");

  // importing one back
  document.getElementById("import-profile").click();
  await settle(300);
  note(visible("import-sheet"), "the import sheet opens");
  note(visible("import-needs-phrase"), "a profile with an account asks for the words");
  note(document.getElementById("import-summary").textContent.includes("2 games"),
       "the import sheet says what is in the file");
  document.getElementById("import-go").click();
  await settle(400);
  note(window.__calls.includes("apply_profile"), "import applies the profile");

  // importing a .CT without knowing you can drag one in
  document.getElementById("import-table").click();
  await settle(300);
  note(window.__calls.includes("pick_table"), "settings can import a .CT file");

  // the about links go somewhere
  const aboutNav = [...document.querySelectorAll(".nav-item")].find(i => i.dataset.view === "about");
  aboutNav.click();
  await settle(200);
  const link = document.querySelector("#view-about [data-open]");
  note(!!link, "about lists a source link");
  const ver = document.getElementById("about-version");
  note(ver.textContent.includes("0.1.0"), "about says which version this is");
  /* it used to sit under four paragraphs where nobody found it. next to the
     name is where you look when you want to know which build you have */
  note(ver.closest(".about-head") !== null, "and the version sits beside the name");
  const heading = document.querySelector(".about-head h2");
  note(heading && ver.getBoundingClientRect().top < heading.getBoundingClientRect().bottom + 4,
       "on the same line as it, not under the prose");
  link.click();
  await settle(200);
  note(window.__calls.includes("open_url"), "the source link actually opens something");

  // window buttons
  document.getElementById("win-max").click();
  await settle(200);
  note(document.body.classList.contains("maximised"), "maximising swaps the icon");
  document.getElementById("win-max").click();
  await settle(200);
  note(!document.body.classList.contains("maximised"), "restoring swaps it back");

  // our own upload, back on the game page, should say so
  const railAgain = document.querySelectorAll("#library-rail .rail-game");
  railAgain[0].click();
  await settle(500);
  note(document.querySelector("#shared-list .mine") !== null,
       "a table you uploaded yourself says (you)");

  for (const item of document.querySelectorAll(".nav-item")) {
    const target = item.dataset.view;
    item.click();
    await settle(200);
    note(visible("view-" + target), "nav opens " + target);
  }

  note(!document.getElementById("library-count").textContent.includes("1 games"),
       "counts read right at one");

  // a new sitting, and coming back to the library is when it gets asked
  QUESTION = {id: 9, game: "Detroit: Become Human", by: "", played: "40 minutes", cheats: 1};
  [...document.querySelectorAll(".nav-item")].find(i => i.dataset.view === "settings").click();
  await settle(200);
  [...document.querySelectorAll(".nav-item")].find(i => i.dataset.view === "library").click();
  await settle(400);
  note(visible("ask"), "coming back to the library asks about the last sitting");
  note(document.getElementById("ask-detail").textContent.includes("The table you downloaded"),
       "an anonymous table is not called nobody's table");

  document.getElementById("ask-yes").click();
  await settle(400);
  note(window.__answered && window.__answered.id === 9 && window.__answered.up === true,
       "answering sends the vote for that table");
  note(!visible("ask"), "and the question is done with");

  // the toast used to be drawn half its own width to the right and jump
  // left the moment the animation finished, because the shared rise keyframes
  // end on transform:none and that threw the centring away
  const shownAt = el => {
    const m = new DOMMatrixReadOnly(getComputedStyle(el).transform);
    return m.m41;
  };
  toast("Checking the toast sits still");
  const bar = document.getElementById("toast");
  await settle(40);
  const early = shownAt(bar);
  await settle(400);
  const settled = shownAt(bar);
  note(Math.abs(early - settled) < 2,
       "the toast does not slide sideways as it appears (" +
       Math.round(early) + " then " + Math.round(settled) + ")");
  note(Math.abs(settled + bar.offsetWidth / 2) < 2,
       "and it is actually centred");

  // rust error strings start lowercase and land in front of somebody as is
  toast("bring the game to the front first", true);
  note(document.getElementById("toast").textContent === "Bring the game to the front first",
       "a message from the backend reads like a sentence");
  toast("witcher2.exe is not running", true);
  note(document.getElementById("toast").textContent === "witcher2.exe is not running",
       "but a file name is left alone");
  document.getElementById("toast").hidden = true;

  note(window.__errors.length === 0,
       "no uncaught errors" + (window.__errors.length ? ": " + window.__errors.join(" | ") : ""));
  } catch (e) {
    out.push("FAIL the probe itself threw: " + (e && e.message ? e.message : e));
  }
  finish();
})();
</script>
"""


def find_browser():
    candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        shutil.which("msedge"),
        shutil.which("chrome"),
        shutil.which("chromium"),
        shutil.which("chromium-browser"),
    ]
    return next((c for c in candidates if c and os.path.exists(c)), None)


def main():
    browser = find_browser()
    if not browser:
        print("no chromium based browser found, skipping the click through")
        return 0

    work = os.path.join(tempfile.mkdtemp(prefix="freeplay-uitest-"), "ui")
    shutil.copytree(UI, work)

    page_path = os.path.join(work, "index.html")
    page = open(page_path, encoding="utf-8").read()
    tag = '<script src="app.js"></script>'
    if tag not in page:
        print("index.html no longer loads app.js the way this harness expects")
        return 1
    open(page_path, "w", encoding="utf-8").write(page.replace(tag, STUB + tag + PROBE))

    url = "file:///" + page_path.replace("\\", "/")
    run = subprocess.run(
        [browser, "--headless", "--disable-gpu", "--allow-file-access-from-files",
         # the wide layout is where the two column settings grid lives, and the
         # default headless window is narrow enough to never see it
         "--window-size=1600,1000",
         "--virtual-time-budget=40000", "--dump-dom", url],
        capture_output=True, text=True, timeout=180)

    found = re.search(r'<pre id="probe-results">(.*?)</pre>', run.stdout, re.S)
    if not found:
        print("the probe never ran, so app.js threw before it could report")
        for line in run.stderr.splitlines():
            if any(word in line.lower() for word in ("error", "uncaught", "cannot", "null")):
                print("  ", line.strip()[:300])
        return 1

    body = found.group(1).strip()
    print(body)
    failed = [line for line in body.splitlines() if line.startswith("FAIL")]
    print("\n%d passed, %d failed" % (len(body.splitlines()) - len(failed), len(failed)))
    shutil.rmtree(os.path.dirname(work), ignore_errors=True)
    return 1 if failed else 0


sys.exit(main())
