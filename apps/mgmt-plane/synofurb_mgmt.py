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

DEFAULT_VERSION = "0.6.0"


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
    sys_root: str = "/sys"            # where md sysfs attributes are read/written (tests point it into a temp dir)
    proc_root: str = "/proc"
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
RAID_LEVELS = ("none", "raid0", "raid1", "raid5", "raid6", "raid10", "shr1", "shr2")
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
    kind: str = "cmd"            # cmd | mdconf | fstab | mkdir | sysfs | unitfile
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
        if self.kind == "sysfs":
            return "echo %s > %s" % (self.meta.get("value"), self.meta.get("path"))
        if self.kind == "unitfile":
            return "[write %s]" % self.meta.get("path")
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
                   "active": None, "health": None, "degraded": False, "sync": None, "bitmap": None}
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
        bm = re.search(r"bitmap:\s*(\d+)/(\d+) pages \[(\d+)KB\],\s*(\d+)KB chunk", raw)
        if bm:
            cur["bitmap"] = {"pages_used": int(bm.group(1)), "pages": int(bm.group(2)), "chunk_kb": int(bm.group(4))}
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
    if level.startswith("shr"):
        raise ValidationError("SHR layouts are planned by plan_shr_create")
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
    steps += volume_tail_steps(name, vg, fs, mountpoint, size_gib)
    return steps


def volume_tail_steps(name: str, vg: str, fs: str, mountpoint: str, size_gib: int | None) -> list[Step]:
    """lvcreate -> mkfs -> fstab -> mount (shared by plain, RAID and SHR volume creation)."""
    lvp = lv_path(vg, name)
    steps: list[Step] = []
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

    def mdstat(self) -> list[dict]:
        return parse_mdstat(read_text(self.cfg.proc_root + "/mdstat"))

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
    query: str = ""                       # raw query string (add-on logs use ?tail=N)

    @property
    def user(self) -> str | None:
        return self.session.user if self.session else self.audit_user


@dataclass
class Route:
    fn: Callable[[Request], Any]
    auth: bool
    mutating: bool


ROUTES: dict[tuple[str, str], Route] = {}
DYNAMIC_ROUTES: list[tuple[str, "re.Pattern[str]", Route]] = []      # (method, path regex, route): paths with an id in them (add-ons, jobs)


def find_route(method: str, path: str) -> Route | None:
    rt = ROUTES.get((method, path))
    if rt is not None:
        return rt
    return next((r for m_, rx, r in DYNAMIC_ROUTES if m_ == method and rx.fullmatch(path)), None)


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
    except SafetyError as e:
        raise ApiError(409, str(e))
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
        if step.kind == "sysfs":
            path, value = str(step.meta["path"]), str(step.meta["value"])
            if not re.fullmatch(r"/sys/block/md\d{1,4}/md/(sync_action|sync_speed_min|sync_speed_max|stripe_cache_size)", path) or not re.fullmatch(r"[a-z0-9]{1,12}", value):
                return CompletedResult(1, "", "refusing to write %r" % path)
            with open(cfg.sys_root + path[len("/sys"):], "w", encoding="utf-8") as fh:
                fh.write(value + "\n")
            return CompletedResult(0, "wrote %s to %s" % (value, path))
        if step.kind == "unitfile":
            path = str(step.meta["path"])
            if path not in RAID_SCRUB_UNITS:
                return CompletedResult(1, "", "refusing to write %r" % path)
            write_atomic(cfg.p(path), str(step.meta["content"]))
            return CompletedResult(0, "wrote " + path)
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
        "disks": app.lsblk(), "md": {"arrays": app.mdstat(), "scan": scan},
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
    if level.startswith("shr"):
        if any(by_path[d]["type"] != "disk" for d in devices):
            raise ApiError(400, "SHR needs whole disks, not partitions")
        pl = vcall(plan_shr_create, name, {d: by_path[d]["size"] for d in devices}, level, fs, mp, size)
        return {"name": name, "vg": vg, "level": level, "fs": fs, "mountpoint": mp, "usable_bytes": pl.info["usable_bytes"], "warnings": pl.warnings,
                "steps": pl.steps, "shr": pl.info["shr"]}
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
            "md": app.mdstat(), "raid_levels": list(RAID_LEVELS),
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
            "md": app.mdstat(), "lvs": app.lvs()}


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
# 16c. RAID management: inventory, plans, SHR (see docs/RAID.md)
#      Pure parsers and plan functions first (unit-tested), routes at the end.
# ============================================================================

RAID_CREATE_LEVELS = ("raid0", "raid1", "raid5", "raid6", "raid10")
BUSY_ACTIONS = frozenset({"resync", "recover", "recovery", "reshape", "check", "repair"})
SCRUB_ACTIONS = ("check", "repair", "idle")
FOREIGN_FREE_OPS = frozenset({"scrub", "assemble"})        # allowed on foreign arrays without the extra acknowledgement
MIB = 1024 ** 2
SHR_RESERVED_BYTES = 2 * MIB                                # 1 MiB at the start (alignment) + 1 MiB at the end (GPT backup)
RAID_SCRUB_UNITS = ("/etc/systemd/system/synofurb-raid-scrub.service", "/etc/systemd/system/synofurb-raid-scrub.timer")
LEVEL_TRANSITIONS: dict[tuple[str, str], dict] = {
    # (from, to): what mdadm supports and what it needs. delta = change in the number of member devices.
    ("raid1", "raid5"): {"members": 2, "delta": 0, "note": "A 2-disk RAID5 is created; add disks afterwards to gain space."},
    ("raid5", "raid6"): {"delta": 1, "needs_new": 1, "note": "One extra disk is added and becomes the second parity."},
    ("raid6", "raid5"): {"delta": -1, "note": "One disk is released and becomes a spare; protection drops to one failed disk."},
    ("raid0", "raid5"): {"delta": 1, "needs_new": 1, "note": "One extra disk provides the parity; the data is restriped."},
}


class SafetyError(ValidationError):
    """An operation refused by the safety rules (shown to the user as a 409)."""


def valid_md_name(v: Any) -> str:
    s = _need_str(v, "array")
    if not re.fullmatch(r"md\d{1,4}", s):
        raise ValidationError("invalid array name (expected md0, md127, ...)")
    return s


def valid_md_label(v: Any) -> str:
    s = _need_str(v, "array name")
    if not re.fullmatch(r"[a-z][a-z0-9_]{0,23}", s):
        raise ValidationError("array name must match [a-z][a-z0-9_]{0,23}")
    return s


# ---- parsers -----------------------------------------------------------------

_DETAIL_KV = re.compile(r"^\s*([A-Za-z][A-Za-z ]*?)\s*:\s*(.*?)\s*$")
_DETAIL_ROW = re.compile(r"^\s*(\d+|-)\s+(\d+)\s+(\d+)\s+(\d+|-)\s+(.+?)\s*$")


def norm_member_state(raw: str) -> str:
    r = raw.lower()
    if "faulty" in r:
        return "faulty"
    if "removed" in r:
        return "removed"
    if "rebuilding" in r:
        return "rebuilding"
    if "replacement" in r:
        return "replacement"
    if r.startswith("spare"):
        return "spare"
    if "active" in r:
        return "active"
    return r or "unknown"


def _first_int(v: str) -> int | None:
    m = re.match(r"\s*(\d+)", v)
    return int(m.group(1)) if m else None


def parse_mdadm_detail(text: str) -> dict:
    """Parse ``mdadm --detail /dev/mdX``. Sizes are bytes (mdadm prints KiB), members come from the device table."""
    d: dict[str, Any] = {"device": None, "members": []}
    kv: dict[str, str] = {}
    in_table = False
    for line in text.splitlines():
        h = re.match(r"^(/dev/\S+):\s*$", line)
        if h:
            d["device"] = h.group(1)
            continue
        if re.match(r"\s*Number\s+Major\s+Minor\s+RaidDevice", line):
            in_table = True
            continue
        if in_table:
            m = _DETAIL_ROW.match(line)
            if m:
                rest = m.group(5)
                dm = re.search(r"(/dev/\S+)\s*$", rest)
                dev = dm.group(1) if dm else None
                raw_state = (rest[:dm.start()] if dm else rest).strip()
                d["members"].append({"number": None if m.group(1) == "-" else int(m.group(1)), "slot": None if m.group(4) == "-" else int(m.group(4)),
                                     "device": dev, "state": norm_member_state(raw_state), "raw_state": raw_state,
                                     "write_mostly": "writemostly" in raw_state.replace("-", "").lower()})
            continue
        m = _DETAIL_KV.match(line)
        if m:
            kv[m.group(1).strip().lower().replace(" ", "_")] = m.group(2)
    d["version"] = kv.get("version")
    d["level"] = (kv.get("raid_level") or "").strip() or None
    for k_in, k_out in (("array_size", "array_size"), ("used_dev_size", "used_dev_size")):
        n = _first_int(kv.get(k_in, ""))
        d[k_out] = n * 1024 if n is not None else None
    for k in ("raid_devices", "total_devices", "active_devices", "working_devices", "failed_devices", "spare_devices"):
        d[k] = _first_int(kv.get(k, ""))
    st = kv.get("state", "")
    d["state"] = st
    d["state_flags"] = [x.strip() for x in st.split(",") if x.strip()]
    d["layout"] = kv.get("layout")
    ck = re.match(r"(\d+)\s*([KMG]?)", kv.get("chunk_size", ""))
    d["chunk_bytes"] = int(ck.group(1)) * {"": 1, "K": 1024, "M": MIB, "G": 1024 ** 3}[ck.group(2)] if ck else None
    d["bitmap"] = "internal" if kv.get("intent_bitmap", "").lower().startswith("internal") else (kv.get("intent_bitmap") or None)
    d["consistency_policy"] = kv.get("consistency_policy")
    nm = kv.get("name", "")
    nmm = re.match(r"(?:(?P<host>[^:\s]+):)?(?P<label>\S+)\s*(?P<rest>\(.*\))?", nm)
    d["name_raw"] = nm
    d["name_host"] = nmm.group("host") if nmm else None
    d["name_label"] = nmm.group("label") if nmm else None
    d["name_local"] = bool(nmm and nmm.group("rest") and "local to host" in nmm.group("rest"))
    d["uuid"] = kv.get("uuid")
    d["events"] = _first_int(kv.get("events", ""))
    d["progress"] = None
    for key, kind in (("rebuild_status", "recovery"), ("resync_status", "resync"), ("reshape_status", "reshape"), ("check_status", "check")):
        pm = re.match(r"\s*(\d+(?:\.\d+)?)%", kv.get(key, ""))
        if pm:
            d["progress"] = {"kind": kind, "percent": float(pm.group(1))}
    d["delta_devices"] = kv.get("delta_devices")
    d["new_level"] = kv.get("new_level")
    return d


def parse_sync_completed(text: str) -> float | None:
    """``/sys/block/mdX/md/sync_completed`` is "done / total" in sectors, or "none"/"delayed"."""
    m = re.match(r"\s*(\d+)\s*/\s*(\d+)", text or "")
    if not m or int(m.group(2)) == 0:
        return None
    return round(100.0 * int(m.group(1)) / int(m.group(2)), 1)


def read_md_sysfs(cfg: Config, name: str) -> dict:
    """The md attributes that matter for the UI. Missing files (no md support, dry run) give an empty dict."""
    base = "%s/block/%s/md/" % (cfg.sys_root, name)

    def rd(f: str) -> str | None:
        t = read_text(base + f)
        return t.strip() if t else None
    out: dict[str, Any] = {}
    sa = rd("sync_action")
    if sa is not None:
        out["sync_action"] = sa
    mm = rd("mismatch_cnt")
    out["mismatch_cnt"] = int(mm) if mm and mm.isdigit() else None
    for f in ("array_state", "last_sync_action", "bitmap/location", "consistent"):
        v = rd(f)
        if v is not None:
            out[f.replace("/", "_")] = v
    out["sync_completed"] = parse_sync_completed(rd("sync_completed") or "")
    for f in ("sync_speed", "degraded", "stripe_cache_size"):
        v = rd(f)
        out[f] = int(v) if v and v.isdigit() else None
    for f in ("sync_speed_min", "sync_speed_max"):
        v = rd(f)
        mv = re.match(r"(\w+)", v or "")
        out[f] = (int(mv.group(1)) if mv and mv.group(1).isdigit() else "system") if v else None
    return out


# ---- inventory ---------------------------------------------------------------

def array_is_foreign(detail: dict, hostname: str, vg_names: list[str]) -> list[str]:
    """Reasons an array looks like it was made elsewhere (empty list = ours)."""
    why = []
    host = detail.get("name_host")
    if host and host != hostname and not detail.get("name_local"):
        why.append("created on host %s" % host)
    if any(not v.startswith("vg_") for v in vg_names):
        why.append("volume group %s" % ", ".join(v for v in vg_names if not v.startswith("vg_")))
    return why


def build_array_inventory(mdstat: list[dict], details: dict[str, dict], sysfs: dict[str, dict], devs: list[dict],
                          pvs: list[dict], lvs: list[dict], hostname: str) -> list[dict]:
    """Merge mdstat, ``mdadm --detail`` and sysfs into one record per array (pure)."""
    by_k = {d["kname"]: d for d in devs}
    by_path = {d["path"]: d for d in devs}
    out = []
    for a in mdstat:
        name = a["name"]
        det, sysf = details.get(name) or {}, sysfs.get(name) or {}
        dev = by_k.get(name)
        members = []
        if det.get("members"):
            src = det["members"]
        else:
            src = [{"slot": mb["role"], "device": "/dev/" + mb["name"], "state": "faulty" if mb["failed"] else "spare" if mb["spare"] else "active"} for mb in a["members"]]
        for mb in src:
            m = {"device": mb["device"], "name": mb["device"].replace("/dev/", "") if mb["device"] else None, "slot": mb.get("slot"), "state": mb["state"],
                 "write_mostly": bool(mb.get("write_mostly")), "disk": None, "model": None, "serial": None, "size": None}
            ld = by_path.get(mb["device"] or "")
            if ld:
                m["size"] = ld["size"]
                parent = by_k.get(ld["pkname"] or "") if ld["type"] == "part" else ld
                if parent:
                    m["disk"], m["model"], m["serial"] = parent["kname"], parent["model"], parent["serial"]
            members.append(m)
        vg_names = sorted({p["vg_name"] for p in pvs if p["pv_name"] in ("/dev/" + name, "/dev/md/" + str(det.get("name_label") or name)) and p["vg_name"]})
        lv_names, mounts = [], []
        if dev:
            for d in [dev] + list(descendants(by_k, name)):
                mounts += d["mountpoints"]
                if d["type"] == "lvm":
                    for lv in lvs:
                        if d["name"] == mapper_name(lv["vg_name"], lv["lv_name"]):
                            lv_names.append("%s/%s" % (lv["vg_name"], lv["lv_name"]))
                            if lv["vg_name"] not in vg_names:
                                vg_names.append(lv["vg_name"])
        action = sysf.get("sync_action")
        sync = a["sync"]
        if sync is None and action and action in BUSY_ACTIONS:
            sync = {"action": action, "percent": sysf.get("sync_completed"), "finish_min": None, "speed_kbs": sysf.get("sync_speed")}
        if sync is None and det.get("progress"):
            sync = {"action": det["progress"]["kind"], "percent": det["progress"]["percent"], "finish_min": None, "speed_kbs": None}
        if sync and sysf.get("sync_speed") and not sync.get("speed_kbs"):
            sync = dict(sync, speed_kbs=sysf["sync_speed"])
        flags = det.get("state_flags") or []
        degraded = bool(a["degraded"] or "degraded" in flags or (sysf.get("degraded") or 0) > 0)
        reasons = array_is_foreign(det, hostname, vg_names) if det else (["volume group %s" % ", ".join(v for v in vg_names if not v.startswith("vg_"))] if any(not v.startswith("vg_") for v in vg_names) else [])
        out.append({
            "name": name, "device": "/dev/" + name, "label": det.get("name_label") or name, "level": det.get("level") or a["level"],
            "state": ("inactive" if a["state"] == "inactive" else "degraded" if degraded else ", ".join(flags) if flags else a["state"]),
            "flags": flags, "degraded": degraded, "size": det.get("array_size") or a["size"], "used_dev_size": det.get("used_dev_size"),
            "raid_devices": det.get("raid_devices") or a["total"], "chunk_bytes": det.get("chunk_bytes"), "layout": det.get("layout"),
            "bitmap": det.get("bitmap") or ("internal" if a.get("bitmap") else None), "members": members,
            "spares": [m["device"] for m in members if m["state"] == "spare"], "failed": [m["device"] for m in members if m["state"] == "faulty"],
            "sync": sync, "busy": bool(sync and sync.get("action") in BUSY_ACTIONS) or (action in BUSY_ACTIONS if action else False),
            "mismatch_cnt": sysf.get("mismatch_cnt"), "last_sync_action": sysf.get("last_sync_action"), "sync_speed_min": sysf.get("sync_speed_min"),
            "sync_speed_max": sysf.get("sync_speed_max"), "stripe_cache_size": sysf.get("stripe_cache_size"), "uuid": det.get("uuid"), "events": det.get("events"),
            "foreign": bool(reasons), "foreign_reasons": reasons, "vgs": sorted(vg_names), "lvs": sorted(set(lv_names)),
            "mounts": sorted(set(mounts)), "in_use": bool(mounts or lv_names), "holds_system": bool(dev and holds_system(by_k, dev)),
            "inactive": a["state"] == "inactive",
        })
    return sorted(out, key=lambda x: x["name"])


# ---- SHR slice algorithm (pure; mirrored by the Rust planner, vectors in tests/shr_vectors.json) -------------------------

def shr_data_bytes(disk_bytes: int) -> int:
    """Data space of a whole disk after the alignment and GPT reserve."""
    return max(0, disk_bytes - SHR_RESERVED_BYTES)


def shr_slices(disks: dict[str, int], mode: str) -> dict:
    """Cut the data partitions of ``disks`` (name -> bytes) into slices.

    Round sizes down to whole GiB; sizes within 1 GiB of the lowest size of a tier merge into that tier. Slice j spans
    [t(j-1), t(j)) and its members are all disks with size >= t(j). One member: unusable. SHR-1: 2 -> RAID1, >=3 -> RAID5.
    SHR-2: >=4 -> RAID6, 2 or 3 -> RAID1 (with a warning). Usable: RAID1 = len, RAID5 = (m-1) len, RAID6 = (m-2) len."""
    if mode not in ("shr1", "shr2"):
        raise ValidationError("mode must be shr1 or shr2")
    minimum = 2 if mode == "shr1" else 4
    if len(disks) < minimum:
        raise ValidationError("%s needs at least %d disks (got %d)" % ("SHR-1" if mode == "shr1" else "SHR-2", minimum, len(disks)))
    order = {n: i for i, n in enumerate(disks)}
    tiers: list[list] = []                      # [tier_bytes, [names]]
    low = None
    for name, size in sorted(disks.items(), key=lambda kv: (kv[1], order[kv[0]])):
        if low is None or size - low >= GIB:
            low = size
            tiers.append([(size // GIB) * GIB, [name]])
        else:
            tiers[-1][1].append(name)
    slices, warnings, prev, total = [], [], 0, 0
    for j, (t, _) in enumerate(tiers):
        length = t - prev
        prev = t
        if length <= 0:
            continue
        names = {n for _, ns in tiers[j:] for n in ns}
        members = [n for n in disks if n in names]
        m = len(members)
        idx = len(slices) + 1
        note = None
        if m == 1:
            level, usable = None, 0
            warnings.append("Slice %d: %d GiB on %s has no partner disk and stays unused." % (idx, length // GIB, members[0]))
        elif mode == "shr1":
            level = "raid1" if m == 2 else "raid5"
            usable = length if m == 2 else (m - 1) * length
        elif m >= 4:
            level, usable = "raid6", (m - 2) * length
        else:
            level, usable = "raid1", length
            note = "Only %d members: tolerates %d failure(s), not 2." % (m, m - 1)
            warnings.append("Slice %d has %d disks and is mirrored; it tolerates %d failed disk(s), not 2." % (idx, m, m - 1))
        slices.append({"index": idx, "start": t - length, "length": length, "members": members, "level": level, "usable": usable, "unusable": m == 1, "note": note})
        total += usable
    return {"mode": mode, "tiers": [t for t, _ in tiers], "slices": slices, "usable_bytes": total, "raw_bytes": sum(disks.values()), "warnings": warnings}


def part_path(disk: str, n: int) -> str:
    return "%s%s%d" % (disk, "p" if disk[-1].isdigit() else "", n)


def shr_partition_steps(disk: str, parts: list[tuple[str, int, int]], fresh: bool) -> list[Step]:
    """parted steps for one disk. ``parts`` = [(label, start_bytes, length_bytes)] with start relative to the data area."""
    steps = []
    if fresh:
        steps.append(Step("Wipe %s" % disk, ["wipefs", "-a", disk]))
        steps.append(Step("Create a GPT on %s" % disk, ["parted", "-s", disk, "mklabel", "gpt"]))
    for label, start, length in parts:
        s = (MIB + start) // MIB
        steps.append(Step("Create partition %s on %s" % (label, disk),
                          ["parted", "-s", "-a", "optimal", disk, "mkpart", label, "%dMiB" % s, "%dMiB" % (s + length // MIB)]))
    return steps


def raid_create_steps(label: str, level: str, devices: list[str], spares: list[str] | None = None, chunk_kib: int | None = None,
                      bitmap: bool = True) -> list[Step]:
    md = "/dev/md/" + label
    spares = spares or []
    argv = ["mdadm", "--create", md, "--run", "--level=" + level, "--raid-devices=%d" % len(devices), "--metadata=1.2", "--name=" + label]
    if spares:
        argv.append("--spare-devices=%d" % len(spares))
    if chunk_kib and level in ("raid0", "raid5", "raid6", "raid10"):
        argv.append("--chunk=%d" % chunk_kib)
    if bitmap and level != "raid0":
        argv.append("--bitmap=internal")
    return [Step("Create %s array %s from %d devices" % (level, md, len(devices)), argv + devices + spares, timeout=180),
            Step("Wait for device nodes", ["udevadm", "settle", "--timeout=30"], ignore_error=True, timeout=45)]


def md_record_steps() -> list[Step]:
    return [Step("Record arrays in /etc/mdadm/mdadm.conf", ["mdadm", "--detail", "--scan"], kind="mdconf"),
            Step("Update initramfs so the arrays assemble at boot", ["update-initramfs", "-u"], timeout=600)]


@dataclass
class RaidPlan:
    """What the server would do. ``confirm`` is the phrase the user must type (None = no typed confirmation)."""
    op: str
    title: str
    steps: list[Step] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    confirm: str | None = None
    array: str | None = None
    follow_ups: list[Step] = field(default_factory=list)
    info: dict = field(default_factory=dict)

    def to_dict(self) -> dict:
        return dict(self.info, op=self.op, title=self.title, steps=[s.to_dict() for s in self.steps], warnings=self.warnings, confirm=self.confirm,
                    array=self.array, follow_ups=[s.to_dict() for s in self.follow_ups], destructive=self.confirm is not None)


def plan_shr_create(name: str, disks: dict[str, int], mode: str, fs: str, mountpoint: str, size_gib: int | None = None,
                    data_sizes: dict[str, int] | None = None) -> RaidPlan:
    """SHR volume: partition -> one md array per slice -> one volume group -> linear LV -> filesystem.

    ``disks`` maps whole-disk paths to their raw size; slices are computed on the data size (raw minus the reserve)."""
    valid_volume_name(name)
    valid_fs_create(fs)
    valid_mountpoint(mountpoint)
    paths = [valid_device(d) for d in disks]
    if size_gib is not None and (not isinstance(size_gib, int) or size_gib < 1):
        raise ValidationError("size must be a whole number of GiB, or empty for all")
    sizes = data_sizes or {d: shr_data_bytes(s) for d, s in disks.items()}
    lay = shr_slices(sizes, mode)
    usable_slices = [s for s in lay["slices"] if s["level"]]
    if not usable_slices:
        raise ValidationError("these disks give no usable SHR capacity")
    vg = "vg_" + name
    steps: list[Step] = []
    per_disk: dict[str, list] = {}
    for sl in usable_slices:
        for d in sl["members"]:
            per_disk.setdefault(d, []).append((("%s_%d" % (name, sl["index"])), sl["start"], sl["length"]))
    part_of: dict[tuple[str, int], str] = {}
    for d in disks:
        if d in per_disk:
            for k, (label, start, length) in enumerate(per_disk[d], 1):
                part_of[(d, int(label.rsplit("_", 1)[1]))] = part_path(d, k)
            steps += shr_partition_steps(d, per_disk[d], True)
    steps.append(Step("Re-read partition tables", ["partprobe"] + [d for d in disks if d in per_disk], ignore_error=True))
    steps.append(Step("Wait for device nodes", ["udevadm", "settle", "--timeout=30"], ignore_error=True, timeout=45))
    pvs = []
    for sl in usable_slices:
        parts = [part_of[(d, sl["index"])] for d in sl["members"]]
        steps += raid_create_steps("%s_%d" % (name, sl["index"]), sl["level"], parts)
        pvs.append("/dev/md/%s_%d" % (name, sl["index"]))
    steps += md_record_steps()
    steps.append(Step("Create LVM physical volumes", ["pvcreate", "--yes"] + pvs))
    steps.append(Step("Create volume group " + vg, ["vgcreate", vg] + pvs))
    if size_gib is not None and size_gib * GIB > lay["usable_bytes"]:
        raise ValidationError("requested %d GiB exceeds the usable capacity (%d GiB)" % (size_gib, lay["usable_bytes"] // GIB))
    steps += volume_tail_steps(name, vg, fs, mountpoint, size_gib)
    warnings = ["ALL DATA ON THE SELECTED DISKS WILL BE DESTROYED."] + lay["warnings"]
    warnings.append("SHR-style layout managed by Debian; DSM will not adopt it.")
    return RaidPlan("shr-create", "Create %s volume %s" % ("SHR-1" if mode == "shr1" else "SHR-2", name), steps, warnings, confirm=name,
                    info={"shr": lay, "usable_bytes": lay["usable_bytes"], "name": name, "vg": vg, "mountpoint": mountpoint, "fs": fs, "level": mode})


# ---- SHR growth ---------------------------------------------------------------

def shr_layout_from_arrays(arrays: list[dict], volume: str) -> dict:
    """Rebuild the slice layout of an existing synofurb SHR volume from the array inventory.

    Returns {"mode", "slices": [{index, array, label, level, length, members: {disk: partition}}], "disks": {disk: {"size", "parts"}}}."""
    pat = re.compile(r"^%s_(\d+)$" % re.escape(volume))
    slices, disks = [], {}
    for a in arrays:
        m = pat.match(a["label"] or "")
        if not m or ("vg_" + volume) not in a["vgs"]:
            continue
        members = {mb["disk"]: mb["device"] for mb in a["members"] if mb["disk"] and mb["state"] in ("active", "rebuilding", "spare", "faulty")}
        length = min([mb["size"] for mb in a["members"] if mb["size"]] or [a["used_dev_size"] or 0])
        slices.append({"index": int(m.group(1)), "array": a["name"], "label": a["label"], "level": a["level"], "length": length, "members": members,
                       "raid_devices": a["raid_devices"], "degraded": a["degraded"], "busy": a["busy"]})
        for dsk, part in members.items():
            disks.setdefault(dsk, {"parts": []})["parts"].append((int(m.group(1)), part, length))
    slices.sort(key=lambda s: s["index"])
    mode = "shr2" if any(s["level"] == "raid6" for s in slices) else "shr1"
    return {"mode": mode, "slices": slices, "disks": disks, "volume": volume}


def slice_level_for(mode: str, members: int) -> str | None:
    if members < 2:
        return None
    if mode == "shr1":
        return "raid1" if members == 2 else "raid5"
    return "raid6" if members >= 4 else "raid1"


def plan_shr_add_disk(layout: dict, disk: str, disk_bytes: int, backup_dir: str = "/var/lib/synofurb-mgmt") -> RaidPlan:
    """Add one whole disk to an SHR volume.

    1. The new disk joins every existing slice it can still cover (in slice order); each array is grown by one member.
    2. Free space that belongs to no array (the new disk's rest and the free tails of the old disks) is re-sliced with the
       normal algorithm into new arrays, which extend the volume group."""
    vol, mode = layout["volume"], layout["mode"]
    if disk in layout["disks"]:
        raise SafetyError("%s is already part of this volume" % disk)
    for sl in layout["slices"]:
        if sl["busy"] or sl["degraded"]:
            raise SafetyError("array %s is %s; wait until it is healthy" % (sl["array"], "degraded" if sl["degraded"] else "busy"))
    warnings: list[str] = []
    free_new, off_new = shr_data_bytes(disk_bytes), 0
    joined: list[tuple[dict, str, int]] = []                  # (slice, new partition, offset on the new disk)
    for sl in layout["slices"]:
        length = sl["length"] // MIB * MIB
        if free_new < length:
            break
        members = len(sl["members"])
        if sl["level"] == "raid1" and members >= 3 and mode == "shr2":
            warnings.append("Slice %d is a 3-way mirror and cannot become RAID6 in place; its space is not extended." % sl["index"])
            break
        joined.append((sl, part_path(disk, len(joined) + 1), off_new))
        off_new += length
        free_new -= length
    tails = {d: info["size"] - sum(p[2] for p in info["parts"]) for d, info in layout["disks"].items()}
    ends = {d: sum(p[2] for p in info["parts"]) for d, info in layout["disks"].items()}
    tails[disk], ends[disk] = free_new, off_new
    tails = {d: t for d, t in tails.items() if t >= GIB}
    eff = mode if len(tails) >= (2 if mode == "shr1" else 4) else "shr1"
    new_slices = [s for s in shr_slices(tails, eff)["slices"] if s["level"]] if len(tails) >= 2 else []
    if new_slices and eff != mode:
        warnings.append("Fewer than 4 disks have free space: the new slices use SHR-1 protection.")
    base = max([s["index"] for s in layout["slices"]] + [0])
    counts = {d: len(info["parts"]) for d, info in layout["disks"].items()}
    counts[disk] = len(joined)
    new_by_disk: dict[str, list] = {}
    arrays: list[tuple[str, dict, dict]] = []                # (label, slice, {disk: partition})
    for k, sl in enumerate(new_slices, 1):
        label = "%s_%d" % (vol, base + k)
        pm = {}
        for d in sl["members"]:
            counts[d] += 1
            new_by_disk.setdefault(d, []).append((label, ends[d] + sl["start"], sl["length"] // MIB * MIB))
            pm[d] = part_path(d, counts[d])
        arrays.append((label, sl, pm))
    steps: list[Step] = []
    new_disk_parts = [("%s_%d" % (vol, s["index"]), off, s["length"] // MIB * MIB) for s, _, off in joined] + new_by_disk.pop(disk, [])
    if new_disk_parts:
        steps += shr_partition_steps(disk, new_disk_parts, True)
    for d, plist in new_by_disk.items():
        steps += shr_partition_steps(d, plist, False)
    touched = ([disk] if new_disk_parts else []) + list(new_by_disk)
    if touched:
        steps.append(Step("Re-read partition tables", ["partprobe"] + touched, ignore_error=True))
        steps.append(Step("Wait for device nodes", ["udevadm", "settle", "--timeout=30"], ignore_error=True, timeout=45))
    for sl, part, _ in joined:
        md, backup = "/dev/" + sl["array"], "%s/md-%s.backup" % (backup_dir, sl["array"])
        members = len(sl["members"])
        if sl["level"] == "raid1" and members == 2 and mode == "shr1":
            steps += [Step("Convert %s to RAID5" % sl["array"], ["mdadm", "--grow", md, "--level=5", "--backup-file=" + backup], timeout=120),
                      Step("Add %s to %s" % (part, sl["array"]), ["mdadm", "--manage", md, "--add", part]),
                      Step("Reshape %s to 3 devices" % sl["array"], ["mdadm", "--grow", md, "--raid-devices=3", "--backup-file=" + backup], timeout=120)]
        elif sl["level"] == "raid1":
            steps += [Step("Add %s to %s" % (part, sl["array"]), ["mdadm", "--manage", md, "--add", part]),
                      Step("Grow the mirror %s to %d devices" % (sl["array"], members + 1), ["mdadm", "--grow", md, "--raid-devices=%d" % (members + 1)])]
        else:
            steps += [Step("Add %s to %s" % (part, sl["array"]), ["mdadm", "--manage", md, "--add", part]),
                      Step("Reshape %s to %d devices" % (sl["array"], members + 1), ["mdadm", "--grow", md, "--raid-devices=%d" % (members + 1), "--backup-file=" + backup], timeout=120)]
    pvs = []
    for label, sl, pm in arrays:
        steps += raid_create_steps(label, sl["level"], [pm[d] for d in sl["members"]])
        pvs.append("/dev/md/" + label)
    if arrays:
        steps += md_record_steps()
        steps.append(Step("Create LVM physical volumes", ["pvcreate", "--yes"] + pvs))
        steps.append(Step("Add the new arrays to vg_" + vol, ["vgextend", "vg_" + vol] + pvs))
    if not steps:
        raise SafetyError("%s adds no usable capacity to this volume" % disk)
    follow = []
    if joined:
        follow.append(Step("Grow the physical volumes", ["pvresize"] + ["/dev/" + s["array"] for s, _, _ in joined], ignore_error=True))
    follow.append(Step("Extend the logical volume into the new space", ["lvextend", "-l", "+100%FREE", "/dev/vg_%s/%s" % (vol, vol)]))
    warnings.insert(0, "Reshaping runs for hours; the volume stays usable but slower. Do not remove disks meanwhile.")
    return RaidPlan("shr-add-disk", "Add %s to volume %s" % (disk, vol), steps, warnings, confirm=vol, follow_ups=follow,
                    info={"volume": vol, "joined_slices": [s["index"] for s, _, _ in joined], "new_slices": [label for label, _, _ in arrays], "after_reshape": True})


def plan_shr_replace_disk(layout: dict, old: str, new: str, new_bytes: int, old_failed: bool) -> RaidPlan:
    """Replace one disk of an SHR volume by a disk at least as large: same partitions, then --replace (or fail/remove/add) per array."""
    vol = layout["volume"]
    info = layout["disks"].get(old)
    if not info:
        raise SafetyError("%s is not part of volume %s" % (old, vol))
    if new in layout["disks"]:
        raise SafetyError("%s is already part of this volume" % new)
    need = sum(p[2] for p in info["parts"])
    if shr_data_bytes(new_bytes) < need:
        raise SafetyError("the new disk is too small (%d GiB needed)" % (need // GIB + 1))
    if not old_failed and any(s["degraded"] for s in layout["slices"]):
        raise SafetyError("a slice is degraded; rebuild it first or replace the failed disk")
    if any(s["busy"] for s in layout["slices"]):
        raise SafetyError("an array is busy (sync or reshape); wait until it is idle")
    parts = sorted(info["parts"])
    plist, off = [], 0
    for idx, _, length in parts:
        plist.append(("%s_%d" % (vol, idx), off, length))
        off += length
    steps = shr_partition_steps(new, plist, True)
    steps.append(Step("Re-read partition tables", ["partprobe", new], ignore_error=True))
    steps.append(Step("Wait for device nodes", ["udevadm", "settle", "--timeout=30"], ignore_error=True, timeout=45))
    later: list[Step] = []
    for k, (idx, oldpart, _) in enumerate(parts, 1):
        sl = next(s for s in layout["slices"] if s["index"] == idx)
        md, newpart = "/dev/" + sl["array"], part_path(new, k)
        if old_failed:
            steps += [Step("Remove %s from %s" % (oldpart, sl["array"]), ["mdadm", "--manage", md, "--fail", oldpart], ignore_error=True),
                      Step("Remove %s from %s" % (oldpart, sl["array"]), ["mdadm", "--manage", md, "--remove", oldpart], ignore_error=True),
                      Step("Add %s to %s" % (newpart, sl["array"]), ["mdadm", "--manage", md, "--add", newpart])]
        else:
            steps += [Step("Add %s to %s as a spare" % (newpart, sl["array"]), ["mdadm", "--manage", md, "--add", newpart]),
                      Step("Replace %s by %s in %s" % (oldpart, newpart, sl["array"]), ["mdadm", "--manage", md, "--replace", oldpart, "--with", newpart])]
            later.append(Step("When %s has finished: remove %s" % (sl["array"], oldpart), ["mdadm", "--manage", md, "--remove", oldpart]))
    warnings = ["Rebuilding takes hours; keep the other disks healthy until it finishes."]
    if shr_data_bytes(new_bytes) >= need + GIB:
        warnings.append("The new disk is larger: the extra space can be added with 'Add disk' afterwards.")
    return RaidPlan("shr-replace-disk", "Replace %s by %s in %s" % (old, new, vol), steps, warnings, confirm=vol, follow_ups=later,
                    info={"volume": vol, "old": old, "new": new})


# ---- operations on a single array --------------------------------------------

def min_active(level: str | None, n: int) -> int:
    """Smallest number of working members that still holds all data."""
    return {"raid0": n, "raid1": 1, "raid4": n - 1, "raid5": n - 1, "raid6": n - 2, "raid10": max(1, n // 2)}.get(level or "", n)


def fault_tolerance(level: str | None, n: int) -> int:
    return max(0, n - min_active(level, n)) if level != "raid10" else 1


def _need_idle(arr: dict, what: str) -> None:
    if arr["busy"]:
        raise SafetyError("%s is %s; wait until it is idle before %s" % (arr["name"], (arr["sync"] or {}).get("action") or "busy", what))


def _need_member(arr: dict, dev: str) -> dict:
    m = next((x for x in arr["members"] if x["device"] == dev), None)
    if m is None:
        raise ValidationError("%s is not a member of %s" % (dev, arr["name"]))
    return m


def _free_device(devs_by_path: dict[str, dict], by_k: dict[str, dict], dev: str, min_bytes: int = 0) -> dict:
    d = devs_by_path.get(dev)
    if d is None:
        raise ValidationError("%s: no such block device" % dev)
    why = device_unusable_reason(d, by_k)
    if why:
        raise SafetyError("%s cannot be used: %s" % (dev, why))
    if min_bytes and d["size"] < min_bytes:
        raise SafetyError("%s is smaller than the array members (%d GiB needed)" % (dev, min_bytes // GIB + 1))
    return d


def _member_size(arr: dict) -> int:
    return arr["used_dev_size"] or min([m["size"] for m in arr["members"] if m["size"]] or [0])


def backup_file(cfg_state: str, arr: dict) -> str:
    return "%s/md-%s.backup" % (cfg_state, arr["name"])


def plan_add_spare(arr: dict, dev: str, free: dict) -> RaidPlan:
    d = free
    return RaidPlan("add-spare", "Add spare to %s" % arr["name"], [Step("Add %s to %s as a spare" % (dev, arr["name"]), ["mdadm", "--manage", arr["device"], "--add", dev])],
                    ["%s will be overwritten." % dev] + (["The array is degraded: the spare starts rebuilding at once."] if arr["degraded"] else []),
                    confirm=arr["name"], array=arr["name"], info={"size": d["size"]})


def plan_fail(arr: dict, dev: str) -> RaidPlan:
    m = _need_member(arr, dev)
    _need_idle(arr, "failing a disk")
    if m["state"] in ("faulty", "removed"):
        raise SafetyError("%s has already failed; remove it instead" % dev)
    active = [x for x in arr["members"] if x["state"] == "active"]
    if m["state"] == "active" and len(active) - 1 < min_active(arr["level"], arr["raid_devices"] or len(active)):
        raise SafetyError("failing %s would make %s unreadable" % (dev, arr["name"]))
    return RaidPlan("fail", "Fail %s in %s" % (dev, arr["name"]), [Step("Mark %s as failed in %s" % (dev, arr["name"]), ["mdadm", "--manage", arr["device"], "--fail", dev])],
                    ["%s stays in the array as failed until you remove it." % dev], confirm=arr["name"], array=arr["name"])


def plan_remove(arr: dict, dev: str) -> RaidPlan:
    m = _need_member(arr, dev)
    if m["state"] == "active" or m["state"] == "rebuilding":
        raise SafetyError("%s is working; fail it first" % dev)
    return RaidPlan("remove", "Remove %s from %s" % (dev, arr["name"]), [Step("Remove %s from %s" % (dev, arr["name"]), ["mdadm", "--manage", arr["device"], "--remove", dev])],
                    [], confirm=arr["name"], array=arr["name"])


def plan_replace(arr: dict, old: str, new: str, free: dict) -> RaidPlan:
    m = _need_member(arr, old)
    _need_idle(arr, "replacing a disk")
    if (arr["level"] or "") == "raid0":
        raise SafetyError("RAID0 has no redundancy to replace from")
    md = arr["device"]
    if m["state"] == "faulty":
        steps = [Step("Remove %s from %s" % (old, arr["name"]), ["mdadm", "--manage", md, "--remove", old], ignore_error=True),
                 Step("Add %s to %s" % (new, arr["name"]), ["mdadm", "--manage", md, "--add", new])]
        follow, warn = [], ["The new disk is rebuilt from the others; the array is exposed until it finishes."]
    else:
        if arr["degraded"]:
            raise SafetyError("%s is degraded; replace the failed disk first" % arr["name"])
        if m["state"] != "active":
            raise SafetyError("%s is %s and cannot be replaced" % (old, m["state"]))
        steps = [Step("Add %s to %s as a spare" % (new, arr["name"]), ["mdadm", "--manage", md, "--add", new]),
                 Step("Copy %s onto %s" % (old, new), ["mdadm", "--manage", md, "--replace", old, "--with", new])]
        follow = [Step("When the copy has finished: remove %s" % old, ["mdadm", "--manage", md, "--remove", old])]
        warn = ["Redundancy is kept during the copy. %s becomes a failed member when it is done." % old]
    return RaidPlan("replace", "Replace %s in %s" % (old, arr["name"]), steps, warn, confirm=arr["name"], array=arr["name"], follow_ups=follow, info={"mode": "failed" if m["state"] == "faulty" else "healthy"})


def after_grow_follow_ups(arr: dict, lvs: list[dict], devs: list[dict]) -> list[Step]:
    """pvresize + lvextend + filesystem grow for every logical volume that lives on the array."""
    steps = [Step("Grow the physical volume on %s" % arr["name"], ["pvresize", arr["device"]])]
    by_name = {d["name"]: d for d in devs}
    for lv in lvs:
        if "%s/%s" % (lv["vg_name"], lv["lv_name"]) in arr["lvs"]:
            d = by_name.get(mapper_name(lv["vg_name"], lv["lv_name"]))
            mp = d["mountpoints"][0] if d and d["mountpoints"] else None
            try:
                steps += plan_volume_extend(lv["vg_name"], lv["lv_name"], d["fstype"] if d else None, mp, None)
            except ValidationError:
                steps.append(Step("Extend %s/%s" % (lv["vg_name"], lv["lv_name"]), ["lvextend", "-l", "+100%FREE", lv["lv_path"]]))
    return steps


def plan_grow(arr: dict, add: list[str], size_max: bool, state_dir: str, follow_ups: list[Step]) -> RaidPlan:
    _need_idle(arr, "growing")
    lvl = arr["level"]
    if arr["degraded"]:
        raise SafetyError("%s is degraded; rebuild it before growing" % arr["name"])
    if lvl not in ("raid1", "raid5", "raid6"):
        raise SafetyError("%s cannot be grown" % (lvl or "this array"))
    if add and size_max:
        raise ValidationError("add disks and use all space in two separate steps")
    if not add and not size_max and not arr["spares"]:
        raise ValidationError("choose disks to add, or use all space")
    md = arr["device"]
    steps: list[Step] = []
    warnings: list[str] = []
    if size_max:
        steps.append(Step("Use all space on the members of %s" % arr["name"], ["mdadm", "--grow", md, "--size=max"], timeout=120))
        warnings.append("Only useful after every member was replaced by a larger disk.")
    else:
        n = arr["raid_devices"] or len(arr["members"])
        steps += [Step("Add %s to %s" % (d, arr["name"]), ["mdadm", "--manage", md, "--add", d]) for d in add]
        n_new = n + len(add) + (len(arr["spares"]) if not add else 0)
        argv = ["mdadm", "--grow", md, "--raid-devices=%d" % n_new]
        if lvl in ("raid5", "raid6"):
            argv.append("--backup-file=" + backup_file(state_dir, arr))
        steps.append(Step("Reshape %s to %d devices" % (arr["name"], n_new), argv, timeout=120))
        warnings.append("The reshape runs for hours; the array stays usable but slower.")
    return RaidPlan("grow", "Grow %s" % arr["name"], steps, warnings, confirm=arr["name"], array=arr["name"], follow_ups=follow_ups, info={"after_reshape": True})


def plan_change_level(arr: dict, target: str, add: list[str], state_dir: str) -> RaidPlan:
    _need_idle(arr, "changing the level")
    if arr["degraded"]:
        raise SafetyError("%s is degraded; rebuild it first" % arr["name"])
    key = (arr["level"], target)
    tr = LEVEL_TRANSITIONS.get(key)
    if not tr:
        ok = "; ".join("%s to %s" % k for k in sorted(LEVEL_TRANSITIONS))
        raise SafetyError("mdadm cannot convert %s to %s. Supported: %s" % (arr["level"], target, ok))
    n = arr["raid_devices"] or len(arr["members"])
    if tr.get("members") and n != tr["members"]:
        raise SafetyError("%s to %s needs exactly %d members (this array has %d)" % (key[0], key[1], tr["members"], n))
    need = tr.get("needs_new", 0)
    if need > len(add) + len(arr["spares"]):
        raise SafetyError("add %d more disk(s) first" % (need - len(add) - len(arr["spares"])))
    md = arr["device"]
    steps = [Step("Add %s to %s" % (d, arr["name"]), ["mdadm", "--manage", md, "--add", d]) for d in add]
    n_new = n + tr["delta"] + (len(add) if key == ("raid1", "raid5") else 0)
    argv = ["mdadm", "--grow", md, "--level=" + target[4:]]
    if n_new != n:
        argv.append("--raid-devices=%d" % n_new)
    argv.append("--backup-file=" + backup_file(state_dir, arr))
    steps.append(Step("Convert %s from %s to %s" % (arr["name"], key[0], target), argv, timeout=120))
    return RaidPlan("change-level", "Change %s to %s" % (arr["name"], target), steps, [tr["note"], "The conversion rewrites all data and runs for hours; do not power off."],
                    confirm=arr["name"], array=arr["name"], info={"from": key[0], "to": target, "after_reshape": True})


def plan_scrub(arr: dict, action: str) -> RaidPlan:
    if action not in SCRUB_ACTIONS:
        raise ValidationError("action must be one of: " + ", ".join(SCRUB_ACTIONS))
    if arr["inactive"]:
        raise SafetyError("%s is inactive" % arr["name"])
    if action != "idle":
        if (arr["level"] or "") in ("raid0", "linear", None):
            raise SafetyError("%s has no redundancy to check" % arr["name"])
        _need_idle(arr, "starting a check")
        if arr["degraded"] and action == "repair":
            raise SafetyError("repair on a degraded array is refused; rebuild first")
    title = {"check": "Check", "repair": "Repair", "idle": "Stop the running task on"}[action]
    warn = ["Repair rewrites blocks that disagree; use it only after a check found mismatches."] if action == "repair" else []
    return RaidPlan("scrub", "%s %s" % (title, arr["name"]), [Step("%s %s" % (title, arr["name"]), None, kind="sysfs", meta={"path": "/sys/block/%s/md/sync_action" % arr["name"], "value": action})],
                    warn, confirm=None, array=arr["name"], info={"action": action, "writes": action == "repair"})


def plan_stop(arr: dict) -> RaidPlan:
    if arr["mounts"]:
        raise SafetyError("%s is mounted at %s" % (arr["name"], ", ".join(arr["mounts"])))
    if arr["holds_system"]:
        raise SafetyError("%s holds the system" % arr["name"])
    if arr["lvs"]:
        raise SafetyError("%s is in use by %s" % (arr["name"], ", ".join(arr["lvs"])))
    if arr["busy"]:
        raise SafetyError("%s is %s; stopping now would interrupt it" % (arr["name"], (arr["sync"] or {}).get("action")))
    return RaidPlan("stop", "Stop %s" % arr["name"], [Step("Stop %s" % arr["name"], ["mdadm", "--stop", arr["device"]])], [], confirm=arr["name"], array=arr["name"])


def plan_assemble() -> RaidPlan:
    return RaidPlan("assemble", "Assemble arrays", [Step("Assemble arrays", ["mdadm", "--assemble", "--scan"], ignore_error=True, timeout=120),
                                                     Step("Scan LVM", ["vgscan"], ignore_error=True), Step("Activate logical volumes", ["vgchange", "-ay"], ignore_error=True)])


def scrub_unit_files() -> list[Step]:
    svc = ("[Unit]\nDescription=Check all RAID arrays (synofurb)\nConditionPathExists=/opt/synofurb-mgmt/synofurb_mgmt.py\n\n"
           "[Service]\nType=oneshot\nExecStart=/usr/bin/python3 /opt/synofurb-mgmt/synofurb_mgmt.py --raid-scrub\n")
    tmr = ("[Unit]\nDescription=Monthly RAID check (synofurb)\n\n[Timer]\nOnCalendar=monthly\nRandomizedDelaySec=6h\nPersistent=true\n\n"
           "[Install]\nWantedBy=timers.target\n")
    return [Step("Write the check service", None, kind="unitfile", meta={"path": RAID_SCRUB_UNITS[0], "content": svc}),
            Step("Write the monthly timer", None, kind="unitfile", meta={"path": RAID_SCRUB_UNITS[1], "content": tmr}),
            Step("Reload systemd units", ["systemctl", "daemon-reload"]),
            Step("Enable the monthly check", ["systemctl", "enable", "--now", "synofurb-raid-scrub.timer"])]


def plan_tuning(arr: dict | None, body: dict) -> RaidPlan:
    steps: list[Step] = []
    if body.get("scrub_schedule") in ("enable", "disable"):
        steps += scrub_unit_files() if body["scrub_schedule"] == "enable" else [Step("Disable the monthly check", ["systemctl", "disable", "--now", "synofurb-raid-scrub.timer"], ignore_error=True)]
    if arr is not None:
        for key, fname in (("speed_min", "sync_speed_min"), ("speed_max", "sync_speed_max")):
            v = body.get(key)
            if v in (None, ""):
                continue
            if v != "system" and not (isinstance(v, int) and not isinstance(v, bool) and 0 <= v <= 4_000_000):
                raise ValidationError("%s must be 'system' or a whole number of KiB/s up to 4000000" % key)
            steps.append(Step("Set %s of %s" % (fname, arr["name"]), None, kind="sysfs", meta={"path": "/sys/block/%s/md/%s" % (arr["name"], fname), "value": str(v)}))
        sc = body.get("stripe_cache")
        if sc not in (None, ""):
            if not (isinstance(sc, int) and not isinstance(sc, bool) and 17 <= sc <= 32768) or (arr["level"] not in ("raid5", "raid6")):
                raise ValidationError("stripe cache is 17-32768 pages and only for RAID5/6")
            steps.append(Step("Set the stripe cache of %s" % arr["name"], None, kind="sysfs", meta={"path": "/sys/block/%s/md/stripe_cache_size" % arr["name"], "value": str(sc)}))
        bm = body.get("bitmap")
        if bm in ("internal", "none"):
            if (arr["level"] or "") == "raid0":
                raise SafetyError("RAID0 has no bitmap")
            steps.append(Step("Set the write-intent bitmap of %s to %s" % (arr["name"], bm), ["mdadm", "--grow", arr["device"], "--bitmap=" + bm], timeout=60))
    if not steps:
        raise ValidationError("nothing to change")
    return RaidPlan("tuning", "Tune %s" % (arr["name"] if arr else "RAID checks"), steps, [], confirm=None, array=arr["name"] if arr else None)


def plan_raid_create(label: str, level: str, devices: list[str], spares: list[str], chunk_kib: int | None, bitmap: bool, sizes: dict[str, int],
                     volume: dict | None = None) -> RaidPlan:
    valid_md_label(label)
    if level not in RAID_CREATE_LEVELS:
        raise ValidationError("level must be one of: " + ", ".join(RAID_CREATE_LEVELS))
    allp = [valid_device(d) for d in devices + spares]
    if len(set(allp)) != len(allp):
        raise ValidationError("duplicate devices selected")
    check_raid_counts(level, len(devices))
    if spares and level == "raid0":
        raise ValidationError("RAID0 cannot have spares")
    if chunk_kib is not None and (not isinstance(chunk_kib, int) or chunk_kib not in (16, 32, 64, 128, 256, 512, 1024, 2048, 4096)):
        raise ValidationError("chunk size must be 16, 32, 64, 128, 256, 512, 1024, 2048 or 4096 KiB")
    steps = [Step("Wipe old signatures on %s" % d, ["wipefs", "-a", d]) for d in allp]
    steps += raid_create_steps(label, level, devices, spares, chunk_kib, bitmap)
    steps += md_record_steps()
    sz = [sizes[d] for d in devices if d in sizes]
    warnings = ["ALL DATA ON THE SELECTED DEVICES WILL BE DESTROYED."]
    if sz and max(sz) > min(sz) * 1.1:
        warnings.append("Devices differ in size; only the smallest size is used per member.")
    if level == "raid0":
        warnings.append("RAID0 has no redundancy.")
    info = {"usable_bytes": estimate_usable(level, sz) if sz else 0, "tolerance": fault_tolerance(level, len(devices)), "label": label, "level": level}
    if volume:
        valid_fs_create(volume.get("fs", "xfs"))
        vg, mp = "vg_" + label, valid_mountpoint(volume.get("mountpoint") or "/srv/" + label)
        steps.append(Step("Create LVM physical volume", ["pvcreate", "--yes", "/dev/md/" + label]))
        steps.append(Step("Create volume group " + vg, ["vgcreate", vg, "/dev/md/" + label]))
        steps += volume_tail_steps(label, vg, volume.get("fs", "xfs"), mp, volume.get("size_gib"))
        info["mountpoint"] = mp
    return RaidPlan("create", "Create %s array %s" % (level, label), steps, warnings, confirm=label, info=info)


# ---- context + routes ----------------------------------------------------------

@dataclass
class RaidCtx:
    app: App
    devs: list[dict]
    by_k: dict[str, dict]
    by_path: dict[str, dict]
    arrays: list[dict]
    lvs: list[dict]
    mdadm: bool

    def array(self, name: Any) -> dict:
        n = vcall(valid_md_name, name)
        a = next((x for x in self.arrays if x["name"] == n), None)
        if a is None:
            raise ApiError(404, "no such array %s" % n)
        return a


def gather_raid_ctx(app: App) -> RaidCtx:
    devs = app.lsblk()
    stat = app.mdstat()
    have = bool(app.runner.which("mdadm"))
    details = {}
    if have:
        for a in stat:
            r = app.run(["mdadm", "--detail", "/dev/" + a["name"]], timeout=20)
            if r.ok and r.out.strip():
                details[a["name"]] = parse_mdadm_detail(r.out)
    sysfs = {a["name"]: read_md_sysfs(app.cfg, a["name"]) for a in stat}
    lvs = app.lvs()
    arrays = build_array_inventory(stat, details, sysfs, devs, app.pvs(), lvs, socket.gethostname())
    return RaidCtx(app, devs, {d["kname"]: d for d in devs}, {d["path"]: d for d in devs}, arrays, lvs, have)


def _dev_list(v: Any, what: str = "devices") -> list[str]:
    if v in (None, ""):
        return []
    if not isinstance(v, list):
        raise ApiError(400, "%s must be a list" % what)
    return [vcall(valid_device, d) for d in v]


def shr_volume_names(ctx: RaidCtx) -> list[str]:
    names = set()
    for a in ctx.arrays:
        m = re.match(r"^(.+)_\d+$", a["label"] or "")
        if m and ("vg_" + m.group(1)) in a["vgs"]:
            names.add(m.group(1))
    return sorted(names)


def make_raid_plan(ctx: RaidCtx, op: str, b: dict) -> tuple[RaidPlan, dict | None]:
    """Build the plan for ``op`` from live state. Returns (plan, array record or None)."""
    app = ctx.app
    state = app.cfg.state_dir
    if op in ("create", "add-spare", "fail", "remove", "replace", "grow", "change-level", "scrub", "stop", "assemble", "tuning", "shr-create", "shr-add-disk", "shr-replace-disk") and not ctx.mdadm:
        raise ApiError(409, "mdadm is not installed (apt install mdadm)")
    if op == "assemble":
        return plan_assemble(), None
    if op == "tuning" and not b.get("array"):
        return vcall(plan_tuning, None, b), None
    if op == "create":
        label = vcall(valid_md_label, b.get("name"))
        level = b.get("level")
        devs, spares = _dev_list(b.get("devices")), _dev_list(b.get("spares"), "spares")
        sizes = {}
        for d in devs + spares:
            sizes[d] = _free_device(ctx.by_path, ctx.by_k, d)["size"] if d in ctx.by_path else _raise(ValidationError("%s: no such block device" % d))
        if any(a["label"] == label or a["name"] == label for a in ctx.arrays):
            raise ApiError(409, "an array named %s already exists" % label)
        vol = b.get("volume") if isinstance(b.get("volume"), dict) else None
        if vol and any(v["vg_name"] == "vg_" + label for v in app.vgs()):
            raise ApiError(409, "volume group vg_%s already exists" % label)
        chunk = b.get("chunk_kib")
        return vcall(plan_raid_create, label, level, devs, spares, chunk if chunk not in (None, "") else None, b.get("bitmap", True) is not False, sizes, vol), None
    if op == "shr-create":
        mode, name = b.get("mode") or b.get("level"), b.get("name")
        disks = _dev_list(b.get("devices"), "disks")
        sizes = {}
        for d in disks:
            dev = ctx.by_path.get(d)
            if dev is None:
                raise ApiError(400, "%s: no such block device" % d)
            if dev["type"] != "disk":
                raise ApiError(400, "%s is a partition; SHR needs whole disks" % d)
            _free_device(ctx.by_path, ctx.by_k, d)
            sizes[d] = dev["size"]
        vg = "vg_" + str(name)
        if any(v["vg_name"] == vg for v in app.vgs()):
            raise ApiError(409, "volume group %s already exists" % vg)
        mp = b.get("mountpoint") or "/srv/" + str(name)
        size = None if b.get("size_gib") in (None, "", "all") else _int(b.get("size_gib"), "size_gib")
        return vcall(plan_shr_create, name, sizes, mode, b.get("fs", "xfs"), mp, size), None
    if op in ("shr-add-disk", "shr-replace-disk"):
        vol = vcall(valid_volume_name, b.get("volume"))
        if vol not in shr_volume_names(ctx):
            raise ApiError(404, "no SHR volume %s created here" % vol)
        arrs = [a for a in ctx.arrays if a["label"] and re.match(r"^%s_\d+$" % re.escape(vol), a["label"])]
        if any(a["foreign"] for a in arrs):
            raise SafetyError("%s was created by another system" % vol)
        lay = shr_layout_from_arrays(ctx.arrays, vol)
        for dsk, info in lay["disks"].items():
            dd = ctx.by_k.get(dsk)
            info["size"] = shr_data_bytes(dd["size"]) if dd else sum(p[2] for p in info["parts"])
        new = vcall(valid_device, b.get("device"))
        nd = ctx.by_path.get(new)
        if nd is None or nd["type"] != "disk":
            raise ApiError(400, "%s must be a whole disk" % new)
        _free_device(ctx.by_path, ctx.by_k, new)
        if op == "shr-add-disk":
            return plan_shr_add_disk({**lay, "disks": {"/dev/" + k: v for k, v in lay["disks"].items()}}, new, nd["size"], state), None
        old = vcall(valid_device, b.get("old"))
        oldk = old.replace("/dev/", "")
        failed = any(mb["disk"] == oldk and mb["state"] in ("faulty", "removed") for a in arrs for mb in a["members"])
        plan = plan_shr_replace_disk({**lay, "disks": {"/dev/" + k: v for k, v in lay["disks"].items()}}, old, new, nd["size"], failed)
        return plan, None
    arr = ctx.array(b.get("array"))
    dev = None
    if op in ("add-spare", "fail", "remove"):
        dev = vcall(valid_device, b.get("device"))
    if op == "add-spare":
        d = _free_device(ctx.by_path, ctx.by_k, dev, _member_size(arr))
        return plan_add_spare(arr, dev, d), arr
    if op == "fail":
        return vcall(plan_fail, arr, dev), arr
    if op == "remove":
        return vcall(plan_remove, arr, dev), arr
    if op == "replace":
        old, new = vcall(valid_device, b.get("old")), vcall(valid_device, b.get("new"))
        d = _free_device(ctx.by_path, ctx.by_k, new, _member_size(arr))
        return vcall(plan_replace, arr, old, new, d), arr
    if op == "grow":
        add = _dev_list(b.get("add"), "add")
        for d in add:
            _free_device(ctx.by_path, ctx.by_k, d, _member_size(arr))
        return vcall(plan_grow, arr, add, bool(b.get("size_max")), state, after_grow_follow_ups(arr, ctx.lvs, ctx.devs)), arr
    if op == "change-level":
        add = _dev_list(b.get("add"), "add")
        for d in add:
            _free_device(ctx.by_path, ctx.by_k, d, _member_size(arr))
        return vcall(plan_change_level, arr, b.get("target"), add, state), arr
    if op == "scrub":
        return vcall(plan_scrub, arr, b.get("action", "check")), arr
    if op == "stop":
        return vcall(plan_stop, arr), arr
    if op == "tuning":
        return vcall(plan_tuning, arr, b), arr
    raise ApiError(404, "unknown RAID operation")


def _raise(e: Exception) -> Any:
    raise e


def _int(v: Any, what: str) -> int:
    try:
        return int(v)
    except (TypeError, ValueError):
        raise ApiError(400, "%s must be a whole number" % what)


def raid_foreign_gate(op: str, arr: dict | None, plan: RaidPlan, b: dict, execute: bool) -> None:
    """Foreign arrays: only show, assemble, scrub (check/idle). Anything else needs the extra acknowledgement and the typed array name."""
    if arr is None or not arr["foreign"] or op in FOREIGN_FREE_OPS and not plan.info.get("writes"):
        return
    plan.warnings.insert(0, "This array was not created by synofurb (%s). Changes can make DSM unable to use it." % "; ".join(arr["foreign_reasons"]))
    plan.confirm = plan.confirm or arr["name"]
    plan.info["needs_foreign_ack"] = True
    if execute and b.get("foreign_ack") is not True:
        raise ApiError(400, "this array was not created here: tick 'I understand this is not mine' to change it")


def raid_status(app: App, ctx: RaidCtx) -> dict:
    cands, excl = candidate_devices(ctx.devs)
    speed = {"min": None, "max": None}
    for k in ("min", "max"):
        t = read_text("%s/sys/dev/raid/speed_limit_%s" % (app.cfg.proc_root, k)).strip()
        speed[k] = int(t) if t.isdigit() else None
    timer = {"enabled": None, "active": None, "installed": os.path.exists(app.cfg.p(RAID_SCRUB_UNITS[1]))}
    if ctx.mdadm:
        timer["enabled"] = app.run(["systemctl", "is-enabled", "synofurb-raid-scrub.timer"], timeout=10).out.strip() or "disabled"
        timer["active"] = app.run(["systemctl", "is-active", "synofurb-raid-scrub.timer"], timeout=10).out.strip() or "inactive"
    return {"ok": True, "available": ctx.mdadm, "arrays": ctx.arrays, "candidates": cands, "excluded": excl, "speed_limits": speed, "scrub_timer": timer,
            "shr_volumes": shr_volume_names(ctx), "levels": {k: {"min": RAID_MIN[k]} for k in RAID_MIN},
            "monitor": {"service": app.run(["systemctl", "is-active", "mdmonitor"], timeout=10).out.strip() or "unknown"} if ctx.mdadm else None}


@route("GET", "/api/raid")
def api_raid(req: Request) -> dict:
    return raid_status(req.app, gather_raid_ctx(req.app))


def raid_run(req: Request, op: str, execute: bool) -> dict:
    app, b = req.app, req.body
    ctx = gather_raid_ctx(app)
    try:
        plan, arr = make_raid_plan(ctx, op, b)
    except SafetyError as e:
        raise ApiError(409, str(e))
    except ValidationError as e:
        raise ApiError(400, str(e))
    raid_foreign_gate(op, arr, plan, b, execute)
    if not execute:
        return dict(plan.to_dict(), ok=True)
    if plan.confirm:
        need_confirm(b, plan.confirm)
    ok, results = run_steps(app, plan.steps)
    out = {"ok": ok, "op": op, "results": results, "follow_ups": [s.to_dict() for s in plan.follow_ups], "title": plan.title}
    if not ok:
        out["error"] = "a step failed; nothing was rolled back automatically"
        return 500, out
    return out


@route("POST", "/api/raid/plan", mutating=False)
def api_raid_plan(req: Request) -> dict:
    op = req.body.get("op")
    if not isinstance(op, str):
        raise ApiError(400, "op is required")
    return raid_run(req, op, False)


def _register_raid_ops() -> None:
    for op in ("create", "add-spare", "fail", "remove", "replace", "grow", "change-level", "scrub", "stop", "assemble", "tuning", "shr-create", "shr-add-disk", "shr-replace-disk"):
        ROUTES[("POST", "/api/raid/" + op)] = Route(lambda req, _op=op: raid_run(req, _op, True), True, True)


_register_raid_ops()


@route("POST", "/api/raid/after-grow")
def api_raid_after_grow(req: Request) -> dict:
    """pvresize + lvextend + filesystem grow once a reshape or 'size=max' has finished."""
    app, b = req.app, req.body
    ctx = gather_raid_ctx(app)
    arr = ctx.array(b.get("array"))
    _need_idle_api(arr)
    if arr["foreign"] and b.get("foreign_ack") is not True:
        raise ApiError(400, "this array was not created here: tick 'I understand this is not mine' to change it")
    need_confirm(b, arr["name"])
    steps = after_grow_follow_ups(arr, ctx.lvs, ctx.devs)
    ok, results = run_steps(app, steps)
    return (200 if ok else 500), {"ok": ok, "results": results, **({} if ok else {"error": "a step failed"})}


def _need_idle_api(arr: dict) -> None:
    try:
        _need_idle(arr, "extending the volume")
    except SafetyError as e:
        raise ApiError(409, str(e))


def raid_scrub_all(app: App) -> int:
    """Used by the monthly timer: start a check on every idle, healthy redundant array."""
    started = 0
    for a in app.mdstat():
        if a["level"] in ("raid1", "raid4", "raid5", "raid6", "raid10") and not a["degraded"] and not a["sync"] and a["state"] != "inactive":
            r = exec_step(app, Step("check", None, kind="sysfs", meta={"path": "/sys/block/%s/md/sync_action" % a["name"], "value": "check"}))
            started += 1 if r.ok else 0
    return started


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
        csp = ("default-src 'self'; script-src 'nonce-%s'; style-src 'nonce-%s'; img-src 'self' data:; font-src data:; "
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
        rt = find_route(method, path)
        if rt is None:
            other = any(p == path for (_, p) in ROUTES) or any(rx.fullmatch(path) for _, rx, _r in DYNAMIC_ROUTES)
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
        req = Request(app, method, path, {}, session, ip, sid, query=urllib.parse.urlsplit(self.path).query)
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
# 17b. Add-ons (plugin framework for Docker apps). The implementation is synofurb_addons.py (docs/ADDONS.md);
#      this block is the whole glue: environment, routes, CLI. A bundle without that module simply has no add-ons.
# ============================================================================

try:
    import synofurb_addons as addons_mod
except ImportError:                                           # pragma: no cover - an update bundle from before add-ons existed
    addons_mod = None


class _AddonJobLock:
    """Add-on jobs and management-plane updates exclude each other through the same job.lock."""

    def __init__(self, cfg: Config):
        self.lock = JobLock(cfg)

    def acquire(self, kind: str, label: str, job_id: str = "") -> dict | None:
        return self.lock.acquire(kind, label, job_id)

    def release(self, token: dict | None) -> None:
        self.lock.release(token)

    def holder(self) -> dict | None:
        return self.lock.holder()


def addons_env(app: App) -> Any:
    cfg, runner = app.cfg, app.runner
    real = isinstance(runner, RealRunner)
    extra: dict = {}
    if cfg.dry_run:                                           # nothing real exists to probe or wait for
        if isinstance(runner, DryRunRunner):
            runner.canned.setdefault("dpkg-query", "ii ")
            runner.canned.setdefault("apt-cache", "  Candidate: 1.0\n")
        extra = dict(http_probe=lambda url, timeout=4: 200, tcp_probe=lambda port, timeout=3: True, sleep=lambda s: None, port_in_use=lambda port: False)
    return addons_mod.AddonEnv(**extra, 
        root=cfg.root, run=runner.run, stream=(lambda argv, on_line, timeout: addons_mod.stream_command(argv, on_line, timeout)) if real else None,
        mounts=app.findmnt, audit=app.audit.log, script_dir=os.path.dirname(os.path.abspath(__file__)), state_dir=cfg.state_dir,
        login_group=cfg.login_group, sys_root=cfg.sys_root, job_lock=_AddonJobLock(cfg))


def addons_manager(app: App) -> Any:
    """One manager per App (created on first use). In --dry-run the Docker calls go to the in-memory test double."""
    mgr = getattr(app, "_addons", None)
    if mgr is None:
        env = addons_env(app)
        docker = addons_mod.FakeDocker(env) if app.cfg.dry_run else None
        mgr = app._addons = addons_mod.Manager(env, docker)                # type: ignore[attr-defined]
    return mgr


if addons_mod is not None:
    addons_mod.register_routes(sys.modules[__name__], addons_manager)


def addons_cli(args: argparse.Namespace, extra: list[str], cfg: Config, runner: Runner) -> int:
    if addons_mod is None:
        print("error: this management plane has no add-on module", file=sys.stderr)
        return 2
    if getattr(args, "json", False):
        extra = extra + ["--json"]
    return addons_mod.cli_main(extra, addons_manager(App(cfg, runner)))


# ============================================================================
# 18a. CLI contract for the desktop app (no server, usable over SSH as root)
#      synofurb_mgmt.py health --json
#      synofurb_mgmt.py update --from DIR [--force] [--json]  /  update --rollback [--json]
# ============================================================================

HEALTH_SCHEMA = 1
HEALTH_SERVICES = ("synofurb-mgmt", "ssh", "systemd-networkd", "systemd-resolved")
INSTALL_DIR = "/opt/synofurb-mgmt"
UNIT_PATH = "/etc/systemd/system/synofurb-mgmt.service"
BUNDLE_REQUIRED = ("synofurb_mgmt.py", "VERSION", "README.md")
BUNDLE_OPTIONAL_UNIT = "synofurb-mgmt.service"
BUNDLE_ADDON_FILES = ("synofurb_addons.py", "addons.pack.json")      # the add-on framework and the packed add-on definitions (optional)
BUNDLE_EDGE_FILES = ("synofurb_edge.py",)                             # certificates (lego) and the HAProxy front (optional, docs/CERTIFICATES.md)
MANIFEST_NAME = "MANIFEST.sha256"
MAX_BUNDLE_FILE = 8 * 1024 * 1024
EXIT_OK, EXIT_REFUSED, EXIT_ROLLED_BACK, EXIT_ROLLBACK_FAILED = 0, 2, 3, 4


def _pct(used: float, total: float) -> int | None:
    return int(round(100.0 * used / total)) if total else None


def collect_health(app: App, quick: bool = False, budget_s: float = 40.0) -> dict:
    """One JSON-serialisable health document. Every section is independent: a failing part becomes null plus a warning."""
    t_end = time.monotonic() + budget_s
    warnings: list[str] = []
    crit: list[str] = []
    cfg = app.cfg

    def part(name: str, fn: Callable[[], Any], default: Any = None) -> Any:
        try:
            return fn()
        except Exception:                                   # noqa: BLE001 - the contract is "never raise"
            warnings.append("%s unavailable" % name)
            return default

    def uptime() -> float | None:
        t = read_text("/proc/uptime").split()
        return round(float(t[0])) if t else None

    def load() -> list[float] | None:
        return [round(x, 2) for x in os.getloadavg()]

    def mem() -> dict | None:
        mi = parse_meminfo(read_text(cfg.proc_root + "/meminfo"))
        return {"total": mi.get("MemTotal"), "avail": mi.get("MemAvailable")} if mi else None

    def osname() -> str | None:
        o = parse_os_release(app.read("/etc/os-release") or read_text("/etc/os-release"))
        return o.get("PRETTY_NAME") or o.get("NAME")

    def root_fs() -> dict | None:
        sv = os.statvfs("/")
        used = sv.f_blocks - sv.f_bfree
        return {"used_pct": _pct(used, used + sv.f_bavail)}

    def arrays() -> list[dict]:
        out = []
        for a in app.mdstat():
            sync = a["sync"] or {}
            state = "inactive" if a["state"] == "inactive" else "degraded" if a["degraded"] else "resyncing" if sync else "clean"
            out.append({"name": a["name"], "level": a["level"], "state": state, "degraded": bool(a["degraded"]),
                        "sync_pct": sync.get("percent"), "sync_action": sync.get("action")})
        return out

    def disks() -> list[dict]:
        devs = [d for d in app.lsblk() if d["type"] == "disk" and not d["kname"].startswith(("loop", "zram", "ram", "sr", "nbd"))]
        have = bool(app.runner.which("smartctl")) and not quick
        out = []
        for d in devs:
            smart, temp = "unknown", None
            if have and time.monotonic() < t_end:
                r = app.run(["smartctl", "-H", "-j", d["path"]], timeout=10)       # rc is a bitmask; parse regardless
                info = parse_smart(r.out) if r.out.strip() else {}
                if info.get("passed") is True:
                    smart = "passed"
                elif info.get("passed") is False:
                    smart = "failed"
                temp = info.get("temp")
            out.append({"dev": d["path"], "model": d["model"], "size": d["size"], "smart": smart, "temp_c": temp})
        return out

    def volumes() -> list[dict]:
        return [{"mount": r["target"], "used_pct": _pct(r["used"], r["size"]), "fstype": r["fstype"]} for r in app.findmnt()
                if r["real"] and (r["target"] == "/" or str(r["target"]).startswith(("/srv/", "/mnt/")))]

    def services() -> list[dict]:
        names = list(HEALTH_SERVICES) + (["mdmonitor"] if app.mdstat() else [])
        return [{"name": n, "active": app.run(["systemctl", "is-active", n + ".service"], timeout=8).out.strip() == "active"} for n in names]

    def boot() -> dict:
        present = os.path.isfile(cfg.p(DOM_TOOL))
        default = None
        if present and not quick:
            st, _ = _dom_status(app)
            default = boot_view(st)["default_id"] if st else None
        return {"default": default, "flash_present": present}

    def updates() -> dict | None:
        if quick or time.monotonic() >= t_end or not app.runner.which("apt-get"):
            return None
        r = app.run(["apt-get", "-s", "-o", "Debug::NoLocking=1", "upgrade"], timeout=30)
        if not r.ok:
            raise RuntimeError("apt")
        inst = [ln for ln in r.out.splitlines() if ln.startswith("Inst ")]
        return {"apt_pending": len(inst), "security_pending": sum(1 for ln in inst if re.search(r"-security|Debian-Security", ln))}

    doc: dict[str, Any] = {"schema": HEALTH_SCHEMA, "version": VERSION, "hostname": part("hostname", socket.gethostname),
                           "uptime_s": part("uptime", uptime), "load": part("load", load), "mem": part("memory", mem), "os": part("os", osname),
                           "kernel": part("kernel", lambda: os.uname().release), "root_fs": part("root fs", root_fs), "arrays": part("arrays", arrays),
                           "disks": part("disks", disks), "volumes": part("volumes", volumes), "services": part("services", services),
                           "boot": part("boot", boot), "updates": part("updates", updates)}
    for a in doc["arrays"] or []:
        if a["degraded"]:
            crit.append("%s degraded" % a["name"])
        elif a["state"] == "inactive":
            warnings.append("%s inactive" % a["name"])
        elif a["sync_action"]:
            warnings.append("%s %s %s%%" % (a["name"], a["sync_action"], a["sync_pct"] if a["sync_pct"] is not None else "?"))
    for d in doc["disks"] or []:
        if d["smart"] == "failed":
            crit.append("%s SMART failed" % d["dev"].replace("/dev/", ""))
        if d["temp_c"] is not None and d["temp_c"] >= 60:
            (crit if d["temp_c"] >= 70 else warnings).append("%s %d C" % (d["dev"].replace("/dev/", ""), d["temp_c"]))
    for v in ([{"mount": "root", "used_pct": (doc["root_fs"] or {}).get("used_pct")}] + (doc["volumes"] or [])):
        if v["used_pct"] is not None and v["used_pct"] >= 85:
            (crit if v["used_pct"] >= 95 else warnings).append("%s %d%% full" % (v["mount"], v["used_pct"]))
    for sv in doc["services"] or []:
        if not sv["active"]:
            warnings.append("%s not running" % sv["name"])
    if doc["updates"] and doc["updates"]["security_pending"]:
        warnings.append("%d security updates pending" % doc["updates"]["security_pending"])
    mm = doc["mem"] or {}
    if mm.get("total") and mm.get("avail") is not None and mm["avail"] < mm["total"] * 0.05:
        warnings.append("memory almost full")
    doc["overall"] = "crit" if crit else "warn" if warnings else "ok"
    doc["warnings"] = (crit + warnings)[:20]
    return doc


# ---- update ------------------------------------------------------------------

def parse_version(v: str) -> tuple[int, int, int]:
    m_ = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.]+)?", v.strip())
    if not m_:
        raise ValidationError("not a version: %r" % v[:40])
    return int(m_.group(1)), int(m_.group(2)), int(m_.group(3))


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()


def verify_bundle(src: str) -> tuple[dict[str, str], str]:
    """Check a local update directory against its MANIFEST.sha256. Returns ({name: sha256}, new version). Raises ValidationError."""
    if "://" in src:
        raise ValidationError("update takes a local directory; network sources are not supported")
    if not os.path.isdir(src) or os.path.islink(src):
        raise ValidationError("%s is not a directory" % src)
    man = os.path.join(src, MANIFEST_NAME)
    if not os.path.isfile(man) or os.path.islink(man):
        raise ValidationError("%s is missing: refusing to install unverified code" % MANIFEST_NAME)
    expected: dict[str, str] = {}
    for ln in read_text(man).splitlines():
        if not ln.strip():
            continue
        mm = re.fullmatch(r"([0-9a-f]{64})\s+\*?([A-Za-z0-9._-]{1,64})", ln.strip())
        if not mm or mm.group(2) == MANIFEST_NAME or mm.group(2) in expected:
            raise ValidationError("bad manifest line: %r" % ln[:80])
        expected[mm.group(2)] = mm.group(1)
    present = set(os.listdir(src)) - {MANIFEST_NAME}
    for name in sorted(present):
        p = os.path.join(src, name)
        if os.path.islink(p) or not os.path.isfile(p):
            raise ValidationError("%s is not a regular file" % name)
        if os.path.getsize(p) > MAX_BUNDLE_FILE:
            raise ValidationError("%s is too large" % name)
    if present != set(expected):
        raise ValidationError("files and manifest differ (unlisted: %s; missing: %s)" % (", ".join(sorted(present - set(expected))) or "none", ", ".join(sorted(set(expected) - present)) or "none"))
    for name in BUNDLE_REQUIRED:
        if name not in expected:
            raise ValidationError("%s is missing from the update" % name)
    allowed = set(BUNDLE_REQUIRED) | {BUNDLE_OPTIONAL_UNIT} | set(BUNDLE_ADDON_FILES) | set(BUNDLE_EDGE_FILES)
    if set(expected) - allowed:
        raise ValidationError("unexpected file(s): %s" % ", ".join(sorted(set(expected) - allowed)))
    for name, digest in expected.items():
        if not hmac.compare_digest(sha256_file(os.path.join(src, name)), digest):
            raise ValidationError("checksum mismatch for %s" % name)
    version = read_text(os.path.join(src, "VERSION")).strip().splitlines()[0] if read_text(os.path.join(src, "VERSION")).strip() else ""
    parse_version(version)
    try:
        compile(read_text(os.path.join(src, "synofurb_mgmt.py")), "synofurb_mgmt.py", "exec")      # syntax only; never executed here
    except SyntaxError as e:
        raise ValidationError("synofurb_mgmt.py does not compile: %s" % e)
    return expected, version


def swap_dirs(a: str, b: str) -> None:
    """Atomically exchange two directories (renameat2 RENAME_EXCHANGE); falls back to three renames."""
    try:
        libc = ctypes.CDLL(None, use_errno=True)
        if libc.renameat2(-100, a.encode(), -100, b.encode(), 2) == 0:
            return
    except (OSError, AttributeError):
        pass
    tmp = a + ".swap"
    os.rename(a, tmp)
    os.rename(b, a)
    os.rename(tmp, b)


def default_probe(cfg: Config) -> bool:
    for host in ("127.0.0.1", "::1"):
        try:
            with socket.create_connection((host, cfg.port), timeout=2):
                return True
        except OSError:
            continue
    return False


class Updater:
    """update --from DIR: verify, stage, swap, restart, poll health, roll back on failure. Emits NDJSON-able progress."""

    def __init__(self, cfg: Config, runner: Runner, emit: Callable[[str, str, str], None], probe: Callable[[Config], bool] | None = None,
                 health_timeout: float = 30.0, sleep: Callable[[float], None] = time.sleep):
        self.cfg, self.runner, self.emit = cfg, runner, emit
        self.probe, self.health_timeout, self.sleep = probe or default_probe, health_timeout, sleep
        self.before_restart: Callable[[], None] | None = None          # the web job waits here so its response is delivered before the service goes down
        self.inst, self.new, self.prev = cfg.p(INSTALL_DIR), cfg.p(INSTALL_DIR + ".new"), cfg.p(INSTALL_DIR + ".prev")

    def installed_version(self) -> str | None:
        t = read_text(os.path.join(self.inst, "VERSION")).strip()
        return t.splitlines()[0] if t else None

    def restart(self, reload_units: bool) -> bool:
        if self.before_restart:
            self.before_restart()
        if reload_units:
            self.runner.run(["systemctl", "daemon-reload"], timeout=30)
        return self.runner.run(["systemctl", "restart", "synofurb-mgmt.service"], timeout=60).ok

    def healthy(self, expect_version: str | None) -> tuple[bool, str]:
        t_end = time.monotonic() + self.health_timeout
        last = "no answer"
        while True:
            active = self.runner.run(["systemctl", "is-active", "synofurb-mgmt.service"], timeout=10).out.strip() == "active"
            if not active:
                last = "service is not active"
            elif not self.probe(self.cfg):
                last = "port %d is not listening" % self.cfg.port
            else:
                script = os.path.join(self.inst, "synofurb_mgmt.py")
                argv = [sys.executable or "/usr/bin/python3", script, "health", "--json", "--quick"] + (["--root", self.cfg.root] if self.cfg.root else [])
                r = self.runner.run(argv, timeout=25)
                try:
                    h = json.loads(r.out.strip().splitlines()[-1]) if r.out.strip() else {}
                except ValueError:
                    h = {}
                if h.get("schema") == HEALTH_SCHEMA and (expect_version is None or h.get("version") == expect_version):
                    return True, "version %s answers" % h.get("version")
                last = "health check failed (%s)" % (("version %s" % h.get("version")) if h.get("schema") else "no valid answer")
            if time.monotonic() >= t_end:
                return False, last
            self.sleep(1.0)

    def _unit_swap(self, content: str | None) -> bool:
        """Install the packaged unit file (if any); remember the previous one for a rollback. Returns True if it changed."""
        path = self.cfg.p(UNIT_PATH)
        old = read_text(path) if os.path.isfile(path) else None
        if content is None or content == old:
            return False
        if old is not None:
            write_atomic(os.path.join(self.prev, "unit.backup"), old)
        write_atomic(path, content)
        return True

    def update(self, src: str, force: bool = False) -> int:
        emit = self.emit
        emit("verify", "start", "checking %s" % src)
        try:
            files, new_v = verify_bundle(src)
            cur = self.installed_version()
            if cur and parse_version(new_v) < parse_version(cur) and not force:
                raise ValidationError("version %s is older than the installed %s (use --force to downgrade)" % (new_v, cur))
        except (ValidationError, OSError) as e:
            emit("verify", "fail", str(e))
            return EXIT_REFUSED
        emit("verify", "ok", "manifest verified, version %s%s" % (new_v, " (installed %s)" % cur if cur else ""))
        emit("stage", "start", "copying to %s" % INSTALL_DIR + ".new")
        try:
            if os.path.lexists(self.new):
                shutil.rmtree(self.new)
            os.makedirs(self.new)
            for name in files:
                shutil.copyfile(os.path.join(src, name), os.path.join(self.new, name))
                os.chmod(os.path.join(self.new, name), 0o644)
                if sha256_file(os.path.join(self.new, name)) != files[name]:
                    raise OSError("copy of %s differs from the manifest" % name)
        except OSError as e:
            emit("stage", "fail", str(e))
            shutil.rmtree(self.new, ignore_errors=True)
            return EXIT_REFUSED
        emit("stage", "ok", "%d files staged" % len(files))
        emit("swap", "start", "swapping %s" % INSTALL_DIR)
        had_prev_before = os.path.isdir(self.inst)
        try:
            if had_prev_before:
                if os.path.lexists(self.prev):
                    shutil.rmtree(self.prev)
                swap_dirs(self.inst, self.new)                      # inst = new code, .new = old code
                os.rename(self.new, self.prev)
            else:
                os.rename(self.new, self.inst)
            unit_changed = self._unit_swap(read_text(os.path.join(self.inst, BUNDLE_OPTIONAL_UNIT)) if BUNDLE_OPTIONAL_UNIT in files else None)
        except OSError as e:
            emit("swap", "fail", str(e))
            return self._rollback_after_failure(new_v, had_prev_before, False)
        emit("swap", "ok", "previous version kept in %s" % INSTALL_DIR + ".prev" if had_prev_before else "installed")
        emit("restart", "start", "restarting synofurb-mgmt.service")
        if not self.restart(unit_changed):
            emit("restart", "fail", "systemctl restart failed")
            return self._rollback_after_failure(new_v, had_prev_before, unit_changed)
        emit("restart", "ok", "service restarted")
        emit("health", "start", "waiting up to %d s for the new version" % int(self.health_timeout))
        ok, why = self.healthy(new_v)
        if not ok:
            emit("health", "fail", why)
            return self._rollback_after_failure(new_v, had_prev_before, unit_changed)
        emit("health", "ok", why)
        emit("result", "ok", "updated %s -> %s" % (cur or "none", new_v))
        return EXIT_OK

    def _restore(self, had_prev: bool, unit_changed: bool) -> bool:
        """Put the previous install back (inst <-> prev exchange) and its unit file."""
        try:
            bpath = os.path.join(self.prev, "unit.backup")
            backup = read_text(bpath) if (unit_changed and os.path.isfile(bpath)) else None      # read before the directories move
            if had_prev and os.path.isdir(self.prev):
                swap_dirs(self.inst, self.prev)
            if backup is not None:
                write_atomic(self.cfg.p(UNIT_PATH), backup)
            for d in (self.inst, self.prev):
                if os.path.isfile(os.path.join(d, "unit.backup")):
                    os.unlink(os.path.join(d, "unit.backup"))
            return True
        except OSError as e:
            self.emit("rollback", "fail", str(e))
            return False

    def _rollback_after_failure(self, failed_v: str, had_prev: bool, unit_changed: bool) -> int:
        self.emit("rollback", "start", "restoring the previous version")
        if not had_prev:
            self.emit("rollback", "fail", "there was no previous install to restore")
            self.emit("result", "fail", "update failed; nothing to roll back to")
            return EXIT_ROLLBACK_FAILED
        if not self._restore(had_prev, unit_changed):
            self.emit("result", "fail", "update failed and the rollback failed")
            return EXIT_ROLLBACK_FAILED
        self.restart(unit_changed)
        ok, why = self.healthy(None)
        if not ok:
            self.emit("rollback", "fail", "previous version does not answer: " + why)
            self.emit("result", "fail", "update failed and the previous version does not answer")
            return EXIT_ROLLBACK_FAILED
        self.emit("rollback", "ok", "previous version %s is running again" % (self.installed_version() or "?"))
        self.emit("result", "fail", "update to %s failed; rolled back" % failed_v)
        return EXIT_ROLLED_BACK

    def rollback(self) -> int:
        """update --rollback: exchange the installed and the .prev version."""
        self.emit("rollback", "start", "restoring %s" % INSTALL_DIR + ".prev")
        if not os.path.isfile(os.path.join(self.prev, "synofurb_mgmt.py")):
            self.emit("rollback", "fail", "no previous version is kept")
            return EXIT_REFUSED
        unit_changed = os.path.isfile(os.path.join(self.prev, "unit.backup"))
        if not self._restore(True, unit_changed):
            return EXIT_ROLLBACK_FAILED
        self.restart(unit_changed)
        ok, why = self.healthy(None)
        if not ok:
            self.emit("rollback", "fail", why)
            self.emit("result", "fail", "rolled back but the service does not answer: " + why)
            return EXIT_ROLLBACK_FAILED
        self.emit("rollback", "ok", "version %s is running" % (self.installed_version() or "?"))
        self.emit("result", "ok", "rolled back to %s" % (self.installed_version() or "?"))
        return EXIT_OK


# ============================================================================
# 18b. Self-update: ONE pipeline for the web UI, `update --from-github` and the app's push
#      check -> stage -> verify -> swap -> health -> rollback.  The last four steps are `Updater` above (offline, shared with
#      `update --from DIR`); this section adds the sources (a GitHub release asset or the monorepo sub-folder), the job lock,
#      the job records (version history: who/what/when), the audit trail and the web routes.  Nothing here runs on the app's
#      push path except the lock, the history record and the audit line.
# ============================================================================

import gzip  # noqa: E402
import http.client  # noqa: E402
import tarfile  # noqa: E402
import urllib.error  # noqa: E402
import urllib.request  # noqa: E402
import zlib  # noqa: E402

UPDATE_REPO = "andreaswiren/synofurb"
UPDATE_SUBDIR = "apps/mgmt-plane"
# the only hosts a download may touch (https, every redirect hop); release downloads end up on a githubusercontent.com CDN host
UPDATE_HOSTS = ("api.github.com", "github.com", "codeload.github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com")
UPDATE_PLACEHOLDER_KEY = "REPLACE_WITH_MINISIGN_PUBLIC_KEY"       # same placeholder as install.sh; a real key makes signatures mandatory
UPDATE_PUBKEY = UPDATE_PLACEHOLDER_KEY
BUNDLE_FILES = BUNDLE_REQUIRED + (BUNDLE_OPTIONAL_UNIT,) + BUNDLE_ADDON_FILES + BUNDLE_EDGE_FILES          # manifest order, identical to FILES in tools/make-update-bundle.py
MAX_API, MAX_SUMS, MAX_ASSET, MAX_REPO_TARBALL, MAX_REPO_UNPACKED = 2 << 20, 256 << 10, 8 << 20, 256 << 20, 2 << 30
MAX_ADDON_FILE, MAX_ADDON_FILES, MAX_ADDON_BYTES = 256 << 10, 600, 8 << 20       # add-on definitions found in a source tree
BUNDLE_GENERATED = ("addons.pack.json",)                                          # made at build/stage time, never copied from an archive tree
UPDATE_CHANNELS = ("stable", "main")
UPDATE_SOURCES = ("app", "manual", "github-stable", "github-main", "rollback")
ASSET_RE = re.compile(r"synofurb-mgmt-(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)\.tar\.gz")
TAG_RE = re.compile(r"v?\d+\.\d+\.\d+(?:[-+.][0-9A-Za-z.+-]{0,40})?")
REF_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._/-]{0,99}")
SHA_RE = re.compile(r"[0-9a-f]{40}")
APP_STAGE_PREFIX = ".synofurb-mgmt-stage-"                          # what the desktop app's push stages into (manage.rs); informational only


class UpdateError(Exception):
    """A refusal before anything on the unit changed. ``kind`` is stable (the UI and tests key on it); ``message`` is short and safe to show."""

    def __init__(self, kind: str, message: str):
        super().__init__(message)
        self.kind, self.message = kind, message


def bundle_member_ok(name: str) -> bool:
    """Which files an update may contain. The one place to extend if the bundle contract ever grows (verify_bundle must agree)."""
    return name in BUNDLE_FILES


def write_manifest(directory: str, names: Iterable[str] = BUNDLE_FILES) -> list[str]:
    """MANIFEST.sha256 in `sha256sum` format; the same lines `tools/make-update-bundle.py` writes (a test compares them)."""
    lines = ["%s  %s" % (sha256_file(os.path.join(directory, n)), n) for n in names if os.path.isfile(os.path.join(directory, n))]
    write_atomic(os.path.join(directory, MANIFEST_NAME), "\n".join(lines) + "\n")
    return lines


# ---- HTTP (https to GitHub only) ---------------------------------------------

class _PolicyRedirect(urllib.request.HTTPRedirectHandler):
    max_redirections = 5

    def __init__(self, check: Callable[[str], None]):
        self._check = check

    def redirect_request(self, req, fp, code, msg, headers, newurl):          # type: ignore[no-untyped-def]
        self._check(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class HttpFetcher:
    """GET with a host policy (https, GitHub hosts, no credentials in the URL, every redirect hop checked), hard size limits and
    typed errors. No token is ever sent. ``loopback_http`` exists for the tests' fake release server only."""

    def __init__(self, hosts: Iterable[str] = UPDATE_HOSTS, loopback_http: bool = False, timeout: float = 20.0):
        self.hosts, self.loopback_http, self.timeout = tuple(hosts), loopback_http, timeout
        self.opener = urllib.request.build_opener(_PolicyRedirect(self.check_url))

    def check_url(self, url: str) -> None:
        u = urllib.parse.urlsplit(url)
        host = (u.hostname or "").lower()
        try:
            port = u.port
        except ValueError:
            port = -1
        if u.username or u.password or "\\" in url:
            raise UpdateError("policy", "refusing a URL with credentials")
        if u.scheme == "https" and host in self.hosts and port in (None, 443):
            return
        if self.loopback_http and u.scheme in ("http", "https") and host in ("127.0.0.1", "localhost", "::1"):
            return
        raise UpdateError("policy", "refusing %s: only https to GitHub is allowed" % (host or "that address"))

    def _open(self, url: str, accept: str):
        self.check_url(url)
        req = urllib.request.Request(url, headers={"User-Agent": "synofurb-mgmt/" + VERSION, "Accept": accept})
        try:
            return self.opener.open(req, timeout=self.timeout)
        except urllib.error.HTTPError as e:
            if e.code == 429 or (e.code == 403 and e.headers.get("X-RateLimit-Remaining") == "0"):
                try:
                    mins = max(1, int((int(e.headers.get("X-RateLimit-Reset", "0")) - time.time()) / 60) + 1)
                except ValueError:
                    mins = 10
                raise UpdateError("rate_limit", "GitHub rate limit reached; try again in about %d minute(s)" % min(mins, 120))
            if e.code == 404:
                raise UpdateError("not_found", "GitHub did not find it (HTTP 404); a private repository cannot be checked")
            raise UpdateError("http", "GitHub answered HTTP %d" % e.code)
        except (urllib.error.URLError, http.client.HTTPException, OSError, ValueError) as e:
            reason = getattr(e, "reason", e)
            raise UpdateError("offline", "Cannot reach GitHub (%s)" % type(reason).__name__)

    def get_bytes(self, url: str, max_bytes: int, accept: str = "application/json") -> bytes:
        try:
            with self._open(url, accept) as r:
                if int(r.headers.get("Content-Length") or 0) > max_bytes:
                    raise UpdateError("size", "the answer is larger than the %d KiB limit" % (max_bytes >> 10))
                data = r.read(max_bytes + 1)
        except (OSError, http.client.HTTPException, ValueError) as e:
            raise UpdateError("offline", "the answer from GitHub was interrupted (%s)" % type(e).__name__)
        if len(data) > max_bytes:
            raise UpdateError("size", "the answer is larger than the %d KiB limit" % (max_bytes >> 10))
        return data

    def download(self, url: str, dest: str, max_bytes: int) -> tuple[str, int]:
        """Stream to ``dest`` (0600), hashing as it goes. Returns (sha256, size). A partial file is removed."""
        h = hashlib.sha256()
        n = 0
        fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        try:
            with os.fdopen(fd, "wb") as out, self._open(url, "application/octet-stream") as r:
                if int(r.headers.get("Content-Length") or 0) > max_bytes:
                    raise UpdateError("size", "the download is larger than the %d MiB limit" % (max_bytes >> 20))
                while True:
                    chunk = r.read(1 << 16)
                    if not chunk:
                        break
                    n += len(chunk)
                    if n > max_bytes:
                        raise UpdateError("size", "the download is larger than the %d MiB limit" % (max_bytes >> 20))
                    h.update(chunk)
                    out.write(chunk)
        except UpdateError:
            _unlink(dest)
            raise
        except (OSError, ValueError, http.client.HTTPException) as e:
            _unlink(dest)
            raise UpdateError("offline", "the download was interrupted (%s)" % type(e).__name__)
        except BaseException:
            _unlink(dest)
            raise
        return h.hexdigest(), n


def _unlink(path: str) -> None:
    try:
        os.unlink(path)
    except OSError:
        pass


class _CapReader:
    """File-like wrapper that refuses to hand out more than ``cap`` bytes (a gzip bomb stops here)."""

    def __init__(self, fh: Any, cap: int):
        self.fh, self.cap, self.n = fh, cap, 0

    def read(self, size: int = -1) -> bytes:
        data = self.fh.read(size)
        self.n += len(data)
        if self.n > self.cap:
            raise UpdateError("size", "the archive unpacks to more than %d MiB" % (self.cap >> 20))
        return data


def _config_json(cfg: Config) -> dict:
    try:
        with open(cfg.p(cfg.config_path), encoding="utf-8") as fh:
            d = json.load(fh)
        return d if isinstance(d, dict) else {}
    except (OSError, ValueError):
        return {}


def update_pubkey(cfg: Config) -> str:
    """The minisign public key releases must be signed with. The placeholder (nothing configured) means releases are unsigned today."""
    k = _config_json(cfg).get("update_pubkey")
    return k if isinstance(k, str) and re.fullmatch(r"RW[A-Za-z0-9+/=]{20,100}", k) else UPDATE_PUBKEY


def update_trust(cfg: Config, channel: str, has_signature: bool) -> tuple[str, str | None]:
    """(mode, blocked_reason). mode: `signed` (minisign verified), `unsigned` (SHA-256 only, needs the person's consent) or `blocked`."""
    if update_pubkey(cfg) == UPDATE_PLACEHOLDER_KEY:
        return "unsigned", None
    if channel == "stable":
        return ("signed", None) if has_signature else ("blocked", "this release has no SHA256SUMS.minisig")
    if _config_json(cfg).get("update_allow_unsigned_source") is True:
        return "unsigned", None
    return "blocked", "a signing key is configured and a source tree cannot be verified"


def plain_notes(md: str, limit: int = 1500) -> str:
    out = []
    for ln in (md or "").replace("\r", "").splitlines()[:40]:
        ln = re.sub(r"^\s{0,3}#{1,6}\s*", "", ln)
        ln = re.sub(r"^\s*[-*]\s+", "\u2022 ", ln)
        ln = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", ln).replace("**", "").replace("`", "")
        out.append(ln.rstrip())
    return re.sub(r"\n{3,}", "\n\n", "\n".join(out)).strip()[:limit]


def parse_sums(text: str, name: str) -> str:
    for ln in text.splitlines():
        parts = ln.strip().split(None, 1)
        if len(parts) == 2 and parts[1].lstrip("*") == name:
            if not re.fullmatch(r"[0-9a-fA-F]{64}", parts[0]):
                raise UpdateError("parse", "SHA256SUMS has a malformed line for %s" % name)
            return parts[0].lower()
    raise UpdateError("checksum", "%s is not listed in SHA256SUMS: refusing an unlisted file" % name)


def valid_ref(ref: Any) -> str:
    if not isinstance(ref, str) or not REF_RE.fullmatch(ref) or ".." in ref or "//" in ref or ref.endswith(("/", ".lock", ".")):
        raise UpdateError("policy", "not a valid branch or tag name")
    return ref


def installed_info(cfg: Config) -> tuple[str | None, str | None]:
    inst = cfg.p(INSTALL_DIR)
    t = read_text(os.path.join(inst, "VERSION")).strip()
    script = os.path.join(inst, "synofurb_mgmt.py")
    return (t.splitlines()[0] if t else None), (sha256_file(script) if os.path.isfile(script) else None)


def _cmp_state(remote: str, installed: str | None, channel: str) -> str:
    try:
        r, i = parse_version(remote), parse_version(installed or "")
    except ValidationError:
        return "available"
    return "available" if r > i else "older" if r < i else ("same" if channel == "main" else "current")


class GithubSource:
    """Where an update can come from: the newest release asset (`stable`) or apps/mgmt-plane of a ref of the monorepo (`main`)."""

    def __init__(self, fetch: HttpFetcher, repo: str = UPDATE_REPO, api_base: str = "https://api.github.com",
                 dl_base: str = "https://github.com", codeload_base: str = "https://codeload.github.com"):
        self.fetch, self.repo, self.api, self.dl, self.code = fetch, repo, api_base.rstrip("/"), dl_base.rstrip("/"), codeload_base.rstrip("/")

    def _json(self, url: str) -> Any:
        try:
            return json.loads(self.fetch.get_bytes(url, MAX_API, "application/vnd.github+json").decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            raise UpdateError("parse", "GitHub sent an answer that could not be read")

    # -- check -----------------------------------------------------------------
    def check(self, cfg: Config, channel: str, ref: str = "") -> dict:
        installed, _ = installed_info(cfg)
        if channel == "stable":
            info = self._check_stable()
        elif channel == "main":
            info = self._check_main(valid_ref(ref or "main"))
        else:
            raise UpdateError("policy", "unknown channel")
        mode, blocked = update_trust(cfg, channel, info.pop("has_signature", False))
        info.update(channel=channel, installed=installed, state=_cmp_state(info["version"], installed, channel), mode=mode, blocked=blocked,
                    checked_at=int(time.time()), repo=self.repo)
        return info

    def _check_stable(self) -> dict:
        rels = self._json("%s/repos/%s/releases?per_page=30" % (self.api, self.repo))
        best = None
        for rel in rels if isinstance(rels, list) else []:
            if not isinstance(rel, dict) or rel.get("draft") or rel.get("prerelease"):
                continue
            tag = rel.get("tag_name")
            assets = {a.get("name"): a for a in rel.get("assets") or [] if isinstance(a, dict)}
            if not isinstance(tag, str) or not TAG_RE.fullmatch(tag):
                continue
            for name, a in assets.items():
                m_ = ASSET_RE.fullmatch(name) if isinstance(name, str) else None
                if not m_ or "-" in m_.group(1):
                    continue
                key = parse_version(m_.group(1))
                if best is None or key > best[0]:
                    best = (key, m_.group(1), tag, name, rel, assets)
        if best is None:
            raise UpdateError("no_release", "No management plane release found")
        _, version, tag, name, rel, assets = best
        if "SHA256SUMS" not in assets:
            raise UpdateError("blocked", "release %s has no SHA256SUMS: refusing to install it" % tag)
        size = assets[name].get("size")
        if isinstance(size, int) and size > MAX_ASSET:
            raise UpdateError("size", "the release asset is larger than the %d MiB limit" % (MAX_ASSET >> 20))
        return {"version": version, "tag": tag, "asset": name, "ref": "", "commit": "", "published": str(rel.get("published_at") or "")[:10],
                "notes": plain_notes(str(rel.get("body") or "")), "has_signature": "SHA256SUMS.minisig" in assets}

    def _check_main(self, ref: str) -> dict:
        qref = urllib.parse.quote(ref, safe="/")
        commits = self._json("%s/repos/%s/commits?sha=%s&path=%s&per_page=5" % (self.api, self.repo, qref, UPDATE_SUBDIR))
        rows = [c for c in commits if isinstance(c, dict) and isinstance(c.get("sha"), str) and SHA_RE.fullmatch(c["sha"])] if isinstance(commits, list) else []
        if not rows:
            raise UpdateError("not_found", "No commit of %s found at %r" % (UPDATE_SUBDIR, ref))
        sha = rows[0]["sha"]
        raw = self.fetch.get_bytes("%s/repos/%s/contents/%s/VERSION?ref=%s" % (self.api, self.repo, UPDATE_SUBDIR, sha), 4096, "application/vnd.github.raw+json")
        version = raw.decode("utf-8", "replace").strip().splitlines()[0] if raw.strip() else ""
        try:
            parse_version(version)
        except ValidationError:
            raise UpdateError("parse", "the VERSION file at that ref is not a version")
        def notes_date(c: dict) -> str:
            who = (c.get("commit") or {}).get("author") if isinstance(c.get("commit"), dict) else None
            return str(who.get("date") or "")[:10] if isinstance(who, dict) else ""

        def subject(c: dict) -> str:
            msg = (c.get("commit") or {}).get("message") if isinstance(c.get("commit"), dict) else ""
            return (str(msg or "").splitlines() or [""])[0][:100]
        notes = ["\u2022 %s %s (%s)" % (c["sha"][:7], subject(c), notes_date(c)) for c in rows]
        return {"version": version, "tag": "", "asset": "", "ref": ref, "commit": sha, "published": notes_date(rows[0]), "notes": "\n".join(notes), "has_signature": False}

    # -- stage -----------------------------------------------------------------
    def stage(self, cfg: Config, runner: Runner, info: dict, work: str, emit: Callable[[str, str, str], None]) -> str:
        """Download and verify what ``check`` found into ``work``/bundle (files + MANIFEST.sha256). Returns the directory."""
        dest = os.path.join(work, "bundle")
        os.makedirs(dest, mode=0o700, exist_ok=True)
        if info["channel"] == "stable":
            self._stage_stable(cfg, runner, info, work, dest, emit)
        else:
            self._stage_main(info, work, dest, emit)
        names = [n for n in BUNDLE_FILES if os.path.isfile(os.path.join(dest, n))]
        for n in BUNDLE_REQUIRED:
            if n not in names:
                raise UpdateError("archive", "%s is missing from the update" % n)
        ver = read_text(os.path.join(dest, "VERSION")).strip().splitlines()
        if not ver or ver[0] != info["version"]:
            raise UpdateError("archive", "the downloaded VERSION (%s) is not the version that was checked (%s)" % (ver[0] if ver else "none", info["version"]))
        code_v = re.search(r'^DEFAULT_VERSION = "([^"]+)"', read_text(os.path.join(dest, "synofurb_mgmt.py")), re.M)
        if not code_v or code_v.group(1) != info["version"]:
            raise UpdateError("archive", "VERSION and DEFAULT_VERSION in synofurb_mgmt.py disagree: refusing")
        write_manifest(dest, names)
        return dest

    def _stage_stable(self, cfg: Config, runner: Runner, info: dict, work: str, dest: str, emit: Callable[[str, str, str], None]) -> None:
        base = "%s/%s/releases/download/%s" % (self.dl, self.repo, info["tag"])
        sums_raw = self.fetch.get_bytes(base + "/SHA256SUMS", MAX_SUMS, "application/octet-stream")
        sums = sums_raw.decode("utf-8", "replace")
        if info["mode"] == "signed":
            minisign = runner.which("minisign")
            if not minisign:
                raise UpdateError("signature", "cannot verify the signature: minisign is not installed (apt install minisign)")
            sig = self.fetch.get_bytes(base + "/SHA256SUMS.minisig", MAX_SUMS, "application/octet-stream")
            p_sums, p_sig = os.path.join(work, "SHA256SUMS"), os.path.join(work, "SHA256SUMS.minisig")
            for p, data in ((p_sums, sums_raw), (p_sig, sig)):
                with open(p, "wb") as fh:
                    fh.write(data)
            if not runner.run([minisign, "-Vm", p_sums, "-x", p_sig, "-P", update_pubkey(cfg)], timeout=30).ok:
                raise UpdateError("signature", "SIGNATURE VERIFICATION FAILED for SHA256SUMS: the release is not authentic")
            emit("download", "ok", "signature verified (minisign)")
        expected = parse_sums(sums, info["asset"])
        archive = os.path.join(work, info["asset"])
        digest, size = self.fetch.download("%s/%s" % (base, info["asset"]), archive, MAX_ASSET)
        if not hmac.compare_digest(digest, expected):
            raise UpdateError("checksum", "SHA-256 MISMATCH for %s: nothing was installed" % info["asset"])
        emit("download", "ok", "%s %d KiB, SHA-256 %s... matches SHA256SUMS%s" % (info["asset"], size >> 10, digest[:12], "" if info["mode"] == "signed" else " (unsigned)"))
        try:
            with tarfile.open(archive, "r:gz") as tf:
                seen: set[str] = set()
                for mem in tf:
                    name = mem.name[2:] if mem.name.startswith("./") else mem.name
                    if mem.isdir() and name in ("", "."):
                        continue
                    if not mem.isreg() or not bundle_member_ok(name) or name in seen:
                        raise UpdateError("archive", "unexpected member in the archive: %s" % name[:60])
                    seen.add(name)
                    _write_member(dest, name, tf.extractfile(mem), mem.size)
        except (tarfile.TarError, EOFError, OSError, zlib.error) as e:
            raise UpdateError("archive", "cannot read the archive (%s)" % type(e).__name__)

    def _stage_main(self, info: dict, work: str, dest: str, emit: Callable[[str, str, str], None]) -> None:
        archive = os.path.join(work, "repo.tar.gz")
        digest, size = self.fetch.download("%s/%s/tar.gz/%s" % (self.code, self.repo, info["commit"]), archive, MAX_REPO_TARBALL)
        emit("download", "ok", "commit %s, %d MiB, SHA-256 %s... (no signature exists for a source tree)" % (info["commit"][:7], size >> 20, digest[:12]))
        top, seen, n_addon, b_addon = None, set(), 0, 0
        addons_dir = os.path.join(work, "addons")
        try:
            with open(archive, "rb") as raw, gzip.GzipFile(fileobj=raw) as gz, tarfile.open(fileobj=_CapReader(gz, MAX_REPO_UNPACKED), mode="r|") as tf:      # type: ignore[arg-type]
                for mem in tf:
                    parts = mem.name.split("/")
                    rel = parts[3:]
                    if len(parts) < 4 or parts[1:3] != UPDATE_SUBDIR.split("/"):
                        continue
                    flat = len(rel) == 1 and bundle_member_ok(rel[0]) and rel[0] not in BUNDLE_GENERATED
                    adn = len(rel) == 3 and rel[0] == "addons"                # addons/<id>/<file>: packed into addons.pack.json below, never installed as files
                    if not (flat or adn):
                        continue
                    if not mem.isreg() or (flat and rel[0] in seen) or (top is not None and parts[0] != top) or not re.fullmatch(r"[A-Za-z0-9._-]{1,120}", parts[0]):
                        raise UpdateError("archive", "unexpected entry in the source archive: %s" % rel[-1])
                    top = parts[0]
                    if flat:
                        seen.add(rel[0])
                        _write_member(dest, rel[0], tf.extractfile(mem), mem.size)
                        continue
                    n_addon, b_addon = n_addon + 1, b_addon + mem.size
                    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,31}", rel[1]) or not re.fullmatch(r"[A-Za-z0-9._-]{1,64}", rel[2]) or rel[2].startswith(".") \
                            or mem.size > MAX_ADDON_FILE or n_addon > MAX_ADDON_FILES or b_addon > MAX_ADDON_BYTES:
                        raise UpdateError("archive", "an add-on definition is not acceptable: %s/%s" % (rel[1][:32], rel[2][:32]))
                    os.makedirs(os.path.join(addons_dir, rel[1]), mode=0o700, exist_ok=True)
                    _write_member(os.path.join(addons_dir, rel[1]), rel[2], tf.extractfile(mem), mem.size, MAX_ADDON_FILE)
        except (tarfile.TarError, EOFError, OSError, zlib.error) as e:
            raise UpdateError("archive", "cannot read the source archive (%s)" % type(e).__name__)
        finally:
            _unlink(archive)
        if "synofurb_addons.py" in seen:                                      # the same packing make-update-bundle.py does, by the installed (trusted) module
            if addons_mod is None:
                emit("download", "ok", "add-on definitions skipped: the installed version has no add-on framework to pack them")
            else:
                try:
                    pack = addons_mod.build_pack(addons_dir)
                except Exception as e:                                        # noqa: BLE001 - AddonError or OSError: refuse, nothing was installed
                    raise UpdateError("archive", "the add-on definitions in the source are not valid (%s)" % str(e)[:120])
                write_atomic(os.path.join(dest, "addons.pack.json"), pack)


def _write_member(dest: str, name: str, src: Any, size: int, limit: int = MAX_BUNDLE_FILE) -> None:
    if size > limit or src is None:
        raise UpdateError("size", "%s is too large" % name)
    fd = os.open(os.path.join(dest, name), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)       # name comes from a fixed allow-list, never from the archive path
    with os.fdopen(fd, "wb") as out:
        left = size
        while left > 0:
            chunk = src.read(min(1 << 16, left))
            if not chunk:
                raise UpdateError("archive", "%s is truncated" % name)
            out.write(chunk)
            left -= len(chunk)


# ---- job lock, job records, settings -------------------------------------------

def _pid_alive(pid: Any) -> bool:
    if not isinstance(pid, int) or pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except OSError:
        return True
    return True


class JobLock:
    """`<state>/job.lock`: at most one update or add-on job at a time, across processes (the web service, the transient unit and the
    app's SSH session). Created with O_EXCL; a lock whose process is gone is stale and is removed. Add-on jobs use the same class:
    ``acquire("addon", label)``."""

    def __init__(self, cfg: Config):
        self.path = cfg.p(cfg.state_dir + "/job.lock")

    def holder(self) -> dict | None:
        try:
            with open(self.path, encoding="utf-8") as fh:
                d = json.load(fh)
        except (OSError, ValueError):
            return None
        if isinstance(d, dict) and _pid_alive(d.get("pid")):
            return d
        _unlink(self.path)
        return None

    def acquire(self, kind: str, label: str, job_id: str = "") -> dict | None:
        os.makedirs(os.path.dirname(self.path), mode=0o700, exist_ok=True)
        rec = {"kind": kind, "label": label[:80], "id": job_id, "pid": os.getpid(), "since": int(time.time()), "token": secrets.token_hex(8)}
        for _ in range(2):
            try:
                fd = os.open(self.path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            except FileExistsError:
                if self.holder() is not None:
                    return None
                continue
            with os.fdopen(fd, "w", encoding="utf-8") as fh:
                json.dump(rec, fh)
            return rec
        return None

    def release(self, rec: dict | None) -> None:
        if rec is None:
            return
        cur = self.holder()
        if cur and cur.get("token") == rec.get("token"):
            _unlink(self.path)


def _update_dir(cfg: Config, *sub: str) -> str:
    return cfg.p(os.path.join(cfg.state_dir, "update", *sub))


def load_update_settings(cfg: Config) -> dict:
    try:
        with open(_update_dir(cfg, "settings.json"), encoding="utf-8") as fh:
            d = json.load(fh)
    except (OSError, ValueError):
        d = {}
    ch = d.get("channel") if isinstance(d, dict) else None
    ref = d.get("ref") if isinstance(d, dict) else None
    return {"channel": ch if ch in UPDATE_CHANNELS else "stable", "ref": ref if isinstance(ref, str) and REF_RE.fullmatch(ref) else "main"}


def save_update_settings(cfg: Config, channel: str, ref: str) -> None:
    write_atomic(_update_dir(cfg, "settings.json"), json.dumps({"channel": channel, "ref": ref}), mode=0o600)


def load_check(cfg: Config) -> dict | None:
    try:
        with open(_update_dir(cfg, "check.json"), encoding="utf-8") as fh:
            d = json.load(fh)
        return d if isinstance(d, dict) and "version" in d else None
    except (OSError, ValueError):
        return None


class JobRecord:
    """One update/rollback, whoever started it (web, GitHub CLI, app push, manual bundle): the version history and the live progress."""

    KEEP = 30
    MAX_EVENTS = 80

    def __init__(self, cfg: Config, data: dict):
        self.cfg, self.data = cfg, data
        self.id = data["id"]
        self.path = _update_dir(cfg, "jobs", self.id + ".json")
        self.echo: Callable[[str, str, str], None] | None = None

    @classmethod
    def create(cls, cfg: Config, **meta: Any) -> "JobRecord":
        jid = meta.pop("id", None) or secrets.token_hex(8)
        if not re.fullmatch(r"[0-9a-f]{16}", jid):
            raise UpdateError("policy", "bad job id")
        rec = cls(cfg, {"id": jid, "state": "queued", "started": round(time.time(), 3), "finished": None, "exit": None, "events": [], "pid": None,
                        "kind": "update", "source": "manual", "ref": "", "commit": "", "by": "", "via": "cli", "from": None, "to": None, "unsigned": False, **meta})
        rec.save()
        rec.prune()
        return rec

    @classmethod
    def load(cls, cfg: Config, jid: str) -> "JobRecord | None":
        if not re.fullmatch(r"[0-9a-f]{16}", jid or ""):
            return None
        try:
            with open(_update_dir(cfg, "jobs", jid + ".json"), encoding="utf-8") as fh:
                d = json.load(fh)
        except (OSError, ValueError):
            return None
        return cls(cfg, d) if isinstance(d, dict) and d.get("id") == jid else None

    def save(self) -> None:
        os.makedirs(os.path.dirname(self.path), mode=0o700, exist_ok=True)
        write_atomic(self.path, json.dumps(self.data, separators=(",", ":")), mode=0o600)

    def prune(self) -> None:
        d = os.path.dirname(self.path)
        files = sorted((f for f in os.listdir(d) if re.fullmatch(r"[0-9a-f]{16}\.json", f)), key=lambda f: os.path.getmtime(os.path.join(d, f)))
        for f in files[:-self.KEEP]:
            _unlink(os.path.join(d, f))

    def start(self) -> None:
        self.data.update(state="running", pid=os.getpid())
        self.save()

    def emit(self, step: str, status: str, message: str) -> None:
        ev = self.data["events"]
        ev.append({"t": int(time.time()), "step": step, "status": status, "message": message[:300]})
        del ev[:-self.MAX_EVENTS]
        self.save()
        if self.echo:
            self.echo(step, status, message)

    def set(self, **kw: Any) -> None:
        self.data.update(kw)
        self.save()

    def finish(self, state: str, code: int) -> None:
        self.data.update(state=state, exit=code, finished=round(time.time(), 3), pid=None)
        self.save()

    def summary(self) -> dict:
        d = self.data
        last = d["events"][-1] if d.get("events") else {}
        return {k: d.get(k) for k in ("id", "kind", "source", "ref", "commit", "by", "via", "from", "to", "state", "exit", "started", "finished", "unsigned")} | {
            "step": last.get("step"), "message": last.get("message")}


def list_jobs(cfg: Config, limit: int = 15) -> list[JobRecord]:
    d = _update_dir(cfg, "jobs")
    try:
        names = [f for f in os.listdir(d) if re.fullmatch(r"[0-9a-f]{16}\.json", f)]
    except OSError:
        return []
    out = [r for r in (JobRecord.load(cfg, f[:-5]) for f in names) if r]
    out.sort(key=lambda r: (r.data.get("started") or 0, r.id), reverse=True)
    return out[:limit]


def reap_jobs(cfg: Config) -> None:
    """A job that says it is queued/running but has no live process (power loss, killed) is closed as failed, so the page never waits for ever."""
    now = time.time()
    for r in list_jobs(cfg, 30):
        d = r.data
        if d.get("state") == "running" and not _pid_alive(d.get("pid")):
            r.emit("result", "fail", "interrupted: the update process ended without a result; check the unit")
            r.finish("failed", EXIT_ROLLBACK_FAILED)
        elif d.get("state") == "queued" and now - (d.get("started") or now) > 90:
            r.emit("result", "fail", "the update process did not start")
            r.finish("failed", EXIT_REFUSED)


# ---- the shared flow -------------------------------------------------------------

def update_audit(cfg: Config, rec: JobRecord, action: str, result: str) -> None:
    d = rec.data
    AuditLog(cfg.p(cfg.audit_path)).log(d.get("by") or "-", d.get("ip") or "-", action, {"job": rec.id, "source": d.get("source"), "via": d.get("via"), "ref": d.get("ref"),
                                                                                         "commit": (d.get("commit") or "")[:12], "from": d.get("from"), "to": d.get("to")}, result)


def preflight(cfg: Config, runner: Runner, force: bool) -> str | None:
    """Reason to refuse an update from GitHub, or None. (The app checks health itself before a push; `--from DIR` stays as it was.)"""
    if force:
        return None
    if not cfg.dry_run and runner.run(["systemctl", "is-active", "synofurb-mgmt.service"], timeout=10).out.strip() != "active":
        return "the web UI service is not active: fix it first, or force the update"
    parent = os.path.dirname(cfg.p(INSTALL_DIR))
    try:
        free = shutil.disk_usage(parent if os.path.isdir(parent) else cfg.p("/")).free
    except OSError:
        free = 1 << 40
    if free < 64 << 20:
        return "less than 64 MiB free next to %s" % INSTALL_DIR
    try:
        doc = collect_health(App(cfg, runner), quick=True, budget_s=20.0)
    except Exception:                                             # noqa: BLE001
        return None
    if doc.get("overall") == "crit":
        return ("the unit is in a critical state (%s); an update needs force" % "; ".join(doc.get("warnings", [])[:2]))[:200]
    return None


def run_update_flow(cfg: Config, runner: Runner, spec: dict, rec: JobRecord, *, fetch: HttpFetcher | None = None, probe: Callable[[Config], bool] | None = None,
                    health_timeout: float = 30.0, sleep: Callable[[float], None] = time.sleep) -> int:
    """check -> stage -> verify -> swap -> health -> rollback, with the job lock, history and audit. ``spec``: mode (dir|github|rollback), src,
    channel, ref, expect, accept_unsigned, force, restart_delay. Exit codes are the Updater's: 0 ok, 2 refused, 3 rolled back, 4 rollback failed."""
    lock = JobLock(cfg)
    held = lock.acquire("update", rec.data["source"], rec.id)
    rec.start()
    update_audit(cfg, rec, "mgmt-update-start", "started")
    work, phase = None, "check"
    code, state = EXIT_REFUSED, "refused"
    try:
        if held is None:
            rec.emit("lock", "fail", "another job is running (%s): wait for it to finish" % ((lock.holder() or {}).get("kind") or "unknown"))
            return code
        cur, _ = installed_info(cfg)
        rec.set(**{"from": cur})
        force, src = bool(spec.get("force")), spec.get("src")
        if spec["mode"] == "github":
            gh = GithubSource(fetch or HttpFetcher())
            rec.emit("check", "start", "asking GitHub (%s)" % ("releases" if spec["channel"] == "stable" else "ref " + spec["ref"]))
            info = gh.check(cfg, spec["channel"], spec["ref"])
            rec.set(ref=info["ref"], commit=info["commit"], to=info["version"], unsigned=info["mode"] == "unsigned")
            rec.emit("check", "ok", "%s %s, %s (installed %s)" % ("release" if info["channel"] == "stable" else "commit " + info["commit"][:7], info["version"], info["state"], cur or "none"))
            expect = spec.get("expect") or ""
            if expect and expect != info["version"] and not (len(expect) >= 7 and info["commit"].startswith(expect)):
                raise UpdateError("mismatch", "GitHub now offers %s, not the %s you were shown: check again" % ((info["commit"] or info["version"])[:12], expect[:12]))
            if info["blocked"]:
                raise UpdateError("blocked", info["blocked"])
            if info["mode"] == "unsigned" and not spec.get("accept_unsigned"):
                raise UpdateError("consent", "this update is not signed (SHA-256 only): pass --accept-unsigned, or tick the box in the web UI")
            if info["state"] == "current" and not force:
                rec.emit("result", "ok", "already up to date (%s)" % cur)
                code, state = EXIT_OK, "current"
                return code
            if info["state"] == "older" and not force:
                raise UpdateError("older", "version %s is older than the installed %s (use --force to downgrade)" % (info["version"], cur))
            why = preflight(cfg, runner, force)
            if why:
                raise UpdateError("unhealthy", why)
            os.makedirs(_update_dir(cfg), mode=0o700, exist_ok=True)
            work = tempfile.mkdtemp(prefix="stage-", dir=_update_dir(cfg))
            phase = "download"
            rec.emit("download", "start", "downloading")
            src = gh.stage(cfg, runner, info, work, rec.emit)
        elif spec["mode"] == "dir":
            vf = os.path.join(src or "", "VERSION")
            rec.set(to=(read_text(vf).strip().splitlines() or [None])[0] if os.path.isfile(vf) else None)
        phase = "update"
        upd = Updater(cfg, runner, rec.emit, probe=probe, health_timeout=health_timeout, sleep=sleep)
        delay = float(spec.get("restart_delay") or 0)
        if delay > 0:
            upd.before_restart = lambda: sleep(delay)
        code = upd.rollback() if spec["mode"] == "rollback" else upd.update(src, force=force)
        if code == EXIT_OK:
            rec.set(to=upd.installed_version())
        state = {EXIT_OK: "ok", EXIT_REFUSED: "refused", EXIT_ROLLED_BACK: "rolled_back", EXIT_ROLLBACK_FAILED: "rollback_failed"}[code]
    except UpdateError as e:
        step = {"consent": "consent", "blocked": "check", "mismatch": "check", "older": "check", "unhealthy": "health"}.get(e.kind, phase)
        rec.emit(step, "fail", e.message)
        code, state = EXIT_REFUSED, "refused"
    except Exception as e:                                         # noqa: BLE001
        traceback.print_exc(file=sys.stderr)
        rec.emit("result", "fail", "unexpected error: %s" % type(e).__name__)
        code, state = (EXIT_ROLLBACK_FAILED if phase == "update" else EXIT_REFUSED), "failed"
    finally:
        if work:
            shutil.rmtree(work, ignore_errors=True)
        if held is not None or rec.data.get("state") == "running":
            rec.finish(state, code)
            update_audit(cfg, rec, "mgmt-update", "%s (exit %d)" % (state, code))
        lock.release(held)
    return code


def default_source(src: str) -> str:
    return "app" if os.path.basename(os.path.normpath(src)).startswith(APP_STAGE_PREFIX) else "manual"


def run_update_cli(args: argparse.Namespace, cfg: Config, runner: Runner, emit: Callable[[str, str, str], None]) -> int:
    """`update --from DIR | --from-github [--ref R] [--check] | --rollback | --status` (called by run_cli after the root check)."""
    who = args.by or os.environ.get("SUDO_USER") or os.environ.get("USER") or "root"
    ip = (os.environ.get("SSH_CONNECTION") or "-").split()[0]
    meta = {"by": who[:64], "ip": ip, "via": args.via or "cli"}
    if args.status:
        reap_jobs(cfg)
        print(json.dumps(update_state(cfg), separators=(",", ":")) if args.json else "installed: %s" % (installed_info(cfg)[0] or "unknown"), flush=True)
        return EXIT_OK
    if args.check:
        if not args.from_github:
            emit("check", "fail", "--check needs --from-github")
            return EXIT_REFUSED
        try:
            info = GithubSource(HttpFetcher()).check(cfg, "main" if args.ref else "stable", args.ref or "")
        except UpdateError as e:
            emit("check", "fail", e.message)
            return EXIT_REFUSED
        print(json.dumps(info, separators=(",", ":")) if args.json else "%s: %s (installed %s) - %s" % (info["channel"], info["version"], info["installed"] or "none", info["state"]), flush=True)
        return EXIT_OK
    spec: dict[str, Any] = {"force": args.force, "restart_delay": args.restart_delay, "accept_unsigned": args.accept_unsigned, "expect": args.expect or "", "src": None}
    if args.rollback:
        spec["mode"] = "rollback"
        meta.update(kind="rollback", source="rollback")
    elif args.from_github:
        spec.update(mode="github", channel="main" if args.ref else "stable", ref=args.ref or "")
        if args.ref:
            try:
                valid_ref(args.ref)
            except UpdateError as e:
                emit("check", "fail", e.message)
                return EXIT_REFUSED
        meta["source"] = "github-main" if args.ref else "github-stable"
    elif args.src:
        spec.update(mode="dir", src=args.src)
        meta["source"] = args.source if args.source in UPDATE_SOURCES else default_source(args.src)
    else:
        emit("verify", "fail", "update needs --from DIR, --from-github or --rollback")
        return EXIT_REFUSED
    if args.job_id:
        rec = JobRecord.load(cfg, args.job_id)
        if rec is None:
            emit("verify", "fail", "unknown job id")
            return EXIT_REFUSED
        rec.data.update({k: v for k, v in meta.items() if not rec.data.get(k)})
    else:
        rec = JobRecord.create(cfg, **meta)
    rec.echo = emit
    return run_update_flow(cfg, runner, spec, rec, health_timeout=args.health_timeout)


# ---- state and web routes ----------------------------------------------------------

def update_state(cfg: Config) -> dict:
    reap_jobs(cfg)
    inst, sha = installed_info(cfg)
    prev = read_text(os.path.join(cfg.p(INSTALL_DIR + ".prev"), "VERSION")).strip().splitlines()
    jobs = list_jobs(cfg)
    active = next((j for j in jobs if j.data.get("state") in ("queued", "running")), None)
    lock = JobLock(cfg).holder()
    s = load_update_settings(cfg)
    chk = load_check(cfg)
    if chk:
        chk["state"], chk["installed"] = _cmp_state(chk["version"], inst, chk.get("channel", "stable")), inst
    return {"ok": True, "installed": inst, "sha256": sha, "previous": prev[0] if prev else None, "channel": s["channel"], "ref": s["ref"], "check": chk,
            "active": active.id if active else None, "busy": (lock or {}).get("kind"), "jobs": [j.summary() for j in jobs],
            "signed": update_pubkey(cfg) != UPDATE_PLACEHOLDER_KEY, "repo": UPDATE_REPO}


def _channel_ref(body: dict) -> tuple[str, str]:
    ch = body.get("channel")
    if ch not in UPDATE_CHANNELS:
        raise ApiError(400, "channel must be stable or main")
    try:
        return ch, (valid_ref(body.get("ref") or "main") if ch == "main" else "")
    except UpdateError as e:
        raise ApiError(400, e.message)


@route("GET", "/api/update")
def api_update(req: Request) -> dict:
    return update_state(req.app.cfg)


@route("POST", "/api/update/check", mutating=False)
def api_update_check(req: Request) -> dict:
    app = req.app
    ch, ref = _channel_ref(req.body)
    try:
        info = GithubSource(getattr(app, "update_fetch", None) or HttpFetcher()).check(app.cfg, ch, ref)
    except UpdateError as e:
        raise ApiError(502 if e.kind in ("offline", "http", "rate_limit") else 400, e.message)
    save_update_settings(app.cfg, ch, ref or "main")
    write_atomic(_update_dir(app.cfg, "check.json"), json.dumps(info, separators=(",", ":")), mode=0o600)
    return {"ok": True, "check": info}


def launch_update_job(app: App, argv: list[str], jid: str) -> None:
    """Start the job OUTSIDE this service's cgroup (a transient systemd unit), so restarting the service cannot kill it."""
    custom = getattr(app, "update_launcher", None)
    if custom:
        custom(argv)
        return
    full = [sys.executable or "/usr/bin/python3", app.script_path] + argv
    if app.cfg.root:
        full += ["--root", app.cfg.root] + (["--dry-run"] if app.cfg.dry_run else [])
    if app.runner.which("systemd-run") and not app.cfg.dry_run:
        r = app.run(["systemd-run", "--unit", "synofurb-mgmt-update-" + jid[:8], "--collect", "--quiet", "--description", "synofurb management plane update",
                     "--setenv=PYTHONUNBUFFERED=1", "--setenv=PYTHONDONTWRITEBYTECODE=1", "--"] + full, timeout=20)
        if not r.ok:
            raise ApiError(500, "could not start the update job: " + (r.err or r.out).strip()[:200])
        return
    subprocess.Popen(full, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True, env=RealRunner.ENV)


def _start_job(req: Request, argv: list[str], **meta: Any) -> dict:
    app = req.app
    reap_jobs(app.cfg)
    h = JobLock(app.cfg).holder()
    if h or any(j.data.get("state") in ("queued", "running") for j in list_jobs(app.cfg, 30)):
        raise ApiError(409, "another job is running (%s): wait for it to finish" % ((h or {}).get("kind") or "update"))
    rec = JobRecord.create(app.cfg, by=req.user or "-", ip=req.ip, via="web", **meta)
    try:
        launch_update_job(app, ["update", "--json", "--job-id", rec.id, "--by", req.user or "-", "--via", "web", "--restart-delay", "3"] + argv, rec.id)
    except ApiError as e:
        rec.emit("result", "fail", e.message)
        rec.finish("failed", EXIT_REFUSED)
        raise
    req.audit_note = "job " + rec.id
    return {"ok": True, "job": rec.id}


@route("POST", "/api/update/start")
def api_update_start(req: Request) -> dict:
    b = req.body
    ch, ref = _channel_ref(b)
    expect = b.get("expect")
    if not isinstance(expect, str) or not re.fullmatch(r"[0-9A-Za-z.+-]{1,40}", expect):
        raise ApiError(400, "expect must be the version or commit that was shown")
    if ch == "main":
        need_confirm(b, expect[:7])
    argv = ["--from-github", "--expect", expect] + (["--ref", ref] if ch == "main" else []) + (["--accept-unsigned"] if b.get("accept_unsigned") is True else []) + (["--force"] if b.get("force") is True else [])
    return _start_job(req, argv, source="github-" + ch, ref=ref)


@route("POST", "/api/update/rollback")
def api_update_rollback(req: Request) -> dict:
    if not os.path.isfile(os.path.join(req.app.cfg.p(INSTALL_DIR + ".prev"), "synofurb_mgmt.py")):
        raise ApiError(409, "no previous version is kept")
    return _start_job(req, ["--rollback"], kind="rollback", source="rollback")


def _job_id(body: dict) -> str:
    jid = body.get("job")
    if not isinstance(jid, str) or not re.fullmatch(r"[0-9a-f]{16}", jid):
        raise ApiError(400, "bad job id")
    return jid


@route("POST", "/api/update/job", mutating=False)
def api_update_job(req: Request) -> dict:
    reap_jobs(req.app.cfg)
    rec = JobRecord.load(req.app.cfg, _job_id(req.body))
    if rec is None:
        raise ApiError(404, "no such job")
    return {"ok": True, "job": rec.summary(), "events": rec.data.get("events", [])}


@route("POST", "/api/update/progress", auth=False, mutating=False)
def api_update_progress(req: Request) -> dict:
    """What the page can still read after the restart dropped its session: state and version numbers only, found by the unguessable job id."""
    rec = JobRecord.load(req.app.cfg, _job_id(req.body))
    if rec is None:
        raise ApiError(404, "no such job")
    s = rec.summary()
    return {"ok": True, "job": {k: s[k] for k in ("id", "kind", "state", "step", "from", "to", "finished")}}


def run_cli(args: argparse.Namespace, cfg: Config, runner: Runner) -> int:
    """Dispatch ``health`` and ``update``. Exit codes: 0 ok, 2 refused/bad input, 3 failed and rolled back, 4 failed and rollback failed."""
    if args.command == "health":
        app = App(cfg, runner)
        try:
            doc = collect_health(app, quick=args.quick)
        except Exception as e:                                       # noqa: BLE001
            doc = {"schema": HEALTH_SCHEMA, "version": VERSION, "overall": "warn", "warnings": ["health collection failed: %s" % type(e).__name__]}
        print(json.dumps(doc, separators=(",", ":")) if args.json else "overall: %s\n%s" % (doc["overall"], "\n".join("- " + w for w in doc.get("warnings", []))), flush=True)
        return 0

    def emit(step: str, status: str, message: str) -> None:
        if args.json:
            print(json.dumps({"step": step, "status": status, "message": message}, separators=(",", ":")), flush=True)
        else:
            print("[%s] %s: %s" % (step, status, message), flush=True)

    if not cfg.root and os.geteuid() != 0:
        emit("verify", "fail", "must run as root")
        return EXIT_REFUSED
    return run_update_cli(args, cfg, runner, emit)


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
    ap.add_argument("command", nargs="?", choices=("health", "update", "addons"), help="health --json | update --from DIR | update --rollback (no server is started)")
    ap.add_argument("--json", action="store_true", help="machine-readable output (health, update)")
    ap.add_argument("--quick", action="store_true", help=argparse.SUPPRESS)
    ap.add_argument("--from", dest="src", help="update: directory with the new version and MANIFEST.sha256")
    ap.add_argument("--force", action="store_true", help="update: allow installing an older version")
    ap.add_argument("--rollback", action="store_true", help="update: restore the previous version")
    ap.add_argument("--health-timeout", type=float, default=30.0, help=argparse.SUPPRESS)
    ap.add_argument("--from-github", action="store_true", help="update: fetch from GitHub (newest release; with --ref the apps/mgmt-plane folder of that branch or tag)")
    ap.add_argument("--ref", help="update --from-github: branch or tag of the monorepo (default: the newest release asset)")
    ap.add_argument("--check", action="store_true", help="update --from-github: only look; change nothing")
    ap.add_argument("--status", action="store_true", help="update: installed version and the last updates")
    ap.add_argument("--accept-unsigned", action="store_true", help="update --from-github: accept a download that has a SHA-256 but no signature")
    ap.add_argument("--source", help=argparse.SUPPRESS)
    ap.add_argument("--by", help=argparse.SUPPRESS)
    ap.add_argument("--via", help=argparse.SUPPRESS)
    ap.add_argument("--job-id", help=argparse.SUPPRESS)
    ap.add_argument("--expect", help=argparse.SUPPRESS)
    ap.add_argument("--restart-delay", type=float, default=0.0, help=argparse.SUPPRESS)
    ap.add_argument("--raid-scrub", action="store_true", help=argparse.SUPPRESS)
    args, extra = ap.parse_known_args(argv)
    if extra and args.command != "addons":              # `addons <command> ...` has its own parser (synofurb_addons.cli_main)
        ap.error("unrecognized arguments: %s" % " ".join(extra))
    args.addons_argv = extra
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.no_tls and not args.dry_run:
        print("error: --no-tls is only allowed together with --dry-run", file=sys.stderr)
        return 2
    cfg = Config(root=args.root, dry_run=args.dry_run)
    if args.dry_run and not cfg.root:
        cfg.root = tempfile.mkdtemp(prefix="synofurb-dry-")
        if not args.command:
            print("dry-run: sandbox root is %s" % cfg.root, file=sys.stderr)
    cfg.load_file()
    if args.port:
        cfg.port = args.port
    if args.bind:
        cfg.bind = args.bind
    cfg.tls = not args.no_tls
    if not args.dry_run and not cfg.root and os.geteuid() != 0 and args.command != "health":
        print("error: must run as root (or use --dry-run)", file=sys.stderr)
        return 2
    runner: Runner = DryRunRunner(passthrough=True, verbose=not args.command) if args.dry_run else RealRunner()
    if args.command == "addons":
        return addons_cli(args, args.addons_argv, cfg, runner)
    if args.command:
        return run_cli(args, cfg, runner)
    if args.dry_run:
        seed_dev_accounts(cfg)
    app = App(cfg, runner)
    if args.net_rollback:
        done = rollback_network(app)
        app.audit.log("system", "-", "net-rollback", {}, "ok: timer fired" if done else "nothing to do")
        return 0
    if args.raid_scrub:
        n = raid_scrub_all(app)
        app.audit.log("system", "-", "raid-scrub", {}, "ok: started %d check(s)" % n)
        return 0
    startup_recovery(app)
    reap_jobs(cfg)
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


# BEGIN GENERATED ASSETS (tools/build-assets.py)
# Do not edit by hand. Lucide icons: ISC licence; Inter and JetBrains Mono: SIL OFL 1.1 (texts in _LICENSES, shown on the About page).

_FONT_FACES = (
    ('Inter', 400, 'd09GMk9UVE8AADM8AAwAAAAAiDAAADLsAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAADYGyBxpqG8o8HIE6BmAAg3QBNgIkA4coBAYFgngHIBtahwXTbT5yOwClUJlRs6M1bJxmAD8f6Sik24PTN9n/lwNuyJB/qFUXSGqrM5jMrp5Hu45+NJbTwYwSNalvyqdWphBDFHWNoiz3em7l8qvgmpMPSwV8HIef/XFsmbKkYHosWDgcIY1JRn14/tQ/9+UFhVMofYWRmhOSIqkmqJM75f91KUelE3zBVQv3ITmENB1f/vn+nt86774LefaDJtBE8jyiIEs8aybSUD68Df/P8/uffoVFVu1UT/X+Bo0Zcf2NKCFmhBDR5kWVQUQNCJG2RMQexISII5G5z53nvgGYW60Yy2QRwUYsiAVsILE10RsdI8pAMRMxC/W0LzEvOvEVXy6gEFtl1FKFpnlmiAwE64gwekIyIPZKfVY5vV9IVChWnuNdBhQ8XK7lVzkaf6oq/lTxVDEllS9g/NrOZPw+uBTVnv6VJX5EVYK87bBta7iIaq9MVTvvPgjp4fhQSBXh+O8Qi4aUc6C7lKfH3z8gPJ4UgkiPQdAjBicQzKBiCpVTAJQIKoJ0DF0MuU6xerHSuNK4ktS56Fy6j6mtXLp1Z3duatelm8rwfC2dvYcqXbhLopAYW52+XerL0tpBV8yfqMhDxiicbF4hvOD/1lJ7cwEApwDQx5jcn4V3O7cbJB0uSN77szQHuKmKqq4x8RXSj5CNa2W7hUI+2aX5/VxMJrBeqZEXUJcWY/v4u3LJmrHQ78F+TIfjNALGJURIQ4wrRERK7/6DJmAZEEL0/zBuhPfPLCJzJzvuhw0/KvmRpIsfZH4k835/QPG+c79mJW8TE8+Cx6v2r4U2d/OeW0e8o4bo7v5p/dt5T2Vg47sKD69gghAxYMqaIw/+QsVKka1Yb4M1GGe25TY74IizNO544rXPkX89IyaEI54kkEQTqSwlepGpsoS0kzNzEUwr5H5+tuBfAelqHKRUDFR747BYqFXeSV3zJHqYPbpBPjwgyuFdMI3ZydbKi/Gn7PMQVS5856xWKvLuXiOPxd/eQJQ1T0T9Gsz9PlJhocGvBcpWDWy8FBLG+wTZiFqZHMQMeDXYD/8kGOzsRNe8OGvkisj44yXSCIrGaVhuTNuvb0N/ubyYqxeK26tD3lClginwFJQVlemnqLiGIbUNRYiEQLg3pLv71tcrh5XSGeSjNYmOuaK0/51R/IaWNwinosjnTAaqb6hp8GlKxNmK8csG5jW6KYZfDCYuMBiZEIy7wgDCAXbDXmiGnQX2wMxijoN94Az//U8V7D/gutAy+H/geUINftAOnTAHOqB14L9xDA4fCF04DpGQAKc25kMibIAkuADSweXBxUHKwtVB9yDjxHXIhVtwBxbBbbgxyFtYAkWDB8g38BDuDUoWHsMjqIA1sBX40BcGQD8weiDTOQiBgT7AwSjQGkRBDPSAXmAPLAwGQ+gJ6yAa9EEJM6AR4kEEMuCBHIaBHhjACLAe1EMlDAdbsAMJ6MIW2AZrYTX0hmUQB7GgA5thB6yHWTAPGmAQ8stgFTTBRhDAGBgNk2A8TAGC/BGshFcHCJ40lybHVkLzDs0///nPt/759EErD5oRQ2LzuDDeGjxq8I2DX55nrXn65jljnk+yyuXy3/m/bOF/+QW/ePeX43753K8O+PVvfj35NyN+c0k1ojqwarDb/vab3/3hdztcN+36v/x+wg1/+cOGNzz+x//74+gbr77x8z8dVQyt3/nz1MpbV1nlb7/621d/363a2/9Y6R8vdtatcWfNHWq+9c+ta30x76rzvjdk4SE71plaZ8YQKZ6BRPuJoB10wwKHVcjecZyTndNBku3cTr1c9jNX1zXsHq9x+yyy3JdIF48Z8pXK9Tys/tSNXp3GyHuz1bbHZ7EL8f3i2/zuhAb/s2EhBgLeJ33glhwWVBFsHry96tQ/ZGOLDh3fvvSVffsYGP59oqaLqI/YOp8sXuTyLdomqiBqUNTsvX9fPfHniy6Lbj4fb2bM4rvsrnyFsTNe+x8T15n+5JT4nvGT8q/SmJib2I312IxW3MdjwsmSBkkHS0dJ53MTT/P3Rk72SM5K3ts+mzQlM2WqddiTjuiq1N6pDan7JJA7bYVUOqlfR0b6hznnAp/yV4ud9XUzsuuzP52RnK/fndgYW6Lt4hb5BfnF+WX5G5/O3xuHSvqHSoXKkYTFEOUZIVlrCJWi9monXg23I7MGuPX3i4/GbZ/9z/xn1Z7Puvk/YU9unAL4pUB4Rd6WdT+on+fQI5xLnh/98P/DBkwazbDk/x/e6gUoqaQpoAz/1DCs+039DcffPcuENJY4abMjDxkP6zPwg+EwhCmjTg0qRUBnTH1z64YSD/LD8t8W/fvh7e5cLSkyV9fDZQiT+hgk04+CXrGW0X0lVih7BJL3jzGyEefLY9ofTkce9f7ghIKyUtyDtcQYLgjipK3IcIEthhUFbjDmZKLghojR9esHfBt8Rkx336rXLY1/lMvJy48+9gq8wpEtGJSss8WW60LVzJYJo1QQzkiorctznSzRa0ouE9Hok5QexoIq6YEnP8g8OSCtC7Q5E58jemei+kAuinIZ6RGSKgP3JBpxskjmnA+XSJRJhn6EfYlaLZHTD6E9jOTtdN+vKW8M/U3Ih1EmEVli8lIjPQReaJaoKcdCXyMF+GokISdWmzCfDVBpRXOor9Y8nlL+Q3457UW7uMI2gxwC7ydR1VDGJ4WgiCKaBQWx3cB7XJnyotG66zXK8UvQEsgpX6OiJWmbdcpXC6GIv+oktpm1kIQMXHmoZEQiwKb3h3I2vBU6e4U7lHO/Lxb+gRTUJs9p8cA8xPZ4F/Wz7H6mJdsedMSaYARRqauhU5RivE1nKdcmpdrbdLrNhrYf0emqQ15ZamoMOiF0Buk37HAzeZb4X6/tVv8H1cFyUXRlZgBTah2lg5i6Grj1OXcvhSbxtl7/+REnbtcNBvpRbyhlG67va+2B65RL6OYditPIm9zQoqEWQ1Fhs7rETNc6+0hzoq/WIrNVYwC9xCg52jcRNkWhoFO82+lO9TrwURJRhDIuKURFVaPdoDosMOpp+1JhtJUue5WK/YYB5jtyg5p0wNcKHyrtlwxMeNDUcyEp7NDM3tsEWvQOll0IXFUvUKAbWBsaYt0xRpznO4NPr7u6wQRTY3Y3ubrJhDZDw6oHgMlmPtR4crNMudxXaw92y4Dnku7NOjINeK5wzsiFrASraPg1zkneXiKtpBnnlGOD9ig8P3ryynCoSX1RNvUqQz2o/aP5quL83tDD67LxtD2xxILLL7j8kDxTR81F+ADkhRqxGiJn6Uq1kLMqPyCj+/g3PwSKP1WO08zPJ21/wrbbO6e+7/0QSkX6ajOgOz56RtmPKC1Ptea0pBNQcla6QP9w6MzwvwOrx36r72DJuMBznNDbS7z/1WzB3YzwQVTpkjjKB9/n+rWTLDvRvhTMNY/P/9KQLOd734Z612dQFYj/sFtpYogXRZjribGeobs9m+1Cd6NwN9qAaOoU6hTl/mFzuK8uBHxSVyk3J6XFJ3WtJdhUQ6f3LUGnMyBH7dk2AiIqnDoKM0cVdQ0VxKTF+zD9e2b9I8q1nfhmO1R+CWlRp4nmA3pqaGh9eRwnWn63uGCY8kksbnqMfMW4KWFKW5GHXnShoD4vgBGWhihLxmeixZws4cbau1HxBVqv/8vuouHUCvzp9BVfHzB96/T2gS/qhWl3Y9+HaOBdZryw0NTQSap+e7N2J54mRKUXiCvXhlwVb8ggbOa3dtUNPUyycO58fQ/pD2xQHZ5JtM8GRf0n2W5G8vYO15T/CT1AmuA7U4kjsXMJ0z2mvWfLr/jmio8nIbIzQ8XIfm0JGUX5Vi347IbeKzcqsVrNvhDbHbn1CRNho++QO9d2siELprWqiurZDVK9f3wv4pXSq8+jB+0HoXbkr8v/Q4UvzJQKF6f0bIOCG4mDQe8Gj/O9Yejk2Bc3wKOfZ082ogML/YqLVIinEp307k9s+A+W3S0Db7H9UaxNxBVpzuJWICdOWP7yiFL56AT1uErpvg6+RLptJhsTF4trrJavl3KNCSXuaOboUoSJE60e6nKwnWjfOErWdAhd7vUqs0kfQPXiN51YqC4N7TW+b9jfRZU654l6VuxAs+FgeMSKqRvfjeo2tQtQOm43WmqtV4GniEzaEBvVpdjEvquYu/VYGeshFrk2g49SyppxZYPlKQuIha3HwWcqpziobaHXHEOxlV9baCtT5twCKL9Y+e4ALPZefPvwN0xrbqSG7SQZ/336jS4sXIqqZkMpp1An0pauI24W4JrZB4TJa029rt4NLVJbBxyhDlSUeBhmRgfEx8OEfUM+MOzcX+x9TR5vQnqyKw43h7hicvYyab9QDwg3DxB3P1P3NqmhxiB4kanEf5xCzUh7CibeEGjKE/f/mA8sOHnCDANTu1pMqA21R5rtUVY3kXATcpvfjFEm7jl29PDc6vv6x5YGpklvQrv+HI9eU1RiKEDYQaR7EuVqF3JJ7pGvtz2x3cq7Dd5t4dzt891e2ubd9I3ScxlGVwcgnbusu70RZr5dFIrurG3S7cGAF6DphKan8sirj/TffzUbYbcmEDMGloPpsv+/0vmW3yf+dhZj1h+z3o7rZ0890BXVsNJ0YC0P//x+50d5EcLKkBPR9AT6y9f8L3JwJvFvtIwrUpmbx6Q3P/pQRpCGj1YinORksfyoc1JyULrmQPdvT7YmbpfVONXDrUqsnUCzpzAsEVHx94h6OS+f7/BVqzeANYFZBjvBJSBPZw7p/lf7eU4J+3FUZ9DRZJL3lNtqddvizt4t5TkV6txR3f/2TMovoZePOoMeqiXCiGFSFZc/2CwCiAohDLFxBT7Tv9hi4FKF4h/rDLpy0DTUY+qjH0NbrPMxjAAVdAZd3vN569NASvOBtQqx66uHhcrppkq2vEKgIWwS/CGlpe3/1c6GLrBZ6OAQYYl8goezuYWYERSdpyX2ODiVYNdd2bBcWTobb0p8PcBMlYH+YEVgT2pYeAq5jWxblWypA2jw1PdltqZlNYOANSskRyRxnpYeikTK+kKnCSVldxLcAcDNnJj8rCY0qQcXOoWDrgU8PUV64q3V6VPgVgY5YeyYPBpRboRdx3TA9io3yoyr0JSNZpihBjhAxtoVBKwMvC7RtAAyqRpUzz9LG5ChxKMGON9RRNJlLbq2rPJNXKJdtvwf2rLNEvIplaQz7td4o0cu2yDP7i4aKITbE3XNXRyjLpCE31e4PfGpzPu1sUJ2COpm8CukyjGykPg3VXlQr2B31Zo2aeAbnvtsCF9ureIf1TKht5gP5Xa3jh6Uh9o45O1vQnnv4FoNvCfGlNN5lTVCJtqEzha4zijaG3E0GvvkukpwK/gUCbmTPEn8VKPdrVNwYz1DlAEoH0crGfp+cQPVw1OsATYBp7nrDHaGK7QbNzAYYJkV/5CsCbb7tIuFIA2KGaZIMJL/pno9zP+dptiT5JfEv6rUMX0I/qlJqi90uuBSyA9axl9Lf7083mIGWLu74eynph45O4Mtz/Y/U03BSBUdpqkNujncoDpCZ1e7ojF1F2pvRwgYCHDfEwKTD7DykXee5AGp44CLxl/RCZxMNAuQYGKZW56I1aGIHgijC7uw/ayrJpbFduNah3GTcPhy0hl0ZNiOcHYcgfZ4ai3R3WxjVBH+7n6knR0amnidbOaKsHc+l5cj3B14mckrOAtcxMBZ2z+fFi8aLxsrwTCthVMaQO1aApQBxxJ1/bC4vhpsEsdxSiX2vZO+b/Q19gktYItbTMvPIyAjppcbx7axuNxzVonj3vHVA910TcuS8mAfAK7MVW9aeoqqywvsp92nyq1f5tjDnE9e2PzhdWE9xo1maLLB46NfguwdzshxoDrjNfaqsbM6peNFYvOsS2j20WNfZCWTLj2+mhr2BgbvsPFwGH9yyftkpBYMEX6CGxpCN02KJdWzsKmTV7QhhMBRVO3+0CBqpxtCxakv3eGu0Xa24/FvRPM4rA35+xGYWnW0cDnnbPyx78vFZ6KCCPywPMElvS2xDFutvM8gLdylR/F9LjEGME65ceWZt8N64R0EJxHW0qPDy93EXvATS8A0QlhjghU5QVUG3w7Vjo0thEutBPkKURuuZX1OUI05cwvDBQXhjIDC78RQh1A+wIGA+XtUtQbxb4kWvKtinncJJP4269y+79fLX0vbvGLnHu0jjGm5F5kMAW1QgyYqWjZa1GniYo08heR7mLT+LymcxDH0kzDU9Fj+haGTgy7nSRFNDu65hdNo8+NHXQgoYry3ngB7CFv2B2FZhtw0eRplqLwZt6x3aXc6q/MGXlCo3v/cMa/zUcU+ld/j/IEPzqqrzqzdXdrFKbgoXFkkS8UDGSJdz4MOd0yQ/j93HQ8O6qV/gX20ssHWH8vYQEP/pfLup21STY3BP+EVWvZm63OgS3XC8ZL9X/tN+NXyfJe6pbafxLEwRiEECZ4J97Hv8VeeBbnj0Imzz7gVTCew1Xd1XB6LUEYZf2xXtpWzB9fajbMlwOFEqkVmL+10H7beJVNvHfe/0mfJ57NeFh0k5Mwqt3ZPJd+jMdIZLYhj4AnicALxJ9Z9Pb8qtkvl7xQCHfjIGHoli8+/qrXgBNo8urew6fRddZcDMoxUDKpz5aJQDqL7bSCegN9IDtLxi4GYlZi2pHEWwhIIp4XISHYMMgArEgvmH2kNOBVhcygVaS/OVCYeT1RlingOXAmXSPz7vxcamRJN2iuxN4C3EpdVZH1W7Wfl5+5C7RhCM2EXwjNhSyePRJeKniNwrqBDFID/ceCFTa8xVsrY9+QFJ98Bi7DoP6XfkyYpFEPVTuXM0bUkGPNUv5ITR0O2ToXZvS7UT5nPaANtAGtpNndaGUmT+2iShsXsc1KZo14l+SPQ/E4M3wc0vKW9ncGvxTOmCDP2Q/gEk+8+IUHeZNDHm0kBkxHSxH5oJW6+vnQt3RRRHjRlHTFkKSvBCoKaiaMRmpRVf72c1hk0sKNU1trxOMOuUYHQz6t14ne7wtjhpQcFj72kFwJFk+OJniLGLXRg04Na5YGfH/jNuK9zCg1hu6qSWlRZyD6REpX1MVn4SebeYX7eYcEdYU4JKSVq10eq+Qc09B1fZjsUexkz1ryxSc9xytxT/L2NTNPKUNgPVUIsbs8IvjOk5Vhl7QEARybhvwZWZj1PoQiSSRvigdfvuHjHmZyy4ZTB9/dOpXhDKIpaZrkXBVFuh6uR+W6JrVfeq/+e6+fuL+7x2pi30hy1hGtRgbaXohoubiurr3xs38PPgfQY8M5Lysoy2ok4uRS+vCZUzbZw0FfjvvkjraQPPI29F4stbVj54IBuzsPORhtxcnUGV9ue0uTp3cNOfNF0p9w4ESi6YAKZowcnuFDlcXNnDQL8xOQ1TXnk7Tj14nvXo5omy6GHeNgB/kT0Uaj/A++wHPLjCYfGfPDjdybicxOF5oOBUXM4hOeCKzF4BKZwUgxe2JUXlY26JZ04td7OoPBao2Lp1xZ/con/sqbxMWOP53d9Yud0dL6YZlBot4ghrqlvmHdv4Hkk2vaDEbOXVgKvG0KAdnyD/ar87taLr/e6icOqx38netcc7XSePGlzxzW1O2gtNw4nqp67uL5qQd1871pSkLjKQBtdA9WT5Se31kIRKBi6e7VtcuGBE/wT3PQyUdaF5EcZtNKloYzn+2lwrlJ84ApQSXjlG+fy8MaDXGADiY3tv5ifPCJoz4ZllwX35jGSpUPoMo/IaaMuHZZCKb2mvDYmMgWALTBufmccFiCqidYG3Grxk5ifCPvtF9JcfdQy6SKFEOOJ1ajoMmsFe+wYZY72M2C5lY07Epo3Q+gj313t5oYDkn63kMsagR28DJQWXnUKnBHeN942d5D3E7+tg0kzPqwqpbZ9SjfZFblf8TtyqvagMlmTvVWIsktcbyS8dcKYGfoAnXmysDj88aMQwbvoQVEfFmSGY8Vx5jlQvwebEwq0GSGF73X+oBeOU1F1qkDUzXakLD75GDuTabIVsVGwySEHbwztPHp18olnT/sPdoJuRelESB5tHzpJo3GKkj4WQT6QgG+XvCGCVOZNGqcep0viPvB54nTJedmV5zoWliEsD6nR+X4lzjJl20FGkNWUJqGv0FfYp1OUB5LS56DO+XPw5yToc1T0JW1Mfp/Ul4BMtVsJVu2UMUnBCqtOtYJaMdYdVOCSFmOs290KVWtqeGqF4ez8oScDV4b+1lVxwbEB+BwLyj1ZaoquextH+w+3i6SP9Udoj073i4iNiYbtUa5nwPtIqbPJ3lQOQhYBBFIaKBGjFyLaxKzlRjrV2dlOtmeiuqDB9tW6ipvlxlOJCg0W295b4SJJ+d+fhEWWRPVOU6FPgneTRhtPzEgqF50jkTgfSAlK4m85kSZWGzW/bWA0gVNDaEIdGO84bEysMXOYC8BpJOGj48xP+9EFHlPEBl2F6iWh9HLdeBB+OTaGd6q8gDlMdzfCoK8ed2BvNN6LinZV/ZwrZbDZHheh6vgG2lfrIq5VLZ5LdMllqvMRGEGTPUeNyRPRgyU7Goh64XAuR/V6Q1n/jif/+/fpz0AFmGlBdzqzx1Sz7Bq6qAzz1bGip5eVmduvsC3aLV/acOAVj2J14ZPiXPWq8haAJom2RnfjOv3ZP9M/FdEASMMPC94eMuVcU/bUiN0yej34exL1bSpmk/aajjlR6wCZa+eSBGmxI5FNPGnlBrkw3WKSkzjfHEudDdXCKqVQKWpknRgayBkDof2DptmYg2OjuqhB99V6AdcrE68yABZZBifdAucF86b8KBbquX0OyepdsC9wv62kL7H+M7MkGgsnyuZ9g9s5YmHGVYo4Uy+XGFKhLBI7dx3sVDGFUCFbyzDbCCqVZfwIF2FtfEkw0pDZqPCQCfC1UhuXzaAWZRkUjp/PutrWlD/gO1ZEtygfWmG56mNSe/VZJ/HO2Sl1HzmZ+KLqXa1tIC2mvvG2MGOg7n88xDXUELCOVt/uI1q6EE3yhNw2ahNcJgm40lElwrnkLhNLUrkgl2FYh+lcuwXW2R3T3yipI4QnhD+ff6cG5PEpfcVg1IbYJfDCqJ3BuB3xi2KNcVt2/eqh6KoQMoS/OWEES9UFkeg8Eq1ghu6Xx5sV9TcEL0z7hZ65+huONyvpb5K9zdSv87Pi/kbhkeCjBAXZ9QICeKkWG0lbHva0yIIjOXVMPYOTT/Fefgdv8tTqEDx6E/rksD61yQxY3vU0t9LqXGw3/Pj0g6szti3RlvKoJldnel5pvpyyI2docRXN4H2Jrn5ZwNLojTTTh9zTBrRi8tGKLohP1XrNocw6/6nm8a1EHxpQpJZrTfbApPEbX7ZgfDXxtjW9184W6Gsax/sMloXTmPw6tiY6Hwg+9hNbJsFVkuWOTd9geKK5QNaHZ8g6AaQIz9HQCHwkcSyweLvhZp7IXgjnxO1IrBP2DieSxP2Bj85Z+oXmkLeiC/y5+At0NEJdUqV79FmlTAS54iRk/RkTn8baRLFAR0YK4QuAFaE0DvYhlsRzgeFTlpJ5L+gLm8ctWoLAEp8PbN26t9NE4XNVFj5CqvAXQpIG/yHU+x3d8kmGfBBnZ561F8RgFTg7ze+rVFb1d9ubmroHbpui4bcQ/n1lb911tO2BgtEr1ffXchPFKRwDq3zC0Rm0X6ufIAGKA4e30dKx56PtDuIw9TpnY6JOUwxEjcwm+W2f5f1ZtayIIg1JI5V4Flkknf/IVpK0OzFNKDZxn0ZXD7DdzoiutQr+NghSCyZY+Il+DL2YfIoIuzUvCWAQ/sjTJCAMJKKptaq0yttGfVn3c+KJJl7stU2fWpNZgzFUf3qz/mh6a9IKPMm7+GSJNRm9qXaNpzYlKkxsxHWh4ZW5HpWjdZ2igkWIwm8A7RtIS+6trruOz3ajI7hfqpJ7FUlBp4v4+AMfwgOsu/O1jXKuvn/n2dMpCRZ/fpqn0ykIw8ZsvAKMYem1A59OA75Ye2pTsIyIFjsra7PqL1813FpCdKzeCu13erwmQvisxKGXxLVECi+SLookZ+iLzXhWjrVM2gZL3uuvAEtaIhREnq7xz5UP+2oxsXLab1/n3KXfhVqQI9oYT0x/RjgcSWciRh7Sdo8+6XcKNQR4OwT78N0502IlcpotobvBHURoLilKzHx5vu+Wfp9ON8HY+YgncUxuZ+sG6JSO1Cus3mDXOpO3Llpx32JJ8tl/rgj7aT54p27VcL3yV6On823y4wfTFUcKwEvr0SIZtsLGE3fLqqdWZemqlY0u7q4KV6K5zC6vnk5uqndDrEGtIazABPwPVFLHQnbZmeoiRNV0ag1VX5yeyZFCLIGvqEkXanrn/ftMfistZrogThe33xalegeMsbwWHp8N4K3kQpkEf0UIBQYNGIUcb7D1Vhw1GrOXX24zuurqz9uWEpPCl9IoG11tOK4SO7fd1uSV06xrBGiu+2VxqkAseyW8wGjucTYZvo7BugBm4nEI9tI7Mx9Osd97/wnc0IuV2lS3NqoYQVW7Ua1tQgfa+NVZN25UL1Vuxo3CfuJaQjPRusw0BGG9b3VNXfd4y6mlwJ57v5/KzuLGpn1O2TyJ0C4nMzpJeSwpAzqoy6UcSjVu9Xvki8Qfj7PIYliGWa/wcdJIwnrEcnE82vPB9o+bBJ4kNpeR2cRnxXKzZkAKvpox6cs8D11hnNQ7/T2dusdGf4QvJEXuC4XZXoX2PvnWv1LxWtJ+qU/8ZfkBViif1c5nkCw4L5H5KwmUArLx/dpkMvH4KoY5ANU9TQmFVOjFogm1oeslDzvrqQIKdWa6VspwexpthXW9Fi21sCSuT/I6D7KS17pJew2Ypyz3SVqkFEAXmMt/SF7fmHwjZDy7Kg00vX21BJj64F9fp8imhzDChdlix0uz54TDzET8nU1e82+YJJKGl45KYEnM95YJ08K0yYqxO86/SpovIWiERyGeRbHG8J1COc6Yhn1X8C+NxKJUJ4T8k0SJOUkib2MeoKRaG6yg6vCWFWm7gopoUm9Z2ucI2Z54i32u88rFqmp3Na7O4L/WNqG2u9PAQiZUtW+X5M48U2vrEJ0fW0M6VPW88hdJzf/pISuiSQ+679S2Inrmi6qrdlc56O5aWCUGlzj31q81c92TOA92t8oCA1s13a2u5kwnf9LGdArgnD/lX7yNKO/qaC0nFihYdrYy/MFmJ8DJCCxAp80p2OUk9uCoDF0NVJtnurvANszX1v2OJmvoFuTma3wMrr1f7J3+bZ/L0LjH6Dywvtbi1taP+Rc8XbsSdEElUs9gs6h0o53KjuU9rnzN+jRBggZ68LH54kUiItuhvJXu3qislrC2M7Ts0hzlxewELWqtzQ3UFVt/VcTbZ3QR8XMDaApbvuiszov7tZolm0sNs1bjFZtplno3z1nlO5hV0iPDalZ3hqApC8J0lqpnnFmyeMmzuli9UdMJtuh9WKPaaDT7+b2uCaVMYEJTkp97t3nplK6rqTTFxU63tN7kRH6xgYdLMsxQAyXPTZ8CIRubfq/1Pt6pXfi9NMmHivw++D3SWXehUlISGb1HFuLVXkWKVYSnEpIQq1WgsbjIKxCekqlnOeAqZY+zQY4p3jVfGGgXHgMVjWk3bz3Mu4niF3h0PfBhJJT31TgrBCJuZt89DE/+tf0+46Eq/3RXPqQoFvyrfqVsSYrFjGstYHqQlh8tNovAFAtZyFb7UzKjGfxpUhL0SoMJmLwC4U2y6JUg+Cjf1WRCAQf3c1Azfr3ENzqYdqC++ZdbJ5AfzjEjO10oShQLKQRUKgiz1r6qSSpV9f4KOFMi13RpmIAD45/Myu+X+uatWsuwQnJn5OKVb4WUXgbOJ0orSX7iPCktVz4geT2cLu/xJ4y5n7//3XTOmX5vORokI5dffiRU/rp83eytz7rtnDvOvbPgnyt9T558nHOJXg+Ua9jntoO9BLMAoacyjgyef2jD1/AWm2fRJdkmRiIPEuhyRJqY9ekSSoRvOD916y5O0s6aMGLcXhvutyWsxOLbSkeTJx466Hiy8leCdo/XE1IluiVqATVgmsS4OnkzUI1pkrkJ3EAYlk5MJGWjwLAnPlM8qXgeW3JiEMkqgZFbLG0oaTibzcSIrA5tFql/GYemDcW/TewdRdzPPiS7S/S29H4Y2ez8/Gl2xnRTOoMOrCe4zQKpDyOhllGYDrln1yL39H56o+QUGTp+aSMnGJAznSzw/N8XpM7aGK/s9v621EmGLrJNPTCF1m9glgl4w8MyW6BBsAE6a/ZvMC3J1+PeXvfp5Ax9f/pTAc+ZA7tQPktnrfYffEbJse5d7V/hy6T/qXdEBNE4tdTEYPK7fwbVExJn9MobbsSzUsVadw98/Kyf82yhBHrpifGpczbGa7s+uyHdsuqhyqrO4Em1A8g1xgVWhtpEjvMQPVUoE1Ve0C+7lLoxI8SJ1o04V1IJHGpT4tSP+SYRx9RvXAb2E8YCRJkyIV57/r8/wWdc2ZH+ySbb77zwaukffSAq/1zKu6hadVjxLeYkcy0vmX85nCOBLp3xHN8lIps0e2k0uJDIFj493JQZnfHU63f9AJ9yZhdGJ/9afceRsBrC2HgdCcMbOzAs2XDNHYbDQnS785lDaQc60zxpFlcRVKVbHgpZUXfrvcbWUeI7XXVdriPEOPn8b9UF+mHxFyGb/JMu8F4inEGzl5YFqwh6KSJKaRDPT9t8AZ7lxJ6MSDbaZZt1Vksf9o3osTsxtnYpcrLxgQ2hgGRxPqKXEjKiqtC6qcLXBUpgfEiNED6+RSghFkCUW+FS5A/bl6I35M2l7obeHtbv2Txtl6tdf3Qgg8q0EBcWGXY3up9+2B+I9u7XcmDSt5iWfOfvN/SS5AK9d8XB8/yYCLSEQLaClYS+/EnBpNTo7GnXbjgGz7NtAWOTdcfssCYshzAh3kdzpqdNZlCy0ca7rgr/YsS1j63Hyeh8I4Rb3SbovtSqN2a8zVQnXt2vloc/F+fFKWThkbyAOPabj6ue2WQJVmE7Zc7SvPL1z4GS5s/svTOy9049WJ/pHD3rESx6j29W9znzrs9nwXfTd3Sm/Yp7LTW2y3wF1WuWg1G138vSGTR7Wg5h4C+1m8x2ANyxcDpRf64oa59oNnfitbq9jTA/ig9d1QtK87U20jxgka7Z17EvoNl75qeKmoLulxQNV+3Ruz45hhkWqnNtzBwl8tBZq+Zql/a6uBcMbHdhePf7Bh9c/P6lzL+qyUNHjUpnHfpK6xvDT3JL7mnDBZ/clAuS4q91I7+SbL7icqijdRLU7c95+0rpDPqsPhEEoAsAAIYBIhYRLxeRry+iv4GIwWoRDRoRwwxDjDASMcEExCSTEYssRmy3A3HbXcR99xEvvUQQgDYnAabJJCVnFkK/um+jAvpgAeDnsFdVNbBeARFwVxwstHQMtGGsZyHAUMM/Je/liy5QPatqysmNkZGhgRFsM79ITatGn3tv/jDajIW3kCOOSWZymWK23FsR04epZBqZ+QIdKRnumvZC7MB3u45vvGSeiGe1vPzrTN7SrV6q9Wi92AuR4fJk4Dtd1uUsZMvZPvzlTbOuWtwg/6dtW42gwFVJmw5dxqxYs8Gx58CRE2cuXLnx5MWbD19+/AUIFCRYiDRZcuUrVqpMH/30N8BAgwwmI1epSg0FpTpDDDPaGGONt8Emux3Q4hC1Vm3adejU5aiTzrrokss0ul1x1TXXPfAST2esq8ClGM/RrBNpF0MCqtDRwIXIxesxTeRqFs+GmCsEfRFrRltPGiqljIExKI8ikXO6l1lEHE06YXEpg5QHJORt8g/WBlFSFMrFoHuSeIwpPpAOs8tlMiogQyNg6zkb5pRjhqOACDv16mxSuaTLWaQJBEW/Q7AtYptJ5u05voAnmLqrokmJQVfKiaE03LQuY/Bg4SNAiAirrmeGfygEHnBbatvY68jTSdeIlUkq73GOzLJRdZVI10N3bm5PCzuiu5bA6N6Mpxuq35szTuS7dN1A0o6Nxa6AgAhKCWavMlZFDQqUHohtuW5ana1i2zG6S7t2zfB6VcZ4pvm/GuQGW5OuOIE2dQJoPKkyVAMUlTIlSD4cO1TdxmkvRIMPAoMMoKc1F6mFC9msyo7RdYzvJw7PJH9EYFYQSzBeIWHIUAJSAabyk0VIYOzcA+i1JLQVooJ8/xFl0iKlzZ/Y0yWlhz4GGGKEsSYRYkwxxwJrbLBVAux6XMQeBxxxwhkXXYGb7p7F6yV444Nv8VNZK8XCOJIx0Qgiz3H5oIBCiiimF33oF5VJNYYCpXXz7rCs4Q0YSUd1jnbH0HFgfOyO7Mt4IGvL0TbrXU7TeAVc9QYLsefCvHppnpNrCaslcMg2p/2IaO4vNz+wYP1p60eMMOG8xJa1bdMy60stU1bE71Xxnc4XMI/PowrfaLu0WCn7+VdA7x+gNpgmekaDOSchcG2gOyldFB5T2VJZDEbBqD4VrINMbqdhxra5ilFODkrF6U+tjYnnTvMA6XvfchVNJuhCUlExr6SBBik6CLqWrkF37jp0hqm7iPfRP5eEZwJx7syk7qrWq91q6Zuq84Q2c2sWy140Qbiq8R5xb0wSqxxJJf/raEc4f8usJS1aK8rWkRrzkOeOq45TfhUHCcNEmkZ0RpRjIEbn0iBPhHOvxRZMbZN80iR1gTAg8KKA/1MKJFbOrcrf0KUoTgFdj8QabhRGTVssTM8CeXD9gaUk/HScWoAmPzWdFOQVFWjAKxVmKjTq0yhuA8ZITC3K9H07FF6O6kz5yhuWooo607RmOS1FFHBuECWWVEbwGjQd6+JVQgk+31XUaW4c3Y+aBpUJBfOAag4FWxrLcs5bLUVEeyl2+Lfc9MSqbrfQbg/og64NeUS5T4vug4+wVULRSM4YaJUwsyKQdrNjwF2RsmjaVJ6bWgCzzhm2RaGoM035w9y4gXVOH8WoceN+MdVbqFSV0ype2Ind/GHza/CAniRVVaqpkgCtni9NWfVUfO9eQ0KoA7WdYqErZ/gO2vdEmhhSxwvptBB8WQGzLh9rS3u3aUQiQWEB9Jw0O22H9LQyXamaJE3E5Bw1EyJhl3sXS0EhzA9JdhxSSWYIoSm2x2A9bldNnLbeC1BgOiXt7LqJSjhJURJ0RvHZFmZi4CXVRABlgyT4uJ2wADFYCjWCDIwomu7fgSEt1X5UouOzZj8qZknf8mmyD1AhHSgj6ho1c6u6v3nuZSBnH6KqZzi1cB2oJInt87tfc0nt4AO4yxK7jbXd9tOJro7pkAH6Rvu6xWl6iCSrCng5JOv14i5PeNq29TbqNOIf0Pv+lXOdKv5Y69egXOb6kW2GP+bWErMWuNz8WjoopoZC22kSkGcqC0X4H+n6XHhAQ4lcHaaACoT4l7YZNbZj/wDatu5aOhHiH4niu8vyM+z9e5bYVbsHMbOFtxkh9Lwv9U9RAAFJgA19DvU28DZDS01yqA9HPaBhIjR1nH8gRdXiqmM8VCv0Hd4DCsSKOgvFaaBOlUdzVfj7BzgFt9D5HtCogpNN6Bddu9atkEb59LP3xhfGrkDmM0u4ggSzMtArUZV+rS7imkWbTbQu2q1oxA0Zzn5+ej+k8aYBeAhBFfIIOGqGLqIWUK/R3cOM0jDCrx1u85kzyl77THaAylQHHTfdKRcsc9kNTW65bcs0DNwmEW+UP9MJGTEvyhLqPFlBhMWdp4gpa4DQrOSACk0Y0yMCrhNHlnQ3KsEUdkGKAqRSOzYYWpyLCHuaQe7GEbihNrTpMyLA82Sd6RSyDshiXDQePQAEAaJ4CG7Nwt5DEMIZKGyihvFZZn1VZEE+gypt+fDiyzeS3WboyjUQIhS0hgwkW6KhCwD8wA3H8uOO2G7eoqHxrE5F0BJsaCImI0xeG+GYtQZirrKGdPVBLmUbLetL58wJZ0hKxkj2RUfosyUgm+aQeRCakXnByHUQAiJYujZrlrRlC3Q09A0AjnyEcOERTysBGnUB7vxQ2u3ZoFNtMKI2q1LMiQNnzpEsmYEVLwU9eamMUCy+y9C55C9cV3bseG86X6H9biDQtCn9rNUhNSBGvkQYqexNMHGK1jV9iqmmmW6GmeaZb5HFFlhoiVlmm2OupZZZboWVVlltjSZrrbMeQ2SY8YCtdtGmomKoQycjhx1h4oQLTGloSDzznB1225P54AP/lT0PWAkc5t/sY208/sIZzX6R61Kt6jjz37MK0IV7MRZ2XQToo+ZdtXRgP9IgR/jEC0lMmURadh6ceCuGeLT5CjfWLrT3kbPkbDkHzsV+Id+Y/nPfeD8R1ml+NuEsOBu2PUXowoOfB7Ntldse7L4frLeAkKEuRIdrtYET+8xGrZo1eGADta220Titg0qnvepxn3DKSaqeMKFFmy49xkyImU4Ic9uS4CaCOd15GJBxEtiXaYjtDtjhskseCRUrXqIkadJlyJQnX4FCRUr00lsfffU3QKUq1WrU2mmfXS54bpNGQ+0xxH43PXbFNcdc1O2Jq8677Y6HBrnhqKduue64e+66bwsWnwAPQ0hEhz4DhoyYs2DJzEt209mW78DeWY7nVrA6/UQKEy5ahCgxEiRLkUoqW45cWeoVq9BDT2VKnVM+wU03kFLcySEAFgJvO8G9j/raAEQ8EQDg4f/NiXxt61J71eId3AUZw2E0ohGlMf09o0kWkPtaw0PrlXEklT9F0DO9zyszisobRm7bCSouSodi/JH4Caq4AN2ZKDpxeg0LDLBPBqZaFyELPuGmkhhp1QZCBC43uQ4a9dDPL1D5REZ85BqCcBVO/N1KhJGMRbltJXA+0wQFF5Lhs1XYkS7fIObFMW2rpu8DRkND90ofgU3YwjxjAElax8iCqDd8jqzRAhDFOmTlubdw1byjShMxvvCMe0O2ehuHkDdbMr0QH5eCiqswVjPb2E6iHqEKp90gSHhMoS5GNh59eGglHzFGT8O5q4lvGMk0/fgMEsfpZ67hQ2zH8TgCD+Iy/B+X65bFRbV6vq66HIq14fnRfOMQq99kQ6MTO/AKqkIexU5YwRkB4wIrBd+VLFlYVxxz+bpBee0W9sFHulGVY37go6Y2UiMOnjM8j/hk+wgYd11BpW02HwAirIwdP2DMrNMNgqZq01QQKJZsjNdjvveKR3hlMo7tBkCXawaa8S0+wqMhNXgBt114HnQL8I7Lh0G9vSBbLrZ1s4ldYXpXqkdDQQ1kUaJ/Z6jtAoPfSRmvChnTx4Dd9ixLbGv8L0U94qVxJc5WJQ7wshJuGPAfEh9yD6/u5fgoPx4Iq4XLYgTayB0D/XQ8ZJ4MC6tT4SMJcWHGnYCV/IpESrVGqCennVEjTsB03AjBKZwxMNIoNI7WF3P7VqnyrfTP2cYb4srmn+s15DK/d/B6W61vNQriw29m5IryWO+3EuxCGmgrwcJLJKWbBcofiSLwvvyeORTHihk3DeSUcRY4gbxC9nnCzf7tmQ0A'),
    ('Inter', 600, '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'),
    ('Inter', 700, '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'),
    ('JetBrains Mono', 400, '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'),
)
_LUCIDE = {
    'check': '<path d="M20 6 9 17l-5-5" />',
    'x': '<path d="M18 6 6 18" /> <path d="m6 6 12 12" />',
    'shield-check': '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z" /> <path d="m9 12 2 2 4-4" />',
    'users': '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" /> <path d="M16 3.128a4 4 0 0 1 0 7.744" /> <path d="M22 21v-2a4 4 0 0 0-3-3.87" /> <circle cx="9" cy="7" r="4" />',
    'circle-question-mark': '<circle cx="12" cy="12" r="10" /> <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" /> <path d="M12 17h.01" />',
    'circle-dashed': '<path d="M10.1 2.182a10 10 0 0 1 3.8 0" /> <path d="M13.9 21.818a10 10 0 0 1-3.8 0" /> <path d="M17.609 3.721a10 10 0 0 1 2.69 2.7" /> <path d="M2.182 13.9a10 10 0 0 1 0-3.8" /> <path d="M20.279 17.609a10 10 0 0 1-2.7 2.69" /> <path d="M21.818 10.1a10 10 0 0 1 0 3.8" /> <path d="M3.721 6.391a10 10 0 0 1 2.7-2.69" /> <path d="M6.391 20.279a10 10 0 0 1-2.69-2.7" />',
    'triangle-alert': '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" /> <path d="M12 9v4" /> <path d="M12 17h.01" />',
    'ban': '<circle cx="12" cy="12" r="10" /> <path d="M4.929 4.929 19.07 19.071" />',
    'octagon-x': '<path d="m15 9-6 6" /> <path d="M2.586 16.726A2 2 0 0 1 2 15.312V8.688a2 2 0 0 1 .586-1.414l4.688-4.688A2 2 0 0 1 8.688 2h6.624a2 2 0 0 1 1.414.586l4.688 4.688A2 2 0 0 1 22 8.688v6.624a2 2 0 0 1-.586 1.414l-4.688 4.688a2 2 0 0 1-1.414.586H8.688a2 2 0 0 1-1.414-.586z" /> <path d="m9 9 6 6" />',
    'info': '<circle cx="12" cy="12" r="10" /> <path d="M12 16v-4" /> <path d="M12 8h.01" />',
    'server': '<rect width="20" height="8" x="2" y="2" rx="2" ry="2" /> <rect width="20" height="8" x="2" y="14" rx="2" ry="2" /> <line x1="6" x2="6.01" y1="6" y2="6" /> <line x1="6" x2="6.01" y1="18" y2="18" />',
    'hard-drive': '<path d="M10 16h.01" /> <path d="M2.212 11.577a2 2 0 0 0-.212.896V18a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5.527a2 2 0 0 0-.212-.896L18.55 5.11A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" /> <path d="M21.946 12.013H2.054" /> <path d="M6 16h.01" />',
    'cpu': '<path d="M12 20v2" /> <path d="M12 2v2" /> <path d="M17 20v2" /> <path d="M17 2v2" /> <path d="M2 12h2" /> <path d="M2 17h2" /> <path d="M2 7h2" /> <path d="M20 12h2" /> <path d="M20 17h2" /> <path d="M20 7h2" /> <path d="M7 20v2" /> <path d="M7 2v2" /> <rect x="4" y="4" width="16" height="16" rx="2" /> <rect x="8" y="8" width="8" height="8" rx="1" />',
    'memory-stick': '<path d="M12 12v-2" /> <path d="M12 18v-2" /> <path d="M16 12v-2" /> <path d="M16 18v-2" /> <path d="M2 11h1.5" /> <path d="M20 18v-2" /> <path d="M20.5 11H22" /> <path d="M4 18v-2" /> <path d="M8 12v-2" /> <path d="M8 18v-2" /> <rect x="2" y="6" width="20" height="10" rx="2" />',
    'network': '<rect x="16" y="16" width="6" height="6" rx="1" /> <rect x="2" y="16" width="6" height="6" rx="1" /> <rect x="9" y="2" width="6" height="6" rx="1" /> <path d="M5 16v-3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v3" /> <path d="M12 12V8" />',
    'power': '<path d="M12 2v10" /> <path d="M18.4 6.6a9 9 0 1 1-12.77.04" />',
    'gauge': '<path d="m12 14 4-4" /> <path d="M3.34 19a10 10 0 1 1 17.32 0" />',
    'layers': '<path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z" /> <path d="M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12" /> <path d="M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17" />',
    'refresh-cw': '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" /> <path d="M21 3v5h-5" /> <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" /> <path d="M8 16H3v5" />',
    'eye': '<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0" /> <circle cx="12" cy="12" r="3" />',
    'eye-off': '<path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49" /> <path d="M14.084 14.158a3 3 0 0 1-4.242-4.242" /> <path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143" /> <path d="m2 2 20 20" />',
    'copy': '<rect width="14" height="14" x="8" y="8" rx="2" ry="2" /> <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />',
    'download': '<path d="M12 15V3" /> <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /> <path d="m7 10 5 5 5-5" />',
    'chevron-down': '<path d="m6 9 6 6 6-6" />',
    'arrow-right': '<path d="M5 12h14" /> <path d="m12 5 7 7-7 7" />',
    'arrow-left': '<path d="m12 19-7-7 7-7" /> <path d="M19 12H5" />',
    'plug': '<path d="M12 22v-5" /> <path d="M15 8V2" /> <path d="M17 8a1 1 0 0 1 1 1v4a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V9a1 1 0 0 1 1-1z" /> <path d="M9 8V2" />',
    'terminal': '<path d="M12 19h8" /> <path d="m4 17 6-6-6-6" />',
    'trash': '<path d="M10 11v6" /> <path d="M14 11v6" /> <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" /> <path d="M3 6h18" /> <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />',
    'lock': '<rect width="18" height="11" x="3" y="11" rx="2" ry="2" /> <path d="M7 11V7a5 5 0 0 1 10 0v4" />',
    'key-round': '<path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z" /> <circle cx="16.5" cy="7.5" r=".5" fill="currentColor" />',
    'clock': '<circle cx="12" cy="12" r="10" /> <path d="M12 6v6l4 2" />',
    'thermometer': '<path d="M14 4v10.54a4 4 0 1 1-4 0V4a2 2 0 0 1 4 0Z" />',
    'activity': '<path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2" />',
    'plus': '<path d="M5 12h14" /> <path d="M12 5v14" />',
    'sun': '<circle cx="12" cy="12" r="4" /> <path d="M12 2v2" /> <path d="M12 20v2" /> <path d="m4.93 4.93 1.41 1.41" /> <path d="m17.66 17.66 1.41 1.41" /> <path d="M2 12h2" /> <path d="M20 12h2" /> <path d="m6.34 17.66-1.41 1.41" /> <path d="m19.07 4.93-1.41 1.41" />',
    'moon': '<path d="M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401" />',
    'database': '<ellipse cx="12" cy="5" rx="9" ry="3" /> <path d="M3 5V19A9 3 0 0 0 21 19V5" /> <path d="M3 12A9 3 0 0 0 21 12" />',
    'undo-2': '<path d="M9 14 4 9l5-5" /> <path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11" />',
    'life-buoy': '<circle cx="12" cy="12" r="10" /> <path d="m4.93 4.93 4.24 4.24" /> <path d="m14.83 9.17 4.24-4.24" /> <path d="m14.83 14.83 4.24 4.24" /> <path d="m9.17 14.83-4.24 4.24" /> <circle cx="12" cy="12" r="4" />',
    'flag': '<path d="M4 22V4a1 1 0 0 1 .4-.8A6 6 0 0 1 8 2c3 0 5 2 7.333 2q2 0 3.067-.8A1 1 0 0 1 20 4v10a1 1 0 0 1-.4.8A6 6 0 0 1 16 16c-3 0-5-2-8-2a6 6 0 0 0-4 1.528" />',
    'menu': '<path d="M4 5h16" /> <path d="M4 12h16" /> <path d="M4 19h16" />',
    'wrench': '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.106-3.105c.32-.322.863-.22.983.218a6 6 0 0 1-8.259 7.057l-7.91 7.91a1 1 0 0 1-2.999-3l7.91-7.91a6 6 0 0 1 7.057-8.259c.438.12.54.662.219.984z" />',
    'package': '<path d="M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73z" /> <path d="M12 22V12" /> <polyline points="3.29 7 12 12 20.71 7" /> <path d="m7.5 4.27 9 5.15" />',
    'settings': '<path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915" /> <circle cx="12" cy="12" r="3" />',
    'panel-top': '<rect width="18" height="18" x="3" y="3" rx="2" /> <path d="M3 9h18" />',
    'search': '<path d="m21 21-4.34-4.34" /> <circle cx="11" cy="11" r="8" />',
    'external-link': '<path d="M15 3h6v6" /> <path d="M10 14 21 3" /> <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />',
    'book-open': '<path d="M12 5v16" /> <path d="M20.001 19A2 2 0 0022 17V5a2 2 0 00-1.999-2L16 3.002A5 5 0 0012 5a5 5 0 00-4-2H4a2 2 0 00-2 2v12a2 2 0 001.999 2H8a5 5 0 014 2 5 5 0 014-2z" />',
    'file-text': '<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z" /> <path d="M14 2v5a1 1 0 0 0 1 1h5" /> <path d="M10 9H8" /> <path d="M16 13H8" /> <path d="M16 17H8" />',
    'recycle': '<path d="M7 19H4.815a1.83 1.83 0 0 1-1.57-.881 1.785 1.785 0 0 1-.004-1.784L7.196 9.5" /> <path d="M11 19h8.203a1.83 1.83 0 0 0 1.556-.89 1.784 1.784 0 0 0 0-1.775l-1.226-2.12" /> <path d="m14 16-3 3 3 3" /> <path d="M8.293 13.596 7.196 9.5 3.1 10.598" /> <path d="m9.344 5.811 1.093-1.892A1.83 1.83 0 0 1 11.985 3a1.784 1.784 0 0 1 1.546.888l3.943 6.843" /> <path d="m13.378 9.633 4.096 1.098 1.097-4.096" />',
    'circle-check': '<circle cx="12" cy="12" r="10" /> <path d="m16 9-5.5 5.5L8 12" />',
    'circle-x': '<circle cx="12" cy="12" r="10" /> <path d="m15 9-6 6" /> <path d="m9 9 6 6" />',
    'scroll-text': '<path d="M15 12h-5" /> <path d="M15 8h-5" /> <path d="M19 17V5a2 2 0 0 0-2-2H4" /> <path d="M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2a1 1 0 0 0 1 1h3" />',
    'clipboard-copy': '<rect width="8" height="4" x="8" y="2" rx="1" ry="1" /> <path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" /> <path d="M16 4h2a2 2 0 0 1 2 2v4" /> <path d="M21 14H11" /> <path d="m15 10-4 4 4 4" />',
    'bell': '<path d="M10.268 21a2 2 0 0 0 3.464 0" /> <path d="M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326" />',
    'container': '<path d="M22 7.7c0-.6-.4-1.2-.8-1.5l-6.3-3.9a1.72 1.72 0 0 0-1.7 0l-10.3 6c-.5.2-.9.8-.9 1.4v6.6c0 .5.4 1.2.8 1.5l6.3 3.9a1.72 1.72 0 0 0 1.7 0l10.3-6c.5-.3.9-1 .9-1.5Z" /> <path d="M10 21.9V14L2.1 9.1" /> <path d="m10 14 11.9-6.9" /> <path d="M14 19.8v-8.1" /> <path d="M18 17.5V9.4" />',
    'camera': '<path d="M13.997 4a2 2 0 0 1 1.76 1.05l.486.9A2 2 0 0 0 18.003 7H20a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2h1.997a2 2 0 0 0 1.759-1.048l.489-.904A2 2 0 0 1 10.004 4z" /> <circle cx="12" cy="13" r="3" />',
    'play': '<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z" />',
    'square': '<rect width="18" height="18" x="3" y="3" rx="2" />',
}
_BRAND = {
    'mark': ('0 0 100 100', '<path d="M68.43 48.92L78.91 67.07Q82.91 74.00 74.91 74.00L65.63 74.00" fill="none" stroke="#7139CB" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="52.63,74.00 66.63,63.64 66.63,84.36" fill="#7139CB" stroke="#7139CB" stroke-width="1.6" stroke-linejoin="round"/><path d="M46.05 74.00L25.09 74.00Q17.09 74.00 21.09 67.07L25.73 59.04" fill="none" stroke="#7139CB" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="32.23,47.78 34.20,65.08 16.26,54.72" fill="#7139CB" stroke="#7139CB" stroke-width="1.6" stroke-linejoin="round"/><path d="M35.52 42.08L46.00 23.93Q50.00 17.00 54.00 23.93L58.64 31.96" fill="none" stroke="#7139CB" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="65.14,43.22 49.17,36.28 67.11,25.92" fill="#7139CB" stroke="#7139CB" stroke-width="1.6" stroke-linejoin="round"/><rect x="38.50" y="48.25" width="23" height="15.5" rx="4.34" fill="#0A0A0A"/><rect x="42.18" y="54.45" width="10.58" height="3.10" rx="1.55" fill="#FFFFFF"/><circle cx="56.90" cy="56.00" r="2.32" fill="#04853A"/>'),
    'mark_dark': ('0 0 100 100', '<path d="M68.43 48.92L78.91 67.07Q82.91 74.00 74.91 74.00L65.63 74.00" fill="none" stroke="#A37BFE" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="52.63,74.00 66.63,63.64 66.63,84.36" fill="#A37BFE" stroke="#A37BFE" stroke-width="1.6" stroke-linejoin="round"/><path d="M46.05 74.00L25.09 74.00Q17.09 74.00 21.09 67.07L25.73 59.04" fill="none" stroke="#A37BFE" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="32.23,47.78 34.20,65.08 16.26,54.72" fill="#A37BFE" stroke="#A37BFE" stroke-width="1.6" stroke-linejoin="round"/><path d="M35.52 42.08L46.00 23.93Q50.00 17.00 54.00 23.93L58.64 31.96" fill="none" stroke="#A37BFE" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="65.14,43.22 49.17,36.28 67.11,25.92" fill="#A37BFE" stroke="#A37BFE" stroke-width="1.6" stroke-linejoin="round"/><rect x="38.50" y="48.25" width="23" height="15.5" rx="4.34" fill="#FFFFFF"/><rect x="42.18" y="54.45" width="10.58" height="3.10" rx="1.55" fill="#17122B"/><circle cx="56.90" cy="56.00" r="2.32" fill="#37BB62"/>'),
    'logo': ('0 0 390 108', '<g><path d="M68.43 48.92L78.91 67.07Q82.91 74.00 74.91 74.00L65.63 74.00" fill="none" stroke="#7139CB" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="52.63,74.00 66.63,63.64 66.63,84.36" fill="#7139CB" stroke="#7139CB" stroke-width="1.6" stroke-linejoin="round"/><path d="M46.05 74.00L25.09 74.00Q17.09 74.00 21.09 67.07L25.73 59.04" fill="none" stroke="#7139CB" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="32.23,47.78 34.20,65.08 16.26,54.72" fill="#7139CB" stroke="#7139CB" stroke-width="1.6" stroke-linejoin="round"/><path d="M35.52 42.08L46.00 23.93Q50.00 17.00 54.00 23.93L58.64 31.96" fill="none" stroke="#7139CB" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="65.14,43.22 49.17,36.28 67.11,25.92" fill="#7139CB" stroke="#7139CB" stroke-width="1.6" stroke-linejoin="round"/><rect x="38.50" y="48.25" width="23" height="15.5" rx="4.34" fill="#0A0A0A"/><rect x="42.18" y="54.45" width="10.58" height="3.10" rx="1.55" fill="#FFFFFF"/><circle cx="56.90" cy="56.00" r="2.32" fill="#04853A"/></g><path d="M134.9833984375 74.6962890625C143.5205078125 74.6962890625 149.5146484375 70.2763671875 149.5146484375 63.8583984375C149.5146484375 59.10546875 146.45703125 56.078125 140.0390625 54.7158203125L134.287109375 53.5048828125C130.8359375 52.748046875 129.4736328125 51.5068359375 129.4736328125 49.5390625C129.4736328125 47.23828125 132.0166015625 45.603515625 135.3466796875 45.603515625C138.9794921875 45.603515625 140.9169921875 47.48046875 141.7041015625 49.841796875L148.7275390625 48.44921875C147.365234375 43.2119140625 142.9150390625 39.73046875 135.255859375 39.73046875C127.142578125 39.73046875 121.6025390625 43.7568359375 121.6025390625 50.0234375C121.6025390625 54.98828125 124.720703125 58.1064453125 131.1083984375 59.4990234375L136.890625 60.740234375C139.978515625 61.40625 141.4619140625 62.6474609375 141.4619140625 64.615234375C141.4619140625 66.916015625 139.009765625 68.671875 135.134765625 68.671875C131.2900390625 68.671875 128.9287109375 67.0673828125 128.0205078125 63.70703125L120.6640625 65.099609375C121.8447265625 71.2451171875 127.203125 74.6962890625 134.9833984375 74.6962890625Z" fill="#0A0A0A"/><path d="M155.15834375 86.0791015625C156.2179140625 86.5029296875 158.1856875 86.896484375 160.425921875 86.896484375C166.54115625 86.896484375 170.0226015625 83.7783203125 172.0811953125 78.359375L186.6124453125 40.154296875H178.28725L171.8995546875 58.560546875C170.90053125 61.466796875 170.1436953125 64.4033203125 169.386859375 67.3701171875C168.72084375 64.4033203125 168.054828125 61.4970703125 167.086078125 58.560546875L160.910296875 40.154296875H152.49428125L165.269671875 74.12109375L164.785296875 76.2705078125C163.9679140625 80.1455078125 161.4854921875 81.1142578125 157.9435 80.17578125L157.0050234375 79.93359375Z" fill="#0A0A0A"/><path d="M199.0979375 54.1708984375C199.0979375 49.2666015625 202.12528125 46.4208984375 206.3332890625 46.4208984375C210.4504765625 46.4208984375 212.9328984375 49.1455078125 212.9328984375 53.6865234375V74.0H220.8039921875V52.4755859375C220.8039921875 44.33203125 216.2024296875 39.73046875 209.2395390625 39.73046875C204.2746953125 39.73046875 200.7629765625 42.15234375 198.7649296875 46.9052734375L198.674109375 40.154296875H191.22684375V74.0H199.0979375Z" fill="#0A0A0A"/><path d="M243.098078125 74.6962890625C252.96721875 74.6962890625 259.29436718750003 67.6728515625 259.29436718750003 57.2587890625C259.29436718750003 46.7236328125 252.96721875 39.73046875 243.098078125 39.73046875C233.2289375 39.73046875 226.9320625 46.7236328125 226.9320625 57.2587890625C226.9320625 67.6728515625 233.2289375 74.6962890625 243.098078125 74.6962890625ZM243.098078125 68.30859375C237.6185859375 68.30859375 234.9545234375 63.404296875 234.9545234375 57.228515625C234.9545234375 50.9921875 237.6185859375 46.1181640625 243.098078125 46.1181640625C248.60784375 46.1181640625 251.3021796875 50.9921875 251.3021796875 57.228515625C251.3021796875 63.404296875 248.60784375 68.30859375 243.098078125 68.30859375Z" fill="#0A0A0A"/><path d="M283.22321875 40.154296875H275.897046875V37.5205078125C275.897046875 34.8564453125 277.0474375 33.3427734375 280.014234375 33.3427734375C281.3159921875 33.3427734375 282.3452890625 33.615234375 283.0113046875 33.8271484375L284.615796875 27.7119140625C283.5865 27.3486328125 281.346265625 26.7734375 278.561109375 26.7734375C272.748609375 26.7734375 268.0562265625 30.1640625 268.0562265625 36.9150390625V40.154296875H261.850171875V46.330078125H268.0562265625V74.0H275.897046875V46.330078125H283.22321875Z" fill="#7139CB"/><path d="M300.3708203125 74.423828125C305.2751171875 74.423828125 308.817109375 72.0625 310.8454296875 67.21875L310.9059765625 74.0H318.383515625V40.154296875H310.4821484375V59.9833984375C310.4821484375 64.8876953125 307.4548046875 67.7333984375 303.2770703125 67.7333984375C299.1598828125 67.7333984375 296.6471875 65.0087890625 296.6471875 60.4677734375V40.154296875H288.77609375V61.6787109375C288.77609375 69.822265625 293.4079296875 74.423828125 300.3708203125 74.423828125Z" fill="#7139CB"/><path d="M325.99498437499994 74.0H333.86607812499994V54.080078125C333.86607812499994 49.78125 337.10533593749994 46.75390625 341.49498437499994 46.75390625C342.88756249999994 46.75390625 344.55260156249994 46.9658203125 345.21861718749994 47.1474609375V39.912109375C344.46178124999994 39.791015625 343.22057031249994 39.7001953125 342.34264062499994 39.7001953125C338.46764062499994 39.7001953125 335.19810937499994 41.91015625 333.98717187499994 45.8154296875H333.62389062499994V40.154296875H325.99498437499994Z" fill="#7139CB"/><path d="M367.7246171874999 74.60546875C375.7470781249999 74.60546875 381.7109453124999 68.248046875 381.7109453124999 57.1376953125C381.7109453124999 45.8759765625 375.5654374999999 39.73046875 367.7548906249999 39.73046875C361.7304765624999 39.73046875 359.2480546874999 43.36328125 358.0673906249999 45.7548828125H357.7343828124999V28.892578125H349.8632890624999V74.0H357.5527421874999V68.6416015625H358.0673906249999C359.2783281249999 71.09375 361.9121171874999 74.60546875 367.7246171874999 74.60546875ZM365.6054765624999 68.126953125C360.4589921874999 68.126953125 357.5830156249999 63.70703125 357.5830156249999 57.107421875C357.5830156249999 50.5078125 360.3984453124999 46.208984375 365.6054765624999 46.208984375C370.9336015624999 46.208984375 373.6279374999999 50.810546875 373.6279374999999 57.107421875C373.6279374999999 63.46484375 370.8730546874999 68.126953125 365.6054765624999 68.126953125Z" fill="#7139CB"/>'),
    'logo_dark': ('0 0 390 108', '<g><path d="M68.43 48.92L78.91 67.07Q82.91 74.00 74.91 74.00L65.63 74.00" fill="none" stroke="#A37BFE" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="52.63,74.00 66.63,63.64 66.63,84.36" fill="#A37BFE" stroke="#A37BFE" stroke-width="1.6" stroke-linejoin="round"/><path d="M46.05 74.00L25.09 74.00Q17.09 74.00 21.09 67.07L25.73 59.04" fill="none" stroke="#A37BFE" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="32.23,47.78 34.20,65.08 16.26,54.72" fill="#A37BFE" stroke="#A37BFE" stroke-width="1.6" stroke-linejoin="round"/><path d="M35.52 42.08L46.00 23.93Q50.00 17.00 54.00 23.93L58.64 31.96" fill="none" stroke="#A37BFE" stroke-width="8.6" stroke-linecap="butt" stroke-linejoin="round"/><polygon points="65.14,43.22 49.17,36.28 67.11,25.92" fill="#A37BFE" stroke="#A37BFE" stroke-width="1.6" stroke-linejoin="round"/><rect x="38.50" y="48.25" width="23" height="15.5" rx="4.34" fill="#FFFFFF"/><rect x="42.18" y="54.45" width="10.58" height="3.10" rx="1.55" fill="#17122B"/><circle cx="56.90" cy="56.00" r="2.32" fill="#37BB62"/></g><path d="M134.9833984375 74.6962890625C143.5205078125 74.6962890625 149.5146484375 70.2763671875 149.5146484375 63.8583984375C149.5146484375 59.10546875 146.45703125 56.078125 140.0390625 54.7158203125L134.287109375 53.5048828125C130.8359375 52.748046875 129.4736328125 51.5068359375 129.4736328125 49.5390625C129.4736328125 47.23828125 132.0166015625 45.603515625 135.3466796875 45.603515625C138.9794921875 45.603515625 140.9169921875 47.48046875 141.7041015625 49.841796875L148.7275390625 48.44921875C147.365234375 43.2119140625 142.9150390625 39.73046875 135.255859375 39.73046875C127.142578125 39.73046875 121.6025390625 43.7568359375 121.6025390625 50.0234375C121.6025390625 54.98828125 124.720703125 58.1064453125 131.1083984375 59.4990234375L136.890625 60.740234375C139.978515625 61.40625 141.4619140625 62.6474609375 141.4619140625 64.615234375C141.4619140625 66.916015625 139.009765625 68.671875 135.134765625 68.671875C131.2900390625 68.671875 128.9287109375 67.0673828125 128.0205078125 63.70703125L120.6640625 65.099609375C121.8447265625 71.2451171875 127.203125 74.6962890625 134.9833984375 74.6962890625Z" fill="#F4F1FF"/><path d="M155.15834375 86.0791015625C156.2179140625 86.5029296875 158.1856875 86.896484375 160.425921875 86.896484375C166.54115625 86.896484375 170.0226015625 83.7783203125 172.0811953125 78.359375L186.6124453125 40.154296875H178.28725L171.8995546875 58.560546875C170.90053125 61.466796875 170.1436953125 64.4033203125 169.386859375 67.3701171875C168.72084375 64.4033203125 168.054828125 61.4970703125 167.086078125 58.560546875L160.910296875 40.154296875H152.49428125L165.269671875 74.12109375L164.785296875 76.2705078125C163.9679140625 80.1455078125 161.4854921875 81.1142578125 157.9435 80.17578125L157.0050234375 79.93359375Z" fill="#F4F1FF"/><path d="M199.0979375 54.1708984375C199.0979375 49.2666015625 202.12528125 46.4208984375 206.3332890625 46.4208984375C210.4504765625 46.4208984375 212.9328984375 49.1455078125 212.9328984375 53.6865234375V74.0H220.8039921875V52.4755859375C220.8039921875 44.33203125 216.2024296875 39.73046875 209.2395390625 39.73046875C204.2746953125 39.73046875 200.7629765625 42.15234375 198.7649296875 46.9052734375L198.674109375 40.154296875H191.22684375V74.0H199.0979375Z" fill="#F4F1FF"/><path d="M243.098078125 74.6962890625C252.96721875 74.6962890625 259.29436718750003 67.6728515625 259.29436718750003 57.2587890625C259.29436718750003 46.7236328125 252.96721875 39.73046875 243.098078125 39.73046875C233.2289375 39.73046875 226.9320625 46.7236328125 226.9320625 57.2587890625C226.9320625 67.6728515625 233.2289375 74.6962890625 243.098078125 74.6962890625ZM243.098078125 68.30859375C237.6185859375 68.30859375 234.9545234375 63.404296875 234.9545234375 57.228515625C234.9545234375 50.9921875 237.6185859375 46.1181640625 243.098078125 46.1181640625C248.60784375 46.1181640625 251.3021796875 50.9921875 251.3021796875 57.228515625C251.3021796875 63.404296875 248.60784375 68.30859375 243.098078125 68.30859375Z" fill="#F4F1FF"/><path d="M283.22321875 40.154296875H275.897046875V37.5205078125C275.897046875 34.8564453125 277.0474375 33.3427734375 280.014234375 33.3427734375C281.3159921875 33.3427734375 282.3452890625 33.615234375 283.0113046875 33.8271484375L284.615796875 27.7119140625C283.5865 27.3486328125 281.346265625 26.7734375 278.561109375 26.7734375C272.748609375 26.7734375 268.0562265625 30.1640625 268.0562265625 36.9150390625V40.154296875H261.850171875V46.330078125H268.0562265625V74.0H275.897046875V46.330078125H283.22321875Z" fill="#A37BFE"/><path d="M300.3708203125 74.423828125C305.2751171875 74.423828125 308.817109375 72.0625 310.8454296875 67.21875L310.9059765625 74.0H318.383515625V40.154296875H310.4821484375V59.9833984375C310.4821484375 64.8876953125 307.4548046875 67.7333984375 303.2770703125 67.7333984375C299.1598828125 67.7333984375 296.6471875 65.0087890625 296.6471875 60.4677734375V40.154296875H288.77609375V61.6787109375C288.77609375 69.822265625 293.4079296875 74.423828125 300.3708203125 74.423828125Z" fill="#A37BFE"/><path d="M325.99498437499994 74.0H333.86607812499994V54.080078125C333.86607812499994 49.78125 337.10533593749994 46.75390625 341.49498437499994 46.75390625C342.88756249999994 46.75390625 344.55260156249994 46.9658203125 345.21861718749994 47.1474609375V39.912109375C344.46178124999994 39.791015625 343.22057031249994 39.7001953125 342.34264062499994 39.7001953125C338.46764062499994 39.7001953125 335.19810937499994 41.91015625 333.98717187499994 45.8154296875H333.62389062499994V40.154296875H325.99498437499994Z" fill="#A37BFE"/><path d="M367.7246171874999 74.60546875C375.7470781249999 74.60546875 381.7109453124999 68.248046875 381.7109453124999 57.1376953125C381.7109453124999 45.8759765625 375.5654374999999 39.73046875 367.7548906249999 39.73046875C361.7304765624999 39.73046875 359.2480546874999 43.36328125 358.0673906249999 45.7548828125H357.7343828124999V28.892578125H349.8632890624999V74.0H357.5527421874999V68.6416015625H358.0673906249999C359.2783281249999 71.09375 361.9121171874999 74.60546875 367.7246171874999 74.60546875ZM365.6054765624999 68.126953125C360.4589921874999 68.126953125 357.5830156249999 63.70703125 357.5830156249999 57.107421875C357.5830156249999 50.5078125 360.3984453124999 46.208984375 365.6054765624999 46.208984375C370.9336015624999 46.208984375 373.6279374999999 50.810546875 373.6279374999999 57.107421875C373.6279374999999 63.46484375 370.8730546874999 68.126953125 365.6054765624999 68.126953125Z" fill="#A37BFE"/>'),
}
_FAVICON_B64 = 'PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMDI0IDEwMjQiIHJvbGU9ImltZyIgYXJpYS1sYWJlbD0ic3lub2Z1cmIgYXBwIGljb24iPjx0aXRsZT5zeW5vZnVyYiBhcHAgaWNvbjwvdGl0bGU+PHJlY3QgeD0iMC4wIiB5PSIwLjAiIHdpZHRoPSIxMDI0LjAiIGhlaWdodD0iMTAyNC4wIiByeD0iMjI5LjEiIGZpbGw9IiM1RTI0QkMiLz48ZyB0cmFuc2Zvcm09InRyYW5zbGF0ZSg1MS4yIDQxLjApIHNjYWxlKDkuMjE2MCkiPjxwYXRoIGQ9Ik02OS40MCA0OC42MEw4MC42NCA2OC4wN1E4NC42NCA3NS4wMCA3Ni42NCA3NS4wMEw2OC43NyA3NS4wMCIgZmlsbD0ibm9uZSIgc3Ryb2tlPSIjRkZGRkZGIiBzdHJva2Utd2lkdGg9IjEzLjUiIHN0cm9rZS1saW5lY2FwPSJidXR0IiBzdHJva2UtbGluZWpvaW49InJvdW5kIi8+PHBvbHlnb24gcG9pbnRzPSI1Mi43Nyw3NS4wMCA2OS43Nyw2Mi40MiA2OS43Nyw4Ny41OCIgZmlsbD0iI0ZGRkZGRiIgc3Ryb2tlPSIjRkZGRkZGIiBzdHJva2Utd2lkdGg9IjEuNiIgc3Ryb2tlLWxpbmVqb2luPSJyb3VuZCIvPjxwYXRoIGQ9Ik00NS44NCA3NS4wMEwyMy4zNiA3NS4wMFExNS4zNiA3NS4wMCAxOS4zNiA2OC4wN0wyMy4yOSA2MS4yNiIgZmlsbD0ibm9uZSIgc3Ryb2tlPSIjRkZGRkZGIiBzdHJva2Utd2lkdGg9IjEzLjUiIHN0cm9rZS1saW5lY2FwPSJidXR0IiBzdHJva2UtbGluZWpvaW49InJvdW5kIi8+PHBvbHlnb24gcG9pbnRzPSIzMS4yOSw0Ny40MCAzMy42OSw2OC40MSAxMS45MCw1NS44MyIgZmlsbD0iI0ZGRkZGRiIgc3Ryb2tlPSIjRkZGRkZGIiBzdHJva2Utd2lkdGg9IjEuNiIgc3Ryb2tlLWxpbmVqb2luPSJyb3VuZCIvPjxwYXRoIGQ9Ik0zNC43NiA0MS40MEw0Ni4wMCAyMS45M1E1MC4wMCAxNS4wMCA1NC4wMCAyMS45M0w1Ny45MyAyOC43NCIgZmlsbD0ibm9uZSIgc3Ryb2tlPSIjRkZGRkZGIiBzdHJva2Utd2lkdGg9IjEzLjUiIHN0cm9rZS1saW5lY2FwPSJidXR0IiBzdHJva2UtbGluZWpvaW49InJvdW5kIi8+PHBvbHlnb24gcG9pbnRzPSI2NS45Myw0Mi42MCA0Ni41NCwzNC4xNyA2OC4zMywyMS41OSIgZmlsbD0iI0ZGRkZGRiIgc3Ryb2tlPSIjRkZGRkZGIiBzdHJva2Utd2lkdGg9IjEuNiIgc3Ryb2tlLWxpbmVqb2luPSJyb3VuZCIvPjxyZWN0IHg9IjQwIiB5PSI0OS41IiB3aWR0aD0iMjAiIGhlaWdodD0iMTMiIHJ4PSI0IiBmaWxsPSIjRkZGRkZGIi8+PGNpcmNsZSBjeD0iNTUiIGN5PSI1NiIgcj0iMi40IiBmaWxsPSIjN0RGMEE4Ii8+PC9nPjwvc3ZnPgo='
_LICENSES = {
    'Inter': 'Copyright 2016-2023 The Inter Project Authors (https://github.com/rsms/inter)\nInter is licensed under the SIL Open Font License, Version 1.1.\n\nPREAMBLE\nThe goals of the Open Font License (OFL) are to stimulate worldwide\ndevelopment of collaborative font projects, to support the font\ncreation efforts of academic and linguistic communities, and to\nprovide a free and open framework in which fonts may be shared and\nimproved in partnership with others.\n\nThe OFL allows the licensed fonts to be used, studied, modified and\nredistributed freely as long as they are not sold by themselves. The\nfonts, including any derivative works, can be bundled, embedded,\nredistributed and/or sold with any software provided that any\nreserved names are not used by derivative works. The fonts and\nderivatives, however, cannot be released under any other type of\nlicense. The requirement for fonts to remain under this license\ndoes not apply to any document created using the fonts or their\nderivatives.\n\nDEFINITIONS\nFont Software refers to the set of files released by the\nCopyright Holder(s) under this license and clearly marked as such.\nThis may include source files, build scripts and documentation.\n\nReserved Font Name refers to any names specified as such after\nthe copyright statement(s).\n\nOriginal Version refers to the collection of Font Software\ncomponents as distributed by the Copyright Holder(s).\n\nModified Version refers to any derivative made by adding to,\ndeleting, or substituting in part or in whole any of the components\nof the Original Version, by changing formats or by porting the Font\nSoftware to a new environment.\n\nAuthor refers to any designer, engineer, programmer, technical\nwriter or other person who contributed to the Font Software.\n\nPERMISSION & CONDITIONS\nPermission is hereby granted, free of charge, to any person\nobtaining a copy of the Font Software, to use, study, copy, merge,\nembed, modify, redistribute, and sell modified and unmodified copies\nof the Font Software, subject to the following conditions:\n\n1) Neither the Font Software nor any of its individual components,\n   in Original or Modified Versions, may be sold by itself.\n\n2) Original or Modified Versions of the Font Software may be bundled,\n   redistributed and/or sold with any software, provided that each\n   copy contains the above copyright notice and this license. These\n   can be included either as stand-alone text files, human-readable\n   headers or in the appropriate machine-readable metadata fields\n   within text or binary files as long as those fields can be easily\n   viewed by the user.\n\n3) No Modified Version of the Font Software may use the Reserved Font\n   Name(s) unless explicit written permission is granted by the\n   corresponding Copyright Holder. This restriction only applies to\n   the primary font name as presented to the users.\n\n4) The name(s) of the Copyright Holder(s) or the Author(s) of the\n   Font Software shall not be used to promote, endorse or advertise\n   any Modified Version, except to acknowledge the contribution(s)\n   of the Copyright Holder(s) and the Author(s) or with their\n   explicit written permission.\n\n5) The Font Software, modified or unmodified, in part or in whole,\n   must be distributed entirely under this license, and must not be\n   distributed under any other license. The requirement for fonts to\n   remain under this license does not apply to any document created\n   using the Font Software.\n\nTERMINATION\nThis license becomes null and void if any of the above conditions\nare not met.\n\nDISCLAIMER\nTHE FONT SOFTWARE IS PROVIDED AS IS, WITHOUT WARRANTY OF ANY KIND,\nEXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF\nMERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT\nOF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL\nTHE COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\nLIABILITY, INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR\nCONSEQUENTIAL DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR\nOTHERWISE, ARISING FROM, OUT OF THE USE OR INABILITY TO USE THE FONT\nSOFTWARE OR FROM OTHER DEALINGS IN THE FONT SOFTWARE.',
    'JetBrains Mono': 'Copyright 2020 The JetBrains Mono Project Authors (https://github.com/JetBrains/JetBrainsMono)\nJetBrains Mono is licensed under the SIL Open Font License, Version 1.1 (as stated in the font files, name IDs 13 and 14).\n\nPREAMBLE\nThe goals of the Open Font License (OFL) are to stimulate worldwide\ndevelopment of collaborative font projects, to support the font\ncreation efforts of academic and linguistic communities, and to\nprovide a free and open framework in which fonts may be shared and\nimproved in partnership with others.\n\nThe OFL allows the licensed fonts to be used, studied, modified and\nredistributed freely as long as they are not sold by themselves. The\nfonts, including any derivative works, can be bundled, embedded,\nredistributed and/or sold with any software provided that any\nreserved names are not used by derivative works. The fonts and\nderivatives, however, cannot be released under any other type of\nlicense. The requirement for fonts to remain under this license\ndoes not apply to any document created using the fonts or their\nderivatives.\n\nDEFINITIONS\nFont Software refers to the set of files released by the\nCopyright Holder(s) under this license and clearly marked as such.\nThis may include source files, build scripts and documentation.\n\nReserved Font Name refers to any names specified as such after\nthe copyright statement(s).\n\nOriginal Version refers to the collection of Font Software\ncomponents as distributed by the Copyright Holder(s).\n\nModified Version refers to any derivative made by adding to,\ndeleting, or substituting in part or in whole any of the components\nof the Original Version, by changing formats or by porting the Font\nSoftware to a new environment.\n\nAuthor refers to any designer, engineer, programmer, technical\nwriter or other person who contributed to the Font Software.\n\nPERMISSION & CONDITIONS\nPermission is hereby granted, free of charge, to any person\nobtaining a copy of the Font Software, to use, study, copy, merge,\nembed, modify, redistribute, and sell modified and unmodified copies\nof the Font Software, subject to the following conditions:\n\n1) Neither the Font Software nor any of its individual components,\n   in Original or Modified Versions, may be sold by itself.\n\n2) Original or Modified Versions of the Font Software may be bundled,\n   redistributed and/or sold with any software, provided that each\n   copy contains the above copyright notice and this license. These\n   can be included either as stand-alone text files, human-readable\n   headers or in the appropriate machine-readable metadata fields\n   within text or binary files as long as those fields can be easily\n   viewed by the user.\n\n3) No Modified Version of the Font Software may use the Reserved Font\n   Name(s) unless explicit written permission is granted by the\n   corresponding Copyright Holder. This restriction only applies to\n   the primary font name as presented to the users.\n\n4) The name(s) of the Copyright Holder(s) or the Author(s) of the\n   Font Software shall not be used to promote, endorse or advertise\n   any Modified Version, except to acknowledge the contribution(s)\n   of the Copyright Holder(s) and the Author(s) or with their\n   explicit written permission.\n\n5) The Font Software, modified or unmodified, in part or in whole,\n   must be distributed entirely under this license, and must not be\n   distributed under any other license. The requirement for fonts to\n   remain under this license does not apply to any document created\n   using the Font Software.\n\nTERMINATION\nThis license becomes null and void if any of the above conditions\nare not met.\n\nDISCLAIMER\nTHE FONT SOFTWARE IS PROVIDED AS IS, WITHOUT WARRANTY OF ANY KIND,\nEXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF\nMERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT\nOF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL\nTHE COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\nLIABILITY, INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR\nCONSEQUENTIAL DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR\nOTHERWISE, ARISING FROM, OUT OF THE USE OR INABILITY TO USE THE FONT\nSOFTWARE OR FROM OTHER DEALINGS IN THE FONT SOFTWARE.',
    'Lucide': 'ISC License\n\nCopyright (c) 2026 Lucide Icons and Contributors\n\nPermission to use, copy, modify, and/or distribute this software for any\npurpose with or without fee is hereby granted, provided that the above\ncopyright notice and this permission notice appear in all copies.\n\nTHE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES\nWITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF\nMERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR\nANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES\nWHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN\nACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF\nOR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.',
}
# END GENERATED ASSETS


def _build_sprite() -> str:
    sym = lambda i, vb, inner: '<symbol id="i-%s" viewBox="%s">%s</symbol>' % (i, vb, inner)
    parts = [sym(n, "0 0 24 24", body) for n, body in _LUCIDE.items()]
    parts += [sym("brand-" + k, vb, inner) for k, (vb, inner) in _BRAND.items()]
    return "".join(parts)


_ICON_SPRITE = _build_sprite()


def _font_css() -> str:
    return "".join("@font-face{font-family:\"%s\";font-style:normal;font-weight:%s;font-display:swap;src:url(data:font/woff2;base64,%s) format('woff2')}" % (fam, w, b64)
                   for fam, w, b64 in _FONT_FACES)

# ---- copy module: every visible string of the RAID and Help pages (short / tip / help, with word budgets enforced by tests) ----
# kind: title (<=4 words), label (<=4), button (<=3), tip (<=12, only shown in tooltips), prose (counted against the 60-word page budget), safety (<=12, may stay visible)
def _c(short: str, kind: str = "label", tip: str | None = None) -> dict:
    d = {"short": short, "kind": kind}
    if tip:
        d["tip"] = tip
    return d


COPY: dict[str, dict] = {
    "raid.title": _c("RAID", "title"),
    "raid.create": _c("Create array", "button", "Build a new array from free disks."),
    "raid.arrays": _c("Arrays"), "raid.hybrid": _c("Hybrid volumes"), "raid.empty": _c("No arrays yet.", "prose"),
    "raid.members": _c("Members"), "raid.size": _c("Size"), "raid.usedby": _c("Used by"), "raid.level": _c("Level"), "raid.usable": _c("Usable"),
    "raid.survives": _c("Survives {n}"), "raid.none": _c("No protection", "label", "Losing any disk loses the data."),
    "raid.imported": _c("Imported", "label", "Made by another system; changes need extra confirmation."),
    "raid.actions": _c("Actions", "button"), "raid.monthly": _c("Monthly check", "label", "Checks every array once a month, in the background."),
    "raid.mismatch": _c("{n} mismatches", "label", "The last check found blocks that differ between members."), "raid.bitmap": _c("Bitmap", "label", "A write-intent bitmap shortens rebuilds after a crash."),
    "state.clean": _c("Clean", "label", "All members are in sync."), "state.degraded": _c("Degraded", "label", "A member is missing or failed, so replace it soon."),
    "state.rebuilding": _c("Rebuilding", "label", "A disk is being filled with data again."), "state.resyncing": _c("Syncing", "label", "Members are being brought in sync."),
    "state.reshaping": _c("Reshaping", "label", "The array is changing size or level; do not power off."), "state.checking": _c("Checking", "label", "Reading all blocks to compare members."),
    "state.repairing": _c("Repairing", "label", "Rewriting blocks that differ."), "state.inactive": _c("Inactive", "label", "Not started: assemble it or check the disks."),
    "ms.active": _c("Active"), "ms.spare": _c("Spare"), "ms.faulty": _c("Failed"), "ms.rebuilding": _c("Rebuilding"), "ms.removed": _c("Missing"), "ms.replacement": _c("Replacing"),
    "act.add_spare": _c("Add spare", "button"), "act.replace": _c("Replace disk", "button"), "act.grow": _c("Grow", "button"), "act.level": _c("Change level", "button"),
    "act.check": _c("Check now", "button"), "act.repair": _c("Repair", "button"), "act.stoptask": _c("Stop task", "button"), "act.tune": _c("Tune", "button"), "act.stop": _c("Stop array", "button"),
    "raid.disk": _c("Disk"), "raid.disks": _c("Disks to add"), "raid.old": _c("Disk to replace"), "raid.new": _c("New disk"), "raid.target": _c("New level"), "raid.allspace": _c("Use all space"),
    "raid.nofree": _c("No free disks.", "prose"), "raid.noredundancy": _c("RAID 0 has no redundancy.", "tip"), "raid.cantgrow": _c("This level cannot grow.", "tip"), "raid.cantlevel": _c("No conversion available.", "tip"),
    "raid.norepair": _c("Not on a degraded array.", "tip"), "raid.inuse": _c("Array is in use.", "tip"), "raid.busy": _c("Wait until it is idle.", "tip"),
    "raid.done": _c("Done."), "raid.failed": _c("A step failed."), "raid.started": _c("Check started."), "raid.type": _c("Type {name} to confirm", "safety"),
    "raid.commands": _c("Commands that will run"), "raid.later": _c("Run when finished"), "raid.ack": _c("I understand this is not mine", "safety"),
    "raid.speedmin": _c("Min speed KiB/s"), "raid.speedmax": _c("Max speed KiB/s"), "raid.name": _c("Name"), "raid.fs": _c("Filesystem"), "raid.withvol": _c("Add a volume"),
    "raid.createrun": _c("Erase and create", "button"),
    "btn.next": _c("Next", "button"), "btn.back": _c("Back", "button"), "btn.cancel": _c("Cancel", "button"), "btn.preview": _c("Preview", "button"), "btn.run": _c("Run", "button"),
    "btn.close": _c("Close", "button"), "btn.save": _c("Save", "button"),
    "wiz.disks": _c("Disks"), "wiz.level": _c("Level"), "wiz.review": _c("Review"),
    "lv.raid0": _c("RAID 0", "label", "Fastest, but any failed disk loses everything."), "lv.raid1": _c("RAID 1", "label", "Mirror: every disk holds a full copy."),
    "lv.raid5": _c("RAID 5", "label", "One disk of parity; survives one failure."), "lv.raid6": _c("RAID 6", "label", "Two disks of parity; survives two failures."),
    "lv.raid10": _c("RAID 10", "label", "Mirrored pairs, striped; needs an even disk count."),
    "lv.shr1": _c("Hybrid 1", "label", "Like Synology SHR: mixes disk sizes, survives one failure."), "lv.shr2": _c("Hybrid 2", "label", "Like Synology SHR-2: mixes disk sizes, survives two failures."),
    "shr.add": _c("Add disk", "button"), "shr.replace": _c("Replace disk", "button"), "shr.slice": _c("Slice"), "shr.slices": _c("Slices"), "shr.unused": _c("Unused"),
    "help.title": _c("Help", "title"), "help.search": _c("Search help"), "help.none": _c("No topic found.", "prose"),
}
# ---- copy for the web pages (shell, login, dashboard, storage, mounts, network, users, boot, system, about); same budgets as above ----
COPY.update({
    # shell
    "shell.brand": _c("synofurb", "title"), "shell.role": _c("Administrator"), "shell.accent": _c("Accent colour", "label", "Pick the accent colour."),
    "shell.theme": _c("Theme", "label", "Switch between light and dark."), "shell.logout": _c("Sign out", "button", "End this session."), "shell.menu": _c("Menu", "button"),
    "shell.nav": _c("Main"),
    "nav.dashboard": _c("Dashboard"), "nav.storage": _c("Storage"), "nav.raid": _c("RAID"), "nav.mounts": _c("Mounts"), "nav.network": _c("Network"), "nav.users": _c("Users"),
    "nav.boot": _c("Boot"), "nav.system": _c("System"), "nav.help": _c("Help"), "nav.about": _c("About"),
    "page.error": _c("Could not load this page.", "prose"), "table.empty": _c("Nothing to show.", "prose"), "btn.confirm": _c("Confirm", "button"),
    "help.about": _c("Help on", "tip"), "help.safe_apply": _c("Safe apply"),
    # login
    "login.title": _c("Sign in", "title"), "login.user": _c("Username", "label", "The account created during installation."), "login.password": _c("Password"),
    "login.go": _c("Sign in", "button"), "login.show": _c("Show password", "button"),
    "login.cert": _c("Certificate", "button", "Compare this fingerprint with the one your browser shows before you trust the page."),
    # common columns, actions, states
    "col.device": _c("Device"), "col.model": _c("Model"), "col.size": _c("Size"), "col.type": _c("Type"), "col.role": _c("Role"), "col.array": _c("Array"), "col.level": _c("Level"),
    "col.state": _c("State"), "col.interface": _c("Interface"), "col.address": _c("Address"), "col.volume": _c("Volume"), "col.mount": _c("Mount"), "col.used": _c("Used"),
    "col.name": _c("Name"), "col.groups": _c("Groups"), "col.keys": _c("Keys"), "col.mode": _c("Mode"), "col.health": _c("Health"), "col.temp": _c("Temp"), "col.fs": _c("Filesystem"),
    "act.delete": _c("Delete", "button"), "act.discard": _c("Discard", "button"), "act.edit": _c("Edit", "button"), "act.extend": _c("Extend", "button"),
    "act.lock": _c("Lock", "button"), "act.unlock": _c("Unlock", "button"), "act.mount": _c("Mount", "button"), "act.unmount": _c("Unmount", "button"),
    "act.restart": _c("Restart", "button"), "act.smart": _c("Check SMART", "button", "Reads each disk's health report; takes a few seconds."), "act.open_raid": _c("Open RAID", "button"),
    "state.active": _c("Active"), "state.down": _c("Down"), "state.invalid": _c("Invalid"), "state.linkup": _c("Up"), "state.locked": _c("Locked"), "state.nocable": _c("No cable"),
    "state.nopw": _c("No password"), "state.unmounted": _c("Not mounted"),
    "smart.ok": _c("OK"), "smart.failing": _c("Failing"), "smart.unknown": _c("Unknown"),
    "mc.installed": _c("Loaded"), "mc.missing": _c("Missing"), "mc.na": _c("Not needed"),
    "role.system": _c("System"), "role.array": _c("Array"), "role.boot": _c("Boot"), "role.unused": _c("Unused"),
    # dashboard
    "dash.health": _c("Health", "label", "Arrays and disks: any degraded array turns this red."), "dash.healthy": _c("Healthy"),
    "dash.storage": _c("Storage", "label", "Space used on the main data volume."), "dash.memory": _c("Memory", "label", "RAM in use, without caches that can be dropped."),
    "dash.load": _c("Load", "label", "Load average for 1, 5 and 15 minutes, then the hottest disk."), "dash.cpu": _c("CPU", "label", "Processor vendor and model."),
    "dash.kernel": _c("Kernel", "label", "The running Linux kernel."), "dash.kernel7": _c("Kernel 7+", "label", "Kernel 7 or newer, as the boot plan requires."),
    "dash.microcode": _c("Microcode", "label", "CPU microcode updates; Intel and AMD only."), "dash.mc_missing": _c("Microcode missing", "label", "Install the microcode package, then reboot."),
    "dash.network": _c("Network", "label", "Interfaces that are up."), "dash.up": _c("up"), "dash.attention": _c("Attention needed", "title"),
    "dash.disks": _c("Disks"), "dash.arrays": _c("Arrays"),
    # storage
    "st.create": _c("Create volume", "button", "Build a volume from free disks, in four steps."), "st.raid_link": _c("RAID", "button", "Arrays and Hybrid RAID have their own page."),
    "st.volumes": _c("Volumes"), "st.vg": _c("Group", "label", "Free space left in this volume group."), "st.free": _c("free"), "st.add_gib": _c("Add GiB"),
    "st.extend": _c("Extend", "tip", "Add space to this volume; the filesystem grows with it."), "st.nofree": _c("No free space", "tip", "The volume group has no free space left."),
    "st.delete": _c("Deletes the volume and everything on it.", "prose"), "st.rmvg": _c("Remove empty group"), "st.empty": _c("No volumes yet.", "prose"),
    "st.mountpoint": _c("Mount point"), "st.size_gib": _c("Size in GiB"), "st.plan": _c("Plan", "title"), "st.nowrite": _c("Preview only", "label", "Nothing is written until you confirm."),
    "dlg.extend": _c("Extend {name}", "title"), "dlg.delete": _c("Delete {name}", "title"), "dlg.create": _c("Create {name}", "title"), "dlg.unmount": _c("Unmount {name}", "title"),
    "wiz.devices": _c("Disks"), "wiz.layout": _c("Layout"), "wiz.volume": _c("Volume"),
    "lv.none": _c("No RAID", "label", "Disks are joined end to end; a failure loses what is on that disk."),
    # mounts
    "mnt.mounted": _c("Mounted", "label", "Filesystems that are mounted right now."), "mnt.imported": _c("Imported", "label", "Disks from another system, ready to mount."),
    "mnt.fstab": _c("Boot table"), "mnt.scan": _c("Scan disks", "button", "Look for new RAID arrays and volumes."), "mnt.none": _c("No imported volumes.", "prose"),
    "mnt.other": _c("other"), "mnt.ro": _c("Read-only", "label", "Safe default: nothing on the disk can change."), "mnt.boot": _c("At boot"), "mnt.rmfstab": _c("Remove boot entry"),
    # network
    "net.hostname": _c("Hostname", "label", "The name of this unit on the network."), "net.ipv4": _c("IPv4"), "net.ipv6": _c("IPv6"), "net.address": _c("Address"),
    "net.gateway": _c("Gateway"), "net.gateway6": _c("IPv6 gateway"), "net.dns": _c("DNS servers", "label", "Separate with commas; empty keeps what DHCP gives."), "net.search": _c("Search domains"),
    "net.mac": _c("MAC"), "net.dhcp": _c("DHCP"), "net.static": _c("Static"), "net.apply": _c("Apply", "button"), "net.apply_go": _c("Apply with rollback", "button"),
    "net.apply_q": _c("Apply network settings?", "title", "The old settings return in 90 seconds unless you press Keep."), "net.applied": _c("Applied."),
    "net.keep": _c("Keep", "button"), "net.kept": _c("Kept."), "net.rollback": _c("Roll back", "button"), "net.rolled": _c("Rolled back."),
    "net.cannot": _c("Cannot reach?", "button", "Do nothing: the old settings return by themselves within 90 seconds."), "net.confirm_in": _c("Confirm in"),
    "net.pending": _c("Pending", "label", "Press Keep once this page works at the new address."),
    # users
    "usr.new": _c("New user", "button", "Create a local account."), "usr.name": _c("Username"), "usr.password": _c("Password"), "usr.admin": _c("Administrator"),
    "usr.sshkey": _c("SSH key"), "usr.rmhome": _c("Delete home folder"), "usr.delete": _c("The account is removed at once.", "prose"), "usr.sudo": _c("sudo", "label", "May run commands as root."),
    # boot
    "boot.entries": _c("Boot entries"), "boot.default": _c("Default"), "boot.failsafe": _c("Fail-safe"), "boot.missing": _c("Missing"),
    "boot.none": _c("This unit does not boot from flash.", "prose"), "boot.part1": _c("Partition 1"), "boot.part2": _c("Partition 2"), "boot.flash": _c("Flash"),
    "boot.backup": _c("Backup", "label", "A copy of the flash boot files, used to restore them."), "boot.exists": _c("Saved"), "boot.nobackup": _c("None yet"),
    "boot.set_default": _c("Set as default", "button"), "boot.reboot_go": _c("Reboot into it", "button"), "boot.reboot_q": _c("Reboot into {name}?", "title"),
    "boot.default_q": _c("Make {name} the default?", "title"), "boot.return_dsm": _c("Return to DSM", "button", "Boots DSM once; the flash stays untouched."),
    # system
    "sys.ssh": _c("SSH"), "sys.enabled": _c("Enabled"), "sys.hostkeys": _c("Host keys"), "sys.consoles": _c("Serial consoles"),
    "sys.services": _c("Services", "label", "Restart one if it misbehaves."), "sys.power": _c("Power"),
    "sys.reboot": _c("Reboot", "button", "Restarts the unit; connections drop for a minute or two."), "sys.shutdown": _c("Shut down", "button", "Powers off; start it again with the button on the unit."),
    "sys.reboot_q": _c("Reboot this unit?", "title"), "sys.shutdown_q": _c("Shut down this unit?", "title"),
    # about
    "about.version": _c("Version"), "about.copy": _c("Copy diagnostics", "button", "Version, host and service state; no secrets."), "about.copied": _c("Copied."),
    "about.docs": _c("Documentation", "button"), "about.source": _c("Source code", "button"), "about.issue": _c("Report issue", "button"), "about.credits": _c("Credits"),
    "about.licence": _c("Licence", "button"), "about.synodeb": _c("synodeb", "button", "Boot-flash technique documented by synodeb (GPL-2.0); no code used."),
    "raid.need2": _c("Needs 2 disks", "tip"), "raid.need4": _c("Needs 4 disks", "tip"), "raid.needeven": _c("Needs an even number", "tip"), "api.expired": _c("Session expired.", "prose"),
    "help.back": _c("Back", "button"), "help.docs": _c("Open docs", "button"),
})
# ---- copy for the Add-ons page (same budgets: label <= 4 words, button <= 3, tip <= 12) ----
COPY.update({
    "nav.addons": _c("Add-ons"), "ad.installed": _c("Installed"), "ad.available": _c("Available"),
    "ad.install": _c("Install", "button"), "ad.retry": _c("Retry", "button", "The last install did not finish; try it again."), "ad.open": _c("Open", "button", "Open the app's web page."),
    "ad.start": _c("Start", "button"), "ad.stop": _c("Stop", "button"), "ad.restart": _c("Restart", "button"), "ad.update": _c("Update", "button", "Download newer images and restart; data is kept."),
    "ad.logs": _c("Logs", "button"), "ad.remove": _c("Remove", "button"), "ad.details": _c("Details", "button"), "ad.refresh": _c("Refresh", "button"), "ad.view": _c("View", "button"),
    "ad.help": _c("Help", "button"), "ad.lines": _c("Lines"), "ad.hide": _c("Hide", "button"), "ad.progress": _c("Progress"), "ad.rev": _c("Revision"),
    "ad.why_priv": _c("Accept the privileges first.", "safety"), "ad.why_type": _c("Type the name first.", "safety"),
    "ad.st.running": _c("Running", "label", "Containers run and the health check answers."), "ad.st.stopped": _c("Stopped", "label", "Installed, but not running."),
    "ad.st.starting": _c("Starting", "label", "Running; the health check does not answer yet."), "ad.st.installing": _c("Installing", "label", "A job is installing it."),
    "ad.st.updating": _c("Updating", "label", "A job is updating it."), "ad.st.removing": _c("Removing", "label", "A job is removing it."),
    "ad.st.unhealthy": _c("Not answering", "label", "Running, but the health check fails; see the logs."), "ad.st.error": _c("Problem", "label", "A container keeps stopping; see the logs."),
    "ad.st.needs-attention": _c("Needs attention", "label", "The last operation failed; see the details."), "ad.st.not-installed": _c("Not installed", "label"),
    "ad.priv.docker-socket": _c("Docker control", "label", "Full control of Docker here, which is the same as root."),
    "ad.priv.host-network": _c("Host network", "label", "Shares this unit's network; its ports are not isolated."),
    "ad.priv.privileged": _c("Privileged", "label", "Unrestricted access to hardware and kernel."), "ad.priv.cap": _c("Extra capability", "label", "An extra kernel capability for the container."),
    "ad.install_t": _c("Install {name}", "title"), "ad.settings": _c("Settings"), "ad.privs": _c("Privileges"), "ad.devices": _c("Devices"), "ad.storage": _c("Storage"),
    "ad.changes": _c("Changes to this unit"), "ad.accept": _c("I accept these privileges", "safety"), "ad.present": _c("Found"), "ad.absent": _c("Not found"),
    "ad.absent_warn": _c("Not found: the app may not start.", "prose"), "ad.free": _c("free"), "ad.once": _c("Used on first install only.", "tip"),
    "ad.secret_set": _c("Set; leave empty to keep it.", "tip"), "ad.needs": _c("Install {name} first.", "safety"), "ad.noweb": _c("No web page.", "prose"),
    "ad.update_q": _c("Update {name}?", "title"), "ad.update_b": _c("Newer images are downloaded and the app restarts. Your data is kept.", "prose"),
    "ad.remove_q": _c("Remove {name}?", "title"), "ad.remove_b": _c("Containers are removed. Data is kept unless you tick the box.", "prose"),
    "ad.deldata": _c("Delete data", "label", "Erases the settings, configuration and stored files for good."),
    "ad.logs_t": _c("Logs of {name}", "title"), "ad.job_t": _c("{name}: {action}", "title"), "ad.busy": _c("A job is running", "title"), "ad.done": _c("Done."),
    "ad.problems": _c("Not loaded", "title"), "ad.note": _c("After install", "label"), "ad.ports": _c("Ports"), "ad.homepage": _c("Project page", "button"), "ad.empty": _c("No add-ons.", "prose"),
    "ad.act.install": _c("installing"), "ad.act.update": _c("updating"), "ad.act.remove": _c("removing"), "ad.act.start": _c("starting"), "ad.act.stop": _c("stopping"), "ad.act.restart": _c("restarting"),
})
# keys that are visible on the default RAID screen (no dialog open): the prose among them must stay within the page budget
# Every key whose text is visible by default on a page (dialogs, popovers, tooltips and closed menus are not). Values shown from the system itself
# (host names, sizes, states of real devices) are data and not counted. The budget: shell + page <= 60 words (design/v2/UX-SPEC.md section 9).
COPY_SHELL = ["shell.brand", "shell.role", "nav.dashboard", "nav.storage", "nav.raid", "nav.mounts", "nav.addons", "nav.network", "nav.users", "nav.boot", "nav.system", "nav.help", "nav.about"]
COPY_SCREENS = {
    "login": ["login.title", "login.user", "login.password", "login.go", "login.cert", "nav.help"],
    "dashboard": ["nav.dashboard", "dash.health", "dash.storage", "dash.memory", "dash.load", "dash.cpu", "dash.kernel", "dash.microcode", "nav.network", "dash.disks", "act.smart",
                  "col.device", "col.model", "col.size", "col.type", "col.role", "dash.arrays", "col.array", "col.level", "col.state", "col.interface", "col.address"],
    "storage": ["nav.storage", "nav.raid", "st.create", "st.volumes", "col.volume", "col.mount", "col.size", "col.used", "act.extend", "act.delete"],
    "raid": ["raid.title", "raid.create", "raid.arrays", "raid.hybrid", "raid.empty", "raid.members", "raid.size", "raid.usedby", "raid.monthly", "raid.actions",
             "state.clean", "state.degraded", "ms.active", "act.add_spare", "act.replace", "act.check", "shr.add", "shr.slice"],
    "mounts": ["nav.mounts", "mnt.mounted", "mnt.imported", "col.mount", "col.device", "col.type", "col.used", "mnt.fstab", "act.unmount"],
    "addons": ["nav.addons", "ad.installed", "ad.available", "col.name", "col.state", "ad.install", "ad.open", "ad.retry", "ad.needs"],
    "network": ["nav.network", "net.ipv4", "net.mac", "act.edit"],
    "users": ["nav.users", "usr.new", "col.name", "col.groups", "col.keys", "col.state"],
    "boot": ["nav.boot", "boot.return_dsm", "boot.entries", "boot.default", "boot.failsafe", "boot.missing", "boot.set_default", "boot.reboot_go", "boot.flash", "boot.part1", "boot.part2", "boot.backup"],
    "system": ["nav.system", "sys.ssh", "sys.enabled", "sys.hostkeys", "sys.consoles", "col.device", "col.state", "sys.services", "act.restart", "sys.power", "sys.reboot", "sys.shutdown"],
    "help": ["help.title", "help.search", "help.none", "help.docs"],
    "about": ["nav.about", "about.copy", "about.docs", "about.source", "about.issue", "about.credits"],
}
HELP_TOPICS: list[dict] = [
    {"id": "raid-levels", "title": "RAID levels", "keywords": "raid0 raid1 raid5 raid6 raid10 mirror parity stripe",
     "body": "RAID joins several disks into one array. The level decides capacity and how many failures it survives.\n\nRAID 0 stripes data: fast, no protection. RAID 1 mirrors: usable space of one disk, survives all but one failure. RAID 5 keeps one disk of parity: survives one failure. RAID 6 keeps two. RAID 10 mirrors pairs and stripes them.\n\nRAID is not a backup. Deleted files and bad writes are copied to every member."},
    {"id": "shr", "title": "Hybrid RAID (SHR)", "keywords": "shr synology hybrid slices mixed sizes lvm",
     "body": "Synology's SHR is ordinary mdadm plus LVM. The disk space is cut into slices at the sizes of your disks. Each slice that exists on several disks becomes one array: a mirror for two disks, RAID 5 for three or more (Hybrid 2 uses RAID 6 from four).\n\nAll arrays are joined into one volume group and one volume. A slice that exists on a single disk cannot be protected and stays unused.\n\nA volume built here is SHR-style and managed by Debian. DSM will not adopt it."},
    {"id": "scrub", "title": "Checks (scrub)", "keywords": "check repair scrub mismatch monthly",
     "body": "A check reads every block and compares the members. It finds silent errors early. The array stays usable but slower.\n\nMismatches are only counted. Repair rewrites differing blocks from the first member, so use it after a check found problems and the disks are healthy.\n\nThe monthly check starts a check on every healthy array once a month. Stop a running check at any time."},
    {"id": "replace", "title": "Replace and rebuild", "keywords": "replace rebuild spare fail remove disk resync recovery",
     "body": "Replace a failed disk: the failed member is removed and the new disk is added. The array rebuilds from the others and is exposed until it finishes, so keep the other disks healthy.\n\nReplace a healthy disk: the new disk is added as a spare and the old one is copied onto it while redundancy is kept. When the copy ends, remove the old disk.\n\nA spare takes over automatically when a member fails."},
    {"id": "grow", "title": "Grow and convert", "keywords": "grow reshape level convert size pvresize lvextend",
     "body": "Adding a disk to RAID 5 or 6 reshapes the array. It runs for hours and the array stays usable. When it ends, extend the volume: resize the physical volume, extend the logical volume, then grow the filesystem.\n\nSupported conversions: RAID 1 to 5, RAID 5 to 6, RAID 6 to 5, RAID 0 to 5. Other changes mean creating a new array and copying the data.\n\nNever power off during a reshape."},
    {"id": "foreign", "title": "Imported arrays", "keywords": "foreign dsm synology imported assemble read-only",
     "body": "Arrays made by DSM or another system are shown as Imported. You can assemble them, check them and mount their volumes read-only.\n\nChanges can make DSM unable to use them. They need the extra confirmation 'I understand this is not mine' and the array name typed in full."},
    {"id": "sign-in", "title": "Sign in", "keywords": "login password session account lockout",
     "body": "Sign in with the account created during installation. After several wrong passwords the sign-in pauses for a short time, per address and per account.\n\nA session ends when you sign out or after 30 minutes without activity.\n\nIf you lost the password, log in on the serial console as an administrator and run passwd for the account."},
    {"id": "certificate", "title": "Certificate", "keywords": "tls https fingerprint self-signed warning browser",
     "body": "The unit makes its own certificate, so the browser warns that it does not know the issuer. That is expected on a private network.\n\nThe Certificate button on the sign-in page shows the SHA-256 fingerprint. Compare it with the fingerprint in the browser warning once; if they match, it is safe to continue."},
    {"id": "storage", "title": "Storage", "keywords": "volume lvm extend delete xfs ext4 create wizard",
     "body": "A volume is a logical volume on LVM, formatted and mounted under /mnt. Create one from free disks in four steps: disks, layout, volume, review.\n\nThe review shows every command before it runs. Extend adds space and grows the filesystem. Delete needs the volume name typed."},
    {"id": "imports", "title": "Imports and mounts", "keywords": "mount import ntfs read-only fstab scan dsm",
     "body": "Mounted lists what is mounted now. Imported lists disks and volumes made elsewhere, such as DSM volumes or NTFS disks.\n\nImports mount read-only by default, so nothing on them can change. Untick Read-only only when you need to write. Mounts live under /mnt; At boot adds a boot-table entry."},
    {"id": "recovery", "title": "Recovery", "keywords": "recover lockout serial console rollback fail-safe dsm restore",
     "body": "Network changes roll back by themselves if you do not press Keep. On units with a boot flash, Return to DSM always works because DSM stays untouched.\n\nIf the web page is unreachable, use the serial console: log in, then check systemctl status synofurb-mgmt and journalctl -u synofurb-mgmt."},
    {"id": "report", "title": "Report a problem", "keywords": "bug issue diagnostics support copy version",
     "body": "Open About and press Copy diagnostics. It copies the version, host name, operating system, kernel and service state. It contains no passwords, keys or certificates.\n\nPaste it into the issue together with what you did and what you expected."},
    {"id": "safe-apply", "title": "Safe network apply", "keywords": "network rollback confirm address",
     "body": "Network changes are applied for 90 seconds. If you can still reach this page at the new address, press Keep. Otherwise the previous settings return by themselves, so a wrong address never locks you out."},
    {"id": "boot", "title": "Boot manager", "keywords": "boot dsm debian fail-safe",
     "body": "On units that boot from an internal flash, you choose what starts next. DSM stays on the flash untouched, so Return to DSM always works. Setting another entry only changes the default."},
    {"id": "addons", "title": "Add-ons", "keywords": "docker portainer container frigate plex app install plugin compose image",
     "body": "Add-ons are apps that run in Docker on this unit. Docker comes first. Portainer then manages the containers in a web page.\n\nBefore an install, the dialog lists the privileges, devices and folders the app asks for. Large data such as recordings and media stays on the redundant data volume. An app that can reach the Docker socket controls everything on this unit, so install only what you trust.\n\nRemove keeps your data. Tick Delete data to erase it for good; you then type the name. Images come from the Internet, so installs and updates need a connection. Secrets such as camera addresses are stored privately and never shown again."},
]


# ---- Updates page (management plane self-update) ----
COPY.update({
    "nav.updates": _c("Updates"),
    "upd.installed": _c("Installed"), "upd.available": _c("Available"), "upd.channel": _c("Channel"), "upd.ref": _c("Branch or tag"),
    "upd.stable": _c("Stable", "button", "The newest release of the management plane."),
    "upd.main": _c("Main", "button", "The source folder of a branch or tag; it is for testing and never signed."),
    "upd.check": _c("Check", "button", "Ask GitHub for the newest version."), "upd.now": _c("Update now", "button"),
    "upd.rollback": _c("Roll back", "button", "Return to the version that was installed before."), "upd.notes": _c("Release notes", "button"),
    "upd.history": _c("History"), "upd.when": _c("When"), "upd.from": _c("From"), "upd.to": _c("To"), "upd.source": _c("Source"), "upd.result": _c("Result"),
    "upd.new": _c("New", "label", "A newer version is available."), "upd.uptodate": _c("Up to date"),
    "upd.older": _c("Older", "label", "GitHub offers an older version than the installed one."),
    "upd.same": _c("Same version", "label", "The source may still differ from what is installed."),
    "upd.unsigned": _c("Not signed", "label", "Only the SHA-256 is checked, so a tampered release would not be noticed."),
    "upd.signed": _c("Signed", "label", "The signature of this release was verified."),
    "upd.consent": _c("I understand and want to update", "safety"),
    "upd.why_consent": _c("Tick the box first.", "safety"), "upd.why_typed": _c("Type the commit first.", "safety"),
    "upd.why_current": _c("Already up to date.", "safety"), "upd.why_reload": _c("Reload the page first.", "safety"), "upd.why_older": _c("Only an older version is offered.", "safety"),
    "upd.src.app": _c("Desktop app"), "upd.src.manual": _c("Command line"), "upd.src.github-stable": _c("GitHub release"), "upd.src.github-main": _c("GitHub source"),
    "upd.src.rollback": _c("Roll back"),
    "upd.s.ok": _c("Updated"), "upd.s.refused": _c("Refused"), "upd.s.rolled_back": _c("Rolled back"), "upd.s.rollback_failed": _c("Rollback failed"),
    "upd.s.failed": _c("Failed"), "upd.s.running": _c("Running"), "upd.s.queued": _c("Running"), "upd.s.current": _c("Up to date"),
    "upd.step.lock": _c("Lock"), "upd.step.check": _c("Check"), "upd.step.download": _c("Download"), "upd.step.consent": _c("Consent"), "upd.step.verify": _c("Verify"),
    "upd.step.stage": _c("Stage"), "upd.step.swap": _c("Swap"), "upd.step.restart": _c("Restart"), "upd.step.health": _c("Health"), "upd.step.rollback": _c("Roll back"),
    "upd.step.result": _c("Done"),
    "upd.confirm_q": _c("Update to {v}?", "title"), "upd.rb_q": _c("Roll back to {v}?", "title"),
    "upd.restart_note": _c("The web UI restarts briefly and goes back if the update fails.", "prose"),
    "upd.restarting": _c("Restarting. This page reconnects by itself.", "prose"), "upd.signin": _c("Sign in again", "button"), "upd.reload": _c("Reload", "button"),
    "upd.done": _c("Updated to {v}.", "prose"), "upd.rolled": _c("Rolled back to {v}.", "prose"), "upd.refused": _c("Update refused.", "prose"),
    "upd.failed": _c("Update failed.", "prose"), "upd.rbfail": _c("Rollback failed. Check the unit.", "prose"), "upd.busy": _c("Another job is running.", "prose"),
    "upd.nocheck": _c("Check first.", "safety"), "upd.none": _c("No updates yet.", "prose"), "upd.unchecked": _c("Not checked", "label"),
    "upd.plane": _c("Management plane"), "upd.force": _c("Update anyway", "button"), "upd.force_note": _c("The unit reports a problem.", "prose"),
})
COPY_SHELL.append("nav.updates")
COPY_SCREENS["updates"] = ["nav.updates", "upd.plane", "upd.installed", "upd.available", "upd.channel", "upd.stable", "upd.main", "upd.check", "upd.now", "upd.rollback", "upd.history",
                           "upd.when", "upd.from", "upd.to", "upd.source", "upd.result", "upd.nocheck"]
HELP_TOPICS.append(
    {"id": "updates", "title": "Updates", "keywords": "update upgrade version github release channel rollback signed unsigned sha-256 app push",
     "body": "Stable installs the newest release of the management plane from GitHub. Main installs the source folder of a branch or tag: it is for testing and is never signed.\n\n"
             "Releases are not signed yet. synofurb checks the SHA-256 listed in the same release, which catches a damaged download but not a tampered release, so you confirm once per update.\n\n"
             "The web UI restarts for a few seconds. If the new version does not answer within 30 seconds, the previous one comes back by itself; Roll back returns to it later.\n\n"
             "The desktop app can push an update without Internet; it appears in the history as Desktop app. Checks go to GitHub only, over https, with no account or token."})


_UI_TEMPLATE = r"""<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>NAS management</title>
<link rel="icon" type="image/svg+xml" href="data:image/svg+xml;base64,__FAVICON__">
<style nonce="__NONCE__">
/*__FONTS__*/
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
dialog.wide{width:min(840px,calc(100vw - 24px))}
dialog::backdrop{background:rgb(0 0 0/.6)}
.dh{display:flex;gap:12px;align-items:flex-start;padding:20px 20px 0}.dh h3{font-size:16px;line-height:21px;font-weight:600}.dh p{margin:2px 0 0;color:var(--fg-2);font-size:13px}
.dh>.ic{width:22px;height:22px;margin-top:1px;color:var(--accent-fg)}.dh.bad>.ic{color:var(--err-solid)}
.db{padding:16px 20px 4px;overflow:auto;min-height:0}
dialog[open]{display:flex;flex-direction:column;max-height:calc(100vh - 32px)}.dh,.df{flex:none}
.df{display:flex;gap:8px;justify-content:flex-end;padding:14px 20px;background:var(--surface-2);border-top:1px solid var(--border);margin-top:12px;flex-wrap:wrap}
div.err{color:var(--err-fg);display:flex;gap:6px;margin:0 0 10px;font-size:13px;font-weight:600}
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
/* updates page */
.us{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:6px}
.us li{display:flex;gap:8px;align-items:center;color:var(--fg-2)}.us li.run{color:var(--fg);font-weight:600}.us li.ok{color:var(--ok-fg)}.us li.fail{color:var(--err-fg);font-weight:600}
.us li small{margin-left:auto;font-weight:400;color:var(--fg-2);overflow-wrap:anywhere;text-align:right}
.upd-ref{max-width:220px}
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
 .ph h1{font-size:22px}.ph>.row{width:100%}.ph .row>.btn:not(.icon-btn){flex:1}
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
 .hres{align-items:flex-end;text-align:right}
 table.st tr.attn>td:first-child,table.st tr.bad>td:first-child{border-left:0}
 table.st tr.attn{border-left:3px solid var(--warn-solid)}table.st tr.bad{border-left:3px solid var(--err-solid)}
 table.st tbody tr:hover td{background:none}
}

/* RAID, menus, tooltips, help */
.sec{font-size:14px;line-height:20px;font-weight:600;margin:20px 0 8px;display:flex;gap:8px;align-items:center}
.arr.bad{border-left:4px solid var(--err-solid)}
.kvrow{display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin-bottom:10px;font-size:13px}
.chips{display:flex;gap:8px;flex-wrap:wrap;margin:10px 0}
.mchip{display:inline-flex;gap:6px;align-items:center;height:28px;padding:0 10px;border:1px solid var(--border-strong);border-radius:6px;background:var(--surface);font-size:12px}
.mchip b{font-weight:600;color:var(--fg-2);font-size:12px}
.mchip.faulty{border:2px solid var(--err-solid);color:var(--err-fg);background:var(--err-bg)}
.mchip.spare,.mchip.removed{border-style:dashed}.mchip.rebuilding,.mchip.replacement{border-color:var(--warn-solid);background:var(--warn-bg);color:var(--warn-fg)}
.pr.big{margin:8px 0}.pr.big span{font-size:12px}
.menuwrap{position:relative}
.menu{position:absolute;right:0;top:calc(100% + 4px);z-index:30;min-width:200px;background:var(--surface);border:1px solid var(--border-strong);border-radius:8px;box-shadow:var(--e2);padding:4px;display:flex;flex-direction:column}
.mi{display:flex;gap:10px;align-items:center;width:100%;text-align:left;min-height:36px;padding:0 10px;border:0;border-radius:6px;background:none;font:400 13px var(--font-ui);color:var(--fg)}
.mi:hover,.mi:focus-visible{background:var(--surface-2)}.mi:active{background:var(--surface-3)}.mi.danger{color:var(--err-fg)}
.mi[aria-disabled=true]{color:var(--fg-2);cursor:not-allowed;text-decoration:line-through}
#tip{position:fixed;z-index:90;max-width:260px;background:var(--fg);color:var(--canvas);padding:6px 10px;border-radius:6px;font-size:12px;line-height:16px;pointer-events:none}
.lvls{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:8px}
.opt.lvl{margin:0}.lvl .row{margin-top:6px}
.shrt{margin-top:12px}
.sw input{border-radius:999px;width:36px}
.f{margin-bottom:14px}.f .lab{display:block;font-size:13px;font-weight:600;margin-bottom:4px}
.helpcols{grid-template-columns:260px minmax(0,1fr)}
.toc{list-style:none;margin:0;padding:0;display:grid;grid-template-columns:repeat(auto-fill,minmax(220px,1fr));gap:8px}
.tocb{width:100%;text-align:left;border:1px solid var(--border);background:var(--surface);border-radius:8px;min-height:44px;padding:0 14px;font:600 13px var(--font-ui);color:var(--fg)}
.tocb:hover{background:var(--surface-2)}.tocb:active{background:var(--surface-3)}
.helpsearch{max-width:420px;margin:0 0 16px}
.helpbody p,.card .cb p{max-width:68ch}
input[type=search]{width:100%}
.btn[aria-disabled=true],.btn[aria-disabled=true]:is(:hover,:active,.primary,.danger){background:var(--surface-2);color:var(--fg-2);border:1px dashed var(--border-strong);cursor:not-allowed;font-weight:600}
/* a control that is off keeps its reason as visible text beside it (a disabled button swallows hover, focus and touch) */
.why{display:flex;gap:6px;align-items:flex-start;font-size:12px;line-height:16px;color:var(--fg-2)}.why .ic{margin-top:1px}
.df .why{flex:1 1 180px;margin-right:auto;align-self:center}
/* themed scrollbars */
*{scrollbar-width:thin;scrollbar-color:var(--border-strong) transparent}
*::-webkit-scrollbar{width:12px;height:12px}*::-webkit-scrollbar-track{background:transparent}
*::-webkit-scrollbar-thumb{background:var(--border-strong);border-radius:999px;border:3px solid transparent;background-clip:content-box}
*::-webkit-scrollbar-thumb:hover{background:var(--fg-2);background-clip:content-box;border:3px solid transparent}
/* brand logo (light/dark variants follow the theme), popovers, tabs, About */
.logo{display:inline-flex;align-items:center}.logo svg{display:block}.logo .lg-d{display:none}
:root[data-theme=dark] .logo .lg-l{display:none}:root[data-theme=dark] .logo .lg-d{display:block}
@media (prefers-color-scheme:dark){:root:not([data-theme=light]) .logo .lg-l{display:none}:root:not([data-theme=light]) .logo .lg-d{display:block}}
.logo.mark svg{width:32px;height:32px}.logo.wordmark svg{height:44px;width:auto}
.brand .logo{flex:none}.brand2 .logo{margin-bottom:6px}.about .logo.wordmark svg{height:56px}
.popwrap{display:inline-block}
.popover{position:fixed;inset:auto;margin:0;max-width:min(300px,calc(100vw - 24px));background:var(--surface);color:var(--fg);border:1px solid var(--border-strong);border-radius:8px;box-shadow:var(--e2);padding:12px 14px;font-size:13px;line-height:19px}
.popover p{margin:0 0 8px}.popover p:last-child{margin:0}
.lrow{justify-content:space-between;margin-top:14px}
.tabs{display:flex;gap:4px;border-bottom:1px solid var(--border);margin:0 0 16px}
.tab{display:inline-flex;gap:8px;align-items:center;min-height:40px;padding:0 14px;border:0;border-bottom:2px solid transparent;background:none;color:var(--fg-2);font:600 13px/19px var(--font-ui)}
.tab:hover{color:var(--fg)}.tab[aria-selected=true]{color:var(--accent-fg);border-bottom-color:var(--accent)}
.about{display:flex;flex-direction:column;gap:16px;align-items:flex-start}
ul.cred{list-style:none;margin:0;padding:0}ul.cred li{display:flex;align-items:center;gap:12px;padding:8px 16px;border-top:1px solid var(--border);min-height:48px}ul.cred li b{min-width:140px}ul.cred .chip{font-size:12px}ul.cred li .btn{margin-left:auto}
pre.lic{max-height:60vh;overflow:auto;white-space:pre-wrap;font-size:12px;line-height:18px}
.cb>.btn+h2{margin-top:14px}.card article p,.helpbody p{max-width:68ch}
textarea.mono{width:100%}
/* add-ons */
.agrid{display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:16px;margin:0 0 16px}
.acard{background:var(--surface);border:1px solid var(--border);border-radius:12px;box-shadow:var(--e1);padding:16px;display:flex;flex-direction:column;gap:10px;min-width:0}
.acard p{margin:0;color:var(--fg-2)}
.acard .ft{margin-top:auto;display:flex;justify-content:space-between;align-items:center;gap:8px;flex-wrap:wrap}
.aname{display:flex;gap:12px;align-items:center;min-width:0}.aname b{overflow-wrap:anywhere}.anm{display:flex;flex-direction:column;min-width:0}
.adet{color:var(--fg-2);font-size:12px;line-height:16px;font-weight:400;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere}
.aico{width:40px;height:40px;border-radius:8px;background:var(--accent-bg);color:var(--accent-fg);display:inline-flex;align-items:center;justify-content:center;flex:none}
.sec{margin:20px 0 12px}
.dsec{margin:0 0 16px}.dsec>h4{margin:0 0 8px;font:600 14px/20px var(--font-ui)}
.dsec .row{margin-bottom:6px}
pre.code.log2{max-height:50vh;min-height:120px}
.chips .chip{white-space:normal;height:auto;min-height:24px;padding:4px 8px;max-width:100%;overflow-wrap:anywhere}
#main:focus,#main:focus-visible{outline:none}
.hres{display:flex;flex-direction:column;gap:4px;align-items:flex-start}.hres small{color:var(--fg-2);font-size:12px;line-height:16px;overflow-wrap:anywhere;max-width:40ch}
dialog.wide progress{margin:0 0 10px}
@media (max-width:1000px){.helpcols{grid-template-columns:1fr}}
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
const S = {csrf: null, user: null, host: '', version: '__VERSION__', page: 'dashboard', pend: null, tick: null, poll: null, st: null, used: null, alerts: null, mtab: null};
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
  if (r.status === 401 && path !== '/api/login') { showLogin(); throw new Error(T('api.expired')); }
  if (!r.ok || j.ok === false) { const e = new Error(j.error || ('HTTP ' + r.status)); e.data = j; throw e; }
  return j;
}
/* loading state: busy glyph (static under reduced motion), clicks ignored, aria-busy */
function busy(b, on) {
  if (!b) return;
  b.classList.toggle('is-loading', on);
  if (on) { b.setAttribute('aria-busy', 'true'); const g = ic('refresh-cw', 'spin'); g.dataset.busy = '1'; b.prepend(g); }
  else { b.removeAttribute('aria-busy'); const g = b.querySelector('[data-busy]'); if (g) g.remove(); }
}
async function act(btn, fn) {
  if (btn && btn.classList.contains('is-loading')) return;
  busy(btn, true);
  try { await fn(); } catch (e) { toast(e.message, true); } finally { busy(btn, false); }
}
/* ---------- building blocks ---------- */
const KIND_ICON = {ok: 'check', warn: 'triangle-alert', err: 'ban', info: 'info', neutral: 'circle-question-mark', accent: 'flag'};
const badge = (kind, text, icon) => h('span', {class: 'badge ' + kind}, ic(icon || KIND_ICON[kind] || 'info', 'sm'), text);
const chip = (a, b) => h('span', {class: 'chip'}, h('b', {}, a), b ? h('span', {class: 'mono'}, b) : null);
function banner(kind, title, text, ...acts) {
  const bi = kind === 'neutral' ? 'info' : KIND_ICON[kind];
  const el = h('div', {class: 'banner ' + kind}, ic(bi, 'lg'),
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
  if (!rows.length) return h('div', {class: 'empty'}, ic('info'), empty || T('table.empty'));
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
  const dlg = h('dialog', {'aria-labelledby': 'dlg-t', class: o.wide ? 'wide' : null});
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
    return [h('button', {type: 'button', class: 'btn', onclick: close}, T('btn.cancel')), ok];
  });
}
/* typed-name confirm: the action button stays disabled until the text matches exactly */
function confirmAction(o) {
  let inp = null, ok = null;
  return dialog(o.title, {danger: o.danger !== false, sub: o.sub, icon: o.icon}, () => h('div', {},
    o.body ? (typeof o.body === 'string' ? h('p', {}, o.body) : o.body) : null,
    o.expected ? field(T('raid.type', {name: o.expected}), inp = h('input', {class: 'mono', autocomplete: 'off', spellcheck: 'false', 'aria-describedby': 'cnt',
      oninput: () => { ok.disabled = inp.value !== o.expected; $('#cnt').textContent = inp.value.length + ' / ' + o.expected.length ; }}),
      null) : null,
    o.expected ? h('small', {class: 'help', id: 'cnt', role: 'status'}, '0 / ' + o.expected.length) : null,
    (o.extra || []).map(x => x)), (close, setErr) => {
      ok = h('button', {type: 'button', class: 'btn ' + (o.danger === false ? 'primary' : 'danger'), disabled: !!o.expected}, o.icon ? ic(o.icon === 'octagon-x' ? 'trash' : o.icon) : (o.danger === false ? null : ic('trash')), o.label || T('btn.confirm'));
      ok.addEventListener('click', async () => {
        if (ok.classList.contains('is-loading')) return;
        busy(ok, true); setErr('');
        try { await o.onConfirm(inp ? inp.value : ''); close(); } catch (e) { setErr(e.message); busy(ok, false); }
      });
      return [h('button', {type: 'button', class: 'btn', onclick: close}, T('btn.cancel')), ok];
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
  const sel = h('select', {'aria-label': T('shell.accent'), onchange: () => setAccent(sel.value)},
    ['purple', 'blue', 'green', 'amber', 'rose'].map(a => h('option', {value: a, selected: a === cur}, a[0].toUpperCase() + a.slice(1))));
  return tip(sel, TIP('shell.accent'));
};
const themeBtn = () => tip(h('button', {type: 'button', class: 'btn ghost icon-btn', 'aria-label': T('shell.theme'), onclick: toggleTheme}, ic('sun', 'lg')), TIP('shell.theme'));
/* ---------- logo (brand SVGs, light and dark variants switched by the theme) ---------- */
function brandSvg(key, cls) {
  const vb = ($('#i-brand-' + key) || {getAttribute: () => '0 0 100 100'}).getAttribute('viewBox') || '0 0 100 100', parts = vb.split(/\s+/).map(Number);
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', vb); s.setAttribute('class', cls); s.setAttribute('aria-hidden', 'true'); s.style.aspectRatio = parts[2] + ' / ' + parts[3];
  const u = document.createElementNS(NS, 'use'); u.setAttribute('href', '#i-brand-' + key); s.append(u);
  return s;
}
const logo = (kind, label) => h('span', {class: 'logo ' + kind, role: 'img', 'aria-label': label || 'synofurb'}, brandSvg(kind === 'mark' ? 'mark' : 'logo', 'lg-l'), brandSvg(kind === 'mark' ? 'mark_dark' : 'logo_dark', 'lg-d'));
/* popover (native popover attribute: Esc and click-outside close it) */
let popN = 0;
function popover(trigger, content, icon, label) {
  const id = 'pop' + (++popN);
  const p = h('div', {id, popover: 'auto', class: 'popover', role: 'dialog'}, content);
  const b = h('button', {type: 'button', class: 'btn ghost sm', popovertarget: id, 'aria-label': label ? trigger + ': ' + label : null}, icon ? ic(icon, 'sm') : null, trigger);
  p.addEventListener('toggle', ev => {
    if (ev.newState !== 'open') return;
    const r = b.getBoundingClientRect(), w = p.offsetWidth;
    p.style.left = Math.max(12, Math.min(r.left, innerWidth - w - 12)) + 'px'; p.style.top = (r.bottom + 6) + 'px';
  });
  return h('span', {class: 'popwrap'}, b, p);
}
/* a control that is off for a reason: aria-disabled keeps it focusable, clicks are ignored, and the reason is visible text the control points to with aria-describedby
   (a native disabled button swallows hover, focus and touch, so a tooltip on it is never seen) */
let whyN = 0;
const whyLine = () => h('small', {class: 'why', id: 'why' + (++whyN), role: 'status', hidden: true});
function setOff(b, line, text, show) {
  if (!b._off) { b._off = true; b.addEventListener('click', ev => { if (b.getAttribute('aria-disabled') === 'true') { ev.preventDefault(); ev.stopImmediatePropagation(); } }, true); }
  if (text) { b.setAttribute('aria-disabled', 'true'); b.setAttribute('aria-describedby', line.id); } else { b.removeAttribute('aria-disabled'); b.removeAttribute('aria-describedby'); }
  if (show !== false) { line.hidden = !text; line.replaceChildren(...(text ? [ic('info', 'sm'), text] : [])); }
  return b;
}
/* ---------- login ---------- */
/*PAGE:login*/
async function showLogin() {
  S.user = null; S.csrf = null; clearPending();
  const err = h('div', {class: 'err', role: 'alert', hidden: true});
  const u = h('input', {name: 'username', autocomplete: 'username', autocapitalize: 'none', spellcheck: 'false', required: true});
  const p = h('input', {name: 'password', type: 'password', autocomplete: 'current-password', required: true});
  const eye = tip(h('button', {type: 'button', class: 'btn icon-btn', 'aria-label': T('login.show'), onclick: () => { const s = p.type === 'password'; p.type = s ? 'text' : 'password'; eye.replaceChildren(ic(s ? 'eye-off' : 'eye')); }}, ic('eye')), TIP('login.show'));
  const go = h('button', {class: 'btn primary lg', type: 'submit'}, T('login.go'), ic('arrow-right'));
  const host = h('p', {class: 'muted mono'}, '');
  const fp = h('p', {class: 'mono'}, '...');
  const form = h('form', {onsubmit: async ev => {
    ev.preventDefault(); if (go.classList.contains('is-loading')) return; busy(go, true); err.hidden = true;
    try { const r = await api('POST', '/api/login', {username: u.value, password: p.value}); S.csrf = r.csrf; S.user = r.user; p.value = ''; await start(); }
    catch (e) { err.hidden = false; err.replaceChildren(ic('octagon-x'), e.message); busy(go, false); }
  }}, h('div', {class: 'brand2'}, logo('wordmark'), h('h1', {}, T('login.title')), host),
    field(T('login.user'), u), field(T('login.password'), h('div', {class: 'pw'}, p, eye)), err, go,
    h('div', {class: 'row lrow'}, popover(T('login.cert'), [h('p', {}, TIP('login.cert')), fp], 'lock'), btn(T('nav.help'), {kind: 'ghost', size: 'sm', icon: 'life-buoy', onclick: () => helpDialog('sign-in')})));
  $('#root').replaceChildren(h('main', {class: 'lw', id: 'main'}, h('div', {}, h('div', {class: 'lc'}, form), h('div', {class: 'ver'}, 'synofurb-mgmt ' + S.version))));
  u.focus();
  try {
    const f = await (await fetch('/api/tls-fingerprint')).json();
    fp.textContent = f.sha256 || '-';
    if (f.hostname) host.textContent = f.hostname;
  } catch (e) { fp.textContent = '-'; }
}
/*ENDPAGE*/
/* ---------- shell ---------- */
const NAV = [['dashboard', 'gauge'], ['storage', 'database'], ['raid', 'layers'], ['mounts', 'package'], ['addons', 'container'], ['network', 'network'], ['users', 'users'], ['boot', 'panel-top'], ['system', 'settings'], ['updates', 'download'], ['help', 'life-buoy'], ['about', 'info']];
const TABBAR = ['dashboard', 'storage', 'raid', 'network', 'boot'];
/*PAGE:shell*/
/*copy: nav.dashboard nav.storage nav.raid nav.mounts nav.network nav.users nav.boot nav.system nav.help nav.about*/
function buildShell() {
  const nav = h('nav', {class: 'nav', 'aria-label': T('shell.nav')});
  const side = h('aside', {class: 'side', id: 'side'}, h('div', {class: 'brand'}, logo('mark'), h('div', {}, h('b', {}, T('shell.brand')), h('small', {id: 'sh'}, S.host))), nav,
    h('div', {class: 'me'}, h('div', {class: 'avatar'}, S.user.slice(0, 2).toUpperCase()), h('div', {}, h('b', {}, S.user), h('small', {}, T('shell.role'))),
      tip(h('button', {type: 'button', class: 'btn ghost icon-btn', 'aria-label': T('shell.logout'), onclick: async () => { try { await api('POST', '/api/logout'); } catch (e) { /* ignore */ } showLogin(); }}, ic('power')), TIP('shell.logout'))),
    h('div', {class: 'ver'}, 'synofurb-mgmt ' + S.version));
  const scrim = h('div', {class: 'scrim', hidden: true, onclick: () => closeNav()});
  const top = h('header', {class: 'top'},
    h('button', {type: 'button', class: 'btn ghost icon-btn burger', 'aria-label': T('shell.menu'), onclick: () => { side.classList.add('open'); scrim.hidden = false; }}, ic('menu', 'lg')),
    h('div', {class: 'crumb', id: 'crumb'}), accentSel(), themeBtn());
  const tabbar = h('nav', {class: 'tabbar', 'aria-label': T('shell.nav')});
  const mc = h('div', {class: 'mc'}, top, h('div', {id: 'pend'}), h('main', {class: 'page', id: 'main', tabindex: '-1'}));
  $('#root').replaceChildren(h('div', {class: 'shell'}, side, mc), scrim, tabbar);
  S.nav = {nav, tabbar, side, scrim};
  renderNav();
}
function closeNav() { S.nav.side.classList.remove('open'); S.nav.scrim.hidden = true; }
function renderNav() {
  const mk = ([id, icon]) => h('button', {type: 'button', 'aria-current': id === S.page ? 'page' : null, onclick: () => { closeNav(); show(id); }}, ic(icon), h('span', {}, T('nav.' + id)));
  S.nav.nav.replaceChildren(...NAV.map(n => {
    const b = mk(n);
    if (n[0] === 'raid' && S.alerts && S.alerts.raid) b.append(badge('warn', String(S.alerts.raid), 'triangle-alert'));
    return b;
  }));
  S.nav.tabbar.replaceChildren(...NAV.filter(n => TABBAR.includes(n[0])).map(n => { const b = mk(n); b.firstChild.className.baseVal += ' lg'; return b; }));
  $('#crumb').replaceChildren(S.host + ' / ', h('b', {}, T('nav.' + S.page)));
}
async function start() {
  const me = await api('GET', '/api/me');
  S.csrf = me.csrf; S.user = me.user; S.host = me.hostname; S.version = me.version;
  document.title = me.hostname + ' - synofurb';
  buildShell();
  setPending(me.net_pending);
  show(S.page);
}
async function show(page) {
  S.page = page; S.used = new Set();
  renderNav();
  const main = $('#main');
  main.replaceChildren(h('div', {class: 'sk', 'aria-busy': 'true'}, ic('refresh-cw', 'spin')));
  try { main.replaceChildren(await PAGES[page]()); main.focus({preventScroll: true}); }
  catch (e) { main.replaceChildren(banner('err', T('page.error'), e.message)); }
}
/*ENDPAGE*/
/* ---------- rollback countdown banner (network safe-apply) ---------- */
function clearPending() {
  clearInterval(S.tick); clearInterval(S.poll); S.tick = S.poll = null; S.pend = null;
  const el = $('#pend'); if (el) el.replaceChildren();
}
/*PAGE:pending*/
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
  const num = h('b', {}), title = h('b', {class: 'tt', role: 'timer'});
  const keep = btn(T('net.keep'), {kind: 'primary', icon: 'check', onclick: () => act(keep, async () => { await api('POST', '/api/network/confirm'); toast(T('net.kept')); clearPending(); if (S.page === 'network') show('network'); })});
  const rb = btn(T('net.rollback'), {onclick: () => act(rb, async () => { await api('POST', '/api/network/rollback'); toast(T('net.rolled')); clearPending(); show(S.page); })});
  const el = h('div', {class: 'rb', role: 'alert'}, h('div', {class: 'row0'}, h('div', {class: 'ring'}, svg, num),
    h('div', {class: 'bt'}, tip(title, TIP('net.pending')), popover(T('net.cannot'), TIP('net.cannot'), 'circle-question-mark'))), h('div', {class: 'acts'}, rb, keep));
  $('#pend').replaceChildren(el);
  const upd = () => {
    const left = Math.max(0, Math.round((end - Date.now()) / 1000));
    num.textContent = left; title.textContent = T('net.confirm_in') + ' ' + Math.floor(left / 60) + ':' + String(left % 60).padStart(2, '0');
    fg.setAttribute('stroke-dashoffset', C * (1 - left / total));
    if (left === 10 || left === 5) $('#live').textContent = T('net.confirm_in') + ' ' + left + ' s';
  };
  upd(); S.tick = setInterval(upd, 1000);
  S.poll = setInterval(async () => { try { const m = await api('GET', '/api/me'); if (!m.net_pending) { clearPending(); toast(T('net.rolled')); } } catch (e) { /* link may be down */ } }, 5000);
}
/*ENDPAGE*/
const PAGES = {};
const COLS = {device: 'col.device', model: 'col.model', size: 'col.size', type: 'col.type', role: 'col.role', array: 'col.array', level: 'col.level', state: 'col.state', iface: 'col.interface', address: 'col.address',
  volume: 'col.volume', mount: 'col.mount', used: 'col.used', name: 'col.name', groups: 'col.groups', keys: 'col.keys', mode: 'col.mode', health: 'col.health', temp: 'col.temp', fs: 'col.fs'};
const th = (...ks) => ks.map(k => (k ? T(COLS[k]) : ''));
const btnIcon = (icon, label, onclick, kind) => tip(h('button', {type: 'button', class: 'btn ' + (kind || 'ghost') + ' icon-btn sm', 'aria-label': label, onclick}, ic(icon)), label);
/* ---------- Dashboard ---------- */
/*PAGE:dashboard*/
/*copy: state.degraded state.clean mc.installed mc.missing mc.na role.system role.array role.boot role.unused smart.ok smart.failing smart.unknown state.linkup state.down*/
PAGES.dashboard = async () => {
  const s = await api('GET', '/api/status');
  S.st = s;
  const mem = s.memory || {}, md = s.md.arrays, ki = s.kernel_info || {}, cpu = s.cpu || {}, mc = s.microcode || {};
  const degraded = md.filter(a => a.degraded), syncing = md.filter(a => a.sync);
  const dataMount = s.mounts.find(m => m.target === '/srv/data') || s.mounts.find(m => m.target === '/');
  const hot = s.temps.length ? Math.max(...s.temps.map(t => t.celsius)) : null;
  S.alerts = {raid: degraded.length + syncing.length};
  renderNav();
  const tl = (el, k) => tip(el, TIP(k));
  const parts = [h('div', {class: 'ph'}, h('div', {}, h('h1', {}, T('nav.dashboard')), h('p', {class: 'mono'}, [s.hostname, s.os, s.kernel, fmtDur(s.uptime)].filter(Boolean).join(' - '))))];
  const tiles = [
    tl(tile(T('dash.health'), 'shield-check', degraded.length ? T('state.degraded') : T('dash.healthy'), '', degraded.map(a => a.name).concat(syncing.map(a => a.name)).join(' ') || ' ', degraded.length ? 'err' : syncing.length ? 'warn' : 'ok'), 'dash.health'),
    tl(dataMount && dataMount.size ? tile(T('nav.storage'), 'database', pct(dataMount.used, dataMount.size), '%', fmtBytes(dataMount.used) + ' / ' + fmtBytes(dataMount.size), pct(dataMount.used, dataMount.size) >= 90 ? 'warn' : '') : tile(T('nav.storage'), 'database', '-', '', ''), 'dash.storage'),
    tl(tile(T('dash.memory'), 'memory-stick', mem.total ? ((mem.total - (mem.available || 0)) / 1073741824).toFixed(1) : '-', 'GiB', fmtBytes(mem.total)), 'dash.memory'),
    tl(tile(T('dash.load'), 'activity', s.load ? s.load[0].toFixed(2) : '-', '', (s.load ? s.load.map(x => x.toFixed(2)).join(' ') : '') + (hot != null ? ' - ' + hot.toFixed(0) + ' °C' : '')), 'dash.load'),
    tl(tile(T('dash.cpu'), 'cpu', cpu.vendor || (s.arch.startsWith('a') ? 'ARM' : '-'), '', cpu.model || s.arch), 'dash.cpu'),
    tl(tile(T('dash.kernel'), 'terminal', ki.release || s.kernel, '', ki.ge7 ? '7+' : (s.arch || '')), ki.ge7 ? 'dash.kernel7' : 'dash.kernel'),
    tl(tile(T('dash.microcode'), 'wrench', mc.applicable ? (mc.installed ? T('mc.installed') : T('mc.missing')) : T('mc.na'), '', mc.applicable ? (mc.package + (mc.version ? ' ' + mc.version : '')) : '', mc.applicable && !mc.installed ? 'warn' : ''), mc.applicable && !mc.installed ? 'dash.mc_missing' : 'dash.microcode'),
    tl(tile(T('nav.network'), 'network', s.interfaces.filter(i => i.ifname !== 'lo' && i.state === 'UP').length, T('dash.up'), s.interfaces.filter(i => i.ifname !== 'lo').map(i => i.ifname).join(' ')), 'dash.network'),
  ];
  parts.push(h('div', {class: 'tiles'}, tiles));
  if (degraded.length) parts.push(banner('err', T('dash.attention'), degraded.map(a => a.name).join(' '), btn(T('act.open_raid'), {size: 'sm', onclick: () => show('raid')})));
  // disks: physical disks with their role; a RAID member shows up under every disk of the array, so follow `children`
  const byK = {}; s.disks.forEach(d => { byK[d.kname] = d; });
  const kidsOf = d => (d.children || []).map(k => byK[k]).filter(Boolean);
  const below = d => { const o = [], st = [...kidsOf(d)], seen = new Set(); while (st.length) { const x = st.pop(); if (seen.has(x.kname)) continue; seen.add(x.kname); o.push(x); st.push(...kidsOf(x)); } return o; };
  const role = d => {
    const sub = below(d), mps = d.mountpoints.concat(...sub.map(x => x.mountpoints)), out = [];
    if (d.kname.startsWith('synoboot') || d.model === 'DOM') return badge('info', T('role.boot'), 'panel-top');
    if (mps.some(m => ['/', '/boot', '/boot/efi', '[SWAP]'].includes(m))) out.push(badge('info', T('role.system'), 'server'));
    const mdn = [...new Set(sub.filter(x => /^raid|^md/.test(x.type)).map(x => x.name))];
    if (mdn.length) out.push(badge('ok', T('role.array') + ' ' + mdn.sort().join(' '), 'layers'));
    if (out.length) return h('div', {class: 'row'}, out);
    if (sub.some(x => x.fstype || x.mountpoints.length) || d.fstype) return badge('neutral', (sub.find(x => x.fstype) || d).fstype || '-', 'database');
    return badge('neutral', T('role.unused'), 'plus');
  };
  const drows = s.disks.filter(d => d.type === 'disk' && !/^(loop|zram|ram|sr|nbd)/.test(d.kname)).map(d => [h('b', {class: 'mono'}, '/dev/' + d.kname), [d.model, d.serial].filter(Boolean).join(' - ') || '-', fmtBytes(d.size),
    [d.tran ? chip(d.tran) : null, chip(d.rota ? 'HDD' : 'SSD')], role(d)]);
  const smart = h('div', {});
  const sb = btn(T('act.smart'), {icon: 'shield-check', onclick: () => act(sb, async () => {
    const r = await api('GET', '/api/smart');
    smart.replaceChildren(!r.available ? h('div', {class: 'empty'}, ic('info'), 'smartctl') :
      table(th('device', 'model', 'health', 'temp'), r.disks.map(d => ({cls: d.passed === false ? 'bad' : d.passed == null ? 'attn' : '', cells: [h('span', {class: 'mono'}, d.device), d.model || '',
        d.passed === true ? badge('ok', T('smart.ok')) : d.passed === false ? badge('err', T('smart.failing'), 'octagon-x') : badge('neutral', T('smart.unknown'), 'circle-dashed'), d.temp != null ? d.temp + ' °C' : '']}))));
  })});
  parts.push(cardFlush(T('dash.disks'), 'hard-drive', sb, table(th('device', 'model', 'size', 'type', 'role'), drows, '', {right: [2]}), smart));
  if (md.length) parts.push(cardFlush(T('dash.arrays'), 'layers', null, table(th('array', 'level', 'state', 'size'), md.map(a => ({cls: a.degraded ? 'bad' : a.sync ? 'attn' : '', cells: [h('b', {class: 'mono'}, a.name), a.level ? chip(a.level.toUpperCase()) : '',
    a.degraded ? badge('err', T('state.degraded'), 'triangle-alert') : a.sync ? badge('warn', a.sync.action + (a.sync.percent != null ? ' ' + a.sync.percent + ' %' : ''), 'refresh-cw') : badge('ok', T('state.clean')), fmtBytes(a.size)]})), '', {right: [3]})));
  parts.push(cardFlush(T('nav.network'), 'network', null, table(th('iface', 'state', 'address'), s.interfaces.filter(i => i.ifname !== 'lo').map(i => [h('span', {class: 'mono'}, i.ifname),
    i.state === 'UP' ? badge('ok', T('state.linkup')) : badge('neutral', T('state.down'), 'x'), h('span', {class: 'mono'}, i.addrs.filter(a => a.scope !== 'link').map(a => a.addr + '/' + a.prefix).join('  '))]))));
  return h('div', {}, parts);
};
/*ENDPAGE*/
/* ---------- Storage ---------- */
const RAID_INFO = {none: 'lv.none', raid0: 'lv.raid0', raid1: 'lv.raid1', raid5: 'lv.raid5', raid6: 'lv.raid6', raid10: 'lv.raid10', shr1: 'lv.shr1', shr2: 'lv.shr2'};
function usableBytes(level, sizes) {
  if (!sizes.length) return 0;
  if (level === 'shr1' || level === 'shr2') { const d = {}; sizes.forEach((s, i) => { d['d' + i] = Math.max(0, s - 2 * MIBN); }); const r = shrSlices(d, level); return r ? r.usable : 0; }
  const m = Math.min(...sizes), n = sizes.length;
  return {none: sizes.reduce((a, b) => a + b, 0), raid0: m * n, raid1: m, raid5: m * (n - 1), raid6: m * (n - 2), raid10: Math.floor(m * n / 2)}[level];
}
function countOk(level, n) {
  if (level === 'none') return n >= 1;
  if (level === 'shr1') return n >= 2;
  if (level === 'shr2') return n >= 4;
  const min = {raid0: 2, raid1: 2, raid5: 3, raid6: 4, raid10: 4}[level];
  return n >= min && (level !== 'raid10' || n % 2 === 0);
}
function logView(results) {
  return h('div', {class: 'log', role: 'log'}, results.map(r => [
    h('div', {class: r.rc === 0 ? '' : 'e'}, h('span', {class: 'lv'}, r.rc === 0 ? '● OK' : '✖ ERR'), h('span', {}, r.desc + '  ($ ' + r.cmd + ')')),
    (r.out || r.err) ? h('span', {class: 'm'}, (r.out + (r.err ? '\n' + r.err : '')).trim()) : null]));
}
/*PAGE:storage*/
PAGES.storage = async () => {
  const [v, m] = await Promise.all([api('GET', '/api/volumes'), api('GET', '/api/mounts')]);
  const reload = () => show('storage');
  const mount = {}; m.mounts.forEach(x => { mount[x.target] = x; });
  const wizardHost = h('div', {id: 'wiz', tabindex: '-1', hidden: true});
  const openWiz = () => { if (!wizardHost.childNodes.length) wizard(wizardHost, v, reload); wizardHost.hidden = false; wizardHost.scrollIntoView({behavior: 'smooth'}); wizardHost.focus(); };
  const parts = [h('div', {class: 'ph'}, h('div', {class: 'row'}, h('h1', {}, T('nav.storage')), infoBtn('storage', T('nav.storage'))), h('div', {class: 'row'},
    tip(btn(T('nav.raid'), {kind: 'ghost', icon: 'layers', onclick: () => show('raid')}), TIP('st.raid_link')), tip(btn(T('st.create'), {kind: 'primary', icon: 'plus', onclick: openWiz}), TIP('st.create'))))];
  const rows = v.lvs.map(l => {
    const free = (v.vgs.find(g => g.vg_name === l.vg_name) || {}).vg_free || 0, mp = l.mountpoints[0], mt = mp && mount[mp];
    const ext = tip(btn(T('act.extend'), {size: 'sm', kind: 'ghost', disabled: free <= 0, onclick: () => formDialog(T('dlg.extend', {name: l.lv_name}), [
      {name: 'add_gib', label: T('st.add_gib'), type: 'number'}], T('act.extend'),
      async x => { await api('POST', '/api/volumes/extend', {vg: l.vg_name, lv: l.lv_name, add_gib: x.add_gib === '' ? null : Number(x.add_gib)}); toast(T('raid.done')); reload(); }, {icon: 'plus', sub: fmtBytes(free) + ' ' + T('st.free')})}), free <= 0 ? TIP('st.nofree') : TIP('st.extend'));
    const del = btn(T('act.delete'), {size: 'sm', kind: 'ghost', icon: 'trash', onclick: () => confirmAction({title: T('dlg.delete', {name: l.lv_name}), expected: l.lv_name, label: T('act.delete'), body: TIP('st.delete'),
      extra: [h('label', {class: 'chk'}, h('input', {type: 'checkbox', id: 'rmvg'}), h('span', {}, T('st.rmvg')))],
      onConfirm: async () => { await api('POST', '/api/volumes/delete', {vg: l.vg_name, lv: l.lv_name, confirm: l.lv_name, remove_vg: $('#rmvg').checked}); toast(T('raid.done')); reload(); }})});
    return [h('b', {}, l.lv_name), h('span', {class: 'mono'}, mp || '-'), fmtBytes(l.lv_size), mt && mt.size ? usage(mt.used, mt.size) : (mp ? '' : badge('neutral', T('state.unmounted'), 'x')), h('div', {class: 'row'}, ext, del)];
  });
  parts.push(cardFlush(T('st.volumes'), 'database', h('div', {class: 'row'}, v.vgs.map(g => tip(chip(g.vg_name, fmtBytes(g.vg_free) + ' ' + T('st.free')), TIP('st.vg')))),
    table(th('volume', 'mount', 'size', 'used', ''), rows, T('st.empty'), {right: [2]})));
  parts.push(wizardHost);
  return h('div', {}, parts);
};
/*copy: wiz.devices wiz.layout wiz.volume wiz.review raid.name raid.fs st.mountpoint st.size_gib btn.back btn.cancel btn.next st.plan raid.commands raid.createrun lv.none lv.raid0 lv.raid1 lv.raid5 lv.raid6 lv.raid10 lv.shr1 lv.shr2 raid.usable*/
function wizard(host, v, reload) {
  const W = {step: 1, devs: new Set(), level: 'none', name: '', fs: 'xfs', mp: '', size: '', plan: null, perr: '', t: null};
  const preview = h('div', {}), stage = h('div', {class: 'cb'});
  const cand = v.candidates, sizes = () => cand.filter(c => W.devs.has(c.path)).map(c => c.size), usable = () => usableBytes(W.level, sizes());
  const nameOk = () => /^[a-z][a-z0-9_]{0,23}$/.test(W.name) && !['sys', 'data', 'root', 'swap', 'boot', 'efi'].includes(W.name);
  const body = () => ({name: W.name, level: W.level, fs: W.fs, mountpoint: W.mp || null, size_gib: W.size ? Number(W.size) : null, devices: [...W.devs]});
  function refreshPlan() {
    clearTimeout(W.t);
    W.t = setTimeout(async () => {
      if (!W.devs.size || !countOk(W.level, W.devs.size) || !nameOk()) { W.plan = null; W.perr = ''; drawPreview(); return; }
      try { W.plan = await api('POST', '/api/volumes/plan', body()); W.perr = ''; } catch (e) { W.plan = null; W.perr = e.message; }
      drawPreview(); if (W.step === 4) draw();
    }, 300);
  }
  function drawPreview() {
    const p = W.plan;
    preview.replaceChildren(h('section', {class: 'card'}, h('div', {class: 'ch'}, h('h2', {}, ic('eye', 'lg'), T('st.plan')), tip(badge('ok', T('st.nowrite'), 'shield-check'), TIP('st.nowrite'))),
      h('div', {class: 'cb plan'}, p ? [p.warnings.map(w => banner('warn', w, '')), h('div', {class: 'cap'}, T('raid.commands')), h('pre', {class: 'code'}, p.steps.map((s, i) => (i + 1) + '  ' + s.cmd).join('\n'))] : (W.perr ? banner('err', W.perr, '') : null))));
  }
  const cancel = () => { host.hidden = true; host.replaceChildren(); wizard(host, v, reload); };
  function nav(canNext, onNext, label) {
    return h('div', {class: 'row'}, W.step > 1 ? btn(T('btn.back'), {icon: 'arrow-left', onclick: () => { W.step--; draw(); }}) : null, btn(T('btn.cancel'), {onclick: cancel}),
      btn(label || T('btn.next'), {kind: 'primary', disabled: !canNext, onclick: onNext || (() => { W.step++; draw(); })}));
  }
  function draw() {
    const labels = [T('wiz.devices'), T('wiz.layout'), T('wiz.volume'), T('wiz.review')];
    const steps = h('div', {class: 'steps', role: 'list'}, labels.map((l, i) => h('div', {role: 'listitem', class: i + 1 === W.step ? 'cur' : i + 1 < W.step ? 'done' : '', 'aria-current': i + 1 === W.step ? 'step' : null}, h('i', {}, i + 1 < W.step ? '✓' : String(i + 1)), h('span', {}, l))));
    let c;
    if (W.step === 1) {
      c = [cand.length ? cand.map(d => { const cb = h('input', {type: 'checkbox', checked: W.devs.has(d.path), onchange: () => { cb.checked ? W.devs.add(d.path) : W.devs.delete(d.path); draw(); refreshPlan(); }});
        return h('label', {class: 'opt' + (W.devs.has(d.path) ? ' sel' : '')}, cb, h('div', {}, h('b', {}, h('span', {class: 'mono'}, d.path), chip(fmtBytes(d.size)), d.tran ? chip(d.tran) : null), h('small', {}, [d.model, d.serial].filter(Boolean).join(' - ')))); }) :
        h('div', {class: 'empty'}, ic('info'), T('raid.nofree')), nav(W.devs.size > 0)];
    } else if (W.step === 2) {
      c = [h('div', {class: 'lvls', role: 'radiogroup', 'aria-label': T('wiz.layout')}, Object.keys(RAID_INFO).map(k => {
        const ok = countOk(k, W.devs.size), r = h('input', {type: 'radio', name: 'lvl', checked: W.level === k, disabled: !ok, onchange: () => { W.level = k; draw(); refreshPlan(); }});
        return tip(h('label', {class: 'opt lvl' + (W.level === k ? ' sel' : '') + (ok ? '' : ' off')}, r, h('div', {}, h('b', {}, T(RAID_INFO[k]), ok ? chip(T('raid.usable') + ' ' + fmtBytes(usableBytes(k, sizes()))) : null))), TIP(RAID_INFO[k]));
      })), nav(countOk(W.level, W.devs.size))];
    } else if (W.step === 3) {
      const maxGib = Math.max(1, Math.floor(usable() / 1073741824));
      const nm = h('input', {class: 'mono', value: W.name, placeholder: 'media', autocomplete: 'off', oninput: () => { W.name = nm.value.trim(); nx.disabled = !nameOk(); refreshPlan(); }});
      const fs = h('select', {onchange: () => { W.fs = fs.value; refreshPlan(); }}, v.filesystems.map(x => h('option', {value: x, selected: x === W.fs}, x)));
      const mp = h('input', {class: 'mono', value: W.mp, placeholder: '/srv/' + (W.name || '...'), autocomplete: 'off', oninput: () => { W.mp = mp.value.trim(); refreshPlan(); }});
      const num = h('input', {type: 'number', min: 1, max: maxGib, value: W.size || '', placeholder: String(maxGib), oninput: () => { W.size = num.value; rng.value = num.value || maxGib; refreshPlan(); }});
      const rng = h('input', {type: 'range', min: 1, max: maxGib, value: W.size || maxGib, 'aria-label': T('st.size_gib'), oninput: () => { W.size = Number(rng.value) >= maxGib ? '' : rng.value; num.value = W.size; refreshPlan(); }});
      const nx = btn(T('btn.next'), {kind: 'primary', disabled: !nameOk(), onclick: () => { W.step = 4; draw(); }});
      c = [h('div', {class: 'grid2'}, field(T('raid.name'), nm), field(T('raid.fs'), tip(fs, TIP('raid.fs')))), field(T('st.mountpoint'), mp), field(T('st.size_gib'), num), rng,
        h('div', {class: 'row'}, btn(T('btn.back'), {icon: 'arrow-left', onclick: () => { W.step = 2; draw(); }}), btn(T('btn.cancel'), {onclick: cancel}), nx)];
    } else {
      const go = btn(T('raid.createrun'), {kind: 'danger', icon: 'trash', disabled: !W.plan, onclick: () => confirmAction({title: T('dlg.create', {name: W.name}), expected: W.name, label: T('raid.createrun'),
        body: h('div', {}, (W.plan ? W.plan.warnings : []).map(w => banner('warn', w, ''))),
        onConfirm: async () => { const res = await api('POST', '/api/volumes/create', Object.assign(body(), {confirm: W.name})); toast(res.mountpoint); reload(); }})});
      c = [W.plan ? [banner('err', W.plan.warnings[0] || '', ''), h('dl', {class: 'kv'}, h('dt', {}, T('raid.name')), h('dd', {}, W.plan.name), h('dt', {}, T('col.device')), h('dd', {class: 'mono'}, [...W.devs].join(' ')),
        h('dt', {}, T('raid.fs')), h('dd', {}, W.plan.fs), h('dt', {}, T('st.mountpoint')), h('dd', {class: 'mono'}, W.plan.mountpoint), h('dt', {}, T('raid.usable')), h('dd', {}, fmtBytes(W.plan.usable_bytes)))] : banner('err', W.perr || '-', ''),
        h('div', {class: 'row'}, btn(T('btn.back'), {kind: 'primary', icon: 'arrow-left', onclick: () => { W.step = 3; draw(); }}), btn(T('btn.cancel'), {onclick: cancel}), go)];
    }
    stage.replaceChildren(steps, ...[c].flat(Infinity).filter(Boolean));
  }
  host.replaceChildren(h('div', {class: 'cols'}, h('section', {class: 'card'}, h('div', {class: 'ch'}, h('h2', {}, ic('plus', 'lg'), T('st.create'))), stage), preview));
  draw(); drawPreview();
}
/*ENDPAGE*/
/* ---------- Mounts (mounted filesystems and imported volumes) ---------- */
/*PAGE:mounts*/
PAGES.mounts = async () => {
  const r = await api('GET', '/api/mounts'), reload = () => show('mounts');
  let tab = S.mtab || 'mounted';
  const area = h('div', {});
  const tabs = h('div', {class: 'tabs', role: 'tablist'});
  const drawTabs = () => tabs.replaceChildren(...['mounted', 'imported'].map(k => tip(h('button', {type: 'button', role: 'tab', 'aria-selected': String(tab === k), class: 'tab', onclick: () => { tab = S.mtab = k; draw(); }}, T('mnt.' + k), k === 'imported' && r.devices.length ? h('span', {class: 'badge neutral'}, String(r.devices.length)) : null), TIP('mnt.' + k))));
  function draw() {
    drawTabs();
    if (tab === 'mounted') {
      const rows = r.mounts.map(m => [h('span', {class: 'mono'}, m.target), h('span', {class: 'mono'}, m.source), m.fstype, m.size ? usage(m.used, m.size) : '', /^\/(srv|mnt)\//.test(m.target) ? btn(T('act.unmount'), {size: 'sm', kind: 'ghost', onclick: () => confirmAction({title: T('dlg.unmount', {name: m.target}), expected: m.target, label: T('act.unmount'),
        extra: [h('label', {class: 'chk'}, h('input', {type: 'checkbox', id: 'rmfs'}), h('span', {}, T('mnt.rmfstab')))],
        onConfirm: async () => { await api('POST', '/api/mounts/unmount', {mountpoint: m.target, confirm: m.target, remove_fstab: $('#rmfs').checked}); toast(T('raid.done')); reload(); }})}) : '']);
      area.replaceChildren(cardFlush(T('nav.mounts'), 'package', null, table(th('mount', 'device', 'type', 'used', ''), rows)),
        h('details', {class: 'card'}, h('summary', {class: 'cb'}, T('mnt.fstab')), table(['', '', '', ''], r.fstab.map(e => [h('span', {class: 'mono'}, e.spec), h('span', {class: 'mono'}, e.mountpoint), e.fstype, e.managed ? badge('accent', 'synofurb', 'check') : '']))));
    } else {
      const scan = btn(T('mnt.scan'), {icon: 'refresh-cw', onclick: () => act(scan, async () => { await api('POST', '/api/mounts/activate'); toast(T('raid.done')); reload(); })});
      const rows = r.devices.map(d => {
        const mp = h('input', {class: 'mono', value: d.suggested, 'aria-label': T('col.mount')}), ro = h('input', {type: 'checkbox', checked: d.ro_default, 'aria-label': T('mnt.ro')}), pe = h('input', {type: 'checkbox', 'aria-label': T('mnt.boot')});
        const go = btn(T('act.mount'), {kind: 'primary', size: 'sm', onclick: () => act(go, async () => { await api('POST', '/api/mounts/mount', {device: d.path, mountpoint: mp.value.trim(), ro: ro.checked, persist: pe.checked}); toast(T('raid.done')); reload(); })});
        return [h('b', {class: 'mono'}, d.path), d.fstype, fmtBytes(d.size), mp, h('div', {class: 'row'}, tip(h('label', {class: 'chk'}, ro, h('span', {}, T('mnt.ro'))), TIP('mnt.ro')), h('label', {class: 'chk'}, pe, h('span', {}, T('mnt.boot')))), go];
      });
      area.replaceChildren(cardFlush(T('mnt.imported'), 'package', h('div', {class: 'row'}, r.other.length ? tip(chip(String(r.other.length), T('mnt.other')), r.other.map(o => o.path + ' ' + o.fstype).join(', ')) : null, scan),
        table(th('device', 'type', 'size', 'mount', 'mode', ''), rows, T('mnt.none'), {right: [2]})));
    }
  }
  draw();
  return h('div', {}, h('div', {class: 'ph'}, h('div', {class: 'row'}, h('h1', {}, T('nav.mounts')), infoBtn('imports', T('nav.mounts')))), tabs, area);
};
/*ENDPAGE*/
/* ---------- Add-ons ---------- */
/*PAGE:addons*/
/*copy: ad.st.running ad.st.stopped ad.st.starting ad.st.installing ad.st.updating ad.st.removing ad.st.unhealthy ad.st.error ad.st.needs-attention ad.st.not-installed*/
const AD_KIND = {running: 'ok', stopped: 'neutral', starting: 'info', installing: 'info', updating: 'info', removing: 'info', unhealthy: 'warn', error: 'err', 'needs-attention': 'warn', 'not-installed': 'neutral'};
const adIcon = n => ($('#i-' + n) ? n : 'package');
const adTag = st => tip(badge(AD_KIND[st] || 'neutral', T('ad.st.' + st)), TIP('ad.st.' + st));
const adUrl = a => (a.link ? a.link.scheme + '://' + location.host.replace(/:\d+$/, '') + ':' + a.link.port + a.link.path : null);
const adHelp = (text, label) => popover(T('ad.help'), h('p', {}, text), 'info', label);
const adPriv = p => tip(h('span', {class: 'chip'}, ic('triangle-alert', 'sm'), p.startsWith('cap:') ? T('ad.priv.cap') + ' ' + p.slice(4) : T('ad.priv.' + p)), p.startsWith('cap:') ? TIP('ad.priv.cap') : TIP('ad.priv.' + p));
const adName = (a, note) => h('span', {class: 'aname'}, h('span', {class: 'aico'}, ic(adIcon(a.icon), 'lg')), h('span', {class: 'anm'}, h('b', {'data-d': ''}, a.name), note ? h('small', {class: 'adet', 'data-d': 'prose'}, note) : null));
/* a job in a dialog: progress bar, current step, log; polls until the job ends */
function adJob(first, name, action, done) {
  const bar = h('progress', {max: 100, value: first.progress || 0, 'aria-label': T('ad.progress')}), msg = h('p', {'data-d': '', role: 'status'}, first.message || ''), log = h('pre', {class: 'code log2', 'aria-live': 'off'}, '');
  let timer = null, hideB = null;
  /* the primary button says Hide while the job runs (closing only hides it) and Close once it has ended */
  const dlg = dialog(T('ad.job_t', {name, action: T('ad.act.' + action)}), {icon: 'refresh-cw', wide: true}, () => h('div', {}, bar, msg, log), close => [hideB = h('button', {type: 'button', class: 'btn primary', onclick: close}, T('ad.hide'))]);
  const poll = async () => {
    try {
      const j = (await api('GET', '/api/jobs/' + first.id)).job;
      bar.value = j.progress; msg.textContent = j.state === 'running' ? j.message : (j.ok ? T('ad.done') : j.error);
      log.textContent = j.log.join('\n'); log.scrollTop = log.scrollHeight;
      if (j.state !== 'running') {
        clearInterval(timer); bar.className = j.ok ? 'ok' : 'err'; hideB.replaceChildren(T('btn.close'));
        if (j.warnings.length) log.textContent += '\n' + j.warnings.join('\n');
        toast(j.ok ? T('ad.done') : j.error, !j.ok); if (done) done();
      }
    } catch (e) { clearInterval(timer); msg.textContent = e.message; }
  };
  dlg.addEventListener('close', () => { clearInterval(timer); if (done) done(); });
  timer = setInterval(poll, 1200); poll();
}
async function adAct(a, action, reload) {
  const r = await api('POST', '/api/addons/' + a.id + '/' + action, {});
  adJob(r.job, a.name, action, reload);
}
function adLogs(a) {
  const pre = h('pre', {class: 'code log2'}, '');
  const sel = h('select', {'aria-label': T('ad.lines'), onchange: () => load()}, [50, 200, 1000].map(n => h('option', {value: n, selected: n === 200}, String(n))));
  async function load() {
    try { const r = await api('GET', '/api/addons/' + a.id + '/logs?tail=' + sel.value); pre.textContent = r.logs || '-'; pre.scrollTop = pre.scrollHeight; }
    catch (e) { pre.textContent = e.message; }
  }
  dialog(T('ad.logs_t', {name: a.name}), {icon: 'scroll-text', wide: true}, () => h('div', {}, h('div', {class: 'row'}, sel, btn(T('ad.refresh'), {icon: 'refresh-cw', size: 'sm', onclick: () => load()})), pre), close => [h('button', {type: 'button', class: 'btn primary', onclick: close}, T('btn.close'))]);
  load();
}
async function adDetails(a) {
  const d = (await api('GET', '/api/addons/' + a.id)).addon, url = adUrl(d);
  dialog(d.name, {icon: adIcon(d.icon), wide: true, sub: d.summary}, () => h('div', {},
    h('div', {class: 'row dsec'}, chip(T('ad.rev'), d.installed_version || d.version), adTag(d.status), url ? h('a', {class: 'btn sm', href: url, target: '_blank', rel: 'noopener noreferrer'}, ic('external-link'), T('ad.open')) : h('span', {class: 'muted'}, T('ad.noweb')),
      d.homepage ? h('a', {class: 'btn sm', href: d.homepage, target: '_blank', rel: 'noopener noreferrer'}, ic('external-link'), T('ad.homepage')) : null),
    d.detail ? banner('warn', T('ad.st.' + d.status), d.detail) : null,
    d.post_install ? h('div', {class: 'dsec'}, h('h4', {}, T('ad.note')), h('p', {'data-d': 'prose'}, d.post_install)) : null,
    d.privileges.length ? h('div', {class: 'dsec'}, h('h4', {}, T('ad.privs')), h('div', {class: 'chips'}, d.privileges.map(p => adPriv(p.id)))) : null,
    d.ports.length ? h('div', {class: 'dsec'}, h('h4', {}, T('ad.ports')), h('div', {class: 'chips'}, d.ports.map(p => chip(p.host + '/' + p.proto, p.label)))) : null,
    d.storage_report.length ? h('div', {class: 'dsec'}, h('h4', {}, T('ad.storage')), d.storage_report.map(s => h('div', {class: 'row'}, h('b', {'data-d': ''}, s.label), h('span', {class: 'mono'}, s.path)))) : null),
    close => [h('button', {type: 'button', class: 'btn primary', onclick: close}, T('btn.close'))]);
}
async function adInstall(a, byId, reload) {
  const d = (await api('GET', '/api/addons/' + a.id)).addon, get = {}, parts = [];
  const priv = h('div', {class: 'chips'}), accept = h('input', {type: 'checkbox', id: 'ad-accept'});
  const privSec = h('div', {class: 'dsec'}, h('h4', {}, T('ad.privs')), priv, h('label', {class: 'chk'}, accept, h('span', {}, T('ad.accept'))));
  let ok = null;
  const why = whyLine();
  const cur = () => { const v = {}; for (const s of d.settings) v[s.key] = get[s.key](); return v; };
  function redraw() {
    const v = cur(), list = d.privileges.map(p => p.id);
    for (const s of d.settings) if (s.type === 'choice') { const c = s.choices.find(x => x.value === v[s.key]); if (c) for (const p of c.privileges) if (!list.includes(p)) list.push(p); }
    priv.replaceChildren(...list.map(adPriv)); privSec.hidden = !list.length;
    if (ok) setOff(ok, why, list.length && !accept.checked ? T('ad.why_priv') : null);
  }
  accept.addEventListener('change', redraw);
  if (d.changes.length) parts.push(h('div', {class: 'dsec'}, h('h4', {}, T('ad.changes')), h('ul', {}, d.changes.map(c => h('li', {'data-d': 'prose'}, c)))));
  const miss = d.requires.filter(r => !(byId[r] && byId[r].installed));
  if (miss.length) parts.push(banner('warn', T('ad.needs', {name: miss.map(r => (byId[r] ? byId[r].name : r)).join(', ')}), null));
  if (d.settings.length) {
    const rows = d.settings.map(s => {
      const id = 'ad-' + s.key, once = s.first_install_only ? h('small', {class: 'help'}, T('ad.once')) : null;
      let inp;
      if (s.type === 'bool') { inp = h('input', {type: 'checkbox', id, checked: !!d.values[s.key]}); get[s.key] = () => inp.checked; return h('div', {class: 'row'}, h('label', {class: 'chk'}, inp, h('span', {'data-d': ''}, s.label)), s.help ? adHelp(s.help, s.label) : null); }
      if (s.type === 'choice') { inp = h('select', {id, onchange: redraw}, s.choices.map(c => h('option', {value: c.value, selected: c.value === d.values[s.key]}, c.label))); get[s.key] = () => inp.value; }
      else if (s.type === 'text') { inp = h('textarea', {id, rows: 4, class: 'mono'}, d.values[s.key] == null ? '' : d.values[s.key]); get[s.key] = () => inp.value; }
      else {
        const sec = s.type === 'secret';
        inp = h('input', {id, type: sec ? 'password' : (s.type === 'int' ? 'number' : 'text'), class: sec ? '' : 'mono', autocomplete: sec ? 'new-password' : 'off', spellcheck: 'false', min: s.min, max: s.max,
          value: sec ? '' : (d.values[s.key] == null ? '' : d.values[s.key]), placeholder: sec ? (d.values[s.key] && d.values[s.key].set ? T('ad.secret_set') : '') : ''});
        get[s.key] = () => (s.type === 'int' && inp.value !== '' ? Number(inp.value) : inp.value);
      }
      return h('div', {class: 'f'}, h('div', {class: 'row'}, h('label', {class: 'lab', for: id, 'data-d': ''}, s.label + (s.required ? ' *' : '')), s.help ? adHelp(s.help, s.label) : null), inp, once);
    });
    parts.push(h('div', {class: 'dsec'}, h('h4', {}, T('ad.settings')), rows));
  }
  const devs = {};
  if (d.device_states.length) parts.push(h('div', {class: 'dsec'}, h('h4', {}, T('ad.devices')), d.device_states.map(x => {
    const cb = h('input', {type: 'checkbox', checked: x.present}); devs[x.id] = cb;
    return h('div', {class: 'row'}, h('label', {class: 'chk'}, cb, h('span', {'data-d': ''}, x.label)), x.present ? badge('ok', T('ad.present')) : tip(badge('warn', T('ad.absent')), T('ad.absent_warn')));
  })));
  const stor = {};
  if (d.storage_report.length) parts.push(h('div', {class: 'dsec'}, h('h4', {}, T('ad.storage')), d.storage_report.map(x => {
    const i = h('input', {class: 'mono', value: x.path, 'aria-label': x.label}); stor[x.key] = i;
    return h('div', {class: 'f'}, h('label', {class: 'lab', 'data-d': ''}, x.label), i, h('small', {class: 'help'}, (x.free_gib == null ? '' : x.free_gib + ' GiB ' + T('ad.free') + ' ') + (x.error || x.warning || '')));
  })));
  parts.push(privSec);
  return dialog(T('ad.install_t', {name: d.name}), {icon: adIcon(d.icon), wide: true, sub: d.summary}, () => h('div', {}, parts), (close, setErr) => {
    ok = h('button', {type: 'button', class: 'btn primary'}, ic('download'), a.status === 'needs-attention' ? T('ad.retry') : T('ad.install'));
    ok.addEventListener('click', async () => {
      if (ok.classList.contains('is-loading')) return;
      busy(ok, true); setErr('');
      try {
        const body = {settings: cur(), storage: {}, devices: {}, accept: true};
        for (const [k, i] of Object.entries(stor)) body.storage[k] = i.value.trim();
        for (const [k, cb] of Object.entries(devs)) body.devices[k] = cb.checked;
        const r = await api('POST', '/api/addons/' + a.id + '/install', body);
        close(); adJob(r.job, d.name, 'install', reload);
      } catch (e) { setErr(e.message); busy(ok, false); }
    });
    redraw();
    return [why, h('button', {type: 'button', class: 'btn', onclick: close}, T('btn.cancel')), ok];
  });
}
function adRemove(a, reload) {
  const del = h('input', {type: 'checkbox'}), typed = h('input', {class: 'mono', autocomplete: 'off', spellcheck: 'false', 'aria-label': T('raid.type', {name: a.id})});
  const typedF = field(T('raid.type', {name: a.id}), typed); typedF.hidden = true;
  let ok = null;
  const why = whyLine();
  const sync = () => { typedF.hidden = !del.checked; if (ok) setOff(ok, why, del.checked && typed.value !== a.id ? T('ad.why_type') : null); };
  del.addEventListener('change', sync); typed.addEventListener('input', sync);
  dialog(T('ad.remove_q', {name: a.name}), {danger: true, sub: T('ad.remove_b')}, () => h('div', {}, tip(h('label', {class: 'chk'}, del, h('span', {}, T('ad.deldata'))), TIP('ad.deldata')), typedF), (close, setErr) => {
    ok = h('button', {type: 'button', class: 'btn danger'}, ic('trash'), T('ad.remove'));
    ok.addEventListener('click', async () => {
      if (ok.classList.contains('is-loading')) return;
      busy(ok, true); setErr('');
      try { const r = await api('POST', '/api/addons/' + a.id + '/remove', {delete_data: del.checked, confirm: del.checked ? typed.value : ''}); close(); adJob(r.job, a.name, 'remove', reload); }
      catch (e) { setErr(e.message); busy(ok, false); }
    });
    sync();
    return [why, h('button', {type: 'button', class: 'btn', onclick: close}, T('btn.cancel')), ok];
  });
}
PAGES.addons = async () => {
  const r = await api('GET', '/api/addons'), reload = () => show('addons');
  const byId = Object.fromEntries(r.addons.map(a => [a.id, a])), inst = r.addons.filter(a => a.installed);
  const missing = a => a.requires.filter(id => !(byId[id] && byId[id].installed && byId[id].status !== 'needs-attention'));
  const parts = [h('div', {class: 'ph'}, h('div', {class: 'row'}, h('h1', {}, T('nav.addons')), infoBtn('addons', T('nav.addons'))))];
  if (r.job) parts.push(banner('neutral', T('ad.busy'), r.job.message, btn(T('ad.view'), {size: 'sm', onclick: () => adJob(r.job, r.job.addon, r.job.kind, reload)})));
  for (const p of r.problems) parts.push(banner('warn', T('ad.problems'), p.id + ': ' + p.error));
  if (inst.length) {
    const rows = inst.map(a => {
      const url = a.status === 'running' || a.status === 'starting' ? adUrl(a) : null, up = a.status === 'running' || a.status === 'starting' || a.status === 'unhealthy' || a.status === 'error';
      const miss = missing(a), att = a.status === 'needs-attention', why = whyLine();
      const retry = att ? setOff(h('button', {type: 'button', class: 'btn primary sm', onclick: () => adInstall(a, byId, reload)}, ic('download'), T('ad.retry')), why, miss.length ? T('ad.needs', {name: miss.map(id => (byId[id] ? byId[id].name : id)).join(', ')}) : null) : null;
      const acts = [retry, url ? h('a', {class: 'btn sm', href: url, target: '_blank', rel: 'noopener noreferrer'}, ic('external-link'), T('ad.open')) : null,
        att ? null : up ? btnIcon('square', T('ad.stop'), () => adAct(a, 'stop', reload)) : btnIcon('play', T('ad.start'), () => adAct(a, 'start', reload)),
        att ? null : btnIcon('refresh-cw', T('ad.restart'), () => adAct(a, 'restart', reload)),
        att ? null : btnIcon('download', T('ad.update'), () => confirmAction({title: T('ad.update_q', {name: a.name}), danger: false, icon: 'download', label: T('ad.update'), body: T('ad.update_b'),
          onConfirm: async () => { const j = await api('POST', '/api/addons/' + a.id + '/update', {}); adJob(j.job, a.name, 'update', reload); }}), a.update_available ? 'primary' : 'ghost'),
        btnIcon('scroll-text', T('ad.logs'), () => adLogs(a)), btnIcon('info', T('ad.details'), () => adDetails(a)),
        a.kind === 'system' ? null : btnIcon('trash', T('ad.remove'), () => adRemove(a, reload))];
      /* the reason of a problem is on the row, not only in the details dialog */
      return [adName(a, ['error', 'unhealthy', 'needs-attention'].includes(a.status) && a.detail ? a.detail : null), adTag(a.status), h('div', {class: 'row'}, acts, retry ? why : null)];
    });
    parts.push(cardFlush(T('ad.installed'), 'container', null, table(th('name', 'state', ''), rows)));
  }
  /* available: not installed yet. An add-on that needs attention is installed, so it is only listed above (with Retry) */
  const cards = r.addons.filter(a => !a.installed).map(a => {
    const miss = missing(a), why = whyLine();
    const can = setOff(h('button', {type: 'button', class: 'btn primary sm', onclick: () => adInstall(a, byId, reload)}, ic('download'), T('ad.install')), why, miss.length ? T('ad.needs', {name: miss.map(id => (byId[id] ? byId[id].name : id)).join(', ')}) : null);
    return h('section', {class: 'acard'}, adName(a), h('p', {'data-d': 'prose'}, a.summary), h('div', {class: 'ft'}, adTag(a.status), can), why);
  });
  if (cards.length) parts.push(h('h2', {class: 'sec'}, ic('plus', 'lg'), T('ad.available')), h('div', {class: 'agrid'}, cards));
  else if (!inst.length) parts.push(h('div', {class: 'empty'}, ic('info'), T('ad.empty')));
  return h('div', {}, parts);
};
/*ENDPAGE*/
/* ---------- Network ---------- */
/*PAGE:network*/
PAGES.network = async () => {
  const r = await api('GET', '/api/network'), reload = () => show('network');
  setPending(r.pending);
  const editHost = h('div', {});
  const parts = [h('div', {class: 'ph'}, h('div', {class: 'row'}, h('h1', {}, T('nav.network')), infoBtn('safe-apply', T('help.safe_apply'))),
    h('div', {class: 'row'}, tip(btn(r.hostname, {icon: 'server', onclick: () => formDialog(T('net.hostname'), [{name: 'hostname', label: T('net.hostname'), value: r.hostname, mono: true}], T('btn.save'), async v => { await api('POST', '/api/hostname', v); toast(T('raid.done')); reload(); }, {icon: 'server'})}), TIP('net.hostname'))))];
  function edit(i) {
    const c = i.config || {}, F = {mode: c.mode || 'dhcp'};
    const inp = (v, ph) => h('input', {class: 'mono', value: v || '', placeholder: ph || '', autocomplete: 'off'});
    const f4 = inp(c.ipv4, '192.168.1.140/24'), g4 = inp(c.gw4, '192.168.1.1'), f6 = inp(c.ipv6, '2001:db8::10/64'), g6 = inp(c.gw6, 'fe80::1'), dns = inp((c.dns || []).join(', '), '192.168.1.1'), sr = inp((c.search || []).join(', '), 'lan');
    const sect = h('div', {}), seg = h('div', {class: 'seg', role: 'group', 'aria-label': T('net.ipv4')});
    const drawMode = () => {
      seg.replaceChildren(...[['dhcp', T('net.dhcp')], ['static', T('net.static')]].map(([k, t]) => h('button', {type: 'button', 'aria-pressed': String(F.mode === k), onclick: () => { F.mode = k; drawMode(); }}, t)));
      sect.replaceChildren(...(F.mode === 'dhcp' ? [] : [h('div', {class: 'grid2'}, field(T('net.address'), f4), field(T('net.gateway'), g4), field(T('net.ipv6'), f6), field(T('net.gateway6'), g6)),
        h('div', {class: 'grid2'}, field(T('net.dns'), tip(dns, TIP('net.dns'))), field(T('net.search'), sr))]));
    };
    drawMode();
    const apply = btn(T('net.apply'), {kind: 'primary', icon: 'undo-2', onclick: () => confirmAction({title: T('net.apply_q'), danger: false, label: T('net.apply_go'), icon: 'undo-2', body: h('p', {}, TIP('net.apply_q')),
      onConfirm: async () => { const res = await api('POST', '/api/network/apply', {ifname: i.ifname, mode: F.mode, ipv4: f4.value.trim(), gateway4: g4.value.trim(), ipv6: f6.value.trim(), gateway6: g6.value.trim(), dns: dns.value, search: sr.value}); toast(T('net.applied')); setPending(res.pending); editHost.replaceChildren(); }})});
    editHost.replaceChildren(card(i.ifname, 'settings', null, h('div', {class: 'f'}, h('span', {class: 'lab'}, T('net.ipv4')), seg), sect, h('div', {class: 'row'}, btn(T('act.discard'), {onclick: () => editHost.replaceChildren()}), apply)));
    editHost.scrollIntoView({behavior: 'smooth'});
  }
  const ifs = r.interfaces.map(i => card(i.ifname, 'network', i.state === 'UP' ? badge('ok', T('state.linkup')) : badge('neutral', T('state.nocable'), 'x'),
    h('dl', {class: 'kv'}, h('dt', {}, T('net.ipv4')), h('dd', {class: 'mono'}, (i.managed ? (i.config.mode === 'static' ? T('net.static') : T('net.dhcp')) + ' ' : '') + (i.addrs.filter(a => a.scope !== 'link').map(a => a.addr + '/' + a.prefix).join(' ') || '-')), h('dt', {}, T('net.mac')), h('dd', {class: 'mono'}, i.mac || '-')),
    i.shadowed_by.length ? tip(badge('warn', i.shadowed_by.length + ' ×', 'triangle-alert'), i.shadowed_by.join(', ')) : null,
    h('div', {class: 'row'}, btn(T('act.edit'), {kind: 'primary', size: 'sm', icon: 'settings', onclick: () => edit(i)}))));
  parts.push(h('div', {class: 'cols'}, h('div', {class: 'stack'}, ifs), editHost));
  return h('div', {}, parts);
};
/*ENDPAGE*/
/* ---------- Users ---------- */
/*PAGE:users*/
PAGES.users = async () => {
  const r = await api('GET', '/api/users'), reload = () => show('users');
  const parts = [h('div', {class: 'ph'}, h('h1', {}, T('nav.users')),
    tip(btn(T('usr.new'), {kind: 'primary', icon: 'plus', onclick: () => formDialog(T('usr.new'), [
      {name: 'username', label: T('usr.name'), mono: true}, {name: 'password', label: T('usr.password'), type: 'password'}, {name: 'groups', label: T('col.groups'), placeholder: 'users, adm'},
      {name: 'sudo', label: T('usr.admin'), type: 'checkbox', value: true}, {name: 'ssh_key', label: T('usr.sshkey'), mono: true}], T('btn.save'), async v => { await api('POST', '/api/users/create', v); toast(T('raid.done')); reload(); }, {icon: 'users'})}), TIP('usr.new')))];
  const rows = r.users.map(u => {
    const acts = [];
    if (!u.root) {
      acts.push(btnIcon('key-round', T('usr.password'), () => formDialog(T('usr.password'), [{name: 'password', label: T('usr.password'), type: 'password'}], T('btn.save'), async v => { await api('POST', '/api/users/password', {username: u.name, password: v.password}); toast(T('raid.done')); }, {icon: 'key-round'})));
      acts.push(btnIcon('file-text', T('usr.sshkey'), () => formDialog(T('usr.sshkey'), [{name: 'ssh_key', label: T('usr.sshkey'), mono: true}], T('btn.save'), async v => { await api('POST', '/api/users/ssh-key', {username: u.name, ssh_key: v.ssh_key}); toast(T('raid.done')); reload(); }, {icon: 'file-text'})));
      acts.push(btnIcon('lock', u.locked ? T('act.unlock') : T('act.lock'), ev => act(ev.currentTarget, async () => { await api('POST', u.locked ? '/api/users/unlock' : '/api/users/lock', {username: u.name}); reload(); })));
      acts.push(btnIcon('trash', T('act.delete'), () => confirmAction({title: T('dlg.delete', {name: u.name}), expected: u.name, label: T('act.delete'), body: TIP('usr.delete'),
        extra: [h('label', {class: 'chk'}, h('input', {type: 'checkbox', id: 'rmhome'}), h('span', {}, T('usr.rmhome')))],
        onConfirm: async () => { await api('POST', '/api/users/delete', {username: u.name, confirm: u.name, remove_home: $('#rmhome').checked}); toast(T('raid.done')); reload(); }}), 'ghost'));
    }
    return [h('b', {}, u.name + (u.name === r.current ? ' *' : '')), u.groups.join(' '), u.ssh_keys,
      h('div', {class: 'row'}, u.root ? badge('warn', 'root', 'lock') : null, u.locked ? badge('err', T('state.locked'), 'lock') : !u.has_password ? badge('warn', T('state.nopw')) : badge('ok', T('state.active')), u.sudo ? tip(badge('accent', 'sudo', 'key-round'), TIP('usr.sudo')) : null), h('div', {class: 'row'}, acts)];
  });
  parts.push(cardFlush(T('nav.users'), 'users', null, table(th('name', 'groups', 'keys', 'state', ''), rows, '', {right: [2]})));
  return h('div', {}, parts);
};
/*ENDPAGE*/
/* ---------- Boot ---------- */
/*PAGE:boot*/
PAGES.boot = async () => {
  const b = await api('GET', '/api/boot'), parts = [];
  if (!b.available) {
    parts.push(h('div', {class: 'ph'}, h('div', {class: 'row'}, h('h1', {}, T('nav.boot')), infoBtn('boot', T('nav.boot')))));
    parts.push(h('div', {class: 'empty'}, ic('info'), T('boot.none')));
    return h('div', {}, parts);
  }
  if (b.error) { parts.push(h('div', {class: 'ph'}, h('h1', {}, T('nav.boot')))); parts.push(banner('err', T('page.error'), b.error)); return h('div', {}, parts); }
  let sel = (b.entries.find(e => e.default) || {}).id || 'debian';
  const dsm = b.entries.find(e => e.id === 'dsm'), area = h('div', {});
  const doSet = (id, reboot) => {
    const e = b.entries.find(x => x.id === id);
    const run = async () => { const res = await api('POST', reboot ? '/api/boot/reboot-to' : '/api/boot/default', {entry: id, confirm: id}); toast(res.message || e.label); if (!reboot) show('boot'); else setTimeout(() => show('boot'), 8000); };
    return confirmAction({title: reboot ? T('boot.reboot_q', {name: e.label}) : T('boot.default_q', {name: e.label}), danger: id === 'agent', expected: e.needs_confirm ? id : null, label: reboot ? T('boot.reboot_go') : T('boot.set_default'), icon: reboot ? 'power' : 'check', body: h('p', {}, e.desc), onConfirm: run});
  };
  function draw() {
    const cur = b.entries.find(e => e.id === sel);
    area.replaceChildren(card(T('boot.entries'), 'panel-top', b.default_valid ? null : badge('err', T('state.invalid'), 'octagon-x'),
      b.entries.map(e => {
        const rd = h('input', {type: 'radio', name: 'be', checked: sel === e.id, disabled: !e.present, onchange: () => { sel = e.id; draw(); }});
        return tip(h('label', {class: 'opt' + (sel === e.id ? ' sel' : '') + (e.present ? '' : ' off')}, rd, h('div', {}, h('b', {}, e.label, e.default ? badge('accent', T('boot.default'), 'flag') : null, e.failsafe ? badge('ok', T('boot.failsafe'), 'shield-check') : null, !e.present ? badge('neutral', T('boot.missing'), 'x') : null))), e.desc);
      }),
      h('div', {class: 'row'}, btn(T('boot.set_default'), {disabled: !cur.present || cur.default, icon: 'check', onclick: () => doSet(sel, false)}), btn(T('boot.reboot_go'), {kind: 'primary', disabled: !cur.present, icon: 'power', onclick: () => doSet(sel, true)}))));
  }
  const rt = dsm && dsm.present ? tip(btn(T('boot.return_dsm'), {kind: 'primary', icon: 'undo-2', size: 'lg', onclick: () => doSet('dsm', true)}), TIP('boot.return_dsm')) : null;
  parts.push(h('div', {class: 'ph'}, h('div', {class: 'row'}, h('h1', {}, T('nav.boot')), infoBtn('boot', T('nav.boot'))), rt));
  if (!b.default_valid) parts.push(banner('err', T('state.invalid'), ''));
  parts.push(area); draw();
  const f = b.flash || {}, p1 = f.p1 || {}, p2 = f.p2 || {};
  const free = p => p.bytes ? tip(usage(p.bytes - (p.free_bytes || 0), p.bytes), fmtBytes(p.free_bytes) + ' ' + T('st.free')) : '-';
  parts.push(h('div', {class: 'cols'}, card(T('boot.flash'), 'hard-drive', chip(f.device || '-'), h('dl', {class: 'kv'}, h('dt', {}, T('boot.part1')), h('dd', {}, free(p1)), h('dt', {}, T('boot.part2')), h('dd', {}, free(p2)))),
    card(T('boot.backup'), 'download', b.backup.exists ? badge('ok', T('boot.exists'), 'check') : badge('warn', T('boot.nobackup'), 'triangle-alert'), tip(h('p', {class: 'mono'}, b.backup.path || '-'), TIP('boot.backup')))));
  return h('div', {}, parts);
};
/*ENDPAGE*/
/* ---------- System ---------- */
const PAGES_ACTIONS = {
  power(action) {
    confirmAction({title: T(action === 'reboot' ? 'sys.reboot_q' : 'sys.shutdown_q'), body: h('p', {}, TIP(action === 'reboot' ? 'sys.reboot' : 'sys.shutdown')), expected: S.host, label: T(action === 'reboot' ? 'sys.reboot' : 'sys.shutdown'), icon: 'power',
      onConfirm: async v => { await api('POST', '/api/system/power', {action, confirm: v}); toast(T('raid.done')); }});
  }
};
/*PAGE:system*/
PAGES.system = async () => {
  const r = await api('GET', '/api/services');
  const rs = r.restartable.map(u => { const b = btn(T('act.restart') + ' ' + u, {size: 'sm', onclick: () => act(b, async () => { await api('POST', '/api/services/restart', {unit: u}); toast(T('raid.done')); })}); return b; });
  const parts = [h('div', {class: 'ph'}, h('h1', {}, T('nav.system')))];
  parts.push(h('div', {class: 'cols'},
    card(T('sys.ssh'), 'key-round', r.sshd.active === 'active' ? badge('ok', T('state.active')) : badge('neutral', r.sshd.active, 'x'), h('dl', {class: 'kv'}, h('dt', {}, T('sys.enabled')), h('dd', {}, r.sshd.enabled), h('dt', {}, T('sys.hostkeys')), h('dd', {}, r.host_keys.map(k => h('div', {class: 'mono'}, k.type + ' ' + k.fingerprint))))),
    card(T('sys.consoles'), 'terminal', null, r.serial_getty.length ? table(th('device', 'state'), r.serial_getty.map(g => [h('span', {class: 'mono'}, g.unit), g.active === 'active' ? badge('ok', T('state.active')) : badge('neutral', g.active, 'x')])) : h('p', {class: 'muted'}, '-'))));
  parts.push(h('div', {class: 'cols'}, card(T('sys.services'), 'refresh-cw', null, tip(h('div', {class: 'row'}, rs), TIP('sys.services'))),
    card(T('sys.power'), 'power', null, h('div', {class: 'row'}, btn(T('sys.reboot'), {icon: 'refresh-cw', onclick: () => PAGES_ACTIONS.power('reboot')}), btn(T('sys.shutdown'), {kind: 'danger', icon: 'power', onclick: () => PAGES_ACTIONS.power('poweroff')})))));
  return h('div', {}, parts);
};
/*ENDPAGE*/
/* ---------- Updates ---------- */
const UPD_ICON = {fail: 'circle-x', ok: 'circle-check', start: 'refresh-cw', pending: 'circle-dashed'};
const UPD_STEPS = ['check', 'download', 'verify', 'stage', 'swap', 'restart', 'health'];
const UPD_STATE_KIND = {ok: 'ok', current: 'ok', refused: 'warn', rolled_back: 'warn', rollback_failed: 'err', failed: 'err', running: 'info', queued: 'info'};
const fmtWhen = t => (t ? new Date(t * 1000).toISOString().slice(0, 16).replace('T', ' ') : '-');
async function rawPost(path, body) {
  const r = await fetch(path, {method: 'POST', credentials: 'same-origin', headers: {'Content-Type': 'application/json', 'X-CSRF-Token': S.csrf || ''}, body: JSON.stringify(body || {})});
  let j = {};
  try { j = await r.json(); } catch (e) { /* not JSON */ }
  return {status: r.status, json: j};
}
/* follow a job: the page polls, survives the service restart (connection errors, then 401 because sessions live in memory) and ends on a result */
function watchJob(jid, host, onEnd, retry) {
  let lost = false;
  let lastJ = null, lastEv = null;
  const draw = (j, events, restarting) => {
    if (j) { lastJ = j; lastEv = events; } else { j = lastJ; events = lastEv; }
    const last = {};
    for (const e of events || []) last[e.step] = e;
    const steps = UPD_STEPS.concat(last.rollback ? ['rollback'] : []);
    const ci = j && j.step ? UPD_STEPS.indexOf(j.step) : -1;
    host.replaceChildren(...[h('ul', {class: 'us', 'aria-live': 'polite'}, steps.map(k => {
      const e = last[k], i = UPD_STEPS.indexOf(k);
      const st = e ? e.status : events ? 'pending' : (i < ci ? 'ok' : i === ci ? 'start' : 'pending');
      const cls = st === 'fail' ? 'fail' : st === 'ok' ? 'ok' : st === 'start' ? 'run' : '';
      const sp = st === 'start' ? 'spin' : '';
      return h('li', {class: cls}, ic(UPD_ICON[st], sp), T('upd.step.' + k), e && e.message ? h('small', {}, e.message) : null);
    })), restarting ? banner('info', T('upd.restarting'), '') : null].filter(Boolean));       // replaceChildren(null) would print the word "null"
  };
  const end = (j, events) => {
    const fail = (events || []).filter(e => e.status === 'fail').pop();
    const kind = j.state === 'ok' || j.state === 'current' ? 'ok' : j.state === 'refused' || j.state === 'rolled_back' ? 'warn' : 'err';
    const text = j.state === 'ok' && j.kind === 'rollback' ? T('upd.rolled', {v: j.to || ''}) : j.state === 'ok' ? T('upd.done', {v: j.to || ''}) : j.state === 'current' ? T('upd.uptodate') : j.state === 'rolled_back' ? T('upd.rolled', {v: j.from || ''})
      : j.state === 'refused' ? T('upd.refused') : j.state === 'rollback_failed' ? T('upd.rbfail') : T('upd.failed');
    if (events) draw(j, events, false); else host.replaceChildren();          // after a restart the session is gone and no events are readable: a list of grey "pending" steps would say nothing true
    const lastOf = {}; for (const e of events || []) lastOf[e.step] = e;
    const shown = fail && lastOf[fail.step] === fail && (UPD_STEPS.includes(fail.step) || fail.step === 'rollback');        // the step list above already prints this message
    host.append(banner(kind, text, fail && !shown ? fail.message : '', btn(lost ? T('upd.signin') : T('upd.reload'), {kind: 'primary', icon: 'refresh-cw', onclick: () => location.reload()}),
      retry && fail && fail.step === 'health' ? btn(T('upd.force'), {icon: 'triangle-alert', onclick: retry}) : null));
    onEnd && onEnd(j);
  };
  const tick = async () => {
    if (S.page !== 'updates' || !host.isConnected) return;
    let j = null, events = null;
    try {
      let r = lost ? null : await rawPost('/api/update/job', {job: jid});
      if (r && r.status === 401) { lost = true; r = null; }
      if (!r) r = await rawPost('/api/update/progress', {job: jid});
      if (r.status === 200 && r.json.job) { j = r.json.job; events = r.json.events || null; }
    } catch (e) { /* the service is restarting */ }
    if (!j) { draw(null, null, true); return setTimeout(tick, 1500); }
    if (!['running', 'queued'].includes(j.state)) return end(j, events);
    draw(j, events, lost || (events || []).some(e => e.step === 'restart' && e.status === 'start'));
    setTimeout(tick, 1200);
  };
  tick();
}
/*PAGE:updates*/
PAGES.updates = async () => {
  const st = await api('GET', '/api/update');
  const F = {channel: st.channel, ref: st.ref, check: st.check};
  const top = h('div', {}), live = h('div', {});
  const mine = () => F.check && F.check.channel === F.channel && (F.channel === 'stable' || F.check.ref === F.ref);
  const running = () => !!st.active || !!st.busy;
  function draw() {
    const c = mine() ? F.check : null;
    const refIn = h('input', {class: 'mono upd-ref', value: F.ref, autocomplete: 'off', spellcheck: 'false', 'aria-label': T('upd.ref'), oninput: () => { F.ref = refIn.value.trim(); }});
    const seg = h('div', {class: 'seg', role: 'group', 'aria-label': T('upd.channel')}, [['stable', 'upd.stable'], ['main', 'upd.main']].map(([k, key]) =>
      tip(h('button', {type: 'button', 'aria-pressed': String(F.channel === k), onclick: () => { F.channel = k; draw(); }}, T(key)), TIP(key))));
    const state = c ? (c.state === 'available' ? badge('accent', T('upd.new'), 'download') : c.state === 'current' ? badge('ok', T('upd.uptodate'), 'check') : c.state === 'older' ? badge('neutral', T('upd.older'), 'clock') : badge('info', T('upd.same'), 'info')) : null;
    const trust = c ? (c.mode === 'signed' ? tip(badge('ok', T('upd.signed'), 'shield-check'), TIP('upd.signed')) : tip(badge('warn', T('upd.unsigned'), 'triangle-alert'), TIP('upd.unsigned'))) : null;
    const chk = btn(T('upd.check'), {icon: 'refresh-cw', onclick: () => act(chk, async () => { const r = await api('POST', '/api/update/check', {channel: F.channel, ref: F.ref}); F.check = r.check; draw(); })});
    const can = c && !running() && !st.stale && !c.blocked && (c.state === 'available' || c.state === 'same');
    const why = whyLine(), go = btn(T('upd.now'), {kind: 'primary', icon: 'download', onclick: () => updateDialog(c)});
    const goWhy = can ? null : !c ? T('upd.nocheck') : c.blocked ? c.blocked : running() ? T('upd.busy') : st.stale ? T('upd.why_reload') : c.state === 'older' ? T('upd.why_older') : T('upd.why_current');
    const rb = st.previous ? btn(T('upd.rollback'), {icon: 'undo-2', onclick: () => rollbackDialog()}) : null;
    const rbWhy = running() ? T('upd.busy') : st.stale ? T('upd.why_reload') : null;
    setOff(go, why, goWhy); if (rb) setOff(rb, why, rbWhy, false);
    if (!goWhy && rbWhy) { why.hidden = false; why.replaceChildren(ic('info', 'sm'), rbWhy); }
    const notes = c && c.notes ? btn(T('upd.notes'), {size: 'sm', kind: 'ghost', icon: 'file-text', onclick: () => dialog(T('upd.notes'), {icon: 'file-text', wide: true, sub: (c.tag || c.commit.slice(0, 7)) + ' ' + (c.published || '')},
      () => h('pre', {class: 'code'}, c.notes), close => [h('button', {type: 'button', class: 'btn primary', onclick: close}, T('btn.close'))])}) : null;
    top.replaceChildren(card(T('upd.plane'), 'download', h('div', {class: 'row'}, state, trust),
      h('dl', {class: 'kv'}, h('dt', {}, T('upd.installed')), h('dd', {class: 'mono'}, st.installed || '-'), h('dt', {}, T('upd.available')), h('dd', {class: 'mono'}, c ? c.version + (c.commit ? ' ' + c.commit.slice(0, 7) : '') : '-'),
        h('dt', {}, T('upd.channel')), h('dd', {}, seg, F.channel === 'main' ? field(T('upd.ref'), refIn) : null)),
      c && c.blocked ? banner('warn', c.blocked, '') : null,
      h('div', {class: 'row'}, tip(chk, TIP('upd.check')), go, rb && !rbWhy ? tip(rb, TIP('upd.rollback')) : rb, notes), why));
  }
  function updateDialog(c, force) {
    const need = c.mode === 'unsigned', typed = c.channel === 'main' ? c.commit.slice(0, 7) : '';
    let box = null, inp = null, ok = null;
    const why = whyLine();
    const gate = () => setOff(ok, why, need && !box.checked ? T('upd.why_consent') : typed && inp.value !== typed ? T('upd.why_typed') : null);
    return dialog(T('upd.confirm_q', {v: c.version}), {icon: 'download', sub: (st.installed || '-') + ' → ' + c.version + (c.commit ? ' (' + c.commit.slice(0, 7) + ')' : '')}, () => h('div', {},
      h('p', {}, T('upd.restart_note')),
      force ? banner('warn', T('upd.force_note'), '') : null,
      need ? banner('warn', T('upd.unsigned'), TIP('upd.unsigned')) : null,
      need ? h('label', {class: 'chk'}, box = h('input', {type: 'checkbox', onchange: gate}), h('span', {}, T('upd.consent'))) : null,
      typed ? field(T('raid.type', {name: typed}), inp = h('input', {class: 'mono', autocomplete: 'off', spellcheck: 'false', oninput: gate})) : null), (close, setErr) => {
      ok = h('button', {type: 'button', class: 'btn primary'}, ic('download'), T('upd.now'));
      ok.addEventListener('click', async () => {
        if (ok.classList.contains('is-loading')) return;
        busy(ok, true); setErr('');
        try { const r = await api('POST', '/api/update/start', {channel: c.channel, ref: c.ref || '', expect: c.channel === 'main' ? c.commit : c.version, accept_unsigned: !!(box && box.checked), force: !!force, confirm: typed}); close(); follow(r.job); }
        catch (e) { setErr(e.message); busy(ok, false); }
      });
      gate();
      return [why, h('button', {type: 'button', class: 'btn', onclick: close}, T('btn.cancel')), ok];
    });
  }
  function rollbackDialog() {
    confirmAction({title: T('upd.rb_q', {v: st.previous}), danger: false, icon: 'undo-2', label: T('upd.rollback'), body: h('p', {}, T('upd.restart_note')),
      onConfirm: async () => { const r = await api('POST', '/api/update/rollback', {}); follow(r.job); }});
  }
  function follow(jid) {
    st.active = jid; draw();
    const body = h('div', {});
    live.replaceChildren(card(T('upd.plane'), 'refresh-cw', null, body));
    watchJob(jid, body, j => { st.active = null; st.stale = j.state === 'ok'; draw(); }, () => F.check && updateDialog(F.check, true));
  }
  draw();
  const hist = cardFlush(T('upd.history'), 'clock', null, table(['upd.when', 'upd.from', 'upd.to', 'upd.source', 'upd.result'].map(k => T(k)),
    st.jobs.map(j => [fmtWhen(j.started), h('span', {class: 'mono'}, j.from || '-'), h('span', {class: 'mono'}, j.to || '-'),
      tip(h('span', {}, T('upd.src.' + j.source) + (j.by && j.by !== '-' ? ' · ' + j.by : '')), j.ref || j.commit ? (j.ref || '') + ' ' + (j.commit || '').slice(0, 7) : null),
      h('div', {class: 'hres'}, badge(UPD_STATE_KIND[j.state] || 'neutral', T('upd.s.' + j.state)),
        !['ok', 'current', 'running', 'queued'].includes(j.state) && j.message ? h('small', {}, j.message) : null)]), T('upd.none')));            // why it failed is text in the row, not a tooltip
  const page = h('div', {}, h('div', {class: 'ph'}, h('div', {class: 'row'}, h('h1', {}, T('nav.updates')), infoBtn('updates', T('nav.updates')))), top, live, hist);
  if (st.active) follow(st.active);
  return page;
};
/*ENDPAGE*/
/* ---------- About ---------- */
const LIC = /*__LIC__*/null;
const LINKS = {docs: 'https://github.com/andreaswiren/synofurb/tree/main/docs', src: 'https://github.com/andreaswiren/synofurb', issue: 'https://github.com/andreaswiren/synofurb/issues/new'};
/*PAGE:about*/
PAGES.about = async () => {
  const [me, st, sv] = await Promise.all([api('GET', '/api/me'), api('GET', '/api/status'), api('GET', '/api/services')]);
  const diag = () => JSON.stringify({version: me.version, host: me.hostname, os: st.os, kernel: st.kernel, arch: st.arch, unit: (st.unit || {}).model || null, ssh: sv.sshd.active, theme: effTheme(), page: 'about'}, null, 1);
  const copy = btn(T('about.copy'), {kind: 'primary', icon: 'clipboard-copy', onclick: async () => {
    try { await navigator.clipboard.writeText(diag()); toast(T('about.copied')); }
    catch (e) { const ta = h('textarea', {class: 'mono', rows: 8, readonly: true}, diag()); dialog(T('about.copy'), {icon: 'clipboard-copy'}, () => ta, close => [h('button', {type: 'button', class: 'btn primary', onclick: close}, T('btn.close'))]); ta.select(); }
  }});
  const lic = (id, name) => btn(T('about.licence'), {size: 'sm', kind: 'ghost', icon: 'scroll-text', onclick: () => dialog(name, {icon: 'scroll-text', wide: true}, () => h('pre', {class: 'code lic'}, LIC[id]), close => [h('button', {type: 'button', class: 'btn primary', onclick: close}, T('btn.close'))])});
  const link = (k, key, icon) => h('a', {class: 'btn', href: LINKS[k], target: '_blank', rel: 'noopener noreferrer'}, ic(icon), T(key));
  const credits = [['Debian', 'DFSG', null], ['Lucide', 'ISC', 'Lucide'], ['Inter', 'OFL 1.1', 'Inter'], ['JetBrains Mono', 'OFL 1.1', 'JetBrains Mono'], ['Montserrat', 'OFL 1.1', null]];
  return h('div', {}, h('div', {class: 'ph'}, h('h1', {}, T('nav.about'))),
    h('section', {class: 'card'}, h('div', {class: 'cb about'}, logo('wordmark'), h('div', {class: 'row'}, chip(T('about.version'), me.version), chip(me.hostname)), h('div', {class: 'row'}, tip(copy, TIP('about.copy')), link('docs', 'about.docs', 'book-open'), link('src', 'about.source', 'external-link'), link('issue', 'about.issue', 'external-link')))),
    h('details', {class: 'card'}, h('summary', {class: 'cb'}, T('about.credits')), h('ul', {class: 'cred'}, credits.map(([n, l, k]) => h('li', {}, h('b', {}, n), h('span', {class: 'chip'}, l), k ? lic(k, n) : null)),
      h('li', {}, popover(T('about.synodeb'), TIP('about.synodeb'), 'info')))));
};
/*ENDPAGE*/
/* ---------- copy, tooltips, menus, help (short / tip / help; word budgets are enforced by the tests) ---------- */
const COPY = /*__COPY__*/null;
const HELP = /*__HELP__*/null;
const T = (k, vars) => { if (S.used) S.used.add(k); const e = COPY[k]; let s = e ? e.short : k; if (vars) for (const [a, b] of Object.entries(vars)) s = s.replace('{' + a + '}', b); return s; };
const TIP = k => (COPY[k] && COPY[k].tip) || null;
let tipEl = null;
function tip(el, text) {
  if (!text) return el;
  if (!tipEl) { tipEl = h('div', {id: 'tip', role: 'tooltip', hidden: true}); document.body.append(tipEl); }
  const show = () => {
    tipEl.textContent = typeof text === 'function' ? text() : text; tipEl.hidden = false; el.setAttribute('aria-describedby', 'tip');
    const r = el.getBoundingClientRect(), w = tipEl.offsetWidth, hh = tipEl.offsetHeight;
    let x = Math.min(Math.max(8, r.left + r.width / 2 - w / 2), innerWidth - w - 8), y = r.bottom + 6;
    if (y + hh > innerHeight - 8) y = Math.max(8, r.top - hh - 6);
    tipEl.style.left = x + 'px'; tipEl.style.top = y + 'px';
  };
  const hide = () => { tipEl.hidden = true; el.removeAttribute('aria-describedby'); };
  el.addEventListener('mouseenter', show); el.addEventListener('focus', show);
  el.addEventListener('mouseleave', hide); el.addEventListener('blur', hide);
  el.addEventListener('keydown', e => { if (e.key === 'Escape') hide(); });
  if (!/^(BUTTON|A|INPUT|SELECT)$/.test(el.tagName) && !el.hasAttribute('tabindex')) el.setAttribute('tabindex', '0');
  return el;
}
function helpDialog(id) {
  const t = HELP.find(x => x.id === id) || HELP[0];
  return dialog(t.title, {icon: 'info'}, () => h('div', {class: 'helpbody'}, t.body.split('\n\n').map(p => h('p', {}, p))), close => [
    h('button', {type: 'button', class: 'btn', onclick: () => { close(); show('help'); }}, T('nav.help')), h('button', {type: 'button', class: 'btn primary', onclick: close}, T('btn.close'))]);
}
const infoBtn = (id, label) => tip(h('button', {type: 'button', class: 'btn ghost icon-btn sm', 'aria-label': T('help.about') + ' ' + (label || id), onclick: () => helpDialog(id)}, ic('info')), T('help.about') + ' ' + (label || id));
function menu(label, items, o = {}) {
  const b = h('button', {type: 'button', class: 'btn ' + (o.kind || ''), 'aria-haspopup': 'menu', 'aria-expanded': 'false'}, label, ic('chevron-down', 'sm'));
  const list = h('div', {class: 'menu', role: 'menu', hidden: true});
  const mis = items.filter(Boolean).map(it => {
    const mi = h('button', {type: 'button', role: 'menuitem', tabindex: '-1', class: 'mi' + (it.danger ? ' danger' : ''), 'aria-disabled': it.off ? 'true' : null,
      onclick: () => { if (it.off) return; close(); it.onclick(); }}, it.icon ? ic(it.icon) : null, it.label);
    if (it.tip) tip(mi, it.tip);
    list.append(mi); return mi;
  });
  const wrap = h('div', {class: 'menuwrap'}, b, list);
  function open() { list.hidden = false; b.setAttribute('aria-expanded', 'true'); mis[0] && mis[0].focus(); setTimeout(() => document.addEventListener('click', away), 0); }
  function close(refocus) { list.hidden = true; b.setAttribute('aria-expanded', 'false'); document.removeEventListener('click', away); if (refocus) b.focus(); }
  function away(e) { if (!wrap.contains(e.target)) close(); }
  b.addEventListener('click', () => (list.hidden ? open() : close()));
  b.addEventListener('keydown', e => { if (e.key === 'ArrowDown' && list.hidden) { e.preventDefault(); open(); } });
  list.addEventListener('keydown', e => {
    const i = mis.indexOf(document.activeElement);
    if (e.key === 'ArrowDown') { e.preventDefault(); mis[(i + 1) % mis.length].focus(); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); mis[(i - 1 + mis.length) % mis.length].focus(); }
    else if (e.key === 'Home') { e.preventDefault(); mis[0].focus(); }
    else if (e.key === 'End') { e.preventDefault(); mis[mis.length - 1].focus(); }
    else if (e.key === 'Escape') { e.preventDefault(); close(true); }
    else if (e.key === 'Tab') close();
  });
  return wrap;
}
/* ---------- SHR slice algorithm (same as shr_slices() in the server; vectors in tests/shr_vectors.json) ---------- */
const GIBN = 1073741824, MIBN = 1048576;
function shrSlices(disks, mode) {   // disks: {name: dataBytes}
  const names = Object.keys(disks), min = mode === 'shr1' ? 2 : 4;
  if (names.length < min) return null;
  const sorted = names.map((n, i) => [n, disks[n], i]).sort((a, b) => a[1] - b[1] || a[2] - b[2]);
  const tiers = []; let low = null;
  for (const [n, s] of sorted) {
    if (low === null || s - low >= GIBN) { low = s; tiers.push([Math.floor(s / GIBN) * GIBN, [n]]); } else tiers[tiers.length - 1][1].push(n);
  }
  const slices = []; let prev = 0, total = 0;
  tiers.forEach(([t], j) => {
    const length = t - prev; prev = t;
    if (length <= 0) return;
    const set = new Set(tiers.slice(j).flatMap(x => x[1])), members = names.filter(n => set.has(n)), m = members.length;
    let level = null, usable = 0, warn = false;
    if (m === 1) level = null;
    else if (mode === 'shr1') { level = m === 2 ? 'raid1' : 'raid5'; usable = m === 2 ? length : (m - 1) * length; }
    else if (m >= 4) { level = 'raid6'; usable = (m - 2) * length; }
    else { level = 'raid1'; usable = length; warn = true; }
    slices.push({index: slices.length + 1, start: t - length, length, members, level, usable, unusable: m === 1, warn});
    total += usable;
  });
  return {mode, slices, usable: total};
}
function levelPreview(level, sizes) {   // -> {usable, tolerance, ok, why}
  const n = sizes.length, mn = Math.min(...sizes);
  if (level === 'shr1' || level === 'shr2') {
    const d = {}; sizes.forEach((s, i) => { d['d' + i] = Math.max(0, s - 2 * MIBN); });
    const r = shrSlices(d, level);
    return r ? {usable: r.usable, tolerance: level === 'shr1' ? 1 : 2, ok: r.usable > 0, shr: r} : {ok: false, why: level === 'shr1' ? T('raid.need2') : T('raid.need4')};
  }
  const need = {raid0: 2, raid1: 2, raid5: 3, raid6: 4, raid10: 4}[level];
  if (n < need) return {ok: false, why: 'Needs ' + need + ' disks'};
  if (level === 'raid10' && n % 2) return {ok: false, why: T('raid.needeven')};
  const u = {raid0: mn * n, raid1: mn, raid5: mn * (n - 1), raid6: mn * (n - 2), raid10: Math.floor(mn * n / 2)}[level];
  return {ok: true, usable: u, tolerance: {raid0: 0, raid1: n - 1, raid5: 1, raid6: 2, raid10: 1}[level]};
}
/* ---------- plan -> typed confirm -> run (shared by every RAID operation) ---------- */
function planView(plan, o) {   // returns {node, ready()} ; o.foreign shows the acknowledgement
  const ack = plan.needs_foreign_ack ? h('input', {type: 'checkbox', id: 'fack'}) : null;
  const typed = plan.confirm ? h('input', {class: 'mono', autocomplete: 'off', spellcheck: 'false', 'aria-label': T('raid.type', {name: plan.confirm}), placeholder: plan.confirm}) : null;
  const node = h('div', {}, plan.warnings.map(w => banner('warn', w, '')),
    h('div', {class: 'cap'}, T('raid.commands')), h('pre', {class: 'code'}, plan.steps.map((s, i) => (i + 1) + '  ' + s.cmd).join('\n')),
    plan.follow_ups.length ? [h('div', {class: 'cap'}, T('raid.later')), h('pre', {class: 'code'}, plan.follow_ups.map(s => s.cmd).join('\n'))] : null,
    ack ? h('label', {class: 'chk'}, ack, h('span', {}, T('raid.ack'))) : null,
    typed ? field(T('raid.type', {name: plan.confirm}), typed) : null);
  return {node, typed, ack, ready: () => (!typed || typed.value === plan.confirm) && (!ack || ack.checked)};
}
function raidFlow(o) {   // o: {title, icon, op, fields, build(values)->body, runLabel, direct, onDone}
  const dlg = h('dialog', {'aria-labelledby': 'rf-t'}), inputs = {};
  const err = h('div', {class: 'err', role: 'alert', hidden: true});
  const body = h('div', {class: 'db'}), foot = h('div', {class: 'df'});
  dlg.append(h('div', {class: 'dh'}, ic(o.icon || 'layers'), h('div', {}, h('h3', {id: 'rf-t'}, o.title))), body, foot);
  const setErr = m => { err.hidden = !m; err.replaceChildren(ic('octagon-x'), m || ''); };
  let done = false;
  const close = () => { dlg.close(); };
  dlg.addEventListener('close', () => { dlg.remove(); if (done) (o.onDone || (() => show('raid')))(); });
  function form() {
    body.replaceChildren(...(o.fields || []).map(f => {
      let inp;
      if (f.type === 'select') inp = h('select', {name: f.name}, f.options.map(x => h('option', {value: x.v ?? x, selected: (x.v ?? x) === f.value}, x.t ?? x)));
      else if (f.type === 'checks') {
        inp = h('div', {role: 'group', 'aria-label': f.label}, f.options.length ? f.options.map(x => { const c = h('input', {type: 'checkbox', value: x.v}); return h('label', {class: 'chk'}, c, h('span', {}, x.t)); }) : h('p', {class: 'muted'}, T('raid.nofree')));
        inputs[f.name] = {checks: true, el: inp}; return h('div', {class: 'f'}, h('span', {class: 'lab'}, f.label), inp);
      } else if (f.type === 'checkbox') inp = h('input', {type: 'checkbox', name: f.name, checked: !!f.value});
      else inp = h('input', {name: f.name, type: f.type || 'text', value: f.value ?? '', class: f.mono ? 'mono' : '', autocomplete: 'off'});
      inputs[f.name] = inp;
      return f.type === 'checkbox' ? h('label', {class: 'chk'}, inp, h('span', {}, f.label)) : field(f.label, inp, null);
    }), err);
    foot.replaceChildren(h('button', {type: 'button', class: 'btn', onclick: close}, T('btn.cancel')),
      h('button', {type: 'button', class: 'btn primary', onclick: ev => act(ev.currentTarget, preview)}, o.direct ? o.runLabel || T('btn.run') : T('btn.preview')));
  }
  const values = () => { const v = {}; for (const [k, i] of Object.entries(inputs)) v[k] = i.checks ? [...i.el.querySelectorAll('input:checked')].map(x => x.value) : i.type === 'checkbox' ? i.checked : i.value; return v; };
  async function preview() {
    setErr('');
    const b = Object.assign({op: o.op}, o.build(values()));
    if (o.direct) { await run(b, null); return; }
    let plan; try { plan = await api('POST', '/api/raid/plan', b); } catch (e) { setErr(e.message); return; }
    const pv = planView(plan);
    body.replaceChildren(pv.node, err);
    const go = h('button', {type: 'button', class: 'btn ' + (plan.confirm ? 'danger' : 'primary'), disabled: true, onclick: ev => act(ev.currentTarget, () => run(b, pv))}, plan.confirm ? [ic('trash'), o.runLabel || T('btn.run')] : (o.runLabel || T('btn.run')));
    const upd = () => { go.disabled = !pv.ready(); };
    if (pv.typed) pv.typed.addEventListener('input', upd); if (pv.ack) pv.ack.addEventListener('change', upd); upd();
    foot.replaceChildren(h('button', {type: 'button', class: 'btn', onclick: form}, T('btn.back')), go);
  }
  async function run(b, pv) {
    const send = Object.assign({}, b, pv ? {confirm: pv.typed ? pv.typed.value : undefined, foreign_ack: pv.ack ? pv.ack.checked : undefined} : {});
    let r;
    try { r = await api('POST', '/api/raid/' + o.op, send); } catch (e) { if (e.data && e.data.results) r = e.data; else { setErr(e.message); return; } }
    done = true;
    const ok = r.ok !== false;
    body.replaceChildren(banner(ok ? 'ok' : 'err', ok ? T('raid.done') : T('raid.failed'), ''), logView(r.results || []),
      r.follow_ups && r.follow_ups.length ? [h('div', {class: 'cap'}, T('raid.later')), h('pre', {class: 'code'}, r.follow_ups.map(s => s.cmd).join('\n'))] : null);
    foot.replaceChildren(h('button', {type: 'button', class: 'btn primary', onclick: close}, T('btn.close')));
  }
  document.body.append(dlg); dlg.showModal(); form();
  if (o.direct && !(o.fields || []).length) preview();
  const f = dlg.querySelector('input,select'); if (f) f.focus();
  return dlg;
}
/* ---------- RAID page ---------- */
const LEVEL_NAME = {raid0: 'RAID 0', raid1: 'RAID 1', raid4: 'RAID 4', raid5: 'RAID 5', raid6: 'RAID 6', raid10: 'RAID 10', linear: 'Linear'};
function arrayState(a) {
  if (a.sync) {
    const k = {recovery: 'rebuilding', recover: 'rebuilding', resync: 'resyncing', reshape: 'reshaping', check: 'checking', repair: 'repairing'}[a.sync.action] || 'resyncing';
    return [k === 'checking' || k === 'repairing' ? 'info' : 'warn', k, a.sync.action === 'check' ? 'search' : 'refresh-cw'];
  }
  if (a.inactive) return ['neutral', 'inactive', 'x'];
  if (a.degraded) return ['err', 'degraded', 'triangle-alert'];
  return ['ok', 'clean', 'check'];
}
function memberChip(mb) {
  const st = mb.state, icon = {active: 'hard-drive', spare: 'plus', faulty: 'octagon-x', rebuilding: 'refresh-cw', removed: 'x', replacement: 'refresh-cw'}[st] || 'hard-drive';
  const c = h('span', {class: 'mchip ' + st}, ic(icon, 'sm'), h('span', {class: 'mono'}, mb.name || ('slot ' + mb.slot)), st === 'active' ? null : h('b', {}, T('ms.' + st)));
  const info = [mb.model, mb.serial, mb.size ? fmtBytes(mb.size) : null, T('ms.' + st)].filter(Boolean).join(' - ');
  return tip(c, info);
}
function freeOptions(R, minBytes) {
  return R.candidates.map(c => ({v: c.path, t: c.path + '  ' + fmtBytes(c.size) + (c.model ? '  ' + c.model : ''), size: c.size})).filter(x => !minBytes || x.size >= minBytes);
}
function arrayCard(a, R) {
  const [kind, stateKey, icon] = arrayState(a), ms = Math.min(...a.members.map(m => m.size || Infinity), Infinity);
  const dis = (cond, why) => cond ? {off: true, tip: why} : {};
  const supported = ['raid1', 'raid5', 'raid6'].includes(a.level), canLevel = ['raid0', 'raid1', 'raid5', 'raid6'].includes(a.level);
  const sel = (label, name, opts) => ({type: 'select', name, label, options: opts.length ? opts : [{v: '', t: '-'}]});
  const free = freeOptions(R, isFinite(ms) ? ms : 0);
  const base = {array: a.name, foreign: a.foreign};
  const flow = (op, title, icon2, fields, build, runLabel) => () => raidFlow({title, icon: icon2, op, fields, build: v => Object.assign({array: a.name}, build(v)), runLabel});
  const memOpts = a.members.filter(m => m.device).map(m => ({v: m.device, t: m.device + (m.state !== 'active' ? ' (' + T('ms.' + m.state) + ')' : '')}));
  const items = [
    Object.assign({label: T('act.add_spare'), icon: 'plus', onclick: flow('add-spare', T('act.add_spare') + ' ' + a.name, 'plus', [sel(T('raid.disk'), 'device', free)], v => ({device: v.device}))}, dis(!free.length, T('raid.nofree'))),
    Object.assign({label: T('act.replace'), icon: 'refresh-cw', onclick: flow('replace', T('act.replace') + ' ' + a.name, 'refresh-cw', [sel(T('raid.old'), 'old', memOpts), sel(T('raid.new'), 'new', free)], v => ({old: v.old, new: v.new}))},
      dis(a.level === 'raid0' || !free.length, a.level === 'raid0' ? T('raid.noredundancy') : T('raid.nofree'))),
    Object.assign({label: T('act.grow'), icon: 'arrow-right', onclick: flow('grow', T('act.grow') + ' ' + a.name, 'arrow-right', [{type: 'checks', name: 'add', label: T('raid.disks'), options: free}, {type: 'checkbox', name: 'size_max', label: T('raid.allspace')}], v => ({add: v.add, size_max: v.size_max}))},
      dis(!supported, T('raid.cantgrow'))),
    Object.assign({label: T('act.level'), icon: 'layers', onclick: flow('change-level', T('act.level') + ' ' + a.name, 'layers', [sel(T('raid.target'), 'target', {raid1: ['raid5'], raid5: ['raid6'], raid6: ['raid5'], raid0: ['raid5']}[a.level] || []), {type: 'checks', name: 'add', label: T('raid.disks'), options: free}], v => ({target: v.target, add: v.add}))},
      dis(!canLevel, T('raid.cantlevel'))),
    a.busy ? {label: T('act.stoptask'), icon: 'x', onclick: () => api('POST', '/api/raid/scrub', {array: a.name, action: 'idle'}).then(() => { toast(T('raid.done')); show('raid'); }).catch(e => toast(e.message, true))} :
      {label: T('act.check'), icon: 'search', onclick: () => api('POST', '/api/raid/scrub', {array: a.name, action: 'check'}).then(() => { toast(T('raid.started')); show('raid'); }).catch(e => toast(e.message, true)),
        off: a.level === 'raid0' || a.inactive, tip: a.level === 'raid0' ? T('raid.noredundancy') : null},
    a.busy ? null : Object.assign({label: T('act.repair'), icon: 'wrench', onclick: flow('scrub', T('act.repair') + ' ' + a.name, 'wrench', [], () => ({action: 'repair'}), T('act.repair'))}, dis(a.level === 'raid0' || a.degraded, T('raid.norepair'))),
    {label: T('act.tune'), icon: 'settings', onclick: () => tuneDialog(a, R)},
    Object.assign({label: T('act.stop'), icon: 'power', danger: true, onclick: flow('stop', T('act.stop') + ' ' + a.name, 'power', [], () => ({}), T('act.stop'))}, dis(a.in_use || a.busy || a.holds_system, T('raid.inuse'))),
  ];
  const cap = h('div', {class: 'kvrow'}, h('span', {class: 'muted'}, T('raid.size')), h('b', {}, fmtBytes(a.size)),
    a.mismatch_cnt ? tip(badge('warn', T('raid.mismatch', {n: a.mismatch_cnt}), 'triangle-alert'), TIP('raid.mismatch')) : null,
    a.bitmap ? tip(chip(T('raid.bitmap')), TIP('raid.bitmap')) : null);
  const prog = a.sync && a.sync.percent != null ? h('div', {class: 'pr big'}, bar(a.sync.percent, 100, kind === 'warn' ? 'warn' : ''), h('span', {class: 'num'}, [a.sync.percent + ' %', a.sync.speed_kbs ? Math.round(a.sync.speed_kbs / 1024) + ' MB/s' : null, a.sync.finish_min ? fmtDur(a.sync.finish_min * 60) : null].filter(Boolean).join(' - '))) : null;
  return h('section', {class: 'card arr' + (a.degraded ? ' bad' : ''), 'aria-label': a.name}, h('div', {class: 'ch'},
    h('h2', {}, ic('layers', 'lg'), h('span', {class: 'mono'}, a.name), chip(LEVEL_NAME[a.level] || a.level || '?'), tip(badge(kind, T('state.' + stateKey), icon), TIP('state.' + stateKey)),
      a.foreign ? tip(badge('accent', T('raid.imported'), 'package'), TIP('raid.imported')) : null),
    menu(T('raid.actions'), items)),
    h('div', {class: 'cb'}, cap, prog, h('div', {class: 'chips', role: 'list', 'aria-label': T('raid.members')}, a.members.map(mb => h('span', {role: 'listitem'}, memberChip(mb)))),
      a.mounts.length || a.lvs.length ? h('div', {class: 'kvrow'}, h('span', {class: 'muted'}, T('raid.usedby')), h('span', {class: 'mono'}, a.lvs.concat(a.mounts).join('  '))) : null));
}
function tuneDialog(a, R) {
  raidFlow({title: T('act.tune') + ' ' + a.name, icon: 'settings', op: 'tuning', direct: true, runLabel: T('btn.save'), fields: [
    {name: 'speed_min', label: T('raid.speedmin'), value: a.sync_speed_min ?? '', mono: true}, {name: 'speed_max', label: T('raid.speedmax'), value: a.sync_speed_max ?? '', mono: true}],
    build: v => ({array: a.name, speed_min: /^\d+$/.test(v.speed_min) ? Number(v.speed_min) : (v.speed_min || null), speed_max: /^\d+$/.test(v.speed_max) ? Number(v.speed_max) : (v.speed_max || null)})});
}
function shrVolumeCard(vol, R) {
  const arrs = R.arrays.filter(a => new RegExp('^' + vol + '_\\d+$').test(a.label) && a.vgs.includes('vg_' + vol)).sort((x, y) => x.label.localeCompare(y.label, undefined, {numeric: true}));
  const disks = [...new Set(arrs.flatMap(a => a.members.map(m => m.disk)))].filter(Boolean);
  const busy = arrs.some(a => a.busy || a.degraded);
  const diskOpts = disks.map(d => ({v: '/dev/' + d, t: '/dev/' + d}));
  const wholeDisks = R.candidates.filter(c => c.type === 'disk').map(c => ({v: c.path, t: c.path + '  ' + fmtBytes(c.size)}));
  const addB = h('button', {type: 'button', class: 'btn sm', onclick: () => raidFlow({title: T('shr.add') + ' ' + vol, op: 'shr-add-disk', icon: 'plus', fields: [{type: 'select', name: 'device', label: T('raid.disk'), options: wholeDisks.length ? wholeDisks : [{v: '', t: '-'}]}], build: v => ({volume: vol, device: v.device})})}, ic('plus'), T('shr.add'));
  const repB = h('button', {type: 'button', class: 'btn sm', onclick: () => raidFlow({title: T('shr.replace') + ' ' + vol, op: 'shr-replace-disk', icon: 'refresh-cw', fields: [{type: 'select', name: 'old', label: T('raid.old'), options: diskOpts}, {type: 'select', name: 'device', label: T('raid.new'), options: wholeDisks.length ? wholeDisks : [{v: '', t: '-'}]}], build: v => ({volume: vol, old: v.old, device: v.device})})}, ic('refresh-cw'), T('shr.replace'));
  if (busy) { addB.setAttribute('aria-disabled', 'true'); repB.setAttribute('aria-disabled', 'true'); tip(addB, T('raid.busy')); tip(repB, T('raid.busy')); }
  const mode = arrs.some(a => a.level === 'raid6') ? 'Hybrid 2' : 'Hybrid 1';
  return h('section', {class: 'card'}, h('div', {class: 'ch'}, h('h2', {}, ic('database', 'lg'), vol, chip(mode)), h('div', {class: 'row'}, addB, repB)),
    h('div', {class: 'cb flush'}, table([T('shr.slice'), T('raid.members'), T('raid.level'), T('raid.size')], arrs.map((a, i) => [i + 1, h('span', {class: 'mono'}, a.members.map(m => m.disk).join(' ')), LEVEL_NAME[a.level] || a.level, fmtBytes(a.size)]), '', {right: [3]})));
}
PAGES.raid = async () => {
  const R = await api('GET', '/api/raid'), reload = () => show('raid');
  const bad = R.arrays.filter(a => a.degraded).length, busyN = R.arrays.filter(a => a.sync).length;
  S.alerts = {raid: bad + busyN}; renderNav();
  const parts = [h('div', {class: 'ph'}, h('div', {class: 'row'}, h('h1', {}, T('raid.title')), infoBtn('raid-levels', T('raid.title'))), h('div', {class: 'row'},
    (() => { const t = h('label', {class: 'chk sw'}, h('input', {type: 'checkbox', checked: R.scrub_timer.enabled === 'enabled', onchange: ev => api('POST', '/api/raid/tuning', {scrub_schedule: ev.target.checked ? 'enable' : 'disable'}).then(() => toast(T('raid.done'))).catch(e => { toast(e.message, true); ev.target.checked = !ev.target.checked; })}), h('span', {}, T('raid.monthly'))); return tip(t, TIP('raid.monthly')); })(),
    btn(T('raid.create'), {kind: 'primary', icon: 'plus', disabled: !R.available, onclick: () => { wiz.hidden = false; wiz.scrollIntoView({behavior: 'smooth'}); }})))];
  if (!R.available) parts.push(banner('warn', 'mdadm', 'apt install mdadm'));
  const wiz = h('div', {id: 'rwiz', hidden: !R.arrays.length ? false : true}); parts.push(wiz);
  parts.push(h('h2', {class: 'sec'}, T('raid.arrays'), tip(h('span', {class: 'badge neutral'}, String(R.arrays.length)), null)));
  parts.push(R.arrays.length ? h('div', {class: 'stack'}, R.arrays.map(a => arrayCard(a, R))) : h('div', {class: 'empty'}, ic('info'), T('raid.empty')));
  if (R.shr_volumes.length) { parts.push(h('h2', {class: 'sec'}, T('raid.hybrid'), infoBtn('shr', T('raid.hybrid')))); parts.push(h('div', {class: 'stack'}, R.shr_volumes.map(v => shrVolumeCard(v, R)))); }
  raidWizard(wiz, R);
  return h('div', {}, parts);
};
function raidWizard(host, R) {
  const W = {step: 1, devs: new Set(), level: null, name: '', vol: false, fs: 'xfs', plan: null, perr: ''};
  const sizeOf = p => (R.candidates.find(c => c.path === p) || {}).size || 0;
  const sizes = () => [...W.devs].map(sizeOf);
  const levels = ['raid1', 'raid5', 'raid6', 'raid10', 'raid0', 'shr1', 'shr2'];
  const stage = h('div', {class: 'cb'});
  const isShr = () => W.level && W.level.startsWith('shr');
  const body = () => isShr() ? {op: 'shr-create', name: W.name, mode: W.level, devices: [...W.devs], fs: W.fs} :
    {op: 'create', name: W.name, level: W.level, devices: [...W.devs], volume: W.vol ? {fs: W.fs} : null};
  async function plan() { try { W.plan = await api('POST', '/api/raid/plan', body()); W.perr = ''; } catch (e) { W.plan = null; W.perr = e.message; } draw(); }
  function stepsBar() { return h('div', {class: 'steps', role: 'list'}, [T('wiz.disks'), T('wiz.level'), T('wiz.review')].map((l, i) => h('div', {role: 'listitem', class: i + 1 === W.step ? 'cur' : i + 1 < W.step ? 'done' : '', 'aria-current': i + 1 === W.step ? 'step' : null}, h('i', {}, i + 1 < W.step ? '✓' : String(i + 1)), h('span', {}, l)))); }
  function nav(canNext, onNext, label) {
    return h('div', {class: 'row'}, W.step > 1 ? btn(T('btn.back'), {icon: 'arrow-left', onclick: () => { W.step--; draw(); }}) : null, btn(T('btn.cancel'), {onclick: () => { host.hidden = !!R.arrays.length; W.step = 1; W.devs.clear(); draw(); }}),
      btn(label || T('btn.next'), {kind: 'primary', disabled: !canNext, onclick: onNext || (() => { W.step++; if (W.step === 3) plan(); else draw(); })}));
  }
  function draw() {
    let c;
    if (W.step === 1) {
      c = [R.candidates.length ? R.candidates.map(d => { const cb = h('input', {type: 'checkbox', checked: W.devs.has(d.path), onchange: () => { cb.checked ? W.devs.add(d.path) : W.devs.delete(d.path); draw(); }});
        return h('label', {class: 'opt' + (W.devs.has(d.path) ? ' sel' : '')}, cb, h('div', {}, h('b', {}, h('span', {class: 'mono'}, d.path), chip(fmtBytes(d.size)), d.tran ? chip(d.tran) : null), h('small', {}, [d.model, d.serial].filter(Boolean).join(' - ')))); }) :
        h('div', {class: 'empty'}, ic('info'), T('raid.nofree')), nav(W.devs.size > 0)];
    } else if (W.step === 2) {
      c = [h('div', {class: 'lvls', role: 'radiogroup', 'aria-label': T('wiz.level')}, levels.map(l => {
        const p = levelPreview(l, sizes()), r = h('input', {type: 'radio', name: 'lvl', checked: W.level === l, disabled: !p.ok, onchange: () => { W.level = l; draw(); }});
        const n = T('lv.' + l);
        const lab = h('label', {class: 'opt lvl' + (W.level === l ? ' sel' : '') + (p.ok ? '' : ' off')}, r, h('div', {}, h('b', {}, n, infoBtnInline(l)),
          p.ok ? h('div', {class: 'row'}, chip(T('raid.usable') + ' ' + fmtBytes(p.usable)), chip(p.tolerance ? T('raid.survives', {n: p.tolerance}) : T('raid.none'))) : h('small', {}, p.why)));
        return tip(lab, TIP('lv.' + l));
      })),
      W.level && levelPreview(W.level, sizes()).shr ? shrTable(levelPreview(W.level, sizes()).shr) : null, nav(W.level && levelPreview(W.level, sizes()).ok)];
    } else {
      const nm = h('input', {class: 'mono', value: W.name, placeholder: 'media', autocomplete: 'off', 'aria-label': T('raid.name'), oninput: () => { W.name = nm.value.trim(); clearTimeout(W.t); W.t = setTimeout(plan, 350); }});
      const fs = h('select', {'aria-label': T('raid.fs'), onchange: () => { W.fs = fs.value; plan(); }}, ['xfs', 'ext4', 'btrfs'].map(x => h('option', {value: x, selected: x === W.fs}, x)));
      const vol = !isShr() ? h('label', {class: 'chk'}, h('input', {type: 'checkbox', checked: W.vol, onchange: ev => { W.vol = ev.target.checked; plan(); }}), h('span', {}, T('raid.withvol'))) : null;
      const pv = W.plan ? planView(W.plan) : null;
      const go = btn(T('raid.createrun'), {kind: 'danger', icon: 'trash', disabled: true});
      const upd = () => { go.disabled = !(pv && pv.ready()); };
      if (pv) { if (pv.typed) pv.typed.addEventListener('input', upd); }
      go.addEventListener('click', () => act(go, async () => {
        const b = body(); b.confirm = pv.typed.value;
        let r; try { r = await api('POST', '/api/raid/' + (isShr() ? 'shr-create' : 'create'), b); } catch (e) { if (e.data && e.data.results) r = e.data; else throw e; }
        stage.replaceChildren(banner(r.ok === false ? 'err' : 'ok', r.ok === false ? T('raid.failed') : T('raid.done'), ''), logView(r.results || []), btn(T('btn.close'), {kind: 'primary', onclick: () => show('raid')}));
      }));
      c = [h('div', {class: 'grid2'}, field(T('raid.name'), nm), isShr() || W.vol ? field(T('raid.fs'), fs) : h('div')), vol,
        W.plan ? pv.node : (W.perr ? banner('err', W.perr, '') : null), h('div', {class: 'row'}, btn(T('btn.back'), {icon: 'arrow-left', onclick: () => { W.step--; draw(); }}), btn(T('btn.cancel'), {onclick: () => { host.hidden = !!R.arrays.length; W.step = 1; W.devs.clear(); draw(); }}), go)];
      setTimeout(upd);
    }
    stage.replaceChildren(stepsBar(), ...[c].flat(Infinity).filter(Boolean));
  }
  const infoBtnInline = l => (l === 'shr1' || l === 'shr2') ? infoBtn('shr', T('raid.hybrid')) : null;
  function shrTable(r) {
    return h('div', {class: 'cb flush shrt'}, h('div', {class: 'cap'}, T('shr.slices')), table([T('shr.slice'), T('raid.members'), T('raid.level'), T('raid.usable')],
      r.slices.map(s => ({cls: s.unusable ? 'attn' : '', cells: [s.index, s.members.length + '  ×  ' + fmtBytes(s.length), s.level ? LEVEL_NAME[s.level] : badge('neutral', T('shr.unused'), 'x'), s.level ? fmtBytes(s.usable) : '-']})), '', {right: [3]}));
  }
  host.replaceChildren(h('section', {class: 'card'}, h('div', {class: 'ch'}, h('h2', {}, ic('plus', 'lg'), T('raid.create'))), stage));
  draw();
}
/* ---------- Help centre ---------- */
/*PAGE:help*/
PAGES.help = async () => {
  let cur = null, q = '';
  const root = h('div', {}), area = h('div', {});
  const search = h('input', {type: 'search', placeholder: T('help.search'), 'aria-label': T('help.search'), autocomplete: 'off', oninput: () => { q = search.value.toLowerCase(); draw(); }});
  function draw() {
    if (cur) {
      const t = HELP.find(x => x.id === cur);
      area.replaceChildren(h('article', {class: 'card'}, h('div', {class: 'cb'}, btn(T('help.back'), {kind: 'ghost', icon: 'arrow-left', onclick: () => { cur = null; draw(); }}), h('h2', {}, t.title), t.body.split('\n\n').map(p => h('p', {}, p)))));
      return;
    }
    const hits = HELP.filter(t => !q || (t.title + ' ' + t.body + ' ' + (t.keywords || '')).toLowerCase().includes(q));
    area.replaceChildren(hits.length ? h('ul', {class: 'toc', role: 'list'}, hits.map(t => h('li', {}, h('button', {type: 'button', class: 'tocb', onclick: () => { cur = t.id; draw(); }}, t.title)))) : h('div', {class: 'empty'}, ic('info'), T('help.none')));
  }
  draw();
  root.append(h('div', {class: 'ph'}, h('h1', {}, T('help.title')), h('a', {class: 'btn', href: LINKS.docs, target: '_blank', rel: 'noopener noreferrer'}, ic('external-link'), T('help.docs'))), h('div', {class: 'helpsearch'}, search), area);
  return root;
};
/*ENDPAGE*/
/* ---------- boot ---------- */
showLogin().then(async () => { try { await start(); } catch (e) { /* not signed in */ } });
</script></body></html>
"""

INDEX_HTML = _UI_TEMPLATE.replace("/*__COPY__*/null", json.dumps(COPY, separators=(",", ":")).replace("</", "<\\/")).replace("/*__HELP__*/null", json.dumps(HELP_TOPICS, separators=(",", ":")).replace("</", "<\\/")).replace("/*__TOKENS__*/", theme_css()).replace("<!--__SPRITE__-->", _ICON_SPRITE).replace("/*__FONTS__*/", _font_css()).replace("__FAVICON__", _FAVICON_B64).replace("/*__LIC__*/null", json.dumps(_LICENSES).replace("</", "<\\/"))


if __name__ == "__main__":
    sys.exit(main())
