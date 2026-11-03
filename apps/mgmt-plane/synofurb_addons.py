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
RAW_COMPOSE_RE = re.compile(r"(?:[A-Za-z0-9._/~][A-Za-z0-9._/@+=,:~-]*)?")
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
    strict_keys: frozenset = frozenset()           # settings that are substituted raw into the compose file

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
                "settings": [{k: v for k, v in s.items() if k != "pattern_re"} for s in self.settings], "storage": self.storage,
                "devices": self.devices, "privileges": [{"id": p, "help": privilege_help(p)} for p in self.privileges],
                "post_install": self.post_install, "changes": list(self.changes), "source": self.source, "data_dir": self.data_dir}


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
    s: dict = {"key": key, "label": _text(d, "label", path, 60), "type": typ, "required": d.get("required", False), "help": _text(d, "help", path, 300, required=False)}
    if not isinstance(s["required"], bool):
        raise _where(path, "'required' must be true or false")
    for k in ("min", "max", "pattern", "choices", "maxlen"):
        if k in d:
            ok = {"min": ("int",), "max": ("int",), "pattern": ("string", "path", "secret"), "choices": ("choice",), "maxlen": ("string", "secret", "text")}[k]
            if typ not in ok:
                raise _where(path, "'%s' is not allowed for type '%s'" % (k, typ))
    if typ == "int":
        lo = _int(d.get("min", -2 ** 31), path, "min", -2 ** 31, 2 ** 31)
        hi = _int(d.get("max", 2 ** 31), path, "max", -2 ** 31, 2 ** 31)
        if lo > hi:
            raise _where(path, "'min' is larger than 'max'")
        s["min"], s["max"] = lo, hi
    if typ == "choice":
        ch = _need_list(d.get("choices"), path + ".choices", 32) if "choices" in d else []
        if not ch:
            raise _where(path, "a choice setting needs 'choices'")
        if not all(isinstance(c, str) and CHOICE_RE.fullmatch(c) for c in ch) or len(set(ch)) != len(ch):
            raise _where(path, "'choices' must be unique short names (letters, digits, . _ -)")
        s["choices"] = list(ch)
    if "pattern" in d:
        s["pattern"] = _text(d, "pattern", path, 120)
        _compile(s["pattern"], path)
    if typ in ("string", "secret", "text"):
        cap = 16000 if typ == "text" else 512
        s["maxlen"] = _int(d.get("maxlen", 4000 if typ == "text" else 200), path, "maxlen", 1, cap)
    if typ == "secret" and "default" in d:
        raise _where(path, "a secret must not have a default")
    if "default" in d:
        try:
            s["default"] = coerce_setting(s, d["default"], strict=False, label=key)
        except AddonError as e:
            raise _where(path, "default is not valid: %s" % e)
    return s


def coerce_setting(s: dict, v: Any, strict: bool, label: str = "") -> Any:
    """Validate one value against its setting definition.  ``strict`` = the value goes raw into a compose file."""
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
        if v not in s["choices"]:
            raise AddonError("%s must be one of: %s" % (name, ", ".join(s["choices"])))
        return v
    if typ == "path":
        return valid_abs_path(v, name)
    if len(v) > s.get("maxlen", 200):
        raise AddonError("%s is too long (max %d characters)" % (name, s.get("maxlen", 200)))
    if CTRL_RE.search(v.replace("\n", "").replace("\t", "") if typ == "text" else v):
        raise AddonError("%s must not contain control characters" % name)
    if "pattern" in s and v and not re.fullmatch(s["pattern"], v):
        raise AddonError("%s does not have the expected format" % name)
    if strict and not RAW_COMPOSE_RE.fullmatch(v):
        raise AddonError("%s may only contain letters, digits and . _ / @ + = , : ~ - (no spaces or quotes)" % name)
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
    _only_keys(d, ("host", "container", "proto", "label"), path)
    proto = d.get("proto", "tcp")
    if proto not in ("tcp", "udp"):
        raise _where(path, "'proto' must be tcp or udp")
    return {"host": _int(d.get("host"), path, "host", 1, 65535), "container": _int(d.get("container"), path, "container", 1, 65535), "proto": proto,
            "label": _text(d, "label", path, 40, required=False)}


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


_RE_BLOCK_VAR = re.compile(r"^[ \t]*\$\{DEVICES_BLOCK\}[ \t]*$", re.M)


def check_devices_block(text: str, name: str) -> None:
    """${DEVICES_BLOCK} expands to several lines, so it must stand alone on its line."""
    n = len(re.findall(r"\$\{?DEVICES_BLOCK\}?", text))
    if n and n != len(_RE_BLOCK_VAR.findall(text)):
        raise AddonError("%s: ${DEVICES_BLOCK} must be alone on its line" % name)
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
    privs = _need_list(d.get("privileges", []), "privileges", 16)
    for p in privs:
        if not isinstance(p, str) or not (p in FIXED_PRIVILEGES or CAP_RE.fullmatch(p)):
            raise _where("privileges", "unknown privilege %r (use %s or cap:<NAME>)" % (str(p)[:30], ", ".join(FIXED_PRIVILEGES)))
    if len(set(privs)) != len(privs):
        raise _where("privileges", "duplicate entry")
    ports = [_parse_port(p, i) for i, p in enumerate(_need_list(d.get("ports", []), "ports", 24))]
    web = _parse_web(d["web"]) if d.get("web") is not None else None
    health = _parse_health(d["healthcheck"]) if d.get("healthcheck") is not None else None
    handler = _text(d, "handler", "addon.json", 32, required=False, regex=KEY_RE)
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
                  files=flist, healthcheck=health, post_install=_text(d, "post_install", "addon.json", 400, required=False), changes=changes, source=source)
    _check_templates(addon, files)
    return addon


def _check_templates(a: Addon, files: dict[str, str]) -> None:
    if a.kind != "compose":
        return
    known = template_vars(a.settings, a.storage)
    need = [a.compose] + [f["template"] for f in a.files]
    for n in need:
        if n not in files:
            raise AddonError("%s: the file %s is listed in addon.json but is missing" % (a.id, n))
    used_raw: set = set()
    for n in need:
        text = files[n]
        ids = scan_template(text, known, n)
        check_devices_block(text, n)
        if n == a.compose:
            used_raw = {i for i in ids if not i.endswith("_json")}
            for i in ids:
                s = a.setting(i)
                if s and s["type"] == "text":
                    raise AddonError("%s: the text setting '%s' cannot be used in the compose file (use it in a rendered file)" % (n, i))
            if "DEVICES_BLOCK" in ids and not a.devices:
                raise AddonError("%s: uses ${DEVICES_BLOCK} but the add-on declares no devices" % n)
            problems = audit_compose(text, a.privileges, devices_ok=True)
            if problems:
                raise AddonError("%s: %s" % (n, "; ".join(problems)))
    a.templates = {n: files[n] for n in need}
    a.strict_keys = frozenset(i for i in used_raw if a.setting(i))
    for f in a.files:
        if f["template"] == a.compose:
            raise AddonError("files: the compose template cannot also be a rendered file")


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
