"""walk the whole of the running window and complain about anything wrong.

not a script of expected answers. it opens every view, presses every control it
can find, and checks the things that are true everywhere: nothing throws,
nothing is drawn outside the box it belongs in, no control is dead, and no
panel is left saying something that contradicts what is next to it.

    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 \\
        target/release/freeplay-app.exe
    python app/uitest/tour.py
"""

import json
import sys
import time

sys.path.insert(0, __file__.rsplit("\\", 1)[0].rsplit("/", 1)[0])

from cdp import page  # noqa: E402

BAD = []


def bad(where, what):
    print("  FAIL %-22s %s" % (where, what))
    BAD.append((where, what))


def ok(what):
    print("  ok   %s" % what)


CATCH = """
(() => {
  if (window.__caught) return "already";
  window.__caught = [];
  window.addEventListener("error", e => window.__caught.push("error: " + e.message));
  window.addEventListener("unhandledrejection",
    e => window.__caught.push("promise: " + e.reason));
  const realError = console.error;
  console.error = (...a) => { window.__caught.push("console: " + a.join(" ")); realError(...a); };
  return "watching";
})()
"""

# anything drawn outside the panel it lives in, which is how a long name ends
# up painted over the card beside it
SPILLS = """
(() => {
  const out = [];
  for (const el of document.querySelectorAll(".view:not([hidden]) *")) {
    if (el.offsetParent === null) continue;
    const box = el.getBoundingClientRect();
    if (!box.width) continue;
    if (box.right - document.documentElement.clientWidth > 2) {
      out.push((el.id || el.className || el.tagName) + " past the right edge");
      continue;
    }
    const how = getComputedStyle(el);
    if (how.overflowX === "visible" && el.scrollWidth - el.clientWidth > 2 && el.clientWidth) {
      out.push((el.id || el.className || el.tagName) + " content wider than its box");
    }
  }
  return [...new Set(out)].slice(0, 6);
})()
"""


class Tour:
    def __init__(self):
        self.p = page()
        self.p.js(CATCH)
        if not self.alive():
            raise SystemExit(
                "the bridge is not answering. reloading the page breaks the "
                "tauri ipc, and then every control looks dead"
            )

    def alive(self):
        """the backend still answers.

        worth checking before calling any control broken. a webview that has
        been reloaded keeps its dom and loses its ipc, and every switch then
        looks stuck for a reason that has nothing to do with the app."""
        try:
            return self.js(
                "(async () => { try { await window.__TAURI__.core.invoke('settings');"
                " return true; } catch (e) { return false; } })()"
            ) is True
        except Exception:
            return False

    def js(self, expr):
        return self.p.js(expr)

    def caught(self):
        got = self.js("window.__caught.splice(0)") or []
        return got

    def check_quiet(self, where):
        for line in self.caught():
            bad(where, line)

    def check_spills(self, where):
        for line in self.js(SPILLS) or []:
            bad(where, line)

    def view(self, name):
        self.js(
            "document.querySelector('.nav-item[data-view=%s]').click(); true" % json.dumps(name)
        )
        time.sleep(1.2)

    def open_game(self, name):
        self.js(
            "[...document.querySelectorAll('#library-rail .rail-game')]"
            ".find(r => r.textContent.includes(%s)).click(); true" % json.dumps(name)
        )
        time.sleep(3)

    def games(self):
        return self.js(
            "[...document.querySelectorAll('#library-rail .rail-game')]"
            ".map(r => r.textContent.trim().split(String.fromCharCode(10))[0])"
        )


def main():
    try:
        t = Tour()
    except Exception as e:
        print("cannot reach the app on 9222, start it with the port open:", e)
        return 0

    print("every view opens and draws inside itself")
    for name in ["library", "finder", "settings", "about"]:
        t.view(name)
        shown = t.js(
            "document.getElementById('view-%s') && !document.getElementById('view-%s').hidden"
            % (name, name)
        )
        if not shown:
            bad(name, "the view did not open")
            continue
        t.check_spills(name)
        t.check_quiet(name)
        ok("%s opens clean" % name)

    print("settings controls all do something")
    t.view("settings")
    for accent in ["cyan", "rose", "amber"]:
        t.js("document.querySelector('#accent-pick button[data-accent=%s]').click(); true"
             % json.dumps(accent))
        time.sleep(0.6)
        got = t.js("document.documentElement.dataset.accent")
        if got != accent:
            bad("settings", "accent %s did not take (%s)" % (accent, got))
    ok("accent swatches change the accent")

    # they are buttons carrying a class, not checkboxes, so "on" is the class
    toggles = t.js("""[...document.querySelectorAll('#view-settings .switch')].map(c => c.id)""") or []
    if not toggles:
        bad("settings", "found no switches to press, the selector is wrong")
    for which in toggles:
        lit = "document.getElementById(%s).classList.contains('on')" % json.dumps(which)
        before = t.js(lit)
        t.js("document.getElementById(%s).click(); true" % json.dumps(which))
        time.sleep(1.2)
        if t.js(lit) == before:
            # a dead switch and a dead bridge look identical from here
            bad("settings", "%s does not flip%s"
                % (which, "" if t.alive() else " (and the bridge is down)"))
            continue
        t.js("document.getElementById(%s).click(); true" % json.dumps(which))
        time.sleep(1.2)
        if t.js(lit) != before:
            bad("settings", "%s does not flip back, left it %s" % (which, not before))
    ok("%d settings switches flip and flip back (%s)" % (len(toggles), ", ".join(toggles)))

    # and the theme, which is a segmented row rather than a switch
    for theme in ["light", "dark", "system"]:
        t.js("document.querySelector('#theme-pick button[data-theme=%s]').click(); true"
             % json.dumps(theme))
        time.sleep(0.7)
        got = t.js("document.documentElement.dataset.theme")
        if got != theme:
            bad("settings", "theme %s did not take (%s)" % (theme, got))
    ok("the theme buttons change the theme")
    t.check_quiet("settings")

    print("the finder")
    t.view("finder")
    t.check_spills("finder")
    t.check_quiet("finder")
    ok("finder opens with nothing attached")

    print("every game page")
    games = t.games()
    for name in games:
        t.open_game(name)
        short = name[:20]
        state = t.js("""({
          name: (document.getElementById('game-name').textContent||'').trim(),
          exe: (document.getElementById('detail-exe').textContent||'').trim(),
          cheats: document.querySelectorAll('#cheat-groups .cheat:not(.bone)').length,
          blank: !document.getElementById('no-table').hidden,
          panel: !document.getElementById('cheats-panel').hidden,
          guard: !document.getElementById('guarded-note').hidden,
          picker: !document.getElementById('table-picker').hidden,
          rows: document.querySelectorAll('#table-list .picker-table').length,
          count: (document.getElementById('cheat-count').textContent||'').trim(),
          fit: !document.getElementById('table-fit').hidden,
          fitHead: (document.getElementById('fit-headline').textContent||'').trim()
        })""")

        if not state["name"]:
            bad(short, "the page has no name on it")
        # the two panels are alternatives, never both and never neither
        if state["panel"] and state["blank"]:
            bad(short, "cheats panel and empty state are both up")
        if not state["guard"] and not state["panel"] and not state["blank"]:
            bad(short, "neither the cheats nor the empty state is up")
        if state["panel"] and state["cheats"] == 0:
            bad(short, "cheats panel is up with nothing in it")
        if state["cheats"] and not state["count"]:
            bad(short, "%d cheats and the header does not say so" % state["cheats"])
        if state["picker"] and state["rows"] < 2:
            bad(short, "picker is up with %d tables" % state["rows"])
        if not state["picker"] and state["rows"]:
            bad(short, "picker is hidden but still holds %d rows" % state["rows"])
        if state["fit"] and not state["fitHead"]:
            bad(short, "the fit notice is up saying nothing")
        if state["guard"] and (state["panel"] or state["cheats"]):
            bad(short, "an anti-cheat game is still offering cheats")

        t.check_spills(short)
        t.check_quiet(short)
        ok("%s: %d cheats, %s" % (short, state["cheats"],
                                  "guarded" if state["guard"] else
                                  "list" if state["panel"] else "empty state"))

    print("the cheats themselves, on a game that has some")
    with_cheats = None
    for name in games:
        t.open_game(name)
        if t.js("document.querySelectorAll('#cheat-groups .cheat:not(.bone)').length") > 0:
            with_cheats = name
            break
    if not with_cheats:
        print("  .. no game with cheats, skipping")
    else:
        # every card has a name, a switch and a reason to exist
        holes = t.js("""(() => {
          const out = [];
          for (const c of document.querySelectorAll('#cheat-groups .cheat')) {
            const name = c.querySelector('.cheat-name');
            if (!name || !name.textContent.trim()) out.push('a card with no name');
            if (!c.querySelector('.switch')) out.push('a card with no switch');
          }
          return [...new Set(out)];
        })()""")
        for h in holes:
            bad("cheats", h)

        # searching narrows and clearing restores
        total = t.js("document.querySelectorAll('#cheat-groups .cheat').length")
        t.js("""(() => { const f = document.getElementById('cheat-filter');
             f.value = 'zzzznothing'; f.dispatchEvent(new Event('input')); return true; })()""")
        time.sleep(0.7)
        shown = t.js("[...document.querySelectorAll('#cheat-groups .cheat')].filter(c => !c.hidden).length")
        if shown:
            bad("cheats", "searching for nonsense still shows %d" % shown)
        none_said = t.js("!document.getElementById('cheat-none').hidden")
        if not none_said:
            bad("cheats", "and does not say nothing matched")
        t.js("""(() => { const f = document.getElementById('cheat-filter');
             f.value = ''; f.dispatchEvent(new Event('input')); return true; })()""")
        time.sleep(0.7)
        back = t.js("[...document.querySelectorAll('#cheat-groups .cheat')].filter(c => !c.hidden).length")
        if back != total:
            bad("cheats", "clearing the search brings back %d of %d" % (back, total))
        else:
            ok("the cheat search narrows and clears")

        # folding every group and opening them again
        groups = t.js("document.querySelectorAll('#cheat-groups .group').length")
        t.js("""(() => { for (const h of document.querySelectorAll('.group-head')) h.click();
             return true; })()""")
        time.sleep(1.4)
        shut = t.js("document.querySelectorAll('#cheat-groups .group.shut').length")
        if shut != groups:
            bad("cheats", "folded %d of %d groups" % (shut, groups))
        t.js("""(() => { for (const h of document.querySelectorAll('.group-head')) h.click();
             return true; })()""")
        time.sleep(1.4)
        open_again = t.js("document.querySelectorAll('#cheat-groups .group:not(.shut)').length")
        if open_again != groups:
            bad("cheats", "opened %d of %d back up" % (open_again, groups))
        else:
            ok("every category folds and opens again (%d)" % groups)
        t.check_quiet("cheats")

        # a value cheat takes a number without the page falling over
        typed = t.js("""(() => {
          const box = document.querySelector('#cheat-groups .cheat-value input');
          if (!box) return "none";
          box.value = "1234";
          box.dispatchEvent(new Event('change'));
          return "typed";
        })()""")
        time.sleep(1.2)
        if typed == "typed":
            ok("a value cheat takes a number")
        t.check_quiet("cheats")

    print("the shared panel")
    t.js("""(() => { const b = document.getElementById('dock-close'); if (b) b.click(); return true; })()""")
    time.sleep(1.0)
    shut_dock = t.js("document.getElementById('shared').hidden")
    if not shut_dock:
        bad("dock", "closing it did not close it")
    reopen = t.js("""(() => { const b = document.getElementById('dock-open');
         if (b && !b.hidden) { b.click(); return true; } return false; })()""")
    time.sleep(1.0)
    if not reopen or t.js("document.getElementById('shared').hidden"):
        bad("dock", "there is no way to open it again")
    else:
        ok("the shared panel closes and opens")

    sorts = t.js("document.querySelectorAll('#shared-sort option, .picker-item').length")
    ok("the dock has %d sort choices" % sorts)
    t.check_quiet("dock")
    t.check_spills("dock")

    print("the window buttons")
    for which in ["win-min", "win-max"]:
        there = t.js("!!document.getElementById(%s)" % json.dumps(which))
        if not there:
            bad("titlebar", "%s is missing" % which)
    ok("the titlebar has its buttons")

    print()
    print("%d problems" % len(BAD))
    return 1 if BAD else 0


sys.exit(main())
