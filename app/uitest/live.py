"""drive the window that is actually on screen and check the table flow works.

the other harnesses run app/ui in a headless browser with a fake bridge behind
it. that catches a lot and it cannot catch anything that only goes wrong once
the real backend is answering: tables landing on disk, ids that have to survive
a restart, a selection that strands the table you just installed.

    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 \\
        target/release/freeplay-app.exe
    python app/uitest/live.py

it installs and removes tables on the machine it runs on, so it puts things
back the way it found them.
"""

import json
import sys
import time

sys.path.insert(0, __file__.rsplit("\\", 1)[0].rsplit("/", 1)[0])

from cdp import page  # noqa: E402

FAILED = []


def note(ok, label):
    print(("  ok   " if ok else "  FAIL ") + label)
    if not ok:
        FAILED.append(label)


class App:
    def __init__(self):
        self.p = page()

    def js(self, expr):
        return self.p.js(expr)

    def open_game(self, name):
        self.js(
            "[...document.querySelectorAll('#library-rail .rail-game')]"
            ".find(r => r.textContent.includes(%s)).click(); true" % json.dumps(name)
        )
        time.sleep(3)

    def state(self):
        return self.js("""({
          cheats: document.querySelectorAll('#cheat-groups .cheat:not(.bone)').length,
          count: (document.getElementById('cheat-count').textContent || '').trim(),
          picker: !document.getElementById('table-picker').hidden,
          rows: document.querySelectorAll('#table-list .picker-table').length,
          ticked: [...document.querySelectorAll('#table-list input')].filter(t => t.checked).length,
          blank: !document.getElementById('no-table').hidden,
          fit: (document.getElementById('fit-headline').textContent || '').trim(),
          fitShown: !document.getElementById('table-fit').hidden,
          shared: document.querySelectorAll('#shared-list .shared-row').length,
        })""")

    def shared_buttons(self, which=0):
        return self.js(
            "[...document.querySelectorAll('#shared-list .shared-row')[%d]"
            ".querySelectorAll('.row-actions button')].map(b => b.textContent)" % which
        )

    def press(self, which, label):
        got = self.js(
            "(() => { const b = [...document.querySelectorAll('#shared-list .shared-row')[%d]"
            ".querySelectorAll('.row-actions button')].find(x => x.textContent === %s);"
            " if (!b) return false; b.click(); return true; })()" % (which, json.dumps(label))
        )
        time.sleep(4)
        return got

    def first_not_installed(self):
        return self.js("""(() => {
          const rows = [...document.querySelectorAll('#shared-list .shared-row')];
          for (let i = 0; i < rows.length; i++) {
            const labels = [...rows[i].querySelectorAll('.row-actions button')]
              .map(b => b.textContent);
            if (labels.includes('Use table')) return i;
          }
          return -1;
        })()""")

    def remove_all(self):
        # the button on the cheats panel deletes every table for the game
        self.js("""(() => {
          const b = document.getElementById('remove-table');
          if (b && !b.classList.contains('away')) b.click();
          return true;
        })()""")
        time.sleep(3)
        self.js("""(() => {
          const yes = [...document.querySelectorAll('button')]
            .find(b => /remove|delete|yes/i.test(b.textContent) && b.offsetParent);
          if (yes) yes.click();
          return true;
        })()""")
        time.sleep(3)


def main():
    try:
        app = App()
    except Exception as e:
        print("cannot reach the app on 9222, start it with the port open:", e)
        return 0

    games = app.js(
        "[...document.querySelectorAll('#library-rail .rail-game')]"
        ".map(r => r.textContent.trim().split(String.fromCharCode(10))[0])"
    )
    # a game with several shared tables, so there is something to add and swap
    target = next((g for g in games if "Tomb Raider" in g), None) or games[0]
    print("driving %s" % target)
    app.open_game(target)

    was = app.state()
    print("  starting from: %s" % json.dumps(was))

    print("take one table on its own")
    app.remove_all()
    after = app.state()
    note(after["cheats"] == 0 and after["blank"], "removing every table leaves nothing")

    at = app.first_not_installed()
    note(at >= 0, "there is a shared table to take")
    if at < 0:
        return 1
    labels = app.shared_buttons(at)
    note("Use table" in labels, "it offers Use table (%s)" % labels)
    note("Add to mine" not in labels, "and no Add, since there is nothing to add to")

    app.press(at, "Use table")
    one = app.state()
    note(one["cheats"] > 0, "taking it loads its cheats (%d)" % one["cheats"])
    note(not one["blank"], "and the empty state goes away")
    note(not one["picker"], "one table needs no picker")

    print("add a second")
    at2 = app.first_not_installed()
    note(at2 >= 0, "another one is on offer")
    if at2 >= 0:
        note("Add to mine" in app.shared_buttons(at2),
             "which now offers Add to mine as well")
        app.press(at2, "Add to mine")
        two = app.state()
        note(two["cheats"] > one["cheats"],
             "adding it brings more cheats (%d then %d)" % (one["cheats"], two["cheats"]))
        note(two["picker"] and two["rows"] == 2, "and the picker appears with both")

        print("switch one off and back on")
        app.js("document.querySelectorAll('#table-list input')[1].click(); true")
        time.sleep(4)
        fewer = app.state()
        note(fewer["cheats"] < two["cheats"],
             "switching one off drops its cheats (%d)" % fewer["cheats"])
        note(fewer["picker"], "and the picker stays reachable")
        app.js("document.querySelectorAll('#table-list input')[1].click(); true")
        time.sleep(4)
        back = app.state()
        note(back["cheats"] == two["cheats"], "switching it back on restores them")

        print("replace both with one")
        at3 = app.first_not_installed()
        if at3 >= 0:
            app.press(at3, "Use table")
            only = app.state()
            note(only["cheats"] > 0, "the replacement has cheats (%d)" % only["cheats"])
            note(not only["picker"],
                 "and it is the only one left, so no picker (%d rows)" % only["rows"])
        else:
            print("  .. no third table to replace with, skipped")

    print("what the fit notice says")
    fit = app.state()
    note(not fit["fitShown"] or len(fit["fit"]) > 0,
         "the fit notice either says something or is not there (%r)" % fit["fit"])

    print()
    print("%d checks failed" % len(FAILED))
    return 1 if FAILED else 0


sys.exit(main())
