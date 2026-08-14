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
    for (const el of document.querySelectorAll("[id], .shared-dock, .game-layout, .game-main, .sidebar, .content, .titlebar")) {
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

  /* under 1240 the dock stops being a side column and stacks under the main
     one, where it is meant to move down as the content above it grows. only
     the side by side layout has to hold still */
  const dockDuring = during.get("#shared");
  const mainCol = before.get(".game-main");
  const beside = mainCol && dockBefore && dockBefore.x >= mainCol.x + mainCol.w - 2;
  if (dockBefore && dockDuring) {
    note(dockBefore.x === dockDuring.x, "and does not slide sideways while it waits");
    if (beside) {
      note(dockBefore.y === dockDuring.y && dockBefore.h === dockDuring.h,
           "nor up and down, while it is a column of its own");
    } else {
      out.push("   stacked under the content at this width, so it may move down");
    }
  }

  note(shifted < 0.05, "layout shift stays small (" + shifted.toFixed(4) + ")");

  /* the flicker, measured. switching game leaves the page half drawn for a
     moment, and anything that changes height up top drags everything under it
     down and then back. the panels are allowed to grow, the things above them
     are not allowed to move */
  const steady = ["#game-detail", "#attach-note", "#table-picker", "#cheats-panel",
                  "#table-credit", "#table-fit", "#cheat-filter"];
  const jumped = [];
  for (const name of steady) {
    const a = before.get(name), b = during.get(name);
    if (!a || !b) continue;
    if (Math.abs(b.y - a.y) > 2 || Math.abs(b.x - a.x) > 2) {
      jumped.push(`${name} dx${b.x - a.x} dy${b.y - a.y}`);
    }
  }
  note(!jumped.length,
       "nothing above the cheat list moves while the next game loads" +
       (jumped.length ? " (" + jumped.join(", ") + ")" : ""));

  /* the switch that actually flickers, and the one this file was not doing:
     a game with cheats to one with none and back. the panel is swapped for
     the empty state, and if anything above them is sized by what is below it
     the whole page walks up and down while you watch */
  out.push("");
  out.push("GOING BETWEEN A GAME WITH CHEATS AND ONE WITHOUT");
  const raider = [...rail].find(r => r.textContent.includes("Tomb Raider"));
  const walked = [];
  if (raider) {
    const above = ["#game-hero", ".game-hero", "#game-detail", "#attach-note",
                   "#table-picker", "#back"];
    for (const [label, to] of [["to one with none", raider], ["and back", witcher]]) {
      witcher.click();
      await settle(900);
      const was = watched();
      to.click();
      await settle(160);
      const mid = watched();
      await settle(900);
      const then = watched();

      for (const name of above) {
        for (const [when, now] of [["mid", mid], ["after", then]]) {
          const a = was.get(name), b = now.get(name);
          if (!a || !b) continue;
          if (Math.abs(b.y - a.y) > 2 || Math.abs(b.x - a.x) > 2) {
            walked.push(`${label} ${name} ${when} dx${b.x - a.x} dy${b.y - a.y}`);
          }
        }
      }
      // and the page must not scroll itself while the answer lands
      out.push(`   ${label}: scrolled ${Math.round(document.querySelector(".content").scrollTop)}`);
    }
  }
  for (const w of walked) out.push("   " + w);
  if (!walked.length) out.push("   nothing above the panels moves");
  note(!walked.length,
       "nor when the cheats panel is swapped for the empty state" +
       (walked.length ? " (" + walked.slice(0, 4).join(", ") + ")" : ""));

  /* the other direction, and it is not a rectangle you can measure. an epic
     app id is 97 characters in a 241px box: the box stays put and the text
     paints straight over whatever is to the right of it, so the only tell is
     content wider than its own box with nothing clipping it */
  out.push("");
  out.push("WHAT HANGS OFF THE RIGHT");
  const spilling = [];
  for (const name of ["library", "game", "finder", "settings", "about"]) {
    const nav = document.querySelector(`.nav-item[data-view="${name}"]`);
    if (nav) nav.click();
    if (name === "game") {
      const raider = [...document.querySelectorAll("#library-rail .rail-game")]
        .find(r => r.textContent.includes("Tomb Raider"));
      (raider || witcher).click();
    }
    await settle(300);

    const view = document.getElementById("view-" + name);
    if (view.hidden) continue;
    for (const el of view.querySelectorAll("*")) {
      if (el.offsetParent === null) continue;
      const over = el.scrollWidth - el.clientWidth;
      if (over <= 2 || !el.clientWidth) continue;
      const style = getComputedStyle(el);
      if (style.overflowX !== "visible") continue;
      const who = el.id ? "#" + el.id : el.tagName.toLowerCase() + "." + String(el.className).split(" ")[0];
      spilling.push(`${name} ${who} +${over}px`);
    }
  }
  for (const s of spilling) out.push("   " + s);
  if (!spilling.length) out.push("   nothing");
  note(!spilling.length,
       "no text runs out of the box it is in" +
       (spilling.length ? " (" + spilling.slice(0, 4).join(", ") + ")" : ""));

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
            [browser, "--headless", "--disable-gpu",
             "--user-data-dir=" + os.path.join(work, "profile"), "--dump-dom",
             "file:///" + page.replace("\\", "/")],
            capture_output=True, text=True, timeout=60)
        return 'id="ok"' in out.stdout
    except Exception:
        return False
    finally:
        shutil.rmtree(work, ignore_errors=True)


def once(browser, work, size):
    """the whole probe at one window size"""
    page_path = os.path.join(work, "index.html")
    url = "file:///" + page_path.replace("\\", "/")
    run = subprocess.run(
        [browser, "--headless", "--disable-gpu", "--allow-file-access-from-files",
         "--user-data-dir=" + tempfile.mkdtemp(prefix="freeplay-profile-"),
         "--window-size=%d,%d" % size, # the pair sweep is twenty switches, and running out of budget
         # makes the browser exit 0 with half a page and no error
         "--virtual-time-budget=300000", "--dump-dom", url],
        capture_output=True, text=True, timeout=180)

    found = re.search(r'<pre id="probe-results">(.*?)</pre>', run.stdout, re.S)
    if not found:
        return None
    return found.group(1).strip().replace("&amp;", "&").replace("&lt;", "<")


# the window is resizable and people make it small. everything used to be
# measured at 1600 wide, where the row of buttons under the game name fits and
# at 1280 it runs off the side of the card
SIZES = [(1600, 1000), (1280, 860), (1024, 800)]


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

    passed = failed = 0
    for size in SIZES:
        body = once(browser, work, size)
        print("=" * 20 + " %dx%d" % size)
        if body is None:
            print("the probe never ran: app.js threw, or it outgrew the time budget")
            failed += 1
            continue
        print(body)
        checks = [l for l in body.splitlines() if l.startswith(("PASS", "FAIL"))]
        bad = [l for l in checks if l.startswith("FAIL")]
        passed += len(checks) - len(bad)
        failed += len(bad)

    print("\n%d passed, %d failed" % (passed, failed))
    shutil.rmtree(os.path.dirname(work), ignore_errors=True)
    return 1 if failed else 0


sys.exit(main())
