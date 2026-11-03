"""Catalog tests for the shipped add-on definitions (apps/mgmt-plane/addons/<id>/).

Independent of the framework: a small validator written from the field rules in docs/ADDONS.md checks every manifest, then every
template is rendered with sample settings (string.Template, strict) for every combination of choice settings and the rendered text is
checked for the lines the definitions must produce. When PyYAML happens to be installed the rendered files are also parsed as YAML
(optional, the management plane itself never parses YAML). Nothing here talks to Docker, a camera or a Plex account.
"""
from __future__ import annotations

import itertools
import json
import os
import re
import string
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ADDONS_DIR = os.path.join(os.path.dirname(HERE), "addons")

try:  # optional extra check only
    import yaml  # type: ignore
except Exception:  # pragma: no cover
    yaml = None

ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,31}$")
KEY_RE = re.compile(r"^[a-z][a-z0-9_]*$")
SETTING_TYPES = {"string", "int", "bool", "secret", "choice", "path", "text"}
KINDS = {"compose", "system"}
CATEGORIES_NONEMPTY = True
HEALTH_TYPES = {"http", "tcp", "container"}
PROTOS = {"tcp", "udp"}
RESERVED = {"ADDON_ID", "DATA_DIR", "PUID", "PGID", "TZ", "DEVICES_BLOCK"}
DETECT_RE = re.compile(r"^(path:/[^\s]+|usb:[0-9a-f]{4}:[0-9a-f]{4}(\|[0-9a-f]{4}:[0-9a-f]{4})*)$")
PRIV_RE = re.compile(r"^(docker-socket|host-network|privileged|cap:[A-Z_]+)$")
DATA_VOLUME = "/srv/data"  # where the installer mounts the redundant vg_data/data LV (crates/synofurb-core/src/script.rs)

# Sample values the loader would provide.
LOADER_VARS = {"ADDON_ID": "x", "PUID": "1000", "PGID": "1000", "TZ": "Europe/Berlin"}
# Pre-indented service-level block, as the contract describes ("a ready-made compose devices: list"); empty when nothing is enabled.
DEVICES_FULL = "    devices:\n      - /dev/dri:/dev/dri\n"


def addon_ids():
    return sorted(d for d in os.listdir(ADDONS_DIR) if os.path.isfile(os.path.join(ADDONS_DIR, d, "addon.json")))


def load(addon_id):
    with open(os.path.join(ADDONS_DIR, addon_id, "addon.json"), encoding="utf-8") as f:
        return json.load(f)


def read_template(addon_id, name):
    with open(os.path.join(ADDONS_DIR, addon_id, name), encoding="utf-8") as f:
        return f.read()


def choice_options(setting):
    """Return [(value, vars)] for a choice setting; entries are plain strings or {value,label,vars[,privileges]} objects."""
    out = []
    for c in setting["choices"]:
        if isinstance(c, str):
            out.append((c, {}))
        else:
            out.append((c["value"], c.get("vars", {})))
    return out


def validate_manifest(m, folder):
    """Return a list of violations of the field rules in docs/ADDONS.md (empty: valid)."""
    errs = []
    add = errs.append
    if m.get("schema") != 1:
        add("schema must be 1")
    aid = m.get("id", "")
    if not isinstance(aid, str) or not ID_RE.match(aid):
        add("bad id %r" % (aid,))
    if aid != folder:
        add("id %r != folder %r" % (aid, folder))
    for k in ("name", "version", "summary", "category", "icon", "homepage"):
        if not isinstance(m.get(k), str) or not m[k].strip():
            add("%s must be a non-empty string" % k)
    if not str(m.get("homepage", "")).startswith("https://"):
        add("homepage must be https")
    if m.get("kind") not in KINDS:
        add("bad kind")
    req = m.get("requires", [])
    if not isinstance(req, list) or not all(isinstance(r, str) and ID_RE.match(r) for r in req):
        add("requires must be a list of ids")
    if m.get("kind") == "compose" and "docker" not in req:
        add("a compose add-on must require docker")
    if aid in req:
        add("requires itself")

    web = m.get("web")
    if web is not None:
        if web.get("scheme") not in ("http", "https"):
            add("web.scheme")
        if not (isinstance(web.get("port"), int) and 1 <= web["port"] <= 65535):
            add("web.port")
        if not str(web.get("path", "")).startswith("/"):
            add("web.path must start with /")

    ports = m.get("ports", [])
    seen = set()
    for p in ports:
        if not (isinstance(p.get("host"), int) and 1 <= p["host"] <= 65535 and isinstance(p.get("container"), int) and 1 <= p["container"] <= 65535):
            add("bad port numbers %r" % (p,))
        if p.get("proto") not in PROTOS:
            add("bad proto %r" % (p,))
        if not p.get("label"):
            add("port needs a label %r" % (p,))
        key = (p.get("host"), p.get("proto"))
        if key in seen:
            add("duplicate host port %r" % (key,))
        seen.add(key)
    if web and ports and not any(p["host"] == web["port"] and p["proto"] == "tcp" for p in ports):
        add("web port is not in ports")

    keys = set()
    for s in m.get("settings", []):
        k = s.get("key", "")
        if not KEY_RE.match(k):
            add("bad setting key %r" % (k,))
        if k in keys:
            add("duplicate setting %r" % (k,))
        keys.add(k)
        if k.upper() in RESERVED or k.upper().startswith("STORAGE_"):
            add("setting key %r collides with a loader variable" % (k,))
        t = s.get("type")
        if t not in SETTING_TYPES:
            add("%s: bad type %r" % (k, t))
            continue
        if not s.get("label") or len(s["label"]) > 60:
            add("%s: label must be short and present" % k)
        if "help" in s and not isinstance(s["help"], str):
            add("%s: help must be text" % k)
        d = s.get("default")
        if t == "int":
            if not isinstance(d, int) or isinstance(d, bool):
                add("%s: int default" % k)
            else:
                if "min" in s and d < s["min"]:
                    add("%s: default below min" % k)
                if "max" in s and d > s["max"]:
                    add("%s: default above max" % k)
            if "min" in s and "max" in s and s["min"] > s["max"]:
                add("%s: min > max" % k)
        elif t == "bool":
            if not isinstance(d, bool):
                add("%s: bool default" % k)
        elif t == "choice":
            if not s.get("choices"):
                add("%s: choice needs choices" % k)
            else:
                vals = [v for v, _ in choice_options(s)]
                if len(set(vals)) != len(vals):
                    add("%s: duplicate choice values" % k)
                if d not in vals:
                    add("%s: default not among choices" % k)
        elif t in ("string", "secret", "path", "text"):
            if d is not None and not isinstance(d, str):
                add("%s: default must be text" % k)
            if t == "path" and isinstance(d, str) and d and not d.startswith("/"):
                add("%s: path default must be absolute" % k)
            if "pattern" in s:
                try:
                    rx = re.compile(s["pattern"])
                except re.error as e:
                    add("%s: bad pattern (%s)" % (k, e))
                else:
                    if isinstance(d, str) and d and not rx.search(d):
                        add("%s: default does not match its pattern" % k)
                    if isinstance(d, str) and d == "" and not rx.search("") and not s.get("required"):
                        # contract reading used here: pattern is checked for non-empty values only (optional empty is allowed)
                        pass

    storage_keys = set()
    for st in m.get("storage", []):
        if not KEY_RE.match(st.get("key", "")):
            add("bad storage key")
        if st.get("key") in storage_keys:
            add("duplicate storage key")
        storage_keys.add(st.get("key"))
        path = st.get("path", "")
        if not path.startswith("/") or ".." in path.split("/"):
            add("storage path must be absolute without ..: %r" % (path,))
        if st.get("bulk") is True and not (path == DATA_VOLUME or path.startswith(DATA_VOLUME + "/")):
            add("bulk storage %r must live on the data volume %s" % (path, DATA_VOLUME))
        if "min_free_gib" in st and not (isinstance(st["min_free_gib"], int) and st["min_free_gib"] >= 0):
            add("min_free_gib")
        if not st.get("label"):
            add("storage label")

    dev_ids = set()
    for dv in m.get("devices", []):
        if not KEY_RE.match(dv.get("id", "")):
            add("bad device id")
        if dv.get("id") in dev_ids:
            add("duplicate device id")
        dev_ids.add(dv.get("id"))
        if not dv.get("host", "").startswith("/dev/"):
            add("device host must be under /dev")
        if not DETECT_RE.match(dv.get("detect", "")):
            add("bad detect %r" % (dv.get("detect"),))
        if dv.get("optional") is not True:
            add("device %s: every shipped device must be optional" % dv.get("id"))

    privs = m.get("privileges")
    if not isinstance(privs, list) or not all(isinstance(p, str) and PRIV_RE.match(p) for p in privs):
        add("bad privileges")

    comp = m.get("compose")
    if m.get("kind") == "compose":
        if not isinstance(comp, str) or not os.path.isfile(os.path.join(ADDONS_DIR, folder, comp)):
            add("compose template missing")
    for f in m.get("files", []):
        if not os.path.isfile(os.path.join(ADDONS_DIR, folder, f.get("template", ""))):
            add("template %r missing" % (f.get("template"),))
        dest = f.get("dest", "")
        if not dest or dest.startswith("/") or ".." in dest.split("/"):
            add("file dest must be relative inside DATA_DIR: %r" % (dest,))
        if not isinstance(f.get("once"), bool):
            add("files[].once must be bool")

    hc = m.get("healthcheck")
    if not hc or hc.get("type") not in HEALTH_TYPES:
        add("bad healthcheck")
    elif hc["type"] == "http" and not str(hc.get("url", "")).startswith("http://127.0.0.1:"):
        add("http healthcheck must target 127.0.0.1")
    if not isinstance(m.get("post_install", ""), str):
        add("post_install must be text")
    return errs


def build_context(m, overrides=None, devices=""):
    """Context a loader would build: settings defaults, loader variables, STORAGE_<KEY>, DATA_DIR, DEVICES_BLOCK, choice vars."""
    ctx = dict(LOADER_VARS)
    ctx["ADDON_ID"] = m["id"]
    ctx["DATA_DIR"] = "/srv/addons/" + m["id"]
    ctx["DEVICES_BLOCK"] = devices
    for st in m.get("storage", []):
        ctx["STORAGE_" + st["key"].upper()] = st["path"]
    overrides = overrides or {}
    for s in m.get("settings", []):
        val = overrides.get(s["key"], s.get("default", ""))
        if isinstance(val, bool):
            val = "true" if val else "false"
        ctx[s["key"]] = str(val)
        if s["type"] == "choice":
            for v, vars_ in choice_options(s):
                if v == overrides.get(s["key"], s.get("default")):
                    for name, text in vars_.items():
                        ctx[name] = text
    return ctx


def render(text, ctx):
    """Strict render: an unresolved ${name} (KeyError) or a stray $ (ValueError) is an error."""
    out = string.Template(text).substitute(ctx)
    assert "${" not in out, "unresolved placeholder left in output"
    return out


def choice_combinations(m):
    """Every combination of choice settings: list of override dicts (the defaults when there is no choice)."""
    choices = [s for s in m.get("settings", []) if s["type"] == "choice"]
    if not choices:
        return [{}]
    combos = itertools.product(*[[v for v, _ in choice_options(s)] for s in choices])
    return [dict(zip([s["key"] for s in choices], c)) for c in combos]


class CatalogShape(unittest.TestCase):
    def test_built_ins_present(self):
        self.assertIn("frigate", addon_ids())
        self.assertIn("plex", addon_ids())

    def test_every_manifest_valid(self):
        for aid in addon_ids():
            with self.subTest(addon=aid):
                self.assertEqual(validate_manifest(load(aid), aid), [])

    def test_validator_rejects_broken_manifests(self):
        base = load("frigate")
        bad = json.loads(json.dumps(base))
        bad["storage"][0]["path"] = "/var/lib/frigate"  # bulk on the system disk
        self.assertTrue(any("data volume" in e for e in validate_manifest(bad, "frigate")))
        bad = json.loads(json.dumps(base))
        bad["id"] = "Frigate"
        self.assertTrue(validate_manifest(bad, "frigate"))
        bad = json.loads(json.dumps(base))
        bad["privileges"] = ["root"]
        self.assertTrue(validate_manifest(bad, "frigate"))
        bad = json.loads(json.dumps(base))
        bad["devices"][1]["detect"] = "usb:zzzz"
        self.assertTrue(validate_manifest(bad, "frigate"))
        bad = json.loads(json.dumps(base))
        bad["settings"][2]["default"] = 9999  # retain_days above max
        self.assertTrue(validate_manifest(bad, "frigate"))
        bad = json.loads(json.dumps(base))
        bad["devices"][1]["optional"] = False
        self.assertTrue(validate_manifest(bad, "frigate"))

    def test_templates_use_only_known_names(self):
        """Every ${name} in a template is a setting key, a loader variable, STORAGE_<key> or a variable set by a choice."""
        for aid in addon_ids():
            m = load(aid)
            known = set(RESERVED) | {s["key"] for s in m["settings"]} | {"STORAGE_" + st["key"].upper() for st in m.get("storage", [])}
            for s in m["settings"]:
                if s["type"] == "choice":
                    for _, vars_ in choice_options(s):
                        known |= set(vars_)
            files = [m["compose"]] + [f["template"] for f in m.get("files", [])]
            for fn in files:
                with self.subTest(addon=aid, template=fn):
                    used = set(re.findall(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}", read_template(aid, fn)))
                    self.assertEqual(used - known, set())
            # choice variables must be defined by every option of that choice (else a render would break for some option)
            for s in m["settings"]:
                if s["type"] == "choice":
                    sets = [set(v) for _, v in choice_options(s)]
                    if any(sets):
                        self.assertTrue(all(x == sets[0] for x in sets), "%s/%s: options define different variables" % (aid, s["key"]))

    def test_no_stray_dollars_or_tabs(self):
        for aid in addon_ids():
            m = load(aid)
            for fn in [m["compose"]] + [f["template"] for f in m.get("files", [])]:
                text = read_template(aid, fn)
                self.assertNotIn("\t", text, fn)
                self.assertNotIn("\r", text, fn)
                self.assertTrue(text.endswith("\n"), fn)

    def test_secret_settings_have_no_secret_default_worth_hiding(self):
        """A shipped default for a secret must be a harmless placeholder (loopback address or empty)."""
        for aid in addon_ids():
            for s in load(aid)["settings"]:
                if s["type"] == "secret":
                    d = s.get("default", "")
                    self.assertTrue(d == "" or "127.0.0.1" in d, "%s/%s: %r" % (aid, s["key"], d))


class FrigateRender(unittest.TestCase):
    def setUp(self):
        self.m = load("frigate")

    def compose(self, overrides=None, devices=""):
        return render(read_template("frigate", "compose.yml.tmpl"), build_context(self.m, overrides, devices))

    def config(self, overrides=None):
        return render(read_template("frigate", "config.yml.tmpl"), build_context(self.m, overrides))

    def test_storage_is_on_the_data_volume(self):
        st = {s["key"]: s for s in self.m["storage"]}
        self.assertTrue(st["media"]["bulk"])
        self.assertEqual(st["media"]["path"], "/srv/data/addons/frigate/media")
        self.assertGreaterEqual(st["media"]["min_free_gib"], 50)

    def test_no_privileges_and_optional_devices(self):
        self.assertEqual(self.m["privileges"], [])
        by = {d["id"]: d for d in self.m["devices"]}
        self.assertEqual(set(by), {"dri", "coral_usb", "coral_pcie"})
        self.assertTrue(all(d["optional"] for d in by.values()))
        self.assertEqual(by["coral_usb"]["detect"], "usb:1a6e:089a|18d1:9302")
        self.assertEqual(by["coral_pcie"]["host"], "/dev/apex_0")
        self.assertEqual(by["dri"]["host"], "/dev/dri")

    def test_compose_required_lines(self):
        out = self.compose()
        for needle in [
            "image: ghcr.io/blakeblackshear/frigate:stable",
            "restart: unless-stopped",
            'shm_size: "512mb"',
            "- /etc/localtime:/etc/localtime:ro",
            "- /srv/addons/frigate/config:/config",
            "- /srv/data/addons/frigate/media:/media/frigate",
            "- /tmp/cache:size=1000m",
            '- "8971:8971"',
            '- "127.0.0.1:5000:5000"',
            '- "8554:8554"',
            '- "8555:8555/tcp"',
            '- "8555:8555/udp"',
            'TZ: "Europe/Berlin"',
        ]:
            self.assertIn(needle, out)
        self.assertNotIn("privileged", out)
        self.assertNotIn("network_mode", out)
        self.assertNotIn("cap_add", out)
        self.assertNotIn('- "5000:5000"', out)  # never published on all interfaces
        self.assertNotIn("devices:", out)  # nothing enabled: no empty devices key

    def test_compose_devices_block_lands_in_the_service(self):
        out = self.compose(devices=DEVICES_FULL)
        self.assertIn("\n    devices:\n      - /dev/dri:/dev/dri\n", out)

    def test_compose_follows_settings(self):
        out = self.compose({"shm_mb": 1024, "cache_mb": 2000})
        self.assertIn('shm_size: "1024mb"', out)
        self.assertIn("- /tmp/cache:size=2000m", out)

    def test_config_defaults(self):
        out = self.config()
        self.assertIn("mqtt:\n  enabled: False\n", out)
        self.assertIn("detectors:\n  cpu1:\n    type: cpu\n", out)
        self.assertNotIn("\nmodel:", out)
        self.assertIn("  hwaccel_args: auto\n", out)
        self.assertIn("    days: 7\n", out)  # continuous
        self.assertIn("go2rtc:\n  streams:\n    camera_1:\n", out)
        for n in (1, 2, 3, 4):
            self.assertIn("  camera_%d:\n    enabled: false\n" % n, out)
            self.assertIn("path: rtsp://127.0.0.1:8554/camera_%d\n" % n, out)
        self.assertEqual(out.count("enabled: false"), 4)  # zero cameras on: valid like Frigate's own dummy_camera example

    def test_config_detector_choices(self):
        cases = {
            "cpu": ("type: cpu", False),
            "openvino_gpu": ("type: openvino\n    device: GPU", True),
            "openvino_cpu": ("type: openvino\n    device: CPU", True),
            "edgetpu_usb": ("type: edgetpu\n    device: usb", False),
            "edgetpu_pci": ("type: edgetpu\n    device: pci", False),
        }
        self.assertEqual(set(cases), {v for v, _ in choice_options([s for s in self.m["settings"] if s["key"] == "detector"][0])})
        for value, (needle, has_model) in cases.items():
            with self.subTest(detector=value):
                out = self.config({"detector": value})
                self.assertIn(needle, out)
                self.assertEqual("\nmodel:\n" in out, has_model)
                self.assertEqual(out.count("detectors:"), 1)
                if has_model:
                    self.assertIn("path: /openvino-model/ssdlite_mobilenet_v2.xml", out)
                    self.assertIn("input_pixel_format: bgr", out)

    def test_config_hwaccel_retention_and_cameras(self):
        out = self.config({"hwaccel": "preset-vaapi", "retain_days": 30, "event_retain_days": 60,
                           "cam1_enabled": True, "cam1_url": "rtsp://admin:pw@192.168.1.20:554/stream1"})
        self.assertIn("  hwaccel_args: preset-vaapi\n", out)
        self.assertIn("  continuous:\n    days: 30\n", out)
        self.assertIn("    retain:\n      days: 60\n", out)
        self.assertIn("  retain:\n    default: 60\n", out)  # snapshots
        self.assertIn('    camera_1:\n      - "rtsp://admin:pw@192.168.1.20:554/stream1"\n', out)
        self.assertIn("  camera_1:\n    enabled: true\n", out)
        self.assertIn("  camera_2:\n    enabled: false\n", out)

    def test_camera_url_pattern_blocks_go2rtc_command_sources(self):
        """go2rtc runs exec:/ffmpeg: sources as commands; the setting pattern must only let network stream URLs through."""
        s = [x for x in self.m["settings"] if x["key"] == "cam1_url"][0]
        rx = re.compile(s["pattern"])
        for ok in ["rtsp://127.0.0.1:554/rtsp", "rtsps://u:p%40x@cam.lan:322/Streaming/Channels/101", "http://cam/stream.m3u8?x=1&y=2"]:
            self.assertTrue(rx.search(ok), ok)
        for bad in ["exec:ffmpeg -i x", "ffmpeg:rtsp://x#video=h264", "rtsp://x/a b", 'rtsp://x/"y', "rtsp://x/{FRIGATE_X}", "rtsp://x/$y",
                    "rtsp://x/y#z", "rtsp://x\n- exec:id", "file:///etc/passwd", "rtsp://x/\\y", ""]:
            self.assertFalse(rx.search(bad), bad)

    def test_every_choice_combination_renders(self):
        for ov in choice_combinations(self.m):
            with self.subTest(**ov):
                self.compose(ov)
                self.config(ov)

    def test_post_install_covers_the_admin_password(self):
        t = self.m["post_install"].lower()
        self.assertIn("password", t)
        self.assertIn("log", t)
        self.assertIn("8971", t)

    @unittest.skipIf(yaml is None, "PyYAML not installed (optional)")
    def test_rendered_files_are_valid_yaml(self):
        for ov in choice_combinations(self.m):
            with self.subTest(**ov):
                cfg = yaml.safe_load(self.config(ov))
                self.assertIs(cfg["mqtt"]["enabled"], False)
                self.assertEqual(len(cfg["detectors"]), 1)
                self.assertEqual(set(cfg["cameras"]), {"camera_1", "camera_2", "camera_3", "camera_4"})
                self.assertTrue(all(c["enabled"] is False for c in cfg["cameras"].values()))
                self.assertEqual(cfg["record"]["continuous"]["days"], 7)
                self.assertEqual(set(cfg["go2rtc"]["streams"]), set(cfg["cameras"]))
                comp = yaml.safe_load(self.compose(ov, DEVICES_FULL))
                svc = comp["services"]["frigate"]
                self.assertEqual(svc["image"], "ghcr.io/blakeblackshear/frigate:stable")
                self.assertEqual(svc["devices"], ["/dev/dri:/dev/dri"])
                self.assertEqual(svc["tmpfs"], ["/tmp/cache:size=1000m"])
                self.assertNotIn("privileged", svc)
        comp = yaml.safe_load(self.compose())
        self.assertNotIn("devices", comp["services"]["frigate"])
        cfg = yaml.safe_load(self.config({"cam1_enabled": True, "cam1_url": "rtsp://a:b@10.0.0.2:554/s"}))
        self.assertIs(cfg["cameras"]["camera_1"]["enabled"], True)
        self.assertEqual(cfg["go2rtc"]["streams"]["camera_1"], ["rtsp://a:b@10.0.0.2:554/s"])


class PlexRender(unittest.TestCase):
    def setUp(self):
        self.m = load("plex")

    def compose(self, overrides=None, devices=""):
        return render(read_template("plex", "compose.yml.tmpl"), build_context(self.m, overrides, devices))

    def test_manifest_facts(self):
        self.assertEqual(self.m["privileges"], ["host-network"])
        self.assertEqual(self.m["healthcheck"], {"type": "http", "url": "http://127.0.0.1:32400/identity"})
        self.assertEqual(self.m["files"], [])
        st = {s["key"]: s for s in self.m["storage"]}
        self.assertTrue(st["transcode"]["bulk"])
        self.assertTrue(st["transcode"]["path"].startswith("/srv/data/"))
        dev = self.m["devices"][0]
        self.assertEqual((dev["host"], dev["optional"]), ("/dev/dri", True))
        settings = {s["key"]: s for s in self.m["settings"]}
        self.assertEqual(settings["plex_claim"]["type"], "secret")
        self.assertEqual(settings["media_dir"]["default"], "/srv/data/media")
        self.assertEqual(settings["media_dir"]["type"], "path")
        self.assertEqual(settings["media_access"]["default"], "ro")
        self.assertEqual(settings["network"]["default"], "host")
        self.assertTrue(any(p["host"] == 32400 and p["proto"] == "tcp" for p in self.m["ports"]))

    def test_claim_token_pattern(self):
        rx = re.compile([s for s in self.m["settings"] if s["key"] == "plex_claim"][0]["pattern"])
        self.assertTrue(rx.search("claim-AbCdEfGh1234567890_-"))
        for bad in ["", "AbCdEfGh1234567890", "claim-x", 'claim-abc"def12345', "claim-abc def12345", "claim-abcdefgh\nPLEX_UID: 0"]:
            self.assertFalse(rx.search(bad), repr(bad))

    def test_compose_host_network(self):
        out = self.compose({"plex_claim": "claim-AbCdEfGh1234567890"})
        for needle in [
            "image: plexinc/pms-docker:latest",
            "restart: unless-stopped",
            "network_mode: host",
            'PLEX_CLAIM: "claim-AbCdEfGh1234567890"',
            'PLEX_UID: "1000"',
            'PLEX_GID: "1000"',
            'TZ: "Europe/Berlin"',
            "- /srv/addons/plex/config:/config",
            "- /srv/data/addons/plex/transcode:/transcode",
            "- /srv/data/media:/data:ro",
        ]:
            self.assertIn(needle, out)
        self.assertNotIn("\n    ports:", out)
        self.assertNotIn("privileged", out)
        self.assertNotIn("devices:", out)

    def test_compose_bridge_publishes_documented_ports(self):
        out = self.compose({"network": "bridge", "advertise_url": "http://192.168.1.10:32400/"})
        self.assertNotIn("network_mode", out)
        for p in ["32400:32400/tcp", "8324:8324/tcp", "32469:32469/tcp", "1900:1900/udp", "32410:32410/udp",
                  "32412:32412/udp", "32413:32413/udp", "32414:32414/udp"]:
            self.assertIn('      - "%s"\n' % p, out)
        self.assertIn('ADVERTISE_IP: "http://192.168.1.10:32400/"', out)
        # every published port is declared in the manifest (conflict checks, firewall hints)
        declared = {(p["host"], p["proto"]) for p in self.m["ports"]}
        published = {(int(a), c) for a, c in re.findall(r'"(\d+):\d+/(tcp|udp)"', out)}
        self.assertEqual(published, declared)

    def test_media_access_rw_and_devices(self):
        out = self.compose({"media_access": "rw", "media_dir": "/srv/data/media/Video"}, devices=DEVICES_FULL)
        self.assertIn("- /srv/data/media/Video:/data:rw", out)
        self.assertIn("\n    devices:\n      - /dev/dri:/dev/dri\n", out)

    def test_every_choice_combination_renders(self):
        for ov in choice_combinations(self.m):
            with self.subTest(**ov):
                self.compose(ov)

    def test_post_install_covers_claim_token(self):
        t = self.m["post_install"]
        self.assertIn("4 minutes", t)
        self.assertIn("optional", t)
        self.assertIn("plex.tv/claim", t)

    @unittest.skipIf(yaml is None, "PyYAML not installed (optional)")
    def test_rendered_compose_is_valid_yaml(self):
        for ov in choice_combinations(self.m):
            for dev in ("", DEVICES_FULL):
                with self.subTest(devices=bool(dev), **ov):
                    svc = yaml.safe_load(self.compose(ov, dev))["services"]["plex"]
                    self.assertEqual(svc["image"], "plexinc/pms-docker:latest")
                    self.assertEqual(svc["environment"]["PLEX_UID"], "1000")
                    self.assertEqual("network_mode" in svc, ov["network"] == "host")
                    self.assertEqual("ports" in svc, ov["network"] == "bridge")
                    self.assertEqual("devices" in svc, bool(dev))
                    self.assertEqual(svc["volumes"][2], "/srv/data/media:/data:" + ov["media_access"])


class PathsAgreeWithInstaller(unittest.TestCase):
    def test_data_volume_mountpoint_matches_the_install_script(self):
        """The installer mounts the data LV at /srv/data; bulk storage in the catalog must sit below it."""
        root = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
        script = os.path.join(root, "crates", "synofurb-core", "src", "script.rs")
        if not os.path.isfile(script):
            self.skipTest("installer sources not present")
        with open(script, encoding="utf-8") as f:
            src = f.read()
        self.assertIn("/dev/mapper/vg_data-data /target" + DATA_VOLUME, src)
        for aid in addon_ids():
            for st in load(aid).get("storage", []):
                if st.get("bulk"):
                    self.assertTrue(st["path"].startswith(DATA_VOLUME + "/"), st["path"])


if __name__ == "__main__":
    unittest.main()
