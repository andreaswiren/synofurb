#!/usr/bin/env python3
"""synofurb-mgmt - tiny web management plane for a Debian 13 NAS.

Single-file, standard-library-only HTTPS server (Python 3.13) that runs as
root under systemd and offers a small JSON API plus an embedded single-page
UI for: status, local users, networkd configuration (with safe apply and
automatic rollback), RAID/LVM volume creation, mounts (incl. imported
Synology volumes) and a few service/system actions.

Design rules
  * No shell=True, argv lists only, fixed PATH, timeout on every command.
  * Every external value is validated by a strict validator before use.
  * All privileged actions go through an injectable ``Runner``; all file
    writes go through ``Config.p()`` so a ``--root`` prefix can sandbox them.
  * Secrets (passwords) travel via stdin only and are redacted from the audit log.
"""
from __future__ import annotations

import argparse
import base64
import ctypes
import ctypes.util
import fnmatch
import glob
import hashlib
import hmac
import ipaddress
import json
import os
import re
import secrets
import shlex
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import urllib.parse
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Callable, Iterable

VERSION = "1.0.0"
SAFE_PATH = "/usr/sbin:/usr/bin:/sbin:/bin"
MAX_BODY = 64 * 1024
GIB = 1024 ** 3


# ============================================================================
# 1. Configuration, errors
# ============================================================================

@dataclass
class Config:
    """Runtime configuration. ``root`` prefixes every filesystem path we touch
    under /etc, /var, /srv, /mnt, /home (never /proc or /sys)."""
    root: str = ""
    dry_run: bool = False
    bind: str = "::"
    port: int = 8443
    tls: bool = True
    idle_timeout: int = 30 * 60
    abs_timeout: int = 12 * 3600
    rollback_seconds: int = 90
    fail_delay: float = 0.4
    login_group: str = "sudo"
    state_dir: str = "/var/lib/synofurb-mgmt"
    audit_path: str = "/var/log/synofurb-mgmt/audit.log"
    config_path: str = "/etc/synofurb/mgmt.json"

    def p(self, path: str) -> str:
        """Map an absolute system path into the (optional) root prefix."""
        if not self.root:
            return path
        return os.path.normpath(os.path.join(self.root, path.lstrip("/")))

    def load_file(self) -> None:
        """Overlay optional /etc/synofurb/mgmt.json (only known, typed keys)."""
        try:
            with open(self.p(self.config_path), encoding="utf-8") as fh:
                data = json.load(fh)
        except (OSError, ValueError):
            return
        if not isinstance(data, dict):
            return
        for key in ("port", "idle_timeout", "abs_timeout", "rollback_seconds"):
            if isinstance(data.get(key), int) and not isinstance(data.get(key), bool) and data[key] > 0:
                setattr(self, key, data[key])
        if isinstance(data.get("bind"), str) and data["bind"]:
            self.bind = data["bind"]
        if isinstance(data.get("login_group"), str) and valid_group_name_ok(data["login_group"]):
            self.login_group = data["login_group"]


class ValidationError(ValueError):
    """Raised by validators for any rejected input."""


class ApiError(Exception):
    """An error with an HTTP status that is safe to show to the client."""

    def __init__(self, status: int, message: str, headers: dict[str, str] | None = None):
        super().__init__(message)
        self.status = status
        self.message = message
        self.headers = headers or {}


# ============================================================================
# 2. Validators (the command-injection boundary)
# ============================================================================

_USER_RE = re.compile(r"[a-z_][a-z0-9_-]{0,31}")
_LABEL_RE = re.compile(r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?")
_IFNAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,14}")
_VOLNAME_RE = re.compile(r"[a-z][a-z0-9_]{0,23}")
_DEV_RE = re.compile(r"/dev/[A-Za-z0-9][A-Za-z0-9_.+-]{0,63}(?:/[A-Za-z0-9][A-Za-z0-9_.+-]{0,63})?")
_MPCOMP_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}")
_LVMNAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.+-]{0,126}")
_UUID_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9-]{2,63}")
_SSHKEY_RE = re.compile(
    r"(?:ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp(?:256|384|521)|sk-ssh-ed25519@openssh\.com|"
    r"sk-ecdsa-sha2-nistp256@openssh\.com) [A-Za-z0-9+/]{16,}={0,3}(?: [\x20-\x7e]{0,100})?")

RESERVED_VOLUME_NAMES = frozenset({"sys", "data", "root", "swap", "boot", "efi"})
CREATE_FS = ("xfs", "ext4", "btrfs")
MOUNT_FS = ("xfs", "ext4", "btrfs", "vfat", "exfat", "ntfs")
RAID_LEVELS = ("none", "raid0", "raid1", "raid5", "raid6", "raid10")
EXTRA_GROUPS_ALWAYS = ("sudo", "users", "adm")
PROTECTED_MOUNTS = frozenset({"/", "/boot", "/boot/efi", "/boot/firmware", "/usr", "/var", "[SWAP]"})
RESTARTABLE_UNITS = ("ssh", "systemd-resolved", "systemd-timesyncd", "cron")


def _need_str(v: Any, what: str) -> str:
    if not isinstance(v, str):
        raise ValidationError(f"{what} must be a string")
    return v


def valid_username(v: Any) -> str:
    s = _need_str(v, "user name")
    if not _USER_RE.fullmatch(s):
        raise ValidationError("invalid user name (a-z, 0-9, _ and -, max 32, must start with a letter or _)")
    return s


def valid_group_name_ok(v: Any) -> bool:
    return isinstance(v, str) and bool(_USER_RE.fullmatch(v))


def valid_hostname(v: Any) -> str:
    s = _need_str(v, "hostname")
    if not 1 <= len(s) <= 64:
        raise ValidationError("hostname must be 1-64 characters")
    for label in s.split("."):
        if not _LABEL_RE.fullmatch(label):
            raise ValidationError("invalid hostname (letters, digits, '-' and dots; no leading/trailing '-')")
    return s.lower()


def valid_ifname(v: Any) -> str:
    s = _need_str(v, "interface name")
    if not _IFNAME_RE.fullmatch(s) or ".." in s:
        raise ValidationError("invalid interface name")
    return s


def valid_volume_name(v: Any) -> str:
    s = _need_str(v, "volume name")
    if not _VOLNAME_RE.fullmatch(s):
        raise ValidationError("volume name must match [a-z][a-z0-9_]{0,23}")
    if s in RESERVED_VOLUME_NAMES:
        raise ValidationError(f"volume name '{s}' is reserved")
    return s


def valid_lvm_name(v: Any) -> str:
    s = _need_str(v, "LVM name")
    if not _LVMNAME_RE.fullmatch(s) or ".." in s:
        raise ValidationError("invalid LVM name")
    return s


def valid_device(v: Any) -> str:
    s = _need_str(v, "device")
    if not _DEV_RE.fullmatch(s) or ".." in s:
        raise ValidationError("invalid device path")
    return s


def valid_fs_create(v: Any) -> str:
    if v not in CREATE_FS:
        raise ValidationError("filesystem must be one of: " + ", ".join(CREATE_FS))
    return v


def valid_fs_mount(v: Any) -> str:
    if v not in MOUNT_FS:
        raise ValidationError("unsupported filesystem for mounting: %s" % (v,))
    return v


def valid_raid_level(v: Any) -> str:
    if v not in RAID_LEVELS:
        raise ValidationError("RAID level must be one of: " + ", ".join(RAID_LEVELS))
    return v


def valid_uuid(v: Any) -> str:
    s = _need_str(v, "uuid")
    if not _UUID_RE.fullmatch(s):
        raise ValidationError("invalid uuid")
    return s


def valid_mountpoint(v: Any) -> str:
    """Absolute, normalised path strictly below /srv or /mnt (no '..', no '//')."""
    s = _need_str(v, "mountpoint")
    if len(s) > 160 or not s.startswith("/") or s.endswith("/") or "//" in s or ".." in s:
        raise ValidationError("mountpoint must be a clean absolute path below /srv or /mnt")
    parts = s.split("/")[1:]
    if parts[0] not in ("srv", "mnt") or len(parts) < 2 or len(parts) > 4:
        raise ValidationError("mountpoint must be /srv/<name> or /mnt/<name> (max 3 levels)")
    for comp in parts[1:]:
        if not _MPCOMP_RE.fullmatch(comp):
            raise ValidationError("mountpoint contains invalid characters")
    return s


def check_no_symlinks(cfg: Config, mountpoint: str) -> None:
    """Reject if any existing component of the mountpoint is a symlink."""
    cur = cfg.root or "/"
    for comp in mountpoint.strip("/").split("/"):
        cur = os.path.join(cur, comp)
        if os.path.islink(cur):
            raise ValidationError("mountpoint path contains a symlink: " + cur[len(cfg.root):])


def valid_password(v: Any) -> str:
    s = _need_str(v, "password")
    if not 8 <= len(s) <= 128:
        raise ValidationError("password must be 8-128 characters")
    if any(c in s for c in "\n\r\x00"):
        raise ValidationError("password contains forbidden control characters")
    return s


def valid_ssh_pubkey(v: Any) -> str:
    s = _need_str(v, "SSH key").strip()
    if not _SSHKEY_RE.fullmatch(s):
        raise ValidationError("not a valid single-line SSH public key (options prefixes are not allowed)")
    return s


def valid_ipv4_cidr(v: Any) -> str:
    s = _need_str(v, "IPv4 address")
    if "/" not in s:
        raise ValidationError("IPv4 address needs a prefix length, e.g. 192.168.1.10/24")
    try:
        iface = ipaddress.IPv4Interface(s)
    except ValueError as e:
        raise ValidationError("invalid IPv4 address/prefix: %s" % e)
    a = iface.ip
    if a.is_unspecified or a.is_loopback or a.is_multicast or a.is_link_local or iface.network.prefixlen < 1:
        raise ValidationError("IPv4 address not usable as a static address")
    n = iface.network
    if n.prefixlen < 31 and (a == n.network_address or a == n.broadcast_address):
        raise ValidationError("address is the network or broadcast address")
    return str(iface)


def valid_ipv6_cidr(v: Any) -> str:
    s = _need_str(v, "IPv6 address")
    if "/" not in s:
        raise ValidationError("IPv6 address needs a prefix length, e.g. 2001:db8::10/64")
    try:
        iface = ipaddress.IPv6Interface(s)
    except ValueError as e:
        raise ValidationError("invalid IPv6 address/prefix: %s" % e)
    a = iface.ip
    if a.is_unspecified or a.is_loopback or a.is_multicast or a.is_link_local:
        raise ValidationError("IPv6 address not usable as a static address")
    return str(iface)


def valid_ip(v: Any, version: int | None = None) -> str:
    s = _need_str(v, "IP address")
    try:
        ip = ipaddress.ip_address(s)
    except ValueError:
        raise ValidationError("invalid IP address: %r" % s[:64])
    if version and ip.version != version:
        raise ValidationError("expected an IPv%d address" % version)
    if ip.is_unspecified or ip.is_multicast:
        raise ValidationError("IP address not usable here")
    return str(ip)


def valid_gateway(v: Any, version: int, iface_cidr: str) -> str:
    """Gateway must be in the interface subnet (IPv6 link-local is exempt)."""
    gw = ipaddress.ip_address(valid_ip(v, version))
    if gw.is_loopback:
        raise ValidationError("gateway cannot be loopback")
    net = ipaddress.ip_interface(iface_cidr).network
    if not (version == 6 and gw.is_link_local) and gw not in net:
        raise ValidationError("gateway %s is not inside %s" % (gw, net))
    return str(gw)


def valid_dns(v: Any) -> str:
    return valid_ip(v)


def valid_search_domain(v: Any) -> str:
    s = _need_str(v, "search domain")
    if not 1 <= len(s) <= 253:
        raise ValidationError("invalid search domain")
    for label in s.split("."):
        if not _LABEL_RE.fullmatch(label):
            raise ValidationError("invalid search domain: %r" % s[:64])
    return s.lower()


def as_bool(v: Any) -> bool:
    return v is True or v in ("true", "1", 1, "on", "yes")


# ============================================================================
# 3. Runner abstraction + file helpers
# ============================================================================

@dataclass
class CompletedResult:
    rc: int
    out: str = ""
    err: str = ""

    @property
    def ok(self) -> bool:
        return self.rc == 0


def resolve_exe(name: str) -> str | None:
    if name.startswith("/"):
        return name if os.access(name, os.X_OK) else None
    return shutil.which(name, path=SAFE_PATH)


class Runner:
    """Interface for running external programs (argv lists only)."""

    def run(self, argv: list[str], input: str | None = None, timeout: int = 60) -> CompletedResult:
        raise NotImplementedError

    def which(self, name: str) -> str | None:
        return resolve_exe(name)


def _check_argv(argv: list[str]) -> None:
    if not isinstance(argv, list) or not argv or not all(isinstance(a, str) and "\x00" not in a for a in argv):
        raise ValueError("argv must be a non-empty list of NUL-free strings")


class RealRunner(Runner):
    """Executes commands with subprocess (no shell, fixed PATH, timeout)."""

    ENV = {"PATH": SAFE_PATH, "LC_ALL": "C", "LANG": "C", "HOME": "/root"}

    def run(self, argv: list[str], input: str | None = None, timeout: int = 60) -> CompletedResult:
        _check_argv(argv)
        exe = resolve_exe(argv[0])
        if exe is None:
            return CompletedResult(127, "", "command not found: " + argv[0])
        try:
            p = subprocess.run([exe] + argv[1:], input=input, capture_output=True, text=True,
                               timeout=timeout, env=self.ENV, shell=False,
                               stdin=None if input is not None else subprocess.DEVNULL,
                               errors="replace")
            return CompletedResult(p.returncode, p.stdout, p.stderr)
        except subprocess.TimeoutExpired:
            return CompletedResult(124, "", "timed out after %ds: %s" % (timeout, argv[0]))
        except OSError as e:
            return CompletedResult(126, "", "cannot execute %s: %s" % (argv[0], e))


class DryRunRunner(Runner):
    """Records commands and returns canned/empty success. With ``passthrough``
    read-only inspection commands (lsblk, ip, ...) really run, so a developer
    sees realistic data while nothing is ever modified."""

    READ_ONLY = frozenset({"lsblk", "ip", "findmnt", "vgs", "lvs", "pvs", "smartctl", "ssh-keygen", "openssl"})

    def __init__(self, canned: dict[str, Any] | None = None, passthrough: bool = False, verbose: bool = False):
        self.calls: list[list[str]] = []
        self.input_sizes: list[int | None] = []
        self.canned: dict[str, Any] = {"blkid": "00000000-0000-4000-8000-dryrun000001\n"}
        self.canned.update(canned or {})
        self.passthrough = passthrough
        self.verbose = verbose
        self._real = RealRunner()

    def _passthrough_ok(self, argv: list[str]) -> bool:
        name = os.path.basename(argv[0])
        if name in self.READ_ONLY:
            return True
        if name == "systemctl" and len(argv) > 1 and argv[1] in ("is-enabled", "is-active", "list-units"):
            return True
        return name == "mdadm" and "--detail" in argv

    def run(self, argv: list[str], input: str | None = None, timeout: int = 60) -> CompletedResult:
        _check_argv(argv)
        self.calls.append(list(argv))
        self.input_sizes.append(None if input is None else len(input))
        if self.verbose:
            print("[dry-run] " + shlex.join(argv), file=sys.stderr)
        if self.passthrough and self._passthrough_ok(argv):
            return self._real.run(argv, input, timeout)
        canned = self.canned.get(os.path.basename(argv[0]))
        if callable(canned):
            canned = canned(argv)
        if isinstance(canned, CompletedResult):
            return canned
        return CompletedResult(0, canned or "", "")

    def which(self, name: str) -> str | None:
        return resolve_exe(name) or ("/usr/sbin/" + name if not self.passthrough else None)


def read_text(path: str) -> str:
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            return fh.read()
    except OSError:
        return ""


def write_atomic(path: str, data: str, mode: int = 0o644, uid: int | None = None, gid: int | None = None) -> None:
    """Atomically replace ``path`` (same-dir temp file + rename)."""
    d = os.path.dirname(path)
    os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".tmp-synofurb-")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(data)
            fh.flush()
            os.fsync(fh.fileno())
        os.chmod(tmp, mode)
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


@dataclass
class Step:
    """One unit of a plan: a command, or an 'internal' action (file edit)."""
    desc: str
    argv: list[str] | None = None
    kind: str = "cmd"            # cmd | mdconf | fstab | mkdir
    timeout: int = 60
    ignore_error: bool = False
    meta: dict = field(default_factory=dict)

    def display(self) -> str:
        if self.kind == "cmd" and self.argv:
            return shlex.join(self.argv)
        if self.kind == "mdconf" and self.argv:
            return shlex.join(self.argv) + "  >> /etc/mdadm/mdadm.conf"
        if self.kind == "fstab":
            return "[edit /etc/fstab] %s %s %s" % (self.meta.get("mountpoint"), self.meta.get("fstype"), self.meta.get("options"))
        if self.kind == "mkdir":
            return "mkdir -p " + shlex.quote(str(self.meta.get("path")))
        return self.kind

    def to_dict(self) -> dict:
        return {"desc": self.desc, "argv": self.argv, "kind": self.kind, "cmd": self.display()}


# ============================================================================
# 4. Parsers (pure)
# ============================================================================

def _to_int(v: Any) -> int:
    try:
        return int(v)
    except (TypeError, ValueError):
        return 0


def _to_bool(v: Any) -> bool:
    return v is True or v in ("1", 1, "true")


LSBLK_COLS = "NAME,KNAME,TYPE,SIZE,FSTYPE,UUID,LABEL,MOUNTPOINTS,MODEL,SERIAL,ROTA,TRAN,PKNAME"


def parse_lsblk(text: str) -> list[dict]:
    """Flatten ``lsblk -J`` output (tree) into a de-duplicated device list.

    Each entry: name, kname, path, type, size, fstype, uuid, label, mountpoints,
    model, serial, rota, tran, pkname, children (knames)."""
    try:
        data = json.loads(text)
    except (ValueError, TypeError):
        return []
    out: dict[str, dict] = {}

    def walk(node: dict, parent: str | None) -> None:
        kname = node.get("kname") or node.get("name")
        if not kname:
            return
        name = node.get("name") or kname
        typ = node.get("type") or ""
        mps = node.get("mountpoints")
        if mps is None:
            mps = [node.get("mountpoint")]
        dev = out.get(kname)
        if dev is None:
            path = "/dev/mapper/" + name if typ in ("lvm", "crypt", "mpath", "dmraid") else "/dev/" + kname
            dev = out[kname] = {
                "name": name, "kname": kname, "path": path, "type": typ,
                "size": _to_int(node.get("size")), "fstype": node.get("fstype") or None,
                "uuid": node.get("uuid") or None, "label": node.get("label") or None,
                "mountpoints": [m for m in mps if m],
                "model": (node.get("model") or "").strip() or None,
                "serial": (node.get("serial") or "").strip() or None,
                "rota": _to_bool(node.get("rota")), "tran": node.get("tran") or None,
                "pkname": node.get("pkname") or parent, "children": [],
            }
        if parent and kname not in out[parent]["children"]:
            out[parent]["children"].append(kname)
        for child in node.get("children") or []:
            walk(child, kname)

    for top in data.get("blockdevices", []) if isinstance(data, dict) else []:
        walk(top, None)
    return list(out.values())


def descendants(by_k: dict[str, dict], kname: str) -> Iterable[dict]:
    seen: set[str] = set()
    stack = list(by_k[kname]["children"]) if kname in by_k else []
    while stack:
        k = stack.pop()
        if k in seen or k not in by_k:
            continue
        seen.add(k)
        yield by_k[k]
        stack.extend(by_k[k]["children"])


def subtree_mounts(by_k: dict[str, dict], dev: dict) -> list[str]:
    mps = list(dev["mountpoints"])
    for d in descendants(by_k, dev["kname"]):
        mps.extend(d["mountpoints"])
    return mps


def holds_system(by_k: dict[str, dict], dev: dict) -> bool:
    """True if the device (or anything stacked on it) holds /, /boot*, swap, /usr, /var."""
    return any(m in PROTECTED_MOUNTS for m in subtree_mounts(by_k, dev))


MIN_MEMBER_BYTES = 256 * 1024 * 1024
_SIG_NAMES = {"linux_raid_member": "a RAID member", "LVM2_member": "an LVM physical volume",
              "swap": "swap space", "crypto_LUKS": "a LUKS volume"}


def device_unusable_reason(dev: dict, by_k: dict[str, dict]) -> str | None:
    """Why a block device may NOT be used as a new volume member (None = free)."""
    if dev["type"] not in ("disk", "part"):
        return "not a disk or partition"
    if dev["size"] < MIN_MEMBER_BYTES:
        return "too small"
    if holds_system(by_k, dev):
        return "holds the root/boot/swap filesystem"
    if dev["fstype"]:
        return "already %s" % _SIG_NAMES.get(dev["fstype"], "formatted (%s)" % dev["fstype"])
    if subtree_mounts(by_k, dev):
        return "mounted"
    if dev["children"]:
        kinds = sorted({by_k[c]["type"] for c in dev["children"] if c in by_k})
        return "in use (has %s)" % "/".join(kinds or ["children"])
    parent = by_k.get(dev["pkname"] or "")
    if parent and holds_system(by_k, parent):
        return "on the system disk"
    return None


def candidate_devices(devs: list[dict]) -> tuple[list[dict], list[dict]]:
    """Split into (free candidates, excluded-with-reason) for whole disks/partitions."""
    by_k = {d["kname"]: d for d in devs}
    free, excl = [], []
    for d in devs:
        if d["type"] not in ("disk", "part") or d["kname"].startswith(("loop", "sr", "zram", "ram")):
            continue
        reason = device_unusable_reason(d, by_k)
        (excl if reason else free).append(dict(d, reason=reason) if reason else d)
    return free, excl


_MD_HEAD = re.compile(r"^(md\w+)\s*:\s*(.*)$")
_MD_MEMBER = re.compile(r"^([A-Za-z0-9_/.-]+)\[(\d+)\]((?:\([A-Z]\))*)$")
_MD_LEVEL = re.compile(r"^(raid\d+|linear|multipath|faulty|container)$")


def parse_mdstat(text: str) -> list[dict]:
    """Parse /proc/mdstat into a list of array dicts."""
    arrays: list[dict] = []
    cur: dict | None = None
    for raw in text.splitlines():
        m = _MD_HEAD.match(raw)
        if m:
            toks = m.group(2).split()
            cur = {"name": m.group(1), "device": "/dev/" + m.group(1), "state": toks[0] if toks else "",
                   "flags": [], "level": None, "members": [], "blocks": 0, "size": 0, "total": None,
                   "active": None, "health": None, "degraded": False, "sync": None}
            for t in toks[1:]:
                mm = _MD_MEMBER.match(t)
                if mm:
                    f = mm.group(3)
                    cur["members"].append({"name": mm.group(1), "role": int(mm.group(2)),
                                           "failed": "(F)" in f, "spare": "(S)" in f})
                elif _MD_LEVEL.match(t):
                    cur["level"] = t
                elif t.startswith("(") and t.endswith(")"):
                    cur["flags"].append(t[1:-1])
            arrays.append(cur)
            continue
        if cur is None or not raw.startswith((" ", "\t")):
            if raw.strip():
                cur = None          # e.g. "unused devices:" ends the last array
            continue
        b = re.search(r"(\d+) blocks", raw)
        if b:
            cur["blocks"] = int(b.group(1))
            cur["size"] = cur["blocks"] * 1024
        h = re.search(r"\[(\d+)/(\d+)\]\s*\[([U_]+)\]", raw)
        if h:
            cur["total"], cur["active"], cur["health"] = int(h.group(1)), int(h.group(2)), h.group(3)
        s = re.search(r"(resync|recovery|reshape|check|repair)\s*=\s*([\d.]+)%", raw)
        if s:
            fin = re.search(r"finish=([\d.]+)min", raw)
            spd = re.search(r"speed=(\d+)K/sec", raw)
            cur["sync"] = {"action": s.group(1), "percent": float(s.group(2)),
                           "finish_min": float(fin.group(1)) if fin else None,
                           "speed_kbs": int(spd.group(1)) if spd else None}
        else:
            d = re.search(r"(resync|recovery|reshape|check)=(DELAYED|PENDING)", raw)
            if d:
                cur["sync"] = {"action": d.group(1), "percent": None, "state": d.group(2).lower()}
    for a in arrays:
        a["degraded"] = bool(a["health"] and "_" in a["health"]) or any(x["failed"] for x in a["members"])
    return arrays


def parse_mdadm_scan(text: str) -> list[dict]:
    """Parse ``mdadm --detail --scan`` ARRAY lines."""
    res = []
    for line in text.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[0] == "ARRAY":
            kv = dict(p.split("=", 1) for p in parts[2:] if "=" in p)
            res.append({"device": parts[1], "uuid": kv.get("UUID"), "name": kv.get("name"),
                        "metadata": kv.get("metadata"), "line": line.strip()})
    return res


_LVM_INT_SUFFIX = ("_size", "_free", "_count", "_used")


def parse_lvm_report(text: str, section: str) -> list[dict]:
    """Rows of a ``vgs/lvs/pvs --reportformat json`` report (sizes become int)."""
    try:
        rows = json.loads(text)["report"][0][section]
    except (ValueError, KeyError, IndexError, TypeError):
        return []
    out = []
    for r in rows:
        out.append({k: (_to_int(v) if k.endswith(_LVM_INT_SUFFIX) else v) for k, v in r.items()})
    return out


PSEUDO_FS = frozenset({"proc", "sysfs", "devtmpfs", "devpts", "tmpfs", "cgroup", "cgroup2", "securityfs", "debugfs",
                       "tracefs", "configfs", "fusectl", "pstore", "bpf", "mqueue", "hugetlbfs", "autofs",
                       "binfmt_misc", "efivarfs", "ramfs", "rpc_pipefs", "nsfs", "fuse.gvfsd-fuse", "overlay", "squashfs"})


def parse_findmnt(text: str) -> list[dict]:
    """Flatten ``findmnt -J`` into rows; ``real`` is False for pseudo filesystems."""
    try:
        data = json.loads(text)
    except (ValueError, TypeError):
        return []
    rows: list[dict] = []

    def walk(n: dict) -> None:
        fs = n.get("fstype") or ""
        rows.append({"target": n.get("target"), "source": n.get("source"), "fstype": fs,
                     "options": n.get("options"), "size": _to_int(n.get("size")),
                     "used": _to_int(n.get("used")), "avail": _to_int(n.get("avail")),
                     "real": fs not in PSEUDO_FS and not str(n.get("target", "")).startswith(("/proc", "/sys", "/run", "/dev"))})
        for c in n.get("children") or []:
            walk(c)

    for top in data.get("filesystems", []) if isinstance(data, dict) else []:
        walk(top)
    return rows


def parse_ip_addr(text: str) -> list[dict]:
    """Reduce ``ip -j addr`` to interface summaries."""
    try:
        data = json.loads(text)
    except (ValueError, TypeError):
        return []
    res = []
    for i in data if isinstance(data, list) else []:
        addrs = [{"family": a.get("family"), "addr": a.get("local"), "prefix": a.get("prefixlen"),
                  "scope": a.get("scope")} for a in i.get("addr_info", []) if a.get("local")]
        res.append({"ifname": i.get("ifname"), "state": i.get("operstate"), "mac": i.get("address"),
                    "mtu": i.get("mtu"), "link_type": i.get("link_type"), "flags": i.get("flags", []),
                    "addrs": addrs})
    return res


# --- passwd / group / shadow -------------------------------------------------

@dataclass
class PasswdEntry:
    name: str
    uid: int
    gid: int
    gecos: str
    home: str
    shell: str


def parse_passwd(text: str) -> dict[str, PasswdEntry]:
    res = {}
    for line in text.splitlines():
        f = line.split(":")
        if len(f) >= 7 and f[0] and not line.startswith("#"):
            res[f[0]] = PasswdEntry(f[0], _to_int(f[2]), _to_int(f[3]), f[4], f[5], f[6])
    return res


def parse_group(text: str) -> dict[str, dict]:
    res = {}
    for line in text.splitlines():
        f = line.split(":")
        if len(f) >= 4 and f[0] and not line.startswith("#"):
            res[f[0]] = {"gid": _to_int(f[2]), "members": [m for m in f[3].split(",") if m]}
    return res


def parse_shadow(text: str) -> dict[str, dict]:
    res = {}
    for line in text.splitlines():
        f = line.split(":")
        if len(f) >= 2 and f[0] and not line.startswith("#"):
            res[f[0]] = {"hash": f[1], "expire": _to_int(f[7]) if len(f) > 7 and f[7] else None}
    return res


def sudo_members(passwd: dict[str, PasswdEntry], group: dict[str, dict], gname: str = "sudo") -> list[str]:
    """Non-root users that are members (supplementary or primary) of the admin group."""
    g = group.get(gname)
    if not g:
        return []
    names = set(g["members"])
    names.update(u.name for u in passwd.values() if u.gid == g["gid"])
    return sorted(n for n in names if n in passwd and passwd[n].uid != 0)


def hash_is_locked(h: str) -> bool:
    return h.startswith("!")


def hash_has_password(h: str) -> bool:
    return bool(h) and h not in ("*", "!", "!!", "!*", "x") and h.lstrip("!").startswith("$")


# ============================================================================
# 5. fstab, mount options
# ============================================================================

FSTAB_MARK = "# synofurb:"


def fstab_entries(text: str) -> list[dict]:
    res = []
    managed_tag = None
    for line in text.splitlines():
        if line.startswith(FSTAB_MARK):
            managed_tag = line[len(FSTAB_MARK):].strip()
            continue
        s = line.strip()
        if not s or s.startswith("#"):
            managed_tag = None
            continue
        f = s.split()
        if len(f) >= 4:
            res.append({"spec": f[0], "mountpoint": f[1], "fstype": f[2], "options": f[3],
                        "managed": managed_tag is not None, "tag": managed_tag})
        managed_tag = None
    return res


def render_fstab_entry(spec: str, mountpoint: str, fstype: str, options: str) -> str:
    return "%s %s %s %s 0 0" % (spec, mountpoint, fstype, options)


def fstab_add(text: str, spec: str, mountpoint: str, fstype: str, options: str, tag: str) -> str:
    """Add (or replace our own) entry for ``mountpoint``. Unrelated lines are preserved verbatim.

    Idempotent. Raises ValueError if a *foreign* (non-managed) entry already uses the mountpoint."""
    for e in fstab_entries(text):
        if e["mountpoint"] == mountpoint and not e["managed"]:
            raise ValueError("fstab already has an unmanaged entry for " + mountpoint)
    text = fstab_remove(text, mountpoint)[0]
    if text and not text.endswith("\n"):
        text += "\n"
    return text + "%s %s\n%s\n" % (FSTAB_MARK, tag, render_fstab_entry(spec, mountpoint, fstype, options))


def fstab_remove(text: str, mountpoint: str, only_managed: bool = False) -> tuple[str, bool]:
    """Remove the entry (and our marker comment) for ``mountpoint``."""
    lines = text.splitlines()
    out: list[str] = []
    removed = False
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith(FSTAB_MARK) and i + 1 < len(lines):
            f = lines[i + 1].split()
            if len(f) >= 4 and not lines[i + 1].lstrip().startswith("#") and f[1] == mountpoint:
                removed = True
                i += 2
                continue
        s = line.strip()
        f = s.split()
        if not only_managed and s and not s.startswith("#") and len(f) >= 4 and f[1] == mountpoint:
            removed = True
            i += 1
            continue
        out.append(line)
        i += 1
    res = "\n".join(out)
    if text.endswith("\n") and res:
        res += "\n"
    return res, removed


def mount_options(fstype: str, uid: int = 0, gid: int = 0, ro: bool = False, persist: bool = False) -> tuple[str, str]:
    """Return (mount type, option string) for a filesystem."""
    if fstype in ("xfs", "ext4", "btrfs"):
        mtype, opts = fstype, ["defaults", "nofail"]
    elif fstype == "vfat":
        mtype, opts = "vfat", ["defaults", "nofail", "uid=%d" % uid, "gid=%d" % gid, "umask=0022"]
    elif fstype == "exfat":
        mtype, opts = "exfat", ["defaults", "nofail", "uid=%d" % uid, "gid=%d" % gid, "umask=0022"]
    elif fstype == "ntfs":
        mtype, opts = "ntfs-3g", ["defaults", "nofail", "uid=%d" % uid, "gid=%d" % gid, "umask=0022"]
    else:
        raise ValidationError("unsupported filesystem: %s" % fstype)
    if ro:
        opts.append("ro")
    if persist:
        opts.append("x-systemd.device-timeout=30")
    return mtype, ",".join(opts)


def suggest_mountpoint(dev: dict) -> str:
    base = re.sub(r"[^A-Za-z0-9._-]+", "_", dev.get("label") or dev["kname"]).strip("._-")[:32] or dev["kname"]
    return "/mnt/" + base


def is_foreign(dev: dict) -> bool:
    """Volumes not created by us (e.g. migrated Synology md/LVM) are foreign -> mount ro by default."""
    return not (dev["type"] == "lvm" and dev["name"].startswith("vg_") and "-" in dev["name"][3:])


# ============================================================================
# 6. systemd-networkd rendering
# ============================================================================

def render_networkd(ifname: str, mode: str = "dhcp", ipv4: str | None = None, gw4: str | None = None,
                    ipv6: str | None = None, gw6: str | None = None, dns: Iterable[str] = (),
                    search: Iterable[str] = ()) -> str:
    """Render a .network file. Inputs must already be validated."""
    valid_ifname(ifname)
    head = ["# Managed by synofurb-mgmt. Manual edits may be overwritten.", "[Match]", "Name=" + ifname, "", "[Network]"]
    if mode == "dhcp":
        body = ["DHCP=yes", "IPv6AcceptRA=yes", "", "[DHCPv4]", "ClientIdentifier=mac", "", "[DHCPv6]", "UseDNS=yes"]
        return "\n".join(head + body) + "\n"
    if mode != "static":
        raise ValidationError("mode must be dhcp or static")
    if not ipv4 or not gw4:
        raise ValidationError("static mode needs an IPv4 address and gateway")
    lines = ["DHCP=no", "IPv6AcceptRA=%s" % ("no" if ipv6 else "yes"), "Address=" + ipv4]
    if ipv6:
        lines.append("Address=" + ipv6)
    lines += ["DNS=" + d for d in dns]
    if search:
        lines.append("Domains=" + " ".join(search))
    lines += ["", "[Route]", "Gateway=" + gw4]
    if ipv6 and gw6:
        lines += ["", "[Route]", "Gateway=" + gw6]
    return "\n".join(head + lines) + "\n"


def parse_networkd(text: str) -> dict:
    """Read back the settings of a file we generated (best effort)."""
    d: dict[str, Any] = {"mode": "dhcp", "ipv4": None, "ipv6": None, "gw4": None, "gw6": None, "dns": [], "search": []}
    section = ""
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("[") and s.endswith("]"):
            section = s
            continue
        if "=" not in s or s.startswith("#"):
            continue
        k, v = (x.strip() for x in s.split("=", 1))
        if k == "DHCP" and v == "no":
            d["mode"] = "static"
        elif k == "Address":
            d["ipv6" if ":" in v else "ipv4"] = v
        elif k == "DNS":
            d["dns"].append(v)
        elif k == "Domains":
            d["search"] = v.split()
        elif k == "Gateway":
            d["gw6" if ":" in v else "gw4"] = v
    return d


# ============================================================================
# 7. Volume plans (pure: build steps, never execute)
# ============================================================================

RAID_MIN = {"raid0": 2, "raid1": 2, "raid5": 3, "raid6": 4, "raid10": 4}


def check_raid_counts(level: str, n: int) -> None:
    if level == "none":
        if n < 1:
            raise ValidationError("select at least one device")
        return
    if n < RAID_MIN[level]:
        raise ValidationError("%s needs at least %d devices (got %d)" % (level, RAID_MIN[level], n))
    if level == "raid10" and n % 2:
        raise ValidationError("raid10 needs an even number of devices")


def fs_label(name: str, fs: str) -> str:
    return name[:12] if fs == "xfs" else name[:16]


def lv_path(vg: str, lv: str) -> str:
    return "/dev/%s/%s" % (vg, lv)


def mapper_name(vg: str, lv: str) -> str:
    return vg.replace("-", "--") + "-" + lv.replace("-", "--")


def plan_volume_create(name: str, devices: list[str], level: str, fs: str, mountpoint: str,
                       size_gib: int | None = None) -> list[Step]:
    """Ordered steps to build RAID(optional) -> PV -> VG -> LV -> mkfs -> fstab -> mount."""
    valid_volume_name(name)
    valid_raid_level(level)
    valid_fs_create(fs)
    valid_mountpoint(mountpoint)
    devices = [valid_device(d) for d in devices]
    if len(set(devices)) != len(devices):
        raise ValidationError("duplicate devices selected")
    check_raid_counts(level, len(devices))
    if size_gib is not None and (not isinstance(size_gib, int) or size_gib < 1):
        raise ValidationError("size must be a whole number of GiB, or empty for all")
    vg = "vg_" + name
    lvp = lv_path(vg, name)
    steps: list[Step] = []
    for d in devices:
        steps.append(Step("Wipe old signatures on %s" % d, ["wipefs", "-a", d]))
    if level != "none":
        md = "/dev/md/" + name
        steps.append(Step("Create %s array %s from %d devices" % (level, md, len(devices)),
                          ["mdadm", "--create", md, "--run", "--level=" + level, "--raid-devices=%d" % len(devices),
                           "--metadata=1.2", "--name=" + name] + devices, timeout=180))
        steps.append(Step("Wait for device nodes", ["udevadm", "settle", "--timeout=30"], ignore_error=True, timeout=45))
        steps.append(Step("Record array in /etc/mdadm/mdadm.conf", ["mdadm", "--detail", "--scan", md], kind="mdconf"))
        steps.append(Step("Update initramfs so the array assembles at boot", ["update-initramfs", "-u"], timeout=600))
        pvs = [md]
    else:
        pvs = devices
    steps.append(Step("Create LVM physical volume(s)", ["pvcreate", "--yes"] + pvs))
    steps.append(Step("Create volume group " + vg, ["vgcreate", vg] + pvs))
    size_args = ["-l", "100%FREE"] if size_gib is None else ["-L", "%dG" % size_gib]
    steps.append(Step("Create logical volume " + name, ["lvcreate", "--yes", "-n", name] + size_args + [vg]))
    steps.append(Step("Wait for device nodes", ["udevadm", "settle", "--timeout=30"], ignore_error=True, timeout=45))
    label = fs_label(name, fs)
    mk = {"xfs": ["mkfs.xfs", "-f", "-L", label, lvp], "ext4": ["mkfs.ext4", "-F", "-L", label, lvp],
          "btrfs": ["mkfs.btrfs", "-f", "-L", label, lvp]}[fs]
    steps.append(Step("Create %s filesystem (label %s)" % (fs, label), mk, timeout=900))
    mtype, opts = mount_options(fs, persist=True)
    steps.append(Step("Add fstab entry (by UUID, nofail)", None, kind="fstab",
                      meta={"device": lvp, "mountpoint": mountpoint, "fstype": mtype, "options": opts, "tag": name}))
    steps.append(Step("Create mountpoint " + mountpoint, None, kind="mkdir", meta={"path": mountpoint}))
    steps.append(Step("Reload systemd units", ["systemctl", "daemon-reload"]))
    steps.append(Step("Mount " + mountpoint, ["mount", mountpoint]))
    return steps


def plan_volume_extend(vg: str, lv: str, fs: str | None, mountpoint: str | None, add_gib: int | None) -> list[Step]:
    """Grow an LV (all free VG space when add_gib is None) and then its filesystem."""
    valid_lvm_name(vg)
    valid_lvm_name(lv)
    if add_gib is not None and (not isinstance(add_gib, int) or add_gib < 1):
        raise ValidationError("size to add must be a whole number of GiB")
    lvp = lv_path(vg, lv)
    grow = ["-l", "+100%FREE"] if add_gib is None else ["-L", "+%dG" % add_gib]
    steps = [Step("Extend logical volume %s/%s" % (vg, lv), ["lvextend"] + grow + [lvp])]
    if fs == "xfs":
        if not mountpoint:
            raise ValidationError("xfs must be mounted to grow it")
        steps.append(Step("Grow xfs filesystem", ["xfs_growfs", mountpoint]))
    elif fs == "ext4":
        steps.append(Step("Grow ext4 filesystem", ["resize2fs", lvp], timeout=600))
    elif fs == "btrfs":
        if not mountpoint:
            raise ValidationError("btrfs must be mounted to grow it")
        steps.append(Step("Grow btrfs filesystem", ["btrfs", "filesystem", "resize", "max", mountpoint]))
    else:
        raise ValidationError("cannot grow filesystem type %r" % (fs,))
    return steps


def plan_volume_delete(vg: str, lv: str, mountpoints: list[str], remove_vg: bool, pvs: list[str]) -> list[Step]:
    valid_lvm_name(vg)
    valid_lvm_name(lv)
    steps: list[Step] = []
    for mp in mountpoints:
        valid_mountpoint(mp)
        steps.append(Step("Unmount " + mp, ["umount", mp]))
        steps.append(Step("Remove fstab entry for " + mp, None, kind="fstab",
                          meta={"remove": True, "mountpoint": mp, "fstype": "", "options": ""}))
    steps.append(Step("Remove logical volume %s/%s" % (vg, lv), ["lvremove", "--yes", lv_path(vg, lv)]))
    if remove_vg:
        steps.append(Step("Remove volume group " + vg, ["vgremove", "--yes", vg]))
        steps += [Step("Remove physical volume " + valid_device(p), ["pvremove", "--yes", p]) for p in pvs]
    return steps


# ============================================================================
# 8. Authentication: shadow verification, sessions, throttling, audit log
# ============================================================================

_CRYPT_LOCK = threading.Lock()
_CRYPT_FN: Any = None
_CRYPT_TRIED = False
DUMMY_SETTING = "$6$synofurbdummy$"


def _load_crypt() -> Any:
    global _CRYPT_FN, _CRYPT_TRIED
    if _CRYPT_TRIED:
        return _CRYPT_FN
    _CRYPT_TRIED = True
    for name in (ctypes.util.find_library("crypt"), "libcrypt.so.1", "libcrypt.so.2"):
        if not name:
            continue
        try:
            lib = ctypes.CDLL(name)
            fn = lib.crypt
            fn.restype = ctypes.c_char_p
            fn.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
            _CRYPT_FN = fn
            break
        except (OSError, AttributeError):
            continue
    return _CRYPT_FN


def crypt_available() -> bool:
    return _load_crypt() is not None


def crypt_hash(password: str, setting: str) -> str | None:
    """libcrypt crypt(3) (yescrypt/sha512/...). Serialised by a lock: crypt() is not thread safe."""
    if "\x00" in password or "\x00" in setting:
        return None
    fn = _load_crypt()
    if fn is None:
        return None
    with _CRYPT_LOCK:
        try:
            res = fn(password.encode("utf-8"), setting.encode("utf-8"))
        except (ValueError, OSError):
            return None
    if not res:
        return None
    s = res.decode("ascii", "replace")
    return None if s.startswith("*") else s


def verify_shadow_hash(password: str, stored: str) -> bool:
    """Constant-time check of ``password`` against a shadow hash. Locked (!), disabled (*),
    empty and non-modular (DES) hashes never verify."""
    if not stored or stored[0] in "!*" or not stored.startswith("$") or len(password) > 1024:
        return False
    h = crypt_hash(password, stored)
    return h is not None and hmac.compare_digest(h.encode(), stored.encode())


class ShadowAuth:
    """Local, PAM-less auth: /etc/shadow + membership of the login group (sudo). uid 0 never logs in."""

    def __init__(self, cfg: Config):
        self.cfg = cfg

    def authenticate(self, username: str, password: str) -> tuple[bool, str]:
        cfg = self.cfg
        passwd = parse_passwd(read_text(cfg.p("/etc/passwd")))
        shadow = parse_shadow(read_text(cfg.p("/etc/shadow")))
        group = parse_group(read_text(cfg.p("/etc/group")))
        user, sh = passwd.get(username), shadow.get(username)
        if user is None or sh is None:
            verify_shadow_hash(password, DUMMY_SETTING)      # equalise timing
            return False, "unknown user"
        ok = verify_shadow_hash(password, sh["hash"])
        if not ok:
            return False, "bad password"
        if user.uid == 0:
            return False, "root login over the web is disabled"
        if username not in sudo_members(passwd, group, cfg.login_group):
            return False, "not a member of group " + cfg.login_group
        if sh["expire"] is not None and sh["expire"] >= 0 and sh["expire"] < int(time.time() // 86400):
            return False, "account expired"
        return True, "ok"


@dataclass
class Session:
    key: str
    user: str
    csrf: str
    ip: str
    created: float
    last_seen: float


class SessionStore:
    """Server-side sessions: 256-bit random id (only its hash is stored), idle + absolute timeout."""

    MAX_SESSIONS = 64

    def __init__(self, idle: int = 1800, absolute: int = 43200, time_fn: Callable[[], float] = time.time):
        self.idle, self.absolute, self.now = idle, absolute, time_fn
        self._s: dict[str, Session] = {}
        self._lock = threading.Lock()

    @staticmethod
    def _key(sid: str) -> str:
        return hashlib.sha256(sid.encode()).hexdigest()

    def _expired(self, s: Session, now: float) -> bool:
        return now - s.last_seen > self.idle or now - s.created > self.absolute

    def create(self, user: str, ip: str) -> tuple[str, Session]:
        sid = secrets.token_urlsafe(32)
        now = self.now()
        sess = Session(self._key(sid), user, secrets.token_urlsafe(32), ip, now, now)
        with self._lock:
            for k in [k for k, v in self._s.items() if self._expired(v, now)]:
                del self._s[k]
            while len(self._s) >= self.MAX_SESSIONS:
                del self._s[min(self._s, key=lambda k: self._s[k].last_seen)]
            self._s[sess.key] = sess
        return sid, sess

    def get(self, sid: str | None) -> Session | None:
        if not sid or len(sid) > 128:
            return None
        k, now = self._key(sid), self.now()
        with self._lock:
            s = self._s.get(k)
            if s is None:
                return None
            if self._expired(s, now):
                del self._s[k]
                return None
            s.last_seen = now
            return s

    def destroy(self, sid: str | None) -> None:
        if sid:
            with self._lock:
                self._s.pop(self._key(sid), None)

    def destroy_user(self, user: str, keep: str | None = None) -> None:
        with self._lock:
            for k in [k for k, v in self._s.items() if v.user == user and k != keep]:
                del self._s[k]


class LoginThrottle:
    """Exponential lockout: after ``max_failures`` failures a key is locked for 5 min, then 10, 20 ... (max 1 h)."""

    def __init__(self, max_failures: int = 5, base_lockout: int = 300, max_lockout: int = 3600,
                 window: int = 900, time_fn: Callable[[], float] = time.time):
        self.max_failures, self.base, self.max_lockout, self.window, self.now = (
            max_failures, base_lockout, max_lockout, window, time_fn)
        self._e: dict[str, list[float]] = {}     # key -> [count, last_fail, locked_until, strikes]
        self._lock = threading.Lock()

    def retry_after(self, *keys: str) -> int:
        now = self.now()
        with self._lock:
            left = max((self._e[k][2] - now for k in keys if k in self._e), default=0)
        return int(left) + 1 if left > 0 else 0

    def fail(self, *keys: str) -> None:
        now = self.now()
        with self._lock:
            if len(self._e) > 4096:
                for k in [k for k, v in self._e.items() if v[2] < now and now - v[1] > self.window]:
                    del self._e[k]
            for k in keys:
                e = self._e.setdefault(k, [0, now, 0.0, 0])
                if now >= e[2] and now - e[1] > self.window:
                    e[0] = 0
                    if now - e[1] > self.max_lockout * 2:
                        e[3] = 0
                e[0] += 1
                e[1] = now
                if e[0] >= self.max_failures:
                    e[3] += 1
                    e[2] = now + min(self.base * 2 ** (e[3] - 1), self.max_lockout)
                    e[0] = 0

    def success(self, *keys: str) -> None:
        with self._lock:
            for k in keys:
                self._e.pop(k, None)


_SECRET_KEYS = ("password", "passwd", "token", "csrf", "secret")


def redact(obj: Any, depth: int = 0) -> Any:
    """Copy of ``obj`` with secret-looking keys masked and long strings truncated."""
    if depth > 6:
        return "..."
    if isinstance(obj, dict):
        return {str(k): ("***" if any(s in str(k).lower() for s in _SECRET_KEYS) else redact(v, depth + 1))
                for k, v in obj.items()}
    if isinstance(obj, (list, tuple)):
        return [redact(v, depth + 1) for v in obj[:50]]
    if isinstance(obj, str) and len(obj) > 300:
        return obj[:300] + "..."
    return obj


class AuditLog:
    """JSON-lines audit log (who, ip, action, redacted args, result)."""

    def __init__(self, path: str):
        self.path = path
        self._lock = threading.Lock()

    def log(self, user: str | None, ip: str, action: str, args: Any, result: str) -> None:
        rec = {"ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "user": user, "ip": ip,
               "action": action, "args": redact(args), "result": result}
        line = json.dumps(rec, ensure_ascii=True, separators=(",", ":")) + "\n"
        try:
            with self._lock:
                os.makedirs(os.path.dirname(self.path), mode=0o750, exist_ok=True)
                fd = os.open(self.path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
                with os.fdopen(fd, "a", encoding="utf-8") as fh:
                    fh.write(line)
        except OSError as e:
            print("audit log write failed: %s" % e, file=sys.stderr)


# ============================================================================
# 9. TLS
# ============================================================================

def tls_paths(cfg: Config) -> tuple[str, str]:
    d = cfg.p(cfg.state_dir + "/tls")
    return d + "/cert.pem", d + "/key.pem"


def cert_fingerprint(certfile: str) -> str:
    der = ssl.PEM_cert_to_DER_cert(read_text(certfile))
    h = hashlib.sha256(der).hexdigest().upper()
    return ":".join(h[i:i + 2] for i in range(0, len(h), 2))


def ensure_tls(cfg: Config, runner: Runner, hostname: str, ips: Iterable[str]) -> tuple[str, str]:
    """Return (cert, key); generate a self-signed EC P-256 cert on first start."""
    cert, key = tls_paths(cfg)
    if os.path.isfile(cert) and os.path.isfile(key):
        return cert, key
    try:
        host = valid_hostname(hostname)
    except ValidationError:
        host = "synofurb"
    san = ["DNS:" + host, "DNS:" + host + ".local", "DNS:localhost"]
    for ip in list(ips) + ["127.0.0.1", "::1"]:
        try:
            s = "IP:" + str(ipaddress.ip_address(ip))
        except ValueError:
            continue
        if s not in san:
            san.append(s)
    os.makedirs(os.path.dirname(cert), mode=0o700, exist_ok=True)
    argv = ["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
            "-days", "3650", "-subj", "/CN=" + host, "-addext", "subjectAltName=" + ",".join(san),
            "-keyout", key, "-out", cert]
    old = os.umask(0o077)
    try:
        res = runner.run(argv, timeout=60)
    finally:
        os.umask(old)
    if not res.ok or not os.path.isfile(cert) or not os.path.isfile(key):
        raise RuntimeError("openssl failed to create a certificate: " + res.err.strip()[:300])
    os.chmod(key, 0o600)
    os.chmod(cert, 0o644)
    return cert, key


# ============================================================================
# 10. Application state, request plumbing, route registry
# ============================================================================

class App:
    """Everything the handlers need; built once per process (and per test)."""

    def __init__(self, cfg: Config, runner: Runner, auth: Any = None):
        self.cfg = cfg
        self.runner = runner
        self.auth = auth or ShadowAuth(cfg)
        self.sessions = SessionStore(cfg.idle_timeout, cfg.abs_timeout)
        self.throttle = LoginThrottle()
        self.audit = AuditLog(cfg.p(cfg.audit_path))
        self.op_lock = threading.RLock()
        self.cookie_name = "__Host-synofurb" if cfg.tls else "synofurb-dev"
        self.fingerprint: str | None = None
        self.power_delay = 2.0
        self.script_path = os.path.abspath(__file__)

    def read(self, path: str) -> str:
        return read_text(self.cfg.p(path))

    def run(self, argv: list[str], input: str | None = None, timeout: int = 60) -> CompletedResult:
        return self.runner.run(argv, input=input, timeout=timeout)

    def lsblk(self) -> list[dict]:
        r = self.run(["lsblk", "-J", "-b", "-o", LSBLK_COLS], timeout=20)
        return parse_lsblk(r.out) if r.ok else []

    def findmnt(self) -> list[dict]:
        r = self.run(["findmnt", "-J", "-b", "-o", "TARGET,SOURCE,FSTYPE,OPTIONS,SIZE,USED,AVAIL"], timeout=15)
        return parse_findmnt(r.out) if r.ok else []

    def lvm(self, tool: str, section: str, cols: str) -> list[dict]:
        r = self.run([tool, "--reportformat", "json", "--units", "b", "--nosuffix", "-o", cols], timeout=20)
        return parse_lvm_report(r.out, section) if r.ok else []

    def vgs(self) -> list[dict]:
        return self.lvm("vgs", "vg", "vg_name,vg_size,vg_free,pv_count,lv_count")

    def lvs(self) -> list[dict]:
        return self.lvm("lvs", "lv", "lv_name,vg_name,lv_size,lv_path,lv_attr")

    def pvs(self) -> list[dict]:
        return self.lvm("pvs", "pv", "pv_name,vg_name,pv_size,pv_free")

    def accounts(self) -> tuple[dict[str, PasswdEntry], dict[str, dict], dict[str, dict]]:
        return (parse_passwd(self.read("/etc/passwd")), parse_group(self.read("/etc/group")),
                parse_shadow(self.read("/etc/shadow")))

    def defer(self, fn: Callable[[], None], delay: float) -> None:
        t = threading.Timer(delay, fn)
        t.daemon = True
        t.start()

    def ip_addresses(self) -> list[str]:
        r = self.run(["ip", "-j", "addr"], timeout=10)
        ips = []
        for i in parse_ip_addr(r.out) if r.ok else []:
            ips += [a["addr"] for a in i["addrs"] if a["scope"] != "link" and a["addr"]]
        return ips


@dataclass
class Request:
    app: App
    method: str
    path: str
    body: dict
    session: Session | None
    ip: str
    sid: str | None = None
    audit_user: str | None = None
    audit_note: str = ""
    cookies_out: list[str] = field(default_factory=list)

    @property
    def user(self) -> str | None:
        return self.session.user if self.session else self.audit_user


@dataclass
class Route:
    fn: Callable[[Request], Any]
    auth: bool
    mutating: bool


ROUTES: dict[tuple[str, str], Route] = {}


def route(method: str, path: str, auth: bool = True, mutating: bool | None = None):
    def deco(fn: Callable[[Request], Any]):
        ROUTES[(method, path)] = Route(fn, auth, method != "GET" if mutating is None else mutating)
        return fn
    return deco


def need_confirm(body: dict, expected: str) -> None:
    c = body.get("confirm")
    if not isinstance(c, str) or not hmac.compare_digest(c.encode(), expected.encode()):
        raise ApiError(400, "confirmation does not match: type %r to confirm" % expected)


def as_list(v: Any, limit: int = 8) -> list[str]:
    if isinstance(v, str):
        v = [x for x in re.split(r"[,\s]+", v) if x]
    if not isinstance(v, list):
        return []
    return [x.strip() for x in v if isinstance(x, str) and x.strip()][:limit]


def vcall(fn: Callable[..., Any], *a: Any, **kw: Any) -> Any:
    """Run a validator, converting ValidationError into a 400."""
    try:
        return fn(*a, **kw)
    except ValidationError as e:
        raise ApiError(400, str(e))


def trunc(s: str, n: int = 2000) -> str:
    return s if len(s) <= n else s[:n] + "...[truncated]"


# --- plan execution -----------------------------------------------------------

def exec_step(app: App, step: Step) -> CompletedResult:
    cfg = app.cfg
    try:
        if step.kind == "mkdir":
            mp = valid_mountpoint(step.meta["path"])
            check_no_symlinks(cfg, mp)
            os.makedirs(cfg.p(mp), exist_ok=True)
            return CompletedResult(0, "created " + mp)
        if step.kind == "mdconf":
            r = app.run(step.argv or [], timeout=step.timeout)
            if not r.ok:
                return r
            conf = cfg.p("/etc/mdadm/mdadm.conf")
            text = read_text(conf) or "# mdadm.conf (managed in part by synofurb-mgmt)\nDEVICE partitions\n"
            have = {a["uuid"] for a in parse_mdadm_scan(text) if a["uuid"]}
            add = [a["line"] for a in parse_mdadm_scan(r.out) if a["uuid"] not in have]
            if add:
                if not text.endswith("\n"):
                    text += "\n"
                write_atomic(conf, text + "\n".join(add) + "\n")
            return CompletedResult(0, "added %d ARRAY line(s)" % len(add))
        if step.kind == "fstab":
            path = cfg.p("/etc/fstab")
            text = read_text(path)
            if os.path.exists(path):
                shutil.copy2(path, path + ".synofurb-bak")
            if step.meta.get("remove"):
                text, _ = fstab_remove(text, step.meta["mountpoint"], only_managed=bool(step.meta.get("only_managed")))
                write_atomic(path, text)
                return CompletedResult(0, "fstab entry removed")
            if step.meta.get("uuid"):
                uuid = valid_uuid(step.meta["uuid"])
            else:
                b = app.run(["blkid", "-s", "UUID", "-o", "value", valid_device(step.meta["device"])], timeout=15)
                if not b.ok or not b.out.strip():
                    return CompletedResult(1, "", "cannot read filesystem UUID: " + b.err.strip())
                uuid = valid_uuid(b.out.strip())
            write_atomic(path, fstab_add(text, "UUID=" + uuid, step.meta["mountpoint"], step.meta["fstype"],
                                         step.meta["options"], step.meta.get("tag", "volume")))
            return CompletedResult(0, "fstab entry written (UUID=%s)" % uuid)
        return app.run(step.argv or [], timeout=step.timeout)
    except (ValidationError, ValueError, OSError) as e:
        return CompletedResult(1, "", str(e))


def run_steps(app: App, steps: list[Step]) -> tuple[bool, list[dict]]:
    """Execute steps in order, stopping at the first failure (no automatic rollback)."""
    results: list[dict] = []
    for s in steps:
        r = exec_step(app, s)
        results.append({"desc": s.desc, "cmd": s.display(), "rc": r.rc, "out": trunc(r.out), "err": trunc(r.err)})
        if not r.ok and not s.ignore_error:
            return False, results
    return True, results


# ============================================================================
# 11. Routes: session, status
# ============================================================================

@route("GET", "/api/tls-fingerprint", auth=False)
def api_tls_fp(req: Request) -> dict:
    return {"ok": True, "tls": req.app.cfg.tls, "sha256": req.app.fingerprint}


@route("POST", "/api/login", auth=False)
def api_login(req: Request) -> dict:
    app = req.app
    u, p = req.body.get("username"), req.body.get("password")
    if not isinstance(u, str) or not isinstance(p, str) or not u or not p or len(u) > 64 or len(p) > 1024:
        raise ApiError(400, "username and password are required")
    req.audit_user = u[:64]
    keys = ("ip:" + req.ip, "user:" + u.lower())
    wait = app.throttle.retry_after(*keys)
    if wait:
        req.audit_note = "locked out"
        raise ApiError(429, "Too many failed attempts. Try again in %d s." % wait, {"Retry-After": str(wait)})
    ok, why = app.auth.authenticate(u, p)
    if not ok:
        app.throttle.fail(*keys)
        req.audit_note = why
        if app.cfg.fail_delay:
            time.sleep(app.cfg.fail_delay)
        raise ApiError(401, "Invalid credentials")
    app.throttle.success(*keys)
    sid, sess = app.sessions.create(u, req.ip)
    req.session = sess
    req.cookies_out.append("%s=%s; Path=/; HttpOnly; SameSite=Strict%s" % (
        app.cookie_name, sid, "; Secure" if app.cfg.tls else ""))
    return {"ok": True, "user": u, "csrf": sess.csrf}


@route("POST", "/api/logout")
def api_logout(req: Request) -> dict:
    req.app.sessions.destroy(req.sid)
    req.cookies_out.append("%s=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict%s" % (
        req.app.cookie_name, "; Secure" if req.app.cfg.tls else ""))
    return {"ok": True}


@route("GET", "/api/me")
def api_me(req: Request) -> dict:
    s = req.session
    assert s is not None
    return {"ok": True, "user": s.user, "csrf": s.csrf, "hostname": socket.gethostname(), "version": VERSION,
            "dry_run": req.app.cfg.dry_run, "net_pending": net_pending_info(req.app)}


def parse_os_release(text: str) -> dict[str, str]:
    d = {}
    for line in text.splitlines():
        if "=" in line and not line.startswith("#"):
            k, v = line.split("=", 1)
            d[k.strip()] = v.strip().strip('"')
    return d


def parse_meminfo(text: str) -> dict[str, int]:
    d = {}
    for line in text.splitlines():
        m = re.match(r"(\w+):\s+(\d+)", line)
        if m:
            d[m.group(1)] = int(m.group(2)) * 1024
    return d


def read_temps() -> list[dict]:
    res = []
    for z in sorted(glob.glob("/sys/class/thermal/thermal_zone*")):
        t = read_text(z + "/temp").strip()
        if t.lstrip("-").isdigit():
            res.append({"zone": read_text(z + "/type").strip() or os.path.basename(z), "celsius": int(t) / 1000.0})
    return res


@route("GET", "/api/status")
def api_status(req: Request) -> dict:
    app = req.app
    osr = parse_os_release(app.read("/etc/os-release") or read_text("/etc/os-release"))
    mem = parse_meminfo(read_text("/proc/meminfo"))
    try:
        uptime = float(read_text("/proc/uptime").split()[0])
    except (IndexError, ValueError):
        uptime = None
    try:
        load = list(os.getloadavg())
    except OSError:
        load = None
    try:
        sv = os.statvfs("/")
        rootfs = {"total": sv.f_blocks * sv.f_frsize, "free": sv.f_bavail * sv.f_frsize}
    except OSError:
        rootfs = None
    r = app.run(["ip", "-j", "addr"], timeout=10)
    scan = []
    if app.runner.which("mdadm"):
        scan = parse_mdadm_scan(app.run(["mdadm", "--detail", "--scan"], timeout=15).out)
    un = os.uname()
    return {
        "ok": True, "hostname": socket.gethostname(), "os": osr.get("PRETTY_NAME") or osr.get("NAME"),
        "kernel": un.release, "arch": un.machine, "uptime": uptime, "load": load,
        "memory": {"total": mem.get("MemTotal"), "available": mem.get("MemAvailable"),
                   "swap_total": mem.get("SwapTotal"), "swap_free": mem.get("SwapFree")},
        "rootfs": rootfs, "temps": read_temps(), "interfaces": parse_ip_addr(r.out) if r.ok else [],
        "disks": app.lsblk(), "md": {"arrays": parse_mdstat(read_text("/proc/mdstat")), "scan": scan},
        "lvm": {"vgs": app.vgs(), "lvs": app.lvs(), "pvs": app.pvs()},
        "mounts": [m for m in app.findmnt() if m["real"]],
    }


def parse_smart(text: str) -> dict:
    try:
        j = json.loads(text)
    except ValueError:
        return {"passed": None, "temp": None, "note": "unreadable smartctl output"}
    passed = (j.get("smart_status") or {}).get("passed")
    msgs = [m.get("string", "") for m in (j.get("smartctl", {}).get("messages") or [])]
    return {"passed": passed, "temp": (j.get("temperature") or {}).get("current"),
            "hours": (j.get("power_on_time") or {}).get("hours"), "note": "; ".join(msgs)[:200]}


@route("GET", "/api/smart")
def api_smart(req: Request) -> dict:
    app = req.app
    if not app.runner.which("smartctl"):
        return {"ok": True, "available": False, "disks": []}
    out = []
    for d in app.lsblk():
        if d["type"] != "disk" or d["kname"].startswith(("loop", "zram", "sr", "ram", "nbd")):
            continue
        r = app.run(["smartctl", "-H", "-j", d["path"]], timeout=20)     # rc is a bitmask; parse regardless
        out.append(dict(parse_smart(r.out), device=d["path"], model=d["model"]))
    return {"ok": True, "available": True, "disks": out}


# ============================================================================
# 12. Routes: users
# ============================================================================

def authorized_keys_path(cfg: Config, home: str) -> str | None:
    if not (home == "/root" or re.fullmatch(r"/home/[A-Za-z0-9_.-]+", home)):
        return None
    return cfg.p(home + "/.ssh/authorized_keys")


def count_keys(path: str | None) -> int:
    if not path:
        return 0
    return sum(1 for ln in read_text(path).splitlines() if ln.strip() and not ln.lstrip().startswith("#"))


def install_ssh_key(app: App, u: PasswdEntry | None, name: str, key: str) -> None:
    """Append a public key to ~/.ssh/authorized_keys (0700/0600, owned by the user, no symlinks)."""
    home = u.home if u else "/home/" + name
    akp = authorized_keys_path(app.cfg, home)
    if akp is None:
        raise ApiError(400, "refusing to write keys into unusual home directory %s" % home)
    sshdir = os.path.dirname(akp)
    if os.path.islink(sshdir) or os.path.islink(akp):
        raise ApiError(400, ".ssh or authorized_keys is a symlink; refusing")
    os.makedirs(sshdir, mode=0o700, exist_ok=True)
    uid, gid = (u.uid, u.gid) if u else (None, None)
    if uid is not None:
        try:
            os.chown(sshdir, uid, gid, follow_symlinks=False)
            os.chmod(sshdir, 0o700)
        except OSError:
            pass
    text = read_text(akp)
    ident = " ".join(key.split()[:2])
    if any(" ".join(ln.split()[:2]) == ident for ln in text.splitlines() if ln.strip()):
        return
    if text and not text.endswith("\n"):
        text += "\n"
    write_atomic(akp, text + key + "\n", mode=0o600, uid=uid, gid=gid)


@route("GET", "/api/users")
def api_users(req: Request) -> dict:
    app = req.app
    passwd, group, shadow = app.accounts()
    gid_name = {g["gid"]: n for n, g in group.items()}
    admins = set(sudo_members(passwd, group, app.cfg.login_group))
    users = []
    for u in sorted(passwd.values(), key=lambda x: x.uid):
        if not (u.uid == 0 or 1000 <= u.uid < 60000):
            continue
        h = shadow.get(u.name, {}).get("hash", "")
        gs = sorted({n for n, g in group.items() if u.name in g["members"]} | {gid_name.get(u.gid, str(u.gid))})
        users.append({"name": u.name, "uid": u.uid, "home": u.home, "shell": u.shell, "groups": gs,
                      "locked": hash_is_locked(h), "has_password": hash_has_password(h),
                      "sudo": u.name in admins, "root": u.uid == 0,
                      "ssh_keys": count_keys(authorized_keys_path(app.cfg, u.home)),
                      "web_login": u.name in admins and hash_has_password(h) and not hash_is_locked(h)})
    return {"ok": True, "users": users, "groups": sorted((set(group) | set(EXTRA_GROUPS_ALWAYS)) - {"root"}),
            "current": req.user}


def norm_groups(v: Any, existing: set[str]) -> list[str]:
    allowed = (existing | set(EXTRA_GROUPS_ALWAYS)) - {"root"}
    res = []
    for g in as_list(v, 16):
        if not valid_group_name_ok(g):
            raise ApiError(400, "invalid group name: %r" % g[:40])
        if g not in allowed:
            raise ApiError(400, "group %r is not allowed or does not exist" % g)
        if g not in res:
            res.append(g)
    return res


def chpasswd(app: App, name: str, password: str) -> CompletedResult:
    return app.run(["chpasswd"], input="%s:%s\n" % (name, password), timeout=30)     # password via stdin only


@route("POST", "/api/users/create")
def api_users_create(req: Request) -> dict:
    app, b = req.app, req.body
    name, pw = vcall(valid_username, b.get("username")), vcall(valid_password, b.get("password"))
    passwd, group, _ = app.accounts()
    if name in passwd:
        raise ApiError(409, "user %s already exists" % name)
    groups = norm_groups(b.get("groups"), set(group))
    if as_bool(b.get("sudo")) and "sudo" not in groups:
        groups.append("sudo")
    key = vcall(valid_ssh_pubkey, b["ssh_key"]) if b.get("ssh_key") else None
    argv = ["useradd", "--create-home", "--shell", "/bin/bash", "--user-group"]
    if groups:
        argv += ["--groups", ",".join(groups)]
    r = app.run(argv + [name], timeout=30)
    if not r.ok:
        raise ApiError(500, "useradd failed: " + r.err.strip()[:300])
    r = chpasswd(app, name, pw)
    if not r.ok:
        app.run(["userdel", "--remove", name], timeout=30)
        raise ApiError(500, "setting the password failed (user rolled back): " + r.err.strip()[:300])
    if key:
        install_ssh_key(app, app.accounts()[0].get(name), name, key)
    return {"ok": True, "user": name, "groups": groups}


def _managed_target(app: App, name: Any) -> tuple[str, PasswdEntry]:
    name = vcall(valid_username, name)
    u = app.accounts()[0].get(name)
    if u is None:
        raise ApiError(404, "no such user")
    if u.uid == 0 or not 1000 <= u.uid < 60000:
        raise ApiError(400, "refusing to touch system accounts or root")
    return name, u


@route("POST", "/api/users/delete")
def api_users_delete(req: Request) -> dict:
    app, b = req.app, req.body
    name, u = _managed_target(app, b.get("username"))
    need_confirm(b, name)
    if name == req.user:
        raise ApiError(400, "you cannot delete the account you are logged in with")
    passwd, group, _ = app.accounts()
    if sudo_members(passwd, group, app.cfg.login_group) == [name]:
        raise ApiError(400, "refusing to delete the last member of group %s" % app.cfg.login_group)
    argv = ["userdel"] + (["--remove"] if as_bool(b.get("remove_home")) else []) + [name]
    r = app.run(argv, timeout=60)
    if not r.ok:
        raise ApiError(500, "userdel failed: " + r.err.strip()[:300])
    app.sessions.destroy_user(name)
    return {"ok": True}


@route("POST", "/api/users/password")
def api_users_password(req: Request) -> dict:
    app, b = req.app, req.body
    name, _ = _managed_target(app, b.get("username"))
    pw = vcall(valid_password, b.get("password"))
    r = chpasswd(app, name, pw)
    if not r.ok:
        raise ApiError(500, "chpasswd failed: " + r.err.strip()[:300])
    app.sessions.destroy_user(name, keep=req.session.key if req.session and name == req.user else None)
    return {"ok": True}


def _set_lock(req: Request, lock: bool) -> dict:
    app = req.app
    name, _ = _managed_target(app, req.body.get("username"))
    if lock:
        if name == req.user:
            raise ApiError(400, "you cannot lock your own account")
        passwd, group, shadow = app.accounts()
        others = [n for n in sudo_members(passwd, group, app.cfg.login_group)
                  if n != name and not hash_is_locked(shadow.get(n, {}).get("hash", "!"))]
        if name in sudo_members(passwd, group, app.cfg.login_group) and not others:
            raise ApiError(400, "refusing to lock the last unlocked administrator")
    r = app.run(["usermod", "-L" if lock else "-U", name], timeout=30)
    if not r.ok:
        raise ApiError(500, "usermod failed: " + r.err.strip()[:300])
    if lock:
        app.sessions.destroy_user(name)
    return {"ok": True}


@route("POST", "/api/users/lock")
def api_users_lock(req: Request) -> dict:
    return _set_lock(req, True)


@route("POST", "/api/users/unlock")
def api_users_unlock(req: Request) -> dict:
    return _set_lock(req, False)


@route("POST", "/api/users/ssh-key")
def api_users_sshkey(req: Request) -> dict:
    app = req.app
    name, u = _managed_target(app, req.body.get("username"))
    install_ssh_key(app, u, name, vcall(valid_ssh_pubkey, req.body.get("ssh_key")))
    return {"ok": True}


# ============================================================================
# 13. Routes: network (safe apply with automatic rollback)
# ============================================================================

ROLLBACK_UNIT = "synofurb-net-rollback"
DISABLED_SUFFIX = ".disabled-by-synofurb"


def net_file(cfg: Config, ifname: str) -> str:
    return cfg.p("/etc/systemd/network/10-synofurb-%s.network" % ifname)


def pending_path(cfg: Config) -> str:
    return cfg.p(cfg.state_dir + "/net-pending.json")


def load_pending(cfg: Config) -> dict | None:
    try:
        with open(pending_path(cfg), encoding="utf-8") as fh:
            d = json.load(fh)
        return d if isinstance(d, dict) and "ifname" in d else None
    except (OSError, ValueError):
        return None


def net_pending_info(app: App) -> dict | None:
    p = load_pending(app.cfg)
    if not p:
        return None
    return {"ifname": p["ifname"], "deadline": p["deadline"], "seconds_left": max(0, int(p["deadline"] - time.time()))}


def match_info(text: str) -> tuple[list[str] | None, bool]:
    """(Name= patterns of [Match] or None, whether other restricting keys are used)."""
    names: list[str] | None = None
    other = False
    in_match = False
    for ln in text.splitlines():
        s = ln.strip()
        if s.startswith("["):
            in_match = s == "[Match]"
        elif in_match and "=" in s and not s.startswith("#"):
            k, v = (x.strip() for x in s.split("=", 1))
            if k == "Name":
                names = (names or []) + v.split()
            elif k in ("MACAddress", "PermanentMACAddress", "Driver", "Path", "Property", "Host", "Virtualization"):
                other = True
    return names, other


def network_conflicts(cfg: Config, ifname: str) -> list[str]:
    """Earlier-sorting .network files that would win over ours for this interface."""
    ours = os.path.basename(net_file(cfg, ifname))
    res = []
    for f in sorted(glob.glob(cfg.p("/etc/systemd/network") + "/*.network")):
        base = os.path.basename(f)
        if base >= ours or base.startswith("10-synofurb-"):
            continue
        names, other = match_info(read_text(f))
        if other:
            continue
        pos = [n for n in (names or []) if not n.startswith("!")]
        neg = [n[1:] for n in (names or []) if n.startswith("!")]
        if (not pos or any(fnmatch.fnmatchcase(ifname, p) for p in pos)) and \
                not any(fnmatch.fnmatchcase(ifname, n) for n in neg):
            res.append(f)
    return res


@route("GET", "/api/network")
def api_network(req: Request) -> dict:
    app = req.app
    r = app.run(["ip", "-j", "addr"], timeout=10)
    ifs = []
    for i in parse_ip_addr(r.out) if r.ok else []:
        if i["ifname"] == "lo" or not _IFNAME_RE.fullmatch(i["ifname"] or ""):
            continue
        f = net_file(app.cfg, i["ifname"])
        text = read_text(f) if os.path.isfile(f) else ""
        i["managed"] = bool(text)
        i["config"] = parse_networkd(text) if text else {"mode": "dhcp"}
        i["shadowed_by"] = [os.path.basename(x) for x in network_conflicts(app.cfg, i["ifname"])]
        ifs.append(i)
    return {"ok": True, "interfaces": ifs, "hostname": socket.gethostname(), "pending": net_pending_info(app),
            "rollback_seconds": app.cfg.rollback_seconds}


def restore_network_files(cfg: Config, p: dict) -> None:
    f = net_file(cfg, p["ifname"])
    if p.get("backup") and os.path.isfile(p["backup"]):
        shutil.copy2(p["backup"], f)
    elif os.path.exists(f):
        os.unlink(f)
    for src, dst in p.get("moved", []):
        if os.path.exists(dst):
            os.replace(dst, src)


def rollback_network(app: App) -> bool:
    """Restore the pre-change config. Used by the timer, the API and startup recovery."""
    p = load_pending(app.cfg)
    if not p:
        return False
    restore_network_files(app.cfg, p)
    try:
        os.unlink(pending_path(app.cfg))
    except OSError:
        pass
    app.run(["networkctl", "reload"], timeout=20)
    app.run(["networkctl", "reconfigure", p["ifname"]], timeout=20)
    return True


@route("POST", "/api/network/apply")
def api_network_apply(req: Request) -> dict:
    app, b, cfg = req.app, req.body, req.app.cfg
    ifname = vcall(valid_ifname, b.get("ifname"))
    known = [i["ifname"] for i in parse_ip_addr(app.run(["ip", "-j", "addr"], timeout=10).out)]
    if ifname == "lo" or ifname not in known:
        raise ApiError(400, "unknown interface")
    mode = b.get("mode", "dhcp")
    kw: dict[str, Any] = {"mode": mode}
    if mode == "static":
        kw["ipv4"] = vcall(valid_ipv4_cidr, b.get("ipv4"))
        kw["gw4"] = vcall(valid_gateway, b.get("gateway4"), 4, kw["ipv4"])
        if b.get("ipv6"):
            kw["ipv6"] = vcall(valid_ipv6_cidr, b["ipv6"])
            if b.get("gateway6"):
                kw["gw6"] = vcall(valid_gateway, b["gateway6"], 6, kw["ipv6"])
        kw["dns"] = [vcall(valid_dns, d) for d in as_list(b.get("dns"))]
        kw["search"] = [vcall(valid_search_domain, d) for d in as_list(b.get("search"))]
    elif mode != "dhcp":
        raise ApiError(400, "mode must be dhcp or static")
    content = vcall(render_networkd, ifname, **kw)

    if net_pending_info(app):
        raise ApiError(409, "a previous network change is still waiting for confirmation")
    f = net_file(cfg, ifname)
    bdir = cfg.p(cfg.state_dir + "/net-backup")
    os.makedirs(bdir, mode=0o700, exist_ok=True)
    backup = None
    if os.path.isfile(f):
        backup = os.path.join(bdir, os.path.basename(f) + ".bak")
        shutil.copy2(f, backup)
    moved = []
    for c in network_conflicts(cfg, ifname):         # earlier files would shadow ours: park them (restored on rollback)
        os.replace(c, c + DISABLED_SUFFIX)
        moved.append([c, c + DISABLED_SUFFIX])
    pend = {"ifname": ifname, "backup": backup, "moved": moved, "deadline": time.time() + cfg.rollback_seconds}
    write_atomic(f, content)
    write_atomic(pending_path(cfg), json.dumps(pend), mode=0o600)
    # Arm the rollback timer BEFORE touching the link: if we lose the connection it still fires.
    app.run(["systemctl", "stop", ROLLBACK_UNIT + ".timer", ROLLBACK_UNIT + ".service"], timeout=15)
    app.run(["systemctl", "reset-failed", ROLLBACK_UNIT + ".service"], timeout=15)
    cmd = [sys.executable or "/usr/bin/python3", app.script_path, "--net-rollback"]
    if cfg.root:
        cmd += ["--root", cfg.root]
    sr = app.run(["systemd-run", "--on-active=%d" % cfg.rollback_seconds, "--unit=" + ROLLBACK_UNIT,
                  "--description=synofurb network auto-rollback"] + cmd, timeout=15)
    if not sr.ok:
        rollback_network(app)
        raise ApiError(500, "could not arm the rollback timer, change reverted: " + sr.err.strip()[:200])
    app.run(["networkctl", "reload"], timeout=20)
    rc = app.run(["networkctl", "reconfigure", ifname], timeout=20)
    return {"ok": True, "pending": net_pending_info(app), "reconfigure_rc": rc.rc, "config": content,
            "message": "Applied. Confirm within %d s from the NEW address or it is rolled back." % cfg.rollback_seconds}


@route("POST", "/api/network/confirm")
def api_network_confirm(req: Request) -> dict:
    app = req.app
    if not load_pending(app.cfg):
        raise ApiError(409, "no network change is waiting for confirmation")
    app.run(["systemctl", "stop", ROLLBACK_UNIT + ".timer"], timeout=15)
    if app.run(["systemctl", "is-active", ROLLBACK_UNIT + ".timer"], timeout=10).out.strip() == "active":
        raise ApiError(500, "could not stop the rollback timer; the change will be reverted")
    try:
        os.unlink(pending_path(app.cfg))
    except OSError:
        pass
    return {"ok": True}


@route("POST", "/api/network/rollback")
def api_network_rollback(req: Request) -> dict:
    app = req.app
    app.run(["systemctl", "stop", ROLLBACK_UNIT + ".timer"], timeout=15)
    if not rollback_network(app):
        raise ApiError(409, "nothing to roll back")
    return {"ok": True}


@route("POST", "/api/hostname")
def api_hostname(req: Request) -> dict:
    name = vcall(valid_hostname, req.body.get("hostname"))
    r = req.app.run(["hostnamectl", "set-hostname", name], timeout=20)
    if not r.ok:
        raise ApiError(500, "hostnamectl failed: " + r.err.strip()[:300])
    return {"ok": True, "hostname": name}


# ============================================================================
# 14. Routes: volumes (RAID / LVM / filesystems)
# ============================================================================

def estimate_usable(level: str, sizes: list[int]) -> int:
    if not sizes:
        return 0
    m, n = min(sizes), len(sizes)
    return {"none": sum(sizes), "raid0": m * n, "raid1": m, "raid5": m * (n - 1),
            "raid6": m * (n - 2), "raid10": m * n // 2}[level]


def build_volume_plan(app: App, b: dict) -> dict:
    """Validate a create request against live system state and build (not run) the plan."""
    name = vcall(valid_volume_name, b.get("name"))
    level = vcall(valid_raid_level, b.get("level", "none"))
    fs = vcall(valid_fs_create, b.get("fs", "xfs"))
    mp = vcall(valid_mountpoint, b.get("mountpoint") or "/srv/" + name)
    vcall(check_no_symlinks, app.cfg, mp)
    raw = b.get("size_gib")
    size: int | None = None
    if raw not in (None, "", "all"):
        try:
            size = int(raw)
        except (TypeError, ValueError):
            raise ApiError(400, "size_gib must be a whole number or empty")
    if not isinstance(b.get("devices"), list):
        raise ApiError(400, "devices must be a list")
    devices = [vcall(valid_device, d) for d in b["devices"]]
    devs = app.lsblk()
    by_k = {d["kname"]: d for d in devs}
    by_path = {d["path"]: d for d in devs}
    sizes = []
    for d in devices:
        dev = by_path.get(d)
        if dev is None:
            raise ApiError(400, "%s: no such block device" % d)
        why = device_unusable_reason(dev, by_k)
        if why:
            raise ApiError(400, "%s cannot be used: %s" % (d, why))
        sizes.append(dev["size"])
    vg = "vg_" + name
    if any(v["vg_name"] == vg for v in app.vgs()):
        raise ApiError(409, "volume group %s already exists" % vg)
    if any(m["target"] == mp for m in app.findmnt()) or any(e["mountpoint"] == mp for e in fstab_entries(app.read("/etc/fstab"))):
        raise ApiError(409, "%s is already mounted or listed in fstab" % mp)
    steps = vcall(plan_volume_create, name, devices, level, fs, mp, size)
    usable = estimate_usable(level, sizes)
    if size is not None and size * GIB > usable:
        raise ApiError(400, "requested %d GiB exceeds the usable capacity (%d GiB)" % (size, usable // GIB))
    warnings = ["ALL DATA ON THE SELECTED DEVICES WILL BE DESTROYED."]
    if sizes and max(sizes) > min(sizes) * 1.1 and level != "none":
        warnings.append("Devices differ in size; only the smallest size is used per member.")
    if level == "none" and len(devices) > 1:
        warnings.append("No RAID: the volume spans the devices without redundancy; losing one loses everything.")
    if level == "raid0":
        warnings.append("RAID0 has no redundancy.")
    return {"name": name, "vg": vg, "level": level, "fs": fs, "mountpoint": mp, "usable_bytes": usable,
            "warnings": warnings, "steps": steps}


@route("POST", "/api/volumes/plan", mutating=False)
def api_volumes_plan(req: Request) -> dict:
    p = build_volume_plan(req.app, req.body)
    p["steps"] = [s.to_dict() for s in p["steps"]]
    p["confirm"] = p["name"]
    return dict(p, ok=True)


@route("POST", "/api/volumes/create")
def api_volumes_create(req: Request) -> dict:
    name = vcall(valid_volume_name, req.body.get("name"))
    need_confirm(req.body, name)
    plan = build_volume_plan(req.app, req.body)          # always re-derived server side
    ok, results = run_steps(req.app, plan["steps"])
    if not ok:
        return 500, {"ok": False, "error": "a step failed; nothing was rolled back automatically", "results": results}
    return {"ok": True, "results": results, "mountpoint": plan["mountpoint"]}


def volume_view(app: App) -> dict:
    devs = app.lsblk()
    by_name = {d["name"]: d for d in devs}
    vgs, lvs, pvs = app.vgs(), app.lvs(), app.pvs()
    out_lvs = []
    for lv in lvs:
        d = by_name.get(mapper_name(lv["vg_name"], lv["lv_name"]))
        out_lvs.append(dict(lv, fstype=d["fstype"] if d else None, mountpoints=d["mountpoints"] if d else [],
                            label=d["label"] if d else None))
    free, excl = candidate_devices(devs)
    return {"ok": True, "vgs": vgs, "lvs": out_lvs, "pvs": pvs, "candidates": free, "excluded": excl,
            "md": parse_mdstat(read_text("/proc/mdstat")), "raid_levels": list(RAID_LEVELS),
            "filesystems": list(CREATE_FS)}


@route("GET", "/api/volumes")
def api_volumes(req: Request) -> dict:
    return volume_view(req.app)


def _find_lv(app: App, vg: Any, lv: Any) -> tuple[str, str, dict | None]:
    vg, lv = vcall(valid_lvm_name, vg), vcall(valid_lvm_name, lv)
    if not any(x["vg_name"] == vg and x["lv_name"] == lv for x in app.lvs()):
        raise ApiError(404, "no such logical volume %s/%s" % (vg, lv))
    return vg, lv, next((d for d in app.lsblk() if d["name"] == mapper_name(vg, lv)), None)


@route("POST", "/api/volumes/extend")
def api_volumes_extend(req: Request) -> dict:
    app, b = req.app, req.body
    vg, lv, dev = _find_lv(app, b.get("vg"), b.get("lv"))
    add = None if b.get("add_gib") in (None, "", "all") else b["add_gib"]
    if add is not None:
        try:
            add = int(add)
        except (TypeError, ValueError):
            raise ApiError(400, "add_gib must be a whole number or empty")
    free = next((v["vg_free"] for v in app.vgs() if v["vg_name"] == vg), 0)
    if free <= 0 or (add is not None and add * GIB > free):
        raise ApiError(400, "not enough free space in %s (%d GiB free)" % (vg, free // GIB))
    steps = vcall(plan_volume_extend, vg, lv, dev["fstype"] if dev else None,
                  dev["mountpoints"][0] if dev and dev["mountpoints"] else None, add)
    ok, results = run_steps(app, steps)
    return (200 if ok else 500), {"ok": ok, "results": results, **({} if ok else {"error": "a step failed"})}


@route("POST", "/api/volumes/delete")
def api_volumes_delete(req: Request) -> dict:
    app, b = req.app, req.body
    vg, lv, dev = _find_lv(app, b.get("vg"), b.get("lv"))
    need_confirm(b, lv)
    if vg == "vg_sys" or lv in ("root", "swap"):
        raise ApiError(400, "refusing to delete system volumes")
    mps = list(dev["mountpoints"]) if dev else []
    if any(m in PROTECTED_MOUNTS for m in mps):
        raise ApiError(400, "volume holds a system filesystem")
    for m in mps:
        vcall(valid_mountpoint, m)
    remove_vg = as_bool(b.get("remove_vg"))
    pv_names = [p["pv_name"] for p in app.pvs() if p["vg_name"] == vg]
    steps = vcall(plan_volume_delete, vg, lv, mps, remove_vg, pv_names)
    ok, results = run_steps(app, steps)
    return (200 if ok else 500), {"ok": ok, "results": results, **({} if ok else {"error": "a step failed"})}


# ============================================================================
# 15. Routes: mounts
# ============================================================================

def mountable_devices(app: App) -> list[dict]:
    res = []
    for d in app.lsblk():
        if d["fstype"] in MOUNT_FS and not d["mountpoints"] and d["type"] in ("part", "disk", "lvm", "md", "crypt", "loop"):
            f = is_foreign(d)
            res.append({"path": d["path"], "fstype": d["fstype"], "label": d["label"], "uuid": d["uuid"],
                        "size": d["size"], "type": d["type"], "model": d["model"], "tran": d["tran"],
                        "foreign": f, "ro_default": f, "suggested": suggest_mountpoint(d)})
    return res


@route("GET", "/api/mounts")
def api_mounts(req: Request) -> dict:
    app = req.app
    return {"ok": True, "mounts": [m for m in app.findmnt() if m["real"]],
            "fstab": fstab_entries(app.read("/etc/fstab")), "devices": mountable_devices(app)}


@route("POST", "/api/mounts/mount")
def api_mounts_mount(req: Request) -> dict:
    app, b = req.app, req.body
    path = vcall(valid_device, b.get("device"))
    dev = next((d for d in app.lsblk() if d["path"] == path), None)
    if dev is None:
        raise ApiError(404, "no such block device")
    if dev["fstype"] not in MOUNT_FS:
        raise ApiError(400, "unsupported or missing filesystem: %s" % dev["fstype"])
    if dev["mountpoints"]:
        raise ApiError(409, "device is already mounted at " + dev["mountpoints"][0])
    mp = vcall(valid_mountpoint, b.get("mountpoint"))
    vcall(check_no_symlinks, app.cfg, mp)
    if any(m["target"] == mp for m in app.findmnt()):
        raise ApiError(409, mp + " is already a mountpoint")
    owner = vcall(valid_username, b.get("owner") or req.user)
    u = app.accounts()[0].get(owner)
    if u is None:
        raise ApiError(400, "unknown owner %s" % owner)
    ro, persist = as_bool(b.get("ro")), as_bool(b.get("persist"))
    steps = [Step("Create mountpoint " + mp, kind="mkdir", meta={"path": mp})]
    if persist:
        if not dev["uuid"]:
            raise ApiError(400, "device has no UUID; cannot persist in fstab")
        if any(e["mountpoint"] == mp and not e["managed"] for e in fstab_entries(app.read("/etc/fstab"))):
            raise ApiError(409, "fstab already has an unmanaged entry for " + mp)
        mtype, opts = mount_options(dev["fstype"], u.uid, u.gid, ro, persist=True)
        steps += [Step("Add fstab entry (UUID, nofail)", kind="fstab",
                       meta={"uuid": vcall(valid_uuid, dev["uuid"]), "device": path, "mountpoint": mp,
                             "fstype": mtype, "options": opts, "tag": "mount"}),
                  Step("Reload systemd units", ["systemctl", "daemon-reload"]),
                  Step("Mount " + mp, ["mount", mp])]
    else:
        mtype, opts = mount_options(dev["fstype"], u.uid, u.gid, ro)
        steps.append(Step("Mount %s on %s" % (path, mp), ["mount", "-t", mtype, "-o", opts, path, mp]))
    ok, results = run_steps(app, steps)
    return (200 if ok else 500), {"ok": ok, "results": results, **({} if ok else {"error": "mount failed"})}


@route("POST", "/api/mounts/unmount")
def api_mounts_unmount(req: Request) -> dict:
    app, b = req.app, req.body
    mp = vcall(valid_mountpoint, b.get("mountpoint"))
    need_confirm(b, mp)
    if not any(m["target"] == mp for m in app.findmnt()):
        raise ApiError(404, mp + " is not mounted")
    steps = [Step("Unmount " + mp, ["umount", mp], timeout=120)]
    if as_bool(b.get("remove_fstab")):
        steps.append(Step("Remove managed fstab entry", kind="fstab",
                          meta={"remove": True, "only_managed": True, "mountpoint": mp}))
    ok, results = run_steps(app, steps)
    return (200 if ok else 500), {"ok": ok, "results": results, **({} if ok else {"error": "unmount failed (busy?)"})}


@route("POST", "/api/mounts/activate")
def api_mounts_activate(req: Request) -> dict:
    app = req.app
    steps = [Step("Assemble RAID arrays", ["mdadm", "--assemble", "--scan"], ignore_error=True, timeout=120),
             Step("Scan LVM volume groups", ["vgscan"], ignore_error=True, timeout=60),
             Step("Activate logical volumes", ["vgchange", "-ay"], ignore_error=True, timeout=120),
             Step("Wait for device nodes", ["udevadm", "settle", "--timeout=30"], ignore_error=True, timeout=45)]
    _, results = run_steps(app, steps)
    return {"ok": True, "results": results, "devices": mountable_devices(app)}


# ============================================================================
# 16. Routes: services / system
# ============================================================================

@route("GET", "/api/services")
def api_services(req: Request) -> dict:
    app = req.app
    keys = []
    for f in sorted(glob.glob(app.cfg.p("/etc/ssh/ssh_host_*_key.pub"))):
        r = app.run(["ssh-keygen", "-lf", f], timeout=10)
        if r.ok and r.out.strip():
            parts = r.out.split()
            keys.append({"file": os.path.basename(f), "bits": parts[0], "fingerprint": parts[1] if len(parts) > 1 else "",
                         "type": parts[-1].strip("()")})
    gt = app.run(["systemctl", "list-units", "serial-getty@*", "--all", "--no-legend", "--plain", "--no-pager"], timeout=15)
    getty = []
    for ln in gt.out.splitlines():
        p = ln.split(None, 4)
        if len(p) >= 4 and p[0].startswith("serial-getty@"):
            getty.append({"unit": p[0], "load": p[1], "active": p[2], "sub": p[3]})
    return {"ok": True, "version": VERSION,
            "sshd": {"enabled": app.run(["systemctl", "is-enabled", "ssh"], timeout=10).out.strip() or "unknown",
                     "active": app.run(["systemctl", "is-active", "ssh"], timeout=10).out.strip() or "unknown"},
            "host_keys": keys, "serial_getty": getty, "restartable": list(RESTARTABLE_UNITS)}


@route("POST", "/api/services/restart")
def api_services_restart(req: Request) -> dict:
    unit = req.body.get("unit")
    if unit not in RESTARTABLE_UNITS:
        raise ApiError(400, "unit not in the restart allow-list")
    r = req.app.run(["systemctl", "restart", unit], timeout=60)
    if not r.ok:
        raise ApiError(500, "restart failed: " + r.err.strip()[:300])
    return {"ok": True}


@route("POST", "/api/system/power")
def api_system_power(req: Request) -> dict:
    app, action = req.app, req.body.get("action")
    if action not in ("reboot", "poweroff"):
        raise ApiError(400, "action must be reboot or poweroff")
    need_confirm(req.body, socket.gethostname())
    app.defer(lambda: app.run(["systemctl", action], timeout=30), app.power_delay)
    return {"ok": True, "message": "%s scheduled" % action}


# ============================================================================
# 17. HTTP layer
# ============================================================================

def client_ip(addr: str) -> str:
    return addr[7:] if addr.startswith("::ffff:") else addr


def origin_ok(origin: str, host: str) -> bool:
    if origin == "null":
        return False
    return urllib.parse.urlsplit(origin).netloc.lower() == host.lower()


def get_cookie(header: str | None, name: str) -> str | None:
    for part in (header or "").split(";"):
        k, _, v = part.strip().partition("=")
        if k == name:
            return v
    return None


class Handler(BaseHTTPRequestHandler):
    server_version = "synofurb-mgmt"
    protocol_version = "HTTP/1.0"
    timeout = 30

    def version_string(self) -> str:
        return self.server_version

    def log_message(self, fmt: str, *args: Any) -> None:       # keep the journal quiet; audit log has the facts
        pass

    # -- plumbing ------------------------------------------------------
    @property
    def app(self) -> App:
        return self.server.app          # type: ignore[attr-defined]

    def _security_headers(self, csp: str) -> None:
        self.send_header("Content-Security-Policy", csp)
        self.send_header("X-Frame-Options", "DENY")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        self.send_header("Permissions-Policy", "camera=(), microphone=(), geolocation=()")
        self.send_header("Cache-Control", "no-store")
        if self.app.cfg.tls:
            self.send_header("Strict-Transport-Security", "max-age=31536000")

    def _send(self, status: int, body: bytes, ctype: str, csp: str, extra: Iterable[tuple[str, str]] = ()) -> None:
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self._security_headers(csp)
        for k, v in extra:
            self.send_header(k, v)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _json(self, status: int, obj: dict, extra: Iterable[tuple[str, str]] = ()) -> None:
        self._send(status, json.dumps(obj).encode(), "application/json; charset=utf-8",
                   "default-src 'none'; frame-ancestors 'none'", extra)

    def _serve_ui(self) -> None:
        nonce = base64.b64encode(secrets.token_bytes(16)).decode()
        html = INDEX_HTML.replace("__NONCE__", nonce).replace("__VERSION__", VERSION)
        csp = ("default-src 'self'; script-src 'nonce-%s'; style-src 'nonce-%s'; img-src 'self' data:; "
               "connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'" % (nonce, nonce))
        self._send(200, html.encode(), "text/html; charset=utf-8", csp)

    def _drain(self, n: int) -> None:
        left = n
        try:
            while left > 0:
                chunk = self.rfile.read(min(65536, left))
                if not chunk:
                    break
                left -= len(chunk)
        except OSError:
            pass

    def _read_json(self) -> dict:
        if self.headers.get("Transfer-Encoding"):
            raise ApiError(400, "chunked bodies are not supported")
        cl = self.headers.get("Content-Length")
        if cl is None:
            raise ApiError(411, "Content-Length required")
        if not cl.isdigit():
            raise ApiError(400, "bad Content-Length")
        n = int(cl)
        if n > MAX_BODY:
            if n <= 1 << 20:
                self._drain(n)
            raise ApiError(413, "request body too large (max %d bytes)" % MAX_BODY)
        if (self.headers.get("Content-Type") or "").split(";")[0].strip().lower() != "application/json":
            raise ApiError(415, "Content-Type must be application/json")
        raw = self.rfile.read(n) if n else b"{}"
        try:
            obj = json.loads(raw.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            raise ApiError(400, "invalid JSON")
        if not isinstance(obj, dict):
            raise ApiError(400, "JSON body must be an object")
        return obj

    # -- dispatch ------------------------------------------------------
    def do_GET(self) -> None:
        self._handle("GET")

    def do_POST(self) -> None:
        self._handle("POST")

    def _method_not_allowed(self) -> None:
        self._json(405, {"ok": False, "error": "method not allowed"})

    do_PUT = do_DELETE = do_PATCH = do_OPTIONS = do_HEAD = _method_not_allowed

    def _handle(self, method: str) -> None:
        app = self.app
        ip = client_ip(self.client_address[0])
        path = urllib.parse.urlsplit(self.path).path
        if method == "GET" and path in ("/", "/index.html"):
            return self._serve_ui()
        rt = ROUTES.get((method, path))
        if rt is None:
            other = any(p == path for (_, p) in ROUTES)
            return self._json(405 if other else 404, {"ok": False, "error": "not found"})
        host = self.headers.get("Host")
        if not host:
            return self._json(400, {"ok": False, "error": "Host header required"})
        sid = get_cookie(self.headers.get("Cookie"), app.cookie_name)
        session = app.sessions.get(sid) if rt.auth else None
        if method != "GET":
            origin = self.headers.get("Origin")
            if origin is not None and not origin_ok(origin, host):
                app.audit.log(session.user if session else None, ip, path, {}, "rejected: cross-origin request")
                return self._json(403, {"ok": False, "error": "cross-origin request rejected"})
        if rt.auth and session is None:
            return self._json(401, {"ok": False, "error": "authentication required"})
        if method != "GET" and rt.auth:
            tok = self.headers.get("X-CSRF-Token") or ""
            if not session or not hmac.compare_digest(tok.encode(), session.csrf.encode()):
                app.audit.log(session.user if session else None, ip, path, {}, "rejected: bad CSRF token")
                return self._json(403, {"ok": False, "error": "missing or invalid CSRF token"})
        req = Request(app, method, path, {}, session, ip, sid)
        status, obj, headers = 200, {}, {}
        result = "ok"
        try:
            if method == "POST":
                req.body = self._read_json()
            if rt.mutating:
                with app.op_lock:
                    res = rt.fn(req)
            else:
                res = rt.fn(req)
            status, obj = res if isinstance(res, tuple) else (200, res)
            if status >= 400:
                result = "error: " + str(obj.get("error", status))
        except ApiError as e:
            status, obj, headers = e.status, {"ok": False, "error": e.message}, e.headers
            result = "error: " + e.message
        except Exception:                                   # never leak internals to the client
            traceback.print_exc(file=sys.stderr)
            status, obj, result = 500, {"ok": False, "error": "internal error"}, "error: internal"
        if rt.mutating:
            note = (" (%s)" % req.audit_note) if req.audit_note else ""
            app.audit.log(req.user, ip, path, req.body, result + note)
        extra = [(k, v) for k, v in headers.items()] + [("Set-Cookie", c) for c in req.cookies_out]
        self._json(status, obj, extra)


class MgmtServer(ThreadingHTTPServer):
    """Dual-stack, TLS-per-connection, bounded-concurrency HTTP server."""

    daemon_threads = True
    request_queue_size = 32
    allow_reuse_address = True
    MAX_CONN = 32

    def __init__(self, addr: tuple[str, int], app: App, ssl_ctx: ssl.SSLContext | None = None):
        self.address_family = socket.AF_INET6 if ":" in addr[0] else socket.AF_INET
        self.app = app
        self.ssl_ctx = ssl_ctx
        self._slots = threading.BoundedSemaphore(self.MAX_CONN)
        super().__init__(addr, Handler)

    def server_bind(self) -> None:
        if self.address_family == socket.AF_INET6:
            self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)     # accept IPv4 too
        import socketserver
        socketserver.TCPServer.server_bind(self)          # skip HTTPServer's slow getfqdn()
        self.server_name, self.server_port = str(self.server_address[0]), self.server_address[1]

    def get_request(self) -> tuple[Any, Any]:
        sock, addr = super().get_request()
        if self.ssl_ctx is not None:
            sock = self.ssl_ctx.wrap_socket(sock, server_side=True, do_handshake_on_connect=False)
        return sock, addr

    def process_request(self, request: Any, client_address: Any) -> None:
        if not self._slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        super().process_request(request, client_address)

    def process_request_thread(self, request: Any, client_address: Any) -> None:
        try:
            super().process_request_thread(request, client_address)
        finally:
            self._slots.release()

    def handle_error(self, request: Any, client_address: Any) -> None:
        exc = sys.exc_info()[1]
        if isinstance(exc, (ssl.SSLError, ConnectionError, TimeoutError, OSError)):
            return
        traceback.print_exc(file=sys.stderr)


def make_server(app: App, bind: str, port: int, ssl_ctx: ssl.SSLContext | None) -> MgmtServer:
    try:
        return MgmtServer((bind, port), app, ssl_ctx)
    except OSError:
        if bind != "::":
            raise
        return MgmtServer(("0.0.0.0", port), app, ssl_ctx)


def make_ssl_context(cert: str, key: str) -> ssl.SSLContext:
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.minimum_version = ssl.TLSVersion.TLSv1_2
    ctx.load_cert_chain(cert, key)
    return ctx


# ============================================================================
# 18. Startup / CLI
# ============================================================================

def seed_dev_accounts(cfg: Config) -> None:
    """Dry-run convenience: create a sandbox admin/admin account under --root."""
    pw = cfg.p("/etc/passwd")
    if os.path.exists(pw):
        return
    h = crypt_hash("admin", "$6$synofurbdev$") or "!"
    write_atomic(pw, "root:x:0:0:root:/root:/bin/bash\nadmin:x:1000:1000:Admin:/home/admin:/bin/bash\n")
    write_atomic(cfg.p("/etc/group"), "root:x:0:\nsudo:x:27:admin\nusers:x:100:\nadm:x:4:\nadmin:x:1000:\n")
    write_atomic(cfg.p("/etc/shadow"), "root:*:19000:0:99999:7:::\nadmin:%s:19000:0:99999:7:::\n" % h, mode=0o600)
    print("dry-run: seeded sandbox account admin / admin", file=sys.stderr)


def startup_recovery(app: App) -> None:
    """An unconfirmed network change that survived a restart/reboot is rolled back."""
    p = load_pending(app.cfg)
    if p and time.time() > p["deadline"]:
        rollback_network(app)
        app.audit.log("system", "-", "net-rollback", {"ifname": p["ifname"]}, "ok: recovered at startup")


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    ap = argparse.ArgumentParser(description="synofurb management plane")
    ap.add_argument("--port", type=int, help="listen port (default 8443)")
    ap.add_argument("--bind", help="listen address (default :: dual-stack)")
    ap.add_argument("--root", default="", help="prefix for all file paths (dev/testing)")
    ap.add_argument("--dry-run", action="store_true", help="never modify the system; record commands only")
    ap.add_argument("--no-tls", action="store_true", help="plain HTTP (development only, requires --dry-run)")
    ap.add_argument("--net-rollback", action="store_true", help=argparse.SUPPRESS)
    return ap.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.no_tls and not args.dry_run:
        print("error: --no-tls is only allowed together with --dry-run", file=sys.stderr)
        return 2
    cfg = Config(root=args.root, dry_run=args.dry_run)
    if args.dry_run and not cfg.root:
        cfg.root = tempfile.mkdtemp(prefix="synofurb-dry-")
        print("dry-run: sandbox root is %s" % cfg.root, file=sys.stderr)
    cfg.load_file()
    if args.port:
        cfg.port = args.port
    if args.bind:
        cfg.bind = args.bind
    cfg.tls = not args.no_tls
    if not args.dry_run and not cfg.root and os.geteuid() != 0:
        print("error: must run as root (or use --dry-run)", file=sys.stderr)
        return 2
    runner: Runner = DryRunRunner(passthrough=True, verbose=True) if args.dry_run else RealRunner()
    if args.dry_run:
        seed_dev_accounts(cfg)
    app = App(cfg, runner)
    if args.net_rollback:
        done = rollback_network(app)
        app.audit.log("system", "-", "net-rollback", {}, "ok: timer fired" if done else "nothing to do")
        return 0
    startup_recovery(app)
    ctx = None
    if cfg.tls:
        cert, key = ensure_tls(cfg, runner, socket.gethostname(), app.ip_addresses())
        app.fingerprint = cert_fingerprint(cert)
        write_atomic(os.path.join(os.path.dirname(cert), "fingerprint.txt"), app.fingerprint + "\n")
        print("TLS SHA-256 fingerprint: %s" % app.fingerprint, flush=True)
        ctx = make_ssl_context(cert, key)
    srv = make_server(app, cfg.bind, cfg.port, ctx)
    print("synofurb-mgmt %s listening on %s%s:%d%s" % (
        VERSION, "https://" if ctx else "http://", srv.server_address[0], srv.server_address[1], " (dry-run)" if args.dry_run else ""),
        flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        srv.server_close()
    return 0


# ============================================================================
# 19. Embedded single-page UI (no external resources; CSP nonce is substituted per response)
# ============================================================================

INDEX_HTML = r"""<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>NAS management</title>
<style nonce="__NONCE__">
:root{--bg:#f5f6f8;--fg:#1c2128;--mut:#5d6672;--card:#fff;--bd:#d6dae0;--acc:#1f6feb;--accfg:#fff;--ok:#1a7f37;--warn:#9a6700;--bad:#cf222e;--code:#eef0f3}
@media (prefers-color-scheme:dark){:root{--bg:#0f1318;--fg:#e4e8ee;--mut:#9aa4b2;--card:#171c23;--bd:#2c343f;--acc:#4c8dff;--accfg:#06101f;--ok:#3fb950;--warn:#d29922;--bad:#f85149;--code:#1e252e}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.45 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
[hidden]{display:none!important}
header{display:flex;flex-wrap:wrap;align-items:center;gap:.5rem 1rem;padding:.6rem 1rem;background:var(--card);border-bottom:1px solid var(--bd);position:sticky;top:0;z-index:5}
header h1{font-size:1.05rem;margin:0}
nav{display:flex;flex-wrap:wrap;gap:.25rem;flex:1}
nav button{background:none;border:1px solid transparent;color:var(--fg);padding:.35rem .7rem;border-radius:6px;cursor:pointer;font:inherit}
nav button[aria-current=true]{background:var(--acc);color:var(--accfg)}
main{max-width:1100px;margin:0 auto;padding:1rem}
.card{background:var(--card);border:1px solid var(--bd);border-radius:10px;padding:.9rem 1rem;margin-bottom:1rem}
.card h2{font-size:1rem;margin:0 0 .6rem}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:1rem}
.kv{display:grid;grid-template-columns:auto 1fr;gap:.2rem .8rem;margin:0}
.kv dt{color:var(--mut)}.kv dd{margin:0;word-break:break-word}
.scroll{overflow-x:auto}
table{border-collapse:collapse;width:100%}
th,td{text-align:left;padding:.35rem .5rem;border-bottom:1px solid var(--bd);vertical-align:top;white-space:nowrap}
th{color:var(--mut);font-weight:600;font-size:.85rem}
td.wrap{white-space:normal}
.btn{font:inherit;padding:.35rem .75rem;border-radius:6px;border:1px solid var(--bd);background:var(--card);color:var(--fg);cursor:pointer}
.btn.primary{background:var(--acc);color:var(--accfg);border-color:var(--acc)}
.btn.danger{color:var(--bad);border-color:var(--bad)}
.btn:disabled{opacity:.5;cursor:default}
.row{display:flex;flex-wrap:wrap;gap:.5rem;align-items:center;margin:.4rem 0}
.badge{display:inline-block;padding:0 .45rem;border-radius:99px;font-size:.78rem;border:1px solid var(--bd);color:var(--mut)}
.badge.ok{color:var(--ok);border-color:var(--ok)}.badge.warn{color:var(--warn);border-color:var(--warn)}.badge.bad{color:var(--bad);border-color:var(--bad)}
pre{background:var(--code);padding:.6rem;border-radius:6px;overflow:auto;white-space:pre-wrap;word-break:break-all;margin:.4rem 0}
input,select{font:inherit;padding:.35rem .5rem;border:1px solid var(--bd);border-radius:6px;background:var(--bg);color:var(--fg);max-width:100%}
label{display:block;margin:.5rem 0}label span{display:block;color:var(--mut);font-size:.85rem}
label.chk{display:flex;gap:.5rem;align-items:center}label.chk span{display:inline;color:var(--fg);font-size:inherit}
small{color:var(--mut);display:block}
meter{width:100%;height:.8rem}
dialog{border:1px solid var(--bd);border-radius:10px;background:var(--card);color:var(--fg);width:min(480px,94vw);padding:1rem}
dialog::backdrop{background:rgba(0,0,0,.5)}
.err{color:var(--bad);margin:.4rem 0;white-space:pre-wrap}
#banner{position:sticky;top:0;z-index:9;background:var(--warn);color:#111;padding:.6rem 1rem;display:flex;gap:.8rem;flex-wrap:wrap;align-items:center}
#toasts{position:fixed;right:.8rem;bottom:.8rem;display:flex;flex-direction:column;gap:.4rem;z-index:20;max-width:90vw}
.toast{background:var(--card);border:1px solid var(--bd);border-left:4px solid var(--ok);padding:.5rem .8rem;border-radius:6px}
.toast.bad{border-left-color:var(--bad)}
.login{max-width:360px;margin:12vh auto}
.ind1{padding-left:1rem}.ind2{padding-left:2rem}.ind3{padding-left:3rem}
.mono{font-family:ui-monospace,Menlo,Consolas,monospace;font-size:.88rem}
footer{color:var(--mut);text-align:center;padding:1rem;font-size:.85rem}
@media (max-width:600px){main{padding:.6rem}.card{padding:.7rem}}
</style></head>
<body>
<div id="banner" hidden></div>
<header id="top" hidden><h1 id="host">NAS</h1><nav id="nav"></nav><span id="who" class="badge"></span><button class="btn" id="logout">Log out</button></header>
<main id="app"></main>
<footer>synofurb-mgmt __VERSION__</footer>
<div id="toasts"></div>
<script nonce="__NONCE__">
'use strict';
const state = {csrf: null, user: null, tab: 'status', deadline: 0, tick: null, poll: null};
const $ = (s, r = document) => r.querySelector(s);
function h(tag, attrs, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (k === 'class') e.className = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else if (v === true) e.setAttribute(k, '');
    else if (v !== false && v != null) e.setAttribute(k, v);
  }
  for (const c of kids.flat(Infinity)) {
    if (c == null || c === false) continue;
    e.append(c.nodeType ? c : document.createTextNode(String(c)));
  }
  return e;
}
const fmtBytes = n => {
  if (n == null) return '-';
  const u = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];
  let i = 0, v = Number(n);
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return (i ? v.toFixed(v >= 100 ? 0 : 1) : v) + ' ' + u[i];
};
const fmtDur = s => {
  if (s == null) return '-';
  const d = Math.floor(s / 86400), hh = Math.floor(s % 86400 / 3600), m = Math.floor(s % 3600 / 60);
  return (d ? d + 'd ' : '') + hh + 'h ' + m + 'm';
};
function toast(msg, bad) {
  const t = h('div', {class: 'toast' + (bad ? ' bad' : '')}, msg);
  $('#toasts').append(t);
  setTimeout(() => t.remove(), bad ? 9000 : 4000);
}
async function api(method, path, body) {
  const opt = {method, headers: {}, credentials: 'same-origin'};
  if (method !== 'GET') {
    opt.headers['Content-Type'] = 'application/json';
    opt.headers['X-CSRF-Token'] = state.csrf || '';
    opt.body = JSON.stringify(body || {});
  }
  const r = await fetch(path, opt);
  let j = {};
  try { j = await r.json(); } catch (e) { /* non-JSON */ }
  if (r.status === 401 && path !== '/api/login') { showLogin(); throw new Error('Session expired, please log in again'); }
  if (!r.ok || j.ok === false) {
    const e = new Error(j.error || ('HTTP ' + r.status));
    e.data = j;
    throw e;
  }
  return j;
}
async function act(btn, fn) {
  if (btn) btn.disabled = true;
  try { await fn(); } catch (e) { toast(e.message, true); } finally { if (btn) btn.disabled = false; }
}
function table(head, rows, empty) {
  if (!rows.length) return h('p', {class: 'badge'}, empty || 'none');
  return h('div', {class: 'scroll'}, h('table', {}, h('thead', {}, h('tr', {}, head.map(x => h('th', {}, x)))),
    h('tbody', {}, rows.map(r => h('tr', {}, r.map(c => h('td', {}, c)))))));
}
const badge = (t, cls) => h('span', {class: 'badge ' + (cls || '')}, t);
const card = (title, ...kids) => h('section', {class: 'card'}, h('h2', {}, title), kids);
const kv = pairs => h('dl', {class: 'kv'}, pairs.filter(p => p[1] != null && p[1] !== '').map(p => [h('dt', {}, p[0]), h('dd', {}, p[1])]));

function formDialog(title, fields, okLabel, onSubmit, danger, intro) {
  const dlg = h('dialog', {});
  const err = h('div', {class: 'err'});
  const inputs = {};
  const form = h('form', {method: 'dialog'}, h('h3', {}, title), intro ? h('p', {}, intro) : null);
  for (const f of fields) {
    let inp;
    if (f.type === 'select') {
      inp = h('select', {name: f.name}, f.options.map(o => h('option', {value: o.v ?? o, selected: (o.v ?? o) === f.value}, o.t ?? o)));
    } else if (f.type === 'checkbox') {
      inp = h('input', {type: 'checkbox', name: f.name, checked: !!f.value});
    } else {
      inp = h('input', {type: f.type || 'text', name: f.name, value: f.value ?? '', placeholder: f.placeholder || '', autocomplete: 'off'});
    }
    inputs[f.name] = inp;
    form.append(h('label', {class: f.type === 'checkbox' ? 'chk' : ''}, h('span', {}, f.label), inp), f.hint ? h('small', {}, f.hint) : null);
  }
  const ok = h('button', {type: 'submit', class: 'btn ' + (danger ? 'danger' : 'primary')}, okLabel);
  form.append(err, h('div', {class: 'row'}, ok, h('button', {type: 'button', class: 'btn', onclick: () => dlg.close()}, 'Cancel')));
  form.addEventListener('submit', async ev => {
    ev.preventDefault();
    const vals = {};
    for (const [k, i] of Object.entries(inputs)) vals[k] = i.type === 'checkbox' ? i.checked : i.value;
    ok.disabled = true; err.textContent = '';
    try { await onSubmit(vals); dlg.close(); } catch (e) { err.textContent = e.message; }
    ok.disabled = false;
  });
  dlg.append(form);
  dlg.addEventListener('close', () => dlg.remove());
  document.body.append(dlg);
  dlg.showModal();
}
const typedConfirm = (title, expected, extra, onSubmit, intro) =>
  formDialog(title, [{name: 'confirm', label: 'Type "' + expected + '" to confirm', placeholder: expected}, ...(extra || [])],
    'Confirm', v => { if (v.confirm !== expected) throw new Error('Confirmation text does not match'); return onSubmit(v); }, true, intro);

/* ---------- login / shell ---------- */
async function showLogin() {
  state.user = null; state.csrf = null; clearPending();
  $('#top').hidden = true;
  const err = h('div', {class: 'err'});
  const fp = h('small', {class: 'mono'});
  const u = h('input', {name: 'username', autocomplete: 'username', autofocus: true});
  const p = h('input', {name: 'password', type: 'password', autocomplete: 'current-password'});
  const btn = h('button', {class: 'btn primary', type: 'submit'}, 'Log in');
  const form = h('form', {class: 'card login'}, h('h2', {}, 'NAS management'),
    h('label', {}, h('span', {}, 'User (member of group sudo)'), u), h('label', {}, h('span', {}, 'Password'), p), err, btn, fp);
  form.addEventListener('submit', async ev => {
    ev.preventDefault(); btn.disabled = true; err.textContent = '';
    try {
      const r = await api('POST', '/api/login', {username: u.value, password: p.value});
      state.csrf = r.csrf; state.user = r.user; p.value = '';
      await start();
    } catch (e) { err.textContent = e.message; }
    btn.disabled = false;
  });
  $('#app').replaceChildren(form);
  try {
    const f = await (await fetch('/api/tls-fingerprint')).json();
    if (f.sha256) fp.textContent = 'Certificate SHA-256: ' + f.sha256;
  } catch (e) { /* ignore */ }
}
const TABS = [['status', 'Status'], ['users', 'Users'], ['network', 'Network'], ['volumes', 'Volumes'], ['mounts', 'Mounts'], ['services', 'System']];
async function start() {
  const me = await api('GET', '/api/me');
  state.csrf = me.csrf; state.user = me.user;
  $('#host').textContent = me.hostname + (me.dry_run ? ' (dry-run)' : '');
  $('#who').textContent = me.user;
  $('#top').hidden = false;
  setPending(me.net_pending);
  show(state.tab);
}
function show(tab) {
  state.tab = tab;
  $('#nav').replaceChildren(...TABS.map(([k, t]) => h('button', {'aria-current': String(k === tab), onclick: () => show(k)}, t)));
  const el = h('div', {}, h('p', {class: 'badge'}, 'Loading...'));
  $('#app').replaceChildren(el);
  VIEWS[tab](el).catch(e => el.replaceChildren(h('div', {class: 'err'}, e.message)));
}
$('#logout').addEventListener('click', async () => { try { await api('POST', '/api/logout'); } catch (e) { /* ignore */ } showLogin(); });

/* ---------- network confirm banner ---------- */
function clearPending() {
  clearInterval(state.tick); clearInterval(state.poll);
  state.tick = state.poll = null; $('#banner').hidden = true;
}
function setPending(p) {
  clearPending();
  if (!p) return;
  state.deadline = Date.now() + p.seconds_left * 1000;
  const left = h('strong', {});
  const confirmBtn = h('button', {class: 'btn primary', onclick: () => act(confirmBtn, async () => {
    await api('POST', '/api/network/confirm'); toast('Network change confirmed'); clearPending(); if (state.tab === 'network') show('network');
  })}, 'Keep new settings');
  const rbBtn = h('button', {class: 'btn', onclick: () => act(rbBtn, async () => {
    await api('POST', '/api/network/rollback'); toast('Rolled back'); clearPending(); show(state.tab);
  })}, 'Roll back now');
  $('#banner').replaceChildren(h('span', {}, 'Network change on ' + p.ifname + ' applied. Auto-rollback in '), left, h('span', {}, ' s unless confirmed.'), confirmBtn, rbBtn);
  $('#banner').hidden = false;
  const upd = () => { left.textContent = Math.max(0, Math.round((state.deadline - Date.now()) / 1000)); };
  upd(); state.tick = setInterval(upd, 1000);
  state.poll = setInterval(async () => {
    try { const m = await api('GET', '/api/me'); if (!m.net_pending) { clearPending(); } } catch (e) { /* link may be down */ }
  }, 5000);
}

/* ---------- views ---------- */
const VIEWS = {};

VIEWS.status = async el => {
  const s = await api('GET', '/api/status');
  const mem = s.memory || {};
  const parts = [];
  parts.push(h('div', {class: 'grid'},
    card('System', kv([['Hostname', s.hostname], ['OS', s.os], ['Kernel', s.kernel], ['Arch', s.arch], ['Uptime', fmtDur(s.uptime)],
      ['Load', s.load && s.load.map(x => x.toFixed(2)).join(' ')],
      ['Temperature', s.temps.length ? s.temps.map(t => t.celsius.toFixed(1) + ' C (' + t.zone + ')').join(', ') : null]])),
    card('Memory & root filesystem',
      mem.total ? [h('div', {}, 'RAM ' + fmtBytes(mem.total - (mem.available || 0)) + ' / ' + fmtBytes(mem.total)),
        h('meter', {min: 0, max: mem.total, value: mem.total - (mem.available || 0)})] : null,
      s.rootfs ? [h('div', {}, 'Root ' + fmtBytes(s.rootfs.total - s.rootfs.free) + ' / ' + fmtBytes(s.rootfs.total)),
        h('meter', {min: 0, max: s.rootfs.total, value: s.rootfs.total - s.rootfs.free})] : null)));
  parts.push(card('Network interfaces', table(['Interface', 'State', 'MAC', 'Addresses'],
    s.interfaces.filter(i => i.ifname !== 'lo').map(i => [i.ifname, i.state, h('span', {class: 'mono'}, i.mac || ''),
      h('span', {class: 'mono'}, i.addrs.filter(a => a.scope !== 'link').map(a => a.addr + '/' + a.prefix).join('  '))]))));
  const kids = {};
  s.disks.forEach(d => { if (d.pkname) (kids[d.pkname] = kids[d.pkname] || []).push(d); });
  const rows = [];
  const walk = (d, depth, seen) => {
    if (seen.has(d.kname)) return;
    seen.add(d.kname);
    rows.push([h('span', {class: 'ind' + Math.min(depth, 3)}, d.name), d.type, fmtBytes(d.size), d.fstype || '', d.label || '',
      d.mountpoints.join(', '), [d.model, d.tran, d.rota ? 'HDD' : (d.type === 'disk' ? 'SSD' : '')].filter(Boolean).join(' ')]);
    (kids[d.kname] || []).forEach(c => walk(c, depth + 1, seen));
  };
  const seen = new Set();
  s.disks.filter(d => !d.pkname).forEach(d => walk(d, 0, seen));
  const smartBox = h('div', {});
  const smartBtn = h('button', {class: 'btn', onclick: () => act(smartBtn, async () => {
    const r = await api('GET', '/api/smart');
    smartBox.replaceChildren(!r.available ? h('p', {class: 'badge'}, 'smartctl is not installed') :
      table(['Disk', 'Model', 'Health', 'Temp', 'Hours', 'Note'], r.disks.map(d => [d.device, d.model || '',
        d.passed === true ? badge('PASSED', 'ok') : d.passed === false ? badge('FAILING', 'bad') : badge('unknown', 'warn'),
        d.temp != null ? d.temp + ' C' : '', d.hours != null ? d.hours : '', d.note || ''])));
  })}, 'Check SMART health');
  parts.push(card('Disks', table(['Device', 'Type', 'Size', 'FS', 'Label', 'Mounted', 'Info'], rows), h('div', {class: 'row'}, smartBtn), smartBox));
  const md = s.md.arrays;
  if (md.length) parts.push(card('RAID (md)', table(['Array', 'Level', 'State', 'Size', 'Members', 'Health', 'Sync'], md.map(a => [
    a.name, a.level || '', a.state, fmtBytes(a.size), a.members.map(m => m.name + (m.failed ? '(F)' : m.spare ? '(S)' : '')).join(' '),
    a.degraded ? badge('DEGRADED ' + (a.health || ''), 'bad') : badge(a.health || 'ok', 'ok'),
    a.sync ? (a.sync.action + ' ' + (a.sync.percent != null ? a.sync.percent + '%' : a.sync.state || '')) : '']))));
  const l = s.lvm;
  if (l.vgs.length) parts.push(card('LVM', table(['VG', 'Size', 'Free', 'PVs', 'LVs'], l.vgs.map(v => [v.vg_name, fmtBytes(v.vg_size), fmtBytes(v.vg_free), v.pv_count, v.lv_count])),
    table(['LV', 'VG', 'Size', 'Attr'], l.lvs.map(v => [v.lv_name, v.vg_name, fmtBytes(v.lv_size), v.lv_attr]))));
  parts.push(card('Mounts', table(['Mountpoint', 'Source', 'FS', 'Used', 'Size'], s.mounts.map(m => [m.target, m.source, m.fstype,
    m.size ? fmtBytes(m.used) : '', m.size ? fmtBytes(m.size) : '']))));
  el.replaceChildren(...parts);
};

VIEWS.users = async el => {
  const r = await api('GET', '/api/users');
  const reload = () => show('users');
  const createBtn = h('button', {class: 'btn primary', onclick: () => formDialog('Create user', [
    {name: 'username', label: 'User name'}, {name: 'password', label: 'Password (min 8)', type: 'password'},
    {name: 'groups', label: 'Extra groups (comma separated)', placeholder: 'users,adm', hint: 'Known: ' + r.groups.join(', ')},
    {name: 'sudo', label: 'Administrator (group sudo, can log in here)', type: 'checkbox', value: true},
    {name: 'ssh_key', label: 'SSH public key (optional)'}], 'Create', async v => {
      await api('POST', '/api/users/create', v); toast('User created'); reload();
    })}, 'Add user');
  const rows = r.users.map(u => {
    const btns = [];
    if (!u.root) {
      btns.push(h('button', {class: 'btn', onclick: () => formDialog('Set password for ' + u.name, [{name: 'password', label: 'New password', type: 'password'}], 'Set',
        async v => { await api('POST', '/api/users/password', {username: u.name, password: v.password}); toast('Password changed'); })}, 'Password'));
      btns.push(h('button', {class: 'btn', onclick: () => formDialog('Add SSH key for ' + u.name, [{name: 'ssh_key', label: 'Public key'}], 'Add',
        async v => { await api('POST', '/api/users/ssh-key', {username: u.name, ssh_key: v.ssh_key}); toast('Key added'); reload(); })}, 'SSH key'));
      btns.push(h('button', {class: 'btn', onclick: ev => act(ev.target, async () => {
        await api('POST', u.locked ? '/api/users/unlock' : '/api/users/lock', {username: u.name}); reload();
      })}, u.locked ? 'Unlock' : 'Lock'));
      btns.push(h('button', {class: 'btn danger', onclick: () => typedConfirm('Delete user ' + u.name, u.name,
        [{name: 'remove_home', label: 'Also delete home directory', type: 'checkbox'}],
        async v => { await api('POST', '/api/users/delete', {username: u.name, confirm: v.confirm, remove_home: v.remove_home}); toast('User deleted'); reload(); })}, 'Delete'));
    }
    return [u.name + (u.name === r.current ? ' (you)' : ''), u.uid, u.groups.join(', '), u.shell,
      [u.root ? badge('root', 'warn') : null, u.locked ? badge('locked', 'bad') : !u.has_password ? badge('no password', 'warn') : badge('active', 'ok'),
       u.sudo ? badge('sudo') : null, u.web_login ? badge('web login') : null], u.ssh_keys, h('div', {class: 'row'}, btns)];
  });
  el.replaceChildren(card('Local users', h('div', {class: 'row'}, createBtn),
    table(['Name', 'UID', 'Groups', 'Shell', 'State', 'Keys', 'Actions'], rows)));
};

VIEWS.network = async el => {
  const r = await api('GET', '/api/network');
  setPending(r.pending);
  const hostBtn = h('button', {class: 'btn', onclick: () => formDialog('Change hostname', [{name: 'hostname', label: 'Hostname', value: r.hostname}], 'Change',
    async v => { await api('POST', '/api/hostname', v); toast('Hostname changed'); show('network'); })}, 'Change hostname');
  const cards = r.interfaces.map(i => {
    const c = i.config || {};
    const edit = h('button', {class: 'btn primary', onclick: () => formDialog('Configure ' + i.ifname, [
      {name: 'mode', label: 'Mode', type: 'select', options: [{v: 'dhcp', t: 'DHCP (IPv4 + IPv6 + SLAAC)'}, {v: 'static', t: 'Static'}], value: c.mode || 'dhcp'},
      {name: 'ipv4', label: 'IPv4 address/prefix', value: c.ipv4 || '', placeholder: '192.168.1.10/24'},
      {name: 'gateway4', label: 'IPv4 gateway', value: c.gw4 || ''},
      {name: 'ipv6', label: 'IPv6 address/prefix (optional)', value: c.ipv6 || '', placeholder: '2001:db8::10/64'},
      {name: 'gateway6', label: 'IPv6 gateway (optional)', value: c.gw6 || ''},
      {name: 'dns', label: 'DNS servers (comma separated)', value: (c.dns || []).join(', ')},
      {name: 'search', label: 'Search domains', value: (c.search || []).join(', ')}],
      'Apply', async v => {
        v.ifname = i.ifname;
        const res = await api('POST', '/api/network/apply', v);
        toast(res.message); setPending(res.pending);
      }, false, 'You may lose the connection. Re-open the page on the new address and press "Keep new settings" within the countdown, otherwise the old settings return automatically.')}, 'Edit');
    return card(i.ifname, kv([['State', i.state], ['MAC', i.mac], ['Config', i.managed ? (c.mode === 'static' ? 'static (managed here)' : 'DHCP (managed here)') : 'not managed here'],
      ['Addresses', i.addrs.filter(a => a.scope !== 'link').map(a => a.addr + '/' + a.prefix).join(', ')],
      ['Note', i.shadowed_by.length ? 'Other files take precedence: ' + i.shadowed_by.join(', ') + ' (they are parked on apply)' : null]]),
      h('div', {class: 'row'}, edit));
  });
  el.replaceChildren(card('Host', kv([['Hostname', r.hostname]]), h('div', {class: 'row'}, hostBtn)), ...cards);
};

VIEWS.volumes = async el => {
  const r = await api('GET', '/api/volumes');
  const reload = () => show('volumes');
  const lvRows = r.lvs.map(v => {
    const free = (r.vgs.find(g => g.vg_name === v.vg_name) || {}).vg_free || 0;
    const ext = h('button', {class: 'btn', disabled: free <= 0, onclick: () => formDialog('Extend ' + v.vg_name + '/' + v.lv_name, [
      {name: 'add_gib', label: 'GiB to add (empty = all ' + fmtBytes(free) + ' free)', type: 'number'}], 'Extend', async x => {
        const res = await api('POST', '/api/volumes/extend', {vg: v.vg_name, lv: v.lv_name, add_gib: x.add_gib === '' ? null : Number(x.add_gib)});
        toast('Extended'); reload();
      })}, 'Extend');
    const del = h('button', {class: 'btn danger', onclick: () => typedConfirm('Delete volume ' + v.lv_name, v.lv_name,
      [{name: 'remove_vg', label: 'Also remove the volume group', type: 'checkbox'}],
      async x => { await api('POST', '/api/volumes/delete', {vg: v.vg_name, lv: v.lv_name, confirm: x.confirm, remove_vg: x.remove_vg}); toast('Deleted'); reload(); },
      'This destroys all data on the volume.')}, 'Delete');
    return [v.lv_name, v.vg_name, fmtBytes(v.lv_size), v.fstype || '', v.mountpoints.join(', '), h('div', {class: 'row'}, ext, del)];
  });
  const cands = r.candidates.map(d => ({d, box: h('input', {type: 'checkbox', value: d.path})}));
  const f = {
    name: h('input', {placeholder: 'media'}), level: h('select', {}, r.raid_levels.map(x => h('option', {value: x}, x))),
    fs: h('select', {}, r.filesystems.map(x => h('option', {value: x}, x))),
    mp: h('input', {placeholder: '/srv/<name> (default)'}), size: h('input', {type: 'number', min: 1, placeholder: 'all'}),
  };
  const out = h('div', {});
  const body = () => ({name: f.name.value.trim(), level: f.level.value, fs: f.fs.value, mountpoint: f.mp.value.trim() || null,
    size_gib: f.size.value === '' ? null : Number(f.size.value), devices: cands.filter(c => c.box.checked).map(c => c.d.path)});
  const planBtn = h('button', {class: 'btn', onclick: () => act(planBtn, async () => {
    const p = await api('POST', '/api/volumes/plan', body());
    const txt = p.steps.map((s, i) => (i + 1) + '. ' + s.desc + '\n     $ ' + s.cmd).join('\n');
    const go = h('button', {class: 'btn danger', onclick: () => typedConfirm('Create volume ' + p.name, p.name, [], async x => {
      const res = await api('POST', '/api/volumes/create', Object.assign(body(), {confirm: x.confirm}));
      toast('Volume created at ' + res.mountpoint); reload();
    }, p.warnings.join(' '))}, 'Execute plan...');
    out.replaceChildren(h('div', {}, p.warnings.map(w => badge(w, 'warn'))), h('p', {}, 'Usable capacity about ' + fmtBytes(p.usable_bytes)), h('pre', {class: 'mono'}, txt), go);
  })}, 'Preview plan');
  const createCard = card('Create volume',
    cands.length ? h('div', {}, cands.map(c => h('label', {class: 'chk'}, c.box, h('span', {}, c.d.path + '  ' + fmtBytes(c.d.size) + '  ' + [c.d.model, c.d.tran].filter(Boolean).join(' '))))) :
      h('p', {class: 'badge'}, 'No free disks or partitions found'),
    h('div', {class: 'grid'}, h('label', {}, h('span', {}, 'Name'), f.name), h('label', {}, h('span', {}, 'RAID level'), f.level),
      h('label', {}, h('span', {}, 'Filesystem'), f.fs), h('label', {}, h('span', {}, 'Mountpoint'), f.mp), h('label', {}, h('span', {}, 'Size GiB (empty = all)'), f.size)),
    h('div', {class: 'row'}, planBtn), out,
    r.excluded.length ? h('details', {}, h('summary', {}, r.excluded.length + ' devices not available'),
      table(['Device', 'Size', 'Reason'], r.excluded.map(d => [d.path, fmtBytes(d.size), d.reason]))) : null);
  el.replaceChildren(
    card('Volume groups', table(['VG', 'Size', 'Free', 'PVs', 'LVs'], r.vgs.map(v => [v.vg_name, fmtBytes(v.vg_size), fmtBytes(v.vg_free), v.pv_count, v.lv_count]))),
    card('Logical volumes', table(['LV', 'VG', 'Size', 'FS', 'Mounted', 'Actions'], lvRows)), createCard);
};

VIEWS.mounts = async el => {
  const r = await api('GET', '/api/mounts');
  const reload = () => show('mounts');
  const mrows = r.mounts.map(m => {
    const un = h('button', {class: 'btn danger', onclick: () => typedConfirm('Unmount ' + m.target, m.target,
      [{name: 'remove_fstab', label: 'Also remove the managed fstab entry', type: 'checkbox'}],
      async x => { await api('POST', '/api/mounts/unmount', {mountpoint: m.target, confirm: x.confirm, remove_fstab: x.remove_fstab}); toast('Unmounted'); reload(); })}, 'Unmount');
    return [m.target, m.source, m.fstype, h('span', {class: 'mono'}, m.options || ''), /^\/(srv|mnt)\//.test(m.target) ? un : ''];
  });
  const drows = r.devices.map(d => {
    const mp = h('input', {value: d.suggested}), ro = h('input', {type: 'checkbox', checked: d.ro_default}), pe = h('input', {type: 'checkbox'});
    const go = h('button', {class: 'btn primary', onclick: () => act(go, async () => {
      await api('POST', '/api/mounts/mount', {device: d.path, mountpoint: mp.value.trim(), ro: ro.checked, persist: pe.checked});
      toast('Mounted'); reload();
    })}, 'Mount');
    return [d.path, d.fstype, d.label || '', fmtBytes(d.size), [d.foreign ? badge('imported', 'warn') : null, d.tran ? badge(d.tran) : null],
      mp, h('label', {class: 'chk'}, ro, h('span', {}, 'ro')), h('label', {class: 'chk'}, pe, h('span', {}, 'persist')), go];
  });
  const act2 = h('button', {class: 'btn', onclick: () => act(act2, async () => {
    const res = await api('POST', '/api/mounts/activate'); toast('Scan finished: ' + res.devices.length + ' mountable device(s)'); reload();
  })}, 'Activate imported volumes (RAID/LVM scan)');
  el.replaceChildren(card('Mounted filesystems', table(['Mountpoint', 'Source', 'FS', 'Options', ''], mrows)),
    card('Available filesystems', h('div', {class: 'row'}, act2),
      h('small', {}, 'Imported (foreign) volumes default to read-only. Mountpoints must be below /srv or /mnt.'),
      table(['Device', 'FS', 'Label', 'Size', '', 'Mountpoint', '', '', ''], drows, 'No unmounted filesystems found')),
    card('fstab (managed entries marked)', table(['Spec', 'Mountpoint', 'Type', 'Options', 'Managed'],
      r.fstab.map(e => [e.spec, e.mountpoint, e.fstype, h('span', {class: 'mono'}, e.options), e.managed ? badge('synofurb', 'ok') : '']))));
};

VIEWS.services = async el => {
  const r = await api('GET', '/api/services');
  const restart = r.restartable.map(u => { const b = h('button', {class: 'btn', onclick: () => act(b, async () => {
    await api('POST', '/api/services/restart', {unit: u}); toast(u + ' restarted'); })}, 'Restart ' + u); return b; });
  const power = a => h('button', {class: 'btn danger', onclick: async () => {
    const host = (await api('GET', '/api/me')).hostname;
    typedConfirm(a + ' this NAS', host, [], async x => { await api('POST', '/api/system/power', {action: a, confirm: x.confirm}); toast(a + ' scheduled'); });
  }}, a === 'reboot' ? 'Reboot' : 'Power off');
  el.replaceChildren(
    card('SSH', kv([['sshd enabled', r.sshd.enabled], ['sshd active', r.sshd.active]]),
      table(['Type', 'Bits', 'Fingerprint'], r.host_keys.map(k => [k.type, k.bits, h('span', {class: 'mono'}, k.fingerprint)]), 'No host keys found')),
    card('Serial consoles', table(['Unit', 'Load', 'Active', 'Sub'], r.serial_getty.map(g => [g.unit, g.load, g.active, g.sub]), 'No serial-getty units')),
    card('Services', h('div', {class: 'row'}, restart)),
    card('Power', h('div', {class: 'row'}, power('reboot'), power('poweroff'))));
};

showLogin().then(async () => { try { await start(); } catch (e) { /* not logged in */ } });
</script></body></html>
"""


if __name__ == "__main__":
    sys.exit(main())
