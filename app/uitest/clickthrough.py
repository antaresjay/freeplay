"""Click through the interface with a fake Tauri bridge behind it.

There is no bundler here and nothing type checks the front end, so the only way
to know a click still works is to do the click. This copies app/ui somewhere
temporary, replaces window.__TAURI__ with canned answers, drives the interface
in headless edge or chrome, and fails if anything throws or a view will not open.

A missing id="game-cover" once meant every game page threw before it was shown,
so clicking a game silently did nothing. Nothing in cargo test could see that.

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

window.__calls = [];
window.__TAURI__ = {
  core: {
    invoke: async (cmd) => {
      window.__calls.push(cmd);
      switch (cmd) {
        case "settings":
        case "save_settings":
          return {theme:"dark", accent:"amber", pinned:["steam:1222140"], favourites:[]};
        case "list_games": return GAMES;
        case "game_art": return {cover:null, hero:null, logo:null};
        case "cheats": return [];
        case "list_processes": return [{pid:1234, name:"witcher2.exe"}];
        case "attach": return {process:"witcher2.exe", pid:1234, game:"The Witcher 2", table:false};
        default: return null;
      }
    }
  },
  window: {
    getCurrentWindow: () => ({ minimize(){}, toggleMaximize(){}, close(){} })
  }
};
</script>
"""

PROBE = r"""
<script>
(async () => {
  const out = [];
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

  for (const item of document.querySelectorAll(".nav-item")) {
    const target = item.dataset.view;
    item.click();
    await settle(200);
    note(visible("view-" + target), "nav opens " + target);
  }

  note(window.__errors.length === 0,
       "no uncaught errors" + (window.__errors.length ? ": " + window.__errors.join(" | ") : ""));

  const pre = document.createElement("pre");
  pre.id = "probe-results";
  pre.textContent = out.join("\n");
  document.body.appendChild(pre);
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
         "--virtual-time-budget=6000", "--dump-dom", url],
        capture_output=True, text=True, timeout=180)

    found = re.search(r'<pre id="probe-results">(.*?)</pre>', run.stdout, re.S)
    if not found:
        print("the probe never ran, so app.js threw before it could report")
        for line in run.stderr.splitlines():
            if "error" in line.lower():
                print("  ", line.strip()[:200])
        return 1

    body = found.group(1).strip()
    print(body)
    failed = [line for line in body.splitlines() if line.startswith("FAIL")]
    print("\n%d passed, %d failed" % (len(body.splitlines()) - len(failed), len(failed)))
    shutil.rmtree(os.path.dirname(work), ignore_errors=True)
    return 1 if failed else 0


sys.exit(main())
