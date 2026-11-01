"""CLI contract tests: `health --json` and `update --from DIR / --rollback` (no server, fake systemctl, --root sandbox)."""
from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import io
import json
import os
import shutil
import socket
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import synofurb_mgmt as m  # noqa: E402
from test_mgmt import FakeRunner  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def load_bundler():
    spec = importlib.util.spec_from_file_location("make_update_bundle", os.path.join(ROOT, "tools", "make-update-bundle.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


STUB = "# stub synofurb_mgmt %s\nVERSION = %r\n"


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        fh.write(text)


def make_bundle(d, version="1.2.0", unit=True, tamper=None, extra=None, manifest=True):
    write(os.path.join(d, "synofurb_mgmt.py"), STUB % (version, version))
    write(os.path.join(d, "VERSION"), version + "\n")
    write(os.path.join(d, "README.md"), "# readme %s\n" % version)
    if unit:
        write(os.path.join(d, "synofurb-mgmt.service"), "[Service]\nExecStart=/usr/bin/python3 /opt/synofurb-mgmt/synofurb_mgmt.py # %s\n" % version)
    lines = []
    for name in sorted(os.listdir(d)):
        with open(os.path.join(d, name), "rb") as fh:
            lines.append("%s  %s" % (hashlib.sha256(fh.read()).hexdigest(), name))
    if manifest:
        write(os.path.join(d, "MANIFEST.sha256"), "\n".join(lines) + "\n")
    if tamper:
        with open(os.path.join(d, tamper), "a") as fh:
            fh.write("# evil\n")
    if extra:
        write(os.path.join(d, extra), "x")


HEALTH_OK = json.dumps({"schema": 1, "version": "1.2.0", "overall": "ok"})


class CliBase(unittest.TestCase):
    def setUp(self):
        self.t = tempfile.mkdtemp(prefix="synofurb-cli-")
        self.addCleanup(shutil.rmtree, self.t, ignore_errors=True)
        self.cfg = m.Config(root=os.path.join(self.t, "root"), port=0)
        os.makedirs(self.cfg.root)
        self.runner = FakeRunner()
        self.runner.on(["systemctl", "is-active"], "active\n")
        self.events: list[tuple[str, str, str]] = []
        self.src = os.path.join(self.t, "bundle")

    def install_old(self, version="1.0.0"):
        inst = self.cfg.p(m.INSTALL_DIR)
        write(os.path.join(inst, "synofurb_mgmt.py"), STUB % (version, version))
        write(os.path.join(inst, "VERSION"), version + "\n")
        write(self.cfg.p(m.UNIT_PATH), "[Service]\nExecStart=old\n")

    def updater(self, probe=lambda cfg: True, timeout=0.05):
        return m.Updater(self.cfg, self.runner, lambda s, st, msg: self.events.append((s, st, msg)), probe=probe, health_timeout=timeout, sleep=lambda x: None)

    def steps(self):
        return [(s, st) for s, st, _ in self.events]

    def installed(self, name="VERSION"):
        return m.read_text(os.path.join(self.cfg.p(m.INSTALL_DIR), name)).strip()

    def restarts(self):
        return sum(1 for c in self.runner.calls if c[:2] == ["systemctl", "restart"])


class HealthTests(unittest.TestCase):
    def app(self, **canned):
        r = FakeRunner()
        r.avail.add("apt-get")
        r.on(["lsblk"], json.dumps({"blockdevices": [{"name": "sda", "kname": "sda", "type": "disk", "size": 4 * 10 ** 12, "model": "WD40", "serial": "S", "rota": True, "tran": "sata", "mountpoints": [None], "pkname": None},
                                                       {"name": "loop0", "kname": "loop0", "type": "loop", "size": 1, "mountpoints": [None]}]}))
        r.on(["findmnt"], json.dumps({"filesystems": [{"target": "/", "source": "/dev/x", "fstype": "ext4", "size": 100, "used": 40, "avail": 60,
                                                       "children": [{"target": "/srv/data", "source": "/dev/y", "fstype": "xfs", "size": 1000, "used": 900, "avail": 100}, {"target": "/proc", "source": "proc", "fstype": "proc"}]}]}))
        r.on(["smartctl"], json.dumps({"smart_status": {"passed": True}, "temperature": {"current": 38}}))
        r.on(["systemctl", "is-active"], "active\n")
        r.on(["apt-get"], "Inst libc6 [1] (2 Debian-Security:13/stable-security [amd64])\nInst vim [1] (2 Debian:13/stable [amd64])\nConf libc6\n")
        return m.App(m.Config(root=tempfile.mkdtemp()), r), r

    def doc(self, mdstat=None, **kw):
        app, r = self.app()
        import tempfile as tf
        p = tf.mkdtemp()
        os.makedirs(os.path.join(p, "proc"))
        open(os.path.join(p, "proc", "mdstat"), "w").write(mdstat or "Personalities : [raid5]\nmd1 : active raid5 sdb2[1] sda2[0]\n      100 blocks super 1.2 [2/2] [UU]\n\nunused devices: <none>\n")
        app.cfg.proc_root = os.path.join(p, "proc")
        open(os.path.join(p, "proc", "meminfo"), "w").write("MemTotal: 1000 kB\nMemAvailable: 600 kB\n")
        return m.collect_health(app, **kw), r

    def test_contract_shape(self):
        d, _ = self.doc()
        self.assertEqual(set(d), {"schema", "version", "hostname", "uptime_s", "load", "mem", "os", "kernel", "root_fs", "arrays", "disks", "volumes", "services", "boot", "updates", "overall", "warnings"})
        self.assertEqual((d["schema"], d["version"]), (1, m.VERSION))
        self.assertEqual(len(d["load"]), 3)
        self.assertEqual(d["mem"], {"total": 1024000, "avail": 614400})
        self.assertEqual(set(d["arrays"][0]), {"name", "level", "state", "degraded", "sync_pct", "sync_action"})
        self.assertEqual(d["disks"], [{"dev": "/dev/sda", "model": "WD40", "size": 4 * 10 ** 12, "smart": "passed", "temp_c": 38}])
        self.assertEqual(d["volumes"], [{"mount": "/", "used_pct": 40, "fstype": "ext4"}, {"mount": "/srv/data", "used_pct": 90, "fstype": "xfs"}])
        self.assertIn("used_pct", d["root_fs"])
        self.assertEqual(d["updates"], {"apt_pending": 2, "security_pending": 1})
        self.assertEqual(set(d["boot"]), {"default", "flash_present"})
        self.assertEqual({s["name"] for s in d["services"]} >= {"synofurb-mgmt", "ssh"}, True)
        json.dumps(d)

    def test_overall_levels(self):
        d, _ = self.doc()
        self.assertEqual(d["overall"], "warn")                       # volume 90% full + a security update pending
        self.assertTrue(any("90% full" in w for w in d["warnings"]))
        self.assertTrue(any("security" in w for w in d["warnings"]))
        deg = "Personalities : [raid5]\nmd1 : active raid5 sdb2[1] sda2[0]\n      100 blocks super 1.2 [3/2] [UU_]\n\nunused devices: <none>\n"
        d, _ = self.doc(mdstat=deg)
        self.assertEqual(d["overall"], "crit")
        self.assertIn("md1 degraded", d["warnings"])
        self.assertTrue(d["arrays"][0]["degraded"])
        syncing = "Personalities : [raid5]\nmd1 : active raid5 sdb2[1] sda2[0]\n      100 blocks super 1.2 [2/2] [UU]\n      [==>...]  resync = 41.0% (1/2) finish=1.0min speed=1K/sec\n\nunused devices: <none>\n"
        d, _ = self.doc(mdstat=syncing)
        self.assertEqual((d["arrays"][0]["state"], d["arrays"][0]["sync_pct"], d["arrays"][0]["sync_action"]), ("resyncing", 41.0, "resync"))
        self.assertIn("md1 resync 41.0%", d["warnings"])

    def test_smart_failure_is_critical(self):
        app, r = self.app()
        r.on(["smartctl"], json.dumps({"smart_status": {"passed": False}}), rc=4)
        d = m.collect_health(app)
        self.assertEqual(d["disks"][0]["smart"], "failed")
        self.assertEqual(d["overall"], "crit")
        self.assertIn("sda SMART failed", d["warnings"])

    def test_broken_parts_become_null_plus_warning_and_never_raise(self):
        app, r = self.app()
        def boom(argv):
            raise RuntimeError("boom")
        r.on(["lsblk"], boom).on(["findmnt"], boom).on(["apt-get"], "", rc=100)
        d = m.collect_health(app)
        self.assertIsNone(d["disks"])
        self.assertIsNone(d["volumes"])
        self.assertIsNone(d["updates"])
        self.assertTrue({"disks unavailable", "volumes unavailable", "updates unavailable"} <= set(d["warnings"]))
        self.assertIn(d["overall"], ("warn", "crit"))

    def test_timeouts_do_not_hang_and_are_unknown(self):
        app, r = self.app()
        r.on(["smartctl"], "", rc=124, err="timed out").on(["apt-get"], "", rc=124)
        d = m.collect_health(app)
        self.assertEqual(d["disks"][0]["smart"], "unknown")
        self.assertIsNone(d["disks"][0]["temp_c"])
        self.assertIsNone(d["updates"])

    def test_quick_skips_slow_probes(self):
        app, r = self.app()
        d = m.collect_health(app, quick=True)
        self.assertFalse(r.called("smartctl"))
        self.assertFalse(r.called("apt-get"))
        self.assertIsNone(d["updates"])
        self.assertEqual(d["disks"][0]["smart"], "unknown")

    def test_exhausted_budget_skips_remaining_smart_calls(self):
        app, r = self.app()
        d = m.collect_health(app, budget_s=-1)
        self.assertFalse(r.called("smartctl"))
        self.assertEqual(d["disks"][0]["smart"], "unknown")

    def test_command_line_prints_one_json_object_and_exits_zero(self):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = m.main(["health", "--json", "--dry-run", "--root", tempfile.mkdtemp()])
        self.assertEqual(rc, 0)
        lines = out.getvalue().strip().splitlines()
        self.assertEqual(len(lines), 1)
        d = json.loads(lines[0])
        self.assertEqual(d["schema"], 1)
        self.assertIn(d["overall"], ("ok", "warn", "crit"))
        self.assertEqual(m.main(["health", "--json", "--quick", "--dry-run"]), 0)


class VerifyTests(CliBase):
    def test_good_bundle(self):
        make_bundle(self.src)
        files, v = m.verify_bundle(self.src)
        self.assertEqual(v, "1.2.0")
        self.assertEqual(set(files), {"synofurb_mgmt.py", "VERSION", "README.md", "synofurb-mgmt.service"})

    def test_bundle_without_unit_is_fine(self):
        make_bundle(self.src, unit=False)
        self.assertEqual(m.verify_bundle(self.src)[1], "1.2.0")

    def test_every_file_is_checked_against_the_manifest(self):
        for name in ("synofurb_mgmt.py", "VERSION", "README.md", "synofurb-mgmt.service"):
            d = os.path.join(self.t, "b-" + name)
            make_bundle(d, tamper=name)
            with self.assertRaises(m.ValidationError, msg=name) as cm:
                m.verify_bundle(d)
            self.assertIn("checksum mismatch", str(cm.exception))

    def test_manifest_problems(self):
        d = os.path.join(self.t, "nomani")
        make_bundle(d, manifest=False)
        with self.assertRaisesRegex(m.ValidationError, "MANIFEST"):
            m.verify_bundle(d)
        d = os.path.join(self.t, "extra")
        make_bundle(d, extra="evil.py")
        with self.assertRaisesRegex(m.ValidationError, "differ"):
            m.verify_bundle(d)
        d = os.path.join(self.t, "missing")
        make_bundle(d)
        os.unlink(os.path.join(d, "README.md"))
        with self.assertRaisesRegex(m.ValidationError, "differ"):
            m.verify_bundle(d)
        d = os.path.join(self.t, "traversal")
        make_bundle(d)
        write(os.path.join(d, "MANIFEST.sha256"), "%s  ../etc/passwd\n" % ("0" * 64))
        with self.assertRaises(m.ValidationError):
            m.verify_bundle(d)
        d = os.path.join(self.t, "dup")
        make_bundle(d)
        line = m.read_text(os.path.join(d, "MANIFEST.sha256")).splitlines()[0]
        write(os.path.join(d, "MANIFEST.sha256"), line + "\n" + line + "\n")
        with self.assertRaises(m.ValidationError):
            m.verify_bundle(d)

    def test_symlink_and_url_and_syntax_refused(self):
        d = os.path.join(self.t, "link")
        make_bundle(d)
        os.unlink(os.path.join(d, "README.md"))
        os.symlink("/etc/hostname", os.path.join(d, "README.md"))
        with self.assertRaisesRegex(m.ValidationError, "regular file"):
            m.verify_bundle(d)
        with self.assertRaisesRegex(m.ValidationError, "local directory"):
            m.verify_bundle("https://example.org/update")
        d = os.path.join(self.t, "syn")
        make_bundle(d)
        write(os.path.join(d, "synofurb_mgmt.py"), "def broken(:\n")
        make_manifest = lambda: write(os.path.join(d, "MANIFEST.sha256"), "".join("%s  %s\n" % (hashlib.sha256(open(os.path.join(d, n), "rb").read()).hexdigest(), n) for n in sorted(os.listdir(d)) if n != "MANIFEST.sha256"))  # noqa: E731
        make_manifest()
        with self.assertRaisesRegex(m.ValidationError, "compile"):
            m.verify_bundle(d)

    def test_bad_version(self):
        d = os.path.join(self.t, "ver")
        make_bundle(d, version="banana")
        with self.assertRaises(m.ValidationError):
            m.verify_bundle(d)
        self.assertEqual(m.parse_version("0.10.2"), (0, 10, 2))
        self.assertLess(m.parse_version("0.9.9"), m.parse_version("0.10.0"))


class UpdateTests(CliBase):
    def test_success_swaps_keeps_previous_and_restarts(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0")
        self.runner.on([sys.executable], HEALTH_OK)
        rc = self.updater().update(self.src)
        self.assertEqual(rc, 0, self.events)
        self.assertEqual(self.installed(), "1.2.0")
        self.assertIn("# stub synofurb_mgmt 1.2.0", self.installed("synofurb_mgmt.py"))
        prev = self.cfg.p(m.INSTALL_DIR + ".prev")
        self.assertEqual(m.read_text(os.path.join(prev, "VERSION")).strip(), "1.0.0")
        self.assertFalse(os.path.exists(self.cfg.p(m.INSTALL_DIR + ".new")))
        self.assertEqual(self.steps(), [("verify", "start"), ("verify", "ok"), ("stage", "start"), ("stage", "ok"), ("swap", "start"), ("swap", "ok"),
                                        ("restart", "start"), ("restart", "ok"), ("health", "start"), ("health", "ok"), ("result", "ok")])
        self.assertEqual(self.restarts(), 1)
        self.assertTrue(self.runner.called("systemctl", "daemon-reload"))
        self.assertIn("1.2.0", m.read_text(self.cfg.p(m.UNIT_PATH)))
        self.assertFalse(os.path.exists(os.path.join(self.cfg.p(m.INSTALL_DIR), "unit.backup")))
        self.assertEqual(os.stat(os.path.join(self.cfg.p(m.INSTALL_DIR), "synofurb_mgmt.py")).st_mode & 0o777, 0o644)

    def test_fresh_install_without_previous(self):
        make_bundle(self.src, "1.2.0")
        self.runner.on([sys.executable], HEALTH_OK)
        self.assertEqual(self.updater().update(self.src), 0)
        self.assertEqual(self.installed(), "1.2.0")
        self.assertFalse(os.path.exists(os.path.join(self.cfg.p(m.INSTALL_DIR + ".prev"), "synofurb_mgmt.py")))

    def test_tampered_bundle_is_refused_and_nothing_changes(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0", tamper="synofurb_mgmt.py")
        rc = self.updater().update(self.src)
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertEqual(self.installed(), "1.0.0")
        self.assertEqual(self.restarts(), 0)
        self.assertEqual(self.steps()[-1], ("verify", "fail"))
        self.assertFalse(os.path.exists(self.cfg.p(m.INSTALL_DIR + ".new")))

    def test_older_version_refused_unless_forced(self):
        self.install_old("1.5.0")
        make_bundle(self.src, "1.2.0")
        self.runner.on([sys.executable], HEALTH_OK)
        self.assertEqual(self.updater().update(self.src), m.EXIT_REFUSED)
        self.assertIn("older", self.events[-1][2])
        self.assertEqual(self.installed(), "1.5.0")
        self.assertEqual(self.restarts(), 0)
        self.events.clear()
        self.assertEqual(self.updater().update(self.src, force=True), 0, self.events)
        self.assertEqual(self.installed(), "1.2.0")

    def test_equal_version_is_allowed(self):
        self.install_old("1.2.0")
        make_bundle(self.src, "1.2.0")
        self.runner.on([sys.executable], HEALTH_OK)
        self.assertEqual(self.updater().update(self.src), 0)

    def test_failed_health_rolls_back(self):
        self.install_old("1.0.0")
        old_code = self.installed("synofurb_mgmt.py")
        make_bundle(self.src, "1.2.0")
        answers = iter([json.dumps({"schema": 1, "version": "0.0.0"})])                     # the new version never answers correctly
        self.runner.on([sys.executable], lambda argv: m.CompletedResult(0, json.dumps({"schema": 1, "version": "1.0.0"}) if "1.0.0" in m.read_text(argv[1]) else "garbage", ""))
        rc = self.updater().update(self.src)
        self.assertEqual(rc, m.EXIT_ROLLED_BACK, self.events)
        self.assertEqual(self.installed(), "1.0.0")
        self.assertEqual(self.installed("synofurb_mgmt.py"), old_code)
        self.assertEqual(m.read_text(self.cfg.p(m.UNIT_PATH)), "[Service]\nExecStart=old\n")      # the old unit file is back too
        self.assertEqual(self.restarts(), 2)                                                      # once for the update, once for the rollback
        st = self.steps()
        self.assertIn(("health", "fail"), st)
        self.assertEqual(st[-3:], [("rollback", "start"), ("rollback", "ok"), ("result", "fail")])
        self.assertEqual(m.read_text(os.path.join(self.cfg.p(m.INSTALL_DIR + ".prev"), "VERSION")).strip(), "1.2.0")

    def test_closed_port_rolls_back(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0")
        self.runner.on([sys.executable], lambda argv: m.CompletedResult(0, json.dumps({"schema": 1, "version": "1.0.0" if "1.0.0" in m.read_text(argv[1]) else "1.2.0"}), ""))
        calls = []
        def probe(cfg):
            calls.append(1)
            return "1.0.0" in m.read_text(os.path.join(cfg.p(m.INSTALL_DIR), "VERSION"))       # only the old version listens
        self.assertEqual(self.updater(probe=probe).update(self.src), m.EXIT_ROLLED_BACK)
        self.assertEqual(self.installed(), "1.0.0")
        self.assertIn("not listening", [e for e in self.events if e[:2] == ("health", "fail")][0][2])

    def test_service_that_does_not_start_rolls_back(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0")
        state = {"n": 0}
        def restart(argv):
            state["n"] += 1
            return m.CompletedResult(1 if state["n"] == 1 else 0, "", "Job failed")
        self.runner.on(["systemctl", "restart"], restart)
        self.runner.on([sys.executable], lambda argv: m.CompletedResult(0, json.dumps({"schema": 1, "version": "1.0.0"}), ""))
        self.assertEqual(self.updater().update(self.src), m.EXIT_ROLLED_BACK)
        self.assertEqual(self.installed(), "1.0.0")
        self.assertIn(("restart", "fail"), self.steps())

    def test_rollback_that_also_fails_is_reported(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0")
        self.runner.on([sys.executable], "garbage")
        self.assertEqual(self.updater().update(self.src), m.EXIT_ROLLBACK_FAILED)
        self.assertEqual(self.events[-1][:2], ("result", "fail"))

    def test_explicit_rollback_exchanges_installs(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0")
        self.runner.on([sys.executable], lambda argv: m.CompletedResult(0, json.dumps({"schema": 1, "version": m.read_text(os.path.join(os.path.dirname(argv[1]), "VERSION")).strip()}), ""))
        self.assertEqual(self.updater().update(self.src), 0)
        self.events.clear()
        self.assertEqual(self.updater().rollback(), 0, self.events)
        self.assertEqual(self.installed(), "1.0.0")
        self.assertEqual(m.read_text(self.cfg.p(m.UNIT_PATH)), "[Service]\nExecStart=old\n")
        self.assertEqual(m.read_text(os.path.join(self.cfg.p(m.INSTALL_DIR + ".prev"), "VERSION")).strip(), "1.2.0")     # roll forward is possible
        self.assertEqual(self.updater().rollback(), 0)
        self.assertEqual(self.installed(), "1.2.0")

    def test_rollback_without_previous_is_refused(self):
        self.install_old("1.0.0")
        self.assertEqual(self.updater().rollback(), m.EXIT_REFUSED)
        self.assertEqual(self.restarts(), 0)

    def test_unit_unchanged_means_no_daemon_reload(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0", unit=False)
        self.runner.on([sys.executable], HEALTH_OK)
        self.assertEqual(self.updater().update(self.src), 0)
        self.assertFalse(self.runner.called("systemctl", "daemon-reload"))
        self.assertEqual(m.read_text(self.cfg.p(m.UNIT_PATH)), "[Service]\nExecStart=old\n")

    def test_swap_dirs_exchanges_contents(self):
        a, b = os.path.join(self.t, "a"), os.path.join(self.t, "b")
        write(os.path.join(a, "x"), "A")
        write(os.path.join(b, "x"), "B")
        m.swap_dirs(a, b)
        self.assertEqual(m.read_text(os.path.join(a, "x")), "B")
        self.assertEqual(m.read_text(os.path.join(b, "x")), "A")

    def test_default_probe_sees_a_listener(self):
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        s.listen(1)
        self.addCleanup(s.close)
        self.assertTrue(m.default_probe(m.Config(port=s.getsockname()[1])))
        s2 = socket.socket()
        s2.bind(("127.0.0.1", 0))
        port = s2.getsockname()[1]
        s2.close()
        self.assertFalse(m.default_probe(m.Config(port=port)))

    def test_no_network_code_in_the_update_path(self):
        import inspect
        src = inspect.getsource(m.Updater) + inspect.getsource(m.verify_bundle)
        for bad in ("urllib", "http.client", "requests", "socket.create_connection", "urlopen"):
            self.assertNotIn(bad, src)


class CommandLineTests(CliBase):
    def run_cli(self, *argv):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            rc = m.main(list(argv))
        return rc, [json.loads(ln) for ln in out.getvalue().splitlines() if ln.startswith("{")], out.getvalue()

    def test_update_ndjson_and_exit_codes(self):
        root = self.cfg.root
        make_bundle(self.src, "1.2.0", tamper="README.md")
        rc, lines, _ = self.run_cli("update", "--from", self.src, "--root", root, "--json", "--dry-run")
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertEqual(lines[-1]["step"], "verify")
        self.assertEqual(set(lines[-1]), {"step", "status", "message"})
        self.assertEqual(lines[-1]["status"], "fail")
        rc, lines, _ = self.run_cli("update", "--json", "--root", root, "--dry-run")                  # neither --from nor --rollback
        self.assertEqual(rc, m.EXIT_REFUSED)
        rc, lines, _ = self.run_cli("update", "--rollback", "--json", "--root", root, "--dry-run")
        self.assertEqual(rc, m.EXIT_REFUSED)
        rc, _, text = self.run_cli("update", "--from", self.src, "--root", root, "--dry-run")           # human output without --json
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertIn("[verify] fail", text)

    def test_bundle_builder_roundtrip(self):
        mod = load_bundler()
        mod.build_bundle(self.src)
        files, v = m.verify_bundle(self.src)
        self.assertEqual(v, m.VERSION)
        self.assertIn("synofurb_mgmt.py", files)
        with open(os.path.join(self.src, "synofurb_mgmt.py"), "a") as fh:
            fh.write("\n# tampered\n")
        with self.assertRaises(m.ValidationError):
            m.verify_bundle(self.src)


if __name__ == "__main__":
    unittest.main()
