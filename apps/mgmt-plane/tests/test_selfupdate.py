"""Self-update: one pipeline for the web UI, `update --from-github` and the app's push.

Everything runs against a fake release server on 127.0.0.1 (the real HttpFetcher, plain http only because of `loopback_http`) and a fake
systemd/health (FakeRunner), exactly like test_cli.py. Not covered here (no systemd, no Internet): see docs/MANAGE.md "What is verified".
"""
from __future__ import annotations

import contextlib
import functools
import gzip
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import unittest
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, HERE)
import synofurb_mgmt as m  # noqa: E402
from test_cli import CliBase, load_bundler, make_bundle, write  # noqa: E402
from test_mgmt import Client, Env, FakeRunner, HttpTestCase  # noqa: E402

REPO = m.UPDATE_REPO


def script_for(version):
    return '# stub synofurb_mgmt\nDEFAULT_VERSION = "%s"\n' % version


def bundle_files(version, unit=True, **override):
    files = {"synofurb_mgmt.py": script_for(version), "VERSION": version + "\n", "README.md": "# readme %s\n" % version}
    if unit:
        files["synofurb-mgmt.service"] = "[Service]\nExecStart=/usr/bin/python3 /opt/synofurb-mgmt/synofurb_mgmt.py # %s\n" % version
    files.update(override)
    return files


def targz(members, top=None):
    """members: {name: bytes|str|(TarInfo, bytes)}; `top` prefixes every name."""
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.PAX_FORMAT) as tf:
        for name, data in members.items():
            if isinstance(data, tuple):
                ti, data = data
                ti.name = (top + "/" if top else "") + name
            else:
                ti = tarfile.TarInfo((top + "/" if top else "") + name)
            data = data.encode() if isinstance(data, str) else data
            ti.size = len(data) if ti.type == tarfile.REGTYPE else 0
            tf.addfile(ti, io.BytesIO(data) if ti.type == tarfile.REGTYPE else None)
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0) as gz:
        gz.write(raw.getvalue())
    return out.getvalue()


def sym(target):
    ti = tarfile.TarInfo("x")
    ti.type, ti.linkname = tarfile.SYMTYPE, target
    return ti


class FakeGithub:
    """api.github.com + github.com release downloads + codeload.github.com in one loopback server."""

    def __init__(self):
        self.routes: dict[str, tuple[int, bytes, dict]] = {}
        self.releases: list[dict] = []
        self.commits: dict[str, list] = {}
        self.hits: list[str] = []
        outer = self

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                outer.hits.append(self.path)
                u = urllib.parse.urlsplit(self.path)
                if u.path == "/repos/%s/releases" % REPO:
                    status, body, hdr = 200, json.dumps(outer.releases).encode(), {}
                elif u.path == "/repos/%s/commits" % REPO:
                    ref = urllib.parse.parse_qs(u.query).get("sha", [""])[0]
                    status, body, hdr = 200, json.dumps(outer.commits.get(ref, [])).encode(), {}
                else:
                    status, body, hdr = outer.routes.get(u.path, (404, b"{}", {}))
                self.send_response(status)
                for k, v in hdr.items():
                    self.send_header(k, v)
                if "Content-Length" not in hdr:
                    self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.base = "http://127.0.0.1:%d" % self.srv.server_address[1]
        threading.Thread(target=lambda: self.srv.serve_forever(poll_interval=0.02), daemon=True).start()

    def close(self):
        self.srv.shutdown()
        self.srv.server_close()

    def fetcher(self, **kw):
        return m.HttpFetcher(loopback_http=True, timeout=5, **kw)

    def source(self, **kw):
        return m.GithubSource(self.fetcher(), api_base=self.base, dl_base=self.base, codeload_base=self.base, **kw)

    # -- releases ---------------------------------------------------------------
    def add_release(self, version, tag=None, prerelease=False, draft=False, body="", signature=False, sums=True, asset_bytes=None, sums_digest=None, members=None, extra_assets=()):
        tag = tag or "v" + version
        name = "synofurb-mgmt-%s.tar.gz" % version
        data = asset_bytes if asset_bytes is not None else targz(members if members is not None else {k: v for k, v in bundle_files(version).items()})
        digest = sums_digest or hashlib.sha256(data).hexdigest()
        base = "/%s/releases/download/%s/" % (REPO, tag)
        self.routes[base + name] = (200, data, {})
        assets = [{"name": name, "size": len(data)}] + [{"name": a, "size": 10} for a in extra_assets]
        if sums:
            self.routes[base + "SHA256SUMS"] = (200, ("%s  %s\n%s  other.bin\n" % (digest, name, "0" * 64)).encode(), {})
            assets.append({"name": "SHA256SUMS", "size": 100})
        if signature:
            self.routes[base + "SHA256SUMS.minisig"] = (200, b"untrusted comment: x\nRWSIG\n", {})
            assets.append({"name": "SHA256SUMS.minisig", "size": 100})
        self.releases.append({"tag_name": tag, "draft": draft, "prerelease": prerelease, "body": body, "published_at": "2026-05-0%dT10:00:00Z" % (len(self.releases) + 1), "assets": assets})
        return data

    # -- monorepo ---------------------------------------------------------------
    def add_main(self, version, sha=None, ref="main", messages=("Fix a thing", "Add a thing"), tree=None, top=None, tar_bytes=None, version_status=200):
        sha = sha or hashlib.sha1(version.encode()).hexdigest()
        files = {"apps/mgmt-plane/" + k: v for k, v in bundle_files(version).items()}
        files.update({"README.md": "# repo\n", "crates/x/lib.rs": "fn main(){}", "apps/mgmt-plane/tests/test_x.py": "x", "apps/mgmt-plane/addons/frigate/addon.json": "{}"})
        files.update(tree or {})
        data = tar_bytes if tar_bytes is not None else targz(files, top=top or "synofurb-" + sha)
        self.routes["/%s/tar.gz/%s" % (REPO, sha)] = (200, data, {})
        self.routes["/repos/%s/contents/%s/VERSION" % (REPO, m.UPDATE_SUBDIR)] = (version_status, (version + "\n").encode(), {})
        self.commits[ref] = [{"sha": sha, "commit": {"message": msg + "\n\nbody", "author": {"name": "A", "date": "2026-05-1%dT09:00:00Z" % i}}} for i, msg in enumerate(messages)]
        return sha, data


class FlowBase(CliBase):
    """CliBase plus a fake GitHub and a runner that answers `health --json` with the version of the code that is installed."""

    def setUp(self):
        super().setUp()
        self.gh = FakeGithub()
        self.addCleanup(self.gh.close)
        self.runner.on([sys.executable], lambda argv: m.CompletedResult(0, json.dumps({"schema": 1, "version": m.read_text(os.path.join(os.path.dirname(argv[1]), "VERSION")).strip()}), ""))

    def install_old(self, version="1.0.0"):
        inst = self.cfg.p(m.INSTALL_DIR)
        write(os.path.join(inst, "synofurb_mgmt.py"), script_for(version))
        write(os.path.join(inst, "VERSION"), version + "\n")
        write(self.cfg.p(m.UNIT_PATH), "[Service]\nExecStart=old\n")

    def flow(self, mode="github", channel="stable", ref="", rec_kw=None, spec_kw=None, runner=None, probe=lambda cfg: True, **kw):
        spec = {"mode": mode, "channel": channel, "ref": ref, "src": None, "force": False, "accept_unsigned": True, "expect": "", "restart_delay": 0}
        spec.update(spec_kw or {})
        meta = {"source": "github-" + channel if mode == "github" else "manual", "by": "tester", "via": "cli"}
        meta.update(rec_kw or {})
        rec = m.JobRecord.create(self.cfg, **meta)
        rec.echo = lambda s, st, msg: self.events.append((s, st, msg))
        with mock.patch.object(m, "GithubSource", functools.partial(m.GithubSource, api_base=self.gh.base, dl_base=self.gh.base, codeload_base=self.gh.base)):
            rc = m.run_update_flow(self.cfg, runner or self.runner, spec, rec, fetch=self.gh.fetcher(), probe=probe, health_timeout=0.05, sleep=lambda x: None, **kw)
        return rc, m.JobRecord.load(self.cfg, rec.id)

    def audit_lines(self):
        return [json.loads(ln) for ln in m.read_text(self.cfg.p(self.cfg.audit_path)).splitlines()]


# ---------------------------------------------------------------------------------------------------------------- check

class CheckTests(FlowBase):
    def check(self, channel="stable", ref=""):
        return self.gh.source().check(self.cfg, channel, ref)

    def test_stable_picks_the_highest_release_asset_and_ignores_the_rest(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.1.0", body="## Changes\n- **fix** [a](http://x) `code`\n")
        self.gh.add_release("1.3.0", prerelease=True)
        self.gh.add_release("1.4.0", draft=True)
        self.gh.add_release("1.2.0", extra_assets=("synofurb-0.2.2-linux-x86_64.tar.gz", "synofurb-mgmt-notes.tar.gz"))
        self.gh.add_release("1.5.0-rc.1")
        info = self.check()
        self.assertEqual((info["version"], info["tag"], info["asset"], info["state"], info["installed"]), ("1.2.0", "v1.2.0", "synofurb-mgmt-1.2.0.tar.gz", "available", "1.0.0"))
        self.assertEqual((info["mode"], info["blocked"], info["channel"]), ("unsigned", None, "stable"))
        old = self.gh.releases[0]
        self.gh.releases[:] = [old]
        info = self.check()
        self.assertEqual(info["notes"], "Changes\n• fix a code")

    def test_states_current_older_available(self):
        self.gh.add_release("1.0.0")
        self.install_old("1.0.0")
        self.assertEqual(self.check()["state"], "current")
        self.install_old("1.1.0")
        self.assertEqual(self.check()["state"], "older")
        self.install_old("0.9.0")
        self.assertEqual(self.check()["state"], "available")

    def test_release_without_sums_or_without_asset(self):
        with self.assertRaises(m.UpdateError) as c:
            self.check()
        self.assertEqual(c.exception.kind, "no_release")
        self.gh.add_release("1.1.0", sums=False)
        with self.assertRaises(m.UpdateError) as c:
            self.check()
        self.assertEqual(c.exception.kind, "blocked")

    def test_main_channel_reads_commits_and_version(self):
        self.install_old("1.0.0")
        sha, _ = self.gh.add_main("1.2.0", messages=("Newest change", "Older change"))
        info = self.check("main", "main")
        self.assertEqual((info["version"], info["commit"], info["ref"], info["state"], info["mode"]), ("1.2.0", sha, "main", "available", "unsigned"))
        self.assertIn("• %s Newest change (2026-05-10)" % sha[:7], info["notes"])
        self.assertIn("Older change", info["notes"])
        self.assertTrue(any("path=apps%2Fmgmt-plane" in h or "path=apps/mgmt-plane" in h for h in self.gh.hits))
        self.install_old("1.2.0")
        self.assertEqual(self.check("main", "main")["state"], "same")

    def test_refs_are_validated(self):
        for bad in ("", "..", "a..b", "-x", "a b", "x;y", "a//b", "x/", "x.lock", "$(id)", "a" * 101, None, 5):
            if bad == "":
                continue                                                       # empty means "main"
            with self.assertRaises(m.UpdateError, msg=repr(bad)):
                m.valid_ref(bad)
        for ok in ("main", "v0.2.3", "claude/synofurb-x", "release-1.2"):
            self.assertEqual(m.valid_ref(ok), ok)

    def test_unknown_ref_and_bad_version(self):
        with self.assertRaises(m.UpdateError) as c:
            self.check("main", "nope")
        self.assertEqual(c.exception.kind, "not_found")
        self.gh.add_main("1.2.0")
        self.gh.routes["/repos/%s/contents/%s/VERSION" % (REPO, m.UPDATE_SUBDIR)] = (200, b"not a version\n", {})
        with self.assertRaises(m.UpdateError) as c:
            self.check("main", "main")
        self.assertEqual(c.exception.kind, "parse")

    def test_trust_modes(self):
        cfgfile = self.cfg.p(self.cfg.config_path)
        self.assertEqual(m.update_trust(self.cfg, "stable", False), ("unsigned", None))           # the placeholder key: releases are unsigned today
        write(cfgfile, json.dumps({"update_pubkey": "RWQ" + "A" * 40}))
        self.assertEqual(m.update_trust(self.cfg, "stable", True), ("signed", None))
        self.assertEqual(m.update_trust(self.cfg, "stable", False)[0], "blocked")
        self.assertEqual(m.update_trust(self.cfg, "main", False)[0], "blocked")                  # a source tree has no signature
        write(cfgfile, json.dumps({"update_pubkey": "RWQ" + "A" * 40, "update_allow_unsigned_source": True}))
        self.assertEqual(m.update_trust(self.cfg, "main", False), ("unsigned", None))
        write(cfgfile, json.dumps({"update_pubkey": "garbage"}))
        self.assertEqual(m.update_trust(self.cfg, "stable", False), ("unsigned", None))           # an invalid key is ignored, never trusted
        install_sh = open(os.path.join(os.path.dirname(HERE), "install.sh"), encoding="utf-8").read()
        self.assertIn('MINISIGN_PUBKEY_BUILTIN="%s"' % m.UPDATE_PLACEHOLDER_KEY, install_sh)      # same placeholder as the shell installer


# ---------------------------------------------------------------------------------------------------------------- stable update

class StableUpdateTests(FlowBase):
    def test_full_update_from_a_release_asset(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0")
        rc, rec = self.flow()
        self.assertEqual(rc, 0, self.events)
        self.assertEqual(self.installed(), "1.2.0")
        self.assertEqual(self.installed("synofurb_mgmt.py"), script_for("1.2.0").strip())
        self.assertEqual(m.read_text(os.path.join(self.cfg.p(m.INSTALL_DIR + ".prev"), "VERSION")).strip(), "1.0.0")      # the previous version is kept
        self.assertEqual(self.steps()[:2], [("check", "start"), ("check", "ok")])
        self.assertIn(("download", "start"), self.steps())
        self.assertEqual([s for s in self.steps() if s[0] in ("verify", "stage", "swap", "restart", "health", "result")][-1], ("result", "ok"))
        d = rec.data
        self.assertEqual((d["state"], d["exit"], d["from"], d["to"], d["source"], d["by"], d["via"], d["unsigned"]), ("ok", 0, "1.0.0", "1.2.0", "github-stable", "tester", "cli", True))
        self.assertEqual(m.read_text(os.path.join(self.cfg.p(m.INSTALL_DIR), "MANIFEST.sha256")), "")                    # the manifest is not installed, only verified
        self.assertFalse(os.path.exists(m.JobLock(self.cfg).path))                                                      # lock released
        self.assertEqual(os.listdir(m._update_dir(self.cfg)).count("jobs"), 1)
        self.assertEqual([p for p in os.listdir(m._update_dir(self.cfg)) if p.startswith("stage-")], [])                # staging area removed
        acts = [(a["action"], a["result"]) for a in self.audit_lines()]
        self.assertEqual(acts, [("mgmt-update-start", "started"), ("mgmt-update", "ok (exit 0)")])
        last = self.audit_lines()[-1]
        self.assertEqual((last["user"], last["args"]["from"], last["args"]["to"], last["args"]["source"]), ("tester", "1.0.0", "1.2.0", "github-stable"))

    def test_unsigned_needs_consent_and_nothing_is_downloaded_without_it(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0")
        rc, rec = self.flow(spec_kw={"accept_unsigned": False})
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertEqual(self.steps()[-1], ("consent", "fail"))
        self.assertEqual(self.installed(), "1.0.0")
        self.assertFalse(any("/download/" in h for h in self.gh.hits))
        self.assertEqual(rec.data["state"], "refused")

    def test_checksum_mismatch_is_refused_and_nothing_changes(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0", sums_digest="f" * 64)
        rc, rec = self.flow()
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertIn("MISMATCH", self.events[-1][2])
        self.assertEqual(self.installed(), "1.0.0")
        self.assertFalse(os.path.exists(self.cfg.p(m.INSTALL_DIR + ".new")))
        self.assertEqual(self.restarts(), 0)

    def test_asset_not_listed_in_sums(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0")
        self.gh.routes["/%s/releases/download/v1.2.0/SHA256SUMS" % REPO] = (200, ("%s  other.bin\n" % ("0" * 64)).encode(), {})
        rc, _ = self.flow()
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertIn("not listed", self.events[-1][2])

    def test_archive_with_hostile_members_is_refused(self):
        self.install_old("1.0.0")
        good = {k: v for k, v in bundle_files("1.2.0").items()}
        cases = {"traversal": {**good, "../evil": "x"}, "extra file": {**good, "extra.py": "x"}, "absolute": {**good, "/etc/passwd": "x"},
                 "symlink": {**good, "VERSION": (sym("/etc/passwd"), b"")}, "subdir": {**good, "sub/synofurb_mgmt.py": "x"}}
        for label, members in cases.items():
            self.gh.releases.clear()
            self.gh.add_release("1.2.0", members=members)
            self.events.clear()
            rc, _ = self.flow()
            self.assertEqual(rc, m.EXIT_REFUSED, label)
            self.assertEqual(self.installed(), "1.0.0", label)
        self.assertFalse(os.path.exists(os.path.join(self.t, "evil")))

    def test_archive_whose_version_disagrees_is_refused(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0", members=bundle_files("1.2.0", VERSION="1.9.0\n"))
        rc, _ = self.flow()
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertIn("not the version that was checked", self.events[-1][2])
        self.gh.releases.clear()
        self.gh.add_release("1.2.0", members=bundle_files("1.2.0", **{"synofurb_mgmt.py": script_for("1.1.0")}))
        rc, _ = self.flow()
        self.assertIn("disagree", self.events[-1][2])

    def test_new_version_that_does_not_answer_is_rolled_back(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0")
        r = FakeRunner()
        r.on(["systemctl", "is-active"], "active\n")
        r.on([sys.executable], lambda argv: m.CompletedResult(0, json.dumps({"schema": 1, "version": "1.0.0"}) if "1.0.0" in m.read_text(argv[1]) else "garbage", ""))
        rc, rec = self.flow(runner=r)
        self.assertEqual(rc, m.EXIT_ROLLED_BACK, self.events)
        self.assertEqual(self.installed(), "1.0.0")
        self.assertEqual((rec.data["state"], rec.data["from"], rec.data["to"]), ("rolled_back", "1.0.0", "1.2.0"))      # `to` stays the version that was attempted
        self.assertEqual(self.steps()[-3:], [("rollback", "start"), ("rollback", "ok"), ("result", "fail")])
        self.assertEqual(self.audit_lines()[-1]["result"], "rolled_back (exit 3)")
        self.assertFalse(os.path.exists(m.JobLock(self.cfg).path))

    def test_current_is_a_no_op_unless_forced_and_older_needs_force(self):
        self.gh.add_release("1.2.0")
        self.install_old("1.2.0")
        rc, rec = self.flow()
        self.assertEqual((rc, rec.data["state"]), (0, "current"))
        self.assertEqual(self.steps()[-1], ("result", "ok"))
        self.assertEqual(self.restarts(), 0)
        self.install_old("1.5.0")
        self.events.clear()
        rc, _ = self.flow()
        self.assertEqual((rc, self.steps()[-1]), (m.EXIT_REFUSED, ("check", "fail")))
        self.assertEqual(self.installed(), "1.5.0")
        self.events.clear()
        rc, _ = self.flow(spec_kw={"force": True})
        self.assertEqual((rc, self.installed()), (0, "1.2.0"))

    def test_what_you_saw_is_what_you_get(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0")
        rc, _ = self.flow(spec_kw={"expect": "1.1.0"})
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertIn("check again", self.events[-1][2])
        self.assertEqual(self.installed(), "1.0.0")
        self.events.clear()
        rc, _ = self.flow(spec_kw={"expect": "1.2.0"})
        self.assertEqual(rc, 0)

    def test_signed_release_needs_minisign_and_a_good_signature(self):
        write(self.cfg.p(self.cfg.config_path), json.dumps({"update_pubkey": "RWQ" + "A" * 40}))
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0", signature=True)
        rc, _ = self.flow(spec_kw={"accept_unsigned": False})                       # not installed: refused, never downgraded to unsigned
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertIn("minisign is not installed", self.events[-1][2])
        self.runner.avail.add("minisign")
        self.runner.on(["/usr/bin/minisign"], "", rc=1)
        self.events.clear()
        rc, _ = self.flow(spec_kw={"accept_unsigned": False})
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertIn("SIGNATURE VERIFICATION FAILED", self.events[-1][2])
        self.assertEqual(self.installed(), "1.0.0")
        self.runner.on(["/usr/bin/minisign"], "ok")
        self.events.clear()
        rc, rec = self.flow(spec_kw={"accept_unsigned": False})
        self.assertEqual((rc, rec.data["unsigned"]), (0, False))
        call = [c for c in self.runner.calls if c[:1] == ["/usr/bin/minisign"]][-1]
        self.assertEqual(call[1], "-Vm")
        self.assertEqual(call[-2:], ["-P", "RWQ" + "A" * 40])

    def test_signed_mode_without_a_signature_asset_is_blocked(self):
        write(self.cfg.p(self.cfg.config_path), json.dumps({"update_pubkey": "RWQ" + "A" * 40}))
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0")
        rc, _ = self.flow()
        self.assertEqual((rc, self.installed()), (m.EXIT_REFUSED, "1.0.0"))
        self.assertIn("no SHA256SUMS.minisig", self.events[-1][2])


# ---------------------------------------------------------------------------------------------------------------- main channel

class MainUpdateTests(FlowBase):
    def test_extracts_only_the_allowed_files_of_the_subfolder(self):
        self.install_old("1.0.0")
        sha, _ = self.gh.add_main("1.2.0")
        rc, rec = self.flow(channel="main", ref="main", spec_kw={"expect": sha[:7]})
        self.assertEqual(rc, 0, self.events)
        self.assertEqual(sorted(os.listdir(self.cfg.p(m.INSTALL_DIR))), ["README.md", "VERSION", "synofurb-mgmt.service", "synofurb_mgmt.py"])
        self.assertEqual(self.installed(), "1.2.0")
        self.assertEqual((rec.data["source"], rec.data["ref"], rec.data["commit"], rec.data["unsigned"]), ("github-main", "main", sha, True))
        self.assertIn("no signature exists", [e for e in self.events if e[0] == "download" and e[1] == "ok"][0][2])
        self.assertTrue(any(h.endswith("/tar.gz/" + sha) for h in self.gh.hits))                                          # the tarball is pinned to the checked commit

    def test_consent_is_always_required_and_a_signing_key_blocks_the_channel(self):
        self.install_old("1.0.0")
        self.gh.add_main("1.2.0")
        rc, _ = self.flow(channel="main", ref="main", spec_kw={"accept_unsigned": False})
        self.assertEqual((rc, self.steps()[-1]), (m.EXIT_REFUSED, ("consent", "fail")))
        write(self.cfg.p(self.cfg.config_path), json.dumps({"update_pubkey": "RWQ" + "A" * 40}))
        self.events.clear()
        rc, _ = self.flow(channel="main", ref="main")
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertIn("signing key is configured", self.events[-1][2])
        self.assertEqual(self.installed(), "1.0.0")

    def test_same_version_is_updated_but_older_needs_force(self):
        self.install_old("1.2.0")
        self.gh.add_main("1.2.0")
        rc, rec = self.flow(channel="main", ref="main")
        self.assertEqual((rc, rec.data["state"]), (0, "ok"))                                                              # a development build: same version, new code
        self.install_old("1.4.0")
        rc, _ = self.flow(channel="main", ref="main")
        self.assertEqual(rc, m.EXIT_REFUSED)

    def test_hostile_tarballs(self):
        self.install_old("1.0.0")
        sha = "a" * 40
        good = {"apps/mgmt-plane/" + k: v for k, v in bundle_files("1.2.0").items()}
        cases = {
            "symlinked VERSION": {**good, "apps/mgmt-plane/VERSION": (sym("/etc/shadow"), b"")},
            "duplicate": None,
            "two top dirs": None,
            "version mismatch": {**good, "apps/mgmt-plane/VERSION": "9.9.9\n"},
        }
        for label, files in cases.items():
            self.gh.commits.clear()
            if label == "duplicate":
                raw = io.BytesIO()
                with tarfile.open(fileobj=raw, mode="w") as tf:
                    for n, d in (("t/apps/mgmt-plane/VERSION", b"1.2.0\n"), ("t/apps/mgmt-plane/VERSION", b"1.2.0\n")):
                        ti = tarfile.TarInfo(n)
                        ti.size = len(d)
                        tf.addfile(ti, io.BytesIO(d))
                data = gzip.compress(raw.getvalue())
                self.gh.add_main("1.2.0", sha=sha, tar_bytes=data)
            elif label == "two top dirs":
                self.gh.add_main("1.2.0", sha=sha, tar_bytes=targz({"a/apps/mgmt-plane/VERSION": "1.2.0\n", "b/apps/mgmt-plane/README.md": "x"}))
            else:
                self.gh.add_main("1.2.0", sha=sha, tar_bytes=targz(files, top="t"))
            self.events.clear()
            rc, _ = self.flow(channel="main", ref="main")
            self.assertEqual(rc, m.EXIT_REFUSED, label)
            self.assertEqual(self.installed(), "1.0.0", label)

    def test_unpacked_size_is_capped(self):
        self.install_old("1.0.0")
        self.gh.add_main("1.2.0", tree={"big.bin": "0" * (3 << 20)})
        with mock.patch.object(m, "MAX_REPO_UNPACKED", 1 << 20):
            rc, _ = self.flow(channel="main", ref="main")
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertIn("unpacks to more than", self.events[-1][2])
        self.assertEqual(self.installed(), "1.0.0")


# ---------------------------------------------------------------------------------------------------------------- add-on files in the bundle

class AddonFilesTests(FlowBase):
    ADDONS = {"apps/mgmt-plane/synofurb_addons.py": "# stub addons module\n",
              "apps/mgmt-plane/addons/frigate/addon.json": '{"id": "frigate"}', "apps/mgmt-plane/addons/frigate/compose.yml.tmpl": "services: {}\n",
              "apps/mgmt-plane/addons/docker/addon.json": '{"id": "docker"}'}

    def stage_main(self, tree):
        self.gh.add_main("1.2.0", tree=tree)
        info = self.gh.source().check(self.cfg, "main", "main")
        work = tempfile.mkdtemp(dir=self.t)
        return self.gh.source().stage(self.cfg, self.runner, info, work, lambda *a: None)

    def test_source_tree_addons_are_packed_like_make_update_bundle(self):
        d = self.stage_main(self.ADDONS)
        self.assertEqual(sorted(os.listdir(d)), sorted(["README.md", "VERSION", "synofurb_mgmt.py", "synofurb-mgmt.service", "synofurb_addons.py", "addons.pack.json", m.MANIFEST_NAME]))
        ref = os.path.join(self.t, "ref")
        for k, v in bundle_files("1.2.0").items():
            write(os.path.join(ref, k), v)
        for k, v in self.ADDONS.items():
            write(os.path.join(ref, k[len("apps/mgmt-plane/"):]), v)
        load_bundler().build_bundle(os.path.join(self.t, "tool"), root=ref)
        for name in os.listdir(d):
            self.assertEqual(m.read_text(os.path.join(d, name)), m.read_text(os.path.join(self.t, "tool", name)), name)       # files AND manifest are identical
        pack = json.loads(m.read_text(os.path.join(d, "addons.pack.json")))
        self.assertEqual(sorted(pack["addons"]), ["docker", "frigate"])
        m.verify_bundle(d)

    def test_a_literal_pack_in_the_tree_is_never_trusted(self):
        d = self.stage_main({**self.ADDONS, "apps/mgmt-plane/addons.pack.json": '{"schema": 1, "addons": {"evil": {}}}'})
        self.assertNotIn("evil", m.read_text(os.path.join(d, "addons.pack.json")))

    def test_hostile_addon_entries_are_refused(self):
        self.install_old("1.0.0")
        bad = {"traversal": {"apps/mgmt-plane/addons/../evil": "x"}, "dot file": {"apps/mgmt-plane/addons/frigate/.hidden": "x"},
               "bad id": {"apps/mgmt-plane/addons/Fri Gate/addon.json": "x"}, "symlink": {"apps/mgmt-plane/addons/frigate/addon.json": (sym("/etc/passwd"), b"")},
               "too big": {"apps/mgmt-plane/addons/frigate/big.tmpl": "x" * (m.MAX_ADDON_FILE + 1)}}
        for label, extra in bad.items():
            self.gh.commits.clear()
            self.gh.add_main("1.2.0", tree={**self.ADDONS, **extra})
            self.events.clear()
            rc, _ = self.flow(channel="main", ref="main")
            self.assertEqual(rc, m.EXIT_REFUSED, label)
            self.assertEqual(self.installed(), "1.0.0", label)
        with mock.patch.object(m, "MAX_ADDON_FILES", 2):
            self.gh.commits.clear()
            self.gh.add_main("1.2.0", tree=self.ADDONS)
            rc, _ = self.flow(channel="main", ref="main")
            self.assertEqual(rc, m.EXIT_REFUSED)

    def test_release_asset_with_the_addon_files_installs_them(self):
        self.install_old("1.0.0")
        files = bundle_files("1.2.0", **{"synofurb_addons.py": "# addons\n", "addons.pack.json": '{"schema": 1, "addons": {}}\n'})
        self.gh.add_release("1.2.0", members=files)
        rc, _ = self.flow()
        self.assertEqual(rc, 0, self.events)
        self.assertEqual(sorted(os.listdir(self.cfg.p(m.INSTALL_DIR))), ["README.md", "VERSION", "addons.pack.json", "synofurb-mgmt.service", "synofurb_addons.py", "synofurb_mgmt.py"])


class RealTreeTests(FlowBase):
    """The real apps/mgmt-plane of this repository, packed the two ways GitHub serves it, goes through the same staging."""
    ROOT = os.path.dirname(HERE)

    def test_release_asset_built_by_package_mgmt_sh(self):
        import shutil as sh
        if not sh.which("bash") or not sh.which("sed"):
            self.skipTest("bash missing")
        out = os.path.join(self.t, "rel")
        r = subprocess.run(["bash", os.path.join(os.path.dirname(os.path.dirname(self.ROOT)), "scripts", "package-mgmt.sh"), "--out", out], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.gh.add_release(m.VERSION, asset_bytes=open(os.path.join(out, "synofurb-mgmt-%s.tar.gz" % m.VERSION), "rb").read())
        self.install_old("0.0.1")
        rc, _ = self.flow()
        self.assertEqual(rc, 0, self.events)
        self.assertEqual(self.installed(), m.VERSION)
        self.assertEqual(self.installed("synofurb_mgmt.py"), m.read_text(os.path.join(self.ROOT, "synofurb_mgmt.py")).strip())

    def test_monorepo_tarball_of_the_real_tree(self):
        tree = {}
        for dp, dn, fn in os.walk(self.ROOT):
            dn[:] = [d for d in dn if d not in ("__pycache__", ".git")]
            for f in fn:
                full = os.path.join(dp, f)
                tree["apps/mgmt-plane/" + os.path.relpath(full, self.ROOT).replace(os.sep, "/")] = open(full, "rb").read()
        sha = "b" * 40
        self.gh.routes["/%s/tar.gz/%s" % (REPO, sha)] = (200, targz(tree, top="synofurb-" + sha), {})
        self.gh.routes["/repos/%s/contents/%s/VERSION" % (REPO, m.UPDATE_SUBDIR)] = (200, (m.VERSION + "\n").encode(), {})
        self.gh.commits["main"] = [{"sha": sha, "commit": {"message": "x", "author": {"date": "2026-01-01T00:00:00Z"}}}]
        self.install_old("0.0.1")
        info = self.gh.source().check(self.cfg, "main", "main")
        work = tempfile.mkdtemp(dir=self.t)
        d = self.gh.source().stage(self.cfg, self.runner, info, work, lambda *a: None)
        load_bundler().build_bundle(os.path.join(self.t, "tool"), root=self.ROOT)
        self.assertEqual(sorted(os.listdir(d)), sorted(os.listdir(os.path.join(self.t, "tool"))))
        for name in os.listdir(d):
            self.assertEqual(m.read_text(os.path.join(d, name)), m.read_text(os.path.join(self.t, "tool", name)), name)


# ---------------------------------------------------------------------------------------------------------------- safety

class SafetyTests(FlowBase):
    def test_unhealthy_unit_refuses_a_github_update_unless_forced(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0")
        crit = {"schema": 1, "overall": "crit", "warnings": ["array md1 degraded"]}
        with mock.patch.object(m, "collect_health", return_value=crit):
            rc, _ = self.flow()
            self.assertEqual((rc, self.steps()[-1]), (m.EXIT_REFUSED, ("health", "fail")))
            self.assertIn("array md1 degraded", self.events[-1][2])
            self.assertEqual(self.installed(), "1.0.0")
            self.events.clear()
            rc, _ = self.flow(spec_kw={"force": True})
            self.assertEqual(rc, 0)
        self.install_old("1.0.0")
        self.runner.on(["systemctl", "is-active"], "inactive\n", rc=3)
        self.events.clear()
        rc, _ = self.flow()
        self.assertEqual(rc, m.EXIT_REFUSED)
        self.assertIn("not active", self.events[-1][2])

    def test_push_path_is_not_health_gated_and_needs_no_network(self):
        """The app checks health itself; `--from DIR` keeps the contract. It must not touch the network at all."""
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0")
        import urllib.request
        with mock.patch.object(urllib.request.OpenerDirector, "open", side_effect=AssertionError("network used on the push path")), \
                mock.patch.object(m.HttpFetcher, "_open", side_effect=AssertionError("network used on the push path")), \
                mock.patch.object(m, "collect_health", side_effect=AssertionError("health gate on the push path")):
            rc, rec = self.flow(mode="dir", spec_kw={"src": self.src}, rec_kw={"source": "manual"})
        self.assertEqual((rc, self.installed()), (0, "1.2.0"))
        self.assertEqual(rec.data["to"], "1.2.0")

    def test_lock_refuses_a_second_job_and_clears_a_stale_one(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0")
        lock = m.JobLock(self.cfg)
        mine = lock.acquire("addon", "frigate")
        self.assertIsNotNone(mine)
        self.assertIsNone(lock.acquire("update", "x"))                                                               # second acquirer is refused
        rc, rec = self.flow(mode="dir", spec_kw={"src": self.src})
        self.assertEqual((rc, self.steps()[-1], rec.data["state"]), (m.EXIT_REFUSED, ("lock", "fail"), "refused"))
        self.assertIn("addon", self.events[-1][2])
        self.assertEqual(self.installed(), "1.0.0")
        self.assertTrue(os.path.exists(lock.path))                                                                     # the refused job did not release someone else's lock
        lock.release(mine)
        self.assertFalse(os.path.exists(lock.path))
        p = subprocess.Popen([sys.executable, "-c", "pass"])
        p.wait()
        write(lock.path, json.dumps({"kind": "addon", "pid": p.pid, "since": 1}))                                       # holder is gone: stale
        self.assertIsNone(lock.holder())
        rc, _ = self.flow(mode="dir", spec_kw={"src": self.src})
        self.assertEqual(rc, 0)

    def test_releasing_one_lock_never_frees_the_next_one(self):
        """Same process, same second: the release is matched by token, not by pid and time."""
        lock = m.JobLock(self.cfg)
        first = lock.acquire("addon", "a")
        lock.release(first)
        second = lock.acquire("addon", "b")
        lock.release(first)                                                                                        # a late, repeated release of the first job
        self.assertEqual((lock.holder() or {}).get("label"), "b")
        lock.release(second)
        self.assertIsNone(lock.holder())

    def test_audit_records_refusals_and_never_a_secret(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0", tamper="README.md")
        rc, rec = self.flow(mode="dir", spec_kw={"src": self.src}, rec_kw={"by": "alice"})
        self.assertEqual(rc, m.EXIT_REFUSED)
        lines = self.audit_lines()
        self.assertEqual([(a["action"], a["user"]) for a in lines], [("mgmt-update-start", "alice"), ("mgmt-update", "alice")])
        self.assertEqual(lines[-1]["result"], "refused (exit 2)")
        text = m.read_text(self.cfg.p(self.cfg.audit_path)) + json.dumps(rec.data)
        self.assertNotRegex(text.lower(), r"token|password|secret|authorization")

    def test_no_credentials_are_ever_sent(self):
        import inspect
        src = inspect.getsource(m.HttpFetcher) + inspect.getsource(m.GithubSource)
        for needle in ("Authorization", "GITHUB_TOKEN", "os.environ", "Bearer"):
            self.assertNotIn(needle, src)

    def test_jobs_are_reaped_when_their_process_is_gone(self):
        p = subprocess.Popen([sys.executable, "-c", "pass"])
        p.wait()
        rec = m.JobRecord.create(self.cfg, source="github-stable", by="a")
        rec.data.update(state="running", pid=p.pid)
        rec.save()
        old = m.JobRecord.create(self.cfg, source="github-stable", by="a", started=int(time.time()) - 500)
        m.reap_jobs(self.cfg)
        self.assertEqual(m.JobRecord.load(self.cfg, rec.id).data["state"], "failed")
        self.assertEqual(m.JobRecord.load(self.cfg, old.id).data["state"], "failed")
        self.assertIn("interrupted", m.JobRecord.load(self.cfg, rec.id).data["events"][-1]["message"])

    def test_job_records_are_pruned(self):
        for _ in range(m.JobRecord.KEEP + 5):
            m.JobRecord.create(self.cfg, source="manual")
        self.assertEqual(len(os.listdir(m._update_dir(self.cfg, "jobs"))), m.JobRecord.KEEP)


# ---------------------------------------------------------------------------------------------------------------- history (who/what)

class HistoryTests(FlowBase):
    def setUp(self):
        super().setUp()
        p = mock.patch.object(m, "default_probe", lambda cfg: True)
        p.start()
        self.addCleanup(p.stop)

    def test_app_push_and_manual_bundles_are_told_apart(self):
        self.install_old("1.0.0")
        app_dir = os.path.join(self.t, m.APP_STAGE_PREFIX + "abc123")
        make_bundle(app_dir, "1.1.0")
        make_bundle(self.src, "1.2.0")
        self.assertEqual(m.default_source(app_dir), "app")
        self.assertEqual(m.default_source(self.src), "manual")
        for d in (app_dir, self.src):
            args = m.parse_args(["update", "--from", d, "--json", "--health-timeout", "0.05"])
            with contextlib.redirect_stdout(io.StringIO()):
                rc = m.run_cli(args, self.cfg, self.runner)
            self.assertEqual(rc, 0)
        jobs = m.update_state(self.cfg)["jobs"]
        self.assertEqual([(j["source"], j["from"], j["to"], j["state"]) for j in jobs], [("manual", "1.1.0", "1.2.0", "ok"), ("app", "1.0.0", "1.1.0", "ok")])

    def test_state_lists_versions_previous_and_the_active_job(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0")
        self.flow(mode="dir", spec_kw={"src": self.src})
        st = m.update_state(self.cfg)
        self.assertEqual((st["installed"], st["previous"], st["active"], st["busy"], st["channel"], st["signed"]), ("1.2.0", "1.0.0", None, None, "stable", False))
        queued = m.JobRecord.create(self.cfg, source="github-stable", by="web")
        self.assertEqual(m.update_state(self.cfg)["active"], queued.id)

    def test_rollback_is_recorded_as_a_rollback(self):
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0")
        self.flow(mode="dir", spec_kw={"src": self.src})
        rc, rec = self.flow(mode="rollback", rec_kw={"kind": "rollback", "source": "rollback"})
        self.assertEqual((rc, self.installed()), (0, "1.0.0"))
        self.assertEqual((rec.data["kind"], rec.data["from"], rec.data["to"], rec.data["state"]), ("rollback", "1.2.0", "1.0.0", "ok"))

    def test_restart_waits_for_the_response(self):
        """The web job passes --restart-delay: the Updater sleeps right before it restarts the service."""
        self.install_old("1.0.0")
        make_bundle(self.src, "1.2.0")
        naps = []
        rec = m.JobRecord.create(self.cfg, source="manual", by="t")
        order = []
        self.runner.rules.append((["systemctl", "restart"], lambda argv: order.append("restart") or m.CompletedResult(0, "", "")))
        rc = m.run_update_flow(self.cfg, self.runner, {"mode": "dir", "src": self.src, "restart_delay": 3}, rec, probe=lambda c: True, health_timeout=0.05,
                               sleep=lambda x: (naps.append(x), order.append("sleep")))
        self.assertEqual(rc, 0)
        self.assertEqual(order[:2], ["sleep", "restart"])
        self.assertEqual(naps[0], 3)


# ---------------------------------------------------------------------------------------------------------------- http layer

class FetcherTests(unittest.TestCase):
    def setUp(self):
        self.gh = FakeGithub()
        self.addCleanup(self.gh.close)
        self.t = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.t, ignore_errors=True)

    def test_host_policy(self):
        f = m.HttpFetcher()
        for url in ("http://api.github.com/x", "https://evil.example/x", "https://github.com.evil.example/x", "https://user:pw@github.com/x", "https://github.com:8443/x",
                    "ftp://github.com/x", "https://raw.githubusercontent.com/x", "https://githubusercontent.com/x", "file:///etc/passwd", "https://127.0.0.1/x", "https://github.com\\@evil/x"):
            with self.assertRaises(m.UpdateError, msg=url) as c:
                f.check_url(url)
            self.assertEqual(c.exception.kind, "policy")
        for url in ("https://api.github.com/repos/x", "https://github.com/a/b/releases/download/v1/x", "https://codeload.github.com/a/b/tar.gz/abc",
                    "https://objects.githubusercontent.com/x", "https://release-assets.githubusercontent.com/x", "https://GitHub.com/x"):
            f.check_url(url)
        with self.assertRaises(m.UpdateError):
            f.get_bytes("http://127.0.0.1:1/x", 10)                                            # loopback is refused unless the test switch is on

    def test_redirect_to_another_host_is_refused_on_every_hop(self):
        self.gh.routes["/hop"] = (302, b"", {"Location": "https://evil.example/payload"})
        self.gh.routes["/hop2"] = (302, b"", {"Location": "/hop"})
        for p in ("/hop", "/hop2"):
            with self.assertRaises(m.UpdateError) as c:
                self.gh.fetcher().get_bytes(self.gh.base + p, 1000)
            self.assertEqual(c.exception.kind, "policy", p)
        self.gh.routes["/ok"] = (302, b"", {"Location": "/final"})
        self.gh.routes["/final"] = (200, b"fine", {})
        self.assertEqual(self.gh.fetcher().get_bytes(self.gh.base + "/ok", 1000), b"fine")

    def test_size_limits(self):
        self.gh.routes["/big"] = (200, b"x" * 5000, {})
        self.gh.routes["/liar"] = (200, b"x" * 100, {"Content-Length": "100"})
        f = self.gh.fetcher()
        with self.assertRaises(m.UpdateError) as c:
            f.get_bytes(self.gh.base + "/big", 1000)
        self.assertEqual(c.exception.kind, "size")
        dest = os.path.join(self.t, "dl")
        with self.assertRaises(m.UpdateError) as c:
            f.download(self.gh.base + "/big", dest, 1000)
        self.assertEqual(c.exception.kind, "size")
        self.assertFalse(os.path.exists(dest))                                                 # partial downloads are removed
        digest, n = f.download(self.gh.base + "/liar", dest, 1000)
        self.assertEqual((n, digest), (100, hashlib.sha256(b"x" * 100).hexdigest()))
        self.assertEqual(oct(os.stat(dest).st_mode & 0o777), "0o600")

    def test_typed_errors(self):
        f = self.gh.fetcher()
        self.gh.routes["/rl"] = (403, b"{}", {"X-RateLimit-Remaining": "0", "X-RateLimit-Reset": str(int(time.time()) + 600)})
        self.gh.routes["/forbidden"] = (403, b"{}", {})
        self.gh.routes["/boom"] = (500, b"{}", {})
        kinds = {}
        for p in ("/rl", "/forbidden", "/boom", "/missing"):
            with self.assertRaises(m.UpdateError) as c:
                f.get_bytes(self.gh.base + p, 100)
            kinds[p] = c.exception
        self.assertEqual({p: e.kind for p, e in kinds.items()}, {"/rl": "rate_limit", "/forbidden": "http", "/boom": "http", "/missing": "not_found"})
        self.assertRegex(kinds["/rl"].message, r"about (9|10|11) minute")
        self.gh.close()
        with self.assertRaises(m.UpdateError) as c:
            f.get_bytes(self.gh.base + "/x", 100)
        self.assertEqual(c.exception.kind, "offline")

    def test_user_agent_and_no_credentials(self):
        seen = {}

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                seen.update(self.headers.items())
                self.send_response(200)
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"{}")
        srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.server_close)
        self.addCleanup(srv.shutdown)
        m.HttpFetcher(loopback_http=True).get_bytes("http://127.0.0.1:%d/" % srv.server_address[1], 10)
        self.assertTrue(seen["User-Agent"].startswith("synofurb-mgmt/"))
        self.assertNotIn("Authorization", seen)
        self.assertNotIn("Cookie", seen)


# ---------------------------------------------------------------------------------------------------------------- CLI

class GithubCliTests(FlowBase):
    def setUp(self):
        super().setUp()
        p = mock.patch.object(m, "default_probe", lambda cfg: True)
        p.start()
        self.addCleanup(p.stop)

    def run_cli(self, *argv):
        out = io.StringIO()
        with mock.patch.object(m, "GithubSource", functools.partial(m.GithubSource, api_base=self.gh.base, dl_base=self.gh.base, codeload_base=self.gh.base)), \
                mock.patch.object(m, "HttpFetcher", functools.partial(m.HttpFetcher, loopback_http=True)), \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            rc = m.run_cli(m.parse_args(["update", "--json", "--health-timeout", "0.05"] + list(argv)), self.cfg, self.runner)
        return rc, [json.loads(ln) for ln in out.getvalue().splitlines() if ln.startswith("{")]

    def test_check_prints_one_json_object_and_changes_nothing(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0")
        rc, lines = self.run_cli("--from-github", "--check")
        self.assertEqual((rc, len(lines), lines[0]["version"], lines[0]["state"], lines[0]["channel"]), (0, 1, "1.2.0", "available", "stable"))
        self.assertEqual(self.installed(), "1.0.0")
        self.assertEqual(self.restarts(), 0)
        self.assertEqual(os.listdir(m._update_dir(self.cfg)) if os.path.isdir(m._update_dir(self.cfg)) else [], [])         # a check leaves no job behind
        sha, _ = self.gh.add_main("1.3.0")
        rc, lines = self.run_cli("--from-github", "--check", "--ref", "main")
        self.assertEqual((rc, lines[0]["channel"], lines[0]["commit"]), (0, "main", sha))
        rc, lines = self.run_cli("--check")
        self.assertEqual(rc, m.EXIT_REFUSED)

    def test_update_from_github_with_consent(self):
        self.install_old("1.0.0")
        self.gh.add_release("1.2.0")
        rc, lines = self.run_cli("--from-github")
        self.assertEqual((rc, lines[-1]["step"], lines[-1]["status"]), (m.EXIT_REFUSED, "consent", "fail"))
        self.assertEqual(set(lines[-1]), {"step", "status", "message"})                           # same NDJSON shape as the push path
        rc, lines = self.run_cli("--from-github", "--accept-unsigned", "--by", "me")
        self.assertEqual((rc, lines[-1]), (0, {"step": "result", "status": "ok", "message": "updated 1.0.0 -> 1.2.0"}))
        self.assertEqual(self.installed(), "1.2.0")
        self.assertEqual(m.update_state(self.cfg)["jobs"][0]["by"], "me")

    def test_update_from_a_ref_and_bad_refs(self):
        self.install_old("1.0.0")
        sha, _ = self.gh.add_main("1.3.0", ref="v1.3.0")
        rc, lines = self.run_cli("--from-github", "--ref", "v1.3.0", "--accept-unsigned")
        self.assertEqual((rc, self.installed()), (0, "1.3.0"))
        rc, lines = self.run_cli("--from-github", "--ref", "../../etc", "--accept-unsigned")
        self.assertEqual((rc, lines[-1]["step"]), (m.EXIT_REFUSED, "check"))

    def test_offline_is_a_refusal_not_a_crash(self):
        self.install_old("1.0.0")
        self.gh.close()
        rc, lines = self.run_cli("--from-github", "--accept-unsigned")
        self.assertEqual((rc, lines[-1]["status"], self.installed()), (m.EXIT_REFUSED, "fail", "1.0.0"))
        self.assertIn("Cannot reach GitHub", lines[-1]["message"])

    def test_status_and_missing_source(self):
        self.install_old("1.0.0")
        rc, lines = self.run_cli("--status")
        self.assertEqual((rc, lines[0]["installed"], lines[0]["jobs"]), (0, "1.0.0", []))
        rc, lines = self.run_cli()
        self.assertEqual((rc, lines[-1]["step"]), (m.EXIT_REFUSED, "verify"))


# ---------------------------------------------------------------------------------------------------------------- web routes

class WebTests(HttpTestCase):
    def setUp(self):
        super().setUp()
        p = mock.patch.object(m, "default_probe", lambda cfg: True)
        p.start()
        self.addCleanup(p.stop)
        self.gh = FakeGithub()
        self.addCleanup(self.gh.close)
        app = self.env.app
        app.update_fetch = self.gh.fetcher()
        self.patches = [mock.patch.object(m, "GithubSource", functools.partial(m.GithubSource, api_base=self.gh.base, dl_base=self.gh.base, codeload_base=self.gh.base)),
                        mock.patch.object(m, "HttpFetcher", functools.partial(m.HttpFetcher, loopback_http=True))]
        for p in self.patches:
            p.start()
            self.addCleanup(p.stop)
        self.launched = []
        self.threads = []
        self.runner.on(["systemctl", "is-active"], "active\n")
        self.runner.on([sys.executable], lambda argv: m.CompletedResult(0, json.dumps({"schema": 1, "version": m.read_text(os.path.join(os.path.dirname(argv[1]), "VERSION")).strip()}), ""))
        cfg = self.env.cfg
        inst = cfg.p(m.INSTALL_DIR)
        write(os.path.join(inst, "synofurb_mgmt.py"), script_for("1.0.0"))
        write(os.path.join(inst, "VERSION"), "1.0.0\n")

        def launcher(argv):
            self.launched.append(argv)
            args = m.parse_args(argv + ["--health-timeout", "0.05"])
            t = threading.Thread(target=lambda: self.codes.append(self.run_job(args)))
            self.threads.append(t)
            t.start()
        self.codes = []
        app.update_launcher = launcher

    def run_job(self, args):
        with contextlib.redirect_stdout(io.StringIO()):
            return m.run_cli(args, self.env.cfg, self.runner)

    def join(self):
        for t in self.threads:
            t.join(10)

    def test_requires_login_and_csrf(self):
        for method, path in (("GET", "/api/update"), ("POST", "/api/update/check"), ("POST", "/api/update/start"), ("POST", "/api/update/rollback"), ("POST", "/api/update/job")):
            self.assertEqual(self.cl.call(method, path, {} if method == "POST" else None)[0], 401, path)
        self.logged_in()
        self.assertEqual(self.cl.call("POST", "/api/update/start", {"channel": "stable", "expect": "1.2.0"}, csrf=False)[0], 403)

    def test_check_start_progress_and_history(self):
        self.gh.add_release("1.2.0", body="- shiny")
        cl = self.logged_in()
        st, obj, _ = cl.call("POST", "/api/update/check", {"channel": "stable"})
        self.assertEqual((st, obj["check"]["version"], obj["check"]["state"], obj["check"]["notes"]), (200, "1.2.0", "available", "• shiny"))
        s = cl.call("GET", "/api/update")[1]
        self.assertEqual((s["installed"], s["channel"], s["check"]["version"], s["active"], s["previous"]), ("1.0.0", "stable", "1.2.0", None, None))
        st, obj, _ = cl.call("POST", "/api/update/start", {"channel": "stable", "expect": "1.2.0", "accept_unsigned": True})
        self.assertEqual(st, 200, obj)
        jid = obj["job"]
        self.join()
        self.assertEqual(self.codes, [0])
        argv = self.launched[0]
        self.assertEqual(argv[:2], ["update", "--json"])
        self.assertIn("--from-github", argv)
        self.assertIn("--accept-unsigned", argv)
        self.assertEqual(argv[argv.index("--restart-delay") + 1], "3")
        self.assertEqual(argv[argv.index("--by") + 1], "admin")
        full = cl.call("POST", "/api/update/job", {"job": jid})[1]
        self.assertEqual((full["job"]["state"], full["job"]["by"], full["job"]["via"], full["job"]["from"], full["job"]["to"]), ("ok", "admin", "web", "1.0.0", "1.2.0"))
        self.assertEqual(full["events"][-1]["step"], "result")
        self.assertEqual(self.cl.__class__(self.env).raw("POST", "/api/update/progress", {"job": jid})[1]["job"]["state"], "ok")  # readable without a session, after the restart
        hist = cl.call("GET", "/api/update")[1]
        self.assertEqual((hist["installed"], hist["previous"], hist["jobs"][0]["source"], hist["jobs"][0]["via"]), ("1.2.0", "1.0.0", "github-stable", "web"))
        self.assertIn("job " + jid, self.env.read(self.env.cfg.audit_path))                        # the request itself is audited with the job id

    def test_progress_endpoint_leaks_nothing_but_state(self):
        rec = m.JobRecord.create(self.env.cfg, source="github-main", by="admin", ref="secret-branch", commit="a" * 40)
        rec.emit("check", "ok", "internal detail")
        st, obj, _ = Client(self.env).raw("POST", "/api/update/progress", {"job": rec.id})
        self.assertEqual(sorted(obj["job"]), ["finished", "from", "id", "kind", "state", "step", "to"])
        self.assertNotIn("secret-branch", json.dumps(obj))
        self.assertNotIn("internal detail", json.dumps(obj))
        for bad in ("../../etc/passwd", "zz", "0" * 16, 5, None):
            st, obj, _ = Client(self.env).raw("POST", "/api/update/progress", {"job": bad})
            self.assertIn(st, (400, 404), bad)

    def test_main_channel_needs_the_typed_commit(self):
        sha, _ = self.gh.add_main("1.3.0")
        cl = self.logged_in()
        body = {"channel": "main", "ref": "main", "expect": sha, "accept_unsigned": True}
        st, obj, _ = cl.call("POST", "/api/update/start", body)
        self.assertEqual(st, 400)
        self.assertIn(sha[:7], obj["error"])
        self.assertEqual(self.launched, [])
        st, obj, _ = cl.call("POST", "/api/update/start", dict(body, confirm=sha[:7]))
        self.assertEqual(st, 200, obj)
        self.join()
        self.assertEqual(self.codes, [0])
        self.assertEqual(m.read_text(os.path.join(self.env.cfg.p(m.INSTALL_DIR), "VERSION")).strip(), "1.3.0")
        self.assertIn("--ref", self.launched[0])

    def test_inputs_are_validated(self):
        cl = self.logged_in()
        for body in ({}, {"channel": "beta", "expect": "1"}, {"channel": "stable"}, {"channel": "stable", "expect": "x y"}, {"channel": "main", "ref": "../x", "expect": "1.2.0", "confirm": "1.2.0"},
                     {"channel": "stable", "expect": "1.2.0;rm"}):
            self.assertEqual(cl.call("POST", "/api/update/start", body)[0], 400, body)
        self.assertEqual(cl.call("POST", "/api/update/check", {"channel": "main", "ref": "a b"})[0], 400)
        self.assertEqual(self.launched, [])

    def test_a_running_job_blocks_a_second_one_with_409(self):
        self.gh.add_release("1.2.0")
        cl = self.logged_in()
        lock = m.JobLock(self.env.cfg)
        held = lock.acquire("addon", "frigate")
        st, obj, _ = cl.call("POST", "/api/update/start", {"channel": "stable", "expect": "1.2.0", "accept_unsigned": True})
        self.assertEqual(st, 409)
        self.assertIn("addon", obj["error"])
        self.assertEqual(cl.call("POST", "/api/update/rollback", {})[0], 409)                    # no previous version, checked first
        self.assertEqual(self.launched, [])
        lock.release(held)

    def test_rollback_job(self):
        cl = self.logged_in()
        self.assertEqual(cl.call("POST", "/api/update/rollback", {})[0], 409)
        write(os.path.join(self.env.cfg.p(m.INSTALL_DIR + ".prev"), "synofurb_mgmt.py"), script_for("0.9.0"))
        write(os.path.join(self.env.cfg.p(m.INSTALL_DIR + ".prev"), "VERSION"), "0.9.0\n")
        st, obj, _ = cl.call("POST", "/api/update/rollback", {})
        self.assertEqual(st, 200, obj)
        self.join()
        self.assertEqual(self.codes, [0])
        j = cl.call("POST", "/api/update/job", {"job": obj["job"]})[1]["job"]
        self.assertEqual((j["kind"], j["source"], j["state"], j["to"]), ("rollback", "rollback", "ok", "0.9.0"))

    def test_launcher_failure_is_reported_and_leaves_no_running_job(self):
        self.env.app.update_launcher = None
        self.runner.avail.add("systemd-run")
        self.env.app.cfg.dry_run = False
        self.runner.on(["systemd-run"], "", rc=1, err="Failed to connect to bus")
        self.gh.add_release("1.2.0")
        cl = self.logged_in()
        st, obj, _ = cl.call("POST", "/api/update/start", {"channel": "stable", "expect": "1.2.0"})
        self.assertEqual(st, 500)
        self.assertIn("could not start", obj["error"])
        s = cl.call("GET", "/api/update")[1]
        self.assertEqual((s["active"], s["jobs"][0]["state"]), (None, "failed"))

    def test_job_runs_in_its_own_systemd_unit(self):
        """The web service must not be the parent of the job: systemd-run puts it in a transient unit outside synofurb-mgmt.service's cgroup."""
        self.env.app.update_launcher = None
        self.env.app.cfg.dry_run = False
        self.runner.avail.add("systemd-run")
        self.gh.add_release("1.2.0")
        cl = self.logged_in()
        st, obj, _ = cl.call("POST", "/api/update/start", {"channel": "stable", "expect": "1.2.0", "accept_unsigned": True})
        self.assertEqual(st, 200, obj)
        call = [c for c in self.runner.calls if c[:1] == ["systemd-run"]][0]
        self.assertIn("--collect", call)
        self.assertEqual(call[call.index("--unit") + 1], "synofurb-mgmt-update-" + obj["job"][:8])
        sep = call.index("--")
        self.assertEqual(call[sep + 1:sep + 2], [sys.executable])
        self.assertIn(obj["job"], call[sep + 1:])
        self.assertIn("--job-id", call[sep + 1:])
        self.assertIn("--root", call[sep + 1:])                                                    # tests only: a sandbox root is passed on


class StaticTests(unittest.TestCase):
    def test_dynamic_copy_keys_exist(self):
        for src in m.UPDATE_SOURCES:
            self.assertIn("upd.src." + src, m.COPY)
        for st in ("ok", "current", "refused", "rolled_back", "rollback_failed", "failed", "running", "queued"):
            self.assertIn("upd.s." + st, m.COPY)
        for step in ("lock", "check", "download", "consent", "verify", "stage", "swap", "restart", "health", "rollback", "result"):
            self.assertIn("upd.step." + step, m.COPY)
        self.assertIn("updates", [t["id"] for t in m.HELP_TOPICS])

    def test_make_update_bundle_and_manifest_share_the_file_list(self):
        mod = load_bundler()
        self.assertEqual(tuple(mod.FILES), m.BUNDLE_FILES)
        self.assertTrue(all(m.bundle_member_ok(n) for n in m.BUNDLE_FILES))
        self.assertFalse(any(m.bundle_member_ok(n) for n in ("../x", "MANIFEST.sha256", "a/b", "x.py", "")))

    def test_the_release_workflow_builds_the_asset_the_stable_channel_reads(self):
        root = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
        text = open(os.path.join(root, "scripts", "package-mgmt.sh"), encoding="utf-8").read()
        self.assertIn("synofurb-mgmt-$MV.tar.gz", text)
        self.assertTrue(m.ASSET_RE.fullmatch("synofurb-mgmt-0.5.0.tar.gz"))
        self.assertFalse(m.ASSET_RE.fullmatch("synofurb-0.2.2-linux-x86_64.tar.gz"))


if __name__ == "__main__":
    unittest.main()
