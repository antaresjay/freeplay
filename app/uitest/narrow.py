"""sweep every page for spills and overlaps across a spread of window sizes.

layout.py measures what moves between two game pages at three widths.
this walks every page at six, looking for the two ways a small window breaks
quietly: content wider than the box that is supposed to hold it, and two
things drawn on top of each other. the second one is invisible to scrollWidth,
so sibling rectangles are compared directly.

the console header is forced into its widest state, rearm offer and all,
because the stub's default state never shows it and that is exactly how the
overlap it caused survived every other harness.

    python app/uitest/narrow.py
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
  const settle = ms => new Promise(r => setTimeout(r, ms));
  const finish = () => {
    const pre = document.createElement("pre");
    pre.id = "probe-results";
    pre.textContent = out.join(String.fromCharCode(10));
    document.body.appendChild(pre);
  };
  const name = el => el.id ? "#" + el.id :
    (el.tagName.toLowerCase() + "." + String(el.className).split(" ")[0]);

  const spills = () => {
    const bad = [];
    for (const el of document.querySelectorAll(".view:not([hidden]) *, #status-rail, #status-rail *")) {
      if (el.offsetParent === null) continue;
      const box = el.getBoundingClientRect();
      if (!box.width) continue;
      if (box.right - document.documentElement.clientWidth > 2) {
        bad.push(name(el) + " past right edge");
        continue;
      }
      const how = getComputedStyle(el);
      if (how.overflowX === "visible" && el.scrollWidth - el.clientWidth > 2 && el.clientWidth) {
        bad.push(name(el) + " wider than box");
      }
    }
    return [...new Set(bad)];
  };

  /* two static siblings drawn on top of each other. absolute and fixed
     things overlap on purpose, and so do the surfaces built on layering,
     so those are left alone */
  const overlaps = () => {
    const bad = [];
    const skip = (el) => {
      if (el instanceof SVGElement) return true;
      const cs = getComputedStyle(el);
      if (cs.position === "absolute" || cs.position === "fixed") return true;
      if (cs.display === "contents") return true;
      return !!el.closest(
        ".art, .thumb, .feature, .game-hero-art, .game-cover, .menu, .picker, " +
        ".picker-menu, .palette, .sheet, .switch, .card, .splash");
    };
    for (const el of document.querySelectorAll(".view:not([hidden]) *")) {
      if (el.offsetParent === null || skip(el)) continue;
      const a = el.getBoundingClientRect();
      if (!a.width || !a.height) continue;
      let sib = el.nextElementSibling;
      while (sib) {
        if (!(sib.offsetParent === null) && !skip(sib)) {
          const b = sib.getBoundingClientRect();
          if (b.width && b.height &&
              a.right - 3 > b.left && b.right - 3 > a.left &&
              a.bottom - 3 > b.top && b.bottom - 3 > a.top) {
            bad.push(name(el) + " overlaps " + name(sib));
          }
        }
        sib = sib.nextElementSibling;
      }
    }
    return [...new Set(bad)];
  };

  try {
    await new Promise(r => {
      const t = setInterval(() => {
        if (!document.body.classList.contains("booting")) { clearInterval(t); r(); }
      }, 100);
    });
    await settle(700);

    const report = (page) => {
      for (const s of spills()) out.push("BAD " + page + " | " + s);
      for (const o of overlaps()) out.push("BAD " + page + " | " + o);
    };

    report("library");

    const rail = () => [...document.querySelectorAll("#library-rail .rail-game")];
    for (const wanted of ["Witcher", "Tomb Raider", "GOG", "FINALS"]) {
      const row = rail().find(r => r.textContent.includes(wanted));
      if (!row) continue;
      row.click();
      await settle(900);
      const offer = document.getElementById("rearm-last");
      if (offer && offer.hidden && document.querySelectorAll("#cheat-groups .cheat").length) {
        offer.hidden = false;
        offer.textContent = "Turn on what you had (3)";
      }
      await settle(100);
      report("game:" + wanted);
    }

    for (const view of ["finder", "settings", "about"]) {
      document.querySelector(`.nav-item[data-view="${view}"]`).click();
      await settle(400);
      report(view);
    }
    out.push("DONE");
  } catch (e) {
    out.push("BAD probe threw: " + (e && e.message ? e.message : e));
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


# the window minimum is 900. the rest walk up through the sizes people
# actually leave a helper app at
SIZES = [(900, 620), (980, 700), (1060, 760), (1180, 760), (1320, 820), (1600, 1000)]


def main():
    browser = find_browser()
    if not browser:
        print("no chromium based browser found, skipping the size sweep")
        return 0

    work = os.path.join(tempfile.mkdtemp(prefix="freeplay-narrow-"), "ui")
    shutil.copytree(UI, work)
    page_path = os.path.join(work, "index.html")
    page = open(page_path, encoding="utf-8").read()
    tag = '<script src="app.js"></script>'
    open(page_path, "w", encoding="utf-8").write(page.replace(tag, STUB + tag + PROBE))
    url = "file:///" + page_path.replace("\\", "/")

    bad = 0
    for size in SIZES:
        run = subprocess.run(
            [browser, "--headless", "--disable-gpu", "--allow-file-access-from-files",
             "--user-data-dir=" + tempfile.mkdtemp(prefix="freeplay-profile-"),
             "--window-size=%d,%d" % size,
             "--virtual-time-budget=120000", "--dump-dom", url],
            capture_output=True, text=True, timeout=180)
        found = re.search(r'<pre id="probe-results">(.*?)</pre>', run.stdout, re.S)
        body = found.group(1).strip().replace("&amp;", "&") if found else "BAD the probe never ran"
        lines = [l for l in body.splitlines() if l.startswith("BAD")]
        print("== %dx%d: %s" % (size[0], size[1], "clean" if not lines else ""))
        for l in lines:
            print("   ", l)
        bad += len(lines)

    shutil.rmtree(os.path.dirname(work), ignore_errors=True)
    print("\n%d problems" % bad)
    return 1 if bad else 0


sys.exit(main())
