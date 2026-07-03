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

const GAMES = [
  {key:"steam:20920", name:"The Witcher 2", store:"Steam", exe:"witcher2.exe",
   dir:"D:/games/witcher2", app_id:"20920", running:true, has_table:false,
   guard:null, minutes:1801, last_played:1785824227, pinned:false, favourite:false},
  {key:"steam:1222140", name:"Detroit Become Human", store:"Steam", exe:"Detroit.exe",
   dir:"D:/games/detroit", app_id:"1222140", running:false, has_table:false,
   guard:null, minutes:1065, last_played:1783896512, pinned:true, favourite:false},
  {key:"steam:2073850", name:"THE FINALS", store:"Steam", exe:"Discovery.exe",
   dir:"D:/games/finals", app_id:"2073850", running:false, has_table:false,
   guard:"easyanticheat", minutes:73451, last_played:1786142235, pinned:false, favourite:false}
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

const SORTS = [
  {key:"best", label:"Best match"},
  {key:"votes", label:"Most liked"},
  {key:"downloads", label:"Most used"},
  {key:"new", label:"Newest"}
];

const SHARED = [
  {id:7, game:"The Witcher 2", by:"aSwedishMagyar", cheats:23, up:9, down:1,
   downloads:140, built_for:"3.5.0.1", added:1786142235, standing:"", installed:false},
  {id:8, game:"The Witcher 2", by:"", cheats:11, up:0, down:0,
   downloads:0, built_for:"", added:1786142235, standing:"", installed:true}
];

const PHRASE = ["cold","burst","cash","camel","cargo","bread","cloth","clerk",
  "adopt","camel","bear","candy","chalk","bank","alloy","boot","column"];

window.__calls = [];
/* save_settings hands back what it was given, same as the real one. a stub
   that answered with a fixed object hid the bug where pinning a game did not
   show up until you left the page */
let SETTINGS = {theme:"dark", accent:"amber", pinned:["steam:1222140"],
                favourites:[], cheats_open:true};
window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      window.__calls.push(cmd);
      switch (cmd) {
        case "settings": return SETTINGS;
        case "save_settings":
          SETTINGS = {...SETTINGS, ...args.next};
          return SETTINGS;
        case "list_games": return GAMES;
        case "game_art": return {cover:null, hero:null, logo:null};
        case "cheats": return CHEATS;
        case "set_cheat": return null;
        case "sort_options": return SORTS;
        case "shared_tables": return SHARED;
        case "install_shared": return "The Witcher 2 is ready, 23 cheats";
        case "using": return [7, false];
        case "rate_shared": return null;
        case "share_table": return "shared, it is number 9";
        case "whoami": return null;
        case "claim_name": return PHRASE;
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
        case "version": return "Version 0.1.0 for 64-bit Windows";
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

  await settle(500);

  const cards = document.querySelectorAll("#grid .card, #pinned-grid .card");
  note(cards.length >= 1, "library rendered cards (" + cards.length + ")");
  note(visible("view-library"), "library view is showing");

  if (cards.length) {
    cards[0].click();
    await settle(300);
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

    // the panel the cheats live in
    note(visible("cheat-dock"), "cheats sit in their own panel");
    note(!visible("dock-open"), "the reopen tab is hidden while the panel is open");
    document.getElementById("dock-close").click();
    await settle(300);
    note(!visible("cheat-dock"), "the panel folds away");
    note(visible("dock-open"), "and leaves a tab to bring it back");
    document.getElementById("dock-open").click();
    await settle(300);
    note(visible("cheat-dock"), "the tab brings it back");

    // pinning without having to leave the page and come back
    const pin = document.getElementById("game-pin");
    const wasPinned = pin.classList.contains("on");
    pin.click();
    await settle(300);
    note(pin.classList.contains("on") !== wasPinned, "the pin fills in straight away");
    note(pin.title.toLowerCase().includes(wasPinned ? "pin to" : "unpin"),
         "and the tooltip says what clicking it again does");

    const fav = document.getElementById("game-fav");
    fav.click();
    await settle(300);
    note(fav.classList.contains("on"), "the star fills in straight away");

    document.getElementById("game-import").click();
    await settle(250);
    note(window.__calls.includes("pick_table"), "the game page can import a .CT");

    // shared tables
    const rows = document.querySelectorAll("#shared-list .shared-row");
    note(rows.length === 2, "shared tables listed (" + rows.length + ")");
    note(document.querySelectorAll("#shared-sort option").length === 4,
         "sort options filled in");
    note(document.querySelector("#shared-list .shared-row .verified") !== null,
         "a signed name is marked as signed");
    note(document.querySelectorAll("#shared-list .shared-row.have").length === 1,
         "one is already installed");

    const blurb = document.querySelector("#shared-list .shared-facts").textContent;
    note(blurb.includes("23 cheats"), "row shows the cheat count");
    note(blurb.includes("9 up"), "row shows votes");
    note(blurb.includes("140 downloads"), "row shows downloads");
    note(blurb.includes("3.5.0.1"), "row shows the build");

    const use = [...document.querySelectorAll("#shared-list button")].find(b => !b.disabled);
    use.click();
    await settle(400);
    note(window.__calls.includes("install_shared"), "using a shared table installs it");

    document.getElementById("game-play").click();
    await settle(300);
    note(window.__calls.includes("launch_game"), "play button invokes launch_game");
  }

  const back = document.getElementById("back");
  if (back) {
    back.click();
    await settle(250);
    note(visible("view-library"), "back returns to the library");
  }

  const rail = document.querySelectorAll("#library-rail .rail-game");
  note(rail.length >= 1, "side rail rendered (" + rail.length + ")");
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
  note(document.getElementById("tables-state").textContent !== "Checking",
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
  document.getElementById("filter").value = "";
  document.getElementById("filter").dispatchEvent(new Event("input"));
  await settle(250);
  note(!visible("library-empty"), "clearing the filter brings them back");

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

  // saving the words rather than copying them by hand
  document.getElementById("claim-name").click();
  await settle(200);
  document.getElementById("name-input").value = "someoneElse";
  document.getElementById("name-go").click();
  await settle(400);
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
  note(document.getElementById("about-version").textContent.includes("0.1.0"),
       "about says which version this is");
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

  for (const item of document.querySelectorAll(".nav-item")) {
    const target = item.dataset.view;
    item.click();
    await settle(200);
    note(visible("view-" + target), "nav opens " + target);
  }

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
         "--virtual-time-budget=25000", "--dump-dom", url],
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
