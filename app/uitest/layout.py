"""measure what actually moves when you switch games.

clickthrough.py asserts one number at a time, which is how a panel that jumps
sideways went unnoticed for three rounds. this takes the geometry of every
element before and after a switch and prints anything that moved, plus what
chrome reports as layout shift.

it prints a report and only fails on the things that must never move: the
window chrome, the sidebar, and the position and width of the shared panel.

    python app/uitest/layout.py
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
UI = os.path.join(HERE, "..", "ui")

# the fake bridge lives in clickthrough.py, so it cannot drift from this one
CLICKTHROUGH = open(os.path.join(HERE, "clickthrough.py"), encoding="utf-8").read()
STUB = re.search(r'STUB = r"""(.*?)"""', CLICKTHROUGH, re.S).group(1)

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

  // what chrome itself thinks moved, and which elements it blames
  let shifted = 0;
  const blamed = new Set();
  try {
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (entry.hadRecentInput) continue;
        shifted += entry.value;
        for (const source of entry.sources || []) {
          const el = source.node;
          if (el && el.nodeType === 1) {
            blamed.add(el.id || el.className || el.tagName.toLowerCase());
          }
        }
      }
    }).observe({ type: "layout-shift", buffered: true });
  } catch (e) {
    out.push("NOTE layout-shift is not observable here: " + e);
  }

  try {
  const note = (ok, label) => out.push((ok ? "PASS " : "FAIL ") + label);
  const settle = ms => new Promise(r => setTimeout(r, ms));

  // every element worth watching, by a name we can print
  const watched = () => {
    const found = new Map();
    for (const el of document.querySelectorAll("[id], .shared-dock, .game-layout, .sidebar, .content, .titlebar")) {
      if (el.offsetParent === null && el.tagName !== "BODY") continue;
      const name = el.id ? "#" + el.id : "." + String(el.className).split(" ")[0];
      const box = el.getBoundingClientRect();
      if (box.width === 0 && box.height === 0) continue;
      found.set(name, {
        x: Math.round(box.x), y: Math.round(box.y),
        w: Math.round(box.width), h: Math.round(box.height),
      });
    }
    return found;
  };

  const diff = (before, after) => {
    const moved = [];
    for (const [name, was] of before) {
      const now = after.get(name);
      if (!now) continue;
      const dx = now.x - was.x, dy = now.y - was.y;
      const dw = now.w - was.w, dh = now.h - was.h;
      if (dx || dy || dw || dh) moved.push({ name, dx, dy, dw, dh });
    }
    return moved;
  };

  await settle(600);

  const rail = document.querySelectorAll("#library-rail .rail-game");
  const witcher = [...rail].find(r => r.textContent.includes("Witcher"));
  const detroit = [...rail].find(r => r.textContent.includes("Detroit"));

  witcher.click();
  await settle(900);
  const before = watched();

  // mid switch, while the answer is still on its way
  detroit.click();
  await settle(150);
  const during = watched();

  await settle(900);
  const after = watched();

  const shape = m => `${m.name} dx${m.dx} dy${m.dy} dw${m.dw} dh${m.dh}`;

  out.push("MOVED WHILE WAITING");
  const waiting = diff(before, during);
  for (const m of waiting) out.push("   " + shape(m));
  if (!waiting.length) out.push("   nothing");

  out.push("MOVED ONCE IT LANDED");
  const landed = diff(before, after);
  for (const m of landed) out.push("   " + shape(m));
  if (!landed.length) out.push("   nothing");

  out.push("LAYOUT SHIFT SCORE " + shifted.toFixed(4));
  if (blamed.size) out.push("BLAMED " + [...blamed].join(", "));
  out.push("");

  // the things that must not move, whatever the game
  const fixed = ["#library-rail", ".sidebar", ".titlebar", "#grid"];
  for (const name of fixed) {
    const a = before.get(name), b = after.get(name);
    if (!a || !b) continue;
    note(a.x === b.x && a.w === b.w,
         `${name} does not move or resize sideways (dx${b.x - a.x} dw${b.w - a.w})`);
  }

  // it carries an id, so that is what it is filed under
  const dockBefore = before.get("#shared"), dockAfter = after.get("#shared");
  if (dockBefore && dockAfter) {
    note(dockBefore.x === dockAfter.x,
         "the shared panel stays in the same column (dx" + (dockAfter.x - dockBefore.x) + ")");
    note(dockBefore.w === dockAfter.w,
         "and keeps its width (dw" + (dockAfter.w - dockBefore.w) + ")");
  }

  const dockDuring = during.get("#shared");
  if (dockBefore && dockDuring) {
    note(dockBefore.x === dockDuring.x && dockBefore.y === dockDuring.y &&
         dockBefore.h === dockDuring.h,
         "and does not budge while it waits");
  }

  note(shifted < 0.05, "layout shift stays small (" + shifted.toFixed(4) + ")");

  /* how much of the window each page actually uses. settings capped itself at
     1180px and stranded everything to the right of that on a big monitor,
     which nothing here was measuring */
  out.push("");
  out.push("WHAT EACH PAGE LEAVES UNUSED ON THE RIGHT");
  const stranded = [];
  for (const name of ["library", "game", "finder", "settings", "about"]) {
    const nav = document.querySelector(`.nav-item[data-view="${name}"]`);
    if (nav) nav.click(); else witcher.click();
    if (name === "game") witcher.click();
    await settle(250);

    const view = document.getElementById("view-" + name);
    if (view.hidden) { out.push("   " + name + " did not open"); continue; }
    const edge = view.getBoundingClientRect().right;

    // the furthest right anything on the page reaches
    let reach = 0;
    for (const el of view.querySelectorAll("*")) {
      if (el.offsetParent === null) continue;
      const box = el.getBoundingClientRect();
      if (box.width > 0) reach = Math.max(reach, box.right);
    }
    const spare = Math.round(edge - reach);
    out.push(`   ${name} ${spare}px`);
    if (spare > 80) stranded.push(name + " " + spare + "px");
  }

  note(!stranded.length,
       "every page reaches the right edge of the window" +
       (stranded.length ? " (" + stranded.join(", ") + ")" : ""));

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
            [browser, "--headless", "--disable-gpu", "--dump-dom",
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
        print("no chromium based browser found, skipping the layout check")
        return 0

    work = os.path.join(tempfile.mkdtemp(prefix="freeplay-layout-"), "ui")
    shutil.copytree(UI, work)

    page_path = os.path.join(work, "index.html")
    page = open(page_path, encoding="utf-8").read()
    tag = '<script src="app.js"></script>'
    open(page_path, "w", encoding="utf-8").write(page.replace(tag, STUB + tag + PROBE))

    url = "file:///" + page_path.replace("\\", "/")
    run = subprocess.run(
        [browser, "--headless", "--disable-gpu", "--allow-file-access-from-files",
         "--window-size=1600,1000", "--virtual-time-budget=30000", "--dump-dom", url],
        capture_output=True, text=True, timeout=180)

    found = re.search(r'<pre id="probe-results">(.*?)</pre>', run.stdout, re.S)
    if not found:
        print("the probe never ran, so app.js threw before it could report")
        return 1

    body = found.group(1).strip().replace("&amp;", "&").replace("&lt;", "<")
    print(body)
    failed = [line for line in body.splitlines() if line.startswith("FAIL")]
    checks = [line for line in body.splitlines() if line.startswith(("PASS", "FAIL"))]
    print("\n%d passed, %d failed" % (len(checks) - len(failed), len(failed)))
    shutil.rmtree(os.path.dirname(work), ignore_errors=True)
    return 1 if failed else 0


sys.exit(main())
