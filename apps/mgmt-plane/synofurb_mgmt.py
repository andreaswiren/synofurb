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

DEFAULT_VERSION = "0.2.0"


def read_version() -> str:
    """Version from the VERSION file next to this script (packaged by the installer), else the built-in default."""
    try:
        with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "VERSION"), encoding="utf-8") as fh:
            v = fh.readline().strip()
        if re.fullmatch(r"\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.]+)?", v):
            return v
    except OSError:
        pass
    return DEFAULT_VERSION


VERSION = read_version()
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
    return {"ok": True, "tls": req.app.cfg.tls, "sha256": req.app.fingerprint, "hostname": socket.gethostname(),
            "version": VERSION}


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


CPU_VENDORS = {"GenuineIntel": "Intel", "AuthenticAMD": "AMD", "HygonGenuine": "Hygon", "CentaurHauls": "Zhaoxin/VIA"}
MICROCODE_PACKAGES = {"Intel": "intel-microcode", "AMD": "amd64-microcode"}
DSM_INSPECT_PATH = "/var/lib/synofurb/dsm-inspect.txt"
DSM_INSPECT_KEYS = ("model", "hw_version", "dsm_version", "cpu", "cpu_vendor", "mem_kib", "uefi", "dt_model", "uname_m")


def parse_cpuinfo(text: str) -> dict:
    """Vendor (Intel/AMD/...), model name, logical CPU count and loaded microcode revision from /proc/cpuinfo."""
    vendor_id = model = micro = None
    n = 0
    for ln in text.splitlines():
        k, _, v = ln.partition(":")
        k, v = k.strip(), v.strip()
        if k == "processor":
            n += 1
        elif k == "vendor_id" and vendor_id is None:
            vendor_id = v
        elif k in ("model name", "Hardware", "Model") and model is None and v:
            model = v
        elif k == "microcode" and micro is None:
            micro = v
    return {"vendor": CPU_VENDORS.get(vendor_id or "", vendor_id), "vendor_id": vendor_id, "model": model,
            "threads": n or None, "microcode_revision": micro}


def kernel_info(release: str) -> dict:
    """Split a kernel release; ``ge7`` flags kernels >= 7 (newer than most NAS vendor drivers were tested on)."""
    m = re.match(r"(\d+)\.(\d+)", release or "")
    major, minor = (int(m.group(1)), int(m.group(2))) if m else (None, None)
    return {"release": release, "major": major, "minor": minor, "ge7": bool(major is not None and major >= 7)}


def parse_cmdline_consoles(cmdline: str) -> list[dict]:
    """``console=ttyS0,115200n8`` entries of the kernel command line (the last one is the primary console)."""
    res = []
    for tok in cmdline.split():
        if tok.startswith("console=") and len(tok) > 8:
            dev, _, opts = tok[8:].partition(",")
            res.append({"device": dev, "options": opts or None, "serial": dev.startswith(("ttyS", "ttyAMA", "ttyUSB", "ttymxc", "ttyO"))})
    return res


def parse_dsm_inspect(text: str) -> dict[str, str]:
    """``KV|key|value`` lines of dsm-inspect.txt (written by the installer before it touched the NAS)."""
    d = {}
    for ln in text.splitlines():
        p = ln.split("|", 2)
        if len(p) == 3 and p[0] == "KV" and p[1] in DSM_INSPECT_KEYS and p[2].strip():
            d[p[1]] = p[2].strip()[:120]
    return d


def unit_info(cfg: Config) -> dict:
    """Model of the NAS: syno_hw_version (only exists on Synology kernels) or the pre-install DSM inspection."""
    hw = read_text("/proc/sys/kernel/syno_hw_version").strip()[:60]
    kv = parse_dsm_inspect(read_text(cfg.p(DSM_INSPECT_PATH)))
    model = hw or kv.get("model") or kv.get("hw_version")
    return {"model": model or None, "hw_version": hw or kv.get("hw_version"), "dsm_version": kv.get("dsm_version"),
            "source": "syno_hw_version" if hw else ("dsm-inspect.txt" if kv else None),
            "cpu_before": kv.get("cpu"), "mem_kib": kv.get("mem_kib"), "uefi": kv.get("uefi")}


def microcode_info(app: "App", cpu: dict, arch: str) -> dict:
    """State of the vendor microcode package (x86 only) and the revision currently loaded."""
    pkg = MICROCODE_PACKAGES.get(cpu.get("vendor") or "")
    if arch not in ("x86_64", "i686") or pkg is None:
        return {"applicable": False, "package": None, "installed": None, "version": None,
                "loaded_revision": cpu.get("microcode_revision")}
    r = app.run(["dpkg-query", "-W", "-f", "${db:Status-Abbrev}\t${Version}\n", pkg], timeout=15)
    abbr, _, ver = r.out.strip().partition("\t")
    installed = r.ok and abbr.strip().startswith("ii")
    return {"applicable": True, "package": pkg, "installed": installed, "version": ver.strip() or None if installed else None,
            "loaded_revision": cpu.get("microcode_revision")}


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
    cpu = parse_cpuinfo(read_text("/proc/cpuinfo"))
    cmdline = read_text("/proc/cmdline").strip()
    return {
        "ok": True, "hostname": socket.gethostname(), "os": osr.get("PRETTY_NAME") or osr.get("NAME"),
        "kernel": un.release, "kernel_info": kernel_info(un.release), "arch": un.machine, "uptime": uptime, "load": load,
        "cpu": cpu, "microcode": microcode_info(app, cpu, un.machine),
        "console": {"cmdline_consoles": parse_cmdline_consoles(cmdline), "cmdline_has_console": "console=" in cmdline},
        "unit": unit_info(app.cfg), "version": VERSION, "boot_flash": os.path.isfile(app.cfg.p(DOM_TOOL)),
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


def other_signatures(app: App) -> list[dict]:
    """Unmounted devices carrying a signature we cannot mount directly (RAID/LVM members, LUKS, ...)."""
    notes = {"linux_raid_member": "RAID member: use Activate to assemble the array",
             "LVM2_member": "LVM physical volume: use Activate to bring up its volumes",
             "crypto_LUKS": "encrypted (LUKS): unlock it manually", "swap": "swap space"}
    return [{"path": d["path"], "fstype": d["fstype"], "size": d["size"], "type": d["type"], "note": notes.get(d["fstype"], "filesystem not supported here")}
            for d in app.lsblk() if d["fstype"] and d["fstype"] not in MOUNT_FS and not d["mountpoints"]
            and d["type"] in ("part", "disk", "lvm", "md", "crypt") and not d["children"]]


@route("GET", "/api/mounts")
def api_mounts(req: Request) -> dict:
    app = req.app
    return {"ok": True, "mounts": [m for m in app.findmnt() if m["real"]],
            "fstab": fstab_entries(app.read("/etc/fstab")), "devices": mountable_devices(app),
            "other": other_signatures(app)}


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
    # Foreign (imported, e.g. Synology) volumes are mounted read-only unless the caller explicitly sends ro=false.
    ro = as_bool(b["ro"]) if "ro" in b else is_foreign(dev)
    persist = as_bool(b.get("persist"))
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
    return {"ok": True, "results": results, "devices": mountable_devices(app), "other": other_signatures(app),
            "md": parse_mdstat(read_text("/proc/mdstat")), "lvs": app.lvs()}


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
# 16b. Routes: boot manager (units that boot through the internal boot flash / DOM)
# ============================================================================

DOM_TOOL = "/usr/local/sbin/synofurb-dom"
BOOT_IDS = ("dsm", "debian", "debian-old", "agent")
BOOT_NAMES = {"debian": "DEBIAN", "debian-old": "DEBIAN_OLD", "agent": "SYNOFURB_BOOT"}
BOOT_INFO = {
    "dsm": {"label": "Synology DSM", "failsafe": True, "risk": "safe",
            "desc": "The original firmware. It stays on the boot flash untouched and is the fail-safe: it does not depend on Debian, the network or your data volumes."},
    "debian": {"label": "Debian 13", "failsafe": False, "risk": "safe",
               "desc": "The installed system on the NAS volumes."},
    "debian-old": {"label": "Debian 13 (previous kernel)", "failsafe": False, "risk": "safe",
                   "desc": "Kept automatically before each kernel update. Use it if the newest kernel misbehaves."},
    "agent": {"label": "synofurb boot agent", "failsafe": False, "risk": "caution",
              "desc": "RAM-only rescue agent controlled over SSH from your PC. A reinstall erases disks only after you confirm on the PC."},
}


def boot_view(st: dict) -> dict:
    """Map ``synofurb-dom status --json`` onto our four stable entry ids."""
    entries = [e for e in (st.get("entries") or []) if isinstance(e, dict)]
    dname = (st.get("default") or {}).get("name") or None
    used, out = set(), []
    for bid in BOOT_IDS:
        ent = (next((e for e in entries if e.get("dsm")), None) if bid == "dsm"
               else next((e for e in entries if e.get("name") == BOOT_NAMES[bid]), None))
        if ent:
            used.add(ent.get("name"))
        out.append(dict(BOOT_INFO[bid], id=bid, present=ent is not None, name=ent.get("name") if ent else None,
                        default=bool(ent and dname and ent.get("name") == dname),
                        needs_confirm=bid != "debian"))
    flash = {"device": st.get("device"), "bytes": st.get("device_bytes"),
             "p1": st.get("p1") or {}, "p2": st.get("p2") or {}}
    bk = st.get("backup") or {}
    return {"entries": out, "other_entries": [e.get("name") for e in entries if e.get("name") not in used],
            "default_id": next((e["id"] for e in out if e["default"]), None), "default_name": dname,
            "default_valid": dname is not None, "flash": flash,
            "backup": {"exists": bool(bk.get("exists")), "path": bk.get("path") or None},
            "tool_version": st.get("version")}


def _dom_status(app: App) -> tuple[dict | None, str]:
    r = app.run([DOM_TOOL, "status", "--json"], timeout=90)
    if not r.ok:
        return None, "synofurb-dom status failed: " + (r.err or r.out).strip()[:300]
    try:
        st = json.loads(r.out)
    except ValueError:
        return None, "synofurb-dom status returned unreadable output"
    return (st, "") if isinstance(st, dict) else (None, "synofurb-dom status returned unreadable output")


@route("GET", "/api/boot")
def api_boot(req: Request) -> dict:
    app = req.app
    if not os.path.isfile(app.cfg.p(DOM_TOOL)):
        return {"ok": True, "available": False, "message": "not a boot-flash unit (synofurb-dom is not installed)"}
    st, err = _dom_status(app)
    if st is None:
        return {"ok": True, "available": True, "error": err}
    return dict(boot_view(st), ok=True, available=True)


def _boot_entry(req: Request) -> str:
    """Validate {entry, confirm}: known id, tool present, entry exists on the flash, typed confirm unless debian."""
    app, b = req.app, req.body
    entry = b.get("entry")
    if entry not in BOOT_IDS:
        raise ApiError(400, "entry must be one of: " + ", ".join(BOOT_IDS))
    if not os.path.isfile(app.cfg.p(DOM_TOOL)):
        raise ApiError(409, "not a boot-flash unit (synofurb-dom is not installed)")
    if entry != "debian":
        need_confirm(b, entry)
    st, err = _dom_status(app)
    if st is None:
        raise ApiError(500, err)
    if not next(e for e in boot_view(st)["entries"] if e["id"] == entry)["present"]:
        raise ApiError(404, "boot entry %s is not present on the boot flash" % entry)
    return entry


@route("POST", "/api/boot/default")
def api_boot_default(req: Request) -> dict:
    entry = _boot_entry(req)
    r = req.app.run([DOM_TOOL, "default", entry, "--yes"], timeout=180)
    if not r.ok:
        raise ApiError(500, "synofurb-dom default failed: " + (r.err or r.out).strip()[:300])
    return {"ok": True, "entry": entry, "output": trunc((r.out + r.err).strip(), 1000)}


@route("POST", "/api/boot/reboot-to")
def api_boot_reboot_to(req: Request) -> dict:
    app = req.app
    entry = _boot_entry(req)
    r = app.run([DOM_TOOL, "default", entry, "--yes"], timeout=180)
    if not r.ok:
        raise ApiError(500, "synofurb-dom default failed, not rebooting: " + (r.err or r.out).strip()[:300])
    sr = app.run(["systemd-run", "--on-active=5", "systemctl", "reboot"], timeout=15)
    if not sr.ok:
        raise ApiError(500, "default entry set to %s, but the reboot could not be scheduled: %s" % (entry, sr.err.strip()[:200]))
    return {"ok": True, "entry": entry, "message": "Default set to %s; rebooting in 5 s." % entry}


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
# 19. Embedded single-page UI ("ForgeVault family" design: tokens from design/mockups/ui/tokens.json).
#     No external resources. The CSP nonce is substituted per response.
# ============================================================================

# name, light hex, dark hex - same names as tokens.json (color.light.* / color.dark.*), '_' becomes '-' in CSS
_TOKEN_TABLE = """canvas FAFAFA 0A0A0A
surface FFFFFF 171717
surface_2 F3F3F3 202020
surface_3 E9E9E9 2B2B2B
sidebar F7F7F7 101010
border E5E5E5 2A2A2A
border_strong 808080 868686
fg 0A0A0A FAFAFA
fg_2 525252 B1B1B1
placeholder 696969 9E9E9E
accent 7139CB A37BFE
accent_hover 622CB5 B091FE
accent_pressed 541EA0 946AEE
on_accent FFFFFF 0E0C15
accent_bg F1EDFE 31254D
accent_fg 5C29AA C8B7FF
ring 7139CB A37BFE
ring_inverse A37BFE A37BFE
danger C60014 B7191C
danger_hover AC0311 A50E15
danger_pressed 94020D 8D0B11
danger_edge C60014 FF9592
on_danger FFFFFF FFFFFF
ok_solid 047E39 37BB62
ok_fg 005725 6ED889
ok_bg DEF8E2 0F3118
warn_solid A76C01 F0B135
warn_fg 6F4606 F1C45E
warn_bg FFF3D3 3D2A02
err_solid C60014 FC5855
err_fg A70311 FFA09C
err_bg FEEFEF 4F1A18
info_solid 1570D1 539AF2
info_fg 004B96 91C1FF
info_bg E7F1FF 132946
log_bg 121114 060607
log_fg DEDEDE DEDEDE
log_dim 9E9E9E 989898
log_dbg 9DA5B1 979FAB
log_inf 89C3FE 89C3FE
log_wrn F7C243 F7C243
log_err FE8A88 FE8A88
ok_border 66B585 257D41
warn_border CFA95F 9F741E
err_border DF6C77 AE3C3A
info_border 73AAE6 3667A5
accent_border B193E4 6A50A6
log_wrn_bg 322A1B 28200F
log_err_bg 3C2729 331E1E
scrim 000000 000000"""

# web-plane accent sets: accent, token, light hex, dark hex (tokens.json accent.<name>.<theme>.<token>)
_ACCENT_TABLE = """purple accent 7139CB A37BFE
purple accent_hover 6327BA B296FE
purple accent_pressed 560EA9 9968FB
purple on_accent FFFFFF 0E0C15
purple accent_bg F1EDFE 31254D
purple accent_fg 5E1EB3 C8B7FF
purple ring 7139CB A37BFE
blue accent 036ED4 4197FF
blue accent_hover 0260BB 64A8FE
blue accent_pressed 0152A2 1085FC
blue on_accent FFFFFF 080E16
blue accent_bg E7F1FF 132E50
blue accent_fg 015AB1 9DC7FE
blue ring 036ED4 4197FF
green accent 0E803B 47AA62
green accent_hover 026F31 62B777
green accent_pressed 025F29 359A54
green on_accent FFFFFF 071009
green accent_bg E5F5E7 0D371A
green accent_fg 03692E 8FD89E
green ring 0E803B 47AA62
amber accent 9B6200 CE871B
amber accent_hover 875503 DA9845
amber accent_pressed 754901 BC7802
amber on_accent FFFFFF 130C05
amber accent_bg FBEEDF 422701
amber accent_fg 805002 F2B772
amber ring 9B6200 CE871B
rose accent D6224E FF5C74
rose accent_hover C30040 FE7F8D
rose accent_pressed AA0037 F24363
rose on_accent FFFFFF 150A0A
rose accent_bg FEEBEB 4B1D22
rose accent_fg B9003D FEA9AF
rose ring D6224E FF5C74"""
ACCENTS = ("purple", "blue", "green", "amber", "rose")


def theme_tokens() -> dict[str, dict[str, str]]:
    out: dict[str, dict[str, str]] = {"light": {}, "dark": {}}
    for ln in _TOKEN_TABLE.splitlines():
        name, lt, dk = ln.split()
        out["light"][name], out["dark"][name] = "#" + lt, "#" + dk
    return out


def accent_tokens() -> dict[str, dict[str, dict[str, str]]]:
    out: dict[str, dict[str, dict[str, str]]] = {a: {"light": {}, "dark": {}} for a in ACCENTS}
    for ln in _ACCENT_TABLE.splitlines():
        acc, name, lt, dk = ln.split()
        out[acc]["light"][name], out[acc]["dark"][name] = "#" + lt, "#" + dk
    return out


def theme_css() -> str:
    """Light by default, dark via prefers-color-scheme, both overridable with data-theme on <html>;
    the four non-default accents override only the accent tokens (data-accent on <html>)."""
    t = theme_tokens()
    var = lambda d: "".join("--%s:%s;" % (k.replace("_", "-"), v) for k, v in d.items())      # noqa: E731
    css = (":root{color-scheme:light;%s}" % var(t["light"])
           + "@media (prefers-color-scheme:dark){:root:not([data-theme=light]){color-scheme:dark;%s}}" % var(t["dark"])
           + ":root[data-theme=dark]{color-scheme:dark;%s}" % var(t["dark"]))
    for acc, sets in accent_tokens().items():
        if acc == "purple":
            continue
        sel = ":root[data-accent=%s]" % acc
        css += ("%s{%s}" % (sel, var(sets["light"]))
                + "@media (prefers-color-scheme:dark){%s:not([data-theme=light]){%s}}" % (sel, var(sets["dark"]))
                + "%s[data-theme=dark]{%s}" % (sel, var(sets["dark"])))
    return css


_ICON_SPRITE = '''<symbol id="i-check" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5"></path></symbol><symbol id="i-x" viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"></path></symbol><symbol id="i-shield-check" viewBox="0 0 24 24"><path d="M20 13c0 5-3.5 7.5-8 9-4.5-1.5-8-4-8-9V5l8-3 8 3z"></path><path d="m9 12 2 2 4-4"></path></symbol><symbol id="i-users" viewBox="0 0 24 24"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"></path><circle cx="9" cy="7" r="4"></circle><path d="M22 21v-2a4 4 0 0 0-3-3.9M16 3.1a4 4 0 0 1 0 7.8"></path></symbol><symbol id="i-help" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" stroke-dasharray="3 3"></circle><path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3M12 17h.01"></path></symbol><symbol id="i-alert" viewBox="0 0 24 24"><path d="m21.7 18-8-14a2 2 0 0 0-3.5 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.7-3z"></path><path d="M12 9v4M12 17h.01"></path></symbol><symbol id="i-ban" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"></circle><path d="m4.9 4.9 14.2 14.2"></path></symbol><symbol id="i-octagon-x" viewBox="0 0 24 24"><path d="M7.9 2h8.2L22 7.9v8.2L16.1 22H7.9L2 16.1V7.9z"></path><path d="m15 9-6 6M9 9l6 6"></path></symbol><symbol id="i-info" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"></circle><path d="M12 16v-4M12 8h.01"></path></symbol><symbol id="i-server" viewBox="0 0 24 24"><rect x="2" y="3" width="20" height="8" rx="2"></rect><rect x="2" y="13" width="20" height="8" rx="2"></rect><path d="M6 7h.01M6 17h.01"></path></symbol><symbol id="i-disk" viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="18" rx="3"></rect><circle cx="12" cy="12" r="4"></circle><path d="M12 12h.01M17 7h.01"></path></symbol><symbol id="i-cpu" viewBox="0 0 24 24"><rect x="5" y="5" width="14" height="14" rx="2"></rect><rect x="9" y="9" width="6" height="6"></rect><path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3"></path></symbol><symbol id="i-mem" viewBox="0 0 24 24"><path d="M6 19v-3M10 19v-3M14 19v-3M18 19v-3M8 11V9M16 11V9M12 11V9"></path><path d="M2 15h20M2 7a1 1 0 0 1 1-1h18a1 1 0 0 1 1 1v8H2z"></path></symbol><symbol id="i-network" viewBox="0 0 24 24"><rect x="16" y="16" width="6" height="6" rx="1"></rect><rect x="2" y="16" width="6" height="6" rx="1"></rect><rect x="9" y="2" width="6" height="6" rx="1"></rect><path d="M5 16v-3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v3M12 12V8"></path></symbol><symbol id="i-power" viewBox="0 0 24 24"><path d="M12 2v10M18.4 6.6a9 9 0 1 1-12.8 0"></path></symbol><symbol id="i-gauge" viewBox="0 0 24 24"><path d="m12 14 4-4"></path><path d="M3.3 19a10 10 0 1 1 17.4 0"></path></symbol><symbol id="i-layers" viewBox="0 0 24 24"><path d="m12 2 10 5-10 5L2 7z"></path><path d="m2 12 10 5 10-5M2 17l10 5 10-5"></path></symbol><symbol id="i-refresh" viewBox="0 0 24 24"><path d="M3 12a9 9 0 0 1 15-6.7L21 8M21 3v5h-5M21 12a9 9 0 0 1-15 6.7L3 16M3 21v-5h5"></path></symbol><symbol id="i-eye" viewBox="0 0 24 24"><path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"></path><circle cx="12" cy="12" r="3"></circle></symbol><symbol id="i-eye-off" viewBox="0 0 24 24"><path d="M9.9 4.2A10 10 0 0 1 12 4c6.4 0 10 8 10 8a17 17 0 0 1-2.2 3.2M6.6 6.6A17 17 0 0 0 2 12s3.6 8 10 8a9.7 9.7 0 0 0 5.4-1.6M14.1 14.1a3 3 0 1 1-4.2-4.2M2 2l20 20"></path></symbol><symbol id="i-copy" viewBox="0 0 24 24"><rect x="9" y="9" width="13" height="13" rx="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></symbol><symbol id="i-download" viewBox="0 0 24 24"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3"></path></symbol><symbol id="i-chev-d" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6"></path></symbol><symbol id="i-arrow-r" viewBox="0 0 24 24"><path d="M5 12h14M12 5l7 7-7 7"></path></symbol><symbol id="i-arrow-l" viewBox="0 0 24 24"><path d="M19 12H5M12 19l-7-7 7-7"></path></symbol><symbol id="i-plug" viewBox="0 0 24 24"><path d="M12 22v-5M9 8V2M15 8V2M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8z"></path></symbol><symbol id="i-terminal" viewBox="0 0 24 24"><path d="m4 17 6-6-6-6M12 19h8"></path></symbol><symbol id="i-trash" viewBox="0 0 24 24"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M10 11v6M14 11v6"></path></symbol><symbol id="i-lock" viewBox="0 0 24 24"><rect x="3" y="11" width="18" height="11" rx="2"></rect><path d="M7 11V7a5 5 0 0 1 10 0v4"></path></symbol><symbol id="i-key" viewBox="0 0 24 24"><circle cx="7.5" cy="15.5" r="5.5"></circle><path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path></symbol><symbol id="i-clock" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"></circle><path d="M12 6v6l4 2"></path></symbol><symbol id="i-thermo" viewBox="0 0 24 24"><path d="M14 4v10.5a4 4 0 1 1-4 0V4a2 2 0 0 1 4 0z"></path></symbol><symbol id="i-activity" viewBox="0 0 24 24"><path d="M22 12h-4l-3 9L9 3l-3 9H2"></path></symbol><symbol id="i-plus" viewBox="0 0 24 24"><path d="M5 12h14M12 5v14"></path></symbol><symbol id="i-sun" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"></circle><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"></path></symbol><symbol id="i-database" viewBox="0 0 24 24"><ellipse cx="12" cy="5" rx="9" ry="3"></ellipse><path d="M3 5v14c0 1.7 4 3 9 3s9-1.3 9-3V5M3 12c0 1.7 4 3 9 3s9-1.3 9-3"></path></symbol><symbol id="i-undo" viewBox="0 0 24 24"><path d="M3 7v6h6"></path><path d="M21 17a9 9 0 0 0-15-6.7L3 13"></path></symbol><symbol id="i-lifebuoy" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"></circle><circle cx="12" cy="12" r="4"></circle><path d="m4.9 4.9 4.3 4.3M14.8 14.8l4.3 4.3M14.8 9.2l4.3-4.3M4.9 19.1l4.3-4.3"></path></symbol><symbol id="i-flag" viewBox="0 0 24 24"><path d="M4 22V4M4 4h13l-2 4 2 4H4"></path></symbol><symbol id="i-menu" viewBox="0 0 24 24"><path d="M3 6h18M3 12h18M3 18h18"></path></symbol><symbol id="i-wrench" viewBox="0 0 24 24"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.8-3.8a6 6 0 0 1-7.9 7.9l-6.9 6.9a2.1 2.1 0 0 1-3-3l6.9-6.9a6 6 0 0 1 7.9-7.9z"></path></symbol><symbol id="i-package" viewBox="0 0 24 24"><path d="m7.5 4.3 9 5.2M21 8v8a2 2 0 0 1-1 1.7l-7 4a2 2 0 0 1-2 0l-7-4A2 2 0 0 1 3 16V8a2 2 0 0 1 1-1.7l7-4a2 2 0 0 1 2 0l7 4A2 2 0 0 1 21 8z"></path><path d="M3.3 7 12 12l8.7-5M12 22V12"></path></symbol><symbol id="i-settings" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"></path></symbol><symbol id="i-board" viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="18" rx="2"></rect><path d="M3 9h18M9 21V9"></path></symbol><symbol id="i-logo" viewBox="0 0 32 32"><rect x="2" y="2" width="28" height="28" rx="8" fill="currentColor"></rect><rect x="7" y="8" width="18" height="4" rx="1.6" fill="var(--on-accent)"></rect><rect x="7" y="14" width="18" height="4" rx="1.6" fill="var(--on-accent)" opacity=".7"></rect><rect x="7" y="20" width="18" height="4" rx="1.6" fill="var(--on-accent)" opacity=".45"></rect><circle cx="22" cy="10" r="1.2" fill="currentColor"></circle></symbol><symbol id="i-moon" viewBox="0 0 24 24"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9z"></path></symbol>'''

_UI_TEMPLATE = r"""<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>NAS management</title>
<style nonce="__NONCE__">
/*__TOKENS__*/
:root{--font-ui:Inter,"Segoe UI Variable Text","Segoe UI",-apple-system,BlinkMacSystemFont,"SF Pro Text",Roboto,"Helvetica Neue",Arial,sans-serif;--font-mono:"JetBrains Mono","Cascadia Mono","SF Mono",ui-monospace,Menlo,Consolas,monospace;
--e1:0 1px 2px rgb(0 0 0/.05);--e2:0 4px 12px -2px rgb(0 0 0/.14);--e3:0 24px 48px -12px rgb(0 0 0/.5);--top-h:52px}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--canvas);color:var(--fg);font:400 14px/21px var(--font-ui);font-variant-numeric:tabular-nums;-webkit-font-smoothing:antialiased}
[hidden]{display:none!important}
button,input,select{font-family:inherit;color:inherit}button{cursor:pointer}
.sprite{position:absolute;width:0;height:0;overflow:hidden}
.ic{width:16px;height:16px;flex:none;fill:none;stroke:currentColor;stroke-width:2;stroke-linecap:round;stroke-linejoin:round;vertical-align:-3px}
.ic.sm{width:14px;height:14px}.ic.lg{width:20px;height:20px}.ic.xl{width:28px;height:28px}
.spin{animation:sp 1.2s linear infinite}@keyframes sp{to{transform:rotate(360deg)}}
.mono{font-family:var(--font-mono);font-size:13px;line-height:20px}
.muted{color:var(--fg-2)}
:focus-visible{outline:2px solid var(--ring);outline-offset:2px}
.skip{position:absolute;left:-999px;top:0;background:var(--accent);color:var(--on-accent);padding:8px 12px;z-index:100;border-radius:0 0 8px 0}
.skip:focus{left:0}
.vh{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap}
/* shell: sidebar 224, top bar 52 */
.shell{display:grid;grid-template-columns:224px minmax(0,1fr);min-height:100vh}
.side{background:var(--sidebar);border-right:1px solid var(--border);display:flex;flex-direction:column;gap:2px;padding:12px 8px;position:sticky;top:0;height:100vh}
.brand{display:flex;gap:8px;align-items:center;padding:4px 8px 12px}
.brand .ic{width:32px;height:32px;color:var(--accent);stroke:none}
.brand b{display:block;font-size:16px;line-height:20px;font-weight:700}.brand small{display:block;color:var(--fg-2);font-size:12px;line-height:16px;overflow-wrap:anywhere}
.grp{font-size:12px;line-height:16px;font-weight:600;color:var(--fg-2);padding:8px 12px 4px}
.nav{display:flex;flex-direction:column;gap:2px;flex:1;overflow:auto}
.nav button{display:flex;align-items:center;gap:12px;width:100%;text-align:left;background:none;border:0;border-radius:8px;padding:0 12px;min-height:36px;color:var(--fg);font:400 13px/19px var(--font-ui);transition:background .12s,color .12s}
.nav button .ic{color:var(--fg-2)}
.nav button:hover{background:var(--surface-2)}.nav button:active{background:var(--surface-3)}
.nav button[aria-current=page]{background:var(--accent-bg);color:var(--accent-fg);font-weight:600}.nav button[aria-current=page] .ic{color:var(--accent-fg)}
.nav .badge{margin-left:auto}
.me{border-top:1px solid var(--border);padding:12px 8px 0;display:flex;gap:12px;align-items:center;font-size:13px}
.avatar{width:32px;height:32px;border-radius:999px;background:var(--accent-bg);color:var(--accent-fg);border:1px solid var(--accent-border);display:grid;place-items:center;font-weight:600;font-size:12px;flex:none}
.me small{display:block;color:var(--fg-2);font-size:12px;line-height:16px}
.ver{color:var(--fg-2);font-size:12px;line-height:16px;text-align:center;padding:8px 0 0}
.mc{min-width:0;display:flex;flex-direction:column}
.top{height:var(--top-h);display:flex;align-items:center;gap:8px;padding:0 24px;border-bottom:1px solid var(--border);background:var(--surface);position:sticky;top:0;z-index:6}
.crumb{color:var(--fg-2);font-size:13px;flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.crumb b{color:var(--fg);font-weight:600}
.top select{width:auto;height:32px;padding:0 8px;font-size:12px}
.page{padding:20px 24px 24px;max-width:1280px;width:100%}
.ph{display:flex;gap:16px;align-items:flex-start;justify-content:space-between;flex-wrap:wrap;margin-bottom:16px}
h1{font-size:24px;line-height:30px;font-weight:700;letter-spacing:-.015em;margin:0}
.ph p{margin:2px 0 0;color:var(--fg-2)}
h2,h3{margin:0}
.row{display:flex;gap:8px;align-items:center;flex-wrap:wrap}
.stack{display:flex;flex-direction:column;gap:16px}
.cols{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:16px;align-items:start}
.tiles{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:16px;margin-bottom:16px}
.btn.burger,.tabbar{display:none}
/* cards, tiles */
.card{background:var(--surface);border:1px solid var(--border);border-radius:12px;box-shadow:var(--e1);margin-bottom:16px;min-width:0}
.cols .card,.stack .card{margin-bottom:0}
.ch{display:flex;align-items:center;gap:8px;padding:12px 16px;border-bottom:1px solid var(--border)}
.ch h2{font-size:14px;line-height:20px;font-weight:600;flex:1;display:flex;gap:8px;align-items:center;min-width:0}
.cb{padding:16px}.cb.flush{padding:0}.cb>.row,.cb>.kv+.row,.cb>p+.row{margin-top:12px}
.tile{background:var(--surface);border:1px solid var(--border);border-radius:12px;box-shadow:var(--e1);padding:12px 16px;min-width:0}
.tile .lab{display:flex;justify-content:space-between;align-items:center;color:var(--fg-2);font-size:13px;font-weight:600}
.tile .lab .ic{color:var(--fg-2)}
.tile .val{font-size:28px;line-height:34px;font-weight:700;letter-spacing:-.02em;margin-top:2px;overflow-wrap:anywhere}
.tile .val small{font-size:13px;font-weight:600;color:var(--fg-2);letter-spacing:0;margin-left:4px}
.tile .sub{font-size:12px;line-height:16px;color:var(--fg-2);margin-top:2px;overflow-wrap:anywhere}
.tile.e-ok{border-left:4px solid var(--ok-solid)}.tile.e-warn{border-left:4px solid var(--warn-solid)}.tile.e-err{border-left:4px solid var(--err-solid)}
.kv{display:grid;grid-template-columns:minmax(90px,max-content) 1fr;gap:6px 16px;margin:0}
.kv dt{color:var(--fg-2)}.kv dd{margin:0;overflow-wrap:anywhere}
/* buttons: default, hover, pressed, focus, disabled, loading */
.btn{display:inline-flex;align-items:center;justify-content:center;gap:8px;height:36px;padding:0 16px;border-radius:8px;border:1px solid var(--border-strong);background:var(--surface);color:var(--fg);font:600 13px/1 var(--font-ui);white-space:nowrap;text-decoration:none;transition:background .12s,border-color .12s}
.btn:hover{background:var(--surface-2)}.btn:active{background:var(--surface-3)}
.btn.primary{background:var(--accent);border-color:var(--accent);color:var(--on-accent)}
.btn.primary:hover{background:var(--accent-hover);border-color:var(--accent-hover)}.btn.primary:active{background:var(--accent-pressed);border-color:var(--accent-pressed)}
.btn.danger{background:var(--danger);color:var(--on-danger);border:2px solid var(--danger-edge);font-weight:700}
.btn.danger:hover{background:var(--danger-hover)}.btn.danger:active{background:var(--danger-pressed)}
.btn.ghost{border-color:transparent;background:transparent}.btn.ghost:hover{background:var(--surface-2)}.btn.ghost:active{background:var(--surface-3)}
.btn.sm{height:28px;padding:0 12px}.btn.lg{height:42px;padding:0 20px;font-size:14px}
.icon-btn{width:36px;padding:0}
.btn:disabled,.btn.danger:disabled,.btn.primary:disabled{background:var(--surface-2);color:var(--fg-2);border:1px dashed var(--border-strong);cursor:not-allowed;font-weight:600}
.btn.is-loading{pointer-events:none}.btn.primary.is-loading{background:var(--accent-hover)}.btn.danger.is-loading{background:var(--danger-hover)}.btn.is-loading{cursor:progress}
/* forms */
label.f{display:block;margin:0 0 14px}
label.f>span{display:block;font-size:13px;font-weight:600;margin-bottom:4px}
.help{display:block;font-size:12px;line-height:16px;color:var(--fg-2);margin-top:4px}
input,select{font-size:13px;line-height:19px;height:36px;padding:0 12px;border:1px solid var(--border-strong);border-radius:8px;background:var(--surface);color:var(--fg);width:100%;max-width:100%}
input::placeholder{color:var(--placeholder);opacity:1}
input:hover,select:hover{border-color:var(--fg-2)}
input:focus-visible,select:focus-visible{outline:2px solid var(--ring);outline-offset:2px;border-color:var(--ring)}
input:disabled,select:disabled{background:var(--surface-2);border-style:dashed;color:var(--fg-2);cursor:not-allowed}
input.is-err{border:2px solid var(--err-solid)}input.is-ok{border:2px solid var(--ok-solid)}
input.mono,select.mono{font-family:var(--font-mono)}
input[type=checkbox],input[type=radio]{appearance:none;-webkit-appearance:none;width:20px;height:20px;min-width:20px;padding:0;margin:0;border:2px solid var(--border-strong);background:var(--surface);display:inline-grid;place-items:center;flex:none;cursor:pointer}
input[type=checkbox]{border-radius:6px}input[type=radio]{border-radius:999px}
input[type=checkbox]:hover,input[type=radio]:hover{border-color:var(--fg-2)}
input[type=checkbox]:checked{background:var(--accent);border-color:var(--accent)}
input[type=checkbox]:checked::after{content:"";width:5px;height:10px;border:solid var(--on-accent);border-width:0 2.5px 2.5px 0;transform:rotate(45deg) translate(-1px,-1px)}
input[type=checkbox]:checked:hover{background:var(--accent-hover);border-color:var(--accent-hover)}
input[type=radio]:checked{border-color:var(--accent)}input[type=radio]:checked::after{content:"";width:10px;height:10px;border-radius:999px;background:var(--accent)}
input[type=checkbox]:disabled,input[type=radio]:disabled{background:var(--surface-2);border-style:dashed;cursor:not-allowed}
input[type=range]{appearance:auto;-webkit-appearance:auto;padding:0;border:0;height:24px;accent-color:var(--accent);background:none;display:block}
.chk{display:flex;gap:12px;align-items:flex-start;margin:0 0 8px;cursor:pointer;font-size:13px}
.seg{display:inline-flex;border:1px solid var(--border-strong);border-radius:8px;background:var(--surface);padding:2px;gap:2px;margin-bottom:14px}
.seg button{border:0;border-radius:6px;background:transparent;color:var(--fg-2);padding:0 12px;height:28px;font:600 12px/16px var(--font-ui)}
.seg button:hover{background:var(--surface-2);color:var(--fg)}.seg button:active{background:var(--surface-3)}
.seg button[aria-pressed=true]{background:var(--accent);color:var(--on-accent)}
.grid2{display:grid;grid-template-columns:1fr 1fr;gap:0 14px}
.opt{display:flex;gap:12px;align-items:flex-start;padding:12px 16px;border:1px solid var(--border-strong);border-radius:8px;margin-bottom:8px;cursor:pointer;background:var(--surface)}
.opt:hover{background:var(--surface-2)}
.opt.sel{background:var(--accent-bg);border-color:var(--accent);border-left-width:4px}
.opt.off{border-style:dashed;cursor:not-allowed;background:var(--surface-2);color:var(--fg-2)}
.opt:has(input:focus-visible){outline:2px solid var(--ring);outline-offset:2px}
.opt b{display:flex;gap:8px;align-items:center;flex-wrap:wrap;font-weight:600}.opt small{display:block;color:var(--fg-2);font-size:13px;line-height:19px;margin-top:2px}
/* badges, banners: opaque tints and borders, no blending */
.badge{display:inline-flex;align-items:center;gap:4px;height:24px;padding:0 8px 0 6px;border-radius:999px;border:1px solid var(--border-strong);background:var(--surface-2);color:var(--fg-2);font:600 12px/1 var(--font-ui);white-space:nowrap;vertical-align:middle}
.badge.ok{background:var(--ok-bg);color:var(--ok-fg);border-color:var(--ok-border)}
.badge.warn{background:var(--warn-bg);color:var(--warn-fg);border-color:var(--warn-border)}
.badge.err{background:var(--err-bg);color:var(--err-fg);border-color:var(--err-border)}
.badge.info{background:var(--info-bg);color:var(--info-fg);border-color:var(--info-border)}
.badge.accent{background:var(--accent-bg);color:var(--accent-fg);border-color:var(--accent-border)}
.badge.neutral{border-style:dashed}
.chip{display:inline-flex;gap:6px;align-items:center;border:1px solid var(--border-strong);border-radius:6px;padding:0 8px;height:24px;font:600 12px/1 var(--font-ui);background:var(--surface);white-space:nowrap}
.chip .mono{font-size:12px;font-weight:400;color:var(--fg-2)}
.banner{display:flex;gap:12px;align-items:flex-start;padding:12px 16px;border-radius:8px;border:1px solid;margin-bottom:16px}
.banner>.ic{width:18px;height:18px;margin-top:2px}
.banner .bt{flex:1;min-width:0}.banner b{display:block;font-weight:600}.banner p{margin:2px 0 0}
.banner .acts{display:flex;gap:8px;flex-wrap:wrap;align-items:center}
.banner.info{background:var(--info-bg);color:var(--info-fg);border-color:var(--info-border)}
.banner.ok{background:var(--ok-bg);color:var(--ok-fg);border-color:var(--ok-border)}
.banner.warn{background:var(--warn-bg);color:var(--warn-fg);border-color:var(--warn-border)}
.banner.err{background:var(--err-bg);color:var(--err-fg);border-color:var(--err-border);border-left:4px solid var(--err-solid)}
.banner.accent{background:var(--accent-bg);color:var(--accent-fg);border-color:var(--accent-border)}
.banner .btn{color:var(--fg)}.banner .btn.primary{color:var(--on-accent)}.banner .btn.danger{color:var(--on-danger)}
/* persistent safe-apply banner: sticky under the top bar on every page */
#pend{position:sticky;top:var(--top-h);z-index:5}
.rb{display:flex;gap:16px;align-items:center;padding:12px 24px;background:var(--warn-bg);color:var(--warn-fg);border-bottom:1px solid var(--warn-border)}
.rb .tt{font-size:14px;font-weight:700;display:block}.rb p{margin:2px 0 0;font-size:13px}.rb .bt{flex:1;min-width:0}
.rb .row0{display:flex;gap:16px;align-items:center;flex:1;min-width:0}.rb .acts{display:flex;gap:8px;flex:none}.rb .btn{color:var(--fg)}.rb .btn.primary{color:var(--on-accent)}
.ring{width:56px;height:56px;flex:none;position:relative}
.ring svg{width:56px;height:56px;transform:rotate(-90deg)}
.ring circle{fill:none;stroke-width:5}.ring .tr{stroke:var(--warn-border)}.ring .fg{stroke:var(--warn-fg);stroke-linecap:round}
.ring b{position:absolute;inset:0;display:grid;place-items:center;font:700 14px var(--font-mono)}
/* tables */
.tw{overflow-x:auto}
table{border-collapse:collapse;width:100%;font-size:13px;line-height:19px}
th{background:var(--surface-2);color:var(--fg-2);font:600 12px/16px var(--font-ui);text-align:left;padding:8px 16px;border-bottom:1px solid var(--border);white-space:nowrap}
td{padding:12px 16px;border-bottom:1px solid var(--border);vertical-align:middle}
tbody tr:last-child td{border-bottom:0}
tbody tr:hover td{background:var(--surface-2)}tbody tr:active td{background:var(--surface-3)}
tr.attn>td:first-child{border-left:3px solid var(--warn-solid)}tr.bad>td:first-child{border-left:3px solid var(--err-solid)}
td.r,th.r{text-align:right}
.ind1{padding-left:14px}.ind2{padding-left:28px}.ind3{padding-left:42px}
.empty{padding:20px 16px;color:var(--fg-2);display:flex;gap:12px;align-items:center}
/* progress: outlined track, solid fill */
progress{appearance:none;-webkit-appearance:none;display:block;width:100%;height:12px;border:1px solid var(--border-strong);border-radius:999px;background:var(--surface-3);overflow:hidden}
progress::-webkit-progress-bar{background:var(--surface-3);border-radius:999px}
progress::-webkit-progress-value{background:var(--accent);border-radius:999px}
progress::-moz-progress-bar{background:var(--accent);border-radius:999px}
progress.warn::-webkit-progress-value{background:var(--warn-solid)}progress.warn::-moz-progress-bar{background:var(--warn-solid)}
progress.err::-webkit-progress-value{background:var(--err-solid)}progress.err::-moz-progress-bar{background:var(--err-solid)}
progress.ok::-webkit-progress-value{background:var(--ok-solid)}progress.ok::-moz-progress-bar{background:var(--ok-solid)}
.pr{display:flex;align-items:center;gap:8px;min-width:120px}.pr progress{flex:1;min-width:64px}.pr span{font-size:12px;color:var(--fg-2);white-space:nowrap}
/* wizard stepper */
.steps{display:flex;border:1px solid var(--border);border-radius:8px;overflow:hidden;margin-bottom:16px;background:var(--surface)}
.steps div{flex:1;display:flex;gap:8px;align-items:center;justify-content:center;padding:8px 12px;font-size:13px;color:var(--fg-2);border-right:1px solid var(--border);white-space:nowrap}
.steps div:last-child{border-right:0}
.steps div i{font-style:normal;display:grid;place-items:center;width:20px;height:20px;border-radius:999px;border:2px solid var(--border-strong);font-size:12px;flex:none}
.steps .cur{background:var(--accent-bg);color:var(--accent-fg);font-weight:600}.steps .cur i{border-color:var(--accent)}
.steps .done{color:var(--fg)}.steps .done i{background:var(--accent);border-color:var(--accent);color:var(--on-accent)}
.plan li{display:flex;gap:12px;margin:0 0 10px;list-style:none}.plan ul{margin:0;padding:0}
.cap{font:600 12px/16px var(--font-ui);color:var(--fg-2);margin:14px 0 6px}
pre.code{background:var(--surface-2);border:1px solid var(--border);border-radius:8px;padding:12px;margin:0;overflow:auto;font:400 13px/1.7 var(--font-mono);white-space:pre-wrap;overflow-wrap:anywhere}
.log{background:var(--log-bg);color:var(--log-fg);border-radius:8px;padding:8px 0;font:400 12px/20px var(--font-mono);max-height:340px;overflow:auto}
.log div{display:grid;grid-template-columns:72px 1fr;gap:8px;padding:1px 12px;border-left:3px solid transparent}
.log .lv{color:var(--log-inf)}.log .e{background:var(--log-err-bg);border-left-color:var(--log-err)}.log .e .lv{color:var(--log-err)}
.log .m{color:var(--log-dim);padding-left:92px;display:block;grid-column:1/-1;white-space:pre-wrap;overflow-wrap:anywhere;border:0}
.log :focus-visible{outline-color:var(--ring-inverse)}
/* dialog */
dialog{border:1px solid var(--border);border-radius:12px;background:var(--surface);color:var(--fg);width:min(520px,calc(100vw - 24px));padding:0;box-shadow:var(--e3);overflow:hidden}
dialog::backdrop{background:rgb(0 0 0/.6)}
.dh{display:flex;gap:12px;align-items:flex-start;padding:20px 20px 0}.dh h3{font-size:16px;line-height:21px;font-weight:600}.dh p{margin:2px 0 0;color:var(--fg-2);font-size:13px}
.dh>.ic{width:22px;height:22px;margin-top:1px;color:var(--accent-fg)}.dh.bad>.ic{color:var(--err-solid)}
.db{padding:16px 20px 4px}
.df{display:flex;gap:8px;justify-content:flex-end;padding:14px 20px;background:var(--surface-2);border-top:1px solid var(--border);margin-top:12px;flex-wrap:wrap}
.err{color:var(--err-fg);display:flex;gap:6px;margin:0 0 10px;font-size:13px;font-weight:600}
/* login */
.lw{min-height:100vh;display:grid;place-items:center;padding:16px}
.lw>div{width:min(400px,100%)}
.lc{width:100%;background:var(--surface);border:1px solid var(--border);border-radius:12px;box-shadow:var(--e1);overflow:hidden}
.lc form{padding:28px 28px 22px}
.lc .brand2{text-align:center;margin-bottom:18px}.lc .brand2 .ic{width:36px;height:36px;color:var(--accent);stroke:none}
.lc h1{font-size:20px;line-height:26px;margin-top:6px}
.lc .btn.primary{width:100%}
.lf{background:var(--surface-2);border-top:1px solid var(--border);padding:12px 16px;font-size:12px;line-height:16px;color:var(--fg-2);display:flex;gap:8px;align-items:flex-start}
.lf .mono{font-size:12px;overflow-wrap:anywhere;color:var(--fg)}
.pw{position:relative}.pw .icon-btn{position:absolute;right:0;top:0;border:0;background:none;height:36px}
#toasts{position:fixed;right:16px;bottom:16px;display:flex;flex-direction:column;gap:8px;z-index:50;max-width:calc(100vw - 32px)}
.toast{background:var(--surface);border:1px solid var(--border-strong);border-left:4px solid var(--ok-solid);padding:12px 16px;border-radius:8px;box-shadow:var(--e2);display:flex;gap:8px;max-width:420px}
.toast.bad{border-left-color:var(--err-solid)}
.sk{color:var(--fg-2);padding:24px 0}
td .chk{white-space:nowrap;margin:0}
details>summary{cursor:pointer}
/* wide: two-column grids collapse */
@media (max-width:1000px){.tiles{grid-template-columns:repeat(2,minmax(0,1fr))}.cols{grid-template-columns:1fr}.steps div span{display:none}.steps .cur span{display:inline}}
/* phone: bottom tab bar, 44 px controls, tables as cards, stacked banner with full-width buttons */
@media (max-width:700px){
 :root{--top-h:56px}
 .shell{display:block}
 .side{position:fixed;inset:0 auto 0 0;width:min(280px,86vw);z-index:30;transform:translateX(-100%);transition:transform .2s}
 .side.open{transform:none;box-shadow:var(--e3)}
 .btn.burger{display:inline-flex}
 .top{padding:0 12px}.page{padding:16px 12px 76px}
 .tabbar{display:flex;position:fixed;left:0;right:0;bottom:0;z-index:20;background:var(--surface);border-top:1px solid var(--border);padding:4px 4px calc(4px + env(safe-area-inset-bottom))}
 .tabbar button{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:2px;min-height:56px;border:0;border-radius:8px;background:none;color:var(--fg-2);font:400 12px/16px var(--font-ui)}
 .tabbar button:hover{background:var(--surface-2)}.tabbar button:active{background:var(--surface-3)}
 .tabbar button[aria-current=page]{background:var(--accent-bg);color:var(--accent-fg);font-weight:600}
 .scrim{position:fixed;inset:0;background:rgb(0 0 0/.6);z-index:25}
 .btn,.btn.sm,.btn.lg,.icon-btn,select,input:not([type=checkbox]):not([type=radio]):not([type=range]){height:44px;min-height:44px}
 .icon-btn{width:44px}.pw .icon-btn{height:44px}
 .top select{height:44px;font-size:13px}
 .nav button{min-height:44px}
 .chk{min-height:44px;align-items:center;margin:0}.opt{padding:14px 16px}
 .seg button{height:40px}
 details>summary{padding:12px 0;min-height:44px}
 input[type=range]{height:44px}
 .tiles{grid-template-columns:repeat(2,minmax(0,1fr));gap:12px}.tile{padding:12px}.tile .val{font-size:24px;line-height:30px}
 .grid2{grid-template-columns:1fr}
 .ph h1{font-size:22px}.ph .row,.ph .btn{width:100%}
 .steps div{padding:8px 4px}
 .rb{flex-direction:column;align-items:stretch;gap:12px;padding:12px}.rb .acts{flex-direction:column}.rb .acts .btn{width:100%}
 .rb .row0{display:flex;gap:12px;align-items:center}
 .banner{flex-wrap:wrap}.banner .bt{flex:1 1 calc(100% - 64px)}.banner .acts{flex:1 1 100%}.banner .acts .btn{flex:1}
 table.st thead{display:none}
 table.st,table.st tbody,table.st tr,table.st td{display:block}
 table.st tr{border-bottom:1px solid var(--border);padding:12px 0}
 table.st td{border:0;padding:2px 16px;display:flex;gap:12px;justify-content:space-between;align-items:center;text-align:right}
 table.st td::before{content:attr(data-l);color:var(--fg-2);font-size:12px;font-weight:600;text-align:left;flex:none}
 table.st td[data-l=""]::before{content:none}
 table.st td[data-l=""]{justify-content:flex-start;text-align:left}
 table.st tr.attn>td:first-child,table.st tr.bad>td:first-child{border-left:0}
 table.st tr.attn{border-left:3px solid var(--warn-solid)}table.st tr.bad{border-left:3px solid var(--err-solid)}
 table.st tbody tr:hover td{background:none}
}
@media (prefers-reduced-motion:reduce){*{transition:none!important}.spin{animation:none}}
@media (forced-colors:active){.btn,.badge,.card,.tile,.opt{border:1px solid ButtonText}.opt.sel{outline:2px solid Highlight}}
</style>
<script nonce="__NONCE__">
try{var d=document.documentElement,t=localStorage.getItem('fv-theme'),a=localStorage.getItem('fv-accent');if(t==='light'||t==='dark')d.setAttribute('data-theme',t);if(a&&/^(blue|green|amber|rose)$/.test(a))d.setAttribute('data-accent',a)}catch(e){}
</script>
</head>
<body>
<svg class="sprite" aria-hidden="true" focusable="false"><!--__SPRITE__--></svg>
<a class="skip" href="#main">Skip to content</a>
<div id="root"></div>
<div id="toasts" role="status" aria-live="polite"></div>
<div id="live" class="vh" role="status" aria-live="polite"></div>
<script nonce="__NONCE__">
'use strict';
const NS = 'http://www.w3.org/2000/svg';
const S = {csrf: null, user: null, host: '', version: '__VERSION__', page: 'dashboard', pend: null, tick: null, poll: null, st: null};
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
function ic(name, cls) {
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('class', 'ic ' + (cls || ''));
  s.setAttribute('aria-hidden', 'true');
  const u = document.createElementNS(NS, 'use');
  u.setAttribute('href', '#i-' + name);
  s.append(u);
  return s;
}
const fmtBytes = n => {
  if (n == null || isNaN(n)) return '-';
  const u = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];
  let i = 0, v = Number(n);
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return (i ? v.toFixed(v >= 100 ? 0 : 2) : v) + ' ' + u[i];
};
const fmtDur = s => {
  if (s == null) return '-';
  const d = Math.floor(s / 86400), hh = Math.floor(s % 86400 / 3600), m = Math.floor(s % 3600 / 60);
  return (d ? d + ' d ' : '') + hh + ' h ' + m + ' m';
};
const pct = (a, b) => b ? Math.round(100 * a / b) : 0;
function toast(msg, bad) {
  const t = h('div', {class: 'toast' + (bad ? ' bad' : '')}, ic(bad ? 'octagon-x' : 'check'), h('span', {}, msg));
  $('#toasts').append(t);
  setTimeout(() => t.remove(), bad ? 9000 : 4000);
}
async function api(method, path, body) {
  const opt = {method, headers: {}, credentials: 'same-origin'};
  if (method !== 'GET') {
    opt.headers['Content-Type'] = 'application/json';
    opt.headers['X-CSRF-Token'] = S.csrf || '';
    opt.body = JSON.stringify(body || {});
  }
  const r = await fetch(path, opt);
  let j = {};
  try { j = await r.json(); } catch (e) { /* non-JSON */ }
  if (r.status === 401 && path !== '/api/login') { showLogin(); throw new Error('Session expired, please log in again'); }
  if (!r.ok || j.ok === false) { const e = new Error(j.error || ('HTTP ' + r.status)); e.data = j; throw e; }
  return j;
}
/* loading state: busy glyph (static under reduced motion), clicks ignored, aria-busy */
function busy(b, on) {
  if (!b) return;
  b.classList.toggle('is-loading', on);
  if (on) { b.setAttribute('aria-busy', 'true'); const g = ic('refresh', 'spin'); g.dataset.busy = '1'; b.prepend(g); }
  else { b.removeAttribute('aria-busy'); const g = b.querySelector('[data-busy]'); if (g) g.remove(); }
}
async function act(btn, fn) {
  if (btn && btn.classList.contains('is-loading')) return;
  busy(btn, true);
  try { await fn(); } catch (e) { toast(e.message, true); } finally { busy(btn, false); }
}
/* ---------- building blocks ---------- */
const KIND_ICON = {ok: 'check', warn: 'alert', err: 'ban', info: 'info', neutral: 'help', accent: 'flag'};
const badge = (kind, text, icon) => h('span', {class: 'badge ' + kind}, ic(icon || KIND_ICON[kind] || 'info', 'sm'), text);
const chip = (a, b) => h('span', {class: 'chip'}, h('b', {}, a), b ? h('span', {class: 'mono'}, b) : null);
function banner(kind, title, text, ...acts) {
  const el = h('div', {class: 'banner ' + kind}, ic(KIND_ICON[kind] === 'help' ? 'info' : KIND_ICON[kind], 'lg'),
    h('div', {class: 'bt'}, h('b', {}, title), text ? h('p', {}, text) : null),
    acts.length ? h('div', {class: 'acts'}, acts) : null);
  if (kind === 'warn' || kind === 'err') el.setAttribute('role', 'alert');
  return el;
}
const btn = (label, o = {}) => h('button', {type: 'button', class: 'btn ' + (o.kind || '') + ' ' + (o.size || ''), disabled: !!o.disabled, onclick: o.onclick, title: o.title},
  o.icon ? ic(o.icon) : null, label);
function card(title, icon, right, ...body) {
  return h('section', {class: 'card'}, h('div', {class: 'ch'}, h('h2', {}, icon ? ic(icon, 'lg') : null, title), right || null),
    h('div', {class: 'cb'}, body));
}
function cardFlush(title, icon, right, ...body) {
  return h('section', {class: 'card'}, h('div', {class: 'ch'}, h('h2', {}, icon ? ic(icon, 'lg') : null, title), right || null), h('div', {class: 'cb flush'}, body));
}
function tile(label, icon, value, unit, sub, edge) {
  return h('div', {class: 'tile' + (edge ? ' e-' + edge : '')}, h('div', {class: 'lab'}, label, ic(icon)),
    h('div', {class: 'val'}, value, unit ? h('small', {}, unit) : null), h('div', {class: 'sub'}, sub || ' '));
}
function table(head, rows, empty, o = {}) {
  if (!rows.length) return h('div', {class: 'empty'}, ic('info'), empty || 'Nothing to show');
  return h('div', {class: 'tw'}, h('table', {class: 'st'},
    h('thead', {}, h('tr', {}, head.map((x, i) => h('th', {class: (o.right || []).includes(i) ? 'r' : ''}, x)))),
    h('tbody', {}, rows.map(r => {
      const cells = Array.isArray(r) ? r : r.cells;
      return h('tr', {class: Array.isArray(r) ? '' : (r.cls || '')}, cells.map((c, i) => h('td', {'data-l': head[i] || '', class: (o.right || []).includes(i) ? 'r num' : ''}, c)));
    }))));
}
function pageHead(title, sub, ...actions) {
  return h('div', {class: 'ph'}, h('div', {}, h('h1', {}, title), sub ? h('p', {}, sub) : null), actions.length ? h('div', {class: 'row'}, actions) : null);
}
function bar(value, max, kind) {
  const p = h('progress', {max: max || 1, value: Math.min(value, max || 1), class: kind || ''});
  return p;
}
function usage(used, total) {
  const p = pct(used, total), kind = p >= 95 ? 'err' : p >= 85 ? 'warn' : '';
  return h('div', {class: 'pr'}, bar(used, total, kind), h('span', {class: 'num'}, fmtBytes(used) + ' / ' + fmtBytes(total)));
}
function field(label, input, help) { return h('label', {class: 'f'}, h('span', {}, label), input, help ? h('small', {class: 'help'}, help) : null); }
/* ---------- dialogs (also the confirm-action pattern: simple / typed name) ---------- */
function dialog(title, o, bodyFn, footerFn) {
  const dlg = h('dialog', {'aria-labelledby': 'dlg-t'});
  const close = () => dlg.close();
  dlg.append(h('div', {class: 'dh' + (o.danger ? ' bad' : '')}, ic(o.icon || (o.danger ? 'octagon-x' : 'info')),
    h('div', {}, h('h3', {id: 'dlg-t'}, title), o.sub ? h('p', {}, o.sub) : null)));
  const err = h('div', {class: 'err', role: 'alert', hidden: true});
  const body = h('div', {class: 'db'}, bodyFn ? bodyFn(close) : null, err);
  dlg.append(body, h('div', {class: 'df'}, footerFn(close, msg => { err.hidden = !msg; err.replaceChildren(ic('octagon-x'), msg || ''); })));
  dlg.addEventListener('close', () => dlg.remove());
  document.body.append(dlg);
  dlg.showModal();
  const first = dlg.querySelector('input,select'); if (first) first.focus();
  return dlg;
}
function formDialog(title, fields, okLabel, onSubmit, o = {}) {
  const inputs = {};
  const rows = fields.map(f => {
    let inp;
    if (f.type === 'select') inp = h('select', {name: f.name}, f.options.map(x => h('option', {value: x.v ?? x, selected: (x.v ?? x) === f.value}, x.t ?? x)));
    else if (f.type === 'checkbox') inp = h('input', {type: 'checkbox', name: f.name, checked: !!f.value});
    else inp = h('input', {type: f.type || 'text', name: f.name, value: f.value ?? '', placeholder: f.placeholder || '', autocomplete: f.type === 'password' ? 'new-password' : 'off', class: f.mono ? 'mono' : ''});
    inputs[f.name] = inp;
    return f.type === 'checkbox' ? h('label', {class: 'chk'}, inp, h('span', {}, f.label)) : field(f.label, inp, f.hint);
  });
  return dialog(title, o, () => h('form', {onsubmit: ev => ev.preventDefault()}, rows), (close, setErr) => {
    const ok = h('button', {type: 'button', class: 'btn ' + (o.danger ? 'danger' : 'primary')}, o.okIcon ? ic(o.okIcon) : null, okLabel);
    ok.addEventListener('click', async () => {
      const v = {};
      for (const [k, i] of Object.entries(inputs)) v[k] = i.type === 'checkbox' ? i.checked : i.value;
      if (ok.classList.contains('is-loading')) return;
      busy(ok, true); setErr('');
      try { await onSubmit(v); close(); } catch (e) { setErr(e.message); busy(ok, false); }
    });
    return [h('button', {type: 'button', class: 'btn', onclick: close}, 'Cancel'), ok];
  });
}
/* typed-name confirm: the action button stays disabled until the text matches exactly */
function confirmAction(o) {
  let inp = null, ok = null;
  return dialog(o.title, {danger: o.danger !== false, sub: o.sub, icon: o.icon}, () => h('div', {},
    o.body ? (typeof o.body === 'string' ? h('p', {}, o.body) : o.body) : null,
    o.expected ? field('Type "' + o.expected + '" to confirm', inp = h('input', {class: 'mono', autocomplete: 'off', spellcheck: 'false', 'aria-describedby': 'cnt',
      oninput: () => { ok.disabled = inp.value !== o.expected; $('#cnt').textContent = inp.value.length + ' / ' + o.expected.length + (inp.value === o.expected ? ' - matches' : ''); }}),
      null) : null,
    o.expected ? h('small', {class: 'help', id: 'cnt', role: 'status'}, '0 / ' + o.expected.length) : null,
    (o.extra || []).map(x => x)), (close, setErr) => {
      ok = h('button', {type: 'button', class: 'btn ' + (o.danger === false ? 'primary' : 'danger'), disabled: !!o.expected}, o.icon ? ic(o.icon === 'octagon-x' ? 'trash' : o.icon) : (o.danger === false ? null : ic('trash')), o.label || 'Confirm');
      ok.addEventListener('click', async () => {
        if (ok.classList.contains('is-loading')) return;
        busy(ok, true); setErr('');
        try { await o.onConfirm(inp ? inp.value : ''); close(); } catch (e) { setErr(e.message); busy(ok, false); }
      });
      return [h('button', {type: 'button', class: 'btn', onclick: close}, 'Cancel'), ok];
    });
}
/* ---------- theme ---------- */
function effTheme() {
  const t = document.documentElement.getAttribute('data-theme');
  return t || (matchMedia('(prefers-color-scheme:dark)').matches ? 'dark' : 'light');
}
function toggleTheme() {
  const n = effTheme() === 'dark' ? 'light' : 'dark';
  document.documentElement.setAttribute('data-theme', n);
  try { localStorage.setItem('fv-theme', n); } catch (e) { /* private mode */ }
}
function setAccent(a) {
  const d = document.documentElement;
  if (a === 'purple') d.removeAttribute('data-accent'); else d.setAttribute('data-accent', a);
  try { localStorage.setItem('fv-accent', a); } catch (e) { /* private mode */ }
}
const accentSel = () => {
  let cur = 'purple'; try { cur = localStorage.getItem('fv-accent') || 'purple'; } catch (e) { /* ignore */ }
  const sel = h('select', {'aria-label': 'Accent colour', title: 'Accent colour', onchange: () => setAccent(sel.value)},
    ['purple', 'blue', 'green', 'amber', 'rose'].map(a => h('option', {value: a, selected: a === cur}, a[0].toUpperCase() + a.slice(1))));
  return sel;
};
const themeBtn = () => h('button', {type: 'button', class: 'btn ghost icon-btn', title: 'Toggle light / dark theme', 'aria-label': 'Toggle light or dark theme', onclick: toggleTheme}, ic('sun', 'lg'));
/* ---------- login ---------- */
async function showLogin() {
  S.user = null; S.csrf = null; clearPending();
  const err = h('div', {class: 'err', role: 'alert', hidden: true});
  const u = h('input', {name: 'username', autocomplete: 'username', autocapitalize: 'none', spellcheck: 'false', required: true});
  const p = h('input', {name: 'password', type: 'password', autocomplete: 'current-password', required: true});
  const eye = h('button', {type: 'button', class: 'btn icon-btn', 'aria-label': 'Show password', onclick: () => { const s = p.type === 'password'; p.type = s ? 'text' : 'password'; eye.replaceChildren(ic(s ? 'eye-off' : 'eye')); }}, ic('eye'));
  const go = h('button', {class: 'btn primary lg', type: 'submit'}, 'Sign in', ic('arrow-r'));
  const title = h('h1', {}, 'NAS management');
  const sub = h('p', {class: 'muted'}, 'Sign in to manage this NAS');
  const fp = h('span', {class: 'mono'}, 'checking...');
  const form = h('form', {onsubmit: async ev => {
    ev.preventDefault(); if (go.classList.contains('is-loading')) return; busy(go, true); err.hidden = true;
    try { const r = await api('POST', '/api/login', {username: u.value, password: p.value}); S.csrf = r.csrf; S.user = r.user; p.value = ''; await start(); }
    catch (e) { err.hidden = false; err.replaceChildren(ic('octagon-x'), e.message); busy(go, false); }
  }}, h('div', {class: 'brand2'}, ic('logo'), title, sub),
    field('Username', u), field('Password', h('div', {class: 'pw'}, p, eye)), err, go,
    h('p', {class: 'help'}, 'Members of the "sudo" group only. The root account cannot sign in here.'));
  $('#root').replaceChildren(h('main', {class: 'lw', id: 'main'}, h('div', {}, h('div', {class: 'lc'}, form,
    h('div', {class: 'lf'}, ic('lock'), h('div', {}, 'Self-signed certificate. Compare the SHA-256 fingerprint with the installer summary: ', fp))),
    h('div', {class: 'ver'}, 'synofurb-mgmt ' + S.version))));
  u.focus();
  try {
    const f = await (await fetch('/api/tls-fingerprint')).json();
    fp.textContent = f.sha256 || 'not available (plain HTTP)';
    if (f.hostname) title.textContent = f.hostname;
  } catch (e) { fp.textContent = 'not available'; }
}
/* ---------- shell ---------- */
const NAV = [
  ['dashboard', 'Dashboard', 'gauge'], ['storage', 'Storage', 'database'], ['imported', 'Imported volumes', 'package'],
  ['network', 'Network', 'network'], ['users', 'Users', 'users'], ['boot', 'Boot manager', 'board'], ['system', 'System', 'settings'],
];
const TABBAR = ['dashboard', 'storage', 'network', 'users', 'boot'];
function buildShell() {
  const nav = h('nav', {class: 'nav', 'aria-label': 'Main'});
  const side = h('aside', {class: 'side', id: 'side'}, h('div', {class: 'brand'}, ic('logo'), h('div', {}, h('b', {}, 'synofurb'), h('small', {id: 'sh'}, S.host))),
    h('div', {class: 'grp'}, 'Manage'), nav,
    h('div', {class: 'me'}, h('div', {class: 'avatar'}, S.user.slice(0, 2).toUpperCase()), h('div', {}, h('b', {}, S.user), h('small', {}, 'Administrator')),
      h('button', {type: 'button', class: 'btn ghost icon-btn', title: 'Log out', 'aria-label': 'Log out', onclick: async () => { try { await api('POST', '/api/logout'); } catch (e) { /* ignore */ } showLogin(); }}, ic('power'))),
    h('div', {class: 'ver'}, 'synofurb-mgmt ' + S.version));
  const scrim = h('div', {class: 'scrim', hidden: true, onclick: () => closeNav()});
  const top = h('header', {class: 'top'},
    h('button', {type: 'button', class: 'btn ghost icon-btn burger', 'aria-label': 'Open menu', onclick: () => { side.classList.add('open'); scrim.hidden = false; }}, ic('menu', 'lg')),
    h('div', {class: 'crumb', id: 'crumb'}), accentSel(), themeBtn());
  const tabbar = h('nav', {class: 'tabbar', 'aria-label': 'Quick navigation'});
  const mc = h('div', {class: 'mc'}, top, h('div', {id: 'pend'}), h('main', {class: 'page', id: 'main', tabindex: '-1'}));
  $('#root').replaceChildren(h('div', {class: 'shell'}, side, mc), scrim, tabbar);
  S.nav = {nav, tabbar, side, scrim};
  renderNav();
}
function closeNav() { S.nav.side.classList.remove('open'); S.nav.scrim.hidden = true; }
function renderNav() {
  const mk = ([id, label, icon], cls) => h('button', {type: 'button', 'aria-current': id === S.page ? 'page' : null, onclick: () => { closeNav(); show(id); }}, ic(icon), h('span', {}, label));
  S.nav.nav.replaceChildren(...NAV.map(n => {
    const b = mk(n);
    if (n[0] === 'storage' && S.alerts && S.alerts.storage) b.append(badge('warn', String(S.alerts.storage), 'alert'));
    return b;
  }));
  S.nav.tabbar.replaceChildren(...NAV.filter(n => TABBAR.includes(n[0])).map(n => { const b = mk(n); b.firstChild.className.baseVal += ' lg'; return b; }));
  const cur = NAV.find(n => n[0] === S.page);
  $('#crumb').replaceChildren(S.host + ' / ', h('b', {}, cur ? cur[1] : ''));
}
async function start() {
  const me = await api('GET', '/api/me');
  S.csrf = me.csrf; S.user = me.user; S.host = me.hostname; S.version = me.version;
  document.title = me.hostname + ' - NAS management';
  buildShell();
  setPending(me.net_pending);
  show(S.page);
}
async function show(page) {
  S.page = page;
  renderNav();
  const main = $('#main');
  main.replaceChildren(h('div', {class: 'sk'}, 'Loading...'));
  try { main.replaceChildren(await PAGES[page]()); }
  catch (e) { main.replaceChildren(banner('err', 'Could not load this page', e.message)); }
}
/* ---------- rollback countdown banner (network safe-apply) ---------- */
function clearPending() {
  clearInterval(S.tick); clearInterval(S.poll); S.tick = S.poll = null; S.pend = null;
  const el = $('#pend'); if (el) el.replaceChildren();
}
function setPending(p) {
  clearPending();
  if (!p || !$('#pend')) return;
  S.pend = p;
  const total = Math.max(p.seconds_left, 1), end = Date.now() + p.seconds_left * 1000;
  const C = 2 * Math.PI * 18;
  const fg = document.createElementNS(NS, 'circle');
  for (const [k, v] of [['class', 'fg'], ['cx', 24], ['cy', 24], ['r', 18], ['stroke-dasharray', C]]) fg.setAttribute(k, v);
  const tr = document.createElementNS(NS, 'circle');
  for (const [k, v] of [['class', 'tr'], ['cx', 24], ['cy', 24], ['r', 18]]) tr.setAttribute(k, v);
  const svg = document.createElementNS(NS, 'svg'); svg.setAttribute('viewBox', '0 0 48 48'); svg.setAttribute('aria-hidden', 'true'); svg.append(tr, fg);
  const num = h('b', {});
  const title = h('b', {class: 'tt'});
  const keep = btn('Keep these settings', {kind: 'primary', icon: 'check', onclick: () => act(keep, async () => { await api('POST', '/api/network/confirm'); toast('Network change confirmed'); clearPending(); if (S.page === 'network') show('network'); })});
  const rb = btn('Roll back now', {onclick: () => act(rb, async () => { await api('POST', '/api/network/rollback'); toast('Rolled back'); clearPending(); show(S.page); })});
  const el = h('div', {class: 'rb', role: 'alert'}, h('div', {class: 'row0'}, h('div', {class: 'ring'}, svg, num),
    h('div', {class: 'bt'}, title, h('p', {}, 'Changed ' + p.ifname + '. If you can read this page at the new address, press Keep. Otherwise the previous settings return automatically; no action needed.'))),
    h('div', {class: 'acts'}, rb, keep));
  $('#pend').replaceChildren(el);
  const upd = () => {
    const left = Math.max(0, Math.round((end - Date.now()) / 1000));
    num.textContent = left; title.textContent = 'Confirm network change - rolling back in ' + Math.floor(left / 60) + ':' + String(left % 60).padStart(2, '0');
    fg.setAttribute('stroke-dashoffset', C * (1 - left / total));
    if (left === 10 || left === 5) $('#live').textContent = 'Network change rolls back in ' + left + ' seconds';
  };
  upd(); S.tick = setInterval(upd, 1000);
  S.poll = setInterval(async () => { try { const m = await api('GET', '/api/me'); if (!m.net_pending) { clearPending(); toast('Network change is no longer pending'); } } catch (e) { /* link may be down */ } }, 5000);
}
const PAGES = {};
/* ---------- Dashboard ---------- */
PAGES.dashboard = async () => {
  const s = await api('GET', '/api/status');
  S.st = s;
  const mem = s.memory || {}, md = s.md.arrays, ki = s.kernel_info || {}, cpu = s.cpu || {}, mc = s.microcode || {}, un = s.unit || {};
  const degraded = md.filter(a => a.degraded), syncing = md.filter(a => a.sync);
  const dataMount = s.mounts.find(m => m.target === '/srv/data') || s.mounts.find(m => m.target === '/');
  const hot = s.temps.length ? Math.max(...s.temps.map(t => t.celsius)) : null;
  const notices = degraded.length + syncing.length + (S.pend ? 1 : 0);
  S.alerts = {storage: degraded.length + syncing.length};
  renderNav();
  const parts = [pageHead('Dashboard', s.hostname + ' - up ' + fmtDur(s.uptime) + ' - ' + (s.os || 'Linux') + ' - kernel ' + s.kernel,
    btn('Reboot...', {icon: 'power', onclick: () => PAGES_ACTIONS.power('reboot')}))];
  const tiles = [
    tile('Health', 'shield-check', degraded.length ? 'Degraded' : 'Healthy', '', degraded.length ? degraded.map(a => a.name).join(', ') + ' needs attention' : (notices ? 'No critical alerts - ' + notices + ' notice' + (notices > 1 ? 's' : '') : 'No alerts'), degraded.length ? 'err' : notices ? 'warn' : 'ok'),
    dataMount && dataMount.size ? tile('Storage', 'database', pct(dataMount.used, dataMount.size), '% used', fmtBytes(dataMount.used) + ' of ' + fmtBytes(dataMount.size) + ' on ' + dataMount.target, pct(dataMount.used, dataMount.size) >= 90 ? 'warn' : '') : tile('Storage', 'database', '-', '', 'no data volume mounted'),
    tile('Memory', 'mem', mem.total ? ((mem.total - (mem.available || 0)) / 1073741824).toFixed(1) : '-', 'GiB', 'of ' + fmtBytes(mem.total) + (mem.swap_total ? ' - swap ' + fmtBytes(mem.swap_total - (mem.swap_free || 0)) : '')),
    tile('Load', 'activity', s.load ? s.load[0].toFixed(2) : '-', '', (cpu.threads || '?') + ' threads - ' + (s.load ? s.load.map(x => x.toFixed(2)).join(' ') : '') + (hot != null ? ' - ' + hot.toFixed(0) + ' °C' : '')),
    tile('CPU', 'cpu', cpu.vendor || (s.arch.startsWith('a') ? 'ARM' : '-'), '', (cpu.model || s.arch)),
    tile('Kernel', 'terminal', ki.release || s.kernel, '', ki.ge7 ? 'Linux 7 or newer' : (s.arch || ''), ''),
    tile('Microcode', 'wrench', mc.applicable ? (mc.installed ? 'Installed' : 'Missing') : 'n/a', '', mc.applicable ? (mc.package + (mc.version ? ' ' + mc.version : '') + (mc.loaded_revision ? ' - rev ' + mc.loaded_revision : '')) : 'x86 CPUs only', mc.applicable && !mc.installed ? 'warn' : ''),
    tile('Network', 'network', s.interfaces.filter(i => i.ifname !== 'lo' && i.state === 'UP').length, 'up', s.interfaces.filter(i => i.ifname !== 'lo').map(i => i.ifname).join(', ') || 'no interfaces'),
  ];
  parts.push(h('div', {class: 'tiles'}, tiles));
  if (degraded.length) parts.push(banner('err', 'RAID array degraded: ' + degraded.map(a => a.name).join(', '), 'A member is missing or failed. Data is still readable on redundant levels, but replace the disk soon.', btn('Open storage', {size: 'sm', onclick: () => show('storage')})));
  syncing.forEach(a => parts.push(banner('warn', a.name + ' is ' + a.sync.action + (a.sync.percent != null ? ' (' + a.sync.percent + '%)' : ''), 'Volumes stay usable; performance is reduced' + (a.sync.finish_min ? ' for about ' + Math.round(a.sync.finish_min) + ' min.' : '.'))));
  if (ki.ge7) parts.push(banner('info', 'Kernel ' + ki.release, 'Linux 7 or newer is running. If a driver for your hardware misbehaves, the boot manager can start the previous kernel or DSM.', btn('Boot manager', {size: 'sm', onclick: () => show('boot')})));
  if (mc.applicable && !mc.installed) parts.push(banner('warn', 'CPU microcode package is not installed', 'Install ' + mc.package + ' (apt install ' + mc.package + ') to receive CPU security and stability fixes.'));
  // disks: one row per physical disk with its role; the full device tree is folded away
  const kids = {}; s.disks.forEach(d => { if (d.pkname) (kids[d.pkname] = kids[d.pkname] || []).push(d); });
  const below = d => { const o = [], st = [...(kids[d.kname] || [])], seen = new Set(); while (st.length) { const x = st.pop(); if (seen.has(x.kname)) continue; seen.add(x.kname); o.push(x); st.push(...(kids[x.kname] || [])); } return o; };
  const role = d => {
    const sub = below(d), mps = d.mountpoints.concat(...sub.map(x => x.mountpoints));
    if (mps.some(m => ['/', '/boot', '/boot/efi', '[SWAP]'].includes(m))) return badge('info', 'System', 'server');
    const mdn = sub.filter(x => /^raid|^md/.test(x.type)).map(x => x.name);
    if (mdn.length) return badge('ok', 'Array ' + [...new Set(mdn)].join(', '), 'layers');
    if (sub.some(x => x.fstype || x.mountpoints.length) || d.fstype) return badge('neutral', (sub.find(x => x.fstype) || d).fstype || 'In use', 'database');
    return badge('neutral', 'Unused', 'plus');
  };
  const drows = s.disks.filter(d => d.type === 'disk' && !/^(loop|zram|ram|sr|nbd)/.test(d.kname)).map(d => [h('b', {class: 'mono'}, '/dev/' + d.kname), [d.model, d.serial].filter(Boolean).join(' - ') || '-', fmtBytes(d.size),
    [d.tran ? chip(d.tran) : null, chip(d.rota ? 'HDD' : 'SSD')], role(d)]);
  const trows = [], tseen = new Set();
  const walk = (d, depth) => {
    if (tseen.has(d.kname)) return; tseen.add(d.kname);
    trows.push([h('span', {class: 'mono ind' + Math.min(depth, 3)}, d.name), d.type, fmtBytes(d.size), d.fstype || '', d.label || '', h('span', {class: 'mono'}, d.mountpoints.join(', '))]);
    (kids[d.kname] || []).forEach(c => walk(c, depth + 1));
  };
  s.disks.filter(d => !d.pkname).forEach(d => walk(d, 0));
  const smart = h('div', {});
  const sb = btn('Check SMART health', {icon: 'shield-check', onclick: () => act(sb, async () => {
    const r = await api('GET', '/api/smart');
    smart.replaceChildren(!r.available ? h('div', {class: 'empty'}, ic('info'), 'smartctl is not installed (apt install smartmontools)') :
      table(['Disk', 'Model', 'Health', 'Temp', 'Power-on', 'Note'], r.disks.map(d => ({cls: d.passed === false ? 'bad' : d.passed == null ? 'attn' : '', cells: [h('span', {class: 'mono'}, d.device), d.model || '',
        d.passed === true ? badge('ok', 'SMART OK') : d.passed === false ? badge('err', 'Failing', 'octagon-x') : badge('neutral', 'Unknown'), d.temp != null ? d.temp + ' \u00b0C' : '', d.hours != null ? d.hours + ' h' : '', d.note || '']}))));
  })});
  parts.push(cardFlush('Disks', 'disk', sb, table(['Device', 'Model', 'Size', 'Type', 'Role'], drows, 'No disks found', {right: [2]}), smart,
    h('details', {class: 'cb'}, h('summary', {class: 'muted'}, 'All block devices (' + trows.length + ')'), table(['Device', 'Type', 'Size', 'FS', 'Label', 'Mounted'], trows, '', {right: [2]}))));
  if (md.length) parts.push(cardFlush('RAID (mdadm)', 'layers', degraded.length ? badge('err', 'Degraded') : badge('ok', 'Clean'),
    table(['Array', 'Level', 'State', 'Size', 'Members', 'Health', 'Sync'], md.map(a => ({cls: a.degraded ? 'bad' : a.sync ? 'attn' : '', cells: [h('b', {class: 'mono'}, a.name), a.level ? chip(a.level.toUpperCase()) : '', a.state, fmtBytes(a.size),
      h('span', {class: 'mono'}, a.members.map(m => m.name + (m.failed ? '(F)' : m.spare ? '(S)' : '')).join(' ')), a.degraded ? badge('err', 'Degraded ' + (a.health || '')) : badge('ok', a.health || 'OK'), a.sync ? a.sync.action + ' ' + (a.sync.percent != null ? a.sync.percent + '%' : a.sync.state) : '']})))));
  if (s.lvm.vgs.length) parts.push(cardFlush('LVM', 'database', null, table(['VG', 'Size', 'Free', 'PVs', 'LVs'], s.lvm.vgs.map(v => [h('span', {class: 'mono'}, v.vg_name), fmtBytes(v.vg_size), fmtBytes(v.vg_free), v.pv_count, v.lv_count]), '', {right: [1, 2]}),
    table(['LV', 'VG', 'Size', 'Attr'], s.lvm.lvs.map(v => [h('span', {class: 'mono'}, v.lv_name), v.vg_name, fmtBytes(v.lv_size), h('span', {class: 'mono'}, v.lv_attr)]), '', {right: [2]})));
  parts.push(cardFlush('Network interfaces', 'network', null, table(['Interface', 'State', 'MAC', 'Addresses'], s.interfaces.filter(i => i.ifname !== 'lo').map(i => [h('span', {class: 'mono'}, i.ifname),
    i.state === 'UP' ? badge('ok', 'Link up') : badge('neutral', i.state || 'down', 'x'), h('span', {class: 'mono'}, i.mac || ''), h('span', {class: 'mono'}, i.addrs.filter(a => a.scope !== 'link').map(a => a.addr + '/' + a.prefix).join('  '))]), 'No interfaces')));
  const cons = s.console.cmdline_consoles;
  parts.push(h('div', {class: 'cols'},
    card('Unit', 'server', un.source ? badge('info', 'from ' + un.source) : badge('neutral', 'not available'), un.model || un.dsm_version ? h('dl', {class: 'kv'},
      h('dt', {}, 'Model'), h('dd', {}, un.model || '-'), un.hw_version && un.hw_version !== un.model ? [h('dt', {}, 'Hardware'), h('dd', {}, un.hw_version)] : null,
      un.dsm_version ? [h('dt', {}, 'Previous DSM'), h('dd', {}, un.dsm_version)] : null, un.cpu_before ? [h('dt', {}, 'CPU'), h('dd', {}, un.cpu_before)] : null) :
      h('p', {class: 'muted'}, 'No model information found (/proc/sys/kernel/syno_hw_version or /var/lib/synofurb/dsm-inspect.txt).')),
    card('Serial console', 'terminal', cons.length ? badge('ok', 'console= set') : badge('neutral', 'no console= argument'),
      cons.length ? h('dl', {class: 'kv'}, cons.map(c => [h('dt', {}, c.serial ? 'Serial' : 'Console'), h('dd', {class: 'mono'}, c.device + (c.options ? ' @ ' + c.options : ''))])) :
        h('p', {class: 'muted'}, 'The kernel command line has no console= argument, so boot messages are not sent to a serial port.'))));
  return h('div', {}, parts);
};
/* ---------- Storage ---------- */
const RAID_INFO = {
  none: ['No RAID (span)', 'One device, or several devices joined end to end. No redundancy.', 1],
  raid0: ['RAID 0 (stripe)', 'Fastest; losing any device loses everything.', 2],
  raid1: ['RAID 1 (mirror)', 'Same data on every device; survives all but one failure.', 2],
  raid5: ['RAID 5', 'One device of parity; survives one failure.', 3],
  raid6: ['RAID 6', 'Two devices of parity; survives two failures.', 4],
  raid10: ['RAID 10', 'Mirrored stripes; needs an even number of devices.', 4],
};
function usableBytes(level, sizes) {
  if (!sizes.length) return 0;
  const m = Math.min(...sizes), n = sizes.length;
  return {none: sizes.reduce((a, b) => a + b, 0), raid0: m * n, raid1: m, raid5: m * (n - 1), raid6: m * (n - 2), raid10: Math.floor(m * n / 2)}[level];
}
function countOk(level, n) {
  if (level === 'none') return n >= 1;
  const min = {raid0: 2, raid1: 2, raid5: 3, raid6: 4, raid10: 4}[level];
  return n >= min && (level !== 'raid10' || n % 2 === 0);
}
function logView(results) {
  return h('div', {class: 'log', role: 'log'}, results.map(r => [
    h('div', {class: r.rc === 0 ? '' : 'e'}, h('span', {class: 'lv'}, r.rc === 0 ? '● OK' : '✖ ERR'), h('span', {}, r.desc + '  ($ ' + r.cmd + ')')),
    (r.out || r.err) ? h('span', {class: 'm'}, (r.out + (r.err ? '\n' + r.err : '')).trim()) : null]));
}
PAGES.storage = async () => {
  const [v, m] = await Promise.all([api('GET', '/api/volumes'), api('GET', '/api/mounts')]);
  const reload = () => show('storage');
  const mount = {}; m.mounts.forEach(x => { mount[x.target] = x; });
  const syncing = v.md.filter(a => a.sync), degraded = v.md.filter(a => a.degraded);
  S.alerts = {storage: degraded.length + syncing.length}; renderNav();
  const parts = [pageHead('Storage', v.md.length + ' RAID array' + (v.md.length === 1 ? '' : 's') + ' - ' + v.vgs.length + ' volume group' + (v.vgs.length === 1 ? '' : 's') + ' - ' + v.lvs.length + ' volume' + (v.lvs.length === 1 ? '' : 's'),
    btn('Create volume', {kind: 'primary', icon: 'plus', onclick: () => { wizardHost.hidden = false; wizardHost.scrollIntoView({behavior: 'smooth'}); wizardHost.focus(); }}))];
  degraded.forEach(a => parts.push(banner('err', a.name + ' is degraded (' + a.health + ')', 'A member is missing or failed. Replace the disk and re-add it with mdadm.')));
  syncing.forEach(a => parts.push(banner('warn', a.name + ' is ' + a.sync.action + (a.sync.percent != null ? ' (' + a.sync.percent + '%)' : ''), 'Volumes are usable; performance is reduced until it finishes' + (a.sync.finish_min ? ' in about ' + Math.round(a.sync.finish_min) + ' min.' : '.'))));
  const rows = v.lvs.map(l => {
    const free = (v.vgs.find(g => g.vg_name === l.vg_name) || {}).vg_free || 0, mp = l.mountpoints[0], mt = mp && mount[mp];
    const ext = btn('Extend', {size: 'sm', disabled: free <= 0, title: free <= 0 ? 'No free space in ' + l.vg_name : '', onclick: () => formDialog('Extend ' + l.vg_name + '/' + l.lv_name, [
      {name: 'add_gib', label: 'GiB to add', type: 'number', hint: 'Leave empty to use all ' + fmtBytes(free) + ' free in ' + l.vg_name + '. The filesystem is grown afterwards.'}], 'Extend',
      async x => { await api('POST', '/api/volumes/extend', {vg: l.vg_name, lv: l.lv_name, add_gib: x.add_gib === '' ? null : Number(x.add_gib)}); toast('Volume extended'); reload(); }, {icon: 'plus', sub: 'Online, no data is touched'})});
    const del = btn('Delete', {size: 'sm', kind: 'danger', icon: 'trash', onclick: () => confirmAction({title: 'Delete volume ' + l.lv_name, sub: 'This destroys all data on the volume.', expected: l.lv_name, label: 'Delete volume',
      body: 'It is unmounted, its fstab entry is removed and the logical volume is deleted.',
      extra: [h('label', {class: 'chk'}, h('input', {type: 'checkbox', id: 'rmvg'}), h('span', {}, 'Also remove the volume group ' + l.vg_name + ' and its physical volumes'))],
      onConfirm: async () => { await api('POST', '/api/volumes/delete', {vg: l.vg_name, lv: l.lv_name, confirm: l.lv_name, remove_vg: $('#rmvg').checked}); toast('Volume deleted'); reload(); }})});
    return [h('b', {}, l.lv_name), h('span', {class: 'mono'}, l.vg_name), h('span', {class: 'mono'}, mp || '-'), l.fstype || '-', fmtBytes(l.lv_size),
      mt && mt.size ? usage(mt.used, mt.size) : '', mp ? badge('ok', 'Mounted') : badge('neutral', 'Not mounted', 'x'), h('div', {class: 'row'}, ext, del)];
  });
  parts.push(cardFlush('Volumes', 'database', null, table(['Name', 'Pool', 'Mount', 'FS', 'Size', 'Used', 'State', ''], rows, 'No logical volumes yet'),
    v.vgs.length ? h('div', {class: 'cb muted'}, v.vgs.map(g => g.vg_name + ': ' + fmtBytes(g.vg_size) + ' total, ' + fmtBytes(g.vg_free) + ' unallocated').join(' - ')) : null));
  const wizardHost = h('div', {id: 'wiz', tabindex: '-1', hidden: !!v.lvs.length}); parts.push(wizardHost);
  wizard(wizardHost, v, reload);
  return h('div', {}, parts);
};
function wizard(host, v, reload) {
  const W = {step: 1, devs: new Set(), level: 'none', name: '', fs: 'xfs', mp: '', size: null, plan: null, planErr: '', timer: null};
  const preview = h('div', {});
  const stage = h('div', {});
  const cand = v.candidates;
  const sizes = () => cand.filter(c => W.devs.has(c.path)).map(c => c.size);
  const usable = () => usableBytes(W.level, sizes());
  const nameOk = () => /^[a-z][a-z0-9_]{0,23}$/.test(W.name) && !['sys', 'data', 'root', 'swap', 'boot', 'efi'].includes(W.name);
  const body = () => ({name: W.name, level: W.level, fs: W.fs, mountpoint: W.mp || null, size_gib: W.size ? Number(W.size) : null, devices: [...W.devs]});
  async function refreshPlan() {
    clearTimeout(W.timer);
    W.timer = setTimeout(async () => {
      if (!W.devs.size || !countOk(W.level, W.devs.size) || !nameOk()) { W.plan = null; W.planErr = ''; drawPreview(); return; }
      try { W.plan = await api('POST', '/api/volumes/plan', body()); W.planErr = ''; } catch (e) { W.plan = null; W.planErr = e.message; }
      drawPreview(); if (W.step === 4) draw();
    }, 300);
  }
  function drawPreview() {
    const p = W.plan;
    preview.replaceChildren(h('section', {class: 'card'}, h('div', {class: 'ch'}, h('h2', {}, ic('eye', 'lg'), 'Plan preview'), badge('ok', 'Nothing written yet', 'shield-check')),
      h('div', {class: 'cb plan'}, p ? [
        h('ul', {}, [
          h('li', {}, ic('plus'), h('span', {}, h('b', {}, 'Create '), (W.level === 'none' ? 'logical volume ' : W.level.toUpperCase() + ' array and logical volume ') + p.name + ' (about ' + fmtBytes(p.usable_bytes) + ' usable) in ' + p.vg)),
          h('li', {}, ic('plus'), h('span', {}, h('b', {}, 'Format '), 'as ' + p.fs + ', label ' + p.name)),
          h('li', {}, ic('plus'), h('span', {}, h('b', {}, 'Mount '), 'at ' + p.mountpoint + ' and add it to /etc/fstab (nofail)')),
          h('li', {}, ic('info'), h('span', {}, h('b', {}, 'Unchanged: '), 'all other volumes and disks'))]),
        p.warnings.map(w => banner('warn', w, '')),
        h('div', {class: 'cap'}, 'Commands that will run (as root)'),
        h('pre', {class: 'code'}, p.steps.map((s, i) => (i + 1) + '  ' + s.cmd).join('\n'))] :
        [W.planErr ? banner('err', 'This plan is not valid', W.planErr) : h('p', {class: 'muted'}, 'Pick devices, a layout and a name to see the exact steps and commands here. Nothing is executed until you confirm on the last step.')])));
  }
  function nav(canNext, nextLabel, onNext) {
    return h('div', {class: 'row'}, W.step > 1 ? btn('Back', {icon: 'arrow-l', onclick: () => { W.step--; draw(); }}) : null,
      btn('Cancel', {onclick: () => { host.hidden = !!v.lvs.length; W.step = 1; W.devs.clear(); draw(); drawPreview(); }}),
      btn(nextLabel || 'Next', {kind: 'primary', disabled: !canNext, onclick: onNext || (() => { W.step++; draw(); })}));
  }
  function draw() {
    const labels = ['Devices', 'Layout', 'Volume', 'Review'];
    const steps = h('div', {class: 'steps', role: 'list'}, labels.map((l, i) => h('div', {role: 'listitem', class: i + 1 === W.step ? 'cur' : i + 1 < W.step ? 'done' : '', 'aria-current': i + 1 === W.step ? 'step' : null}, h('i', {}, i + 1 < W.step ? '✓' : String(i + 1)), h('span', {}, l))));
    let c;
    if (W.step === 1) {
      c = [h('p', {class: 'muted'}, 'Select the disks or partitions to use. They will be erased.'),
        cand.length ? cand.map(d => {
          const cb = h('input', {type: 'checkbox', checked: W.devs.has(d.path), onchange: () => { cb.checked ? W.devs.add(d.path) : W.devs.delete(d.path); draw(); refreshPlan(); }});
          return h('label', {class: 'opt' + (W.devs.has(d.path) ? ' sel' : '')}, cb, h('div', {}, h('b', {}, h('span', {class: 'mono'}, d.path), chip(fmtBytes(d.size)), d.tran ? chip(d.tran) : null), h('small', {}, [d.model, d.serial, d.type === 'part' ? 'partition of ' + d.pkname : 'whole disk'].filter(Boolean).join(' - '))));
        }) : banner('info', 'No free disks or partitions', 'Every device is mounted, formatted, part of an array or holds the system.'),
        v.excluded.length ? h('details', {}, h('summary', {class: 'muted'}, v.excluded.length + ' devices are not available'), table(['Device', 'Size', 'Why not'], v.excluded.map(d => [h('span', {class: 'mono'}, d.path), fmtBytes(d.size), d.reason]))) : null,
        nav(W.devs.size > 0)];
    } else if (W.step === 2) {
      c = [h('p', {class: 'muted'}, W.devs.size + ' device' + (W.devs.size === 1 ? '' : 's') + ' selected.'),
        Object.entries(RAID_INFO).map(([k, [t, d, min]]) => {
          const ok = countOk(k, W.devs.size);
          const r = h('input', {type: 'radio', name: 'lvl', checked: W.level === k, disabled: !ok, onchange: () => { W.level = k; draw(); refreshPlan(); }});
          return h('label', {class: 'opt' + (W.level === k ? ' sel' : '') + (ok ? '' : ' off')}, r, h('div', {}, h('b', {}, t, ok ? chip('about ' + fmtBytes(usableBytes(k, sizes())) + ' usable') : badge('neutral', 'needs ' + (k === 'raid10' ? 'an even number, at least 4' : min) + ' devices', 'x')), h('small', {}, d)));
        }), nav(countOk(W.level, W.devs.size))];
    } else if (W.step === 3) {
      const maxGib = Math.max(1, Math.floor(usable() / 1073741824));
      const nm = h('input', {class: 'mono', value: W.name, placeholder: 'media', autocomplete: 'off', oninput: () => { W.name = nm.value.trim(); upNext(); refreshPlan(); }});
      const fs = h('select', {onchange: () => { W.fs = fs.value; refreshPlan(); }}, v.filesystems.map(x => h('option', {value: x, selected: x === W.fs}, x)));
      const mp = h('input', {class: 'mono', value: W.mp, placeholder: '/srv/' + (W.name || '<name>'), autocomplete: 'off', oninput: () => { W.mp = mp.value.trim(); refreshPlan(); }});
      const num = h('input', {type: 'number', min: 1, max: maxGib, value: W.size || '', placeholder: 'all', oninput: () => { W.size = num.value; rng.value = num.value || maxGib; refreshPlan(); }});
      const rng = h('input', {type: 'range', min: 1, max: maxGib, value: W.size || maxGib, 'aria-label': 'Size in GiB', oninput: () => { W.size = Number(rng.value) >= maxGib ? '' : rng.value; num.value = W.size; refreshPlan(); }});
      const nx = btn('Next: Review', {kind: 'primary', disabled: !nameOk(), onclick: () => { W.step = 4; draw(); }});
      const upNext = () => { nx.disabled = !nameOk(); };
      c = [h('div', {class: 'grid2'}, field('Name', nm, 'a-z, 0-9 and underscore; creates volume group vg_' + (W.name || '<name>')), field('Filesystem', fs, 'xfs is the default')),
        field('Mountpoint', mp, 'Below /srv or /mnt'),
        field('Size in GiB (empty = all, ' + maxGib + ' GiB max)', num), rng,
        h('div', {class: 'row'}, btn('Back', {icon: 'arrow-l', onclick: () => { W.step = 2; draw(); }}), btn('Cancel', {onclick: () => { host.hidden = !!v.lvs.length; W.step = 1; W.devs.clear(); draw(); drawPreview(); }}), nx)];
    } else {
      const go = btn('Erase ' + W.devs.size + ' device' + (W.devs.size === 1 ? '' : 's') + ' and create ' + W.name, {kind: 'danger', icon: 'trash', disabled: !W.plan, onclick: () => confirmAction({
        title: 'Create volume ' + W.name, sub: 'All data on ' + [...W.devs].join(', ') + ' will be destroyed.', expected: W.name, label: 'Erase and create',
        body: h('div', {}, W.plan.warnings.map(w => banner('warn', w, '')), h('p', {class: 'muted'}, 'This runs ' + W.plan.steps.length + ' steps as root. It stops at the first failure and does not undo earlier steps.')),
        onConfirm: async () => {
          const res = await api('POST', '/api/volumes/create', Object.assign(body(), {confirm: W.name}));
          toast('Volume created at ' + res.mountpoint); reload();
        }})});
      c = [W.plan ? [banner('err', 'Data on the selected devices will be destroyed', W.plan.warnings.slice(1).join(' ')),
        h('dl', {class: 'kv'}, h('dt', {}, 'Name'), h('dd', {}, W.plan.name), h('dt', {}, 'Layout'), h('dd', {}, RAID_INFO[W.level][0]), h('dt', {}, 'Devices'), h('dd', {class: 'mono'}, [...W.devs].join(', ')),
          h('dt', {}, 'Filesystem'), h('dd', {}, W.plan.fs), h('dt', {}, 'Mountpoint'), h('dd', {class: 'mono'}, W.plan.mountpoint), h('dt', {}, 'Usable'), h('dd', {}, 'about ' + fmtBytes(W.plan.usable_bytes)))] :
        banner('err', 'This plan is not valid', W.planErr || 'Check the previous steps.'),
        h('div', {class: 'row'}, btn('Back to plan', {kind: 'primary', icon: 'arrow-l', onclick: () => { W.step = 3; draw(); }}), btn('Cancel', {onclick: () => { host.hidden = !!v.lvs.length; W.step = 1; W.devs.clear(); draw(); drawPreview(); }}), go)];
    }
    stage.replaceChildren(h('section', {class: 'card'}, h('div', {class: 'ch'}, h('h2', {}, ic('plus', 'lg'), 'Create volume')), h('div', {class: 'cb'}, steps, c)));
  }
  host.replaceChildren(h('div', {class: 'cols'}, stage, preview));
  draw(); drawPreview();
}
/* ---------- Imported volumes ---------- */
PAGES.imported = async () => {
  const r = await api('GET', '/api/mounts');
  const reload = () => show('imported');
  const parts = [pageHead('Imported volumes', 'Volumes from another system (for example a migrated Synology RAID) and removable disks.',
    (() => { const b = btn('Scan RAID and LVM', {icon: 'refresh', onclick: () => act(b, async () => {
      const res = await api('POST', '/api/mounts/activate'); toast('Scan finished: ' + res.devices.length + ' mountable volume' + (res.devices.length === 1 ? '' : 's')); reload();
    })}); return b; })())];
  parts.push(banner('info', 'Read-only by default', 'Imported volumes are mounted read-only so nothing on them can change. Untick "Read-only" only when you are sure; imported Synology volumes may contain features this kernel cannot fully write.'));
  const rows = r.devices.map(d => {
    const mp = h('input', {class: 'mono', value: d.suggested, 'aria-label': 'Mountpoint for ' + d.path});
    const ro = h('input', {type: 'checkbox', checked: d.ro_default, 'aria-label': 'Read-only'});
    const warn = h('span', {class: 'help', hidden: ro.checked}, 'Writable mount of an imported volume');
    ro.addEventListener('change', () => { warn.hidden = ro.checked; });
    const pe = h('input', {type: 'checkbox', 'aria-label': 'Mount at boot'});
    const go = btn('Mount', {kind: 'primary', size: 'sm', onclick: () => act(go, async () => {
      await api('POST', '/api/mounts/mount', {device: d.path, mountpoint: mp.value.trim(), ro: ro.checked, persist: pe.checked}); toast('Mounted ' + d.path); reload(); })});
    return [h('b', {class: 'mono'}, d.path), d.fstype, d.label || '', fmtBytes(d.size),
      [d.foreign ? badge('warn', 'Imported', 'package') : badge('neutral', 'Local'), d.tran ? chip(d.tran) : null], mp,
      h('label', {class: 'chk'}, ro, h('span', {}, 'Read-only')), h('label', {class: 'chk'}, pe, h('span', {}, 'At boot')), h('div', {}, go, warn)];
  });
  parts.push(cardFlush('Volumes ready to mount', 'package', badge(r.devices.length ? 'info' : 'neutral', r.devices.length + ' found'),
    table(['Device', 'FS', 'Label', 'Size', 'Origin', 'Mountpoint', 'Mode', 'Persist', ''], rows, 'No unmounted filesystems found. Insert a disk or scan for RAID/LVM volumes.')));
  if (r.other.length) parts.push(cardFlush('Other signatures', 'help', null, table(['Device', 'Signature', 'Size', 'What to do'], r.other.map(o => [h('span', {class: 'mono'}, o.path), o.fstype, fmtBytes(o.size), o.note]))));
  const mrows = r.mounts.map(m => [h('span', {class: 'mono'}, m.target), h('span', {class: 'mono'}, m.source), m.fstype, h('span', {class: 'mono'}, m.options || ''), m.size ? usage(m.used, m.size) : '',
    /^\/(srv|mnt)\//.test(m.target) ? btn('Unmount', {size: 'sm', onclick: () => confirmAction({title: 'Unmount ' + m.target, sub: 'Services using it will lose access.', expected: m.target, label: 'Unmount',
      extra: [h('label', {class: 'chk'}, h('input', {type: 'checkbox', id: 'rmfs'}), h('span', {}, 'Also remove the managed fstab entry'))],
      onConfirm: async () => { await api('POST', '/api/mounts/unmount', {mountpoint: m.target, confirm: m.target, remove_fstab: $('#rmfs').checked}); toast('Unmounted'); reload(); }})}) : '']);
  parts.push(cardFlush('Mounted filesystems', 'disk', null, table(['Mountpoint', 'Source', 'FS', 'Options', 'Used', ''], mrows)));
  parts.push(cardFlush('fstab', 'board', null, table(['Spec', 'Mountpoint', 'Type', 'Options', 'Managed'], r.fstab.map(e => [h('span', {class: 'mono'}, e.spec), h('span', {class: 'mono'}, e.mountpoint), e.fstype, h('span', {class: 'mono'}, e.options), e.managed ? badge('accent', 'synofurb', 'check') : '']), 'No fstab entries')));
  return h('div', {}, parts);
};
/* ---------- Network ---------- */
PAGES.network = async () => {
  const r = await api('GET', '/api/network');
  const reload = () => show('network');
  setPending(r.pending);
  const editHost = h('div', {});
  const parts = [pageHead('Network', 'Hostname ' + r.hostname + ' - safe apply with automatic rollback after ' + r.rollback_seconds + ' s',
    btn('Change hostname', {icon: 'wrench', onclick: () => formDialog('Change hostname', [{name: 'hostname', label: 'Hostname', value: r.hostname, mono: true}], 'Change', async v => { await api('POST', '/api/hostname', v); toast('Hostname changed'); reload(); }, {sub: 'Letters, digits and hyphens'}) }))];
  function edit(i) {
    const c = i.config || {};
    const F = {mode: c.mode || 'dhcp'};
    const inp = (v, ph) => h('input', {class: 'mono', value: v || '', placeholder: ph || '', autocomplete: 'off'});
    const f4 = inp(c.ipv4, '192.168.1.140/24'), g4 = inp(c.gw4, '192.168.1.1'), f6 = inp(c.ipv6, '2001:db8::10/64'), g6 = inp(c.gw6, 'fe80::1'), dns = inp((c.dns || []).join(', '), '192.168.1.1, 9.9.9.9'), sr = inp((c.search || []).join(', '), 'lan');
    const sect = h('div', {});
    const seg = h('div', {class: 'seg', role: 'group', 'aria-label': 'Addressing mode'});
    const drawMode = () => {
      seg.replaceChildren(...[['dhcp', 'DHCP'], ['static', 'Static']].map(([k, t]) => h('button', {type: 'button', 'aria-pressed': String(F.mode === k), onclick: () => { F.mode = k; drawMode(); }}, t)));
      sect.replaceChildren(...(F.mode === 'dhcp' ? [banner('info', 'DHCP with IPv6 autoconfiguration', 'IPv4 and IPv6 addresses come from the router (DHCPv4, DHCPv6 and SLAAC). The DHCP client id is the MAC address.')] :
        [h('div', {class: 'grid2'}, field('IPv4 address/prefix', f4), field('IPv4 gateway', g4), field('IPv6 address/prefix (optional)', f6, 'Leave empty to keep SLAAC'), field('IPv6 gateway (optional)', g6)),
          h('div', {class: 'grid2'}, field('DNS servers', dns, 'Comma separated'), field('Search domains', sr))]));
    };
    drawMode();
    const apply = btn('Apply with rollback', {kind: 'primary', icon: 'undo', onclick: () => act(apply, async () => {
      const res = await api('POST', '/api/network/apply', {ifname: i.ifname, mode: F.mode, ipv4: f4.value.trim(), gateway4: g4.value.trim(), ipv6: f6.value.trim(), gateway6: g6.value.trim(), dns: dns.value, search: sr.value});
      toast(res.message); setPending(res.pending); editHost.replaceChildren();
    })});
    editHost.replaceChildren(card('Edit ' + i.ifname, 'settings', null, seg, sect,
      banner('info', 'Safe apply is on', 'Changes are applied for ' + r.rollback_seconds + ' s and rolled back unless you confirm from the new address.'),
      h('div', {class: 'row'}, btn('Discard', {onclick: () => editHost.replaceChildren()}), apply)));
    editHost.scrollIntoView({behavior: 'smooth'});
  }
  const ifs = r.interfaces.map(i => card(i.ifname, 'network', i.state === 'UP' ? badge('ok', 'Link up') : badge('neutral', i.state === 'DOWN' || !i.state ? 'No link' : i.state, 'x'),
    h('dl', {class: 'kv'}, h('dt', {}, 'Config'), h('dd', {}, i.managed ? (i.config.mode === 'static' ? 'Static (managed here)' : 'DHCP (managed here)') : 'Not managed here'),
      h('dt', {}, 'Addresses'), h('dd', {class: 'mono'}, i.addrs.filter(a => a.scope !== 'link').map(a => a.addr + '/' + a.prefix).join(', ') || 'none'),
      h('dt', {}, 'MAC'), h('dd', {class: 'mono'}, i.mac || '-'), h('dt', {}, 'MTU'), h('dd', {}, i.mtu)),
    i.shadowed_by.length ? banner('warn', 'Other config files take precedence', i.shadowed_by.join(', ') + ' would override this page; they are set aside when you apply.') : null,
    h('div', {class: 'row'}, btn('Edit', {kind: 'primary', size: 'sm', icon: 'settings', onclick: () => edit(i)}))));
  parts.push(h('div', {class: 'cols'}, h('div', {class: 'stack'}, ifs.length ? ifs : banner('info', 'No network interfaces found', '')), editHost));
  return h('div', {}, parts);
};
/* ---------- Users ---------- */
PAGES.users = async () => {
  const r = await api('GET', '/api/users');
  const reload = () => show('users');
  const parts = [pageHead('Users', 'Local accounts. Members of the sudo group can sign in to this page.',
    btn('Add user', {kind: 'primary', icon: 'plus', onclick: () => formDialog('Add user', [
      {name: 'username', label: 'User name', mono: true}, {name: 'password', label: 'Password', type: 'password', hint: 'At least 8 characters'},
      {name: 'groups', label: 'Extra groups', placeholder: 'users, adm', hint: 'Known: ' + r.groups.join(', ')},
      {name: 'sudo', label: 'Administrator (group sudo; may sign in here and use sudo)', type: 'checkbox', value: true},
      {name: 'ssh_key', label: 'SSH public key (optional)', mono: true}], 'Create user', async v => { await api('POST', '/api/users/create', v); toast('User created'); reload(); }, {icon: 'users'}) }))];
  const rows = r.users.map(u => {
    const acts = [];
    if (!u.root) {
      acts.push(btn('Password', {size: 'sm', icon: 'key', onclick: () => formDialog('Set password for ' + u.name, [{name: 'password', label: 'New password', type: 'password'}], 'Set password', async v => { await api('POST', '/api/users/password', {username: u.name, password: v.password}); toast('Password changed'); }, {icon: 'key'})}));
      acts.push(btn('SSH key', {size: 'sm', onclick: () => formDialog('Add SSH key for ' + u.name, [{name: 'ssh_key', label: 'Public key', mono: true}], 'Add key', async v => { await api('POST', '/api/users/ssh-key', {username: u.name, ssh_key: v.ssh_key}); toast('Key added'); reload(); }, {icon: 'key'})}));
      const lk = btn(u.locked ? 'Unlock' : 'Lock', {size: 'sm', icon: 'lock', onclick: () => act(lk, async () => { await api('POST', u.locked ? '/api/users/unlock' : '/api/users/lock', {username: u.name}); reload(); })});
      acts.push(lk);
      acts.push(btn('Delete', {size: 'sm', kind: 'danger', icon: 'trash', onclick: () => confirmAction({title: 'Delete user ' + u.name, expected: u.name, label: 'Delete user', sub: 'The account is removed immediately.',
        extra: [h('label', {class: 'chk'}, h('input', {type: 'checkbox', id: 'rmhome'}), h('span', {}, 'Also delete the home directory'))],
        onConfirm: async () => { await api('POST', '/api/users/delete', {username: u.name, confirm: u.name, remove_home: $('#rmhome').checked}); toast('User deleted'); reload(); }})}));
    }
    return [h('b', {}, u.name + (u.name === r.current ? ' (you)' : '')), u.uid, u.groups.join(', '), h('span', {class: 'mono'}, u.shell),
      h('div', {class: 'row'}, u.root ? badge('warn', 'root', 'lock') : null, u.locked ? badge('err', 'Locked', 'lock') : !u.has_password ? badge('warn', 'No password') : badge('ok', 'Active'), u.sudo ? badge('accent', 'sudo', 'key') : null, u.web_login ? badge('info', 'Web sign-in', 'check') : null),
      u.ssh_keys, h('div', {class: 'row'}, acts)];
  });
  parts.push(cardFlush('Local users', 'users', badge('neutral', r.users.length + ' accounts'), table(['Name', 'UID', 'Groups', 'Shell', 'State', 'Keys', ''], rows, 'No users', {right: [1, 5]})));
  return h('div', {}, parts);
};
/* ---------- Boot manager ---------- */
PAGES.boot = async () => {
  const b = await api('GET', '/api/boot');
  const parts = [];
  if (!b.available) {
    parts.push(pageHead('Boot manager', 'Choose what this NAS starts next time it reboots.'));
    parts.push(banner('info', 'Not a boot-flash unit', 'This NAS does not boot through an internal boot flash, so there is nothing to switch here. (The synofurb-dom tool is not installed.)'));
    return h('div', {}, parts);
  }
  if (b.error) { parts.push(pageHead('Boot manager', '')); parts.push(banner('err', 'Could not read the boot flash', b.error)); return h('div', {}, parts); }
  let sel = (b.entries.find(e => e.default) || {}).id || 'debian';
  const dsm = b.entries.find(e => e.id === 'dsm');
  const area = h('div', {});
  const doSet = (id, reboot) => {
    const e = b.entries.find(x => x.id === id);
    const run = async () => {
      const res = await api('POST', reboot ? '/api/boot/reboot-to' : '/api/boot/default', {entry: id, confirm: id});
      toast(res.message || ('Default boot entry is now ' + e.label)); if (!reboot) show('boot'); else setTimeout(() => show('boot'), 8000);
    };
    if (!e.needs_confirm) { return confirmAction({title: (reboot ? 'Reboot into ' : 'Set default to ') + e.label, danger: false, label: reboot ? 'Reboot now' : 'Set as default', icon: reboot ? 'power' : 'check',
      sub: reboot ? 'The NAS restarts in 5 seconds. Expect about 90 s of downtime.' : 'Takes effect on the next reboot.', onConfirm: run}); }
    return confirmAction({title: (reboot ? 'Reboot into ' : 'Set default to ') + e.label, expected: id, danger: id === 'agent', label: reboot ? 'Set default and reboot' : 'Set as default', icon: reboot ? 'power' : 'check',
      sub: id === 'dsm' ? 'The NAS will start the original Synology firmware.' : 'Type the entry name to confirm.',
      body: id === 'dsm' ? 'Debian stays installed and its data is untouched; DSM may need to be set up again if its system partition was reclaimed.' : (id === 'agent' ? 'The boot agent is a RAM-only rescue system. A reinstall from there erases disks only after you confirm on your PC.' : ''), onConfirm: run});
  };
  function draw() {
    const cur = b.entries.find(e => e.id === sel);
    area.replaceChildren(card('Boot entries', 'board', b.default_valid ? null : badge('err', 'Default is invalid', 'octagon-x'),
      b.entries.map(e => {
        const rd = h('input', {type: 'radio', name: 'be', checked: sel === e.id, disabled: !e.present, onchange: () => { sel = e.id; draw(); }});
        return h('label', {class: 'opt' + (sel === e.id ? ' sel' : '') + (e.present ? '' : ' off')}, rd, h('div', {}, h('b', {}, e.label, e.default ? badge('accent', 'Default', 'flag') : null,
          e.failsafe ? badge('ok', 'Fail-safe', 'shield-check') : null, !e.present ? badge('neutral', 'Not installed', 'x') : null, e.risk === 'caution' && e.present ? badge('warn', 'Can reinstall') : null),
          h('small', {}, e.desc + (e.name ? '  [' + e.name + ']' : ''))));
      }),
      h('div', {class: 'row'}, btn('Set as default', {disabled: !cur.present || cur.default, icon: 'check', onclick: () => doSet(sel, false)}),
        btn('Reboot into ' + cur.label, {kind: 'primary', disabled: !cur.present, icon: 'power', onclick: () => doSet(sel, true)}))));
  }
  const rt = dsm && dsm.present ? btn('Return to DSM', {kind: 'primary', icon: 'undo', size: 'lg', disabled: false, onclick: () => doSet('dsm', true)}) : null;
  parts.push(pageHead('Boot manager', 'Choose what this NAS starts next time it reboots.', rt));
  parts.push(banner('info', 'DSM is your fail-safe', 'The original Synology firmware stays on the boot flash and is never modified by synofurb. If Debian does not come up, power-cycle the NAS and select DSM from the boot menu, or run "synofurb recover" from your PC.'));
  if (!b.default_valid) parts.push(banner('err', 'The boot flash default points at no entry', 'Pick an entry below and set it as default to repair this.'));
  if (b.other_entries.length) parts.push(banner('info', 'Other entries on the flash', b.other_entries.join(', ') + ' (not managed on this page)'));
  parts.push(area); draw();
  const f = b.flash || {}, p1 = f.p1 || {}, p2 = f.p2 || {};
  const free = (p) => p.bytes ? usage((p.bytes - (p.free_bytes || 0)), p.bytes) : h('span', {class: 'muted'}, 'unknown');
  parts.push(h('div', {class: 'cols'},
    card('Boot flash', 'disk', badge('info', f.device || 'unknown device', 'disk'), h('dl', {class: 'kv'}, h('dt', {}, 'Boot partition'), h('dd', {}, free(p1), p1.free_bytes != null ? h('span', {class: 'help'}, fmtBytes(p1.free_bytes) + ' free') : null),
      h('dt', {}, 'Data partition'), h('dd', {}, free(p2), p2.free_bytes != null ? h('span', {class: 'help'}, fmtBytes(p2.free_bytes) + ' free') : null), h('dt', {}, 'Total'), h('dd', {}, fmtBytes(f.bytes)))),
    card('DOM backup', 'download', b.backup.exists ? badge('ok', 'Backup exists') : badge('warn', 'No backup yet'), h('p', {}, b.backup.exists ? 'A full image of the boot flash was taken before the first change.' : 'The automatic backup is created before the first write to the boot flash. Keep a copy off the NAS.'),
      b.backup.path ? h('p', {class: 'mono'}, b.backup.path) : null, b.tool_version ? h('p', {class: 'help'}, 'synofurb-dom ' + b.tool_version) : null)));
  return h('div', {}, parts);
};
/* ---------- System ---------- */
const PAGES_ACTIONS = {
  power(action) {
    confirmAction({title: (action === 'reboot' ? 'Reboot' : 'Power off') + ' this NAS', sub: action === 'reboot' ? 'Services stop in order; SMB and NFS clients disconnect.' : 'You will have to press the power button to start it again.',
      expected: S.host, label: action === 'reboot' ? 'Reboot' : 'Power off', icon: 'power',
      onConfirm: async v => { await api('POST', '/api/system/power', {action, confirm: v}); toast(action + ' scheduled'); }});
  }
};
PAGES.system = async () => {
  const [r, s] = await Promise.all([api('GET', '/api/services'), S.st ? Promise.resolve(S.st) : api('GET', '/api/status')]);
  const parts = [pageHead('System', 'Services, host keys and power.')];
  parts.push(h('div', {class: 'cols'},
    card('SSH', 'key', r.sshd.active === 'active' ? badge('ok', 'Running') : badge('neutral', r.sshd.active, 'x'), h('dl', {class: 'kv'}, h('dt', {}, 'Enabled'), h('dd', {}, r.sshd.enabled), h('dt', {}, 'Active'), h('dd', {}, r.sshd.active)),
      h('div', {class: 'cap'}, 'Host key fingerprints'), r.host_keys.length ? r.host_keys.map(k => h('p', {}, h('b', {}, k.type + ' ' + k.bits), h('br'), h('span', {class: 'mono'}, k.fingerprint))) : h('p', {class: 'muted'}, 'No host keys found')),
    card('Serial consoles', 'terminal', null, r.serial_getty.length ? table(['Unit', 'Active', 'Sub'], r.serial_getty.map(g => [h('span', {class: 'mono'}, g.unit), g.active === 'active' ? badge('ok', 'active') : badge('neutral', g.active, 'x'), g.sub])) : h('p', {class: 'muted'}, 'No serial-getty units are active.'),
      s.console.cmdline_consoles.length ? h('p', {class: 'help'}, 'Kernel console: ' + s.console.cmdline_consoles.map(c => c.device + (c.options ? ',' + c.options : '')).join(' ')) : null)));
  const rs = r.restartable.map(u => { const b = btn('Restart ' + u, {size: 'sm', onclick: () => act(b, async () => { await api('POST', '/api/services/restart', {unit: u}); toast(u + ' restarted'); })}); return b; });
  parts.push(h('div', {class: 'cols'},
    card('Restart a service', 'refresh', null, h('p', {class: 'muted'}, 'Only services that are safe to restart while the NAS is in use.'), h('div', {class: 'row'}, rs)),
    card('Power', 'power', null, h('p', {class: 'muted'}, 'Both actions ask you to type the hostname.'), h('div', {class: 'row'}, btn('Reboot', {icon: 'refresh', onclick: () => PAGES_ACTIONS.power('reboot')}), btn('Power off', {kind: 'danger', icon: 'power', onclick: () => PAGES_ACTIONS.power('poweroff')})))));
  const un = s.unit || {};
  parts.push(card('About', 'info', badge('info', 'v' + r.version), h('dl', {class: 'kv'}, h('dt', {}, 'Management plane'), h('dd', {}, 'synofurb-mgmt ' + r.version), h('dt', {}, 'Unit model'), h('dd', {}, un.model || 'unknown'),
    h('dt', {}, 'Kernel'), h('dd', {}, s.kernel + (s.kernel_info && s.kernel_info.ge7 ? '  (Linux 7+)' : '')), h('dt', {}, 'OS'), h('dd', {}, s.os || '-'), h('dt', {}, 'Architecture'), h('dd', {}, s.arch))));
  return h('div', {}, parts);
};
/* ---------- boot ---------- */
showLogin().then(async () => { try { await start(); } catch (e) { /* not signed in */ } });
</script></body></html>
"""

INDEX_HTML = _UI_TEMPLATE.replace("/*__TOKENS__*/", theme_css()).replace("<!--__SPRITE__-->", _ICON_SPRITE)


if __name__ == "__main__":
    sys.exit(main())
