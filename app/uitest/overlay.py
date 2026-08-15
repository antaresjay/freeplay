"""click through the overlay page with a fake tauri bridge behind it.

the overlay is a second window with its own page and its own script, so
clickthrough.py never touches it. same idea: copy app/ui somewhere temporary,
swap window.__TAURI__ for canned answers, drive it in headless edge, and fail
if anything throws or a control does nothing.

    python app/uitest/overlay.py
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

const plain = {editable:false, kind:"", value:"", current:"", choices:[], hex:false, holds:true};
let ATTACHED = {process:"witcher2.exe", pid:1234, game:"The Witcher 2",
                table:true, arch:"32-bit"};
let FOLDED = {};
let CHEATS = [
  {id:"base", name:"Get Witcher Base", category:"Misc", description:"", hint:"",
   state:"ready", reason:"", armed:false, live:false, does:"Script", ...plain},
  {id:"vitality", name:"Infinite Vitality", category:"Player", description:"", hint:"",
   state:"on", reason:"", armed:true, live:true, does:"Freeze", ...plain,
   editable:true, kind:"f32", value:"9999", current:"312", key:"F1"},
  {id:"endurance", name:"[Endurance] Health", category:"Player", description:"", hint:"",
   state:"on", reason:"", armed:true, live:true, does:"Freeze", ...plain},
  {id:"orens", name:"Orens", category:"Resources", description:"money", hint:"",
   state:"wait", reason:"load a save first", armed:true, live:false, does:"Value", ...plain,
   editable:true, kind:"i32", value:"5000", current:""},
  {id:"difficulty", name:"Difficulty", category:"Game", description:"", hint:"",
   state:"ready", reason:"", armed:false, live:false, does:"Set once", ...plain,
   editable:true, kind:"i32", value:"1", holds:false, suspect:true,
   choices:[{value:"0",label:"Easy"},{value:"1",label:"Normal"},{value:"2",label:"Hard"}]}
];

window.__calls = [];
window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      window.__calls.push(cmd);
      switch (cmd) {
        case "overlay_game": return ATTACHED;
        // real ipc serialises, so nothing on this side ever holds the same
        // object the page is holding. handing back CHEATS itself made a
        // toggle look like it flipped twice
        case "cheats": return ATTACHED ? JSON.parse(JSON.stringify(CHEATS)) : [];
        case "folded": return (FOLDED[args.exe] || []).slice();
        case "fold": {
          const held = FOLDED[args.exe] || (FOLDED[args.exe] = []);
          if (args.shut) { if (!held.includes(args.category)) held.push(args.category); }
          else FOLDED[args.exe] = held.filter(c => c !== args.category);
          window.__folded = JSON.parse(JSON.stringify(FOLDED));
          return null;
        }
        case "overlay_status":
          return {on:true, key:"Ctrl+Shift+O", showing:true, clash:null, game:"witcher2.exe", accent:"cyan"};
        case "set_cheat":
          CHEATS.find(c => c.id === args.id).armed = args.on;
          return null;
        case "set_cheat_value":
          window.__wrote = args.value;
          return args.value;
        case "hide_overlay": window.__hidden = true; return null;
        default: return null;
      }
    }
  },
  window: { getCurrentWindow: () => ({ minimize(){}, close(){} }) }
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

  note(document.getElementById("ov-game").textContent === "The Witcher 2",
       "the overlay names the game it is over");
  note(document.getElementById("ov-sub").textContent === "witcher2.exe . 32-bit",
       "and which process, without the noise ("
       + document.getElementById("ov-sub").textContent + ")");
  note(document.getElementById("ov-key").textContent === "Ctrl+Shift+O",
       "the footer reminds you of the shortcut");

  const cards = document.querySelectorAll(".ov-cheat");
  note(cards.length === 5, "every cheat is listed (" + cards.length + ")");
  note(document.querySelectorAll(".ov-group").length === 4,
       "grouped by category the same way the main window does");
  note(document.querySelectorAll(".ov-cheat.on").length === 2,
       "the ones that are running are drawn as running");
  note(document.querySelectorAll(".ov-cheat.armed").length === 3,
       "three are switched on");

  // the row is already accent coloured with a bar down the side, so saying
  // "On" underneath as well is the same thing twice
  const running = document.querySelector(".ov-cheat.on");
  note(!!running, "the cheat that is running stands out on its own");
  note(getComputedStyle(running).borderLeftColor !== "rgba(0, 0, 0, 0)",
       "with a bar down the side");
  note(getComputedStyle(running.querySelector(".ov-why")).display === "none",
       "and does not repeat itself underneath");
  note(document.querySelector(".ov-why.wait").textContent === "load a save first",
       "and one waiting says what it is waiting for");
  note(document.getElementById("ov-count").textContent === "3 of 5 cheats on",
       "the footer counts them (" + document.getElementById("ov-count").textContent + ")");

  // the key the table bound rides along, drawn as a little keycap
  const keycaps = document.querySelectorAll(".ov-kbd");
  note(keycaps.length === 1 && keycaps[0].textContent === "F1",
       "a cheat with a key shows it (" + keycaps.length + ")");

  // the crash memory reads the same over the game
  const scar = document.querySelector(".ov-why.dead");
  note(!!scar && scar.textContent === "Went down with the game last time",
       "a blamed cheat warns in the overlay too");

  // switching one on from over the game, which is the whole point
  const off = [...document.querySelectorAll(".ov-cheat")]
    .find(c => !c.classList.contains("armed"));
  off.querySelector(".switch").click();
  await settle(300);
  note(window.__calls.includes("set_cheat"), "a toggle reaches the game");
  note(off.classList.contains("armed"), "and the card follows straight away");

  // numbers, same as the main window
  const boxes = document.querySelectorAll(".ov-value input");
  note(boxes.length === 2, "cheats that take a number get a box (" + boxes.length + ")");
  note(boxes[0].value === "9999", "with what is set in it");
  note(document.querySelectorAll(".ov-value select").length === 1,
       "and a dropdown where the table lists options");
  note(document.querySelector("[data-live-for=vitality]").textContent === "312",
       "the live value shows next to the box");

  boxes[1].value = "12345";
  boxes[1].dispatchEvent(new Event("change"));
  await settle(300);
  note(window.__wrote === "12345", "typing a number sends it");

  // nothing should be rebuilt underneath you while it polls
  const held = document.querySelector(".ov-cheat");
  await settle(1400);
  note(document.querySelector(".ov-cheat") === held,
       "polling does not rebuild the list while you are using it");

  // searching
  document.getElementById("ov-filter").value = "orens";
  document.getElementById("ov-filter").dispatchEvent(new Event("input"));
  await settle(300);
  const shown = [...document.querySelectorAll(".ov-cheat")].filter(c => !c.hidden);
  note(shown.length === 1, "the search narrows it down (" + shown.length + ")");
  note(document.querySelectorAll(".ov-group:not([hidden])").length === 1,
       "and empty categories go with it");
  document.getElementById("ov-filter").value = "";
  document.getElementById("ov-filter").dispatchEvent(new Event("input"));
  await settle(300);
  note([...document.querySelectorAll(".ov-cheat")].every(c => !c.hidden),
       "clearing it brings them back");

  /* categories fold here too, into the same list the main window writes, so
     shutting one in either place shuts it in both */
  {
    const player = [...document.querySelectorAll(".ov-group")]
      .find(g => g.querySelector("h3").textContent === "Player");
    const head = player.querySelector(".ov-group-head");
    const body = player.querySelector(".ov-group-body");
    note(!player.classList.contains("shut"), "an overlay group starts open");

    head.click();
    await settle(200);
    note(player.classList.contains("shut"), "clicking its heading folds it");
    note(getComputedStyle(body).display === "none", "and its cheats go away");
    note(window.__folded && (window.__folded["witcher2.exe"] || []).includes("Player"),
         "written to the same list the main window reads");

    document.getElementById("ov-filter").value = "endurance";
    document.getElementById("ov-filter").dispatchEvent(new Event("input"));
    await settle(300);
    note(getComputedStyle(body).display !== "none",
         "searching opens a folded group so its matches can be seen");
    document.getElementById("ov-filter").value = "";
    document.getElementById("ov-filter").dispatchEvent(new Event("input"));
    await settle(300);

    head.click();
    await settle(200);
    note(!player.classList.contains("shut"), "clicking again opens it");
  }

  /* two cheats in the same category sat flush against each other. both were
     lit, so the two accent panels touched and read as one shape with a seam
     down it */
  {
    const rows = [...document.querySelectorAll(".ov-group")]
      .map(g => [...g.querySelectorAll(".ov-cheat")])
      .find(g => g.length > 1);
    const a = rows[0].getBoundingClientRect();
    const b = rows[1].getBoundingClientRect();
    const gap = Math.round(b.top - a.bottom);
    out.push("   two rows in one group are " + gap + "px apart");
    note(gap >= 2, "rows in the same category do not touch (" + gap + "px)");
  }

  /* the window is sized to the game, not to the card. with a handful of
     cheats in it the card used to stretch the full height of the game and
     leave a dark empty column down the side of the screen */
  {
    document.getElementById("ov-filter").value = "";
    document.getElementById("ov-filter").dispatchEvent(new Event("input"));
    await settle(250);
    const tall = () => document.querySelector(".ov").getBoundingClientRect().height;
    const full = tall();
    const rows = [...document.querySelectorAll(".ov-cheat")].length;
    out.push("   card " + Math.round(full) + " for " + rows +
             " cheats, window " + window.innerHeight);
    note(full < window.innerHeight - 40,
         "the card takes what it needs rather than the whole window (" +
         Math.round(full) + " of " + window.innerHeight + ")");

    // and it has to keep tracking, not just happen to be short
    document.getElementById("ov-filter").value = "vitality";
    document.getElementById("ov-filter").dispatchEvent(new Event("input"));
    await settle(250);
    const one = tall();
    out.push("   card " + Math.round(one) + " with one cheat showing");
    note(one < full - 40,
         "and shrinks when the list does (" + Math.round(one) +
         " against " + Math.round(full) + ")");
    document.getElementById("ov-filter").value = "";
    document.getElementById("ov-filter").dispatchEvent(new Event("input"));
    await settle(250);
  }

  /* its own window, its own document. it stayed amber while the rest of the
     app was blue, so the accent has to come over the bridge like everything
     else here */
  note(document.documentElement.dataset.accent === "cyan",
       "the overlay wears the accent picked in settings (" +
       document.documentElement.dataset.accent + ")");
  const lit = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  note(lit && lit !== "",
       "and that accent resolves to a colour (" + lit + ")");

  // getting out of the way
  document.getElementById("ov-close").click();
  await settle(200);
  note(window.__hidden === true, "the close button hides it");

  window.__hidden = false;
  document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true}));
  await settle(200);
  note(window.__hidden === true, "and so does escape");

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
    ]
    found = [c for c in candidates if c and os.path.exists(c)]
    return next((c for c in found if loads_a_file(c)), None)


def loads_a_file(browser):
    """edge stopped loading file:// urls in headless and says nothing about it,
    so the first candidate that can read one off disk is the one to use."""
    work = tempfile.mkdtemp(prefix="freeplay-probe-")
    page = os.path.join(work, "probe.html")
    open(page, "w", encoding="utf-8").write("<b id=ok>ok</b>")
    try:
        out = subprocess.run(
            [browser, "--headless", "--disable-gpu",
             "--user-data-dir=" + os.path.join(work, "profile"), "--dump-dom",
             "file:///" + page.replace("\\", "/")],
            capture_output=True, text=True, timeout=60)
        return 'id="ok"' in out.stdout
    except Exception:
        return False
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    browser = find_browser()
    if not browser:
        print("no chromium based browser found, skipping the overlay click through")
        return 0

    work = os.path.join(tempfile.mkdtemp(prefix="freeplay-overlay-"), "ui")
    shutil.copytree(UI, work)

    page_path = os.path.join(work, "overlay.html")
    page = open(page_path, encoding="utf-8").read()
    tag = '<script src="overlay.js"></script>'
    if tag not in page:
        print("overlay.html no longer loads overlay.js the way this harness expects")
        return 1
    open(page_path, "w", encoding="utf-8").write(page.replace(tag, STUB + tag + PROBE))

    url = "file:///" + page_path.replace("\\", "/")
    run = subprocess.run(
        [browser, "--headless", "--disable-gpu", "--allow-file-access-from-files",
         "--user-data-dir=" + tempfile.mkdtemp(prefix="freeplay-profile-"),
         # the wide layout is where the two column settings grid lives, and the
         # default headless window is narrow enough to never see it
         "--window-size=1600,1000",
         "--virtual-time-budget=20000", "--dump-dom", url],
        capture_output=True, text=True, timeout=180)

    found = re.search(r'<pre id="probe-results">(.*?)</pre>', run.stdout, re.S)
    if not found:
        print("the probe never ran: overlay.js threw, or it outgrew the time budget")
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
