"""Add-on framework tests (synofurb_addons.py + the glue in synofurb_mgmt.py): loader, rendering, lifecycle, jobs, apply-pending, API, CLI.

Nothing here talks to a real Docker daemon: Docker is the ``FakeDocker`` test double (or the real ``Docker`` class on top of a scripted
runner, to check the exact argv), apt/systemctl are scripted ``FakeRunner`` rules, the clock is fake and the file system is a temp root.
"""
from __future__ import annotations

import copy
import io
import json
import os
import shutil
import stat
import sys
import tempfile
import threading
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, ROOT)
sys.path.insert(0, HERE)
import synofurb_addons as A  # noqa: E402
import synofurb_mgmt as m  # noqa: E402
from test_mgmt import Client, Env, FakeRunner  # noqa: E402

ADDONS_DIR = os.path.join(ROOT, "addons")


class Clock:
    def __init__(self):
        self.t = 1000.0

    def now(self):
        return self.t

    def sleep(self, s):
        self.t += s


def real_sources():
    found = {a: A.read_addon_dir(os.path.join(ADDONS_DIR, a)) for a in sorted(os.listdir(ADDONS_DIR)) if os.path.isdir(os.path.join(ADDONS_DIR, a))}
    return [("builtin", found)]


def real_catalog():
    return A.load_catalog(real_sources(), handlers=tuple(A.SYSTEM_HANDLERS))


BASE = {"schema": 1, "id": "demo", "name": "Demo", "version": "1", "summary": "A demo.", "kind": "compose", "requires": [], "compose": "compose.yml.tmpl",
        "settings": [{"key": "port", "label": "Port", "type": "int", "default": 8080, "min": 1, "max": 65535}]}
BASE_COMPOSE = 'services:\n  demo:\n    image: demo:1\n    ports:\n      - "${port}:80"\n'


def parse(manifest=None, compose=BASE_COMPOSE, extra_files=None, **over):
    mf = copy.deepcopy(manifest or BASE)
    mf.update(over)
    files = {"addon.json": json.dumps(mf), "compose.yml.tmpl": compose}
    files.update(extra_files or {})
    return A.parse_manifest(mf, mf["id"], files, "user", tuple(A.SYSTEM_HANDLERS))


class Sandbox:
    """Temp root with fake /dev and /sys, a clock, a scripted runner and a FakeDocker; ``mgr`` is a ready Manager."""

    def __init__(self, mounts=None, catalog=None, probe_ok=True, users=True):
        self.tmp = tempfile.mkdtemp(prefix="synofurb-addons-")
        self.clock = Clock()
        self.runner = FakeRunner()
        self.audit: list[tuple] = []
        self.mounts = mounts if mounts is not None else [{"target": "/", "source": "/dev/md0", "fstype": "ext4"},
                                                          {"target": "/srv/data", "source": "/dev/mapper/vg_data-data", "fstype": "xfs"}]
        self.http = {"status": 200 if probe_ok else None}
        self.dev = os.path.join(self.tmp, "dev")
        self.sys = os.path.join(self.tmp, "sys")
        os.makedirs(self.dev)
        os.makedirs(self.sys)
        if users:
            self.write("/etc/passwd", "root:x:0:0:root:/root:/bin/bash\nadmin:x:1000:1000::/home/admin:/bin/bash\n")
            self.write("/etc/group", "root:x:0:\nsudo:x:27:admin\ndocker:x:998:\nadmin:x:1000:\n")
        self.env = A.AddonEnv(root=self.tmp, run=self.runner.run, mounts=lambda: self.mounts, audit=lambda *a: self.audit.append(a), script_dir=ROOT,
                              dev_root=self.dev, sys_root=self.sys, sleep=self.clock.sleep, now=self.clock.now, chown=False,
                              http_probe=lambda url, timeout=4: self.http["status"], tcp_probe=lambda port, timeout=3: self.http["status"] is not None,
                              port_in_use=lambda port: False)
        self.docker = A.FakeDocker(self.env)
        self.mgr = A.Manager(self.env, self.docker, catalog)
        os.makedirs(self.env.p("/srv/data"), exist_ok=True)

    def write(self, path, text, mode=0o644):
        p = self.env.p(path) if hasattr(self, "env") else os.path.join(self.tmp, path.lstrip("/"))
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as fh:
            fh.write(text)
        os.chmod(p, mode)

    def read(self, path):
        with open(self.env.p(path)) as fh:
            return fh.read()

    def exists(self, path):
        return os.path.lexists(self.env.p(path))

    def close(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def install_docker(self):
        self.mgr.store.update("docker", status="installed", version="1")

    def run_ok(self, job):
        self.test.assertTrue(job.ok, job.error)
        return job


class SandboxTest(unittest.TestCase):
    mounts = None

    def setUp(self):
        self.sb = Sandbox(mounts=self.mounts)
        self.addCleanup(self.sb.close)
        self.mgr, self.env = self.sb.mgr, self.sb.env

    def install(self, aid, body=None, **kw):
        body = dict({"accept": True}, **(body or {}))
        job = self.mgr.install(aid, body, user="tester", ip="1.2.3.4", background=False, **kw)
        return job


# ============================================================================ loader

class ManifestRuleTests(unittest.TestCase):
    def bad(self, text, **over):
        with self.assertRaises(A.AddonError) as cm:
            parse(**over) if "manifest" not in over else parse(over.pop("manifest"), **over)
        self.assertIn(text, str(cm.exception))

    def mutate(self, **fields):
        mf = copy.deepcopy(BASE)
        mf.update(fields)
        return mf

    def test_minimal_manifest_is_valid(self):
        a = parse()
        self.assertEqual((a.id, a.kind, a.data_dir), ("demo", "compose", "/srv/addons/demo"))
        self.assertEqual(a.value_rules["port"], "cquoted")            # sits inside "${port}:80"

    def test_bad_manifests_are_rejected_with_a_precise_message(self):
        cases = [
            ("'schema' must be 1", self.mutate(schema=2)),
            ("'id' (other) must equal the folder name (demo)", self.mutate(id="other")),
            ("'id' is not valid", self.mutate(id="Bad_ID")),
            ("'kind' must be compose or system", self.mutate(kind="vm")),
            ("unknown field", self.mutate(bogus=1)),
            ("'name' is required", {k: v for k, v in BASE.items() if k != "name"}),
            ("'requires' must list other add-on ids", self.mutate(requires=["demo"])),
            ("'requires' must list other add-on ids", self.mutate(requires=["a", "a"])),
            ("a compose add-on needs 'compose'", {k: v for k, v in BASE.items() if k != "compose"}),
            ("'compose' is not valid", self.mutate(compose="../x.tmpl")),
            ("unknown privilege", self.mutate(privileges=["root"])),
            ("settings[0] (port): 'type' must be one of", self.mutate(settings=[{"key": "port", "label": "P", "type": "float"}])),
            ("duplicate setting key", self.mutate(settings=[{"key": "a", "label": "A"}, {"key": "a", "label": "A"}])),
            ("must not end in '_json'", self.mutate(settings=[{"key": "a_json", "label": "A"}])),
            ("default is not valid", self.mutate(settings=[{"key": "n", "label": "N", "type": "int", "default": 500, "max": 10}])),
            ("a choice setting needs 'choices'", self.mutate(settings=[{"key": "c", "label": "C", "type": "choice"}])),
            ("'min' is not allowed for type 'bool'", self.mutate(settings=[{"key": "b", "label": "B", "type": "bool", "min": 1}])),
            ("'pattern' is not a valid expression", self.mutate(settings=[{"key": "s", "label": "S", "type": "string", "pattern": "("}])),
            ("'bind' must be an IPv4 address", self.mutate(ports=[{"host": 80, "container": 80, "bind": "nope"}])),
            ("'port' must be between 1 and 65535", self.mutate(web={"port": 70000})),
            ("'scheme' must be http or https", self.mutate(web={"port": 80, "scheme": "ftp"})),
            ("only this unit is probed", self.mutate(healthcheck={"type": "http", "url": "http://example.com:80/"})),
            ("'type' must be http, tcp or container", self.mutate(healthcheck={"type": "ping"})),
            ("'dest' must be a relative path", self.mutate(files=[{"template": "c.tmpl", "dest": "../etc/passwd"}])),
            ("'dest' must be a relative path", self.mutate(files=[{"template": "c.tmpl", "dest": "/etc/passwd"}])),
            ("'dest' 'compose.yml' is reserved".replace("'compose.yml'", "'compose.yml'"), self.mutate(files=[{"template": "c.tmpl", "dest": "compose.yml"}])),
            ("two files have the same 'dest'", self.mutate(files=[{"template": "c.tmpl", "dest": "a"}, {"template": "d.tmpl", "dest": "a"}])),
            ("'detect' must be path:<file> or usb", self.mutate(devices=[{"id": "d", "label": "D", "host": "/dev/x", "detect": "pci:1"}])),
            ("'host' must be a plain path below /dev", self.mutate(devices=[{"id": "d", "label": "D", "host": "/etc/shadow"}])),
            ("'host' must be a plain path below /dev", self.mutate(devices=[{"id": "d", "label": "D", "host": "/dev/../etc"}])),
            ("storage '", self.mutate(storage=[{"key": "k", "label": "K", "path": "/etc/foo"}])),
            ("must be a normalized absolute path", self.mutate(storage=[{"key": "k", "label": "K", "path": "/srv/a/../b"}])),
        ]
        for text, mf in cases:
            with self.subTest(text=text):
                files = {"compose.yml.tmpl": BASE_COMPOSE, "c.tmpl": "x", "d.tmpl": "y"}
                with self.assertRaises(A.AddonError) as cm:
                    A.parse_manifest(mf, "demo", dict(files, **{"addon.json": json.dumps(mf)}), "user", ("docker",))
                self.assertIn(text, str(cm.exception))

    def test_bulk_storage_must_live_on_the_data_volume(self):
        good = self.mutate(storage=[{"key": "media", "label": "M", "path": "/srv/data/addons/demo/media", "bulk": True}])
        self.assertEqual(A.parse_manifest(good, "demo", {"compose.yml.tmpl": BASE_COMPOSE}, "user", ()).storage[0]["bulk"], True)
        for path in ("/srv/addons/demo/media", "/mnt/disk/media", "/srv/datax/media"):
            with self.subTest(path=path):
                bad = self.mutate(storage=[{"key": "media", "label": "M", "path": path, "bulk": True}])
                with self.assertRaises(A.AddonError) as cm:
                    A.parse_manifest(bad, "demo", {"compose.yml.tmpl": BASE_COMPOSE}, "user", ())
                self.assertIn("redundant data volume", str(cm.exception))

    def test_system_addon_rules(self):
        mf = {"schema": 1, "id": "sys", "name": "S", "version": "1", "summary": "s", "kind": "system", "handler": "docker"}
        self.assertEqual(A.parse_manifest(mf, "sys", {}, "user", ("docker",)).kind, "system")
        for text, over in (("known 'handler'", {"handler": "rm-rf"}), ("no compose file", {"compose": "c.tmpl"}), ("no compose file", {"privileges": ["privileged"]})):
            with self.subTest(text=text):
                with self.assertRaises(A.AddonError) as cm:
                    A.parse_manifest(dict(mf, **over), "sys", {"c.tmpl": "x"}, "user", ("docker",))
                self.assertIn(text, str(cm.exception))

    def test_choice_options_vars_and_privileges(self):
        s = {"key": "net", "label": "N", "type": "choice", "default": "host", "choices": [
            {"value": "host", "label": "Host", "vars": {"NET_BLOCK": "    network_mode: host\n"}, "privileges": ["host-network"]},
            {"value": "bridge", "vars": {"NET_BLOCK": '    ports:\n      - "80:80"\n'}}]}
        compose = "services:\n  demo:\n    image: demo:1\n${NET_BLOCK}"
        a = parse(self.mutate(settings=[s]), compose=compose)
        self.assertEqual(A.effective_privileges(a, {"net": "host"}), ["host-network"])
        self.assertEqual(A.effective_privileges(a, {"net": "bridge"}), [])
        pub = a.public()["settings"][0]["choices"]
        self.assertEqual(pub[0], {"value": "host", "label": "Host", "privileges": ["host-network"]})      # blocks are not sent to the browser
        # a block that needs a privilege its option does not declare is refused
        s2 = copy.deepcopy(s)
        s2["choices"][0]["privileges"] = []
        with self.assertRaises(A.AddonError) as cm:
            parse(self.mutate(settings=[s2]), compose=compose)
        self.assertIn("host network", str(cm.exception))
        s3 = copy.deepcopy(s)
        s3["choices"][1]["vars"] = {"OTHER": "x"}
        with self.assertRaises(A.AddonError) as cm:
            parse(self.mutate(settings=[s3]), compose=compose)
        self.assertIn("same variable names", str(cm.exception))

    def test_template_problems(self):
        for text, compose in (
                ("unknown placeholder", "image: ${nope}\n"),
                ("not a ${name} placeholder", "image: $5\n"),
                ("must be alone on its line", "a: 1\n  ${DEVICES_BLOCK}\n"),
                ("mounts the Docker socket", "v: /var/run/docker.sock:/var/run/docker.sock\n"),
                ("runs privileged", "    privileged: true\n"),
                ("uses the host network", "    network_mode: host\n"),
                ("adds the capability NET_ADMIN", "    cap_add:\n      - NET_ADMIN\n"),
                ("shares a host namespace", "    pid: host\n"),
                ("confinement", "    security_opt:\n      - seccomp:unconfined\n"),
                ("not allowed in an add-on", "    env_file: /etc/shadow\n"),
                ("not allowed in an add-on", "    build: .\n"),
                ("mounts the host path /etc", "    volumes:\n      - /etc:/hostetc\n"),
                ("mounts the host path /", "    volumes:\n      - /:/host:ro\n"),
                ("by hand", "    devices:\n      - /dev/mem:/dev/mem\n")):
            with self.subTest(text=text):
                with self.assertRaises(A.AddonError) as cm:
                    parse(compose=compose)
                self.assertIn(text.replace("by hand", "devices"), str(cm.exception))
        a = parse(compose="    volumes:\n      - /etc/localtime:/etc/localtime:ro\n      - ${DATA_DIR}/x:/x\n")        # allowed
        self.assertEqual(a.id, "demo")

    def test_declared_privileges_allow_what_they_name(self):
        compose = "    privileged: true\n    network_mode: host\n    cap_add:\n      - NET_ADMIN\n      - SYS_TIME\n    v: /var/run/docker.sock\n"
        a = parse(compose=compose, privileges=["privileged", "host-network", "cap:NET_ADMIN", "cap:SYS_TIME", "docker-socket"])
        self.assertEqual(len(a.privileges), 5)
        with self.assertRaises(A.AddonError):
            parse(compose=compose, privileges=["privileged", "host-network", "cap:NET_ADMIN", "docker-socket"])        # SYS_TIME undeclared

    def test_devices_block_requires_declared_devices(self):
        with self.assertRaises(A.AddonError) as cm:
            parse(compose="a: 1\n${DEVICES_BLOCK}\n")
        self.assertIn("declares no devices", str(cm.exception))

    def test_text_setting_is_not_allowed_in_the_compose_file(self):
        mf = self.mutate(settings=[{"key": "notes", "label": "N", "type": "text"}])
        with self.assertRaises(A.AddonError) as cm:
            parse(mf, compose="a: ${notes}\n")
        self.assertIn("cannot be used in the compose file", str(cm.exception))

    def test_missing_template_file(self):
        mf = self.mutate(files=[{"template": "gone.tmpl", "dest": "c/x"}])
        with self.assertRaises(A.AddonError) as cm:
            parse(mf)
        self.assertIn("gone.tmpl is listed in addon.json but is missing", str(cm.exception))

    def test_first_install_only_note_for_settings_that_feed_once_files(self):
        mf = self.mutate(settings=[{"key": "a", "label": "A", "type": "int", "default": 1}, {"key": "b", "label": "B", "type": "int", "default": 2}],
                         files=[{"template": "c.tmpl", "dest": "conf/c", "once": True}])
        a = parse(mf, compose="x: ${b}\n", extra_files={"c.tmpl": "a=${a}\nb=${b}\n"})
        self.assertEqual({s["key"]: s["first_install_only"] for s in a.settings}, {"a": True, "b": False})


class SettingRuleTests(unittest.TestCase):
    def setting(self, **kw):
        return A._parse_setting(dict({"key": "k", "label": "Key", "type": "string"}, **kw), 0, set())

    def test_pattern_min_max_required_are_enforced_for_non_empty_values(self):
        s = self.setting(pattern="^https?://[^\\s\"'{}$#]+$", min=10, max=40)
        self.assertEqual(A.coerce_setting(s, "https://example.org/a"), "https://example.org/a")
        self.assertEqual(A.coerce_setting(s, ""), "")                                          # emptiness is 'required' business
        for bad in ("exec:rm -rf /", "https://a", "https://example.org/" + "a" * 40, "https://x.org/{a}", "https://x.org/$HOME", "https://x.org/#c"):
            with self.subTest(bad=bad), self.assertRaises(A.AddonError):
                A.coerce_setting(s, bad)

    def test_rules_by_placeholder_position(self):
        s = self.setting()
        self.assertEqual(A.coerce_setting(s, 'a"b', "loose"), 'a"b')
        with self.assertRaises(A.AddonError):
            A.coerce_setting(s, 'a"b', "cquoted")
        with self.assertRaises(A.AddonError):
            A.coerce_setting(s, "a$b", "cquoted")                       # compose would interpolate it
        self.assertEqual(A.coerce_setting(s, "a$b", "fquoted"), "a$b")
        with self.assertRaises(A.AddonError):
            A.coerce_setting(s, "two words", "raw")
        with self.assertRaises(A.AddonError):
            A.coerce_setting(s, "a\nb", "loose")
        self.assertEqual(A.coerce_setting(s, "rtsp://u:p%40w@h:554/s?x=1&y=2", "raw"), "rtsp://u:p%40w@h:554/s?x=1&y=2")

    def test_types(self):
        i = self.setting(type="int", min=1, max=10)
        self.assertEqual(A.coerce_setting(i, "7"), 7)
        for bad in (0, 11, "x", True, 1.5):
            with self.subTest(bad=bad), self.assertRaises(A.AddonError):
                A.coerce_setting(i, bad)
        b = self.setting(type="bool")
        self.assertIs(A.coerce_setting(b, "yes"), True)
        self.assertIs(A.coerce_setting(b, "off"), False)
        with self.assertRaises(A.AddonError):
            A.coerce_setting(b, "maybe")
        c = self.setting(type="choice", choices=["a", {"value": "b", "label": "B"}])
        self.assertEqual(A.coerce_setting(c, "b"), "b")
        with self.assertRaises(A.AddonError):
            A.coerce_setting(c, "c")
        p = self.setting(type="path")
        self.assertEqual(A.coerce_setting(p, "/srv/data/media"), "/srv/data/media")
        for bad in ("/etc/shadow", "/srv/data/../../etc", "srv/data/x", "/srv/data/a b", "/srv/data/x/", "/srv"):
            with self.subTest(bad=bad), self.assertRaises(A.AddonError):
                A.coerce_setting(p, bad)

    def test_secret_may_have_a_placeholder_default(self):
        s = self.setting(type="secret", default="rtsp://127.0.0.1:554/rtsp")
        self.assertEqual(s["default"], "rtsp://127.0.0.1:554/rtsp")


class CatalogTests(unittest.TestCase):
    def test_shipped_catalog_loads_without_problems(self):
        cat = real_catalog()
        self.assertEqual(cat.problems, [])
        self.assertEqual({"docker", "portainer", "frigate", "plex"} - set(cat.addons), set())
        self.assertEqual([a.id for a in cat.order()][:2], ["docker", "portainer"])                  # requires first
        self.assertEqual(cat.dependents("docker"), sorted(set(cat.dependents("docker"))))
        self.assertIn("portainer", cat.dependents("docker"))

    def test_pack_round_trip_equals_the_directory(self):
        pack = A.build_pack(ADDONS_DIR)
        self.assertEqual(pack, A.build_pack(ADDONS_DIR))                                              # deterministic
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        p = os.path.join(d, A.PACK_NAME)
        with open(p, "w") as fh:
            fh.write(pack)
        packed = A.load_catalog([("builtin", A.read_pack(p))], handlers=("docker",))
        self.assertEqual(packed.problems, [])
        self.assertEqual(sorted(packed.addons), sorted(real_catalog().addons))

    def sources(self, **defs):
        return [("builtin", {k: {"addon.json": json.dumps(v), "compose.yml.tmpl": BASE_COMPOSE} for k, v in defs.items()})]

    def mf(self, aid, requires=()):
        return dict(BASE, id=aid, requires=list(requires))

    def test_cycle_is_rejected_for_every_member(self):
        cat = A.load_catalog(self.sources(a=self.mf("a", ["b"]), b=self.mf("b", ["c"]), c=self.mf("c", ["a"]), ok=self.mf("ok")), handlers=())
        self.assertEqual(sorted(cat.addons), ["ok"])
        msgs = {p["id"]: p["error"] for p in cat.problems}
        self.assertEqual(sorted(msgs), ["a", "b", "c"])
        self.assertIn("dependency cycle", msgs["a"])
        self.assertIn("->", msgs["a"])

    def test_self_cycle_via_two_and_orphans_of_a_cycle(self):
        cat = A.load_catalog(self.sources(a=self.mf("a", ["b"]), b=self.mf("b", ["a"]), d=self.mf("d", ["a"])), handlers=())
        self.assertEqual(cat.addons, {})
        self.assertEqual(sorted(p["id"] for p in cat.problems), ["a", "b", "d"])

    def test_unknown_requirement(self):
        cat = A.load_catalog(self.sources(a=self.mf("a", ["ghost"]), b=self.mf("b", ["a"])), handlers=())
        self.assertEqual(cat.addons, {})
        self.assertIn("'ghost'", [p for p in cat.problems if p["id"] == "a"][0]["error"])

    def test_one_bad_addon_does_not_hide_the_others(self):
        src = self.sources(ok=self.mf("ok"), bad=dict(self.mf("bad"), kind="vm"))
        cat = A.load_catalog(src, handlers=())
        self.assertEqual(sorted(cat.addons), ["ok"])
        self.assertEqual(cat.problems[0]["id"], "bad")

    def test_user_addon_cannot_replace_a_builtin(self):
        src = self.sources(ok=self.mf("ok")) + [("user", {"ok": {"addon.json": json.dumps(self.mf("ok")), "compose.yml.tmpl": BASE_COMPOSE}})]
        cat = A.load_catalog(src, handlers=())
        self.assertEqual(cat.addons["ok"].source, "builtin")
        self.assertIn("already provided", cat.problems[0]["error"])

    def test_directory_reader_refuses_links_folders_and_odd_names(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        with open(os.path.join(d, "addon.json"), "w") as fh:
            fh.write("{}")
        os.symlink("/etc/passwd", os.path.join(d, "link.tmpl"))
        with self.assertRaises(A.AddonError) as cm:
            A.read_addon_dir(d)
        self.assertIn("not a regular file", str(cm.exception))
        os.unlink(os.path.join(d, "link.tmpl"))
        os.mkdir(os.path.join(d, "sub"))
        with self.assertRaises(A.AddonError):
            A.read_addon_dir(d)
        os.rmdir(os.path.join(d, "sub"))
        open(os.path.join(d, "bad name"), "w").close()
        with self.assertRaises(A.AddonError) as cm:
            A.read_addon_dir(d)
        self.assertIn("not allowed", str(cm.exception))

    def test_user_folder_is_loaded_and_pending_is_skipped(self):
        sb = Sandbox()
        self.addCleanup(sb.close)
        for aid in ("mine", "pending"):
            sb.write("/etc/synofurb/addons.d/%s/addon.json" % aid, json.dumps(dict(BASE, id=aid)))
            sb.write("/etc/synofurb/addons.d/%s/compose.yml.tmpl" % aid, BASE_COMPOSE)
        sb.write("/etc/synofurb/addons.d/broken/addon.json", "{not json")
        cat = A.load_default_catalog(sb.env)
        self.assertEqual(cat.addons["mine"].source, "user")
        self.assertNotIn("pending", cat.addons)
        self.assertIn("not valid JSON", [p for p in cat.problems if p["id"] == "broken"][0]["error"])


# ============================================================================ rendering

class RenderTests(SandboxTest):
    ident = {"puid": 1000, "pgid": 1000, "tz": "Europe/Stockholm", "admins": ["admin"]}

    def setUp(self):
        super().setUp()
        self.cat = real_catalog()

    def values(self, a, **over):
        v, _, _ = A.validate_settings(a, over)
        return v

    def render(self, aid, devices=(), storage=None, **over):
        a = self.cat.get(aid)
        st = {s["key"]: s["path"] for s in a.storage}
        st.update(storage or {})
        return a, A.render_addon(a, self.values(a, **over), st, [d for d in a.devices if d["id"] in devices], self.ident)

    def test_portainer_has_the_socket_and_declares_it(self):
        a, (compose, extra) = self.render("portainer")
        self.assertIn("/var/run/docker.sock:/var/run/docker.sock", compose.text)
        self.assertIn("/srv/addons/portainer/data:/data", compose.text)
        self.assertIn('TZ: "Europe/Stockholm"', compose.text)
        self.assertIn("portainer/portainer-ce:lts", compose.text)
        self.assertEqual(a.privileges, ["docker-socket"])
        self.assertEqual(extra, [])

    def test_frigate_defaults_devices_and_detector_blocks(self):
        a, (compose, extra) = self.render("frigate")
        self.assertNotIn("devices:", compose.text)
        self.assertIn("/srv/data/addons/frigate/media:/media/frigate", compose.text)
        self.assertIn('shm_size: "512mb"', compose.text)
        cfg = extra[0]
        self.assertEqual(cfg.dest, "config/config.yml")
        self.assertIn("type: cpu", cfg.text)
        self.assertIn("days: 7", cfg.text)
        self.assertEqual(cfg.mode, 0o600)                                     # the camera URLs are secrets
        self.assertTrue(cfg.once)
        _, (compose2, extra2) = self.render("frigate", devices=("dri", "coral_usb"), detector="openvino_gpu", hwaccel="preset-vaapi")
        self.assertIn("    devices:\n      - /dev/dri:/dev/dri\n      - /dev/bus/usb:/dev/bus/usb\n", compose2.text)
        self.assertIn("type: openvino", extra2[0].text)
        self.assertIn("hwaccel_args: preset-vaapi", extra2[0].text)
        self.assertNotIn("${", compose2.text + extra2[0].text)

    def test_secret_never_reaches_the_compose_file_but_files_that_use_it_are_0600(self):
        url = "rtsp://cam:s3cret%40pw@192.168.1.20:554/stream1"
        _, (compose, extra) = self.render("frigate", cam1_enabled=True, cam1_url=url)
        self.assertNotIn("s3cret", compose.text)
        self.assertIn(url, extra[0].text)
        self.assertEqual(extra[0].mode, 0o600)
        self.assertEqual(compose.mode, 0o644)

    def test_plex_modes_and_effective_privileges(self):
        a, (compose, _) = self.render("plex", network="host")
        self.assertIn("    network_mode: host\n", compose.text)
        self.assertIn('PLEX_UID: "1000"', compose.text)
        self.assertIn("/srv/data/media:/data:ro", compose.text)
        self.assertEqual(compose.mode, 0o600)                              # PLEX_CLAIM is a secret placed in the compose file
        _, (compose2, _) = self.render("plex", network="bridge", media_access="rw", plex_claim="claim-abcdefgh12")
        self.assertNotIn("network_mode", compose2.text)
        self.assertIn('      - "32400:32400/tcp"', compose2.text)
        self.assertIn('PLEX_CLAIM: "claim-abcdefgh12"', compose2.text)
        self.assertIn(":/data:rw", compose2.text)
        self.assertIn("host-network", A.effective_privileges(a, {"network": "host"}))

    def test_json_form_is_quoted_and_dollar_safe_in_compose(self):
        mf = dict(BASE, settings=[{"key": "pw", "label": "P", "type": "secret", "default": ""}])
        a = parse(mf, compose='env:\n  PW: ${pw_json}\n')
        c, _ = A.render_addon(a, {"pw": 'p"w$x\\'}, {}, [], self.ident)
        self.assertIn('PW: "p\\"w$$x\\\\"', c.text)                        # $ doubled so compose does not interpolate

    def test_rendered_output_is_audited_again(self):
        mf = dict(BASE, settings=[{"key": "blk", "label": "B", "type": "choice", "default": "a", "choices": [
            {"value": "a", "vars": {"X_BLOCK": "    image: demo\n"}}, {"value": "b", "vars": {"X_BLOCK": "    privileged: true\n"}}]}])
        with self.assertRaises(A.AddonError):
            parse(mf, compose="services:\n  d:\n${X_BLOCK}")                  # caught when the add-on is loaded already
        # a rendered value that sneaks a mount in is caught at render time
        a = parse(dict(BASE, settings=[{"key": "p", "label": "P", "type": "path", "default": "/srv/data/x"}]), compose="    volumes:\n      - ${p}:/x\n")
        c, _ = A.render_addon(a, {"p": "/srv/data/x"}, {}, [], self.ident)
        self.assertIn("/srv/data/x:/x", c.text)

    def test_choice_variables_are_dollar_escaped_in_compose(self):
        mf = dict(BASE, settings=[{"key": "c", "label": "C", "type": "choice", "default": "a", "choices": [{"value": "a", "vars": {"X_BLOCK": "    command: echo $HOME\n"}}]}])
        a = parse(mf, compose="services:\n  d:\n${X_BLOCK}")
        c, _ = A.render_addon(a, {"c": "a"}, {}, [], self.ident)
        self.assertIn("echo $$HOME", c.text)

    def test_once_files_are_not_overwritten(self):
        a, (compose, extra) = self.render("frigate")
        self.assertTrue(A.write_rendered(self.env, a, extra[0], self.ident))
        path = self.env.p("/srv/addons/frigate/config/config.yml")
        with open(path, "a") as fh:
            fh.write("# my edit\n")
        self.assertFalse(A.write_rendered(self.env, a, extra[0], self.ident))
        self.assertIn("# my edit", open(path).read())
        self.assertEqual(stat.S_IMODE(os.stat(path).st_mode), 0o600)

    def test_writing_through_a_link_is_refused(self):
        a, (compose, extra) = self.render("frigate")
        os.makedirs(self.env.p("/srv/addons/frigate"))
        outside = os.path.join(self.sb.tmp, "outside")
        os.makedirs(outside)
        os.symlink(outside, self.env.p("/srv/addons/frigate/config"))
        with self.assertRaises(A.AddonError) as cm:
            A.write_rendered(self.env, a, extra[0], self.ident)
        self.assertIn("outside", str(cm.exception))
        self.assertEqual(os.listdir(outside), [])


class ValidateSettingsTests(unittest.TestCase):
    def setUp(self):
        self.a = real_catalog().get("frigate")

    def test_defaults_and_previous_values(self):
        values, secrets, plain = A.validate_settings(self.a, {"retain_days": "10", "cam1_url": "rtsp://h/x"})
        self.assertEqual(values["retain_days"], 10)
        self.assertEqual(secrets["cam1_url"], "rtsp://h/x")
        self.assertNotIn("cam1_url", plain)
        self.assertEqual(plain["detector"], "cpu")
        v2, s2, _ = A.validate_settings(self.a, {"cam1_url": ""}, plain, secrets)               # empty secret keeps the stored one
        self.assertEqual(s2["cam1_url"], "rtsp://h/x")
        self.assertEqual(v2["retain_days"], 10)

    def test_rejections(self):
        for body, text in (({"nope": 1}, "unknown setting"), ({"retain_days": 0}, "between 1 and 365"), ({"detector": "gpu9"}, "must be one of"),
                           ({"cam1_url": "exec:ffmpeg -i x"}, "expected format"), ({"cam1_url": 'rtsp://h/"x'}, "expected format"),
                           ({"cam1_url": "rtsp://h/$x"}, "expected format"), ({"settings": 1}, "unknown")):
            with self.subTest(body=body), self.assertRaises(A.AddonError) as cm:
                A.validate_settings(self.a, body)
            self.assertIn(text, str(cm.exception))
        plex = real_catalog().get("plex")
        with self.assertRaises(A.AddonError) as cm:
            A.validate_settings(plex, {"media_dir": ""})
        self.assertIn("required", str(cm.exception))


class DeviceTests(SandboxTest):
    def setUp(self):
        super().setUp()
        self.a = real_catalog().get("frigate")

    def usb(self, name, vid, pid):
        d = os.path.join(self.sb.sys, "bus/usb/devices", name)
        os.makedirs(d)
        for k, v in (("idVendor", vid), ("idProduct", pid)):
            with open(os.path.join(d, k), "w") as fh:
                fh.write(v + "\n")

    def test_detection_by_path_and_usb(self):
        states = {d["id"]: d["present"] for d in A.device_states(self.env, self.a)}
        self.assertEqual(states, {"dri": False, "coral_usb": False, "coral_pcie": False})
        os.makedirs(os.path.join(self.sb.dev, "dri"))
        open(os.path.join(self.sb.dev, "dri/renderD128"), "w").close()
        self.usb("1-2", "18d1", "9302")                       # a Coral that already enumerated under its second id
        self.usb("1-3", "046d", "c52b")
        states = {d["id"]: d["present"] for d in A.device_states(self.env, self.a)}
        self.assertEqual(states, {"dri": True, "coral_usb": True, "coral_pcie": False})

    def test_default_uses_present_devices_and_missing_coral_does_not_block(self):
        chosen, warns = A.choose_devices(self.env, self.a, None)
        self.assertEqual((chosen, warns), ([], []))
        os.makedirs(os.path.join(self.sb.dev, "dri"))
        open(os.path.join(self.sb.dev, "dri/renderD128"), "w").close()
        chosen, _ = A.choose_devices(self.env, self.a, None)
        self.assertEqual([d["id"] for d in chosen], ["dri"])
        chosen, _ = A.choose_devices(self.env, self.a, {"dri": False})
        self.assertEqual(chosen, [])

    def test_forcing_an_absent_device_warns(self):
        chosen, warns = A.choose_devices(self.env, self.a, {"coral_usb": True})
        self.assertEqual([d["id"] for d in chosen], ["coral_usb"])
        self.assertIn("was not found", warns[0])
        with self.assertRaises(A.AddonError):
            A.choose_devices(self.env, self.a, {"nope": True})

    def test_block_shape(self):
        self.assertEqual(A.devices_block([]), "")
        self.assertEqual(A.devices_block([self.a.devices[0]]), "    devices:\n      - /dev/dri:/dev/dri\n")


class IdentityTests(SandboxTest):
    def test_puid_is_never_root(self):
        self.assertEqual(self.env.identity()["puid"], 1000)
        self.sb.write("/etc/passwd", "root:x:0:0::/root:/bin/bash\n")
        self.sb.write("/etc/group", "sudo:x:27:root\n")
        ident = self.env.identity()
        self.assertEqual((ident["puid"], ident["pgid"]), (1000, 1000))
        self.sb.write("/etc/passwd", "root:x:0:0::/root:/bin/bash\nbob:x:1234:0::/home/bob:/bin/bash\n")
        self.sb.write("/etc/group", "sudo:x:27:bob\n")
        ident = self.env.identity()
        self.assertEqual(ident["puid"], 1234)
        self.assertNotEqual(ident["pgid"], 0)

    def test_timezone(self):
        self.sb.write("/etc/timezone", "Europe/Stockholm\n")
        self.assertEqual(self.env.identity()["tz"], "Europe/Stockholm")
        self.sb.write("/etc/timezone", "Bad Zone;rm\n")
        self.assertEqual(self.env.identity()["tz"], "UTC")


# ============================================================================ docker argv

class DockerArgvTests(SandboxTest):
    def test_compose_commands_are_exact_argv_lists(self):
        d = A.Docker(self.env)
        f = "/srv/addons/x/compose.yml"
        d.config("synofurb-x", f)
        d.pull("synofurb-x", f)
        d.up("synofurb-x", f)
        d.stop("synofurb-x", f)
        d.restart("synofurb-x", f)
        d.down("synofurb-x", f, volumes=True)
        d.ps("synofurb-x", f)
        d.logs("synofurb-x", f, 50)
        base = ["docker", "compose"]
        tail = ["-p", "synofurb-x", "-f", f]
        flags = ["--ansi", "never", "--progress", "plain"]
        calls = self.sb.runner.calls
        self.assertEqual(calls[0], base + tail + ["config", "-q"])
        self.assertEqual(calls[1], base + flags + tail + ["pull"])
        self.assertEqual(calls[2], base + flags + tail + ["up", "-d", "--remove-orphans"])
        self.assertEqual(calls[3], base + tail + ["stop"])
        self.assertEqual(calls[4], base + tail + ["restart"])
        self.assertEqual(calls[5], base + tail + ["down", "--remove-orphans", "-v"])
        self.assertEqual(calls[6], base + tail + ["ps", "-a", "--format", "json"])
        self.assertEqual(calls[7], base + tail + ["logs", "--no-color", "--timestamps", "--tail", "50"])

    def test_names_and_paths_are_validated(self):
        d = A.Docker(self.env)
        for project, f in (("x; rm", "/srv/a/compose.yml"), ("synofurb-X", "/srv/a/compose.yml"), ("synofurb-x", "relative.yml"),
                           ("synofurb-x", "/srv/../etc/compose.yml"), ("synofurb-x", "/srv/a b/compose.yml"), ("synofurb-x", "/srv/a;id/compose.yml")):
            with self.subTest(project=project, f=f), self.assertRaises(A.AddonError):
                d.up(project, f)
        self.assertEqual(self.sb.runner.calls, [])

    def test_ps_parsing_both_formats(self):
        nd = '{"Name":"a-1","Service":"a","State":"running","Health":"healthy"}\n{"Name":"b-1","Service":"b","State":"exited","ExitCode":2}\n'
        arr = json_array = json.dumps([{"Name": "a-1", "State": "Running"}])
        self.assertEqual([c["state"] for c in A.parse_ps(nd)], ["running", "exited"])
        self.assertEqual(A.parse_ps(nd)[1]["exit_code"], 2)
        self.assertEqual(A.parse_ps(arr)[0]["state"], "running")
        self.assertEqual(A.parse_ps("garbage"), [])

    def test_no_shell_anywhere(self):
        src = open(os.path.join(ROOT, "synofurb_addons.py")).read()
        self.assertNotIn("shell=True", src.replace("shell=False", ""))
        self.assertNotIn("os.system", src)


# ============================================================================ lifecycle

class LifecycleTests(SandboxTest):
    def test_install_portainer_end_to_end(self):
        self.sb.install_docker()
        job = self.install("portainer")
        self.assertTrue(job.ok, job.error)
        self.assertEqual(job.state, "done")
        self.assertEqual(job.steps, ["check", "render", "pull", "start", "health"])
        self.assertEqual([c[0] for c in self.sb.docker.calls if c[0] in ("config", "pull", "up")], ["config", "pull", "up"])
        self.assertTrue(self.sb.exists("/srv/addons/portainer/compose.yml"))
        self.assertFalse(self.sb.exists("/srv/addons/portainer/compose.next.yml"))
        rec = self.mgr.store.get("portainer")
        self.assertEqual((rec["status"], rec["version"], rec["privileges"]), ("installed", "1", ["docker-socket"]))
        self.assertEqual(self.mgr.status("portainer")["status"], "running")
        kinds = [a[2] for a in self.sb.audit]
        self.assertEqual(kinds, ["addons.job.start", "addons.job.install"])
        start = self.sb.audit[0]
        self.assertEqual((start[0], start[1]), ("tester", "1.2.3.4"))
        self.assertEqual(start[3]["privileges"], ["docker-socket"])
        self.assertIs(start[3]["accepted"], True)
        self.assertEqual(self.sb.audit[1][4], "ok")

    def test_requirement_privileges_and_duplicate_are_checked_before_anything_changes(self):
        with self.assertRaises(A.AddonError) as cm:
            self.install("portainer")
        self.assertIn("needs Docker to be installed first", str(cm.exception))
        self.sb.install_docker()
        with self.assertRaises(A.AddonError) as cm:
            self.mgr.install("portainer", {}, background=False)
        self.assertIn("needs these privileges: docker-socket", str(cm.exception))
        self.assertEqual(self.sb.docker.calls, [])
        self.assertFalse(self.sb.exists("/srv/addons/portainer"))
        self.assertIsNone(self.mgr.store.get("portainer"))
        self.install("portainer")
        with self.assertRaises(A.AddonError) as cm:
            self.install("portainer")
        self.assertIn("already installed", str(cm.exception))

    def test_accept_can_be_the_exact_list(self):
        self.sb.install_docker()
        with self.assertRaises(A.AddonError):
            self.mgr.install("portainer", {"accept": ["privileged"]}, background=False)
        self.assertTrue(self.mgr.install("portainer", {"accept": ["docker-socket"]}, background=False).ok)

    def test_unknown_addon_and_ids_that_are_not_ids(self):
        for bad in ("nope", "../x", "a b", ""):
            with self.assertRaises(A.NotFound):
                self.mgr.install(bad, {}, background=False)

    def test_bulk_storage_needs_the_data_volume(self):
        self.sb.install_docker()
        self.sb.mounts[:] = [{"target": "/", "source": "/dev/md0", "fstype": "ext4"}]             # no volume at /srv/data
        with self.assertRaises(A.AddonError) as cm:
            self.install("plex", {"settings": {"media_dir": "/srv/data/media"}})
        msg = str(cm.exception)
        self.assertTrue("folder does not exist" in msg or "redundant data volume" in msg, msg)
        os.makedirs(self.env.p("/srv/data/media"))
        with self.assertRaises(A.AddonError) as cm:
            self.install("plex", {"settings": {}})
        self.assertIn("redundant data volume", str(cm.exception))
        with self.assertRaises(A.AddonError) as cm:
            self.install("plex", {"storage": {"transcode": "/srv/addons/plex/transcode"}})          # a user override outside /srv/data
        self.assertIn("below /srv/data", str(cm.exception))
        self.sb.mounts.append({"target": "/srv/data", "source": "/dev/mapper/vg_data-data", "fstype": "xfs"})
        job = self.install("plex")
        self.assertTrue(job.ok, job.error)
        self.assertTrue(os.path.isdir(self.env.p("/srv/data/addons/plex/transcode")))              # the loader creates the storage directory
        self.assertEqual(self.mgr.store.get("plex")["storage"], {"transcode": "/srv/data/addons/plex/transcode"})

    def test_path_settings_must_exist(self):
        self.sb.install_docker()
        with self.assertRaises(A.AddonError) as cm:
            self.install("plex")
        self.assertIn("does not exist", str(cm.exception))

    def test_plex_privilege_follows_the_chosen_network(self):
        self.sb.install_docker()
        os.makedirs(self.env.p("/srv/data/media"))
        # the shipped manifest declares host-network at the top; both modes are therefore accepted only with the acknowledgement
        with self.assertRaises(A.AddonError):
            self.mgr.install("plex", {}, background=False)
        self.assertTrue(self.mgr.install("plex", {"accept": True, "settings": {"plex_claim": "claim-abcdefgh12", "network": "bridge"}}, background=False).ok)
        self.assertNotIn("network_mode", self.sb.read("/srv/addons/plex/compose.yml"))

    def test_secret_values_are_masked_everywhere(self):
        self.sb.install_docker()
        os.makedirs(self.env.p("/srv/data/media"))
        secret = "claim-SUPERSECRET99"
        job = self.mgr.install("plex", {"accept": True, "settings": {"plex_claim": secret}}, user="u", background=False)
        self.assertTrue(job.ok, job.error)
        state = self.sb.read("/var/lib/synofurb-mgmt/addons.json")
        self.assertNotIn(secret, state)
        self.assertIn("plex_claim", json.loads(state)["addons"]["plex"]["secret_keys"])
        sec_path = self.env.p("/var/lib/synofurb-mgmt/addons-secrets.json")
        self.assertEqual(stat.S_IMODE(os.stat(sec_path).st_mode), 0o600)
        self.assertEqual(json.load(open(sec_path))["plex"]["plex_claim"], secret)
        compose = self.env.p("/srv/addons/plex/compose.yml")
        self.assertEqual(stat.S_IMODE(os.stat(compose).st_mode), 0o600)
        self.assertNotIn(secret, json.dumps(job.view()))
        self.assertNotIn(secret, json.dumps(self.sb.audit))
        d = self.mgr.show("plex")
        self.assertEqual(d["values"]["plex_claim"], {"set": True})
        self.assertNotIn(secret, json.dumps(d))
        self.sb.docker.log_text["synofurb-plex"] = "boot with %s and again %s\n" % (secret, secret)
        text = self.mgr.logs("plex")
        self.assertNotIn(secret, text)
        self.assertIn("***", text)
        job.line("again " + secret)
        self.assertNotIn(secret, json.dumps(job.view()))

    def test_failed_pull_leaves_needs_attention_and_install_can_be_retried(self):
        self.sb.install_docker()
        self.sb.docker.fail["pull"] = "dial tcp: lookup ghcr.io: no such host"
        job = self.install("portainer")
        self.assertFalse(job.ok)
        self.assertEqual(job.state, "failed")
        self.assertIn("could not be downloaded", job.error)
        self.assertIn("no such host", job.error)
        rec = self.mgr.store.get("portainer")
        self.assertEqual(rec["status"], "needs-attention")
        self.assertEqual(self.mgr.status("portainer")["status"], "needs-attention")
        self.assertEqual(self.sb.audit[-1][4].startswith("error:"), True)
        del self.sb.docker.fail["pull"]
        job = self.install("portainer")
        self.assertTrue(job.ok, job.error)
        self.assertEqual(self.mgr.store.get("portainer")["status"], "installed")

    def test_invalid_generated_compose_keeps_the_old_file(self):
        self.sb.install_docker()
        self.install("portainer")
        good = self.sb.read("/srv/addons/portainer/compose.yml")
        self.sb.docker.fail["config"] = "yaml: line 3: mapping values are not allowed"
        job = self.mgr.update("portainer", {}, background=False)
        self.assertFalse(job.ok)
        self.assertIn("not valid", job.error)
        self.assertEqual(self.sb.read("/srv/addons/portainer/compose.yml"), good)
        self.assertFalse(self.sb.exists("/srv/addons/portainer/compose.next.yml"))

    def test_start_stop_restart_update(self):
        self.sb.install_docker()
        self.install("portainer")
        self.assertTrue(self.mgr.control("portainer", "stop", background=False).ok)
        self.assertEqual(self.mgr.status("portainer")["status"], "stopped")
        self.assertTrue(self.mgr.control("portainer", "start", background=False).ok)
        self.assertEqual(self.mgr.status("portainer")["status"], "running")
        self.assertTrue(self.mgr.control("portainer", "restart", background=False).ok)
        n = len(self.sb.docker.calls)
        job = self.mgr.update("portainer", {}, background=False)
        self.assertTrue(job.ok, job.error)
        self.assertEqual([c[0] for c in self.sb.docker.calls[n:] if c[0] in ("config", "pull", "up")], ["config", "pull", "up"])
        with self.assertRaises(A.AddonError):
            self.mgr.control("frigate", "stop", background=False)
        with self.assertRaises(A.AddonError):
            self.mgr.control("portainer", "explode", background=False)

    def test_remove_keeps_data_unless_asked(self):
        self.sb.install_docker()
        self.install("portainer")
        os.makedirs(self.env.p("/srv/addons/portainer/data"))
        open(self.env.p("/srv/addons/portainer/data/db"), "w").close()
        job = self.mgr.remove("portainer", False, background=False)
        self.assertTrue(job.ok, job.error)
        self.assertIsNone(self.mgr.store.get("portainer"))
        self.assertTrue(self.sb.exists("/srv/addons/portainer/data/db"))
        self.assertFalse(self.sb.exists("/srv/addons/portainer/compose.yml"))
        self.assertIn(("down", "synofurb-portainer", self.env.p("/srv/addons/portainer/compose.yml"), False), self.sb.docker.calls)
        self.install("portainer")
        job = self.mgr.remove("portainer", True, background=False)
        self.assertTrue(job.ok, job.error)
        self.assertFalse(self.sb.exists("/srv/addons/portainer"))
        self.assertIn(("down", "synofurb-portainer", self.env.p("/srv/addons/portainer/compose.yml"), True), self.sb.docker.calls)

    def test_delete_data_only_touches_the_addons_own_folders(self):
        self.sb.install_docker()
        os.makedirs(self.env.p("/srv/data/media"))
        open(self.env.p("/srv/data/media/film.mkv"), "w").close()
        self.install("plex")
        self.mgr.store.update("plex", storage={"transcode": "/srv/data/media"})                  # a hand-edited state must not make us delete media
        job = self.mgr.remove("plex", True, background=False)
        self.assertTrue(job.ok, job.error)
        self.assertTrue(self.sb.exists("/srv/data/media/film.mkv"))
        self.assertIn("outside this add-on", " ".join(job.log))

    def test_dependents_block_stop_and_remove_of_docker(self):
        self.sb.install_docker()
        self.install("portainer")
        with self.assertRaises(A.AddonError) as cm:
            self.mgr.remove("docker", background=False)
        self.assertIn("used by portainer", str(cm.exception))
        with self.assertRaises(A.AddonError) as cm:
            self.mgr.control("docker", "stop", background=False)
        self.assertIn("used by portainer", str(cm.exception))
        self.mgr.remove("portainer", background=False)
        with self.assertRaises(A.AddonError) as cm:
            self.mgr.remove("docker", background=False)
        self.assertIn("apt-get remove", str(cm.exception))

    def test_status_mapping(self):
        self.sb.install_docker()
        self.install("portainer")
        proj = self.sb.docker.projects["synofurb-portainer"]
        self.sb.http["status"] = None
        self.assertEqual(self.mgr.status("portainer")["status"], "starting")                      # just started
        self.sb.clock.t += 1000
        self.assertEqual(self.mgr.status("portainer")["status"], "unhealthy")
        self.sb.http["status"] = 503
        self.assertEqual(self.mgr.status("portainer")["status"], "unhealthy")
        self.sb.http["status"] = 401                                                              # < 500 counts as up
        self.assertEqual(self.mgr.status("portainer")["status"], "running")
        proj["state"] = "restarting"
        self.assertEqual(self.mgr.status("portainer")["status"], "error")
        proj["state"] = "exited"
        self.assertEqual(self.mgr.status("portainer")["status"], "stopped")
        self.assertEqual(self.mgr.status("frigate")["status"], "not-installed")

    def test_health_wait_uses_the_manifest_timeout_and_only_warns(self):
        self.sb.install_docker()
        self.sb.http["status"] = None
        t0 = self.sb.clock.t
        job = self.install("portainer")
        self.assertTrue(job.ok, job.error)
        self.assertGreaterEqual(self.sb.clock.t - t0, 180)
        self.assertTrue(any("health check does not answer" in w for w in job.warnings))

    def test_health_check_types(self):
        a = real_catalog().get("frigate")
        self.assertEqual(a.healthcheck["type"], "http")
        self.sb.http["status"] = 200
        self.assertTrue(self.mgr.probe(a))
        tcp = parse(healthcheck={"type": "tcp", "port": 80})
        self.assertTrue(self.mgr.probe(tcp))
        self.sb.http["status"] = None
        self.assertFalse(self.mgr.probe(tcp))
        cont = parse()
        self.sb.docker.projects["synofurb-demo"] = {"state": "running", "file": "", "services": ["a"]}
        open(self.mgr.compose_file(cont), "a") if os.path.isdir(os.path.dirname(self.mgr.compose_file(cont))) else None
        self.assertFalse(self.mgr.probe(cont, []))
        self.assertTrue(self.mgr.probe(cont, [{"state": "running", "health": ""}]))

    def test_web_link_hidden_for_loopback_bound_ports(self):
        a = parse(web={"port": 80, "scheme": "http"}, ports=[{"host": 80, "container": 80, "bind": "127.0.0.1"}])
        self.assertIsNone(self.mgr.web_link(a))
        a = parse(web={"port": 80, "scheme": "http"}, ports=[{"host": 80, "container": 80}])
        self.assertEqual(self.mgr.web_link(a)["port"], 80)

    def test_list_and_show_shapes(self):
        d = self.mgr.list()
        ids = [c["id"] for c in d["addons"]]
        self.assertEqual(ids[:2], ["docker", "portainer"])
        self.assertEqual(d["problems"], [])
        s = self.mgr.show("frigate")
        self.assertEqual(s["storage_report"][0]["path"], "/srv/data/addons/frigate/media")
        self.assertTrue(s["storage_report"][0]["redundant"])
        self.assertEqual({x["id"] for x in s["device_states"]}, {"dri", "coral_usb", "coral_pcie"})
        self.assertEqual([p["id"] for p in s["privileges"]], [])
        self.assertTrue(next(x for x in s["settings"] if x["key"] == "detector")["first_install_only"])
        self.assertNotIn("DETECTOR_BLOCK", json.dumps(s))

    def test_storage_report_flags_low_space_and_wrong_volume(self):
        a = real_catalog().get("frigate")
        rep = A.storage_report(self.env, a, {})
        self.assertEqual(rep[0]["error"], "")
        self.sb.mounts[:] = [{"target": "/", "source": "/dev/md0", "fstype": "ext4"}]
        rep = A.storage_report(self.env, a, {})
        self.assertIn("not on a redundant data volume", rep[0]["error"])

    def test_interrupted_operations_are_marked_on_recovery(self):
        self.mgr.store.update("portainer", status="installing", version="1")
        self.mgr.recover()
        rec = self.mgr.store.get("portainer")
        self.assertEqual(rec["status"], "needs-attention")
        self.assertIn("interrupted", rec["error"])


class JobTests(SandboxTest):
    def test_second_job_is_refused_while_one_runs(self):
        self.sb.install_docker()
        gate, started = threading.Event(), threading.Event()
        orig = self.sb.docker.pull

        def slow_pull(project, f, on_line=None):
            started.set()
            gate.wait(10)
            return orig(project, f, on_line)
        self.sb.docker.pull = slow_pull
        job = self.mgr.install("portainer", {"accept": True}, user="a", background=True)
        self.assertTrue(started.wait(5))
        self.assertEqual(self.mgr.jobs.active().id, job.id)
        view = self.mgr.jobs.get(job.id).view()
        self.assertEqual((view["state"], view["step"]), ("running", "pull"))
        self.assertEqual(self.mgr.list()["job"]["id"], job.id)
        with self.assertRaises(A.Busy) as cm:
            self.mgr.control("docker", "restart", background=False)
        self.assertIn("Another job is running (install portainer)", str(cm.exception))
        with self.assertRaises(A.Busy):
            self.mgr.update("portainer", {}, background=False)
        gate.set()
        for _ in range(100):
            if self.mgr.jobs.get(job.id).state != "running":
                break
            threading.Event().wait(0.05)
        self.assertEqual(self.mgr.jobs.get(job.id).state, "done")
        self.assertIsNone(self.mgr.jobs.active())
        self.assertTrue(self.mgr.control("portainer", "restart", background=False).ok)

    def test_lock_is_shared_with_other_job_kinds(self):
        class Held:
            def acquire(self, kind, label, job_id=""):
                return None

            def release(self, token):
                pass

            def holder(self):
                return {"kind": "update"}
        self.env.job_lock = Held()
        self.sb.install_docker()
        with self.assertRaises(A.Busy) as cm:
            self.install("portainer")
        self.assertIn("(update)", str(cm.exception))

    def test_flock_is_exclusive_between_holders(self):
        lk1, lk2 = A.FlockJobLock(self.env.p("/tmp/x.lock")), A.FlockJobLock(self.env.p("/tmp/x.lock"))
        t = lk1.acquire("addon", "a")
        self.assertIsNotNone(t)
        self.assertIsNone(lk2.acquire("addon", "b"))
        self.assertIsNotNone(lk2.holder())
        lk1.release(t)
        t2 = lk2.acquire("addon", "b")
        self.assertIsNotNone(t2)
        lk2.release(t2)
        self.assertIsNone(lk1.holder())

    def test_lock_is_released_after_a_failure_and_an_exception(self):
        self.sb.install_docker()
        self.sb.docker.up = lambda *a, **k: (_ for _ in ()).throw(RuntimeError("boom"))
        import contextlib
        with contextlib.redirect_stderr(io.StringIO()):
            job = self.install("portainer")
        self.assertFalse(job.ok)
        self.assertEqual(job.error, "internal error; see the job log and the journal")        # no internals to the person
        self.assertIsNone(self.env.job_lock.holder())

    def test_finished_jobs_survive_a_restart(self):
        self.sb.install_docker()
        job = self.install("portainer")
        mgr2 = A.Manager(self.env, self.sb.docker)
        j = mgr2.jobs.get(job.id)
        self.assertEqual((j.state, j.ok, j.kind, j.addon), ("done", True, "install", "portainer"))
        self.assertTrue(any("Waiting for Portainer" in x for x in j.log))

    def test_a_job_that_was_running_at_a_crash_comes_back_failed(self):
        d = self.sb.env.state_path("addons-jobs.json")
        os.makedirs(os.path.dirname(d), exist_ok=True)
        with open(d, "w") as fh:
            json.dump({"schema": 1, "jobs": [{"id": "j" + "a" * 12, "kind": "install", "addon": "x", "state": "running", "steps": ["a"], "log": []}]}, fh)
        mgr = A.Manager(self.env, self.sb.docker)
        self.assertEqual(mgr.jobs.get("j" + "a" * 12).state, "failed")


# ============================================================================ docker add-on

class DockerHandlerTests(SandboxTest):
    def setUp(self):
        super().setUp()
        self.r = self.sb.runner
        self.r.on(["dpkg-query"], "", rc=1)
        self.r.on(["apt-cache", "policy"], "  Candidate: 26.1.5+dfsg1-9+deb13u1\n")
        self.r.on(["id", "-nG"], "admin sudo users\n")

    def apt_calls(self):
        return [c for c in self.r.calls if "apt-get" in c]

    def test_fresh_install_sequence(self):
        job = self.install("docker")
        self.assertTrue(job.ok, job.error)
        self.assertEqual(job.steps, ["check", "packages", "service", "group", "verify"])
        apt = self.apt_calls()
        self.assertEqual(apt[0][:3], ["env", "DEBIAN_FRONTEND=noninteractive", "NEEDRESTART_MODE=a"])
        self.assertIn("update", apt[0])
        inst = apt[1]
        self.assertEqual(inst[inst.index("install") + 1:], ["--no-install-recommends", "ca-certificates", "docker.io", "docker-cli", "docker-compose"])
        self.assertIn("-y", inst)
        self.assertTrue(self.r.called("systemctl", "enable", "--now", "docker.service"))
        self.assertTrue(self.r.called("usermod", "-aG", "docker", "admin"))
        self.assertEqual(self.mgr.store.get("docker")["status"], "installed")
        self.assertIn("log-opts", self.sb.read("/etc/docker/daemon.json"))
        self.assertTrue(self.r.called("systemctl", "restart", "docker.service"))               # daemon.json was new

    def test_idempotent_when_everything_is_in_place(self):
        self.r.on(["dpkg-query"], "ii ")
        self.sb.write("/etc/docker/daemon.json", '{"my": "own"}\n')
        self.r.on(["id", "-nG"], "admin sudo docker\n")
        job = self.install("docker")
        self.assertTrue(job.ok, job.error)
        self.assertEqual(self.apt_calls(), [])
        self.assertFalse(self.r.called("usermod"))
        self.assertFalse(self.r.called("systemctl", "restart", "docker.service"))
        self.assertEqual(self.sb.read("/etc/docker/daemon.json"), '{"my": "own"}\n')           # an existing daemon.json is never touched
        with self.assertRaises(A.AddonError):                                                      # once it is recorded, install says so
            self.install("docker")

    def test_missing_candidate_gives_a_clear_message(self):
        self.r.on(["apt-cache", "policy"], "  Candidate: (none)\n")
        job = self.install("docker")
        self.assertFalse(job.ok)
        self.assertIn("not available from the configured package sources", job.error)
        self.assertEqual(self.mgr.store.get("docker")["status"], "needs-attention")
        self.assertFalse(any("install" in c for c in self.apt_calls()))

    def test_apt_failure_message(self):
        self.r.on(["env", "DEBIAN_FRONTEND=noninteractive", "NEEDRESTART_MODE=a", "apt-get", "-y", "-o", "Dpkg::Options::=--force-confold", "-o",
                   "Dpkg::Options::=--force-confdef", "install"], "", rc=100, err="E: Unable to locate package docker.io")
        job = self.install("docker")
        self.assertFalse(job.ok)
        self.assertIn("apt-get could not install Docker", job.error)

    def test_failed_apt_update_is_only_a_warning(self):
        self.r.on(["env", "DEBIAN_FRONTEND=noninteractive", "NEEDRESTART_MODE=a", "apt-get", "-y", "-o", "Dpkg::Options::=--force-confold", "-o",
                   "Dpkg::Options::=--force-confdef", "update"], "", rc=100, err="Temporary failure resolving 'deb.debian.org'")
        job = self.install("docker")
        self.assertTrue(job.ok, job.error)
        self.assertTrue(any("package lists" in w for w in job.warnings))

    def test_engine_without_compose_plugin_fails_the_verify_step(self):
        self.sb.docker.compose = False
        job = self.install("docker")
        self.assertFalse(job.ok)
        self.assertIn("compose plugin is missing", job.error)

    def test_no_administrator_is_a_warning(self):
        self.sb.write("/etc/group", "root:x:0:\nsudo:x:27:\n")
        job = self.install("docker")
        self.assertTrue(job.ok, job.error)
        self.assertTrue(any("usermod -aG docker" in w for w in job.warnings))
        self.assertFalse(self.r.called("usermod"))

    def test_start_stop_restart_status_logs(self):
        self.install("docker")
        self.r.on(["systemctl", "is-active"], "active\n")
        self.assertEqual(self.mgr.status("docker")["status"], "running")
        self.assertTrue(self.mgr.control("docker", "restart", background=False).ok)
        self.assertTrue(self.mgr.control("docker", "stop", background=False).ok)
        self.assertTrue(self.r.called("systemctl", "stop", "docker.service", "docker.socket"))
        self.r.on(["systemctl", "is-active"], "inactive\n", rc=3)
        self.assertEqual(self.mgr.status("docker")["status"], "stopped")
        self.r.on(["journalctl"], "Nov 3 dockerd started\n")
        self.assertIn("dockerd started", self.mgr.logs("docker"))

    def test_update_upgrades_the_packages(self):
        self.r.on(["dpkg-query"], "ii ")
        self.install("docker")
        self.r.calls.clear()
        self.assertTrue(self.mgr.update("docker", {}, background=False).ok)
        up = [c for c in self.apt_calls() if "install" in c][0]
        self.assertIn("--only-upgrade", up)
        self.assertEqual(up[-3:], ["docker.io", "docker-cli", "docker-compose"])

    def test_package_names_are_the_debian_13_ones(self):
        self.assertEqual(A.DOCKER_PACKAGES, ("docker.io", "docker-cli", "docker-compose"))


# ============================================================================ apply-pending

class PendingTests(SandboxTest):
    def pending(self, aid, doc=None, mode=0o644):
        p = self.env.p("/etc/synofurb/addons.d/pending/%s.json" % aid)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as fh:
            json.dump(doc if doc is not None else {"schema": 1, "id": aid, "settings": {}}, fh)
        os.chmod(p, mode)
        return p

    def result(self):
        return json.load(open(self.env.state_path("addons-apply.json")))

    def test_installs_in_dependency_order_and_removes_the_files(self):
        self.sb.runner.on(["dpkg-query"], "ii ")
        p1, p2 = self.pending("portainer"), self.pending("docker")
        doc = A.apply_pending(self.mgr)
        self.assertEqual([(r["id"], r["status"]) for r in doc["items"]], [("docker", "installed"), ("portainer", "installed")])
        self.assertTrue(doc["ok"])
        self.assertFalse(os.path.exists(p1) or os.path.exists(p2))
        self.assertEqual(self.result()["items"][1]["attempts"], 1)
        self.assertEqual(self.mgr.store.get("portainer")["status"], "installed")
        accepted = [a for a in self.sb.audit if a[2] == "addons.job.start"]
        self.assertEqual({a[0] for a in accepted}, {"installer"})
        self.assertEqual(A.apply_pending(self.mgr)["items"], [])

    def test_retries_with_backoff_and_succeeds(self):
        self.sb.runner.on(["dpkg-query"], "ii ")
        self.pending("docker")
        self.pending("portainer")
        fails = {"n": 0}
        orig = self.sb.docker.pull

        def flaky(project, f, on_line=None):
            if project == "synofurb-portainer" and fails["n"] < 2:
                fails["n"] += 1
                return A.Result(1, "", "network is unreachable")
            return orig(project, f, on_line)
        self.sb.docker.pull = flaky
        slept = []
        self.env.sleep = lambda s: (slept.append(s), self.sb.clock.sleep(s))
        doc = A.apply_pending(self.mgr, attempts=3, delays=(10, 30, 90))
        por = doc["items"][1]
        self.assertEqual((por["status"], por["attempts"]), ("installed", 3))
        self.assertEqual([s for s in slept if s >= 10], [10, 30])
        self.assertEqual(self.mgr.store.get("portainer")["status"], "installed")

    def test_gives_up_after_the_attempts_keeps_the_file_and_never_raises(self):
        self.sb.runner.on(["dpkg-query"], "ii ")
        self.pending("docker")
        keep = self.pending("portainer")
        self.sb.docker.pull = lambda *a, **k: A.Result(1, "", "no route to host")
        doc = A.apply_pending(self.mgr, attempts=2, delays=(1,))
        por = doc["items"][1]
        self.assertEqual((por["status"], por["attempts"]), ("failed", 2))
        self.assertIn("no route to host", por["message"])
        self.assertFalse(doc["ok"])
        self.assertTrue(os.path.exists(keep))                                     # retried at the next boot
        self.assertEqual(self.mgr.store.get("portainer")["status"], "needs-attention")

    def test_failed_dependency_skips_dependents(self):
        self.pending("docker")
        self.pending("portainer")
        self.sb.docker.engine = False
        doc = A.apply_pending(self.mgr, attempts=1)
        self.assertEqual([r["status"] for r in doc["items"]], ["failed", "skipped"])
        self.assertIn("needs docker", doc["items"][1]["message"])

    def test_rejected_files_are_parked(self):
        self.sb.install_docker()
        bad = self.pending("portainer", {"schema": 1, "id": "portainer", "settings": {"nope": 1}})
        ghost = self.pending("ghost")
        doc = A.apply_pending(self.mgr, attempts=3)
        st = {r["id"]: r["status"] for r in doc["items"]}
        self.assertEqual(st, {"portainer": "rejected", "ghost": "rejected"})
        self.assertFalse(os.path.exists(bad) or os.path.exists(ghost))
        self.assertTrue(os.path.exists(os.path.join(os.path.dirname(bad), "rejected", "portainer.json")))

    def test_secrets_need_a_private_file(self):
        self.sb.install_docker()
        os.makedirs(self.env.p("/srv/data/media"))
        doc = {"schema": 1, "id": "plex", "settings": {}, "secrets": {"plex_claim": "claim-abcdefgh12"}}
        open_ = self.pending("plex", doc, 0o644)
        out = A.apply_pending(self.mgr, attempts=1)
        self.assertEqual(out["items"][0]["status"], "rejected")
        self.assertIn("readable by others", out["items"][0]["message"])
        self.pending("plex", doc, 0o600)
        out = A.apply_pending(self.mgr, attempts=1)
        self.assertEqual(out["items"][0]["status"], "installed", out)
        self.assertEqual(self.mgr.store.secrets("plex")["plex_claim"], "claim-abcdefgh12")
        self.assertFalse(os.path.exists(open_))
        self.assertNotIn("claim-abc", json.dumps(self.result()))

    def test_already_installed_is_skipped(self):
        self.sb.install_docker()
        self.pending("docker")
        doc = A.apply_pending(self.mgr)
        self.assertEqual(doc["items"][0]["status"], "installed")
        self.assertIn("already", doc["items"][0]["message"])

    def test_cli_apply_pending_always_exits_zero(self):
        self.pending("ghost")
        out = []
        rc = A.cli_main(["apply-pending"], self.mgr, out.append)
        self.assertEqual(rc, 0)
        self.assertIn("rejected", "\n".join(out))
        self.assertEqual(A.cli_main(["apply-pending"], self.mgr, out.append), 0)


# ============================================================================ CLI

class CliTests(SandboxTest):
    def run_cli(self, *argv, stdin=""):
        out, err = [], []
        rc = A.cli_main(list(argv), self.mgr, out.append, err.append, io.StringIO(stdin))
        return rc, "\n".join(out), "\n".join(err)

    def test_list_show_status(self):
        rc, out, _ = self.run_cli("list")
        self.assertEqual(rc, 0)
        self.assertIn("portainer", out)
        self.assertIn("not-installed", out)
        rc, out, _ = self.run_cli("list", "--json")
        self.assertEqual(json.loads(out)["addons"][0]["id"], "docker")
        rc, out, _ = self.run_cli("show", "frigate")
        self.assertIn("storage media: /srv/data/addons/frigate/media", out)
        rc, out, _ = self.run_cli("status", "portainer")
        self.assertEqual((rc, out), (1, "portainer: not-installed"))
        rc, _, err = self.run_cli("show", "nope")
        self.assertEqual(rc, 2)
        self.assertIn("unknown add-on", err)

    def test_install_needs_yes_for_privileges_and_refuses_secrets_on_argv(self):
        self.sb.install_docker()
        rc, _, err = self.run_cli("install", "portainer")
        self.assertEqual(rc, 2)
        self.assertIn("docker-socket", err)
        os.makedirs(self.env.p("/srv/data/media"))
        rc, _, err = self.run_cli("install", "plex", "--set", "plex_claim=claim-abcdefgh12", "--yes")
        self.assertEqual(rc, 2)
        self.assertIn("standard input", err)
        rc, out, _ = self.run_cli("install", "plex", "--settings-stdin", "--set", "network=bridge", "--yes", stdin='{"plex_claim": "claim-abcdefgh12"}')
        self.assertEqual(rc, 0, out)
        self.assertNotIn("claim-abcdefgh12", out)
        self.assertEqual(self.mgr.store.secrets("plex")["plex_claim"], "claim-abcdefgh12")

    def test_install_start_stop_logs_update_remove(self):
        self.sb.install_docker()
        self.assertEqual(self.run_cli("install", "portainer", "--yes")[0], 0)
        rc, out, _ = self.run_cli("status", "portainer")
        self.assertEqual((rc, out), (0, "portainer: running"))
        for cmd in ("stop", "start", "restart", "update"):
            self.assertEqual(self.run_cli(cmd, "portainer")[0], 0, cmd)
        rc, out, _ = self.run_cli("logs", "portainer", "--tail", "20")
        self.assertIn("started", out)
        rc, _, err = self.run_cli("remove", "portainer", "--delete-data")
        self.assertEqual(rc, 2)
        self.assertIn("--yes", err)
        self.assertEqual(self.run_cli("remove", "portainer", "--delete-data", "--yes")[0], 0)
        self.assertIsNone(self.mgr.store.get("portainer"))

    def test_storage_and_device_options(self):
        self.sb.install_docker()
        rc, out, err = self.run_cli("install", "frigate", "--storage", "media=/srv/data/addons/frigate/rec", "--device", "dri=off", "--set", "retain_days=3", "--yes")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.mgr.store.get("frigate")["storage"], {"media": "/srv/data/addons/frigate/rec"})
        self.assertIn("days: 3", self.sb.read("/srv/addons/frigate/config/config.yml"))
        rc, _, err = self.run_cli("install", "plex", "--device", "dri=maybe")
        self.assertEqual(rc, 2)

    def test_bad_usage_exits_2(self):
        self.assertEqual(self.run_cli("frobnicate")[0], 2)
        self.assertEqual(self.run_cli("install")[0], 2)

    def test_busy_exit_code(self):
        self.sb.install_docker()
        tok = self.env.job_lock.acquire("addon", "x")
        rc, _, err = self.run_cli("install", "portainer", "--yes")
        self.env.job_lock.release(tok)
        self.assertEqual(rc, 1)
        self.assertIn("busy", err)

    def test_main_dispatches_addons(self):
        import contextlib
        root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, root, True)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = m.main(["--dry-run", "--root", root, "addons", "list", "--json"])
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(buf.getvalue())["addons"][0]["id"], "docker")
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            m.parse_args(["health", "--bogus"])


# ============================================================================ web API

class HttpAddonTests(unittest.TestCase):
    def setUp(self):
        self.env = Env()
        self.addCleanup(self.env.close)
        self.cli = Client(self.env)
        self.cli.login()
        self.sb = Sandbox()
        self.addCleanup(self.sb.close)
        # the app's own manager, but with the sandbox's collaborators (fake Docker, clock, probes); root = the app's temp root
        e = A.AddonEnv(root=self.env.tmp, run=self.sb.runner.run, mounts=lambda: [{"target": "/srv/data", "source": "/dev/mapper/vg_data-data", "fstype": "xfs"}],
                       audit=self.env.app.audit.log, script_dir=ROOT, sleep=self.sb.clock.sleep, now=self.sb.clock.now, chown=False,
                       http_probe=lambda url, timeout=4: 200, tcp_probe=lambda p, t=3: True, port_in_use=lambda p: False,
                       dev_root=self.sb.dev, sys_root=self.sb.sys, job_lock=m._AddonJobLock(self.env.cfg))
        self.mgr = self.env.app._addons = A.Manager(e, A.FakeDocker(e))
        self.mgr.store.update("docker", status="installed", version="1")

    def wait(self, jid):
        for _ in range(200):
            st, obj, _ = self.cli.call("GET", "/api/jobs/" + jid)
            self.assertEqual(st, 200)
            if obj["job"]["state"] != "running":
                return obj["job"]
            threading.Event().wait(0.05)
        self.fail("job did not finish")

    def test_requires_a_session_and_csrf(self):
        anon = Client(self.env)
        self.assertEqual(anon.call("GET", "/api/addons")[0], 401)
        self.assertEqual(anon.call("GET", "/api/addons/portainer")[0], 401)
        self.assertEqual(anon.call("GET", "/api/jobs")[0], 401)
        st, obj, _ = self.cli.call("POST", "/api/addons/portainer/install", {"accept": True}, csrf=False)
        self.assertEqual(st, 403)
        self.assertEqual(anon.call("POST", "/api/addons/portainer/install", {})[0], 401)

    def test_list_show_and_unknown(self):
        st, obj, _ = self.cli.call("GET", "/api/addons")
        self.assertEqual(st, 200)
        self.assertEqual(obj["addons"][0]["id"], "docker")
        self.assertEqual(obj["problems"], [])
        st, obj, _ = self.cli.call("GET", "/api/addons/frigate")
        self.assertEqual(st, 200)
        self.assertEqual(obj["addon"]["id"], "frigate")
        self.assertEqual(self.cli.call("GET", "/api/addons/nope")[0], 404)
        self.assertEqual(self.cli.call("GET", "/api/addons/BAD!")[0], 404)
        self.assertEqual(self.cli.call("GET", "/api/jobs/jdeadbeefdead")[0], 404)
        self.assertEqual(self.cli.call("POST", "/api/addons/portainer/frobnicate", {})[0], 404)
        self.assertEqual(self.cli.call("GET", "/api/addons/portainer/install")[0], 405)

    def test_install_job_flow_and_privileges(self):
        st, obj, _ = self.cli.call("POST", "/api/addons/portainer/install", {})
        self.assertEqual(st, 400)
        self.assertIn("docker-socket", obj["error"])
        st, obj, _ = self.cli.call("POST", "/api/addons/portainer/install", {"accept": True})
        self.assertEqual(st, 202)
        job = self.wait(obj["job"]["id"])
        self.assertEqual((job["state"], job["ok"], job["progress"]), ("done", True, 100))
        st, obj, _ = self.cli.call("GET", "/api/addons/portainer")
        self.assertEqual(obj["addon"]["status"], "running")
        self.assertEqual(obj["addon"]["link"]["port"], 9443)
        st, obj, _ = self.cli.call("GET", "/api/jobs")
        self.assertEqual(obj["jobs"][0]["kind"], "install")
        for action in ("stop", "start", "restart", "update"):
            st, obj, _ = self.cli.call("POST", "/api/addons/portainer/" + action, {})
            self.assertEqual(st, 202, action)
            self.assertTrue(self.wait(obj["job"]["id"])["ok"], action)
        st, obj, _ = self.cli.call("GET", "/api/addons/portainer/logs?tail=30")
        self.assertEqual(st, 200)
        self.assertIn("started", obj["logs"])
        self.assertEqual(self.cli.call("GET", "/api/addons/portainer/logs?tail=x")[0], 400)

    def test_audit_log_has_the_request_and_the_job_without_secret_values(self):
        os.makedirs(os.path.join(self.env.tmp, "srv/data/media"))
        body = {"accept": True, "settings": {"plex_claim": "claim-TOPSECRET99", "network": "bridge"}}
        st, obj, _ = self.cli.call("POST", "/api/addons/plex/install", body)
        self.assertEqual(st, 202, obj)
        job = self.wait(obj["job"]["id"])
        self.assertTrue(job["ok"], job)
        log = self.env.read(self.env.cfg.audit_path)
        self.assertNotIn("TOPSECRET", log)
        self.assertNotIn("TOPSECRET", json.dumps(job))
        rows = [json.loads(x) for x in log.splitlines()]
        actions = [r["action"] for r in rows]
        self.assertIn("/api/addons/plex/install", actions)
        self.assertIn("addons.job.start", actions)
        self.assertIn("addons.job.install", actions)
        req = next(r for r in rows if r["action"] == "/api/addons/plex/install")
        self.assertEqual(req["args"]["settings"], {"plex_claim": "***", "network": "bridge"})
        self.assertEqual(req["user"], "admin")
        start = next(r for r in rows if r["action"] == "addons.job.start")
        self.assertEqual(start["args"]["privileges"], ["host-network"])

    def test_remove_with_data_needs_the_typed_name(self):
        self.cli.call("POST", "/api/addons/portainer/install", {"accept": True})
        for _ in range(100):
            if self.mgr.jobs.active() is None:
                break
            threading.Event().wait(0.05)
        st, obj, _ = self.cli.call("POST", "/api/addons/portainer/remove", {"delete_data": True})
        self.assertEqual(st, 400)
        self.assertIn("type the add-on name", obj["error"])
        st, obj, _ = self.cli.call("POST", "/api/addons/portainer/remove", {"delete_data": True, "confirm": "portainer"})
        self.assertEqual(st, 202)
        self.assertTrue(self.wait(obj["job"]["id"])["ok"])
        self.assertIsNone(self.mgr.store.get("portainer"))

    def test_busy_is_409(self):
        tok = self.mgr.env.job_lock.acquire("update", "management plane")
        try:
            st, obj, _ = self.cli.call("POST", "/api/addons/portainer/install", {"accept": True})
        finally:
            self.mgr.env.job_lock.release(tok)
        self.assertEqual(st, 409)
        self.assertIn("update", obj["error"])

    def test_update_job_of_the_management_plane_is_refused_while_an_addon_job_runs(self):
        gate, started = threading.Event(), threading.Event()
        orig = self.mgr.docker.pull

        def slow(project, f, on_line=None):
            started.set()
            gate.wait(10)
            return orig(project, f, on_line)
        self.mgr.docker.pull = slow
        st, obj, _ = self.cli.call("POST", "/api/addons/portainer/install", {"accept": True})
        self.assertEqual(st, 202)
        self.assertTrue(started.wait(5))
        holder = m.JobLock(self.env.cfg).holder()
        self.assertEqual(holder["kind"], "addon")
        self.assertIsNone(m.JobLock(self.env.cfg).acquire("update", "x"))           # the update side cannot start
        gate.set()
        self.wait(obj["job"]["id"])
        self.assertIsNone(m.JobLock(self.env.cfg).holder())


class BundleTests(unittest.TestCase):
    def test_update_bundle_carries_the_module_and_the_pack(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("mub", os.path.join(ROOT, "tools", "make-update-bundle.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        out = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, out, True)
        lines = mod.build_bundle(out)
        names = [ln.split()[-1] for ln in lines]
        self.assertIn("synofurb_addons.py", names)
        self.assertIn(A.PACK_NAME, names)
        self.assertEqual(sorted(os.listdir(out)), sorted(names + ["MANIFEST.sha256"]))
        files, version = m.verify_bundle(out)                         # the unit's own verifier accepts it
        self.assertIn(A.PACK_NAME, files)
        packed = A.load_catalog([("builtin", A.read_pack(os.path.join(out, A.PACK_NAME)))], handlers=("docker",))
        self.assertEqual(packed.problems, [])
        self.assertEqual(sorted(packed.addons), sorted(real_catalog().addons))

    def test_catalog_loads_from_the_pack_when_the_folder_is_absent(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        with open(os.path.join(d, A.PACK_NAME), "w") as fh:
            fh.write(A.build_pack(ADDONS_DIR))
        env = A.AddonEnv(root=d, script_dir=d)
        cat = A.load_default_catalog(env)
        self.assertEqual(cat.problems, [])
        self.assertIn("frigate", cat.addons)


if __name__ == "__main__":
    unittest.main()
