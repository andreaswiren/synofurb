"""Web UI contract: word budget per page, copy discipline, Lucide-only icons, embedded fonts/logo/About, and a browser audit of the demo.

The browser audit needs node + playwright + chromium and is skipped when they are missing; everything else runs on plain Python.
"""
import base64
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, ROOT)
import synofurb_mgmt as m  # noqa: E402

BUDGET = 60
# Manifest prose (add-on summaries) is text a person reads too, so the audit counts it: at most MANIFEST_SUMMARY_WORDS words per summary
# (tests/test_addons_catalog.py) and at most MANIFEST_BUDGET words of it on one page, on top of the 60 words of the page itself.
MANIFEST_BUDGET = 60
MANIFEST_SUMMARY_WORDS = 20


def words(text):
    return len([w for w in re.sub(r"\{[a-z]+\}", "X", text).split() if re.search(r"[A-Za-z]", w)])


def script_source():
    t = m._UI_TEMPLATE
    return t[t.rindex('<script nonce'):t.rindex("</script>")]


class WordBudgetTests(unittest.TestCase):
    def test_every_page_has_a_screen_and_stays_within_60_words(self):
        pages = set(re.findall(r"^PAGES\.([a-z]+) = ", script_source(), re.M))
        self.assertEqual(pages | {"login"}, set(m.COPY_SCREENS))
        shell = sum(words(m.COPY[k]["short"]) for k in m.COPY_SHELL)
        self.assertLessEqual(shell, 14, "shell")
        for page, keys in m.COPY_SCREENS.items():
            for k in keys:
                self.assertIn(k, m.COPY, "%s: %s" % (page, k))
            total = sum(words(m.COPY[k]["short"]) for k in keys)
            if page == "help":
                total += sum(words(t["title"]) for t in m.HELP_TOPICS)
            if page != "login":
                total += shell
            self.assertLessEqual(total, BUDGET, "%s has %d words" % (page, total))

    def test_pages_are_marked_in_the_source(self):
        src = script_source()
        for page in set(m.COPY_SCREENS) - {"raid"}:
            self.assertIn("/*PAGE:%s*/" % page, src, page)

    def test_no_hard_coded_prose_in_the_script(self):
        """All words a person reads live in COPY; literals with several words are class lists, selectors or commands."""
        allowed = {"JetBrains Mono", "apt install mdadm", "users, adm", "noopener noreferrer", "use strict"}
        bad = []
        for lit in re.findall(r"'((?:[^'\\\n]|\\.)*)'", script_source()):
            if lit in allowed or " " not in lit.strip():
                continue
            if len(re.findall(r"[A-Za-z]{3,}", lit)) >= 2 and (re.search(r"[A-Z]", lit) or "," in lit):
                bad.append(lit)
        self.assertEqual(bad, [])

    def test_no_hard_coded_title_or_aria_text(self):
        src = script_source()
        self.assertNotRegex(src, r"""title: '[A-Za-z]""")
        for lit in re.findall(r"""'aria-label': '([^']+)'""", src):
            self.fail("hard-coded aria-label %r" % lit)

    def test_help_topics_cover_the_web_pages(self):
        ids = {t["id"] for t in m.HELP_TOPICS}
        for need in ("safe-apply", "sign-in", "certificate", "storage", "imports", "boot", "recovery", "report", "raid-levels", "shr", "replace", "scrub"):
            self.assertIn(need, ids)
        self.assertGreaterEqual(len(ids), 12)
        used = set(re.findall(r"(?:infoBtn|helpDialog)\('([a-z-]+)'", script_source()))
        self.assertLessEqual(used, ids)


class IconTests(unittest.TestCase):
    NOT_ICONS = {"sm", "lg", "xl", "spin", "ic"}

    def used_icons(self):
        src = script_source()
        names = set()
        for mt in re.finditer(r"\bic\(", src):                      # ic('name', 'size') and ic(cond ? 'a' : 'b')
            depth, i = 1, mt.end()
            while depth and i < len(src):
                depth += {"(": 1, ")": -1}.get(src[i], 0)
                i += 1
            names |= set(re.findall(r"'([a-z0-9-]+)'", src[mt.end():i]))
        for mt in re.finditer(r"icon: ([^,}\n]+)", src):
            names |= set(re.findall(r"'([a-z0-9-]+)'", mt.group(1)))
        for mt in re.finditer(r"badge\('[a-z]+', (?:[^()]|\([^()]*\))*?, '([a-z0-9-]+)'\)", src):
            names.add(mt.group(1))
        for mt in re.finditer(r"const NAV = \[(.*?)\];", src, re.S):
            names |= set(re.findall(r"'([a-z0-9-]+)'\]", mt.group(1)))
        names |= set(re.findall(r"KIND_ICON = \{([^}]*)\}", src) and re.findall(r": '([a-z0-9-]+)'", re.search(r"KIND_ICON = \{([^}]*)\}", src).group(1)))
        return names - self.NOT_ICONS

    def test_every_icon_is_a_lucide_symbol(self):
        used = self.used_icons()
        self.assertGreater(len(used), 30)
        self.assertEqual(sorted(used - set(m._LUCIDE)), [])

    def test_no_hand_drawn_icons_in_the_template(self):
        self.assertEqual(sorted(set(m._ICON_SPRITE.split('id="i-')[1:]) and [s.split('"')[0] for s in m._ICON_SPRITE.split('id="i-')[1:] if not s.startswith("brand-") and s.split('"')[0] not in m._LUCIDE]), [])
        t = m._UI_TEMPLATE
        body = t[t.index("<body"):]
        self.assertNotIn("<path", body.replace("<!--__SPRITE__-->", ""))     # the only SVG shapes come from the generated sprite

    def test_icons_match_the_lucide_sources_when_available(self):
        d = os.environ.get("LUCIDE_DIR") or next(iter(sorted(__import__("glob").glob(os.path.expanduser("~/.cargo/registry/src/*/gpui-kit-assets-*/assets/icons")))), None)
        if not d:
            self.skipTest("Lucide sources not available")
        for name, inner in m._LUCIDE.items():
            p = os.path.join(d, name + ".svg")
            self.assertTrue(os.path.isfile(p), name)
            body = re.search(r"<svg[^>]*>(.*)</svg>", open(p, encoding="utf-8").read(), re.S).group(1)
            self.assertEqual(re.sub(r"\s+", " ", body).strip(), inner, name)

    def test_licences_are_embedded(self):
        for k in ("Inter", "JetBrains Mono", "Lucide"):
            self.assertIn(k, m._LICENSES)
        self.assertIn("SIL OPEN FONT LICENSE", m._LICENSES["Inter"].upper())
        self.assertIn("ISC", m._LICENSES["Lucide"])


class BrandAndFontTests(unittest.TestCase):
    def test_fonts_are_embedded_woff2_and_small(self):
        self.assertEqual(sorted((f, w) for f, w, _ in m._FONT_FACES), [("Inter", 400), ("Inter", 600), ("Inter", 700), ("JetBrains Mono", 400)])
        total = 0
        for _, _, b64 in m._FONT_FACES:
            raw = base64.b64decode(b64)
            self.assertEqual(raw[:4], b"wOF2")
            total += len(b64)
        self.assertLess(total, 250 * 1024)
        html = m.INDEX_HTML
        self.assertEqual(html.count("@font-face"), 4)
        self.assertIn("font/woff2;base64,", html)
        self.assertRegex(html, r"--font-ui:Inter,[^;]*sans-serif")                 # system fallbacks stay in the stack
        self.assertRegex(html, r'--font-mono:"JetBrains Mono",[^;]*monospace')

    def test_logo_favicon_and_sprite(self):
        html = m.INDEX_HTML
        self.assertIn('<link rel="icon" type="image/svg+xml" href="data:image/svg+xml;base64,', html)
        svg = base64.b64decode(m._FAVICON_B64).decode()
        self.assertTrue(svg.lstrip().startswith("<svg"))
        for k in ("mark", "mark_dark", "logo", "logo_dark"):
            self.assertIn('id="i-brand-%s"' % k, html)
        src = script_source()
        self.assertIn("const logo = (kind", src)
        self.assertGreaterEqual(src.count("logo('mark')") + src.count("logo('wordmark')"), 3)    # sidebar, login, About

    def test_csp_allows_only_data_fonts(self):
        fn = open(os.path.join(ROOT, "synofurb_mgmt.py"), encoding="utf-8").read()
        self.assertIn("font-src data:", fn)
        self.assertIn("default-src 'self'", fn)


class AboutAndNavTests(unittest.TestCase):
    def test_nav_and_about(self):
        src = script_source()
        nav = re.search(r"const NAV = \[(.*?)\];", src, re.S).group(1)
        self.assertEqual(re.findall(r"\['([a-z]+)', '", nav), ["dashboard", "storage", "raid", "mounts", "addons", "network", "users", "boot", "system", "updates", "help", "about"])
        for needle in ("PAGES.about", "about.copy", "LIC[id]", "navigator.clipboard", "popover(T('about.synodeb')", "Montserrat", "'Lucide'", "'Inter'", "'JetBrains Mono'"):
            self.assertIn(needle, src)
        self.assertIn('"Lucide"', json.dumps(m._LICENSES))

    def test_version_is_shown_in_the_ui_and_the_api(self):
        self.assertIn("synofurb-mgmt ' + S.version", script_source())


@unittest.skipUnless(shutil.which("node"), "node missing")
class BrowserAuditTests(unittest.TestCase):
    """Builds the demo, opens every page in a real browser (both units, desktop and phone) and counts the words a person reads."""

    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("build_demo", os.path.join(ROOT, "tools", "build-demo.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        cls.tmp = tempfile.TemporaryDirectory()
        path = cls.demo_path = os.path.join(cls.tmp.name, "index.html")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(mod.build())
        npm = shutil.which("npm")
        env = dict(os.environ)
        if npm:
            root = subprocess.run([npm, "root", "-g"], capture_output=True, text=True).stdout.strip()
            env["NODE_PATH"] = root + os.pathsep + env.get("NODE_PATH", "")
        r = subprocess.run(["node", os.path.join(HERE, "ui_audit.js"), "file://" + path], capture_output=True, text=True, timeout=240, env=env)
        cls.result = None
        cls.skip = None
        if r.returncode == 77 or "Cannot find module 'playwright'" in r.stderr:
            cls.skip = "playwright or chromium not available"
        else:
            assert r.returncode == 0, r.stderr
            cls.result = json.loads(r.stdout.strip().splitlines()[-1])
            cls.size = os.path.getsize(path)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def setUp(self):
        if self.skip:
            self.skipTest(self.skip)

    def test_every_page_within_60_visible_words(self):
        res = self.result
        self.assertEqual(sorted(res["pages"]), sorted(p for p in m.COPY_SCREENS if p != "login"))
        for page, r in res["pages"].items():
            self.assertLessEqual(r["n"], BUDGET, "%s: %d words: %s" % (page, r["n"], r["text"]))
        self.assertLessEqual(res["login"]["n"], BUDGET)

    def test_manifest_prose_is_counted_and_capped_separately(self):
        """Add-on summaries are not exempt from the word budget: the worst case (nothing installed, four cards) is measured with its own cap."""
        res = self.result
        worst = res["addonStates"]["nodocker"]
        self.assertGreater(worst["man"], 30, "the card summaries must be visible to the audit")
        self.assertLessEqual(worst["man"], MANIFEST_BUDGET, worst["manText"])
        self.assertLessEqual(worst["n"], BUDGET, worst["text"])
        self.assertLessEqual(res["pages"]["addons"]["man"], MANIFEST_BUDGET)
        for state, r in res["addonStates"].items():
            self.assertLessEqual(r["n"], BUDGET, "%s: %s" % (state, r["text"]))

    def test_no_stray_text_and_every_off_control_gives_its_reason(self):
        """No page state prints null, undefined, NaN, [object ...] or an unresolved {placeholder}; a control that is off says why in visible text."""
        self.assertEqual(self.result["stray"], [])
        self.assertEqual(self.result["problems"], [])

    def test_the_stray_text_check_detects_what_it_must(self):
        """The check itself: it flags the words that once leaked into the Updates card ("null"), and passes ordinary prose."""
        r = self.node("ui_lib.js", "selftest")
        if r is None:
            self.skipTest("no browser")
        res = json.loads(r.stdout.strip().splitlines()[-1])
        for k in ("null", "undef", "obj", "tpl", "attr"):
            self.assertTrue(res[k]["stray"], k)
        self.assertTrue(res["native"]["reasons"] and res["noreason"]["reasons"])
        self.assertEqual(res["clean"], {"stray": [], "reasons": []})

    def test_browser_words_agree_with_the_copy_screens(self):
        """The default-visible keys declared in COPY_SCREENS must cover what the browser shows (labels, headers, buttons)."""
        for page, r in self.result["pages"].items():
            declared = {w.lower() for k in m.COPY_SCREENS[page] + m.COPY_SHELL for w in re.findall(r"[A-Za-z][A-Za-z-]*", m.COPY[k]["short"])}
            if page == "help":
                declared |= {w.lower() for t in m.HELP_TOPICS for w in re.findall(r"[A-Za-z][A-Za-z-]*", t["title"])}
            shown = {w.lower() for t in r["text"] for tok in t.split() if not re.search(r"\d", tok) for w in re.findall(r"[A-Za-z][A-Za-z-]*", tok)}
            data_ok = {"dsm", "debian", "synology", "ssh", "ntfs", "gib", "tib", "mib", "media", "agent", "kernel", "previous", "cron", "systemd-resolved", "systemd-timesyncd", "lucide", "inter", "jetbrains", "mono", "montserrat", "synodeb"}                   # names and units that come from the system
            declared |= {w.lower() for k, e in m.COPY.items() if k.startswith(("state.", "ms.", "raid.imported", "lv.")) for w in re.findall(r"[A-Za-z][A-Za-z-]*", e["short"])}   # state tags are data
            self.assertEqual(sorted(w for w in shown - declared if len(w) > 2 and w not in data_ok and not re.search(r"\d", w))[:12], [], page)

    def test_no_errors_no_external_requests_no_overflow_icons_resolve(self):
        res = self.result
        self.assertEqual(res["errors"], [])
        self.assertEqual(res["unresolved"], [])
        self.assertEqual(res["overflow"], [])

    def node(self, script, *args, timeout=240):
        env = dict(os.environ)
        if shutil.which("npm"):
            env["NODE_PATH"] = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() + os.pathsep + env.get("NODE_PATH", "")
        r = subprocess.run(["node", os.path.join(HERE, script), *args], capture_output=True, text=True, timeout=timeout, env=env)
        if r.returncode == 77:
            return None
        self.assertEqual(r.returncode, 0, r.stderr[-2000:])
        return r

    def test_updates_page_flows(self):
        """A person checks, confirms (consent, typed commit), watches the restart, sees a rollback and a refusal; phone width does not overflow."""
        r = self.node("ui_updates.js", "file://" + self.demo_path)
        if r is None:
            self.skipTest("no browser")
        out = json.loads(r.stdout.strip().splitlines()[-1])
        f = out["flows"]
        self.assertEqual(out["errors"], [])
        self.assertEqual(out["problems"], [])                                  # no "null", no reason that only a mouse can see, no overflow, in any state
        self.assertEqual(f["reason"], {"focused": True, "text": "Check first.", "visible": True, "native": False})
        self.assertTrue(f["reasonClickIgnored"])
        self.assertIn("update to 0.4.0 failed; rolled back", f["historyReason"])      # why an update failed is text in the row
        self.assertEqual(f["dialogReason"], {"text": "Tick the box first.", "visible": True, "inView": True})
        self.assertTrue(f["warningBeforeConsent"])
        self.assertEqual(f["busyReason"], "Another job is running.")
        self.assertEqual(f["currentReason"], "Already up to date.")
        self.assertTrue(f["refusedMessageOnce"])
        self.assertEqual(f["afterDone"], {"off": True, "text": "Reload the page first."})          # the page no longer claims a job is running
        self.assertEqual(f["afterRollback"], {"off": False, "busyLine": False})
        self.assertTrue(f["initial"]["updateDisabled"])                       # nothing to install before a check
        self.assertEqual((f["checked"]["available"], f["checked"]["unsigned"]), ("0.7.0", 1))
        self.assertTrue(f["dialogDisabledUntilConsent"] and f["dialogEnabledAfterConsent"])
        self.assertTrue(f["mainNeedsTyping"] and f["mainEnabledAfterTyping"])
        self.assertTrue(f["sawRestart"])
        self.assertIn("Updated to 0.7.0", f["done"]["banner"])
        self.assertIn("Rolled back to 0.6.0", f["rolledBack"])
        self.assertNotIn("null", f["rolledBack"])
        self.assertIn("Rolled back to 0.4.0", f["rollbackDone"])
        self.assertIn("Cannot reach GitHub", f["offline"])
        self.assertTrue(f["busyDisabled"])
        self.assertIn("Update refused", f["refused"])
        self.assertIn("critical state", f["refused"])
        self.assertIn("Updated to 0.7.0", f["forced"])
        self.assertFalse(f["phoneOverflow"])

    def test_addons_page_flows(self):
        """Install with consent, the job, every status, Docker missing, details at 320 px, logs, remove; the reasons are text, never hover-only."""
        r = self.node("ui_addons.js", "file://" + self.demo_path)
        if r is None:
            self.skipTest("no browser")
        out = json.loads(r.stdout.strip().splitlines()[-1])
        f = out["flows"]
        self.assertEqual(out["errors"], [])
        self.assertEqual(out["problems"], [])
        i = f["install"]                                                         # M3: the reason sits next to the button, inside the dialog's pinned footer
        self.assertTrue(i["off"] and i["reasonVisible"] and i["buttonInView"])
        self.assertEqual(i["reason"], "Accept the privileges first.")
        self.assertTrue(f["installFocused"] and f["installClickIgnored"] and f["installOnAfterAccept"])
        self.assertEqual(len(i["helps"]), len(set(i["helps"])))                  # five Help buttons, five different names
        self.assertGreaterEqual(len(i["helps"]), 4)
        self.assertTrue(all(h.startswith("Help: ") for h in i["helps"]))
        self.assertNotIn("ro", [o for sel in i["media"] for o in sel])           # a label, not the raw value
        self.assertEqual(f["jobRunning"], {"primary": "Hide", "progressName": "Progress"})
        self.assertEqual(f["jobDone"], "Close")
        self.assertTrue(f["focusAfterJob"]["onMain"] and f["focusAfterJob"]["outline"] == "none")
        self.assertEqual(f["plexNowInstalled"], 1)
        s = f["states"]                                                          # m3: one place for an add-on that needs attention
        self.assertEqual((s["plexMentions"], s["plexCards"], s["retry"]), (1, 0, 1))
        self.assertIn("No space left", s["rowDetail"])
        self.assertEqual(s["tags"], ["Running", "Stopped", "Updating", "Needs attention"])
        for t in f["problems"]:                                                  # m2: the state tags keep their size
            self.assertLess(t["w"], 140, t)
            self.assertEqual(t["mb"], "0px", t)
        for key in ("nodocker1280", "nodocker390"):
            cards = {c["name"]: c for c in f[key]}
            self.assertFalse(cards["Docker"]["off"])
            for name in ("Portainer", "Frigate NVR", "Plex Media Server"):
                c = cards[name]
                self.assertTrue(c["off"] and not c["native"] and c["visible"] and c["focusable"], c)
                self.assertEqual(c["reason"], "Install Docker first.")
        self.assertTrue(f["nodockerFocus"])
        self.assertEqual(f["details320"]["chipsOutside"], 0)
        self.assertFalse(f["details320"]["scrolls"])
        self.assertTrue(f["details320"]["labelled"].startswith("Revision"))
        self.assertTrue(f["logs"])
        self.assertTrue(f["removeOff"]["off"] and f["removeOff"]["visible"] and f["removeOn"])
        self.assertTrue(f["installPhone"]["inView"] and not f["installPhone"]["overflow"])

    def test_screenshot_tool_writes_small_pngs(self):
        """tools/shots.js (documented in the README) produces light and dark, desktop and phone shots of one state, each below 120 KB."""
        with tempfile.TemporaryDirectory() as out:
            env = dict(os.environ)
            if shutil.which("npm"):
                env["NODE_PATH"] = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() + os.pathsep + env.get("NODE_PATH", "")
            r = subprocess.run(["node", os.path.join(ROOT, "tools", "shots.js"), "file://" + self.demo_path, out, "--only", "updates-current"], capture_output=True, text=True, timeout=240, env=env)
            if r.returncode == 77:
                self.skipTest("no browser")
            self.assertEqual(r.returncode, 0, r.stderr[-2000:] + r.stdout[-1000:])
            names = sorted(os.listdir(out))
            self.assertEqual(names, ["mgmt-demo-updates-current-dark-phone.png", "mgmt-demo-updates-current-dark.png", "mgmt-demo-updates-current-light-phone.png", "mgmt-demo-updates-current-light.png"])
            for n in names:
                self.assertLess(os.path.getsize(os.path.join(out, n)), 120 * 1024, n)

    def test_fonts_load_and_demo_is_small(self):
        self.assertEqual(sorted(self.result["fonts"]), ["Inter 400", "Inter 600", "Inter 700", "JetBrains Mono 400"])
        self.assertLess(self.size, 1024 * 1024)


if __name__ == "__main__":
    unittest.main()
