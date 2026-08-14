"""count how many times the real window redraws when you switch game.

the mock harnesses could not see this. every answer the game page needs used to
be a separate call painted the moment it landed, so opening a game moved the
list up and then down while the cheats, the picker, the fit notice, the credit
and the folded categories each turned up in their own frame.

start the app with the devtools port open and run this:

    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 \\
        target/release/freeplay-app.exe
    python app/uitest/switching.py

two frames is right: where it started and where it ended up. three or more is
the page being redrawn on the way, which is what you see as a flicker.
"""

import json
import sys
import time

sys.path.insert(0, __file__.rsplit("\\", 1)[0].rsplit("/", 1)[0])

from cdp import page  # noqa: E402

# every element that sits above or around the cheat list, so a move shows up
WATCH = """
(() => {
  window.__trace = [];
  const names = ["#game-detail", "#attach-note", "#table-picker", "#table-fit",
                 "#table-credit", "#cheats-panel", "#cheat-groups", "#no-table"];
  const snap = () => {
    const row = {t: Math.round(performance.now())};
    for (const n of names) {
      const el = document.querySelector(n);
      if (!el) continue;
      const b = el.getBoundingClientRect();
      if (!b.height && !b.width) continue;
      row[n] = [Math.round(b.top), Math.round(b.height)];
    }
    row.cheats = document.querySelectorAll("#cheat-groups .cheat:not(.bone)").length;
    row.bones = document.querySelectorAll("#cheat-groups .bone").length;
    const last = window.__trace[window.__trace.length - 1];
    if (!last || JSON.stringify({...row, t: 0}) !== JSON.stringify({...last, t: 0})) {
      window.__trace.push(row);
    }
  };
  let n = 0;
  const tick = () => { snap(); if (++n < 300) requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  return true;
})()
"""


def main():
    try:
        p = page()
    except Exception as e:
        print("cannot reach the app on 9222, start it with the port open:", e)
        return 0

    games = p.js(
        "[...document.querySelectorAll('#library-rail .rail-game')]"
        ".map(r => r.textContent.trim().split(String.fromCharCode(10))[0])"
    )
    if not games:
        print("no games in the rail, nothing to switch between")
        return 0

    def click(name):
        p.js(
            "[...document.querySelectorAll('#library-rail .rail-game')]"
            ".find(r => r.textContent.includes(%s)).click(); true" % json.dumps(name)
        )

    def cheats():
        return p.js("document.querySelectorAll('#cheat-groups .cheat:not(.bone)').length")

    # only the ones that actually put a list on screen, since a game with no
    # table has nothing to redraw
    with_cheats = []
    for name in games:
        click(name)
        time.sleep(3)
        if cheats() > 0:
            with_cheats.append(name)
    print("games with cheats: " + (", ".join(with_cheats) or "none"))
    if len(with_cheats) < 2:
        print("need two of them to switch between, skipping")
        return 0

    bad = 0
    for a in with_cheats:
        for b in with_cheats:
            if a == b:
                continue
            click(a)
            time.sleep(3.5)
            p.js(WATCH)
            click(b)
            time.sleep(3.5)
            trace = p.js("window.__trace") or []
            mark = "ok  " if len(trace) <= 2 else "FAIL"
            if len(trace) > 2:
                bad += 1
            print("%s %s to %s: %d frames" % (mark, a[:22], b[:22], len(trace)))
            for row in trace:
                groups = row.get("#cheat-groups")
                print("       %s  cheats %-4s bones %-3s groups %s" % (
                    row["t"], row.get("cheats"), row.get("bones"),
                    "top%d h%d" % tuple(groups) if groups else "none"))

    print()
    print("%d switches redrew more than once" % bad)
    return 1 if bad else 0


sys.exit(main())
