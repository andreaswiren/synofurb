#!/usr/bin/env python3
"""synofurb add-ons: the plugin framework for Docker apps of the management plane.

Standard library only, and deliberately free of any import from ``synofurb_mgmt`` (the web UI module loads this one,
never the other way round, so the module also works when ``synofurb_mgmt.py`` runs as ``__main__``).  Contract: docs/ADDONS.md.

  1. manifest loader / validator   ``addons/<id>/addon.json`` (+ user folder ``/etc/synofurb/addons.d/<id>``, + packed form)
  2. template rendering            ``string.Template``; every value is validated before it reaches a template or argv
  3. lifecycle + jobs              install / start / stop / restart / update / remove / status / logs, one job at a time
  4. built-in system handler       ``docker`` (Debian packages, service, group wiring)
  5. REST routes, CLI, apply-pending (the glue in synofurb_mgmt.py only forwards to ``register_routes`` / ``cli_main``)

Everything that touches Docker goes through the small ``Docker`` class, so the whole thing is testable without a daemon
(``FakeDocker`` is the test double; nothing in the test suite talks to a real Docker daemon).

Security rules (see "Implementation notes" in docs/ADDONS.md):
  * no shell, argv lists only; every id, path, project name and setting value is validated before use
  * a setting value reaches a compose template only through a restricted alphabet (or as a JSON-quoted scalar)
  * manifest and file paths are flat names / validated relative paths; nothing can leave the add-on or data directory
  * secrets live in a 0600 file, are never logged, never returned by the API, and are scrubbed from job and container logs
"""
from __future__ import annotations

import collections
import fcntl
import http.client
import ipaddress
import json
import os
import re
import secrets as _secrets
import shutil
import socket
import ssl
import string
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import urllib.parse
from dataclasses import dataclass, field
from typing import Any, Callable, Iterable

SCHEMA = 1
PACK_NAME = "addons.pack.json"
PACK_SCHEMA = 1
MAX_FILE = 256 * 1024
MAX_FILES_PER_ADDON = 24

ID_RE = re.compile(r"[a-z0-9][a-z0-9-]{0,31}")
KEY_RE = re.compile(r"[a-z][a-z0-9_]{0,31}")
FILENAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}")
VERSION_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._+-]{0,31}")
CATEGORY_RE = re.compile(r"[a-z][a-z0-9-]{0,23}")
ICON_RE = re.compile(r"[a-z0-9][a-z0-9-]{0,39}")
LABEL_RE = re.compile(r"[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?")
CHOICE_RE = re.compile(r"[A-Za-z0-9._-]{1,32}")
CAP_RE = re.compile(r"cap:[A-Z][A-Z0-9_]{1,31}")
USB_RE = re.compile(r"[0-9a-f]{4}:[0-9a-f]{4}")
HOMEPAGE_RE = re.compile(r"https?://[A-Za-z0-9.-]{1,100}(?::\d{1,5})?(?:/[A-Za-z0-9._~/%?=&#+-]{0,150})?")
WEB_PATH_RE = re.compile(r"/[A-Za-z0-9._~/%-]{0,100}")
USER_RE = re.compile(r"[a-z_][a-z0-9_-]{0,31}")
CTRL_RE = re.compile(r"[\x00-\x1f\x7f]")
TZ_RE = re.compile(r"[A-Za-z0-9_+/-]{1,64}")

#: characters a path may contain (it is written into a compose file and used as a mount source)
PATH_CHARS = re.compile(r"/[A-Za-z0-9._@+/-]*")
#: raw (unquoted) substitution into a compose file: a conservative plain scalar, no whitespace, no quote, no '$', no '#'
RAW_COMPOSE_RE = re.compile(r"(?:[A-Za-z0-9._/~][A-Za-z0-9._/@+=,:~%?&-]*)?")
#: where the placeholder sits decides how strict its value must be (strictest first)
RULE_ORDER = ("raw", "cquoted", "fquoted", "loose")
VAR_NAME_RE = re.compile(r"[A-Z][A-Z0-9_]{0,31}")
TOP_KEYS = frozenset("schema id name version summary category icon homepage kind handler requires web ports settings storage devices "
                     "privileges compose files healthcheck post_install changes".split())
SETTING_TYPES = ("string", "int", "bool", "secret", "choice", "path", "text")
FIXED_PRIVILEGES = ("docker-socket", "host-network", "privileged")
PRIVILEGE_HELP = {
    "docker-socket": "Full control of Docker on this unit, which is the same as root.",
    "host-network": "Shares the unit's network; its ports are not isolated.",
    "privileged": "Unrestricted access to the unit's hardware and kernel.",
}
DATA_ROOT = "/srv/addons"
DATA_VOLUME = "/srv/data"                       # where the installer mounts the redundant data LV
STORAGE_ROOTS = ("/srv/", "/mnt/")
BUILTIN_IDS_ORDER = ("docker", "portainer")


class AddonError(ValueError):
    """A manifest, request or operation was rejected; the message is safe to show to the person."""


# ============================================================================
# 1. Manifest model + validation
# ============================================================================

@dataclass
class Addon:
    id: str
    name: str
    version: str
    summary: str
    category: str
    icon: str
    homepage: str
    kind: str
    handler: str
    requires: list[str]
    web: dict | None
    ports: list[dict]
    settings: list[dict]
    storage: list[dict]
    devices: list[dict]
    privileges: list[str]
    compose: str
    files: list[dict]
    healthcheck: dict | None
    post_install: str
    changes: list[str]
    source: str                                    # builtin | user
    templates: dict[str, str] = field(default_factory=dict)     # filename -> text (compose + files[].template)
    value_rules: dict = field(default_factory=dict)   # setting key -> strictest RULE_ORDER entry of its placeholders

    @property
    def data_dir(self) -> str:
        return "%s/%s" % (DATA_ROOT, self.id)

    def setting(self, key: str) -> dict | None:
        return next((s for s in self.settings if s["key"] == key), None)

    def secret_keys(self) -> list[str]:
        return [s["key"] for s in self.settings if s["type"] == "secret"]

    def public(self) -> dict:
        """Everything the UI needs to draw the add-on; contains no template text and no secret values."""
        return {"id": self.id, "name": self.name, "version": self.version, "summary": self.summary, "category": self.category, "icon": self.icon,
                "homepage": self.homepage, "kind": self.kind, "requires": list(self.requires), "web": self.web, "ports": self.ports,
                "settings": [_public_setting(s) for s in self.settings], "storage": self.storage,
                "devices": self.devices, "privileges": [{"id": p, "help": privilege_help(p)} for p in self.privileges],
                "post_install": self.post_install, "changes": list(self.changes), "source": self.source, "data_dir": self.data_dir}


def _public_setting(s: dict) -> dict:
    out = {k: v for k, v in s.items() if k != "choices"}
    if "choices" in s:
        out["choices"] = [{"value": c["value"], "label": c["label"], "privileges": list(c["privileges"])} for c in s["choices"]]
    return out


def effective_privileges(a: Addon, values: dict) -> list[str]:
    """Manifest privileges plus those of the chosen options (Plex: host-network only when the host network is chosen)."""
    out = list(a.privileges)
    for s in a.settings:
        if s["type"] == "choice":
            for c in s["choices"]:
                if c["value"] == values.get(s["key"], s.get("default")):
                    out += [p for p in c["privileges"] if p not in out]
    return out


def privilege_help(p: str) -> str:
    if p.startswith("cap:"):
        return "Extra kernel capability %s for the container." % p[4:]
    return PRIVILEGE_HELP.get(p, "")


def _where(path: str, msg: str) -> AddonError:
    return AddonError("%s: %s" % (path, msg))


def _need_dict(v: Any, path: str) -> dict:
    if not isinstance(v, dict):
        raise _where(path, "must be an object")
    return v


def _need_list(v: Any, path: str, limit: int) -> list:
    if not isinstance(v, list):
        raise _where(path, "must be a list")
    if len(v) > limit:
        raise _where(path, "too many entries (max %d)" % limit)
    return v


def _text(d: dict, key: str, path: str, maxlen: int, required: bool = True, default: str = "", regex: re.Pattern | None = None, multiline: bool = False) -> str:
    v = d.get(key)
    if v is None:
        if required:
            raise _where(path, "'%s' is required" % key)
        return default
    if not isinstance(v, str):
        raise _where(path, "'%s' must be a string" % key)
    if not v.strip() and required:
        raise _where(path, "'%s' must not be empty" % key)
    if len(v) > maxlen:
        raise _where(path, "'%s' is too long (max %d characters)" % (key, maxlen))
    if CTRL_RE.search(v.replace("\n", "") if multiline else v):
        raise _where(path, "'%s' must not contain control characters" % key)
    if regex is not None and not regex.fullmatch(v):
        raise _where(path, "'%s' is not valid: %r" % (key, v[:40]))
    return v


def _int(v: Any, path: str, what: str, lo: int, hi: int) -> int:
    if isinstance(v, bool) or not isinstance(v, int):
        raise _where(path, "'%s' must be a whole number" % what)
    if v < lo or v > hi:
        raise _where(path, "'%s' must be between %d and %d" % (what, lo, hi))
    return v


def _only_keys(d: dict, allowed: Iterable[str], path: str) -> None:
    extra = [k for k in d if k not in allowed and not str(k).startswith(("x-", "_"))]
    if extra:
        raise _where(path, "unknown field%s: %s" % ("s" if len(extra) > 1 else "", ", ".join(sorted(map(str, extra))[:5])))


def valid_abs_path(v: Any, what: str = "path", roots: tuple = STORAGE_ROOTS, depth: int = 2) -> str:
    """A normalized absolute path under one of ``roots`` (default /srv or /mnt), a restricted alphabet, no '..'; raises AddonError."""
    if not isinstance(v, str) or not v:
        raise AddonError("%s must be a path" % what)
    if len(v) > 200 or not PATH_CHARS.fullmatch(v):
        raise AddonError("%s may only contain letters, digits and . _ @ + - /" % what)
    parts = v.split("/")[1:]
    if v.endswith("/") or any(p in ("", ".", "..") for p in parts):
        raise AddonError("%s must be a normalized absolute path (no '..', '//' or trailing '/')" % what)
    if not any(v.startswith(r) for r in roots):
        raise AddonError("%s must be below %s" % (what, " or ".join(r.rstrip("/") for r in roots)))
    if len(parts) < depth:
        raise AddonError("%s is too shallow: %s" % (what, v))
    return v


def valid_dest(v: Any, path: str) -> str:
    if not isinstance(v, str) or not v or len(v) > 120:
        raise _where(path, "'dest' must be a relative file path")
    parts = v.split("/")
    if v.startswith("/") or len(parts) > 6 or any(not FILENAME_RE.fullmatch(p) or p in (".", "..") for p in parts):
        raise _where(path, "'dest' must be a relative path of plain names (no '..', no leading '/'): %r" % v[:60])
    if v in ("compose.yml", "compose.next.yml") or parts[0].startswith(".synofurb"):
        raise _where(path, "'dest' %r is reserved" % v)
    return v


def _compile(pattern: str, path: str) -> re.Pattern:
    if len(pattern) > 120:
        raise _where(path, "'pattern' is too long")
    try:
        return re.compile(pattern)
    except re.error as e:
        raise _where(path, "'pattern' is not a valid expression: %s" % e)


# ---- settings -----------------------------------------------------------------

def _parse_privileges(raw: Any, path: str) -> list[str]:
    privs = _need_list(raw, path, 16)
    for p in privs:
        if not isinstance(p, str) or not (p in FIXED_PRIVILEGES or CAP_RE.fullmatch(p)):
            raise _where(path, "unknown privilege %r (use %s or cap:<NAME>)" % (str(p)[:30], ", ".join(FIXED_PRIVILEGES)))
    if len(set(privs)) != len(privs):
        raise _where(path, "duplicate entry")
    return list(privs)


def _parse_choices(raw: Any, path: str) -> list[dict]:
    """Strings, or objects {value, label, vars:{NAME:text}, privileges:[...]}.  The vars of the chosen option become template variables."""
    ch = _need_list(raw if raw is not None else [], path + ".choices", 32)
    if not ch:
        raise _where(path, "a choice setting needs 'choices'")
    out: list[dict] = []
    for i, c in enumerate(ch):
        cp = "%s.choices[%d]" % (path, i)
        if isinstance(c, str):
            c = {"value": c}
        d = _need_dict(c, cp)
        _only_keys(d, ("value", "label", "vars", "privileges"), cp)
        val = d.get("value")
        if not isinstance(val, str) or not CHOICE_RE.fullmatch(val):
            raise _where(cp, "'value' must be a short name (letters, digits, . _ -)")
        vars_ = _need_dict(d.get("vars", {}), cp + ".vars")
        for k, t in vars_.items():
            if not isinstance(k, str) or not VAR_NAME_RE.fullmatch(k) or k in BUILTIN_VARS or k.startswith("STORAGE_"):
                raise _where(cp, "variable name %r is not allowed (use CAPITAL_LETTERS, not a built-in name)" % str(k)[:40])
            if not isinstance(t, str) or len(t) > 4000 or "\x00" in t:
                raise _where(cp, "variable %s must be text of at most 4000 characters" % k)
        out.append({"value": val, "label": _text(d, "label", cp, 80, required=False, default=val), "vars": dict(vars_),
                    "privileges": _parse_privileges(d.get("privileges", []), cp + ".privileges")})
    if len({c["value"] for c in out}) != len(out):
        raise _where(path, "duplicate choice values")
    names = [set(c["vars"]) for c in out]
    if any(n != names[0] for n in names):
        raise _where(path, "every option of one choice must define the same variable names")
    return out


def _parse_setting(raw: Any, i: int, seen: set) -> dict:
    path = "settings[%d]" % i
    d = _need_dict(raw, path)
    _only_keys(d, ("key", "label", "type", "default", "min", "max", "pattern", "choices", "required", "help", "maxlen"), path)
    key = _text(d, "key", path, 32, regex=KEY_RE)
    path = "settings[%d] (%s)" % (i, key)
    if key in seen:
        raise _where(path, "duplicate setting key")
    if key.endswith("_json"):
        raise _where(path, "setting keys must not end in '_json' (reserved for the quoted form)")
    seen.add(key)
    typ = d.get("type", "string")
    if typ not in SETTING_TYPES:
        raise _where(path, "'type' must be one of %s" % ", ".join(SETTING_TYPES))
    s: dict = {"key": key, "label": _text(d, "label", path, 60), "type": typ, "required": d.get("required", False), "help": _text(d, "help", path, 800, required=False)}
    if not isinstance(s["required"], bool):
        raise _where(path, "'required' must be true or false")
    allowed = {"min": ("int", "string", "secret", "path", "text"), "max": ("int", "string", "secret", "path", "text"), "pattern": ("string", "path", "secret", "text"),
               "choices": ("choice",), "maxlen": ("string", "secret", "text")}
    for k, ok in allowed.items():
        if k in d and typ not in ok:
            raise _where(path, "'%s' is not allowed for type '%s'" % (k, typ))
    if typ == "int":
        lo = _int(d.get("min", -2 ** 31), path, "min", -2 ** 31, 2 ** 31)
        hi = _int(d.get("max", 2 ** 31), path, "max", -2 ** 31, 2 ** 31)
        if lo > hi:
            raise _where(path, "'min' is larger than 'max'")
        s["min"], s["max"] = lo, hi
    elif typ in ("string", "secret", "path", "text"):                      # length bounds of non-empty values
        if "min" in d:
            s["min"] = _int(d["min"], path, "min", 0, 16000)
        if "max" in d:
            s["max"] = _int(d["max"], path, "max", 1, 16000)
        if "min" in s and "max" in s and s["min"] > s["max"]:
            raise _where(path, "'min' is larger than 'max'")
    if typ == "choice":
        s["choices"] = _parse_choices(d.get("choices"), path)
    if "pattern" in d:
        s["pattern"] = _text(d, "pattern", path, 160)
        _compile(s["pattern"], path)
    if typ in ("string", "secret", "text"):
        cap = 16000 if typ == "text" else 512
        s["maxlen"] = _int(d.get("maxlen", 4000 if typ == "text" else 200), path, "maxlen", 1, cap)
    if "default" in d:
        try:
            s["default"] = coerce_setting(s, d["default"], "loose", label=key)
        except AddonError as e:
            raise _where(path, "default is not valid: %s" % e)
    return s


def _ctx_ok(rule: str, v: str) -> bool:
    if rule == "raw":
        return bool(RAW_COMPOSE_RE.fullmatch(v))
    if rule == "cquoted":
        return not re.search(r'["\\$]', v)
    if rule == "fquoted":
        return not re.search(r'["\\]', v)
    return True


_CTX_WORDS = {"raw": "letters, digits and . _ / @ + = , : ~ % ? & - (no spaces or quotes)", "cquoted": "no quotes, backslashes or $ signs",
              "fquoted": "no quotes or backslashes"}


def coerce_setting(s: dict, v: Any, rule: str = "loose", label: str = "") -> Any:
    """Validate one value against its setting definition.  ``rule`` says where it will be written (see RULE_ORDER)."""
    name = s.get("label") or label or s["key"]
    typ = s["type"]
    if typ == "bool":
        if isinstance(v, bool):
            return v
        if isinstance(v, str) and v.lower() in ("true", "1", "yes", "on"):
            return True
        if isinstance(v, str) and v.lower() in ("false", "0", "no", "off", ""):
            return False
        raise AddonError("%s must be on or off" % name)
    if typ == "int":
        if isinstance(v, bool) or not isinstance(v, (int, str)):
            raise AddonError("%s must be a whole number" % name)
        try:
            n = int(v) if isinstance(v, int) else int(v.strip(), 10)
        except ValueError:
            raise AddonError("%s must be a whole number" % name)
        if n < s.get("min", -2 ** 31) or n > s.get("max", 2 ** 31):
            raise AddonError("%s must be between %s and %s" % (name, s.get("min", "-"), s.get("max", "-")))
        return n
    if not isinstance(v, str):
        raise AddonError("%s must be text" % name)
    if typ == "choice":
        if v not in [c["value"] for c in s["choices"]]:
            raise AddonError("%s must be one of: %s" % (name, ", ".join(c["value"] for c in s["choices"])))
        return v
    if v == "":
        return v                                                  # emptiness is decided by 'required'
    if typ == "path":
        v = valid_abs_path(v, name)
    if len(v) > s.get("maxlen", 200) and typ != "path":
        raise AddonError("%s is too long (max %d characters)" % (name, s.get("maxlen", 200)))
    if "min" in s and len(v) < s["min"]:
        raise AddonError("%s is too short (min %d characters)" % (name, s["min"]))
    if "max" in s and len(v) > s["max"]:
        raise AddonError("%s is too long (max %d characters)" % (name, s["max"]))
    if CTRL_RE.search(v.replace("\n", "").replace("\t", "") if typ == "text" else v):
        raise AddonError("%s must not contain control characters" % name)
    if "pattern" in s and not re.fullmatch(s["pattern"], v):
        raise AddonError("%s does not have the expected format" % name)
    if not _ctx_ok(rule, v):
        raise AddonError("%s may contain %s" % (name, _CTX_WORDS[rule]))
    return v


# ---- storage, devices ---------------------------------------------------------

def _parse_storage(raw: Any, i: int, seen: set) -> dict:
    path = "storage[%d]" % i
    d = _need_dict(raw, path)
    _only_keys(d, ("key", "label", "path", "bulk", "min_free_gib"), path)
    key = _text(d, "key", path, 32, regex=KEY_RE)
    if key in seen:
        raise _where(path, "duplicate storage key")
    seen.add(key)
    try:
        p = valid_abs_path(d.get("path"), "storage '%s' path" % key)
    except AddonError as e:
        raise _where(path, str(e))
    bulk = d.get("bulk", False)
    if not isinstance(bulk, bool):
        raise _where(path, "'bulk' must be true or false")
    if bulk and not p.startswith(DATA_VOLUME + "/"):
        raise _where(path, "bulk storage must live on the redundant data volume: the path has to be below %s (got %s)" % (DATA_VOLUME, p))
    return {"key": key, "label": _text(d, "label", path, 60), "path": p, "bulk": bulk, "min_free_gib": _int(d.get("min_free_gib", 0), path, "min_free_gib", 0, 10 ** 6)}


def _parse_detect(v: Any, path: str) -> str:
    if v is None:
        return ""
    if not isinstance(v, str):
        raise _where(path, "'detect' must be a string")
    if v.startswith("path:"):
        p = v[5:]
        if not re.fullmatch(r"/(?:dev|sys)/[A-Za-z0-9._/:-]{1,120}", p) or ".." in p.split("/"):
            raise _where(path, "'detect' path must be a plain file path below /dev or /sys")
        return v
    if v.startswith("usb:"):
        alts = v[4:].lower().split("|")
        if not alts or len(alts) > 8 or not all(USB_RE.fullmatch(a) for a in alts):
            raise _where(path, "'detect' usb must look like usb:1a6e:089a|18d1:9302")
        return "usb:" + "|".join(alts)
    raise _where(path, "'detect' must be path:<file> or usb:<vid:pid>|<vid:pid>")


def _parse_device(raw: Any, i: int, seen: set) -> dict:
    path = "devices[%d]" % i
    d = _need_dict(raw, path)
    _only_keys(d, ("id", "label", "host", "optional", "detect"), path)
    did = _text(d, "id", path, 32, regex=KEY_RE)
    if did in seen:
        raise _where(path, "duplicate device id")
    seen.add(did)
    host = _text(d, "host", path, 120)
    if not re.fullmatch(r"/dev/[A-Za-z0-9._/-]{1,100}", host) or ".." in host.split("/") or "//" in host or host.endswith("/"):
        raise _where(path, "'host' must be a plain path below /dev")
    opt = d.get("optional", True)
    if not isinstance(opt, bool):
        raise _where(path, "'optional' must be true or false")
    return {"id": did, "label": _text(d, "label", path, 80), "host": host, "optional": opt, "detect": _parse_detect(d.get("detect"), path)}


# ---- web, ports, healthcheck --------------------------------------------------

def _parse_web(raw: Any) -> dict:
    d = _need_dict(raw, "web")
    _only_keys(d, ("port", "scheme", "path", "hostname_hint"), "web")
    scheme = d.get("scheme", "http")
    if scheme not in ("http", "https"):
        raise _where("web", "'scheme' must be http or https")
    hint = _text(d, "hostname_hint", "web", 63, required=False, regex=LABEL_RE)
    return {"port": _int(d.get("port"), "web", "port", 1, 65535), "scheme": scheme, "path": _text(d, "path", "web", 101, required=False, default="/", regex=WEB_PATH_RE),
            "hostname_hint": hint}


def _parse_port(raw: Any, i: int) -> dict:
    path = "ports[%d]" % i
    d = _need_dict(raw, path)
    _only_keys(d, ("host", "container", "proto", "label", "bind"), path)
    proto = d.get("proto", "tcp")
    if proto not in ("tcp", "udp"):
        raise _where(path, "'proto' must be tcp or udp")
    bind = d.get("bind", "0.0.0.0")
    try:
        if not isinstance(bind, str) or ipaddress.ip_address(bind).version != 4:
            raise ValueError
    except ValueError:
        raise _where(path, "'bind' must be an IPv4 address such as 0.0.0.0 or 127.0.0.1")
    return {"host": _int(d.get("host"), path, "host", 1, 65535), "container": _int(d.get("container"), path, "container", 1, 65535), "proto": proto,
            "label": _text(d, "label", path, 80, required=False), "bind": bind}


def _parse_health(raw: Any) -> dict:
    d = _need_dict(raw, "healthcheck")
    _only_keys(d, ("type", "url", "port", "timeout"), "healthcheck")
    typ = d.get("type")
    if typ not in ("http", "tcp", "container"):
        raise _where("healthcheck", "'type' must be http, tcp or container")
    h: dict = {"type": typ, "timeout": _int(d.get("timeout", 120), "healthcheck", "timeout", 5, 900)}
    if typ == "http":
        url = _text(d, "url", "healthcheck", 200)
        m = re.fullmatch(r"(https?)://(127\.0\.0\.1|localhost|\[::1\]):(\d{1,5})(/[A-Za-z0-9._~/%?=&-]{0,150})?", url)
        if not m or not 0 < int(m.group(3)) < 65536:
            raise _where("healthcheck", "'url' must be http(s)://127.0.0.1:<port>/<path> (only this unit is probed)")
        h["url"] = url
    elif typ == "tcp":
        h["port"] = _int(d.get("port"), "healthcheck", "port", 1, 65535)
    return h


# ---- templates ----------------------------------------------------------------

BUILTIN_VARS = ("ADDON_ID", "DATA_DIR", "PUID", "PGID", "TZ", "DEVICES_BLOCK")


def template_vars(a_settings: list[dict], a_storage: list[dict]) -> set[str]:
    names = set(BUILTIN_VARS)
    for s in a_settings:
        names.add(s["key"])
        if s["type"] in ("string", "secret", "text", "path"):
            names.add(s["key"] + "_json")
    for st in a_storage:
        names.add("STORAGE_" + st["key"].upper())
    for s in a_settings:
        for c in s.get("choices", []):
            names.update(c["vars"])
    return names


def scan_template(text: str, known: set[str], name: str) -> set[str]:
    """Identifiers used by ``text`` (string.Template syntax); raises on a stray '$' or an unknown name."""
    t = string.Template(text)
    if not t.is_valid():
        raise AddonError("%s: contains a '$' that is not a ${name} placeholder (write $$ for a literal dollar sign)" % name)
    ids = set(t.get_identifiers())
    unknown = sorted(ids - known)
    if unknown:
        raise AddonError("%s: unknown placeholder%s ${%s}" % (name, "s" if len(unknown) > 1 else "", "}, ${".join(unknown)))
    return ids


_RE_BLOCK_VAR = re.compile(r"^\$\{DEVICES_BLOCK\}$", re.M)


def check_devices_block(text: str, name: str) -> None:
    """${DEVICES_BLOCK} is empty or a finished, indented service-level block (ends in a newline), so it stands alone at column 0."""
    n = len(re.findall(r"\$\{?DEVICES_BLOCK\}?", text))
    if n and n != len(_RE_BLOCK_VAR.findall(text)):
        raise AddonError("%s: ${DEVICES_BLOCK} must be alone on its line, at column 0" % name)
    if n > 1:
        raise AddonError("%s: ${DEVICES_BLOCK} may appear only once" % name)


_VOL_DENY = ("/", "/etc", "/root", "/boot", "/proc", "/sys", "/usr", "/bin", "/sbin", "/lib", "/lib64", "/var/lib/docker", "/var/run", "/run", "/dev", "/home", "/var/lib/synofurb-mgmt")
_VOL_ALLOW = ("/etc/localtime", "/etc/timezone")


def audit_compose(text: str, privileges: Iterable[str], devices_ok: bool = False) -> list[str]:
    """Line-based guard on a compose file (template or rendered): privileges must be declared, a few constructs are never allowed.

    This is a consistency check between what the manifest announces and what the file does, not a YAML parser and not a
    sandbox: add-on definitions are trusted code (like a package), the user sees the declared privileges before installing."""
    privs = set(privileges)
    bad: list[str] = []
    lines = text.splitlines()
    for n, raw in enumerate(lines):
        line = raw.split("#", 1)[0] if re.match(r"\s*#", raw) else raw
        low = line.lower()
        if "docker.sock" in low and "docker-socket" not in privs:
            bad.append("mounts the Docker socket but does not declare 'docker-socket'")
        if re.match(r"\s*privileged\s*:\s*[\"']?(true|yes|on|1)\b", low) and "privileged" not in privs:
            bad.append("runs privileged but does not declare 'privileged'")
        if re.match(r"\s*network_mode\s*:\s*[\"']?host\b", low) and "host-network" not in privs:
            bad.append("uses the host network but does not declare 'host-network'")
        if re.match(r"\s*(pid|ipc|uts|userns_mode|cgroup)\s*:\s*[\"']?host\b", low):
            bad.append("shares a host namespace (pid/ipc/uts/userns/cgroup), which is not allowed")
        if re.search(r"unconfined|label[:=]disable", low) and re.search(r"security_opt|^\s*-", low):
            bad.append("turns off container confinement (unconfined / label:disable), which is not allowed")
        if re.match(r"\s*(env_file|extends|include|build|device_cgroup_rules)\s*:", low):
            bad.append("uses '%s', which is not allowed in an add-on" % line.split(":")[0].strip())
        if re.match(r"\s*devices\s*:", low) and not devices_ok:
            bad.append("writes a 'devices:' list by hand; use ${DEVICES_BLOCK}")
        m = re.match(r"(\s*)cap_add\s*:\s*(.*)$", line)
        if m:
            items = re.findall(r"[A-Z][A-Z0-9_]+", m.group(2)) if m.group(2).strip() else []
            ind = len(m.group(1))
            for nxt in lines[n + 1:]:
                if not nxt.strip() or nxt.lstrip().startswith("#"):
                    continue
                mi = re.match(r"(\s*)-\s*[\"']?([A-Za-z_]+)", nxt)
                if not mi or len(mi.group(1)) <= ind:
                    break
                items.append(mi.group(2).upper())
            for cap in items:
                if "cap:" + cap not in privs:
                    bad.append("adds the capability %s but does not declare 'cap:%s'" % (cap, cap))
        for src in re.findall(r"(?:^|[\s\"'-])(/[A-Za-z0-9._/@+-]*)(?=:[/\"'$]|:\s*$)", line):
            s = src.rstrip("/") or "/"
            if s in _VOL_ALLOW:
                continue
            if "docker.sock" in s:
                continue
            for deny in _VOL_DENY:
                if s == deny or (deny != "/" and s.startswith(deny + "/")):
                    if s.startswith("/dev/") and devices_ok:
                        continue
                    bad.append("mounts the host path %s, which is not allowed" % s)
                    break
    return sorted(set(bad))


def parse_manifest(raw: Any, folder_id: str, files: dict[str, str], source: str = "builtin", handlers: Iterable[str] = ()) -> Addon:
    """Validate ``addon.json`` (already parsed) together with the add-on's flat file set; raises AddonError with a precise path."""
    d = _need_dict(raw, "addon.json")
    _only_keys(d, TOP_KEYS, "addon.json")
    if d.get("schema") != SCHEMA or isinstance(d.get("schema"), bool):
        raise _where("addon.json", "'schema' must be %d" % SCHEMA)
    aid = _text(d, "id", "addon.json", 32, regex=ID_RE)
    if aid != folder_id:
        raise _where("addon.json", "'id' (%s) must equal the folder name (%s)" % (aid, folder_id))
    kind = d.get("kind")
    if kind not in ("compose", "system"):
        raise _where("addon.json", "'kind' must be compose or system")
    requires = _need_list(d.get("requires", []), "requires", 8)
    if not all(isinstance(r, str) and ID_RE.fullmatch(r) for r in requires) or len(set(requires)) != len(requires) or aid in requires:
        raise _where("addon.json", "'requires' must list other add-on ids once each")
    seen: set = set()
    settings = [_parse_setting(s, i, seen) for i, s in enumerate(_need_list(d.get("settings", []), "settings", 40))]
    seen = set()
    storage = [_parse_storage(s, i, seen) for i, s in enumerate(_need_list(d.get("storage", []), "storage", 12))]
    seen = set()
    devices = [_parse_device(s, i, seen) for i, s in enumerate(_need_list(d.get("devices", []), "devices", 12))]
    privs = _parse_privileges(d.get("privileges", []), "privileges")
    ports = [_parse_port(p, i) for i, p in enumerate(_need_list(d.get("ports", []), "ports", 24))]
    web = _parse_web(d["web"]) if d.get("web") is not None else None
    health = _parse_health(d["healthcheck"]) if d.get("healthcheck") is not None else None
    handler = _text(d, "handler", "addon.json", 32, required=False)
    compose = _text(d, "compose", "addon.json", 64, required=False, regex=FILENAME_RE)
    flist = []
    for i, f in enumerate(_need_list(d.get("files", []), "files", MAX_FILES_PER_ADDON)):
        fp = "files[%d]" % i
        fd = _need_dict(f, fp)
        _only_keys(fd, ("template", "dest", "once"), fp)
        once = fd.get("once", False)
        if not isinstance(once, bool):
            raise _where(fp, "'once' must be true or false")
        flist.append({"template": _text(fd, "template", fp, 64, regex=FILENAME_RE), "dest": valid_dest(fd.get("dest"), fp), "once": once})
    if len({f["dest"] for f in flist}) != len(flist):
        raise _where("files", "two files have the same 'dest'")
    if kind == "system":
        if handler not in set(handlers):
            raise _where("addon.json", "system add-ons need a known 'handler' (known: %s)" % (", ".join(sorted(handlers)) or "none"))
        if compose or flist or storage or devices or ports or privs:
            raise _where("addon.json", "a system add-on has no compose file, files, ports, storage, devices or privileges")
    else:
        if handler:
            raise _where("addon.json", "'handler' is only for system add-ons")
        if not compose:
            raise _where("addon.json", "a compose add-on needs 'compose' (the template file name)")
    homepage = _text(d, "homepage", "addon.json", 200, required=False, regex=HOMEPAGE_RE)
    changes = [c for c in _need_list(d.get("changes", []), "changes", 8)]
    if not all(isinstance(c, str) and 0 < len(c) <= 140 and not CTRL_RE.search(c) for c in changes):
        raise _where("changes", "must be a list of short sentences")
    addon = Addon(id=aid, name=_text(d, "name", "addon.json", 60), version=_text(d, "version", "addon.json", 32, regex=VERSION_RE),
                  summary=_text(d, "summary", "addon.json", 140), category=_text(d, "category", "addon.json", 24, required=False, default="other", regex=CATEGORY_RE),
                  icon=_text(d, "icon", "addon.json", 40, required=False, default="package", regex=ICON_RE), homepage=homepage, kind=kind, handler=handler,
                  requires=list(requires), web=web, ports=ports, settings=settings, storage=storage, devices=devices, privileges=list(privs), compose=compose,
                  files=flist, healthcheck=health, post_install=_text(d, "post_install", "addon.json", 3000, required=False), changes=changes, source=source)
    _check_templates(addon, files)
    return addon


def placeholder_rules(text: str, in_compose: bool) -> dict[str, str]:
    """For every placeholder: how strict its value must be, judged by where it sits on its line (inside "double quotes" or not)."""
    out: dict[str, str] = {}
    for m in string.Template.pattern.finditer(text):
        name = m.group("named") or m.group("braced")
        if not name:
            continue
        line_start = text.rfind("\n", 0, m.start()) + 1
        before = text[line_start:m.start()]
        dq = len(re.findall(r'(?<!\\)"', before)) % 2 == 1
        sq = before.count("'") % 2 == 1
        rule = ("cquoted" if in_compose else "fquoted") if dq else ("raw" if in_compose or sq else "loose")
        if name not in out or RULE_ORDER.index(rule) < RULE_ORDER.index(out[name]):
            out[name] = rule
    return out


def _check_templates(a: Addon, files: dict[str, str]) -> None:
    if a.kind != "compose":
        return
    known = template_vars(a.settings, a.storage)
    need = [a.compose] + [f["template"] for f in a.files]
    for n in need:
        if n not in files:
            raise AddonError("%s: the file %s is listed in addon.json but is missing" % (a.id, n))
    if len(set(need)) != len(need):
        raise AddonError("files: a template file is listed twice (or the compose template is also a rendered file)")
    # variables set by choice options map back to their setting (for the "first install only" note and value rules)
    owner = {name: s["key"] for s in a.settings for c in s.get("choices", [])[:1] for name in c["vars"]}
    rules: dict[str, str] = {}
    usage: dict[str, set] = {}                    # setting key -> {"compose" | "always" | "once"}
    for n in need:
        text = files[n]
        ids = scan_template(text, known, n)
        check_devices_block(text, n)
        in_compose = n == a.compose
        for name, rule in placeholder_rules(text, in_compose).items():
            key = owner.get(name, name[:-5] if name.endswith("_json") else name)
            if a.setting(key) and name == key and (key not in rules or RULE_ORDER.index(rule) < RULE_ORDER.index(rules[key])):
                rules[key] = rule
        for name in ids:
            key = owner.get(name, name[:-5] if name.endswith("_json") else name)
            if a.setting(key):
                f = next((x for x in a.files if x["template"] == n), None)
                usage.setdefault(key, set()).add("compose" if in_compose else ("once" if f and f["once"] else "always"))
        if in_compose:
            for i in ids:
                st = a.setting(i)
                if st and st["type"] == "text":
                    raise AddonError("%s: the text setting '%s' cannot be used in the compose file (use it in a rendered file)" % (n, i))
            if "DEVICES_BLOCK" in ids and not a.devices:
                raise AddonError("%s: uses ${DEVICES_BLOCK} but the add-on declares no devices" % n)
            problems = audit_compose(text, a.privileges)
            if problems:
                raise AddonError("%s: %s" % (n, "; ".join(problems)))
    # per-choice blocks must be consistent with the privileges that choice declares
    for s in a.settings:
        for c in s.get("choices", []):
            for vname, vtext in c["vars"].items():
                problems = audit_compose(vtext, list(a.privileges) + list(c["privileges"])) if vname in _compose_vars(a, files) else []
                if problems:
                    raise AddonError("settings (%s) choice %s, variable %s: %s" % (s["key"], c["value"], vname, "; ".join(problems)))
    a.templates = {n: files[n] for n in need}
    a.value_rules = {k: r for k, r in rules.items()}
    for s in a.settings:
        s["first_install_only"] = bool(usage.get(s["key"])) and usage[s["key"]] == {"once"}


def _compose_vars(a: Addon, files: dict[str, str]) -> set[str]:
    return set(string.Template(files[a.compose]).get_identifiers())


# ============================================================================
# 2. Catalog: directory / pack / user folder, dependency order
# ============================================================================

def read_addon_dir(path: str) -> dict[str, str]:
    """All files of one add-on folder (flat, regular, small, UTF-8); symlinks, sub-folders and odd names are rejected."""
    out: dict[str, str] = {}
    try:
        names = sorted(os.listdir(path))
    except OSError as e:
        raise AddonError("cannot read %s: %s" % (path, e.strerror or e))
    if len(names) > MAX_FILES_PER_ADDON + 2:
        raise AddonError("too many files")
    for n in names:
        p = os.path.join(path, n)
        if not FILENAME_RE.fullmatch(n):
            raise AddonError("file name %r is not allowed" % n[:40])
        if os.path.islink(p) or not os.path.isfile(p):
            raise AddonError("%s is not a regular file (links and folders are not allowed)" % n)
        if os.path.getsize(p) > MAX_FILE:
            raise AddonError("%s is too large" % n)
        try:
            with open(p, encoding="utf-8") as fh:
                out[n] = fh.read()
        except (OSError, UnicodeDecodeError):
            raise AddonError("%s is not readable UTF-8 text" % n)
    return out


def build_pack(addons_dir: str) -> str:
    """The shipped form of ``addons/``: one deterministic JSON document (the update bundle is a flat file list)."""
    pack: dict[str, dict] = {}
    for aid in sorted(os.listdir(addons_dir)) if os.path.isdir(addons_dir) else []:
        p = os.path.join(addons_dir, aid)
        if os.path.isdir(p) and not os.path.islink(p) and ID_RE.fullmatch(aid):
            pack[aid] = read_addon_dir(p)
    return json.dumps({"schema": PACK_SCHEMA, "addons": pack}, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n"


def read_pack(path: str) -> dict[str, dict[str, str]]:
    try:
        with open(path, encoding="utf-8") as fh:
            doc = json.load(fh)
    except (OSError, ValueError):
        return {}
    if not isinstance(doc, dict) or doc.get("schema") != PACK_SCHEMA or not isinstance(doc.get("addons"), dict):
        return {}
    out = {}
    for aid, fl in doc["addons"].items():
        if isinstance(aid, str) and ID_RE.fullmatch(aid) and isinstance(fl, dict) and all(
                isinstance(k, str) and FILENAME_RE.fullmatch(k) and isinstance(v, str) and len(v) <= MAX_FILE for k, v in fl.items()):
            out[aid] = fl
    return out


@dataclass
class Catalog:
    addons: dict[str, Addon] = field(default_factory=dict)
    problems: list[dict] = field(default_factory=list)

    def get(self, aid: str) -> Addon:
        if not isinstance(aid, str) or not ID_RE.fullmatch(aid):
            raise AddonError("not a valid add-on id")
        a = self.addons.get(aid)
        if a is None:
            raise AddonError("unknown add-on '%s'" % aid)
        return a

    def order(self) -> list[Addon]:
        """Dependency order (requires first), ties by id; built-ins keep the documented order."""
        out: list[Addon] = []
        seen: set = set()

        def visit(a: Addon) -> None:
            if a.id in seen:
                return
            seen.add(a.id)
            for r in sorted(a.requires):
                if r in self.addons:
                    visit(self.addons[r])
            out.append(a)
        for aid in sorted(self.addons, key=lambda i: (i not in BUILTIN_IDS_ORDER, BUILTIN_IDS_ORDER.index(i) if i in BUILTIN_IDS_ORDER else 0, i)):
            visit(self.addons[aid])
        return out

    def dependents(self, aid: str) -> list[str]:
        return sorted(a.id for a in self.addons.values() if aid in a.requires)


def _find_cycle(graph: dict[str, list[str]]) -> list[str] | None:
    colour: dict[str, int] = {}
    stack: list[str] = []

    def dfs(n: str) -> list[str] | None:
        colour[n] = 1
        stack.append(n)
        for m in graph.get(n, []):
            if m not in graph:
                continue
            if colour.get(m, 0) == 1:
                return stack[stack.index(m):] + [m]
            if colour.get(m, 0) == 0:
                c = dfs(m)
                if c:
                    return c
        stack.pop()
        colour[n] = 2
        return None
    for n in sorted(graph):
        if colour.get(n, 0) == 0:
            c = dfs(n)
            if c:
                return c
    return None


def load_catalog(sources: list[tuple[str, dict[str, dict[str, str]]]], handlers: Iterable[str] = ("docker",)) -> Catalog:
    """``sources``: [(\"builtin\"|\"user\", {id: {filename: text}})].  One bad add-on never hides the others; it lands in ``problems``."""
    cat = Catalog()
    handlers = tuple(handlers)
    for source, found in sources:
        for aid in sorted(found):
            files = found[aid]
            try:
                if not ID_RE.fullmatch(aid):
                    raise AddonError("folder name %r is not a valid add-on id" % aid[:40])
                if aid in cat.addons:
                    raise AddonError("the id '%s' is already provided by the %s catalog; choose another id" % (aid, cat.addons[aid].source))
                if "addon.json" not in files:
                    raise AddonError("addon.json is missing")
                try:
                    raw = json.loads(files["addon.json"])
                except ValueError as e:
                    raise AddonError("addon.json is not valid JSON: %s" % e)
                cat.addons[aid] = parse_manifest(raw, aid, files, source, handlers)
            except AddonError as e:
                cat.problems.append({"id": aid, "source": source, "error": str(e)})
    # unknown or rejected requirements, then cycles (every member of a cycle is rejected); repeat until nothing changes
    while True:
        dropped = False
        for a in sorted(cat.addons.values(), key=lambda x: x.id):
            missing = [r for r in a.requires if r not in cat.addons]
            if missing:
                cat.problems.append({"id": a.id, "source": a.source, "error": "requires %s, which is not available" % ", ".join("'%s'" % m for m in missing)})
                del cat.addons[a.id]
                dropped = True
        cyc = _find_cycle({a.id: a.requires for a in cat.addons.values()})
        if cyc:
            for n in sorted(set(cyc)):
                a = cat.addons.pop(n)
                cat.problems.append({"id": n, "source": a.source, "error": "dependency cycle: %s" % " -> ".join(cyc)})
            dropped = True
        if not dropped:
            break
    return cat


# ============================================================================
# 3. Environment (everything the framework needs from the host), Docker abstraction
# ============================================================================

@dataclass
class Result:
    rc: int
    out: str = ""
    err: str = ""

    @property
    def ok(self) -> bool:
        return self.rc == 0


SAFE_PATH = "/usr/sbin:/usr/bin:/sbin:/bin"
CHILD_ENV = {"PATH": SAFE_PATH, "LC_ALL": "C", "LANG": "C", "HOME": "/root", "DEBIAN_FRONTEND": "noninteractive", "DOCKER_CLI_HINTS": "false",
             "NEEDRESTART_MODE": "a", "COMPOSE_MENU": "false"}


def stream_command(argv: list[str], on_line: Callable[[str], None], timeout: int) -> Result:
    """Run ``argv`` (no shell), call ``on_line`` for every output line, kill it after ``timeout`` seconds.  Returns the last output lines."""
    if not argv or not all(isinstance(a, str) and "\x00" not in a for a in argv):
        raise ValueError("argv must be a non-empty list of NUL-free strings")
    exe = argv[0] if argv[0].startswith("/") else shutil.which(argv[0], path=SAFE_PATH)
    if not exe or not os.access(exe, os.X_OK):
        return Result(127, "", "command not found: " + argv[0])
    try:
        p = subprocess.Popen([exe] + argv[1:], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=CHILD_ENV, shell=False,
                             text=True, errors="replace", bufsize=1)
    except OSError as e:
        return Result(126, "", "cannot execute %s: %s" % (argv[0], e))
    timer = threading.Timer(timeout, p.kill)
    timer.start()
    tail: collections.deque = collections.deque(maxlen=60)
    try:
        assert p.stdout is not None
        for raw in p.stdout:
            line = raw.rstrip("\r\n")
            tail.append(line)
            on_line(line)
        rc = p.wait()
    finally:
        timer.cancel()
        if p.poll() is None:
            p.kill()
    if rc < 0:
        return Result(124, "\n".join(tail), "timed out after %ds: %s" % (timeout, argv[0]))
    return Result(rc, "\n".join(tail), "")


class AddonEnv:
    """The seams of the framework.  ``synofurb_mgmt`` builds one from its App; tests build one around a temp directory."""

    def __init__(self, root: str = "", run: Callable[..., Any] | None = None, stream: Callable[..., Any] | None = None,
                 mounts: Callable[[], list[dict]] | None = None, audit: Callable[..., None] | None = None, script_dir: str | None = None,
                 state_dir: str = "/var/lib/synofurb-mgmt", user_dir: str = "/etc/synofurb/addons.d", login_group: str = "sudo",
                 dev_root: str = "/dev", sys_root: str = "/sys",
                 sleep: Callable[[float], None] = time.sleep, now: Callable[[], float] = time.time,
                 job_lock: Any = None, http_probe: Callable[[str, int], int | None] | None = None, tcp_probe: Callable[[int, int], bool] | None = None,
                 port_in_use: Callable[[int], bool] | None = None, chown: bool | None = None):
        self.root = root
        self._run = run
        self._stream = stream
        self.mounts = mounts or (lambda: [])
        self._audit = audit
        self.script_dir = script_dir or os.path.dirname(os.path.abspath(__file__))
        self.state_dir, self.user_dir, self.login_group = state_dir, user_dir, login_group
        self.dev_root, self.sys_root = dev_root, sys_root
        self.job_lock = job_lock or FlockJobLock(self.state_path("addons.lock"))
        self.sleep, self.now = sleep, now
        self.http_probe = http_probe or probe_http
        self.tcp_probe = tcp_probe or probe_tcp
        self.port_in_use = port_in_use or (lambda port: probe_tcp(port, 1))
        self.chown = (not root and os.geteuid() == 0) if chown is None else chown

    # -- paths ---------------------------------------------------------------
    def p(self, path: str) -> str:
        return os.path.normpath(os.path.join(self.root, path.lstrip("/"))) if self.root else path

    def state_path(self, name: str) -> str:
        return self.p(self.state_dir + "/" + name)

    # -- commands --------------------------------------------------------------
    def run(self, argv: list[str], input: str | None = None, timeout: int = 60) -> Result:
        if self._run is None:
            return Result(127, "", "no command runner configured")
        r = self._run(argv, input=input, timeout=timeout)
        return Result(r.rc, r.out, r.err)

    def stream(self, argv: list[str], on_line: Callable[[str], None], timeout: int = 600) -> Result:
        if self._stream is not None:
            return self._stream(argv, on_line, timeout)
        r = self.run(argv, timeout=timeout)
        for ln in (r.out + ("\n" + r.err if r.err else "")).splitlines():
            on_line(ln)
        return r

    def audit(self, user: str | None, ip: str, action: str, args: dict, result: str) -> None:
        if self._audit:
            self._audit(user, ip, action, args, result)

    # -- host facts ------------------------------------------------------------
    def identity(self) -> dict:
        """PUID/PGID (the first administrator with a regular uid, else 1000), time zone and the administrator names."""
        pw, gr = _read(self.p("/etc/passwd")), _read(self.p("/etc/group"))
        users: dict[str, tuple[int, int]] = {}
        for ln in pw.splitlines():
            f = ln.split(":")
            if len(f) >= 4 and USER_RE.fullmatch(f[0]) and f[2].isdigit() and f[3].isdigit():
                users[f[0]] = (int(f[2]), int(f[3]))
        admins: list[str] = []
        for ln in gr.splitlines():
            f = ln.split(":")
            if len(f) >= 4 and f[0] == self.login_group:
                admins = [u for u in f[3].split(",") if u in users and users[u][0] >= 1000]
        puid, pgid = users[admins[0]] if admins else (1000, 1000)
        puid, pgid = puid or 1000, pgid or 1000                      # never map a container user to root (uid 0 / gid 0)
        tz = "UTC"
        t = _read(self.p("/etc/timezone")).strip()
        if t and TZ_RE.fullmatch(t):
            tz = t
        else:
            try:
                link = os.readlink(self.p("/etc/localtime"))
                if "zoneinfo/" in link:
                    cand = link.split("zoneinfo/", 1)[1]
                    if TZ_RE.fullmatch(cand):
                        tz = cand
            except OSError:
                pass
        return {"puid": puid, "pgid": pgid, "tz": tz, "admins": admins}

    def hostpath(self, path: str) -> str:
        """Map /dev/... and /sys/... into the (test) device and sysfs roots."""
        if path.startswith("/dev/") or path == "/dev":
            return self.dev_root + path[4:]
        if path.startswith("/sys/") or path == "/sys":
            return self.sys_root + path[4:]
        return self.p(path)


def _read(path: str) -> str:
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            return fh.read()
    except OSError:
        return ""


def _write_atomic(path: str, data: str, mode: int = 0o644, uid: int | None = None, gid: int | None = None) -> None:
    d = os.path.dirname(path)
    os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".tmp-synofurb-")
    try:
        os.fchmod(fd, mode)
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(data)
            fh.flush()
            os.fsync(fh.fileno())
        if uid is not None:
            try:
                os.chown(tmp, uid, gid if gid is not None else -1)
            except OSError:
                pass
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def probe_tcp(port: int, timeout: int = 3) -> bool:
    for host in ("127.0.0.1", "::1"):
        try:
            with socket.create_connection((host, port), timeout=timeout):
                return True
        except OSError:
            continue
    return False


def probe_http(url: str, timeout: int = 4) -> int | None:
    """HTTP status of a loopback URL (no redirects followed, certificates not verified: loopback only); None when nothing answers."""
    m = re.fullmatch(r"(https?)://(127\.0\.0\.1|localhost|\[::1\]):(\d{1,5})(/.*)?", url)
    if not m:
        return None
    host = m.group(2).strip("[]")
    try:
        if m.group(1) == "https":
            ctx = ssl.create_default_context()
            ctx.check_hostname = False
            ctx.verify_mode = ssl.CERT_NONE
            conn: http.client.HTTPConnection = http.client.HTTPSConnection(host, int(m.group(3)), timeout=timeout, context=ctx)
        else:
            conn = http.client.HTTPConnection(host, int(m.group(3)), timeout=timeout)
        conn.request("GET", m.group(4) or "/", headers={"User-Agent": "synofurb-mgmt", "Connection": "close"})
        status = conn.getresponse().status
        conn.close()
        return status
    except (OSError, http.client.HTTPException, ssl.SSLError):
        return None


# ---- Docker -------------------------------------------------------------------

PROJECT_RE = re.compile(r"synofurb-[a-z0-9][a-z0-9-]{0,31}")
COMPOSE_FILE_RE = re.compile(r"/[A-Za-z0-9._@+/-]{1,300}")


def project_name(aid: str) -> str:
    if not ID_RE.fullmatch(aid):
        raise AddonError("not a valid add-on id")
    return "synofurb-" + aid


class Docker:
    """The only place that builds ``docker`` command lines.  Real implementation on top of the environment's runner."""

    def __init__(self, env: AddonEnv):
        self.env = env

    # argv -----------------------------------------------------------------
    def argv(self, project: str, file: str, *args: str, flags: tuple = ()) -> list[str]:
        if not PROJECT_RE.fullmatch(project):
            raise AddonError("bad compose project name")
        if not COMPOSE_FILE_RE.fullmatch(file) or ".." in file.split("/"):
            raise AddonError("bad compose file path")
        return ["docker", "compose", *flags, "-p", project, "-f", file, *args]

    def _go(self, argv: list[str], timeout: int, on_line: Callable[[str], None] | None) -> Result:
        return self.env.stream(argv, on_line, timeout) if on_line else self.env.run(argv, timeout=timeout)

    # engine ---------------------------------------------------------------
    def available(self) -> tuple[bool, str]:
        r = self.env.run(["docker", "version", "--format", "{{.Server.Version}}"], timeout=20)
        if r.ok and r.out.strip():
            return True, r.out.strip().splitlines()[0]
        return False, (r.err or r.out or "docker is not running").strip().splitlines()[-1][:200] if (r.err or r.out) else "docker is not running"

    def compose_available(self) -> tuple[bool, str]:
        r = self.env.run(["docker", "compose", "version", "--short"], timeout=20)
        return (True, r.out.strip()) if r.ok and r.out.strip() else (False, "the docker compose plugin is not installed")

    # project --------------------------------------------------------------
    def config(self, project: str, file: str) -> Result:
        return self.env.run(self.argv(project, file, "config", "-q"), timeout=60)

    def pull(self, project: str, file: str, on_line: Callable[[str], None] | None = None) -> Result:
        return self._go(self.argv(project, file, "pull", flags=("--ansi", "never", "--progress", "plain")), 3600, on_line)

    def up(self, project: str, file: str, on_line: Callable[[str], None] | None = None) -> Result:
        return self._go(self.argv(project, file, "up", "-d", "--remove-orphans", flags=("--ansi", "never", "--progress", "plain")), 900, on_line)

    def stop(self, project: str, file: str) -> Result:
        return self.env.run(self.argv(project, file, "stop"), timeout=180)

    def restart(self, project: str, file: str) -> Result:
        return self.env.run(self.argv(project, file, "restart"), timeout=300)

    def down(self, project: str, file: str, volumes: bool = False) -> Result:
        return self.env.run(self.argv(project, file, "down", "--remove-orphans", *(["-v"] if volumes else [])), timeout=300)

    def ps(self, project: str, file: str) -> list[dict]:
        r = self.env.run(self.argv(project, file, "ps", "-a", "--format", "json"), timeout=30)
        return parse_ps(r.out) if r.ok else []

    def logs(self, project: str, file: str, tail: int) -> Result:
        return self.env.run(self.argv(project, file, "logs", "--no-color", "--timestamps", "--tail", str(int(tail))), timeout=60)


def parse_ps(text: str) -> list[dict]:
    """``docker compose ps --format json``: one JSON array (older) or one object per line (newer)."""
    items: list = []
    text = text.strip()
    if not text:
        return []
    try:
        doc = json.loads(text)
        items = doc if isinstance(doc, list) else [doc]
    except ValueError:
        for ln in text.splitlines():
            try:
                items.append(json.loads(ln))
            except ValueError:
                continue
    out = []
    for it in items:
        if isinstance(it, dict):
            out.append({"name": str(it.get("Name") or it.get("Service") or "")[:80], "service": str(it.get("Service") or "")[:60], "state": str(it.get("State") or "").lower(),
                        "health": str(it.get("Health") or "").lower(), "exit_code": it.get("ExitCode") if isinstance(it.get("ExitCode"), int) else 0})
    return out


class FakeDocker(Docker):
    """Test double (also used by the web demo): remembers projects and containers, records every call, can be told to fail."""

    def __init__(self, env: AddonEnv | None = None, engine: bool = True, compose: bool = True):
        super().__init__(env or AddonEnv())
        self.calls: list[tuple] = []
        self.projects: dict[str, dict] = {}
        self.fail: dict[str, str] = {}            # operation -> error text
        self.engine, self.compose = engine, compose
        self.log_text: dict[str, str] = {}

    def _rec(self, op: str, project: str = "", *a: Any) -> Result | None:
        self.calls.append((op, project) + a)
        if op in self.fail:
            return Result(1, "", self.fail[op])
        return None

    def available(self) -> tuple[bool, str]:
        return (True, "26.1.5") if self.engine else (False, "docker is not running")

    def compose_available(self) -> tuple[bool, str]:
        return (True, "2.26.1") if self.compose else (False, "the docker compose plugin is not installed")

    def _proj(self, project: str) -> dict:
        return self.projects.setdefault(project, {"state": "absent", "file": "", "services": ["app"]})

    def config(self, project: str, file: str) -> Result:
        self.argv(project, file)
        r = self._rec("config", project, file)
        return r or Result(0)

    def pull(self, project: str, file: str, on_line: Callable[[str], None] | None = None) -> Result:
        self.argv(project, file)
        r = self._rec("pull", project, file)
        if on_line and not r:
            on_line("app Pulling")
            on_line("app Pulled")
        return r or Result(0, "app Pulled")

    def up(self, project: str, file: str, on_line: Callable[[str], None] | None = None) -> Result:
        self.argv(project, file)
        r = self._rec("up", project, file)
        if r:
            return r
        self._proj(project).update(state="running", file=file)
        if on_line:
            on_line("Container %s-app-1 Started" % project)
        return Result(0, "started")

    def stop(self, project: str, file: str) -> Result:
        self.argv(project, file)
        r = self._rec("stop", project, file)
        if not r:
            self._proj(project)["state"] = "exited"
        return r or Result(0)

    def restart(self, project: str, file: str) -> Result:
        self.argv(project, file)
        r = self._rec("restart", project, file)
        if not r:
            self._proj(project)["state"] = "running"
        return r or Result(0)

    def down(self, project: str, file: str, volumes: bool = False) -> Result:
        self.argv(project, file)
        r = self._rec("down", project, file, volumes)
        if not r:
            self.projects.pop(project, None)
        return r or Result(0)

    def ps(self, project: str, file: str) -> list[dict]:
        self.argv(project, file)
        self.calls.append(("ps", project, file))
        p = self.projects.get(project)
        if not p or p["state"] == "absent":
            return []
        st = p["state"]
        return [{"name": "%s-%s-1" % (project, s), "service": s, "state": st, "health": "", "exit_code": 0 if st == "running" else 0} for s in p["services"]]

    def logs(self, project: str, file: str, tail: int) -> Result:
        self.argv(project, file)
        r = self._rec("logs", project, file, tail)
        return r or Result(0, self.log_text.get(project, "2026-01-01T00:00:00Z app  | started\n"))


# ============================================================================
# 4. Devices, storage volumes, rendering
# ============================================================================

def detect_device(env: AddonEnv, dev: dict) -> bool:
    """Is the hardware there?  ``path:`` checks a file, ``usb:`` looks through /sys/bus/usb/devices for a vendor:product pair."""
    det = dev.get("detect") or ""
    if det.startswith("usb:"):
        wanted = set(det[4:].split("|"))
        base = env.sys_root + "/bus/usb/devices"
        try:
            names = os.listdir(base)
        except OSError:
            return False
        for n in names:
            vid = _read(os.path.join(base, n, "idVendor")).strip().lower()
            pid = _read(os.path.join(base, n, "idProduct")).strip().lower()
            if vid and pid and "%s:%s" % (vid, pid) in wanted:
                return True
        return False
    target = det[5:] if det.startswith("path:") else dev["host"]
    return os.path.exists(env.hostpath(target))


def device_states(env: AddonEnv, addon: Addon) -> list[dict]:
    return [{"id": d["id"], "label": d["label"], "host": d["host"], "optional": d["optional"], "present": detect_device(env, d)} for d in addon.devices]


def choose_devices(env: AddonEnv, addon: Addon, wanted: dict | None) -> tuple[list[dict], list[str]]:
    """Devices to pass through and warnings.  Default: every present device.  An explicit true forces an absent one (with a warning)."""
    wanted = wanted or {}
    unknown = [k for k in wanted if not any(d["id"] == k for d in addon.devices)]
    if unknown:
        raise AddonError("unknown device '%s'" % unknown[0][:40])
    chosen, warns = [], []
    for st in device_states(env, addon):
        want = wanted.get(st["id"])
        if want is not None and not isinstance(want, bool):
            raise AddonError("device %s must be true or false" % st["id"])
        on = st["present"] if want is None else want
        if on and not st["present"]:
            warns.append("%s was not found but you asked for it; the app may not start" % st["label"])
        if not on and not st["optional"] and want is None:
            raise AddonError("%s is required but was not found" % st["label"])
        if on:
            chosen.append(next(d for d in addon.devices if d["id"] == st["id"]))
    return chosen, warns


def devices_block(chosen: list[dict]) -> str:
    """${DEVICES_BLOCK}: empty, or a finished service-level block indented four spaces (ends in a newline)."""
    if not chosen:
        return ""
    return "    devices:\n" + "".join("      - %s:%s\n" % (d["host"], d["host"]) for d in chosen)


def logical_realpath(env: AddonEnv, path: str) -> str:
    """Resolve symlinks of the nearest existing ancestor; returns a path in the logical (un-prefixed) namespace."""
    real = env.p(path)
    rest: list[str] = []
    while real and not os.path.lexists(real):
        real, tail = os.path.split(real)
        rest.insert(0, tail)
    real = os.path.realpath(real or "/")
    full = os.path.join(real, *rest) if rest else real
    if env.root:
        rr = os.path.realpath(env.root)
        if full == rr or full.startswith(rr + "/"):
            full = full[len(rr):] or "/"
    return full


def is_data_volume(m: dict | None) -> bool:
    """The data volume: a filesystem mounted at /srv/data or below it (never the root filesystem)."""
    return bool(m) and str(m.get("target") or "") != "/" and (str(m["target"]) == DATA_VOLUME or str(m["target"]).startswith(DATA_VOLUME + "/"))


def volume_of(path: str, mounts: list[dict]) -> dict | None:
    best = None
    for m in mounts:
        t = str(m.get("target") or "")
        if t and (t == "/" or path == t or path.startswith(t.rstrip("/") + "/")) and (best is None or len(t) > len(str(best["target"]))):
            best = m
    return best


def storage_report(env: AddonEnv, addon: Addon, paths: dict[str, str]) -> list[dict]:
    mounts = env.mounts()
    out = []
    for st in addon.storage:
        p = paths.get(st["key"], st["path"])
        real = logical_realpath(env, p)
        vol = volume_of(real, mounts)
        row = {"key": st["key"], "label": st["label"], "path": p, "bulk": st["bulk"], "min_free_gib": st["min_free_gib"], "volume": vol.get("target") if vol else "/",
               "redundant": bool(vol and is_data_volume(vol)), "free_gib": None, "error": "", "warning": ""}
        probe = env.p(real)
        while probe and not os.path.exists(probe):
            probe = os.path.dirname(probe)
        try:
            vfs = os.statvfs(probe or "/")
            row["free_gib"] = round(vfs.f_bavail * vfs.f_frsize / 1024 ** 3, 1)
        except OSError:
            pass
        if st["bulk"] and not row["redundant"]:
            row["error"] = ("%s is not on a redundant data volume. Create a RAID or LVM volume under Storage and mount it below /srv or /mnt, "
                            "then choose a folder on it." % st["label"])
        elif row["free_gib"] is not None and st["min_free_gib"] and row["free_gib"] < st["min_free_gib"]:
            row["warning"] = "only %s GiB free, %d GiB recommended" % (row["free_gib"], st["min_free_gib"])
        out.append(row)
    return out


@dataclass
class Rendered:
    dest: str
    text: str
    mode: int
    once: bool
    chown: bool


def _stringify(v: Any) -> str:
    return ("true" if v else "false") if isinstance(v, bool) else str(v)


def _json_quote(v: str, compose: bool) -> str:
    s = json.dumps(v, ensure_ascii=True)
    return s.replace("$", "$$") if compose else s


def build_vars(addon: Addon, values: dict, storage: dict[str, str], ident: dict, compose: bool, block: str = "") -> dict[str, str]:
    esc = (lambda t: t.replace("$", "$$")) if compose else (lambda t: t)
    v: dict[str, str] = {"ADDON_ID": addon.id, "DATA_DIR": addon.data_dir, "PUID": str(ident["puid"]), "PGID": str(ident["pgid"]), "TZ": ident["tz"], "DEVICES_BLOCK": block}
    for s in addon.settings:
        val = values.get(s["key"], "")
        sv = _stringify(val)
        if s["type"] in ("string", "secret", "text", "path"):
            v[s["key"] + "_json"] = _json_quote(sv, compose)
        v[s["key"]] = sv
        if s["type"] == "choice":                                   # the chosen option's variables (conditional blocks)
            for c in s["choices"]:
                if c["value"] == val:
                    v.update({k: esc(t) for k, t in c["vars"].items()})
    for st in addon.storage:
        v["STORAGE_" + st["key"].upper()] = storage.get(st["key"], st["path"])
    return v


def render_template(text: str, variables: dict[str, str], name: str) -> str:
    try:
        return string.Template(text).substitute(variables)
    except KeyError as e:
        raise AddonError("%s: no value for ${%s}" % (name, str(e).strip("'")))
    except ValueError as e:
        raise AddonError("%s: %s" % (name, e))


def render_addon(addon: Addon, values: dict, storage: dict[str, str], chosen_devices: list[dict], ident: dict) -> tuple[Rendered, list[Rendered]]:
    """Render the compose file and the extra files.  A file that uses a secret is written 0600."""
    secret_keys = set(addon.secret_keys())
    owner = {name: s["key"] for s in addon.settings for c in s.get("choices", [])[:1] for name in c["vars"]}

    def uses_secret(text: str) -> bool:
        ids = set(string.Template(text).get_identifiers())
        return any(i in secret_keys or (i.endswith("_json") and i[:-5] in secret_keys) or owner.get(i) in secret_keys for i in ids)
    block = devices_block(chosen_devices)
    ctext = addon.templates[addon.compose]
    out = render_template(ctext, build_vars(addon, values, storage, ident, True, block), addon.compose)
    problems = audit_compose(out, effective_privileges(addon, values), devices_ok=bool(block))
    if problems:
        raise AddonError("the compose file would %s" % "; ".join(problems))
    if not out.endswith("\n"):
        out += "\n"
    compose = Rendered("compose.yml", out, 0o600 if uses_secret(ctext) else 0o644, False, False)
    fvars = build_vars(addon, values, storage, ident, False, block)
    extra = []
    for f in addon.files:
        t = addon.templates[f["template"]]
        extra.append(Rendered(f["dest"], render_template(t, fvars, f["template"]), 0o600 if uses_secret(t) else 0o640, f["once"], True))
    return compose, extra


def write_rendered(env: AddonEnv, addon: Addon, r: Rendered, ident: dict, name: str | None = None) -> bool:
    """Write one rendered file below the add-on's data dir (never through a link).  Returns False when ``once`` skipped it."""
    base = env.p(addon.data_dir)
    dest = os.path.join(base, name or r.dest)
    parent = os.path.dirname(dest)
    os.makedirs(parent, mode=0o755, exist_ok=True)
    real_base = os.path.realpath(base)
    if os.path.realpath(parent) != real_base and not os.path.realpath(parent).startswith(real_base + os.sep):
        raise AddonError("%s would be written outside the add-on's folder" % r.dest)
    if r.once and os.path.lexists(dest):
        return False
    if os.path.islink(dest) or (os.path.exists(dest) and not os.path.isfile(dest)):
        raise AddonError("%s exists and is not a regular file" % r.dest)
    uid = ident["puid"] if (r.chown and env.chown) else None
    _write_atomic(dest, r.text, r.mode, uid, ident["pgid"] if uid is not None else None)
    return True


# ============================================================================
# 5. State, secrets, jobs, the cross-process operation lock
# ============================================================================

class Store:
    """addons.json (state, 0600) + addons-secrets.json (0600).  Secret values never appear in the state file."""

    def __init__(self, env: AddonEnv):
        self.env = env
        self.lock = threading.RLock()
        self.path = env.state_path("addons.json")
        self.secret_path = env.state_path("addons-secrets.json")

    def _load(self, path: str, default: dict) -> dict:
        try:
            with open(path, encoding="utf-8") as fh:
                doc = json.load(fh)
            return doc if isinstance(doc, dict) else default
        except (OSError, ValueError):
            return default

    def load(self) -> dict:
        doc = self._load(self.path, {})
        doc.setdefault("schema", 1)
        if not isinstance(doc.get("addons"), dict):
            doc["addons"] = {}
        return doc

    def save(self, doc: dict) -> None:
        _write_atomic(self.path, json.dumps(doc, indent=1, sort_keys=True) + "\n", 0o600)

    def get(self, aid: str) -> dict | None:
        with self.lock:
            v = self.load()["addons"].get(aid)
            return v if isinstance(v, dict) else None

    def update(self, aid: str, **fields: Any) -> dict:
        with self.lock:
            doc = self.load()
            rec = doc["addons"].setdefault(aid, {})
            rec.update(fields)
            self.save(doc)
            return rec

    def delete(self, aid: str) -> None:
        with self.lock:
            doc = self.load()
            doc["addons"].pop(aid, None)
            self.save(doc)
            sec = self._load(self.secret_path, {})
            if aid in sec:
                sec.pop(aid)
                _write_atomic(self.secret_path, json.dumps(sec, sort_keys=True) + "\n", 0o600)

    def secrets(self, aid: str) -> dict[str, str]:
        v = self._load(self.secret_path, {}).get(aid, {})
        return {k: x for k, x in v.items() if isinstance(k, str) and isinstance(x, str)} if isinstance(v, dict) else {}

    def set_secrets(self, aid: str, values: dict[str, str]) -> None:
        with self.lock:
            sec = self._load(self.secret_path, {})
            sec[aid] = values
            _write_atomic(self.secret_path, json.dumps(sec, sort_keys=True) + "\n", 0o600)

    def all_secret_values(self) -> list[str]:
        out = []
        for v in self._load(self.secret_path, {}).values():
            if isinstance(v, dict):
                out += [x for x in v.values() if isinstance(x, str) and len(x) >= 4]
        return out


class FlockJobLock:
    """Default cross-process job lock: flock on a file in the state directory.  The web module replaces it with an adapter around its own
    ``JobLock`` so that add-on jobs and management-plane updates exclude each other.  Interface: acquire(kind, label, job_id) -> token|None,
    release(token), holder() -> dict|None."""

    def __init__(self, path: str):
        self.path = path

    def acquire(self, kind: str, label: str, job_id: str = "") -> int | None:
        os.makedirs(os.path.dirname(self.path), mode=0o700, exist_ok=True)
        fd = os.open(self.path, os.O_RDWR | os.O_CREAT | getattr(os, "O_CLOEXEC", 0), 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            os.close(fd)
            return None
        return fd

    def release(self, token: int | None) -> None:
        if token is not None:
            try:
                fcntl.flock(token, fcntl.LOCK_UN)
            finally:
                os.close(token)

    def holder(self) -> dict | None:
        t = self.acquire("probe", "")
        if t is None:
            return {"kind": "addon"}
        self.release(t)
        return None


def lock_busy(env: AddonEnv) -> bool:
    """True while some process runs an add-on job (or, with the web module's adapter, any update/add-on job)."""
    return env.job_lock.holder() is not None


class Job:
    MAX_LOG = 300

    def __init__(self, jid: str, kind: str, addon: str, user: str | None, steps: list[str], now: float):
        self.id, self.kind, self.addon, self.user = jid, kind, addon, user
        self.steps = steps
        self.index = 0
        self.state = "running"
        self.message = ""
        self.ok: bool | None = None
        self.error = ""
        self.started, self.finished = now, None
        self.log: collections.deque = collections.deque(maxlen=self.MAX_LOG)
        self.warnings: list[str] = []
        self._lock = threading.Lock()
        self.scrub: Callable[[str], str] = lambda s: s
        self.sink: Callable[[str], None] | None = None        # the CLI prints progress through this

    def step(self, name: str, message: str = "") -> None:
        with self._lock:
            if name in self.steps:
                self.index = self.steps.index(name)
            self.message = message or name
            self.log.append("== " + (message or name))
        if self.sink:
            self.sink("== " + (message or name))

    def line(self, text: str) -> None:
        t = self.scrub(text.rstrip())[:400]
        if t:
            with self._lock:
                self.log.append(t)
            if self.sink:
                self.sink(t)

    def view(self, full: bool = True) -> dict:
        with self._lock:
            total = max(1, len(self.steps))
            done = total if self.state in ("done", "failed") else self.index
            d = {"id": self.id, "kind": self.kind, "addon": self.addon, "state": self.state, "step": self.steps[self.index] if self.steps and self.index < len(self.steps) else "",
                 "steps": list(self.steps), "progress": round(100 * done / total), "message": self.message, "ok": self.ok, "error": self.error,
                 "started": self.started, "finished": self.finished, "warnings": list(self.warnings)}
            if full:
                d["log"] = list(self.log)[-120:]
            return d


class JobStore:
    KEEP = 40

    def __init__(self, env: AddonEnv):
        self.env = env
        self.jobs: collections.OrderedDict[str, Job] = collections.OrderedDict()
        self.lock = threading.Lock()
        self.path = env.state_path("addons-jobs.json")

    def new(self, kind: str, addon: str, user: str | None, steps: list[str]) -> Job:
        with self.lock:
            j = Job("j" + _secrets.token_hex(6), kind, addon, user, steps, self.env.now())
            self.jobs[j.id] = j
            while len(self.jobs) > self.KEEP:
                self.jobs.popitem(last=False)
            return j

    def get(self, jid: str) -> Job | None:
        with self.lock:
            return self.jobs.get(jid)

    def active(self) -> Job | None:
        with self.lock:
            return next((j for j in reversed(self.jobs.values()) if j.state == "running"), None)

    def latest_for(self, addon: str) -> Job | None:
        with self.lock:
            return next((j for j in reversed(self.jobs.values()) if j.addon == addon), None)

    def views(self) -> list[dict]:
        with self.lock:
            return [j.view(full=False) for j in reversed(self.jobs.values())]

    def persist(self) -> None:
        """Finished jobs survive a restart of the web server (the log of the last ones is useful after a failed update)."""
        try:
            with self.lock:
                done = [j.view() for j in self.jobs.values() if j.state != "running"][-self.KEEP:]
            _write_atomic(self.path, json.dumps({"schema": 1, "jobs": done}) + "\n", 0o600)
        except OSError:
            pass

    def restore(self) -> None:
        try:
            with open(self.path, encoding="utf-8") as fh:
                doc = json.load(fh)
        except (OSError, ValueError):
            return
        for v in doc.get("jobs", []) if isinstance(doc, dict) else []:
            if isinstance(v, dict) and isinstance(v.get("id"), str) and re.fullmatch(r"j[0-9a-f]{12}", v["id"]) and v["id"] not in self.jobs:
                j = Job(v["id"], str(v.get("kind", ""))[:20], str(v.get("addon", ""))[:40], None, [str(s)[:40] for s in v.get("steps", [])][:12], float(v.get("started") or 0))
                j.state = v.get("state") if v.get("state") in ("done", "failed") else "failed"
                j.ok, j.error, j.message, j.finished = v.get("ok"), str(v.get("error", ""))[:400], str(v.get("message", ""))[:400], v.get("finished")
                j.log.extend(str(x)[:400] for x in v.get("log", [])[-120:])
                self.jobs[j.id] = j


# ============================================================================
# 6. System handler: Docker itself (Debian packages), idempotent
# ============================================================================

#: Debian 13 (trixie) archive, checked 2026-11-03: docker.io 26.1.5, docker-cli 26.1.5, docker-compose 2.26.1 (Go; installs the
#: `docker compose` plugin into /usr/libexec/docker/cli-plugins), docker-buildx 0.13.1.  Debian 13 has no docker-compose-v2 package.
DOCKER_PACKAGES = ("docker.io", "docker-cli", "docker-compose")
DOCKER_DAEMON_JSON = '{\n  "log-driver": "json-file",\n  "log-opts": {"max-size": "10m", "max-file": "3"}\n}\n'
APT = ["env", "DEBIAN_FRONTEND=noninteractive", "NEEDRESTART_MODE=a", "apt-get", "-y", "-o", "Dpkg::Options::=--force-confold", "-o", "Dpkg::Options::=--force-confdef"]


class DockerHandler:
    """kind=system, handler=docker: install Docker Engine + the compose plugin from Debian, enable the service, wire the group."""
    steps = {"install": ["check", "packages", "service", "group", "verify"], "update": ["packages", "verify"], "start": ["start"], "stop": ["stop"],
             "restart": ["restart"], "remove": []}

    def __init__(self, mgr: "Manager"):
        self.mgr = mgr
        self.env = mgr.env

    def _installed(self, pkg: str) -> bool:
        r = self.env.run(["dpkg-query", "-W", "-f=${db:Status-Abbrev}", pkg], timeout=20)
        return r.ok and r.out.strip().startswith("ii")

    def _candidate(self, pkg: str) -> bool:
        r = self.env.run(["apt-cache", "policy", pkg], timeout=30)
        m = re.search(r"Candidate:\s*(\S+)", r.out)
        return bool(r.ok and m and m.group(1) != "(none)")

    def supports(self, action: str) -> bool:
        return action in self.steps and action != "remove"

    def run(self, action: str, job: Job, ident: dict) -> None:
        getattr(self, "_" + action)(job, ident)

    # -- steps ----------------------------------------------------------------
    def _apt(self, job: Job, args: list[str], timeout: int = 1800) -> Result:
        return self.env.stream(APT + args, job.line, timeout)

    def _packages(self, job: Job, upgrade: bool) -> None:
        missing = [p for p in DOCKER_PACKAGES if not self._installed(p)]
        if not missing and not upgrade:
            job.line("Docker packages are already installed.")
            return
        job.step("packages", "Installing Docker packages" if missing else "Upgrading Docker packages")
        u = self._apt(job, ["update"], 600)
        if not u.ok:
            job.warnings.append("The package lists could not be refreshed (%s)." % (u.err or u.out).strip().splitlines()[-1][:120] if (u.err or u.out).strip() else "The package lists could not be refreshed.")
        absent = [p for p in (missing or DOCKER_PACKAGES) if not self._candidate(p)]
        if absent:
            raise AddonError("%s %s not available from the configured package sources. Docker needs the Debian mirror and Internet access." %
                             (", ".join(absent), "is" if len(absent) == 1 else "are"))
        if missing:
            r = self._apt(job, ["install", "--no-install-recommends", "ca-certificates"] + list(DOCKER_PACKAGES))
        else:
            r = self._apt(job, ["install", "--only-upgrade", "--no-install-recommends"] + list(DOCKER_PACKAGES))
        if not r.ok:
            raise AddonError("apt-get could not install Docker: %s" % ((r.err or r.out).strip().splitlines() or ["see the job log"])[-1][:200])

    def _install(self, job: Job, ident: dict) -> None:
        job.step("check", "Checking the system")
        r = self.env.run(["dpkg-query", "-W", "-f=${Version}", "docker.io"], timeout=20)
        job.line("docker.io: %s" % (r.out.strip() if r.ok and r.out.strip() else "not installed"))
        job.step("packages")
        self._packages(job, upgrade=False)
        job.step("service", "Starting the Docker service")
        conf = self.env.p("/etc/docker/daemon.json")
        created = False
        if not os.path.lexists(conf):
            os.makedirs(os.path.dirname(conf), exist_ok=True)
            _write_atomic(conf, DOCKER_DAEMON_JSON, 0o644)
            created = True
        e = self.env.run(["systemctl", "enable", "--now", "docker.service"], timeout=120)
        if not e.ok:
            raise AddonError("the Docker service could not be started: %s" % ((e.err or e.out).strip().splitlines() or ["systemctl failed"])[-1][:200])
        if created:
            self.env.run(["systemctl", "restart", "docker.service"], timeout=120)
        job.step("group", "Adding administrators to the docker group")
        if not ident["admins"]:
            job.warnings.append("No administrator account was found. Add one with: usermod -aG docker <user>")
        for u in ident["admins"]:
            if not USER_RE.fullmatch(u):
                continue
            g = self.env.run(["id", "-nG", u], timeout=10)
            if "docker" in g.out.split():
                job.line("%s is already in the docker group." % u)
                continue
            m = self.env.run(["usermod", "-aG", "docker", u], timeout=30)
            job.line("%s added to the docker group." % u if m.ok else "could not add %s: %s" % (u, (m.err or m.out).strip()[:100]))
        self._verify(job, ident)

    def _update(self, job: Job, ident: dict) -> None:
        job.step("packages")
        self._packages(job, upgrade=True)
        self._verify(job, ident)

    def _verify(self, job: Job, ident: dict) -> None:
        job.step("verify", "Checking Docker")
        ok, ver = self.mgr.docker.available()
        if not ok:
            raise AddonError("Docker is installed but does not answer: %s" % ver)
        ok2, cv = self.mgr.docker.compose_available()
        if not ok2:
            raise AddonError("Docker answers, but the compose plugin is missing (package docker-compose)")
        job.line("Docker %s, compose %s" % (ver, cv))

    def _start(self, job: Job, ident: dict) -> None:
        r = self.env.run(["systemctl", "start", "docker.service"], timeout=120)
        if not r.ok:
            raise AddonError("could not start Docker: %s" % (r.err or r.out).strip()[:200])

    def _stop(self, job: Job, ident: dict) -> None:
        r = self.env.run(["systemctl", "stop", "docker.service", "docker.socket"], timeout=300)
        if not r.ok:
            raise AddonError("could not stop Docker: %s" % (r.err or r.out).strip()[:200])

    def _restart(self, job: Job, ident: dict) -> None:
        r = self.env.run(["systemctl", "restart", "docker.service"], timeout=300)
        if not r.ok:
            raise AddonError("could not restart Docker: %s" % (r.err or r.out).strip()[:200])

    def status(self) -> dict:
        r = self.env.run(["systemctl", "is-active", "docker.service"], timeout=10)
        active = r.out.strip() == "active"
        return {"status": "running" if active else "stopped", "containers": []}

    def logs(self, tail: int) -> str:
        r = self.env.run(["journalctl", "-u", "docker.service", "-n", str(int(tail)), "--no-pager"], timeout=30)
        return r.out


SYSTEM_HANDLERS: dict[str, type] = {"docker": DockerHandler}


# ============================================================================
# 7. The manager: install / start / stop / restart / update / remove / status / logs as jobs
# ============================================================================

class Busy(AddonError):
    """Another job is running (HTTP 409)."""


class NotFound(AddonError):
    """No such add-on (HTTP 404)."""


ACTIVE_STATUS = {"install": "installing", "update": "updating", "remove": "removing"}


def validate_settings(a: Addon, given: Any, prev: dict | None = None, prev_secrets: dict | None = None) -> tuple[dict, dict, dict]:
    """-> (all values incl. secrets for rendering, secret values, non-secret values to store).  Raises AddonError."""
    if given is None:
        given = {}
    if not isinstance(given, dict):
        raise AddonError("settings must be an object")
    keys = {s["key"] for s in a.settings}
    unknown = [k for k in given if k not in keys]
    if unknown:
        raise AddonError("unknown setting '%s'" % str(unknown[0])[:40])
    prev, prev_secrets = prev or {}, prev_secrets or {}
    values: dict = {}
    for s in a.settings:
        k = s["key"]
        has = k in given and given[k] is not None and not (s["type"] == "secret" and given[k] == "")
        if has:
            v = given[k]
        elif s["type"] == "secret" and k in prev_secrets:
            v = prev_secrets[k]
        elif k in prev:
            v = prev[k]
        elif "default" in s:
            v = s["default"]
        elif s["type"] == "choice":
            v = s["choices"][0]["value"]
        else:
            v = False if s["type"] == "bool" else ""
        v = coerce_setting(s, v, a.value_rules.get(k, "loose"))
        if s["required"] and v in ("", None):
            raise AddonError("%s is required" % s["label"])
        values[k] = v
    secret_vals = {s["key"]: values[s["key"]] for s in a.settings if s["type"] == "secret"}
    plain = {k: v for k, v in values.items() if k not in secret_vals}
    return values, secret_vals, plain


class Manager:
    CATALOG_TTL = 5.0

    def __init__(self, env: AddonEnv, docker: Docker | None = None, catalog: Catalog | None = None):
        self.env = env
        self.docker = docker or Docker(env)
        self.store = Store(env)
        self.jobs = JobStore(env)
        self.jobs.restore()
        self._catalog = catalog
        self._catalog_at = env.now() if catalog else 0.0
        self.handlers = {name: cls(self) for name, cls in SYSTEM_HANDLERS.items()}
        self.recovered = False

    # -- catalog ---------------------------------------------------------------
    def catalog(self) -> Catalog:
        if self._catalog is None or self.env.now() - self._catalog_at > self.CATALOG_TTL:
            self._catalog = load_default_catalog(self.env)
            self._catalog_at = self.env.now()
        return self._catalog

    def addon(self, aid: str) -> Addon:
        try:
            return self.catalog().get(aid)
        except AddonError as e:
            raise NotFound(str(e))

    # -- helpers ---------------------------------------------------------------
    def compose_file(self, a: Addon) -> str:
        return self.env.p(a.data_dir + "/compose.yml")

    def scrubber(self) -> Callable[[str], str]:
        vals = sorted(self.store.all_secret_values(), key=len, reverse=True)

        def scrub(t: str) -> str:
            for v in vals:
                if v in t:
                    t = t.replace(v, "***")
            return t
        return scrub

    def recover(self) -> None:
        """Operations that were running when the process died are marked 'needs attention' (only when no job runs anywhere)."""
        if self.recovered:
            return
        self.recovered = True
        tok = self.env.job_lock.acquire("addon", "recover")
        if tok is None:
            return
        try:
            doc = self.store.load()
            for aid, rec in doc["addons"].items():
                if isinstance(rec, dict) and rec.get("status") in ("installing", "updating", "removing"):
                    self.store.update(aid, status="needs-attention", error="The last operation was interrupted. Run it again.")
        finally:
            self.env.job_lock.release(tok)

    def installed(self, aid: str) -> bool:
        return self.store.get(aid) is not None

    def usable(self, aid: str) -> bool:
        """Installed and not half-done: what ``requires`` means."""
        rec = self.store.get(aid)
        return rec is not None and rec.get("status") == "installed"

    # -- views -----------------------------------------------------------------
    def live_status(self, a: Addon) -> dict:
        rec = self.store.get(a.id)
        if rec is None:
            return {"status": "not-installed", "containers": [], "detail": ""}
        act = self.jobs.active()
        if act and act.addon == a.id and act.kind in ACTIVE_STATUS:
            return {"status": ACTIVE_STATUS[act.kind], "containers": [], "detail": act.message}
        err = rec.get("error", "")
        if rec.get("status") in ("installing", "updating", "removing"):
            return {"status": "needs-attention", "containers": [], "detail": err}
        if a.kind == "system":
            st = self.handlers[a.handler].status()
            st["detail"] = err if rec.get("status") == "needs-attention" else ""
            if rec.get("status") == "needs-attention" and st["status"] != "running":
                st["status"] = "needs-attention"
            return st
        cs = self.docker.ps("synofurb-" + a.id, self.compose_file(a)) if os.path.isfile(self.compose_file(a)) else []
        if not cs:
            return {"status": "needs-attention" if rec.get("status") == "needs-attention" else "stopped", "containers": [], "detail": err}
        states = [c["state"] for c in cs]
        if all(s == "running" for s in states):
            bad = [c for c in cs if c["health"] == "unhealthy"]
            if bad:
                return {"status": "unhealthy", "containers": cs, "detail": "%s reports unhealthy" % bad[0]["name"]}
            if a.healthcheck and not self.probe(a, cs):
                started = float(rec.get("started") or 0)
                window = a.healthcheck["timeout"]
                return {"status": "starting" if self.env.now() - started < window else "unhealthy", "containers": cs, "detail": "the health check does not answer yet"}
            return {"status": "running", "containers": cs, "detail": ""}
        if any(s == "restarting" or (s == "exited" and c["exit_code"] != 0) for s, c in zip(states, cs)):
            return {"status": "error", "containers": cs, "detail": "a container keeps stopping; see the logs"}
        if any(s == "running" for s in states):
            return {"status": "error", "containers": cs, "detail": "some containers are not running"}
        return {"status": "stopped", "containers": cs, "detail": ""}

    def probe(self, a: Addon, containers: list[dict] | None = None) -> bool:
        hc = a.healthcheck
        if not hc or hc["type"] == "container":
            cs = containers if containers is not None else self.docker.ps("synofurb-" + a.id, self.compose_file(a))
            return bool(cs) and all(c["state"] == "running" and c["health"] != "unhealthy" for c in cs)
        if hc["type"] == "http":
            st = self.env.http_probe(hc["url"], 4)
            return st is not None and st < 500
        return bool(self.env.tcp_probe(hc["port"], 3))

    def card(self, a: Addon, live: bool = True) -> dict:
        rec = self.store.get(a.id)
        d = a.public()
        for k in ("settings", "storage", "devices", "post_install", "changes", "data_dir", "ports"):
            d.pop(k, None)
        d["installed"] = rec is not None
        if rec is not None:
            st = self.live_status(a) if live else {"status": rec.get("status", ""), "detail": rec.get("error", "")}
            d.update(status=st["status"], detail=st.get("detail", ""), installed_version=rec.get("version", ""), update_available=rec.get("version") != a.version,
                     link=self.web_link(a))
        else:
            d.update(status="not-installed", detail="", installed_version="", update_available=False, link=None)
        d["dependents"] = self.catalog().dependents(a.id)
        return d

    def web_link(self, a: Addon) -> dict | None:
        if not a.web:
            return None
        hidden = any(p["host"] == a.web["port"] and p.get("bind") == "127.0.0.1" for p in a.ports)
        return None if hidden else {"scheme": a.web["scheme"], "port": a.web["port"], "path": a.web["path"], "hint": a.web["hostname_hint"]}

    def list(self, live: bool = True) -> dict:
        cat = self.catalog()
        act = self.jobs.active()
        return {"addons": [self.card(a, live) for a in cat.order()], "problems": cat.problems, "job": act.view(False) if act else None}

    def show(self, aid: str) -> dict:
        a = self.addon(aid)
        d = self.card(a)
        full = a.public()
        d.update({k: full[k] for k in ("settings", "storage", "devices", "post_install", "changes", "ports", "data_dir")})
        rec = self.store.get(aid) or {}
        saved = rec.get("settings", {})
        sec = set(rec.get("secret_keys", []))
        d["values"] = {s["key"]: ({"set": s["key"] in sec} if s["type"] == "secret" else saved.get(s["key"], s.get("default"))) for s in a.settings}
        d["storage_values"] = rec.get("storage") or {}
        d["storage_report"] = storage_report(self.env, a, rec.get("storage") or {})
        d["device_states"] = device_states(self.env, a)
        d["device_values"] = rec.get("devices") or {}
        d["warnings"] = self.port_warnings(a) if not rec else []
        d["requires_state"] = {r: self.installed(r) for r in a.requires}
        d["job"] = (lambda j: j.view(False) if j else None)(self.jobs.latest_for(aid))
        return d

    def port_warnings(self, a: Addon) -> list[str]:
        out = []
        taken = {(p["host"], p["proto"]) for i in self.store.load()["addons"] if i in self.catalog().addons and i != a.id for p in self.catalog().addons[i].ports}
        for p in a.ports:
            if (p["host"], p["proto"]) in taken:
                out.append("port %d/%s is used by another add-on" % (p["host"], p["proto"]))
            elif p["proto"] == "tcp" and self.env.port_in_use(p["host"]):
                out.append("port %d is already in use on this unit" % p["host"])
        return out

    def status(self, aid: str) -> dict:
        a = self.addon(aid)
        st = self.live_status(a)
        rec = self.store.get(aid) or {}
        return {"id": aid, "installed": bool(rec), "version": rec.get("version", ""), **st}

    def logs(self, aid: str, tail: int = 200) -> str:
        a = self.addon(aid)
        if not self.installed(aid):
            raise AddonError("%s is not installed" % a.name)
        tail = max(10, min(int(tail), 2000))
        if a.kind == "system":
            text = self.handlers[a.handler].logs(tail)
        else:
            f = self.compose_file(a)
            if not os.path.isfile(f):
                raise AddonError("%s has no compose file yet" % a.name)
            r = self.docker.logs("synofurb-" + aid, f, tail)
            text = r.out if r.ok else (r.err or r.out)
        return self.scrubber()(text)[-200000:]

    # -- job plumbing ------------------------------------------------------------
    def _start_job(self, kind: str, a: Addon, steps: list[str], work: Callable[[Job], None], user: str | None, ip: str, audit_args: dict,
                   background: bool, wait_lock: float = 0.0, sink: Callable[[str], None] | None = None) -> Job:
        lock = self.env.job_lock
        deadline = self.env.now() + wait_lock
        token = lock.acquire("addon", "%s %s" % (kind, a.id))
        while token is None and self.env.now() < deadline:
            self.env.sleep(1.0)
            token = lock.acquire("addon", "%s %s" % (kind, a.id))
        if token is None:
            h = lock.holder() or {}
            act = self.jobs.active()
            what = "%s %s" % (act.kind, act.addon) if act else str(h.get("kind") or "another job")
            raise Busy("Another job is running (%s). Wait until it ends." % what)
        job = self.jobs.new(kind, a.id, user, steps)
        job.scrub = self.scrubber()
        job.sink = sink                                # type: ignore[attr-defined]
        self.env.audit(user, ip, "addons.job.start", dict(audit_args, job=job.id, addon=a.id, kind=kind), "ok")

        def run() -> None:
            try:
                work(job)
                job.ok, job.state = True, "done"
                if not job.message or job.message.startswith("=="):
                    job.message = "Done"
            except AddonError as e:
                job.ok, job.state, job.error = False, "failed", job.scrub(str(e))
            except Exception:                                                  # noqa: BLE001 - never leak internals to the client
                traceback.print_exc(file=sys.stderr)
                job.ok, job.state, job.error = False, "failed", "internal error; see the job log and the journal"
            finally:
                job.finished = self.env.now()
                job.log.append(("== done" if job.ok else "== failed: " + job.error))
                self.jobs.persist()
                lock.release(token)
                self.env.audit(user, ip, "addons.job." + kind, dict(audit_args, job=job.id, addon=a.id), "ok" if job.ok else "error: " + job.error)
        if background:
            threading.Thread(target=run, name="addon-" + job.id, daemon=True).start()
        else:
            run()
        return job

    def _accepted(self, privs: list[str], body: dict) -> bool:
        acc = body.get("accept")
        return acc is True or (isinstance(acc, list) and set(privs) <= {str(x) for x in acc})

    def _need_requires(self, a: Addon) -> None:
        miss = [r for r in a.requires if not self.usable(r)]
        if miss:
            names = ", ".join(self.catalog().addons[m].name for m in miss)
            raise AddonError("%s needs %s to be installed first." % (a.name, names))

    # -- install / update ---------------------------------------------------------
    def install(self, aid: str, body: dict, user: str | None = None, ip: str = "-", background: bool = True, wait_lock: float = 0.0, sink: Callable[[str], None] | None = None) -> Job:
        a = self.addon(aid)
        rec = self.store.get(aid)
        if rec is not None and rec.get("status") != "needs-attention":
            raise AddonError("%s is already installed." % a.name)
        self._need_requires(a)
        prev = (rec or {}).get("settings") if rec else None
        values, secret_vals, plain = validate_settings(a, body.get("settings"), prev, self.store.secrets(aid) if rec else None)
        ident = self.env.identity()
        storage_in = body.get("storage") or {}
        if not isinstance(storage_in, dict) or any(k not in {s["key"] for s in a.storage} for k in storage_in):
            raise AddonError("unknown storage key")
        storage: dict[str, str] = {}
        for st in a.storage:
            p = valid_abs_path(storage_in.get(st["key"]) or (rec or {}).get("storage", {}).get(st["key"]) or st["path"], "%s folder" % st["label"])
            if st["bulk"] and not (p == DATA_VOLUME or p.startswith(DATA_VOLUME + "/")):
                raise AddonError("%s must be below %s, the redundant data volume." % (st["label"], DATA_VOLUME))
            storage[st["key"]] = p
        rep = storage_report(self.env, a, storage)
        bad = [r["error"] for r in rep if r["error"]]
        if bad:
            raise AddonError(bad[0])
        chosen, dev_warn = choose_devices(self.env, a, body.get("devices") if isinstance(body.get("devices"), dict) else None)
        for s in a.settings:
            if s["type"] == "path" and values.get(s["key"]) and not os.path.isdir(self.env.p(values[s["key"]])):
                raise AddonError("%s: the folder %s does not exist. Create it first." % (s["label"], values[s["key"]]))
        privs = effective_privileges(a, values)
        if privs and not self._accepted(privs, body):
            raise AddonError("%s needs these privileges: %s. Confirm them to continue." % (a.name, ", ".join(privs)))
        warns = dev_warn + [r["warning"] for r in rep if r["warning"]]
        steps = ["check", "render", "pull", "start", "health"] if a.kind == "compose" else list(self.handlers[a.handler].steps["install"])
        dev_map = {d["id"]: (d in chosen) for d in a.devices}
        audit_args = {"privileges": privs, "devices": [d["id"] for d in chosen], "storage": storage, "settings": sorted(plain), "accepted": True}

        def work(job: Job) -> None:
            job.warnings.extend(warns)
            self.store.update(aid, status="installing", error="", version=a.version, settings=plain, secret_keys=sorted(secret_vals), storage=storage, devices=dev_map,
                              privileges=privs, installed=(rec or {}).get("installed") or self.env.now(), updated=self.env.now())
            self.store.set_secrets(aid, secret_vals)
            job.scrub = self.scrubber()
            try:
                if a.kind == "system":
                    self.handlers[a.handler].run("install", job, ident)
                else:
                    self._install_compose(job, a, values, storage, chosen, ident)
            except BaseException as e:
                self.store.update(aid, status="needs-attention", error=job.scrub(str(e))[:400] if isinstance(e, AddonError) else "internal error")
                raise
            self.store.update(aid, status="installed", error="", started=self.env.now(), updated=self.env.now())
            if a.post_install:
                job.line(a.post_install)
        return self._start_job("install", a, steps, work, user, ip, audit_args, background, wait_lock, sink)

    def _check_docker(self) -> None:
        ok, why = self.docker.available()
        if not ok:
            raise AddonError("Docker is not running (%s). Install or start the Docker add-on first." % why)
        ok, why = self.docker.compose_available()
        if not ok:
            raise AddonError(why + ". Install the Docker add-on again.")

    def _prepare_dirs(self, a: Addon, storage: dict, ident: dict) -> None:
        os.makedirs(self.env.p(a.data_dir), mode=0o755, exist_ok=True)
        for st in a.storage:
            d = self.env.p(storage[st["key"]])
            os.makedirs(d, mode=0o755, exist_ok=True)
            if self.env.chown:
                try:
                    os.chown(d, ident["puid"], ident["pgid"])
                except OSError:
                    pass

    def _install_compose(self, job: Job, a: Addon, values: dict, storage: dict, chosen: list[dict], ident: dict) -> None:
        job.step("check", "Checking Docker")
        self._check_docker()
        job.step("render", "Writing the configuration")
        self._render_and_validate(job, a, values, storage, chosen, ident)
        project, f = "synofurb-" + a.id, self.compose_file(a)
        job.step("pull", "Downloading images (needs Internet)")
        r = self.docker.pull(project, f, job.line)
        if not r.ok:
            raise AddonError("The images could not be downloaded: %s. Check the Internet connection and run Update." % self._last(r))
        job.step("start", "Starting")
        r = self.docker.up(project, f, job.line)
        if not r.ok:
            raise AddonError("The containers did not start: %s" % self._last(r))
        self.store.update(a.id, started=self.env.now())
        job.step("health", "Waiting for %s to answer" % a.name)
        if not self._wait_healthy(a, job):
            job.warnings.append("%s started but its health check does not answer yet. Look at the logs." % a.name)

    @staticmethod
    def _last(r: Result) -> str:
        lines = [x for x in (r.err or r.out).strip().splitlines() if x.strip()]
        return (lines[-1] if lines else "no message")[:240]

    def _render_and_validate(self, job: Job, a: Addon, values: dict, storage: dict, chosen: list[dict], ident: dict) -> None:
        self._prepare_dirs(a, storage, ident)
        compose, extra = render_addon(a, values, storage, chosen, ident)
        write_rendered(self.env, a, compose, ident, "compose.next.yml")
        nxt = self.env.p(a.data_dir + "/compose.next.yml")
        r = self.docker.config("synofurb-" + a.id, nxt)
        if not r.ok:
            os.unlink(nxt)
            raise AddonError("The generated compose file is not valid: %s" % self._last(r))
        os.replace(nxt, self.compose_file(a))
        for x in extra:
            if write_rendered(self.env, a, x, ident):
                job.line("wrote %s" % x.dest)
            else:
                job.line("kept your %s" % x.dest)

    def _wait_healthy(self, a: Addon, job: Job) -> bool:
        t = (a.healthcheck or {}).get("timeout", 60)
        deadline = self.env.now() + t
        while True:
            if self.probe(a):
                return True
            if self.env.now() >= deadline:
                return False
            self.env.sleep(3)

    def update(self, aid: str, body: dict | None = None, user: str | None = None, ip: str = "-", background: bool = True, wait_lock: float = 0.0, sink: Callable[[str], None] | None = None) -> Job:
        a = self.addon(aid)
        rec = self.store.get(aid)
        if rec is None:
            raise AddonError("%s is not installed." % a.name)
        self._need_requires(a)
        ident = self.env.identity()
        sec = self.store.secrets(aid)
        values, secret_vals, plain = validate_settings(a, {}, rec.get("settings"), sec)
        storage = {st["key"]: (rec.get("storage") or {}).get(st["key"], st["path"]) for st in a.storage}
        devs = rec.get("devices") or {}
        chosen, warns = choose_devices(self.env, a, {k: v for k, v in devs.items() if isinstance(v, bool) and k in {d["id"] for d in a.devices}} if devs else None)
        steps = ["check", "render", "pull", "start", "health"] if a.kind == "compose" else list(self.handlers[a.handler].steps["update"])

        def work(job: Job) -> None:
            job.warnings.extend(warns)
            self.store.update(aid, status="updating", error="")
            try:
                if a.kind == "system":
                    self.handlers[a.handler].run("update", job, ident)
                else:
                    self._install_compose(job, a, values, storage, chosen, ident)
            except BaseException as e:
                self.store.update(aid, status="needs-attention", error=job.scrub(str(e))[:400] if isinstance(e, AddonError) else "internal error")
                raise
            self.store.update(aid, status="installed", error="", version=a.version, updated=self.env.now(), started=self.env.now())
        return self._start_job("update", a, steps, work, user, ip, {"version": a.version}, background, wait_lock, sink)

    # -- start / stop / restart ----------------------------------------------------
    def control(self, aid: str, action: str, user: str | None = None, ip: str = "-", background: bool = True, sink: Callable[[str], None] | None = None) -> Job:
        a = self.addon(aid)
        if action not in ("start", "stop", "restart"):
            raise AddonError("unknown action")
        rec = self.store.get(aid)
        if rec is None:
            raise AddonError("%s is not installed." % a.name)
        if action in ("start", "restart"):
            self._need_requires(a)
        if action == "stop" and a.kind == "system":
            running = [d for d in self.catalog().dependents(aid) if self.installed(d)]
            if running:
                raise AddonError("%s is used by %s. Remove or stop those first." % (a.name, ", ".join(running)))
        ident = self.env.identity()

        def work(job: Job) -> None:
            job.step(action, {"start": "Starting", "stop": "Stopping", "restart": "Restarting"}[action])
            if a.kind == "system":
                self.handlers[a.handler].run(action, job, ident)
                return
            self._check_docker()
            f = self.compose_file(a)
            if not os.path.isfile(f):
                raise AddonError("%s has no compose file. Install it again." % a.name)
            project = "synofurb-" + aid
            r = {"start": lambda: self.docker.up(project, f, job.line), "stop": lambda: self.docker.stop(project, f), "restart": lambda: self.docker.restart(project, f)}[action]()
            if not r.ok:
                raise AddonError("%s failed: %s" % (action, self._last(r)))
            if action != "stop":
                self.store.update(aid, started=self.env.now())
        return self._start_job(action, a, [action], work, user, ip, {}, background, 0.0, sink)

    # -- remove -----------------------------------------------------------------------
    def remove(self, aid: str, delete_data: bool = False, user: str | None = None, ip: str = "-", background: bool = True, sink: Callable[[str], None] | None = None) -> Job:
        a = self.addon(aid)
        rec = self.store.get(aid)
        if rec is None:
            raise AddonError("%s is not installed." % a.name)
        users = [d for d in self.catalog().dependents(aid) if self.installed(d)]
        if users:
            raise AddonError("%s is used by %s. Remove those first." % (a.name, ", ".join(users)))
        if a.kind == "system":
            raise AddonError("%s was installed with apt and stays on this unit. Remove it from the add-ons list with: apt-get remove docker.io docker-cli docker-compose" % a.name)
        storage = dict(rec.get("storage") or {})

        def work(job: Job) -> None:
            self.store.update(aid, status="removing")
            f = self.compose_file(a)
            if os.path.isfile(f):
                job.step("stop", "Stopping and removing containers")
                r = self.docker.down("synofurb-" + aid, f, volumes=delete_data)
                if not r.ok:
                    self.store.update(aid, status="needs-attention", error="could not remove the containers: " + self._last(r))
                    raise AddonError("The containers could not be removed: %s" % self._last(r))
            job.step("files", "Deleting data" if delete_data else "Removing the compose file")
            if delete_data:
                own = (a.data_dir, "%s/addons/%s" % (DATA_VOLUME, a.id))
                for p in [a.data_dir] + list(storage.values()):
                    if any(p == o or p.startswith(o + "/") for o in own):
                        self._delete_tree(p, job)
                    else:
                        job.line("not deleting %s (it is outside this add-on's own folders)" % p)
            elif os.path.isfile(f):
                os.unlink(f)
            job.step("forget", "Forgetting the settings")
            self.store.delete(aid)
        return self._start_job("remove", a, ["stop", "files", "forget"], work, user, ip, {"delete_data": bool(delete_data)}, background, 0.0, sink)

    def _delete_tree(self, path: str, job: Job) -> None:
        """Delete an add-on's data folder.  Only plain folders three levels deep below /srv or /mnt; never a mount point or a link target."""
        try:
            valid_abs_path(path, "folder", depth=3)
        except AddonError:
            job.line("not deleting %s (not a safe location)" % path)
            return
        real = self.env.p(path)
        if os.path.islink(real):
            os.unlink(real)
            job.line("removed link %s" % path)
        elif os.path.ismount(real) or path in (DATA_VOLUME, DATA_ROOT):
            job.line("not deleting %s (it is a mount point)" % path)
        elif os.path.isdir(real):
            shutil.rmtree(real)
            job.line("deleted %s" % path)


def load_default_catalog(env: AddonEnv) -> Catalog:
    sources: list[tuple[str, dict]] = []
    d = os.path.join(env.script_dir, "addons")
    builtin: dict[str, dict] = {}
    if os.path.isdir(d):
        for aid in sorted(os.listdir(d)):
            p = os.path.join(d, aid)
            if os.path.isdir(p) and not os.path.islink(p):
                try:
                    builtin[aid] = read_addon_dir(p)
                except AddonError as e:
                    builtin[aid] = {"__error__": str(e)}
    else:
        builtin = read_pack(os.path.join(env.script_dir, PACK_NAME))
    user: dict[str, dict] = {}
    ud = env.p(env.user_dir)
    if os.path.isdir(ud):
        for aid in sorted(os.listdir(ud)):
            p = os.path.join(ud, aid)
            if aid != "pending" and os.path.isdir(p) and not os.path.islink(p):
                try:
                    user[aid] = read_addon_dir(p)
                except AddonError as e:
                    user[aid] = {"__error__": str(e)}
    cat = load_catalog([("builtin", {k: v for k, v in builtin.items() if "__error__" not in v}), ("user", {k: v for k, v in user.items() if "__error__" not in v})],
                       handlers=tuple(SYSTEM_HANDLERS))
    for src, found in (("builtin", builtin), ("user", user)):
        for aid, files in found.items():
            if "__error__" in files:
                cat.problems.append({"id": aid, "source": src, "error": files["__error__"]})
    return cat


# ============================================================================
# 8. apply-pending: the installer's first-boot hand-over
# ============================================================================

APPLY_RESULT = "addons-apply.json"


def _pending_dir(env: AddonEnv) -> str:
    return env.p(env.user_dir + "/pending")


def read_pending(env: AddonEnv) -> tuple[list[dict], list[dict]]:
    """-> (items, rejected).  Each pending file is {"schema":1,"id":...,"settings":{},"storage":{},"devices":{},"secrets":{}}.
    A file that carries secrets must not be readable by group or others."""
    d = _pending_dir(env)
    items: list[dict] = []
    rejected: list[dict] = []
    try:
        names = sorted(n for n in os.listdir(d) if n.endswith(".json"))
    except OSError:
        return [], []
    for n in names:
        p = os.path.join(d, n)
        aid = n[:-5]
        try:
            st = os.lstat(p)
            if not os.path.isfile(p) or os.path.islink(p) or st.st_size > 64 * 1024:
                raise AddonError("not a regular small file")
            with open(p, encoding="utf-8") as fh:
                doc = json.load(fh)
            if not isinstance(doc, dict) or doc.get("schema", 1) != 1 or doc.get("id", aid) != aid or not ID_RE.fullmatch(aid):
                raise AddonError("not a valid pending file for '%s'" % aid[:40])
            sec = doc.get("secrets") or {}
            if not isinstance(sec, dict) or not isinstance(doc.get("settings", {}), dict):
                raise AddonError("settings and secrets must be objects")
            if sec and (st.st_mode & 0o077):
                raise AddonError("it contains secrets but is readable by others; use mode 0600")
            settings = dict(doc.get("settings") or {})
            settings.update(sec)
            items.append({"id": aid, "file": p, "body": {"settings": settings, "storage": doc.get("storage") or {}, "devices": doc.get("devices") or {}, "accept": True}})
        except (OSError, ValueError, AddonError) as e:
            rejected.append({"id": aid, "file": p, "message": "pending file rejected: %s" % e})
    return items, rejected


def apply_pending(mgr: Manager, attempts: int = 3, delays: tuple = (10, 30, 90), emit: Callable[[str], None] | None = None) -> dict:
    """Install what the installer asked for, dependencies first.  Retries with backoff; never raises; writes addons-apply.json."""
    env = mgr.env
    say = emit or (lambda m: None)
    started = env.now()
    items, rejected = read_pending(env)
    result: list[dict] = []
    for r in rejected:
        result.append({"id": r["id"], "status": "rejected", "message": r["message"], "attempts": 0})
        _park(r["file"])
    try:
        cat = mgr.catalog()
        want = {it["id"]: it for it in items}
        ordered = [a.id for a in cat.order() if a.id in want] + [i for i in want if i not in cat.addons]
        failed: set = set()
        for aid in ordered:
            it = want[aid]
            rec: dict = {"id": aid, "status": "failed", "message": "", "attempts": 0}
            result.append(rec)
            if aid not in cat.addons:
                rec.update(status="rejected", message="unknown add-on '%s'" % aid[:40])
                _park(it["file"])
                continue
            a = cat.addons[aid]
            blocked = [r for r in a.requires if r in failed or (not mgr.installed(r) and r not in want)]
            if blocked:
                rec.update(status="skipped", message="needs %s first" % ", ".join(blocked))
                failed.add(aid)
                continue
            cur = mgr.store.get(aid)
            if cur and cur.get("status") not in ("needs-attention",):
                rec.update(status="installed", message="already installed")
                _unlink(it["file"])
                continue
            for n in range(1, attempts + 1):
                rec["attempts"] = n
                try:
                    say("installing %s (attempt %d of %d)" % (aid, n, attempts))
                    job = mgr.install(aid, it["body"], user="installer", ip="-", background=False, wait_lock=300)
                except Busy as e:
                    rec["message"] = str(e)
                except AddonError as e:                                 # the request itself is wrong: retrying cannot help
                    rec.update(status="rejected", message=str(e))
                    _park(it["file"])
                    break
                else:
                    if job.ok:
                        rec.update(status="installed", message=job.message or "installed")
                        _unlink(it["file"])
                        break
                    rec["message"] = job.error
                if n < attempts:
                    env.sleep(delays[min(n - 1, len(delays) - 1)])
            if rec["status"] in ("failed", "rejected"):
                failed.add(aid)
    except Exception as e:                                                  # noqa: BLE001 - the boot must never fail because of this
        traceback.print_exc(file=sys.stderr)
        result.append({"id": "-", "status": "failed", "message": "internal error: %s" % type(e).__name__, "attempts": 0})
    doc = {"schema": 1, "started": started, "finished": env.now(), "ok": all(r["status"] in ("installed",) for r in result), "items": result}
    try:
        _write_atomic(env.state_path(APPLY_RESULT), json.dumps(doc, indent=1) + "\n", 0o644)
    except OSError:
        pass
    return doc


def _unlink(p: str) -> None:
    try:
        os.unlink(p)
    except OSError:
        pass


def _park(p: str) -> None:
    """Move a rejected pending file aside so it is not tried at every boot (secrets inside stay 0600)."""
    d = os.path.join(os.path.dirname(p), "rejected")
    try:
        os.makedirs(d, mode=0o700, exist_ok=True)
        os.replace(p, os.path.join(d, os.path.basename(p)))
    except OSError:
        pass


# ============================================================================
# 9. REST routes (registered by synofurb_mgmt) and command line
# ============================================================================

def mask_body(a: Addon | None, body: dict) -> None:
    """The audit log records the request body: replace every secret (or unknown) setting value in place."""
    st = body.get("settings")
    if isinstance(st, dict):
        body["settings"] = {k: ("***" if a is None or (a.setting(k) or {"type": "secret"})["type"] == "secret" else v) for k, v in st.items()}


def register_routes(host: Any, get_manager: Callable[[Any], Manager]) -> None:
    """Attach the routes to the web module (``host`` provides Route, ROUTES, DYNAMIC_ROUTES, ApiError)."""
    ApiError, Route = host.ApiError, host.Route

    def guarded(fn: Callable[[Any, Manager], Any]) -> Callable[[Any], Any]:
        def inner(req: Any) -> Any:
            mgr = get_manager(req.app)
            try:
                return fn(req, mgr)
            except NotFound as e:
                raise ApiError(404, str(e))
            except Busy as e:
                raise ApiError(409, str(e))
            except AddonError as e:
                raise ApiError(400, str(e))
        return inner

    def seg(req: Any, i: int) -> str:
        return req.path.split("/")[i]

    def list_(req: Any, mgr: Manager) -> dict:
        mgr.recover()
        d = mgr.list()
        d["ok"] = True
        return d

    def show(req: Any, mgr: Manager) -> dict:
        return {"ok": True, "addon": mgr.show(seg(req, 3))}

    def logs(req: Any, mgr: Manager) -> dict:
        q = urllib.parse.parse_qs(getattr(req, "query", "") or "", max_num_fields=8)
        try:
            tail = int((q.get("tail") or ["200"])[0])
        except ValueError:
            raise AddonError("tail must be a number")
        return {"ok": True, "logs": mgr.logs(seg(req, 3), tail)}

    def job(req: Any, mgr: Manager) -> dict:
        j = mgr.jobs.get(seg(req, 3))
        if not j or not re.fullmatch(r"j[0-9a-f]{12}", seg(req, 3)):
            raise NotFound("no such job")
        return {"ok": True, "job": j.view()}

    def jobs(req: Any, mgr: Manager) -> dict:
        return {"ok": True, "jobs": mgr.jobs.views()}

    def act(req: Any, mgr: Manager) -> Any:
        aid, action = seg(req, 3), seg(req, 4)
        a = mgr.addon(aid)
        body = dict(req.body)                       # the real values; the audit log gets the masked copy below
        mask_body(a, req.body)
        req.audit_note = "%s %s" % (action, aid)
        if action == "install":
            j = mgr.install(aid, body, req.user, req.ip)
        elif action == "update":
            j = mgr.update(aid, body, req.user, req.ip)
        elif action == "remove":
            dd = body.get("delete_data") is True
            if dd and body.get("confirm") != aid:
                raise AddonError("to delete the data, type the add-on name %r to confirm" % aid)
            j = mgr.remove(aid, dd, req.user, req.ip)
        else:
            j = mgr.control(aid, action, req.user, req.ip)
        return 202, {"ok": True, "job": j.view()}

    host.ROUTES[("GET", "/api/addons")] = Route(guarded(list_), True, False)
    host.ROUTES[("GET", "/api/jobs")] = Route(guarded(jobs), True, False)
    dyn = host.DYNAMIC_ROUTES
    dyn.append(("GET", re.compile(r"/api/addons/[a-z0-9-]{1,32}"), Route(guarded(show), True, False)))
    dyn.append(("GET", re.compile(r"/api/addons/[a-z0-9-]{1,32}/logs"), Route(guarded(logs), True, False)))
    dyn.append(("GET", re.compile(r"/api/jobs/j[0-9a-f]{12}"), Route(guarded(job), True, False)))
    dyn.append(("POST", re.compile(r"/api/addons/[a-z0-9-]{1,32}/(?:install|start|stop|restart|update|remove)"), Route(guarded(act), True, True)))


def _fmt_card(c: dict) -> str:
    return "%-12s %-22s %-16s %s" % (c["id"], c["name"][:22], c["status"], c.get("installed_version") or c["version"])


def cli_main(argv: list[str], mgr: Manager, out: Callable[[str], None] = print, err: Callable[[str], None] | None = None, stdin: Any = None) -> int:
    """``synofurb_mgmt.py addons <command>``.  Exit codes: 0 ok, 1 the operation failed, 2 refused or bad input."""
    import argparse
    err = err or (lambda m: print(m, file=sys.stderr))
    ap = argparse.ArgumentParser(prog="synofurb_mgmt.py addons", add_help=True)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("list", "apply-pending"):
        p = sub.add_parser(name)
        p.add_argument("--json", action="store_true")
    for name in ("show", "status", "start", "stop", "restart", "update", "install", "remove", "logs"):
        p = sub.add_parser(name)
        p.add_argument("id")
        p.add_argument("--json", action="store_true")
        if name == "install":
            p.add_argument("--set", action="append", default=[], metavar="KEY=VALUE")
            p.add_argument("--storage", action="append", default=[], metavar="KEY=PATH")
            p.add_argument("--device", action="append", default=[], metavar="ID=on|off")
            p.add_argument("--settings-stdin", action="store_true", help="read a JSON object of settings (secrets!) from standard input")
            p.add_argument("--yes", action="store_true", help="accept the privileges shown")
        if name == "remove":
            p.add_argument("--delete-data", action="store_true")
            p.add_argument("--yes", action="store_true", help="confirm deleting the data")
        if name == "logs":
            p.add_argument("--tail", type=int, default=200)
    try:
        args = ap.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    cmd = args.cmd

    def jprint(obj: Any) -> None:
        out(json.dumps(obj, indent=1, sort_keys=True))
    try:
        mgr.recover()
        if cmd == "list":
            d = mgr.list()
            if args.json:
                jprint(d)
            else:
                out("%-12s %-22s %-16s %s" % ("ID", "NAME", "STATE", "VERSION"))
                for c in d["addons"]:
                    out(_fmt_card(c))
                for p in d["problems"]:
                    err("problem: %s (%s): %s" % (p["id"], p["source"], p["error"]))
            return 0
        if cmd == "apply-pending":
            doc = apply_pending(mgr, emit=lambda m: out(m) if not args.json else None)
            if args.json:
                jprint(doc)
            else:
                for r in doc["items"]:
                    out("%-12s %-9s %s" % (r["id"], r["status"], r["message"]))
                if not doc["items"]:
                    out("nothing pending")
            return 0                                          # never fails the boot
        if cmd == "show":
            d = mgr.show(args.id)
            if args.json:
                jprint(d)
            else:
                out("%s (%s) %s" % (d["name"], d["id"], d["status"]))
                out(d["summary"])
                for s in d["settings"]:
                    v = d["values"].get(s["key"])
                    out("  %-18s %s%s" % (s["key"], "(secret, %s)" % ("set" if v and v.get("set") else "not set") if s["type"] == "secret" else v, "  [%s]" % s["label"]))
                for p in d["privileges"]:
                    out("  privilege: %s - %s" % (p["id"], p["help"]))
                for r in d["storage_report"]:
                    out("  storage %s: %s%s%s" % (r["key"], r["path"], " (%s GiB free)" % r["free_gib"] if r["free_gib"] is not None else "", "  ERROR: " + r["error"] if r["error"] else ""))
                for dv in d["device_states"]:
                    out("  device %s: %s" % (dv["id"], "present" if dv["present"] else "not found"))
            return 0
        if cmd == "status":
            st = mgr.status(args.id)
            out(json.dumps(st, sort_keys=True) if args.json else "%s: %s%s" % (args.id, st["status"], " - " + st["detail"] if st.get("detail") else ""))
            return 0 if st["status"] == "running" else 1
        if cmd == "logs":
            out(mgr.logs(args.id, args.tail))
            return 0
        sink = None if args.json else (lambda m: out(m))
        if cmd == "install":
            a = mgr.addon(args.id)
            body: dict = {"settings": {}, "storage": {}, "devices": {}}
            if args.settings_stdin:
                data = json.loads((stdin or sys.stdin).read() or "{}")
                if not isinstance(data, dict):
                    raise AddonError("standard input must be a JSON object")
                body["settings"].update(data)
            for kv in args.set:
                k, sep, v = kv.partition("=")
                if not sep:
                    raise AddonError("--set needs KEY=VALUE")
                if (a.setting(k) or {}).get("type") == "secret":
                    raise AddonError("%s is a secret: pass it as JSON on standard input (--settings-stdin), not on the command line" % k)
                body["settings"][k] = v
            for kv in args.storage:
                k, sep, v = kv.partition("=")
                if not sep:
                    raise AddonError("--storage needs KEY=PATH")
                body["storage"][k] = v
            for kv in args.device:
                k, sep, v = kv.partition("=")
                if v not in ("on", "off"):
                    raise AddonError("--device needs ID=on or ID=off")
                body["devices"][k] = v == "on"
            if args.yes:
                body["accept"] = True
            job = mgr.install(args.id, body, user="cli", ip="-", background=False, sink=sink)
        elif cmd == "update":
            job = mgr.update(args.id, {}, user="cli", ip="-", background=False, sink=sink)
        elif cmd == "remove":
            if args.delete_data and not args.yes:
                raise AddonError("--delete-data erases the add-on's data for good; add --yes to confirm")
            job = mgr.remove(args.id, args.delete_data, user="cli", ip="-", background=False, sink=sink)
        else:
            job = mgr.control(args.id, cmd, user="cli", ip="-", background=False, sink=sink)
        if args.json:
            jprint(job.view())
        else:
            for w in job.warnings:
                err("warning: " + w)
            out("ok" if job.ok else "failed: " + job.error)
        return 0 if job.ok else 1
    except Busy as e:
        err("busy: %s" % e)
        return 1
    except NotFound as e:
        err("error: %s" % e)
        return 2
    except (AddonError, ValueError) as e:
        err("error: %s" % e)
        return 2
