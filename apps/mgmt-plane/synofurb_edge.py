#!/usr/bin/env python3
"""synofurb_edge: certificates (Let's Encrypt through lego) and the HAProxy front of the management plane.

Standard library only, and deliberately free of any import from ``synofurb_mgmt`` (the web UI module loads this one, never the other way round).
Design and status: docs/CERTIFICATES.md.  Nothing here has run against a real Let's Encrypt, a real DNS zone, a real lego, a real HAProxy or a
real systemd unit: the tests use fake executables (lego, haproxy, systemctl, apt-get, dig, ss, systemd-run) and a loopback server that plays
challenges.addr.tools.

  1. names, secrets, X.509 reading           strict domain validation (punycode, wildcard), SHA-224 CNAME target, a tiny DER reader
  2. environment and settings                 EdgeEnv (every seam), /etc/synofurb/edge/*.json, the addr.tools secret (0600)
  3. DNS                                      CNAME check (dig, else a small UDP resolver), addr.tools sanity request
  4. lego                                     the ONE place that builds the lego argv, the exec adapter for challenges.addr.tools
  5. certificate store + jobs                 /etc/synofurb/certs/<id>/{fullchain.pem,privkey.pem,meta.json}, issue / renew / remove, rate limits
  6. renewal timer, health                    systemd timer + service pair, `certs renew`, the `certs` section of `health --json`
  7. HAProxy front                            pure config rendering, safe apply (haproxy -c, snapshot, revert timer), enable / disable / revert
  8. REST routes, CLI                         `register_routes`, `certs ...`, `proxy ...`
  9. web UI                                   COPY, help topics and the two pages (injected into the single-page UI by synofurb_mgmt)

Security rules: no shell anywhere (argv lists); every domain name, id, port and path that reaches an argv, a file name or the HAProxy
configuration is validated first (``valid_domain``, ``CERT_ID_RE``, integer ports); the addr.tools secret and the account key live in 0600 files,
never in argv, never in a log line, never in an API answer after the moment it is created.
"""
from __future__ import annotations

import base64
import calendar
import collections
import hashlib
import http.client
import ipaddress
import json
import os
import re
import secrets as _secrets
import shutil
import socket
import ssl
import struct
import subprocess
import sys
import threading
import time
import urllib.parse
from dataclasses import dataclass
from typing import Any, Callable, Iterable

SCHEMA = 1
SAFE_PATH = "/usr/sbin:/usr/bin:/sbin:/bin"
INSTALL_SCRIPT = "/opt/synofurb-mgmt/synofurb_mgmt.py"

ACME_URLS = {"production": "https://acme-v02.api.letsencrypt.org/directory", "staging": "https://acme-staging-v02.api.letsencrypt.org/directory"}
ENVIRONMENTS = ("production", "staging")
CHALLENGES = ("dns-01", "http-01", "tls-alpn-01")
KEY_TYPES = ("ec256", "ec384", "rsa2048", "rsa4096")
ADDR_TOOLS_URL = "https://challenges.addr.tools"
ADDR_TOOLS_DOMAIN = "challenges.addr.tools"
TERMS_URL = "https://letsencrypt.org/repository/"      # sent by the API, never written into the page (the page has no outbound links of its own)
RENEW_DAYS = 30                 # renew when this many days (or fewer) are left
WARN_DAYS, CRIT_DAYS = 14, 3    # health: a certificate that is still not renewed by now is a warning, then critical
MAX_NAMES = 10
DUP_LIMIT, DUP_WINDOW = 5, 7 * 86400          # Let's Encrypt: 5 identical certificates per 7 days
FAIL_LIMIT, FAIL_WINDOW = 5, 3600            # Let's Encrypt: 5 failed validations per account, host and hour
RETRY_AFTER_FAILURE = 6 * 3600               # the renewal timer does not retry a failed certificate more often than this
ACME_HTTP_PORT, ACME_ALPN_PORT = 5080, 5443  # lego's loopback listeners behind HAProxy
MGMT_LOOPBACK = "127.0.0.1"
REVERT_SECONDS = 120

EDGE_DIR = "/etc/synofurb/edge"
CERTS_DIR = "/etc/synofurb/certs"
LEGO_DIR = "/etc/synofurb/lego"
HAPROXY_DIR = "/etc/haproxy"
HAPROXY_CFG = HAPROXY_DIR + "/haproxy.cfg"
HAPROXY_CERTS = HAPROXY_DIR + "/certs"
HAPROXY_CRT_LIST = HAPROXY_DIR + "/synofurb-crt-list.txt"
ADAPTER_PATH = "/usr/local/sbin/synofurb-acme-addr-tools"
UNIT_DIR = "/etc/systemd/system"
RENEW_UNIT = "synofurb-certs-renew"
REVERT_UNIT = "synofurb-edge-revert"
MARKER = "# Managed by synofurb-mgmt (synofurb_edge.py). Do not edit by hand: the next apply overwrites this file."
CRT_MARKER = "# Managed by synofurb-mgmt (synofurb_edge.py): the certificate list of the HAProxy front."

LABEL_RE = re.compile(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?")
CERT_ID_RE = re.compile(r"(?:_wildcard\.)?(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?")
EMAIL_RE = re.compile(r"[A-Za-z0-9._%+-]{1,64}@[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?){1,6}")
SECRET_RE = re.compile(r"[A-Za-z0-9._~!@#$%^&*()+=:;,?/-]{16,128}")
JOB_RE = re.compile(r"e[0-9a-f]{12}")
ADDON_ID_RE = re.compile(r"[a-z0-9][a-z0-9-]{0,31}")
BAD_TLDS = frozenset({"local", "localhost", "lan", "internal", "home", "test", "invalid", "example", "arpa", "localdomain", "intranet", "corp", "private"})
PUBLIC_TLD_NOTE = "Let's Encrypt only issues for public names"


class EdgeError(ValueError):
    """Rejected input or a refused operation; the message is safe to show to the person."""


class Busy(EdgeError):
    pass


class NotFound(EdgeError):
    pass


# ============================================================================
# 1. Names, secrets, X.509 reading
# ============================================================================

def valid_domain(value: Any, wildcard: bool = False) -> str:
    """A DNS name Let's Encrypt can certify: lowercase ASCII (non-ASCII labels become punycode), 2..127 labels of letters, digits and hyphens,
    at most one leading ``*.`` (only when ``wildcard``), not an IP address, not a private/special-use suffix.  Anything else raises EdgeError."""
    if not isinstance(value, str):
        raise EdgeError("a name must be text")
    s = value.strip().lower()
    if s.endswith("."):
        s = s[:-1]
    if not s or len(s) > 253 or any(ord(c) < 33 or ord(c) == 127 for c in s):
        raise EdgeError("not a valid name: %r" % value[:60])
    wild = s.startswith("*.")
    base = s[2:] if wild else s
    if wild and not wildcard:
        raise EdgeError("a wildcard (*.name) is not allowed here")
    labels = []
    for lab in base.split("."):
        if not lab:
            raise EdgeError("not a valid name: %r" % value[:60])
        if not lab.isascii():
            try:
                lab = lab.encode("idna").decode("ascii")
            except UnicodeError:
                raise EdgeError("not a valid name: %r" % value[:60])
        labels.append(lab)
    base = ".".join(labels)
    if len(base) + (2 if wild else 0) > 253 or len(labels) < 2 or not all(LABEL_RE.fullmatch(lab) for lab in labels):
        raise EdgeError("not a valid name: %r (letters, digits and hyphens, at least two parts)" % value[:60])
    if labels[-1].isdigit():
        raise EdgeError("an IP address cannot get a certificate: use a name")
    if labels[-1] in BAD_TLDS:
        raise EdgeError("%s: .%s is not a public suffix" % (PUBLIC_TLD_NOTE, labels[-1]))
    return ("*." if wild else "") + base


def valid_names(values: Any, challenge: str) -> list[str]:
    if isinstance(values, str):
        values = [x for x in re.split(r"[,\s]+", values) if x]
    if not isinstance(values, list) or not values:
        raise EdgeError("give at least one name")
    if len(values) > MAX_NAMES:
        raise EdgeError("at most %d names per certificate" % MAX_NAMES)
    out: list[str] = []
    for v in values:
        n = valid_domain(v, wildcard=True)
        if n.startswith("*.") and challenge != "dns-01":
            raise EdgeError("a wildcard (%s) needs the DNS challenge" % n)
        if n not in out:
            out.append(n)
    return out


def cert_id_for(primary: str) -> str:
    """Directory / file name of a certificate: its primary name; the wildcard sign becomes the prefix ``_wildcard.`` (not a valid DNS label, so it cannot clash)."""
    cid = ("_wildcard." + primary[2:]) if primary.startswith("*.") else primary
    if not CERT_ID_RE.fullmatch(cid) or ".." in cid:
        raise EdgeError("bad certificate name")
    return cid


def valid_cert_id(value: Any) -> str:
    if not isinstance(value, str) or not CERT_ID_RE.fullmatch(value) or len(value) > 253 or ".." in value:
        raise EdgeError("bad certificate name")
    return value


def name_matches(pattern: str, host: str) -> bool:
    """Does a certificate name (``a.example.org`` or ``*.example.org``) cover ``host``?  A wildcard covers exactly one extra label."""
    if pattern.startswith("*."):
        suffix = pattern[1:]
        return host.endswith(suffix) and "." not in host[:-len(suffix)] and len(host) > len(suffix)
    return pattern == host


def covers(names: Iterable[str], host: str) -> bool:
    return any(name_matches(n, host) for n in names)


def challenge_record(name: str) -> str:
    """The DNS name that carries the ACME challenge of ``name`` (a wildcard and its base share one record)."""
    return "_acme-challenge." + (name[2:] if name.startswith("*.") else name)


def addr_hash(secret: str) -> str:
    """challenges.addr.tools: the lowercase hex SHA-224 (56 characters) of the secret.  ``echo -n SECRET | shasum -a 224 | cut -c 1-56``."""
    return hashlib.sha224(secret.encode("utf-8")).hexdigest()


def cname_target(secret: str) -> str:
    return addr_hash(secret) + "." + ADDR_TOOLS_DOMAIN


def generate_secret() -> str:
    return _secrets.token_urlsafe(32)               # 43 characters of a-z A-Z 0-9 - _


def valid_secret(value: Any) -> str:
    if not isinstance(value, str) or not SECRET_RE.fullmatch(value):
        raise EdgeError("the secret needs 16 to 128 characters: letters, digits and . _ ~ ! @ # $ % ^ & * ( ) + = : ; , ? / -")
    return value


def mask_secret(text: str, secrets: Iterable[str]) -> str:
    for s in secrets:
        if s and len(s) >= 6:
            text = text.replace(s, "***")
    return text


# ---- a tiny DER reader: validity, SAN, subject CN and fingerprint of the leaf certificate -------------------------------

def _tlv(buf: bytes, pos: int) -> tuple[int, int, int]:
    """(tag, content start, content end) of the element at ``pos``."""
    if pos + 2 > len(buf):
        raise EdgeError("truncated certificate")
    tag, ln = buf[pos], buf[pos + 1]
    pos += 2
    if ln & 0x80:
        n = ln & 0x7F
        if n == 0 or n > 4 or pos + n > len(buf):
            raise EdgeError("bad certificate length")
        ln = int.from_bytes(buf[pos:pos + n], "big")
        pos += n
    if pos + ln > len(buf):
        raise EdgeError("truncated certificate")
    return tag, pos, pos + ln


def _children(buf: bytes, start: int, end: int) -> list[tuple[int, int, int]]:
    out, pos = [], start
    while pos < end:
        tag, s, e = _tlv(buf, pos)
        out.append((tag, s, e))
        pos = e
    return out


def _der_time(buf: bytes, tag: int, s: int, e: int) -> int:
    t = buf[s:e].decode("ascii", "replace")
    m = re.fullmatch(r"(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})Z", t) if tag == 0x17 else re.fullmatch(r"(\d{4})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})Z", t)
    if not m:
        raise EdgeError("bad certificate date")
    y = int(m.group(1))
    if tag == 0x17:
        y += 1900 if y >= 50 else 2000
    return calendar.timegm((y, int(m.group(2)), int(m.group(3)), int(m.group(4)), int(m.group(5)), int(m.group(6))))


def pem_blocks(text: str, label: str = "CERTIFICATE") -> list[bytes]:
    out = []
    for m in re.finditer(r"-----BEGIN %s-----\s*([A-Za-z0-9+/=\s]+?)\s*-----END %s-----" % (label, label), text):
        try:
            out.append(base64.b64decode(re.sub(r"\s+", "", m.group(1)), validate=True))
        except ValueError:
            raise EdgeError("a certificate in the file is not valid PEM")
    return out


def parse_cert(pem: str) -> dict:
    """The first certificate of a PEM: ``{not_before, not_after, sans, cn, fingerprint}`` (epoch seconds, DNS names, sha256 hex with colons)."""
    blocks = pem_blocks(pem)
    if not blocks:
        raise EdgeError("no certificate in the file")
    der = blocks[0]
    tag, s, e = _tlv(der, 0)
    top = _children(der, s, e)
    if tag != 0x30 or not top:
        raise EdgeError("not an X.509 certificate")
    tbs = _children(der, top[0][1], top[0][2])
    i = 1 if tbs and tbs[0][0] == 0xA0 else 0                  # [0] version
    # serial, signature, issuer, validity, subject, spki, ...
    validity = _children(der, tbs[i + 3][1], tbs[i + 3][2])
    nb, na = _der_time(der, *validity[0]), _der_time(der, *validity[1])
    cn = ""
    for _tag, rs, re_ in _children(der, tbs[i + 4][1], tbs[i + 4][2]):                     # RDNs
        for _t2, as_, ae in _children(der, rs, re_):                                       # AttributeTypeAndValue
            kids = _children(der, as_, ae)
            if len(kids) == 2 and der[kids[0][1]:kids[0][2]] == b"\x55\x04\x03":
                cn = der[kids[1][1]:kids[1][2]].decode("utf-8", "replace")
    sans: list[str] = []
    for tg, ts, te in tbs[i + 6:]:
        if tg != 0xA3:
            continue
        for _t, xs, xe in _children(der, *_children(der, ts, te)[0][1:3]):                 # Extensions -> each Extension
            parts = _children(der, xs, xe)
            if parts and der[parts[0][1]:parts[0][2]] == b"\x55\x1d\x11":
                octets = parts[-1]
                gn = _children(der, *_children(der, octets[1], octets[2])[0][1:3])
                sans = [der[a:b].decode("ascii", "replace").lower() for g, a, b in gn if g == 0x82]
    fp = hashlib.sha256(der).hexdigest().upper()
    return {"not_before": nb, "not_after": na, "sans": sans, "cn": cn, "fingerprint": ":".join(fp[k:k + 2] for k in range(0, len(fp), 2)), "chain": len(blocks)}


def days_left(expires: float | None, now: float) -> int | None:
    if expires is None:
        return None
    return int((expires - now) // 86400)



# ============================================================================
# 2. Environment (every seam of the module) and settings
# ============================================================================

@dataclass
class Result:
    rc: int
    out: str = ""
    err: str = ""

    @property
    def ok(self) -> bool:
        return self.rc == 0


def run_command(argv: list[str], input: str | None = None, timeout: int = 60, env: dict | None = None, on_line: Callable[[str], None] | None = None,
                search: str = SAFE_PATH) -> Result:
    """Run ``argv`` (no shell).  ``argv[0]`` is looked up in ``search`` unless absolute.  The child gets a small fixed environment plus ``env``.
    With ``on_line`` the combined output is delivered line by line while the command runs.  Never raises: problems become a non-zero rc."""
    if not argv or not all(isinstance(a, str) and "\x00" not in a for a in argv):
        raise ValueError("argv must be a non-empty list of NUL-free strings")
    exe = argv[0] if argv[0].startswith("/") else shutil.which(argv[0], path=search)
    if not exe or not os.access(exe, os.X_OK):
        return Result(127, "", "command not found: " + argv[0])
    child = {"PATH": search, "LC_ALL": "C", "LANG": "C", "HOME": "/root", "DEBIAN_FRONTEND": "noninteractive", "NEEDRESTART_MODE": "a"}
    child.update(env or {})
    try:
        p = subprocess.Popen([exe] + argv[1:], stdin=subprocess.PIPE if input is not None else subprocess.DEVNULL, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT if on_line else subprocess.PIPE, env=child, shell=False, text=True, errors="replace", bufsize=1)
    except OSError as e:
        return Result(126, "", "cannot execute %s: %s" % (argv[0], e))
    timer = threading.Timer(timeout, p.kill)
    timer.start()
    try:
        if on_line:
            assert p.stdout is not None
            tail: collections.deque = collections.deque(maxlen=80)
            for raw in p.stdout:
                line = raw.rstrip("\r\n")
                tail.append(line)
                on_line(line)
            rc = p.wait()
            out, err = "\n".join(tail), ""
        else:
            out, err = p.communicate(input)
            rc = p.returncode
    finally:
        timer.cancel()
        if p.poll() is None:
            p.kill()
        for fh in (p.stdout, p.stderr, p.stdin):
            if fh is not None:
                try:
                    fh.close()
                except OSError:
                    pass
    if rc < 0:
        return Result(124, out or "", "timed out after %ds: %s" % (timeout, argv[0]))
    return Result(rc, out or "", err or "")


def http_post_form(url: str, fields: dict, timeout: float = 20.0, cafile: str = "") -> tuple[int, str]:
    """POST application/x-www-form-urlencoded to an https URL; returns (status, body).  Raises OSError / ssl.SSLError when the server cannot be reached."""
    u = urllib.parse.urlsplit(url)
    if u.scheme != "https" or not u.hostname:
        raise OSError("only https URLs are used")
    ctx = ssl.create_default_context(cafile=cafile or None)
    conn = http.client.HTTPSConnection(u.hostname, u.port or 443, timeout=timeout, context=ctx)
    try:
        body = urllib.parse.urlencode(fields)
        conn.request("POST", u.path or "/", body=body, headers={"Content-Type": "application/x-www-form-urlencoded", "User-Agent": "synofurb-mgmt", "Connection": "close"})
        r = conn.getresponse()
        return r.status, r.read(4096).decode("utf-8", "replace")
    finally:
        conn.close()


class EdgeEnv:
    """The seams of the module.  ``synofurb_mgmt`` builds one from its App; tests build one around a temp directory and fake executables."""

    def __init__(self, root: str = "", bin_dir: str = "", state_dir: str = "/var/lib/synofurb-mgmt", script_path: str = INSTALL_SCRIPT,
                 audit: Callable[..., None] | None = None, job_lock: Any = None, now: Callable[[], float] = time.time, sleep: Callable[[float], None] = time.sleep,
                 http_post: Callable[..., tuple[int, str]] | None = None, dns_cname: Callable[[str], dict] | None = None,
                 resolve_host: Callable[[str], list[str]] | None = None, local_ips: Callable[[], list[str]] | None = None,
                 sites: Callable[[], list[dict]] | None = None, restart_mgmt: Callable[[], None] | None = None,
                 defer: Callable[[Callable[[], None], float], None] | None = None, port_busy: Callable[[int], bool] | None = None,
                 mgmt_port: int = 8443, run_fn: Callable[..., Result] | None = None, which_fn: Callable[[str], str | None] | None = None):
        self.root, self.bin_dir, self.state_dir, self.script_path = root, bin_dir, state_dir, script_path
        self._audit = audit
        self.job_lock = job_lock or FlockJobLock(self.p(state_dir + "/edge/edge.lock"))
        self.now, self.sleep = now, sleep
        self.http_post = http_post or http_post_form
        self._dns_cname = dns_cname
        self.resolve_host = resolve_host or _resolve_host
        self.local_ips = local_ips or (lambda: [])
        self.sites = sites or (lambda: [])
        self.restart_mgmt = restart_mgmt or (lambda: None)
        self.defer = defer or (lambda fn, delay: fn())
        self._port_busy = port_busy
        self.mgmt_port = mgmt_port
        self._run_fn, self._which_fn = run_fn, which_fn            # --dry-run: record commands instead of running them

    # -- paths -----------------------------------------------------------------
    def p(self, path: str) -> str:
        return os.path.normpath(os.path.join(self.root, path.lstrip("/"))) if self.root else path

    @property
    def search(self) -> str:
        return (self.bin_dir + ":" if self.bin_dir else "") + SAFE_PATH

    def edge_path(self, name: str) -> str:
        return self.p(EDGE_DIR + "/" + name)

    def state_path(self, name: str) -> str:
        return self.p(self.state_dir + "/edge/" + name)

    # -- commands -----------------------------------------------------------------
    def run(self, argv: list[str], input: str | None = None, timeout: int = 60, env: dict | None = None) -> Result:
        if self._run_fn:
            return self._run_fn(argv, input=input, timeout=timeout, env=env)
        return run_command(argv, input=input, timeout=timeout, env=env, search=self.search)

    def stream(self, argv: list[str], on_line: Callable[[str], None], timeout: int = 600, env: dict | None = None) -> Result:
        if self._run_fn:
            r = self._run_fn(argv, input=None, timeout=timeout, env=env)
            for ln in (r.out + ("\n" + r.err if r.err else "")).splitlines():
                on_line(ln)
            return r
        return run_command(argv, timeout=timeout, env=env, on_line=on_line, search=self.search)

    def which(self, name: str) -> str | None:
        if self._which_fn:
            return self._which_fn(name)
        return shutil.which(name, path=self.search)

    def audit(self, user: str | None, ip: str, action: str, args: dict, result: str) -> None:
        if self._audit:
            self._audit(user, ip, action, args, result)

    def port_busy(self, port: int) -> bool:
        if self._port_busy:
            return self._port_busy(port)
        r = self.run(["ss", "-H", "-ltn"], timeout=10)
        return any(re.search(r":%d$" % port, ln.split()[3]) for ln in r.out.splitlines() if len(ln.split()) >= 4) if r.ok else False

    def listening_ports(self) -> set[int]:
        r = self.run(["ss", "-H", "-ltn"], timeout=10)
        ports = set()
        for ln in r.out.splitlines() if r.ok else []:
            f = ln.split()
            if len(f) >= 4 and re.search(r":(\d+)$", f[3]):
                ports.add(int(re.search(r":(\d+)$", f[3]).group(1)))
        return ports

    def dns_cname(self, name: str) -> dict:
        if self._dns_cname:
            return self._dns_cname(name)
        return query_cname(self, name)


def _resolve_host(name: str) -> list[str]:
    try:
        return sorted({ai[4][0] for ai in socket.getaddrinfo(name, None, proto=socket.IPPROTO_TCP)})
    except OSError:
        return []


class FlockJobLock:
    """Default cross-process job lock (a flock on a file in the state directory).  The web module replaces it with an adapter around its own
    ``JobLock`` so that certificate jobs, add-on jobs and management-plane updates exclude each other.  Interface: acquire(kind, label, job_id) ->
    token|None, release(token), holder() -> dict|None."""

    def __init__(self, path: str):
        import fcntl
        self._fcntl = fcntl
        self.path = path

    def acquire(self, kind: str, label: str, job_id: str = "") -> int | None:
        os.makedirs(os.path.dirname(self.path), mode=0o700, exist_ok=True)
        fd = os.open(self.path, os.O_RDWR | os.O_CREAT | getattr(os, "O_CLOEXEC", 0), 0o600)
        try:
            self._fcntl.flock(fd, self._fcntl.LOCK_EX | self._fcntl.LOCK_NB)
        except OSError:
            os.close(fd)
            return None
        return fd

    def release(self, token: int | None) -> None:
        if token is not None:
            try:
                self._fcntl.flock(token, self._fcntl.LOCK_UN)
            finally:
                os.close(token)

    def holder(self) -> dict | None:
        t = self.acquire("probe", "")
        if t is None:
            return {"kind": "edge"}
        self.release(t)
        return None


# ---- small file helpers (0600 files, 0700 directories, atomic replace) ---------------------------------------------------

def mkdirs(path: str, mode: int = 0o700) -> None:
    """makedirs whose new folders really get ``mode`` (the service runs with UMask=0027)."""
    missing, cur = [], path
    while cur and not os.path.isdir(cur):
        missing.append(cur)
        nxt = os.path.dirname(cur)
        if nxt == cur:
            break
        cur = nxt
    for d in reversed(missing):
        try:
            os.mkdir(d, mode)
        except FileExistsError:
            continue
        os.chmod(d, mode)


def write_file(path: str, data: str | bytes, mode: int = 0o600, dir_mode: int = 0o700) -> None:
    """Atomically replace ``path``; the file never exists with wider permissions than ``mode``."""
    import tempfile
    d = os.path.dirname(path)
    mkdirs(d, dir_mode)
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".tmp-synofurb-")
    try:
        os.fchmod(fd, mode)
        with os.fdopen(fd, "wb") as fh:
            fh.write(data if isinstance(data, bytes) else data.encode("utf-8"))
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def read_text(path: str) -> str:
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            return fh.read()
    except OSError:
        return ""


def read_json(path: str, default: Any = None) -> Any:
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return default


def write_json(path: str, obj: Any, mode: int = 0o600) -> None:
    write_file(path, json.dumps(obj, indent=1, sort_keys=True) + "\n", mode)


def unlink(path: str) -> None:
    try:
        os.unlink(path)
    except OSError:
        pass


# ---- settings -------------------------------------------------------------------------------------------------------

SETTING_DEFAULTS = {"email": "", "tos_accepted": None, "environment": "production", "key_type": "ec256", "dns_skip_check": False, "dns_resolvers": "",
                    "addr_tools_url": ADDR_TOOLS_URL, "addr_tools_ca": ""}
URL_RE = re.compile(r"https://[a-z0-9](?:[a-z0-9.-]{0,251}[a-z0-9])?(?::\d{1,5})?")
RESOLVER_RE = re.compile(r"(?:\d{1,3}(?:\.\d{1,3}){3}|\[[0-9a-fA-F:]+\])(?::\d{1,5})?")


def load_settings(env: EdgeEnv) -> dict:
    d = read_json(env.edge_path("settings.json"), {})
    out = dict(SETTING_DEFAULTS)
    if isinstance(d, dict):
        for k, dv in SETTING_DEFAULTS.items():
            v = d.get(k, dv)
            if (v is None or (isinstance(v, (int, float)) and not isinstance(v, bool))) if k == "tos_accepted" else type(v) is type(dv):
                out[k] = v
    if out["environment"] not in ENVIRONMENTS:
        out["environment"] = "production"
    if out["key_type"] not in KEY_TYPES:
        out["key_type"] = "ec256"
    return out


def save_settings(env: EdgeEnv, s: dict) -> None:
    write_json(env.edge_path("settings.json"), {k: s[k] for k in SETTING_DEFAULTS})


def load_secret(env: EdgeEnv) -> str | None:
    d = read_json(env.edge_path("addr-tools.json"), {})
    s = d.get("secret") if isinstance(d, dict) else None
    return s if isinstance(s, str) and SECRET_RE.fullmatch(s) else None


def store_secret(env: EdgeEnv, secret: str | None, settings: dict) -> None:
    """/etc/synofurb/edge/addr-tools.json (0600): what the lego adapter reads.  The only file that holds the secret."""
    if secret is None:
        unlink(env.edge_path("addr-tools.json"))
        return
    write_json(env.edge_path("addr-tools.json"), {"secret": secret, "url": settings["addr_tools_url"], "cafile": env.p(settings["addr_tools_ca"]) if settings["addr_tools_ca"] else ""})


def settings_view(env: EdgeEnv) -> dict:
    s = load_settings(env)
    sec = load_secret(env)
    return {"email": s["email"], "tos_accepted": s["tos_accepted"], "environment": s["environment"], "key_type": s["key_type"], "dns_skip_check": s["dns_skip_check"],
            "dns_resolvers": s["dns_resolvers"], "addr_tools_url": s["addr_tools_url"], "addr_tools_ca": s["addr_tools_ca"],
            "secret": {"set": sec is not None, "target": cname_target(sec) if sec else None, "length": len(sec) if sec else 0}, "terms_url": TERMS_URL}


def update_settings(env: EdgeEnv, body: dict) -> dict:
    """Validate and save.  Keys: email, accept_tos (bool), environment, key_type, dns_skip_check, dns_resolvers, addr_tools_url, addr_tools_ca, secret, generate_secret.
    Returns the settings view; ``secret_once`` is present only when a secret was just created or given (the one moment it is shown)."""
    s = load_settings(env)
    old_secret = load_secret(env)
    if "email" in body:
        e = body["email"]
        if e != "" and (not isinstance(e, str) or not EMAIL_RE.fullmatch(e)):
            raise EdgeError("that does not look like an email address")
        s["email"] = e
    if "accept_tos" in body:
        s["tos_accepted"] = int(env.now()) if body["accept_tos"] is True else None
    if "environment" in body:
        if body["environment"] not in ENVIRONMENTS:
            raise EdgeError("environment must be production or staging")
        s["environment"] = body["environment"]
    if "key_type" in body:
        if body["key_type"] not in KEY_TYPES:
            raise EdgeError("key type must be one of %s" % ", ".join(KEY_TYPES))
        s["key_type"] = body["key_type"]
    if "dns_skip_check" in body:
        if not isinstance(body["dns_skip_check"], bool):
            raise EdgeError("dns_skip_check must be true or false")
        s["dns_skip_check"] = body["dns_skip_check"]
    if "dns_resolvers" in body:
        r = body["dns_resolvers"]
        parts = [x for x in re.split(r"[,\s]+", r) if x] if isinstance(r, str) else None
        if parts is None or len(parts) > 3 or not all(RESOLVER_RE.fullmatch(x) for x in parts):
            raise EdgeError("resolvers: up to three addresses such as 1.1.1.1 or 9.9.9.9:53")
        s["dns_resolvers"] = ",".join(parts)
    if "addr_tools_url" in body:
        u = body["addr_tools_url"]
        if not isinstance(u, str) or not URL_RE.fullmatch(u):
            raise EdgeError("the challenge service URL must be https://host[:port]")
        s["addr_tools_url"] = u
    if "addr_tools_ca" in body:
        c = body["addr_tools_ca"]
        if c != "" and (not isinstance(c, str) or not re.fullmatch(r"/etc/synofurb/edge/[A-Za-z0-9._-]{1,64}", c)):
            raise EdgeError("a CA file must be a file inside /etc/synofurb/edge")
        s["addr_tools_ca"] = c
    secret_once = None
    new_secret = old_secret
    if body.get("generate_secret") is True:
        new_secret = secret_once = generate_secret()
    elif body.get("secret"):
        new_secret = secret_once = valid_secret(body["secret"])
    mkdirs(env.edge_path(""), 0o700)
    save_settings(env, s)
    if new_secret != old_secret or (new_secret and ("addr_tools_url" in body or "addr_tools_ca" in body)):
        store_secret(env, new_secret, s)
    view = settings_view(env)
    if secret_once:
        view["secret_once"] = secret_once
    return view


# ============================================================================
# 3. DNS: the CNAME check and the addr.tools sanity request
# ============================================================================

def _dns_query(name: str, qtype: int, txid: int) -> bytes:
    q = struct.pack(">HHHHHH", txid, 0x0100, 1, 0, 0, 0)
    for lab in name.rstrip(".").split("."):
        b = lab.encode("ascii")
        q += bytes([len(b)]) + b
    return q + b"\x00" + struct.pack(">HH", qtype, 1)


def _dns_name(msg: bytes, pos: int) -> tuple[str, int]:
    labels, jumped, end = [], False, pos
    for _ in range(128):
        if pos >= len(msg):
            raise ValueError("truncated DNS answer")
        n = msg[pos]
        if n == 0:
            pos += 1
            break
        if n & 0xC0 == 0xC0:
            ptr = ((n & 0x3F) << 8) | msg[pos + 1]
            if not jumped:
                end = pos + 2
            pos, jumped = ptr, True
            continue
        labels.append(msg[pos + 1:pos + 1 + n].decode("ascii", "replace").lower())
        pos += 1 + n
    else:
        raise ValueError("bad DNS name")
    return ".".join(labels), (end if jumped else pos)


def parse_dns_answer(msg: bytes, txid: int) -> dict:
    """{'rcode', 'answers': [(owner, type, value)]} where CNAME / NS / PTR values are names, TXT is the joined text, A / AAAA are addresses."""
    if len(msg) < 12 or struct.unpack(">H", msg[:2])[0] != txid:
        raise ValueError("unexpected DNS answer")
    flags, qd, an = struct.unpack(">HHH", msg[2:8])
    pos = 12
    for _ in range(qd):
        _n, pos = _dns_name(msg, pos)
        pos += 4
    out = []
    for _ in range(an):
        owner, pos = _dns_name(msg, pos)
        rtype, _cls, _ttl, rdlen = struct.unpack(">HHIH", msg[pos:pos + 10])
        pos += 10
        rd = msg[pos:pos + rdlen]
        if rtype in (5, 2, 12):
            val = _dns_name(msg, pos)[0]
        elif rtype == 16:
            val, i = "", 0
            while i < len(rd):
                val += rd[i + 1:i + 1 + rd[i]].decode("utf-8", "replace")
                i += 1 + rd[i]
        elif rtype == 1 and rdlen == 4:
            val = socket.inet_ntoa(rd)
        elif rtype == 28 and rdlen == 16:
            val = socket.inet_ntop(socket.AF_INET6, rd)
        else:
            val = ""
        out.append((owner, rtype, val))
        pos += rdlen
    return {"rcode": flags & 0xF, "answers": out}


def system_resolvers(env: EdgeEnv) -> list[str]:
    found = [m.group(1) for ln in read_text(env.p("/etc/resolv.conf")).splitlines() for m in [re.match(r"\s*nameserver\s+(\S+)", ln)] if m]
    return found[:3] or ["1.1.1.1", "9.9.9.9"]


def udp_lookup(servers: list[str], name: str, qtype: int, timeout: float = 3.0, port: int = 53) -> dict:
    last = "no resolver answered"
    for srv in servers:
        host, p = srv, port
        m = re.fullmatch(r"(\[[0-9a-fA-F:]+\]|[0-9.]+):(\d+)", srv)
        if m:
            host, p = m.group(1).strip("[]"), int(m.group(2))
        host = host.strip("[]")
        try:
            fam = socket.AF_INET6 if ":" in host else socket.AF_INET
            txid = int.from_bytes(os.urandom(2), "big")
            with socket.socket(fam, socket.SOCK_DGRAM) as s:
                s.settimeout(timeout)
                s.sendto(_dns_query(name, qtype, txid), (host, p))
                data, _ = s.recvfrom(4096)
            return parse_dns_answer(data, txid)
        except (OSError, ValueError, struct.error) as e:
            last = str(e) or type(e).__name__
    return {"rcode": -1, "answers": [], "error": last}


def query_cname(env: EdgeEnv, name: str) -> dict:
    """The CNAME of ``name``: ``{cname: 'x.y' | None, error: str | None}``.  Uses ``dig`` when installed, else a small built-in UDP resolver."""
    if not re.fullmatch(r"_acme-challenge\.[a-z0-9.-]{1,240}", name):
        return {"cname": None, "error": "bad name"}
    if env.which("dig"):
        r = env.run(["dig", "+short", "+time=3", "+tries=1", "CNAME", name], timeout=20)
        if not r.ok:
            return {"cname": None, "error": (r.err or r.out or "dig failed").strip()[:200]}
        first = next((ln.strip() for ln in r.out.splitlines() if ln.strip() and not ln.startswith(";")), "")
        return {"cname": first.rstrip(".").lower() or None, "error": None}
    res = udp_lookup(system_resolvers(env), name, 5)
    if res.get("error"):
        return {"cname": None, "error": res["error"]}
    if res["rcode"] not in (0, 3):
        return {"cname": None, "error": "the resolver answered with an error (%d)" % res["rcode"]}
    hit = next((v for o, t, v in res["answers"] if t == 5 and o == name), None)
    return {"cname": hit, "error": None}


def check_dns(env: EdgeEnv, name: str) -> dict:
    """Does ``_acme-challenge.<name>`` point at ``<sha224(secret)>.challenges.addr.tools``?  state: ok | wrong | missing | error | nosecret."""
    n = valid_domain(name, wildcard=True)
    rec = challenge_record(n)
    sec = load_secret(env)
    if not sec:
        return {"name": n, "record": rec, "state": "nosecret", "target": None, "found": None, "message": "no secret is set"}
    target = cname_target(sec)
    res = env.dns_cname(rec)
    found = res.get("cname")
    if res.get("error"):
        return {"name": n, "record": rec, "state": "error", "target": target, "found": None, "message": res["error"]}
    if not found:
        return {"name": n, "record": rec, "state": "missing", "target": target, "found": None, "message": "no CNAME record found"}
    return {"name": n, "record": rec, "state": "ok" if found == target else "wrong", "target": target, "found": found,
            "message": "points at the challenge service" if found == target else "points somewhere else"}


def check_secret(env: EdgeEnv) -> dict:
    """The sanity request: POST only ``secret``; the service answers with the sub-domain it derived.  It must equal our SHA-224 (nothing is changed on the service)."""
    sec = load_secret(env)
    s = load_settings(env)
    if not sec:
        return {"ok": False, "state": "nosecret", "message": "no secret is set"}
    try:
        status, body = env.http_post(s["addr_tools_url"], {"secret": sec}, 20.0, env.p(s["addr_tools_ca"]) if s["addr_tools_ca"] else "")
    except (OSError, ssl.SSLError, http.client.HTTPException) as e:
        return {"ok": False, "state": "unreachable", "message": "cannot reach the challenge service: %s" % mask_secret(str(e), [sec])[:160]}
    got = body.strip().lower().rstrip(".")
    if got.endswith("." + ADDR_TOOLS_DOMAIN):
        got = got[:-len(ADDR_TOOLS_DOMAIN) - 1]
    want = addr_hash(sec)
    if status // 100 != 2:
        return {"ok": False, "state": "refused", "status": status, "message": "the service answered %d" % status}
    if got != want:
        return {"ok": False, "state": "mismatch", "status": status, "message": "the service derived a different sub-domain than expected"}
    return {"ok": True, "state": "ok", "status": status, "message": "the service derives the same sub-domain"}


# ============================================================================
# 4. lego: the one place that builds its command line; the exec adapter for challenges.addr.tools
# ============================================================================

def lego_argv(exe: str, *, command: str, environment: str, email: str, path: str, names: list[str], challenge: str, key_type: str,
              http_listen: str = ":80", alpn_listen: str = ":443", dns_skip_check: bool = False, resolvers: Iterable[str] = (), days: int = RENEW_DAYS) -> list[str]:
    """The lego command line, built in ONE place.  Flag names are lego 4.9.1 (Debian trixie) from memory of its CLI: verify on the unit with lego --help.

      global flags   --server URL  --email E  --accept-tos  --path DIR  --key-type ec256|ec384|rsa2048|rsa4096  --domains NAME (repeatable)
      DNS-01         --dns exec  [--dns.disable-cp]  [--dns.resolvers HOST[:PORT]] (repeatable)     env EXEC_PATH, EXEC_PROPAGATION_TIMEOUT, ...
      HTTP-01        --http  --http.port HOST:PORT        (lego's own web server; HAProxy forwards /.well-known/acme-challenge/ to it)
      TLS-ALPN-01    --tls   --tls.port HOST:PORT         (lego's own TLS server; HAProxy sends ALPN acme-tls/1 connections to it)
      sub-commands   run                      register the account if needed and obtain a certificate
                     renew --days N            renew when fewer than N days are left (files must exist in --path)
    Every value is validated before it gets here (names by valid_domain, e-mail by EMAIL_RE, paths are built by the module, ports are integers)."""
    if command not in ("run", "renew") or environment not in ACME_URLS or challenge not in CHALLENGES or key_type not in KEY_TYPES:
        raise EdgeError("bad certificate request")
    if not EMAIL_RE.fullmatch(email):
        raise EdgeError("a valid e-mail address is needed for the Let's Encrypt account")
    for n in names:
        valid_domain(n, wildcard=True)
    argv = [exe, "--server", ACME_URLS[environment], "--email", email, "--accept-tos", "--path", path, "--key-type", key_type]
    for n in names:
        argv += ["--domains", n]
    if challenge == "dns-01":
        argv += ["--dns", "exec"]
        if dns_skip_check:
            argv += ["--dns.disable-cp"]
        for r in resolvers:
            argv += ["--dns.resolvers", r]
    elif challenge == "http-01":
        argv += ["--http", "--http.port", http_listen]
    else:
        argv += ["--tls", "--tls.port", alpn_listen]
    if command == "run":
        argv += ["run"]
    else:
        argv += ["renew", "--days", str(int(days))]
    return argv


def lego_env(adapter: str) -> dict:
    """Environment of the lego child process (the exec provider): EXEC_PATH is the adapter; propagation settings generous for a CNAME chain."""
    return {"EXEC_PATH": adapter, "EXEC_PROPAGATION_TIMEOUT": "300", "EXEC_POLLING_INTERVAL": "5", "EXEC_SEQUENCE_INTERVAL": "10"}


def lego_file_stem(primary: str) -> str:
    """lego names its files after the first domain; the wildcard sign becomes an underscore."""
    return primary.replace("*", "_")


ADAPTER_TEMPLATE = r'''#!/usr/bin/python3
# Generated by synofurb-mgmt (synofurb_edge.py): the lego "exec" DNS provider for https://challenges.addr.tools.  Do not edit; it is rewritten when needed.
#   <this> present [--] <FQDN> <value>   POST secret and txt to the service; success is HTTP 201
#   <this> cleanup [--] <FQDN> <value>   nothing to do: the service removes its records by itself
#   <this> timeout                       nothing
# The secret is read from the root-only file below and is never printed.
import json, ssl, sys, time, urllib.parse, http.client

CONF = %(conf)s

def main(argv):
    args = [a for a in argv[1:] if a != "--"]
    if not args:
        print("usage: present|cleanup|timeout [--] FQDN VALUE", file=sys.stderr)
        return 2
    cmd, rest = args[0], args[1:]
    if cmd in ("cleanup", "timeout"):
        return 0
    if cmd != "present" or len(rest) < 2:
        print("usage: present [--] FQDN VALUE", file=sys.stderr)
        return 2
    value = rest[1]
    if not value or len(value) > 200 or any(c.isspace() or ord(c) < 33 for c in value):
        print("refusing a challenge value that does not look like an ACME value", file=sys.stderr)
        return 2
    try:
        with open(CONF, encoding="utf-8") as fh:
            conf = json.load(fh)
        secret, url, cafile = conf["secret"], conf.get("url") or "https://challenges.addr.tools", conf.get("cafile") or None
    except (OSError, ValueError, KeyError):
        print("the challenge service is not configured (no secret)", file=sys.stderr)
        return 1
    u = urllib.parse.urlsplit(url)
    if u.scheme != "https" or not u.hostname:
        print("the challenge service URL must be https", file=sys.stderr)
        return 1
    body = urllib.parse.urlencode({"secret": secret, "txt": value})
    last = "no answer"
    for attempt in range(3):
        try:
            conn = http.client.HTTPSConnection(u.hostname, u.port or 443, timeout=20, context=ssl.create_default_context(cafile=cafile))
            conn.request("POST", u.path or "/", body=body, headers={"Content-Type": "application/x-www-form-urlencoded", "User-Agent": "synofurb-mgmt", "Connection": "close"})
            r = conn.getresponse()
            text = r.read(2000).decode("utf-8", "replace").strip()
            conn.close()
            if r.status == 201:
                print("challenge published (HTTP 201)")
                return 0
            last = "HTTP %%d %%s" %% (r.status, text.replace(secret, "***")[:200])
            if r.status < 500:
                break
        except (OSError, ssl.SSLError, http.client.HTTPException) as e:
            last = str(e).replace(secret, "***")[:200]
        time.sleep(2 * (attempt + 1))
    print("the challenge service refused or did not answer: " + last, file=sys.stderr)
    return 1

sys.exit(main(sys.argv))
'''


def adapter_source(conf_path: str) -> str:
    if not re.fullmatch(r"/[A-Za-z0-9._/-]{1,200}", conf_path):
        raise EdgeError("bad adapter configuration path")
    return ADAPTER_TEMPLATE % {"conf": json.dumps(conf_path)}


def ensure_adapter(env: EdgeEnv) -> str:
    """Write the exec adapter when missing or different (0700, root); returns the path lego must use as EXEC_PATH."""
    path = env.p(ADAPTER_PATH)
    want = adapter_source(env.edge_path("addr-tools.json"))
    if read_text(path) != want:
        write_file(path, want, 0o700, 0o755)
    os.chmod(path, 0o700)
    return path


_RATE_PATTERNS = (
    (re.compile(r"too many certificates|rateLimited|rate limit", re.I), "ratelimit",
     "Let's Encrypt has a limit for this name (5 identical certificates a week, 50 per registered domain). Wait, or try the staging environment."),
    (re.compile(r"too many failed authorizations|too many failed", re.I), "ratelimit",
     "Too many failed attempts for this name. Let's Encrypt allows 5 per hour; wait an hour, then check the DNS record."),
    (re.compile(r"NXDOMAIN|no such host|DNS problem: NXDOMAIN", re.I), "dns", "The name does not exist in DNS yet."),
    (re.compile(r"CAA record", re.I), "caa", "A CAA record of the domain does not allow Let's Encrypt."),
    (re.compile(r"address already in use|bind:", re.I), "port", "The port lego needs is already in use by another program."),
    (re.compile(r"Timeout during connect|Connection refused|connection refused|i/o timeout|no route to host|Fetching .* Connection", re.I), "reach",
     "Let's Encrypt could not connect. Is port 80 (HTTP-01) or 443 (TLS-ALPN-01) open from the Internet and forwarded to this unit?"),
    (re.compile(r"time limit exceeded|propagat", re.I), "propagation", "The DNS record did not appear in time. Check the CNAME with Check DNS, then try again."),
    (re.compile(r"Incorrect TXT record|unauthorized|DNS problem", re.I), "dns", "Let's Encrypt did not find the expected DNS record. Check the CNAME with Check DNS."),
    (re.compile(r"challenge service refused|not configured", re.I), "secret", "The challenge service refused the secret or could not be reached."),
)


def explain_lego_error(text: str) -> tuple[str, str]:
    """(kind, friendly message) for the output of a failed lego run; the raw last line is appended by the caller."""
    for rx, kind, msg in _RATE_PATTERNS:
        if rx.search(text):
            return kind, msg
    return "other", "Let's Encrypt did not issue the certificate."



# ============================================================================
# 5. Jobs, the certificate store, issue / renew / remove
# ============================================================================

class Job:
    MAX_LOG = 300

    def __init__(self, jid: str, kind: str, subject: str, user: str | None, steps: list[str], now: float):
        self.id, self.kind, self.subject, self.user = jid, kind, subject, user
        self.steps = steps
        self.index = 0
        self.state = "running"
        self.message = ""
        self.ok: bool | None = None
        self.error = ""
        self.started, self.finished = now, None
        self.log: collections.deque = collections.deque(maxlen=self.MAX_LOG)
        self.warnings: list[str] = []
        self.result: dict = {}
        self._lock = threading.Lock()
        self.scrub: Callable[[str], str] = lambda s: s
        self.sink: Callable[[str], None] | None = None

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
            d = {"id": self.id, "kind": self.kind, "subject": self.subject, "state": self.state, "step": self.steps[self.index] if self.steps and self.index < len(self.steps) else "",
                 "steps": list(self.steps), "progress": round(100 * done / total), "message": self.message, "ok": self.ok, "error": self.error,
                 "started": self.started, "finished": self.finished, "warnings": list(self.warnings), "result": dict(self.result)}
            if full:
                d["log"] = list(self.log)[-120:]
            return d


class JobStore:
    KEEP = 30

    def __init__(self, env: EdgeEnv):
        self.env = env
        self.jobs: collections.OrderedDict[str, Job] = collections.OrderedDict()
        self.lock = threading.Lock()
        self.path = env.state_path("jobs.json")

    def new(self, kind: str, subject: str, user: str | None, steps: list[str]) -> Job:
        with self.lock:
            j = Job("e" + _secrets.token_hex(6), kind, subject, user, steps, self.env.now())
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

    def latest(self, subject: str) -> Job | None:
        with self.lock:
            return next((j for j in reversed(self.jobs.values()) if j.subject == subject), None)

    def views(self) -> list[dict]:
        with self.lock:
            return [j.view(False) for j in reversed(self.jobs.values())]

    def persist(self) -> None:
        try:
            with self.lock:
                done = [j.view() for j in self.jobs.values() if j.state != "running"][-self.KEEP:]
            write_json(self.path, {"schema": SCHEMA, "jobs": done})
        except OSError:
            pass

    def restore(self) -> None:
        doc = read_json(self.path, {})
        for v in doc.get("jobs", []) if isinstance(doc, dict) else []:
            if isinstance(v, dict) and isinstance(v.get("id"), str) and JOB_RE.fullmatch(v["id"]) and v["id"] not in self.jobs:
                j = Job(v["id"], str(v.get("kind", ""))[:20], str(v.get("subject", ""))[:80], None, [str(s)[:40] for s in v.get("steps", [])][:12], float(v.get("started") or 0))
                j.state = v.get("state") if v.get("state") in ("done", "failed") else "failed"
                j.ok, j.error, j.message, j.finished = v.get("ok"), str(v.get("error", ""))[:400], str(v.get("message", ""))[:400], v.get("finished")
                j.log.extend(str(x)[:400] for x in v.get("log", [])[-120:])
                self.jobs[j.id] = j


class CertStore:
    """/etc/synofurb/certs/<id>/{fullchain.pem,privkey.pem,meta.json}: directories 0700, files 0600, root."""

    def __init__(self, env: EdgeEnv):
        self.env = env

    def root(self) -> str:
        return self.env.p(CERTS_DIR)

    def dir(self, cid: str) -> str:
        return os.path.join(self.root(), valid_cert_id(cid))

    def ids(self) -> list[str]:
        try:
            names = sorted(os.listdir(self.root()))
        except OSError:
            return []
        return [n for n in names if CERT_ID_RE.fullmatch(n) and not os.path.islink(os.path.join(self.root(), n)) and os.path.isfile(os.path.join(self.root(), n, "meta.json"))]

    def meta(self, cid: str) -> dict | None:
        d = read_json(os.path.join(self.dir(cid), "meta.json"))
        return d if isinstance(d, dict) and d.get("id") == cid else None

    def save_meta(self, cid: str, meta: dict) -> None:
        mkdirs(self.root(), 0o700)
        mkdirs(self.dir(cid), 0o700)
        os.chmod(self.dir(cid), 0o700)
        write_json(os.path.join(self.dir(cid), "meta.json"), meta)

    def write_pems(self, cid: str, fullchain: str, privkey: str) -> None:
        mkdirs(self.dir(cid), 0o700)
        os.chmod(self.dir(cid), 0o700)
        write_file(os.path.join(self.dir(cid), "fullchain.pem"), fullchain, 0o600)
        write_file(os.path.join(self.dir(cid), "privkey.pem"), privkey, 0o600)

    def read_pems(self, cid: str) -> tuple[str, str] | None:
        a, b = read_text(os.path.join(self.dir(cid), "fullchain.pem")), read_text(os.path.join(self.dir(cid), "privkey.pem"))
        return (a, b) if a and b else None

    def remove(self, cid: str) -> None:
        d = self.dir(cid)
        if os.path.isdir(d) and not os.path.islink(d):
            shutil.rmtree(d)

    def combined_path(self, cid: str) -> str:
        return self.env.p(HAPROXY_CERTS + "/" + valid_cert_id(cid) + ".pem")

    def write_combined(self, cid: str) -> bool:
        """The PEM HAProxy loads: certificate chain then key (0600)."""
        pems = self.read_pems(cid)
        if not pems:
            return False
        write_file(self.combined_path(cid), pems[0].rstrip("\n") + "\n" + pems[1].rstrip("\n") + "\n", 0o600, 0o755)
        return True


def apt_install(env: EdgeEnv, job: Job, package: str, why: str) -> None:
    """apt-get update + install one package on demand (Debian mirror and Internet needed)."""
    job.line("installing %s from the Debian mirror (%s)" % (package, why))
    base = ["apt-get", "-y", "-o", "Dpkg::Options::=--force-confold", "-o", "Dpkg::Options::=--force-confdef"]
    for argv in (base + ["update"], base + ["install", "--no-install-recommends", package]):
        r = env.stream(argv, job.line, 900)
        if not r.ok:
            tail = (r.out or r.err).strip().splitlines()[-1:] or [""]
            raise EdgeError("Installing %s needs the Debian mirror and Internet access (%s)" % (package, job.scrub(tail[0])[:160]))
    if not env.which(package):
        raise EdgeError("%s was installed but the program was not found" % package)


class CertManager:
    STEPS = ["check", "tools", "challenge", "obtain", "verify", "store", "publish"]

    def __init__(self, env: EdgeEnv, edge: Any):
        self.env, self.edge = env, edge
        self.store = CertStore(env)
        self.jobs = JobStore(env)
        self.jobs.restore()

    # -- views -------------------------------------------------------------------
    def used_by(self, cid: str) -> list[dict]:
        return self.edge.proxy.used_by(cid)

    def view(self, cid: str, with_jobs: bool = True) -> dict:
        m = self.store.meta(cid)
        if m is None:
            raise NotFound("no such certificate")
        now = self.env.now()
        exp = m.get("expires")
        left = days_left(exp, now)
        job = self.jobs.active()
        working = bool(job and job.subject in (cid, "*"))
        if working:
            status = "working"
        elif exp is None:
            status = "failed" if m.get("last_error") else "none"
        elif exp <= now:
            status = "expired"
        elif m.get("last_error") and left is not None and left <= RENEW_DAYS:
            status = "failed"
        elif left is not None and left <= WARN_DAYS:
            status = "expiring"
        else:
            status = "valid"
        sec = load_secret(self.env)
        cnames = []
        if m["challenge"] == "dns-01":
            seen = set()
            for n in m["names"]:
                rec = challenge_record(n)
                if rec not in seen:
                    seen.add(rec)
                    cnames.append({"name": n, "record": rec, "target": cname_target(sec) if sec else None})
        d = {k: m.get(k) for k in ("id", "primary", "names", "challenge", "environment", "key_type", "created", "renewed", "not_before", "expires", "fingerprint", "last_ok", "last_attempt", "last_error")}
        d.update(days_left=left, status=status, has_cert=exp is not None, used_by=self.used_by(cid), cnames=cnames)
        return d

    def list(self) -> dict:
        certs = [self.view(c) for c in self.store.ids()]
        act = self.jobs.active()
        return {"certs": certs, "settings": settings_view(self.env), "tools": self.tools(), "job": act.view(False) if act else None,
                "renew": read_json(self.env.state_path("renew.json"), None), "timer": self.timer_state()}

    def tools(self) -> dict:
        out = {}
        for name in ("lego", "haproxy"):
            out[name] = {"installed": bool(self.env.which(name))}
        return out

    # -- one job at a time (shared with updates and add-on jobs) ---------------------------
    def _start(self, kind: str, subject: str, steps: list[str], user: str | None, ip: str, body_for_audit: dict, fn: Callable[[Job], None],
               background: bool, sink: Callable[[str], None] | None, label: str) -> Job:
        job = self.jobs.new(kind, subject, user, steps)
        token = self.env.job_lock.acquire("certs", label, job.id)
        if token is None:
            holder = self.env.job_lock.holder() or {}
            with self.jobs.lock:
                self.jobs.jobs.pop(job.id, None)
            raise Busy("Another job is running (%s %s)" % (holder.get("kind", "job"), holder.get("label", "")))
        job.sink = sink
        sec = load_secret(self.env)
        job.scrub = lambda s: mask_secret(s, [sec] if sec else [])
        self.env.audit(user, ip, "certs.job.start", dict(body_for_audit, kind=kind, subject=subject), "ok")

        def runner() -> None:
            try:
                fn(job)
                job.ok, job.state = True, "done"
            except EdgeError as e:
                job.ok, job.state, job.error = False, "failed", job.scrub(str(e))[:600]
                job.line("FAILED: " + job.error)
            except Exception as e:                                   # noqa: BLE001 - a job never raises into the thread
                job.ok, job.state, job.error = False, "failed", "internal error: %s" % type(e).__name__
                job.line("FAILED: " + job.error)
            finally:
                job.finished = self.env.now()
                self.env.job_lock.release(token)
                self.env.audit(user, ip, "certs.job." + kind, dict(subject=subject, job=job.id), "ok" if job.ok else "error: " + job.error[:200])
                self.jobs.persist()
        if background:
            threading.Thread(target=runner, daemon=True, name="certs-" + job.id).start()
        else:
            runner()
        return job

    # -- pre-checks ---------------------------------------------------------------------
    def rate_check(self, names: list[str], environment: str) -> list[str]:
        """Let's Encrypt (production) allows 5 identical certificates per 7 days and 5 failed validations per hour.  Refuse before the call that would be refused."""
        if environment != "production":
            return []
        now = self.env.now()
        events = (read_json(self.env.state_path("history.json"), {}) or {}).get("events", [])
        key = ",".join(sorted(names))
        okn = [e for e in events if e.get("key") == key and e.get("env") == environment and e.get("ok") and now - e.get("ts", 0) < DUP_WINDOW]
        bad = [e for e in events if e.get("key") == key and e.get("env") == environment and not e.get("ok") and now - e.get("ts", 0) < FAIL_WINDOW]
        if len(okn) >= DUP_LIMIT:
            wait = int((min(e["ts"] for e in okn) + DUP_WINDOW - now) // 3600) + 1
            raise EdgeError("Let's Encrypt allows 5 identical certificates per week and %d were issued already. Try again in about %d hours, or use the staging environment." % (len(okn), wait))
        if len(bad) >= FAIL_LIMIT:
            raise EdgeError("Let's Encrypt allows 5 failed attempts per hour for this name and they are used up. Check the DNS record or port, wait an hour, then try again.")
        return ["%d of 5 identical certificates this week are used." % len(okn)] if len(okn) >= DUP_LIMIT - 1 else []

    def record_event(self, names: list[str], environment: str, ok: bool) -> None:
        path = self.env.state_path("history.json")
        events = (read_json(path, {}) or {}).get("events", [])
        now = self.env.now()
        events = [e for e in events if now - e.get("ts", 0) < 30 * 86400][-300:]
        events.append({"ts": int(now), "key": ",".join(sorted(names)), "env": environment, "ok": ok})
        try:
            write_json(path, {"schema": SCHEMA, "events": events})
        except OSError:
            pass

    def listen_addrs(self) -> tuple[str, str]:
        """Where lego's HTTP-01 / TLS-ALPN-01 servers listen: behind HAProxy on loopback ports, else directly on 80 / 443."""
        if self.edge.proxy.enabled():
            return "%s:%d" % (MGMT_LOOPBACK, ACME_HTTP_PORT), "%s:%d" % (MGMT_LOOPBACK, ACME_ALPN_PORT)
        return ":80", ":443"

    def precheck(self, names: list[str], challenge: str) -> dict:
        """What can be known without asking Let's Encrypt: DNS records, where the names point, which ports are free.  Reachability from the Internet cannot be tested from here."""
        out: dict = {"challenge": challenge, "names": [], "ports": []}
        if challenge == "dns-01":
            for n in names:
                out["names"].append(check_dns(self.env, n))
            out["ok"] = bool(names) and all(x["state"] == "ok" for x in out["names"])
            return out
        local = set(self.env.local_ips())
        for n in names:
            ips = self.env.resolve_host(n)
            private = [ip for ip in ips if _is_private(ip)]
            out["names"].append({"name": n, "state": "ok" if ips else "missing", "addresses": ips, "local": bool(set(ips) & local), "private": bool(private),
                                 "message": ("resolves to this unit" if set(ips) & local else "resolves to %s" % ", ".join(ips[:3])) if ips else "the name does not resolve"})
        port = 80 if challenge == "http-01" else 443
        proxy_on = self.edge.proxy.enabled()
        busy = self.env.port_busy(port)
        out["ports"].append({"port": port, "front": proxy_on, "busy": busy and not proxy_on, "state": "ok" if (proxy_on or not busy) else "busy",
                             "message": "answered through the HAProxy front" if proxy_on else ("in use by another program" if busy else "free on this unit")})
        out["ok"] = bool(names) and all(x["state"] == "ok" for x in out["names"]) and all(p["state"] == "ok" for p in out["ports"])
        out["note"] = "reachability from the Internet cannot be tested from here"
        return out

    # -- issue / change / renew -------------------------------------------------------------------
    def _request(self, body: dict) -> dict:
        s = load_settings(self.env)
        challenge = body.get("challenge", "dns-01")
        if challenge not in CHALLENGES:
            raise EdgeError("challenge must be one of %s" % ", ".join(CHALLENGES))
        environment = body.get("environment", s["environment"])
        if environment not in ENVIRONMENTS:
            raise EdgeError("environment must be production or staging")
        names = valid_names(body.get("names"), challenge)
        cid = cert_id_for(names[0])
        key_type = body.get("key_type", s["key_type"])
        if key_type not in KEY_TYPES:
            raise EdgeError("bad key type")
        return {"id": cid, "names": names, "challenge": challenge, "environment": environment, "key_type": key_type, "skip_dns_check": body.get("skip_dns_check") is True}

    def issue(self, body: dict, user: str | None = None, ip: str = "-", background: bool = True, sink: Callable[[str], None] | None = None, change: str | None = None) -> Job:
        req = self._request(body)
        if change is not None:
            valid_cert_id(change)
            if self.store.meta(change) is None:
                raise NotFound("no such certificate")
            if req["id"] != change:
                raise EdgeError("the first name of a certificate cannot change: issue a new certificate instead")
        elif (self.store.meta(req["id"]) or {}).get("expires") is not None:
            raise EdgeError("%s exists already: use Change or Renew" % req["id"])         # one that never got its certificate may simply be tried again
        self._need_account()
        req["command"] = "run"
        return self._start("issue" if change is None else "change", req["id"], self.STEPS, user, ip, {"names": req["names"], "challenge": req["challenge"], "environment": req["environment"]},
                           lambda job: self._issue_job(job, req, create=change is None), background, sink, "issue " + req["id"])

    def renew(self, cid: str, user: str | None = None, ip: str = "-", background: bool = True, sink: Callable[[str], None] | None = None) -> Job:
        cid = valid_cert_id(cid)
        m = self.store.meta(cid)
        if m is None:
            raise NotFound("no such certificate")
        self._need_account()
        req = {"id": cid, "names": m["names"], "challenge": m["challenge"], "environment": m["environment"], "key_type": m.get("key_type") or "ec256", "command": "renew",
               "days": 36500, "skip_dns_check": False}
        return self._start("renew", cid, self.STEPS, user, ip, {"id": cid}, lambda job: self._issue_job(job, req, create=False), background, sink, "renew " + cid)

    def _need_account(self) -> None:
        s = load_settings(self.env)
        if not s["email"] or not s["tos_accepted"]:
            raise EdgeError("Set your e-mail address and accept the Let's Encrypt terms first (Settings).")

    def _issue_job(self, job: Job, req: dict, create: bool) -> None:
        cid, now = req["id"], self.env.now()
        prev = self.store.meta(cid)
        try:
            self._obtain(job, req)
        except EdgeError as e:
            msg = str(e)
            meta = prev or {"schema": SCHEMA, "id": cid, "primary": req["names"][0], "names": req["names"], "challenge": req["challenge"], "environment": req["environment"],
                            "key_type": req["key_type"], "created": int(now), "renewed": None, "not_before": None, "expires": None, "fingerprint": None, "last_ok": None}
            meta.update(last_attempt=int(now), last_error=msg[:600])
            self.store.save_meta(cid, meta)
            raise
        job.step("publish", "Publishing the certificate")
        warn = self.edge.proxy.certs_changed([cid], job)
        if warn:
            job.warnings.append(warn)
        self.ensure_timer(job)
        job.step("publish", "Done")
        job.result = {"id": cid}

    def _obtain(self, job: Job, req: dict, announce: bool = True) -> None:
        env, cid = self.env, req["id"]
        s = load_settings(env)

        def say(step: str, msg: str) -> None:
            job.step(step, msg) if announce else job.line(msg)
        say("check", "Checking the request")
        self._need_account()
        warns = self.rate_check(req["names"], req["environment"])
        job.warnings.extend(warns)
        if req["challenge"] == "dns-01":
            if not load_secret(env):
                raise EdgeError("Set the challenge secret first (Settings), then create the CNAME record.")
            if not req.get("skip_dns_check"):
                bad = []
                for n in req["names"]:
                    c = check_dns(env, n)
                    job.line("%s: %s" % (c["record"], c["message"]))
                    if c["state"] in ("missing", "wrong"):
                        bad.append(c["record"])
                if bad:
                    raise EdgeError("Create the CNAME record first: %s should point at the challenge service (see the target in the dialog)." % ", ".join(sorted(set(bad))))
        else:
            port = 80 if req["challenge"] == "http-01" else 443
            if self.edge.proxy.enabled():
                if not self.edge.proxy.front_active():
                    raise EdgeError("The HAProxy front is enabled but not running: it must answer the %s challenge." % req["challenge"])
                if env.port_busy(ACME_HTTP_PORT if req["challenge"] == "http-01" else ACME_ALPN_PORT):
                    raise EdgeError("lego's challenge port is already in use.")
            elif env.port_busy(port):
                raise EdgeError("Port %d is in use by another program. Free it, or enable the HAProxy front, which answers the challenge for you." % port)
        say("tools", "Checking lego")
        if not env.which("lego"):
            apt_install(env, job, "lego", "Let's Encrypt client")
        say("challenge", "Preparing the %s challenge" % req["challenge"])
        legopath = env.p(LEGO_DIR + "/" + req["environment"])
        mkdirs(env.p(LEGO_DIR), 0o700)
        mkdirs(legopath, 0o700)
        child_env: dict = {}
        if req["challenge"] == "dns-01":
            child_env = lego_env(ensure_adapter(env))
        stem = lego_file_stem(req["names"][0])
        have = os.path.isfile(os.path.join(legopath, "certificates", stem + ".crt"))
        command = "renew" if req["command"] == "renew" and have else "run"
        http_l, alpn_l = self.listen_addrs()
        resolvers = [x for x in s["dns_resolvers"].split(",") if x]
        argv = lego_argv(env.which("lego") or "lego", command=command, environment=req["environment"], email=s["email"], path=legopath, names=req["names"],
                         challenge=req["challenge"], key_type=req["key_type"], http_listen=http_l, alpn_listen=alpn_l, dns_skip_check=s["dns_skip_check"], resolvers=resolvers,
                         days=req.get("days", RENEW_DAYS))
        say("obtain", "Asking Let's Encrypt (%s, %s)" % (req["environment"], req["challenge"]))
        job.line("$ " + " ".join(argv[:1] + [a for a in argv[1:] if not a.startswith("/etc/synofurb")]))
        res = env.stream(argv, job.line, 1200, child_env)
        if not res.ok:
            kind, friendly = explain_lego_error(res.out + "\n" + res.err)
            last = job.scrub(([ln for ln in (res.out or res.err).splitlines() if ln.strip()] or [""])[-1])[:200]
            self.record_event(req["names"], req["environment"], False)
            raise EdgeError("%s%s" % (friendly, (" (" + last + ")") if last else ""))
        say("verify", "Checking the new certificate")
        crt = read_text(os.path.join(legopath, "certificates", stem + ".crt"))
        key = read_text(os.path.join(legopath, "certificates", stem + ".key"))
        if not crt or "PRIVATE KEY" not in key:
            raise EdgeError("lego finished but left no certificate files (%s)" % stem)
        info = parse_cert(crt)
        missing = [n for n in req["names"] if n not in info["sans"]]
        if missing:
            raise EdgeError("the certificate does not cover: %s" % ", ".join(missing))
        if info["not_after"] <= env.now() + 3600:
            raise EdgeError("the new certificate is already expired")
        say("store", "Storing it")
        now = int(env.now())
        prev = self.store.meta(cid)
        meta = {"schema": SCHEMA, "id": cid, "primary": req["names"][0], "names": req["names"], "challenge": req["challenge"], "environment": req["environment"], "key_type": req["key_type"],
                "created": (prev or {}).get("created") or now, "renewed": now if prev and prev.get("expires") else None, "not_before": info["not_before"], "expires": info["not_after"],
                "fingerprint": info["fingerprint"], "last_ok": now, "last_attempt": now, "last_error": ""}
        self.store.write_pems(cid, crt, key)
        self.store.save_meta(cid, meta)
        if os.path.isdir(env.p(HAPROXY_DIR)) and not (self.edge.proxy.enabled() and self.edge.proxy.used_by(cid)):
            self.store.write_combined(cid)                       # a certificate the front serves is replaced only after haproxy -c accepted it (certs_changed)
        self.record_event(req["names"], req["environment"], True)
        job.line("expires %s (%s)" % (time.strftime("%Y-%m-%d", time.gmtime(info["not_after"])), req["environment"]))

    # -- renewal of everything that is due -----------------------------------------------------------------------
    def due(self, cid: str, force: bool = False) -> tuple[bool, str]:
        m = self.store.meta(cid)
        if m is None or m.get("expires") is None:
            return False, "no certificate yet"
        now = self.env.now()
        left = days_left(m["expires"], now)
        if not force and left is not None and left > RENEW_DAYS:
            return False, "%d days left" % left
        if not force and m.get("last_error") and now - (m.get("last_attempt") or 0) < RETRY_AFTER_FAILURE:
            return False, "failed recently, retrying later"
        return True, "%s days left" % left

    def renew_all(self, user: str | None = "timer", ip: str = "-", force: bool = False, only: str | None = None, background: bool = False,
                  sink: Callable[[str], None] | None = None) -> Job:
        ids = [valid_cert_id(only)] if only else self.store.ids()
        if only and self.store.meta(only) is None:
            raise NotFound("no such certificate")
        return self._start("renew-all", "*", ["check", "renew", "publish"], user, ip, {"force": force, "only": only}, lambda job: self._renew_all_job(job, ids, force), background, sink, "renew")

    def _renew_all_job(self, job: Job, ids: list[str], force: bool) -> None:
        job.step("check", "Looking for certificates to renew")
        results, changed, failed = [], [], 0
        todo = []
        for cid in ids:
            ok, why = self.due(cid, force)
            if ok:
                todo.append(cid)
            else:
                results.append({"id": cid, "result": "skipped", "message": why})
        job.line("%d certificate(s), %d due" % (len(ids), len(todo)))
        job.step("renew", "Renewing")
        for cid in todo:
            m = self.store.meta(cid) or {}
            req = {"id": cid, "names": m["names"], "challenge": m["challenge"], "environment": m["environment"], "key_type": m.get("key_type") or "ec256", "command": "renew",
                   "days": 36500 if force else RENEW_DAYS, "skip_dns_check": False}
            job.line("-- %s" % cid)
            try:
                self._obtain(job, req, announce=False)
                changed.append(cid)
                results.append({"id": cid, "result": "renewed", "message": ""})
            except EdgeError as e:
                failed += 1
                msg = job.scrub(str(e))[:300]
                job.line("FAILED %s: %s" % (cid, msg))
                prev = self.store.meta(cid) or m
                prev.update(last_attempt=int(self.env.now()), last_error=msg)
                self.store.save_meta(cid, prev)
                results.append({"id": cid, "result": "failed", "message": msg})
        job.step("publish", "Publishing")
        if changed:
            warn = self.edge.proxy.certs_changed(changed, job)
            if warn:
                job.warnings.append(warn)
            self.ensure_timer(job)
        rec = {"ts": int(self.env.now()), "ok": failed == 0, "renewed": len(changed), "failed": failed, "results": results}
        try:
            write_json(self.env.state_path("renew.json"), rec)
        except OSError:
            pass
        job.result = rec
        if failed:
            raise EdgeError("%d of %d renewals failed" % (failed, len(todo)))

    # -- remove ------------------------------------------------------------------------------------------------------
    def remove(self, cid: str, confirm: str, user: str | None = None, ip: str = "-") -> dict:
        cid = valid_cert_id(cid)
        m = self.store.meta(cid)
        if m is None:
            raise NotFound("no such certificate")
        if confirm != cid:
            raise EdgeError("type the certificate name %r to confirm" % cid)
        users = self.used_by(cid)
        if users:
            raise EdgeError("it is used by %s: choose another certificate for those hostnames first" % ", ".join(u["hostname"] for u in users))
        if self.jobs.active():
            raise Busy("Another job is running")
        self.store.remove(cid)
        unlink(self.store.combined_path(cid))
        for envname in ENVIRONMENTS:
            for ext in (".crt", ".key", ".issuer.crt", ".json"):
                unlink(os.path.join(self.env.p(LEGO_DIR), envname, "certificates", lego_file_stem(m["primary"]) + ext))
        self.env.audit(user, ip, "certs.remove", {"id": cid}, "ok")
        return {"ok": True, "id": cid}

    # -- renewal timer ---------------------------------------------------------------------------------------------------
    def timer_state(self) -> dict:
        have = os.path.isfile(self.env.p(UNIT_DIR + "/" + RENEW_UNIT + ".timer"))
        enabled = self.env.run(["systemctl", "is-enabled", RENEW_UNIT + ".timer"], timeout=10).out.strip() == "enabled" if have else False
        return {"installed": have, "enabled": enabled}

    def ensure_timer(self, job: Job | None = None) -> None:
        """Write the systemd timer + service pair (twice a day, random delay) and enable it.  Idempotent."""
        service, timer = renewal_units(self.env.script_path)
        changed = False
        for name, text in ((RENEW_UNIT + ".service", service), (RENEW_UNIT + ".timer", timer)):
            path = self.env.p(UNIT_DIR + "/" + name)
            if read_text(path) != text:
                write_file(path, text, 0o644, 0o755)
                changed = True
        if changed:
            self.env.run(["systemctl", "daemon-reload"], timeout=30)
        if changed or not self.timer_state()["enabled"]:
            r = self.env.run(["systemctl", "enable", "--now", RENEW_UNIT + ".timer"], timeout=30)
            if not r.ok and job:
                job.warnings.append("The renewal timer could not be enabled: %s" % (r.err or r.out).strip()[:160])
            elif job:
                job.line("renewal timer enabled (twice a day)")


def _is_private(ip: str) -> bool:
    try:
        a = ipaddress.ip_address(ip)
    except ValueError:
        return False
    return a.is_private or a.is_loopback or a.is_link_local


def renewal_units(script_path: str) -> tuple[str, str]:
    """The service and timer text.  ExecStart runs the management plane's own CLI; the random delay keeps all units off the same minute."""
    if not re.fullmatch(r"/[A-Za-z0-9._/-]{1,200}", script_path):
        raise EdgeError("bad script path")
    service = ("[Unit]\nDescription=Renew Let's Encrypt certificates (synofurb-mgmt)\nAfter=network-online.target\nWants=network-online.target\n\n"
               "[Service]\nType=oneshot\nExecStart=/usr/bin/python3 %s certs renew\nNice=10\nTimeoutStartSec=1h\n" % script_path)
    timer = ("[Unit]\nDescription=Renew Let's Encrypt certificates twice a day (synofurb-mgmt)\n\n"
             "[Timer]\nOnCalendar=*-*-* 03,15:00:00\nRandomizedDelaySec=3h\nPersistent=true\nAccuracySec=1min\n\n[Install]\nWantedBy=timers.target\n")
    return service, timer


# ============================================================================
# 6. Health (the `certs` section of `synofurb_mgmt.py health --json`)
# ============================================================================

def health_section(env: EdgeEnv) -> tuple[dict | None, list[str], list[str]]:
    """(section, warnings, criticals).  None when no certificate exists, so units without certificates keep exactly the old document."""
    store = CertStore(env)
    ids = store.ids()
    if not ids:
        return None, [], []
    now = env.now()
    warn, crit, expiring, failed = [], [], [], []
    mins = []
    for cid in ids:
        m = store.meta(cid) or {}
        exp = m.get("expires")
        if exp is None:
            if m.get("last_error"):
                failed.append(cid)
                warn.append("certificate %s: issue failed" % cid)
            continue
        left = days_left(exp, now)
        mins.append(left)
        if left is not None and left < 0:
            crit.append("certificate %s expired" % cid)
            expiring.append({"id": cid, "days": left})
        elif left is not None and left <= CRIT_DAYS:
            crit.append("certificate %s expires in %d days" % (cid, left))
            expiring.append({"id": cid, "days": left})
        elif left is not None and left <= WARN_DAYS:
            warn.append("certificate %s expires in %d days" % (cid, left))
            expiring.append({"id": cid, "days": left})
        elif m.get("last_error") and left is not None and left <= RENEW_DAYS:
            warn.append("certificate %s: renewal failed" % cid)
            failed.append(cid)
    return {"count": len(ids), "min_days": min(mins) if mins else None, "expiring": expiring, "failed": failed}, warn, crit



# ============================================================================
# 7. The HAProxy front: pure rendering, safe apply (haproxy -c, snapshot, revert timer), enable / disable / revert
# ============================================================================
#
# What the front does (docs/CERTIFICATES.md, "HAProxy front"):
#   :80   HTTP-01 answers (/.well-known/acme-challenge/ -> lego on loopback), everything else redirects to https
#   :443  TCP level: ALPN acme-tls/1 -> lego (TLS-ALPN-01); everything else -> the TLS terminator on 127.0.0.1:8444 (PROXY protocol, keeps the client address)
#   :8444 HTTP level, loopback only: terminates TLS (SNI picks the certificate from the crt-list), the Host header picks the backend:
#         one backend per add-on `web` (the hostname the person assigned) and the management plane (its hostname, and the default for any other name
#         or an IP address, so the unit is always reachable)
#   The management plane then listens on 127.0.0.1:<port> only (re-encrypted, self-signed), trusts X-Forwarded-For from loopback, and is reached through 443.

TLS_TERM_PORT = 8444
MGMT_JSON = "/etc/synofurb/mgmt.json"
SSL_CIPHERS = ("ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:"
               "ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305")
SSL_SUITES = "TLS_AES_128_GCM_SHA256:TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256"
PROXY_DEFAULTS = {"schema": SCHEMA, "enabled": False, "previous_bind": None, "mgmt": {"hostname": "", "cert": ""}, "sites": {}}


def render_haproxy(spec: dict, paths: dict) -> tuple[str, str]:
    """(haproxy.cfg, crt-list) for ``spec`` = {mgmt_port, mgmt: {hostname, cert}, sites: [{id, hostname, cert, port, scheme}], certs: {id: [hostnames]}}.
    Pure and deterministic.  Every value that is written is validated here again (hostnames by valid_domain, ids by their regex, ports as integers),
    so no input can add a line to the configuration."""
    mgmt_port = int(spec["mgmt_port"])
    if not 1 <= mgmt_port <= 65535:
        raise EdgeError("bad management port")
    for key in ("crt_list", "default_pem", "pem_dir"):
        if not re.fullmatch(r"/[A-Za-z0-9._/@+-]{1,300}", paths[key]):
            raise EdgeError("bad path for " + key)
    sites = sorted(spec["sites"], key=lambda s: s["id"])
    seen = set()
    rules, backends = [], []
    mg = spec["mgmt"]
    if mg.get("hostname"):
        host = valid_domain(mg["hostname"])
        seen.add(host)
        rules.append("    use_backend be_mgmt if { req.hdr(host),field(1,:),lower -m str %s }" % host)
    for s in sites:
        if not ADDON_ID_RE.fullmatch(s["id"]) or s["scheme"] not in ("http", "https") or not 1 <= int(s["port"]) <= 65535:
            raise EdgeError("bad add-on entry")
        host = valid_domain(s["hostname"])
        if host in seen:
            raise EdgeError("%s is assigned twice" % host)
        seen.add(host)
        rules.append("    use_backend be_addon_%s if { req.hdr(host),field(1,:),lower -m str %s }" % (s["id"], host))
        backends.append("backend be_addon_%s\n    mode http\n    http-request set-header X-Forwarded-Proto https\n    server %s %s:%d%s\n"
                        % (s["id"], s["id"], MGMT_LOOPBACK, int(s["port"]), " ssl verify none" if s["scheme"] == "https" else ""))
    lines = [MARKER, "",
             "global", "    log /dev/log local0", "    log /dev/log local1 notice", "    chroot /var/lib/haproxy", "    user haproxy", "    group haproxy", "    daemon",
             "    ssl-default-bind-options ssl-min-ver TLSv1.2 no-tls-tickets", "    ssl-default-bind-ciphersuites " + SSL_SUITES, "    ssl-default-bind-ciphers " + SSL_CIPHERS, "",
             "defaults", "    log global", "    mode http", "    timeout connect 5s", "    timeout client 5m", "    timeout server 5m", "    timeout tunnel 1h",
             "    timeout http-request 30s", "    timeout http-keep-alive 10s", "    retries 2", "",
             "# Port 80: HTTP-01 challenges go to lego on loopback, everything else is redirected to https.",
             "frontend fe_http", "    mode http", "    option httplog", "    bind 0.0.0.0:80", "    bind :::80 v6only",
             "    acl acme_challenge path_beg /.well-known/acme-challenge/", "    use_backend be_acme_http if acme_challenge",
             "    http-request redirect scheme https code 301 unless acme_challenge", "",
             "backend be_acme_http", "    mode http", "    server lego %s:%d" % (MGMT_LOOPBACK, ACME_HTTP_PORT), "",
             "# Port 443, TCP level: a connection that offers the ALPN protocol acme-tls/1 is a TLS-ALPN-01 validation and goes to lego;",
             "# everything else goes to the TLS terminator below (PROXY protocol keeps the client address).",
             "frontend fe_tls", "    mode tcp", "    option tcplog", "    bind 0.0.0.0:443", "    bind :::443 v6only", "    tcp-request inspect-delay 5s",
             "    tcp-request content accept if { req.ssl_hello_type 1 }", "    use_backend be_acme_alpn if { req.ssl_alpn acme-tls/1 }", "    default_backend be_tls_terminator", "",
             "backend be_acme_alpn", "    mode tcp", "    server lego %s:%d" % (MGMT_LOOPBACK, ACME_ALPN_PORT), "",
             "backend be_tls_terminator", "    mode tcp", "    server terminator %s:%d send-proxy-v2" % (MGMT_LOOPBACK, TLS_TERM_PORT), "",
             "# The TLS terminator (loopback only). SNI picks the certificate (crt-list); the Host header picks the backend.",
             "# Any other name, or an IP address, reaches the management plane, so the unit stays reachable.",
             "frontend fe_https", "    mode http", "    option httplog", "    bind %s:%d accept-proxy ssl crt-list %s alpn h2,http/1.1" % (MGMT_LOOPBACK, TLS_TERM_PORT, paths["crt_list"]),
             "    http-request set-header X-Forwarded-For %[src]", "    http-request set-header X-Forwarded-Proto https"]
    lines += rules + ["    default_backend be_mgmt", "",
                      "backend be_mgmt", "    mode http", "    server mgmt %s:%d ssl verify none" % (MGMT_LOOPBACK, mgmt_port), ""]
    for b in backends:
        lines += b.rstrip("\n").split("\n") + [""]
    cfg = "\n".join(lines).rstrip("\n") + "\n"
    crt = [CRT_MARKER, "# The first line is the default certificate (the management plane's own); the others are chosen by SNI.", paths["default_pem"]]
    for cid in sorted(spec["certs"]):
        valid_cert_id(cid)
        hosts = sorted(valid_domain(h) for h in spec["certs"][cid])
        if hosts:
            crt.append("%s %s" % (paths["pem_dir"].rstrip("/") + "/" + cid + ".pem", " ".join(hosts)))
    return cfg, "\n".join(crt) + "\n"


def parse_haproxy_version(text: str) -> str | None:
    m = re.search(r"HAProxy version (\S+)", text)
    return m.group(1) if m else None


class ProxyManager:
    STEPS = ["tools", "config", "check", "apply", "listener", "confirm"]

    def __init__(self, env: EdgeEnv, edge: Any):
        self.env, self.edge = env, edge

    # -- state ---------------------------------------------------------------------------------
    def state(self) -> dict:
        d = read_json(self.env.edge_path("proxy.json"), {})
        out = json.loads(json.dumps(PROXY_DEFAULTS))
        if isinstance(d, dict):
            out["enabled"] = d.get("enabled") is True
            pb = d.get("previous_bind")
            out["previous_bind"] = pb if isinstance(pb, str) and len(pb) <= 64 else None
            mg = d.get("mgmt")
            if isinstance(mg, dict):
                out["mgmt"] = {"hostname": mg.get("hostname") if isinstance(mg.get("hostname"), str) else "", "cert": mg.get("cert") if isinstance(mg.get("cert"), str) else ""}
            sites = d.get("sites")
            if isinstance(sites, dict):
                for k, v in sites.items():
                    if isinstance(k, str) and ADDON_ID_RE.fullmatch(k) and isinstance(v, dict):
                        out["sites"][k] = {"hostname": v.get("hostname") if isinstance(v.get("hostname"), str) else "", "cert": v.get("cert") if isinstance(v.get("cert"), str) else "",
                                           "scheme": v.get("scheme") if v.get("scheme") in ("http", "https") else ""}
        return out

    def save_state(self, st: dict) -> None:
        write_json(self.env.edge_path("proxy.json"), st)

    def enabled(self) -> bool:
        return self.state()["enabled"]

    def front_active(self) -> bool:
        return self.env.run(["systemctl", "is-active", "haproxy"], timeout=10).out.strip() == "active"

    def used_by(self, cid: str) -> list[dict]:
        st = self.state()
        out = []
        if st["mgmt"]["hostname"] and st["mgmt"]["cert"] == cid:
            out.append({"kind": "mgmt", "id": "mgmt", "hostname": st["mgmt"]["hostname"]})
        for aid, s in sorted(st["sites"].items()):
            if s["hostname"] and s["cert"] == cid:
                out.append({"kind": "addon", "id": aid, "hostname": s["hostname"]})
        return out

    # -- hostnames ---------------------------------------------------------------------------------
    def normalize(self, body: dict) -> dict:
        """Merge ``{mgmt: {hostname, cert}, sites: {id: {hostname, cert, scheme?}}}`` into the saved state, validated.  Returns the new state (not yet saved)."""
        st = self.state()
        store = self.edge.certs.store
        known = {s["id"]: s for s in self.env.sites()}
        entries: list[tuple[str, str, dict]] = []
        if isinstance(body.get("mgmt"), dict):
            entries.append(("mgmt", "mgmt", body["mgmt"]))
        sites = body.get("sites") or {}
        if not isinstance(sites, dict):
            raise EdgeError("sites must be an object")
        for aid, v in sites.items():
            if not isinstance(aid, str) or aid not in known:
                raise EdgeError("%s has no web page to publish" % str(aid)[:40])
            if not isinstance(v, dict):
                raise EdgeError("bad entry for %s" % aid)
            entries.append(("addon", aid, v))
        for kind, aid, v in entries:
            host = v.get("hostname", "")
            cert = v.get("cert", "")
            if not isinstance(host, str) or not isinstance(cert, str):
                raise EdgeError("hostname and certificate must be text")
            host = valid_domain(host) if host.strip() else ""
            cert = valid_cert_id(cert) if cert else ""
            if host:
                if not cert:
                    raise EdgeError("choose a certificate for %s" % host)
                m = store.meta(cert)
                if m is None or m.get("expires") is None:
                    raise EdgeError("the certificate %s is not issued yet" % cert)
                if not covers(m["names"], host):
                    raise EdgeError("the certificate %s does not cover %s" % (cert, host))
            else:
                cert = ""
            if kind == "mgmt":
                st["mgmt"] = {"hostname": host, "cert": cert}
            else:
                scheme = v.get("scheme", st["sites"].get(aid, {}).get("scheme", ""))
                if scheme not in ("", "http", "https"):
                    raise EdgeError("scheme must be http or https")
                st["sites"][aid] = {"hostname": host, "cert": cert, "scheme": scheme}
        hosts = [st["mgmt"]["hostname"]] + [s["hostname"] for s in st["sites"].values()]
        hosts = [h for h in hosts if h]
        if len(hosts) != len(set(hosts)):
            raise EdgeError("a hostname can be assigned only once")
        for aid in list(st["sites"]):
            if aid not in known:
                st["sites"].pop(aid)
        return st

    def rows(self, st: dict | None = None) -> list[dict]:
        st = st or self.state()
        rows = [{"kind": "mgmt", "id": "mgmt", "name": "", "hostname": st["mgmt"]["hostname"], "cert": st["mgmt"]["cert"], "hint": "", "scheme": "https", "port": self.env.mgmt_port}]
        for s in sorted(self.env.sites(), key=lambda x: x["id"]):
            cur = st["sites"].get(s["id"], {"hostname": "", "cert": "", "scheme": ""})
            rows.append({"kind": "addon", "id": s["id"], "name": s["name"], "icon": s.get("icon", ""), "hostname": cur["hostname"], "cert": cur["cert"], "hint": s.get("hint", ""),
                         "scheme": cur["scheme"] or s["scheme"], "port": s["port"]})
        return rows

    # -- rendering ------------------------------------------------------------------------------
    def default_pem_sources(self) -> tuple[str, str]:
        d = self.env.p(self.env.state_dir + "/tls")
        return d + "/cert.pem", d + "/key.pem"

    def build(self, st: dict, paths: dict) -> tuple[str, str, dict[str, str]]:
        """(haproxy.cfg, crt-list, {pem path: content}) for ``st``.  Raises EdgeError when something it needs is missing."""
        store = self.edge.certs.store
        crt_src, key_src = self.default_pem_sources()
        crt, key = read_text(crt_src), read_text(key_src)
        if not crt or "PRIVATE KEY" not in key:
            raise EdgeError("The management plane has no TLS certificate yet: start it once, then try again.")
        pems = {paths["default_pem"]: crt.rstrip("\n") + "\n" + key.rstrip("\n") + "\n"}
        sites_by_id = {s["id"]: s for s in self.env.sites()}
        certs: dict[str, list[str]] = {}
        spec_sites = []
        mg = st["mgmt"]
        if mg["hostname"]:
            certs.setdefault(mg["cert"], []).append(mg["hostname"])
        for aid, s in sorted(st["sites"].items()):
            if not s["hostname"] or aid not in sites_by_id:
                continue
            certs.setdefault(s["cert"], []).append(s["hostname"])
            spec_sites.append({"id": aid, "hostname": s["hostname"], "cert": s["cert"], "port": sites_by_id[aid]["port"], "scheme": s["scheme"] or sites_by_id[aid]["scheme"]})
        for cid in certs:
            p = store.read_pems(cid)
            if not p:
                raise EdgeError("the certificate %s has no files" % cid)
            pems[paths["pem_dir"].rstrip("/") + "/" + cid + ".pem"] = p[0].rstrip("\n") + "\n" + p[1].rstrip("\n") + "\n"
        spec = {"mgmt_port": self.env.mgmt_port, "mgmt": mg, "sites": spec_sites, "certs": certs}
        cfg, crt_list = render_haproxy(spec, paths)
        return cfg, crt_list, pems

    def final_paths(self) -> dict:
        return {"cfg": self.env.p(HAPROXY_CFG), "crt_list": self.env.p(HAPROXY_CRT_LIST), "pem_dir": self.env.p(HAPROXY_CERTS), "default_pem": self.env.p(HAPROXY_CERTS + "/_default.pem")}

    def check_candidate(self, st: dict) -> dict:
        """Render ``st`` into a private temp directory and run ``haproxy -c`` on it.  Nothing live is touched."""
        import tempfile
        env = self.env
        mkdirs(env.state_path(""), 0o700)
        tmp = tempfile.mkdtemp(prefix="check-", dir=env.state_path(""))
        try:
            paths = {"cfg": tmp + "/haproxy.cfg", "crt_list": tmp + "/crt-list", "pem_dir": tmp + "/certs", "default_pem": tmp + "/certs/_default.pem"}
            cfg, crt_list, pems = self.build(st, paths)
            os.mkdir(tmp + "/certs", 0o700)
            for p, text in pems.items():
                write_file(p, text, 0o600)
            write_file(paths["crt_list"], crt_list, 0o600)
            write_file(paths["cfg"], cfg, 0o600)
            if not env.which("haproxy"):
                return {"ok": None, "installed": False, "config": cfg, "output": ""}
            r = env.run(["haproxy", "-c", "-f", paths["cfg"]], timeout=30)
            text = ((r.err + "\n" + r.out).strip())
            return {"ok": r.ok, "installed": True, "config": cfg, "output": text[-1500:]}
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    def test(self, body: dict | None = None) -> dict:
        st = self.normalize(body) if body else self.state()
        return self.check_candidate(st)

    # -- status ------------------------------------------------------------------------------------------
    def haproxy_status(self) -> dict:
        env = self.env
        if not env.which("haproxy"):
            return {"installed": False, "version": None, "active": False, "enabled": False}
        v = env.run(["haproxy", "-v"], timeout=10)
        act = env.run(["systemctl", "is-active", "haproxy"], timeout=10).out.strip() == "active"
        en = env.run(["systemctl", "is-enabled", "haproxy"], timeout=10).out.strip() == "enabled"
        return {"installed": True, "version": parse_haproxy_version(v.out), "active": act, "enabled": en}

    def urls(self, st: dict | None = None) -> list[str]:
        st = st or self.state()
        out = []
        if st["mgmt"]["hostname"]:
            out.append("https://%s/" % st["mgmt"]["hostname"])
        out += ["https://%s/" % (("[%s]" % ip) if ":" in ip else ip) for ip in self.env.local_ips() if not ip.startswith(("127.", "::1", "fe80"))][:3]
        return out

    def pending(self) -> dict | None:
        p = read_json(self.env.state_path("pending.json"), None)
        if not isinstance(p, dict) or not isinstance(p.get("deadline"), (int, float)):
            return None
        return {"kind": p.get("kind", ""), "deadline": p["deadline"], "seconds_left": max(0, int(p["deadline"] - self.env.now())), "seconds": p.get("seconds", REVERT_SECONDS)}

    def view(self) -> dict:
        st = self.state()
        certs = [{"id": c, "names": (self.edge.certs.store.meta(c) or {}).get("names", []), "expires": (self.edge.certs.store.meta(c) or {}).get("expires")} for c in self.edge.certs.store.ids()]
        act = self.edge.certs.jobs.active()
        return {"enabled": st["enabled"], "rows": self.rows(st), "certs": [c for c in certs if c["expires"]], "haproxy": self.haproxy_status(), "pending": self.pending(),
                "urls": self.urls(st), "mgmt_port": self.env.mgmt_port, "reverted": read_json(self.env.state_path("reverted.json"), None), "job": act.view(False) if act else None,
                "recovery": "python3 %s proxy disable" % self.env.script_path}

    # -- the snapshot / revert machinery (the forgevault pattern) --------------------------------------------------
    def snap_dir(self) -> str:
        return self.env.state_path("snap")

    def _snapshot(self) -> None:
        env = self.env
        sd = self.snap_dir()
        shutil.rmtree(sd, ignore_errors=True)
        mkdirs(sd, 0o700)
        meta: dict = {}
        cfg_now, original = env.p(HAPROXY_CFG), env.edge_path("haproxy.cfg.original")
        if os.path.isfile(cfg_now) and not os.path.exists(original) and MARKER not in read_text(cfg_now):
            write_file(original, read_text(cfg_now), 0o600)          # a configuration that was not ours is kept for good, once, before the first overwrite
        for key, src in (("cfg", env.p(HAPROXY_CFG)), ("crt_list", env.p(HAPROXY_CRT_LIST)), ("mgmt_json", env.p(MGMT_JSON)), ("proxy_json", env.edge_path("proxy.json"))):
            if os.path.isfile(src):
                shutil.copyfile(src, os.path.join(sd, key))
                os.chmod(os.path.join(sd, key), 0o600)
                meta[key] = True
        pem_dir = env.p(HAPROXY_CERTS)
        mkdirs(sd + "/certs", 0o700)
        meta["pems"] = []
        if os.path.isdir(pem_dir):
            for n in sorted(os.listdir(pem_dir)):
                if n.endswith(".pem") and os.path.isfile(os.path.join(pem_dir, n)) and not os.path.islink(os.path.join(pem_dir, n)):
                    shutil.copyfile(os.path.join(pem_dir, n), sd + "/certs/" + n)
                    os.chmod(sd + "/certs/" + n, 0o600)
                    meta["pems"].append(n)
        meta["was_active"] = self.front_active()
        meta["was_enabled"] = env.run(["systemctl", "is-enabled", "haproxy"], timeout=10).out.strip() == "enabled"
        write_json(sd + "/meta.json", meta)

    def _restore(self) -> list[str]:
        """Put the snapshot back (files, then the service state).  Returns problems (empty = fine)."""
        env, sd = self.env, self.snap_dir()
        meta = read_json(sd + "/meta.json", None)
        if not isinstance(meta, dict):
            return ["no snapshot to restore"]
        problems = []
        pem_dir = env.p(HAPROXY_CERTS)
        current_mgmt = read_text(env.p(MGMT_JSON))
        for key, dest in (("cfg", env.p(HAPROXY_CFG)), ("crt_list", env.p(HAPROXY_CRT_LIST)), ("mgmt_json", env.p(MGMT_JSON)), ("proxy_json", env.edge_path("proxy.json"))):
            if meta.get(key):
                write_file(dest, read_text(os.path.join(sd, key)), 0o600 if key != "cfg" else 0o644, 0o755)
            else:
                unlink(dest)
        if os.path.isdir(pem_dir):
            for n in os.listdir(pem_dir):
                if n.endswith(".pem") and n not in meta.get("pems", []):
                    unlink(os.path.join(pem_dir, n))
        for n in meta.get("pems", []):
            write_file(os.path.join(pem_dir, n), read_text(sd + "/certs/" + n), 0o600, 0o755)
        if meta.get("was_active"):
            r = env.run(["systemctl", "reload", "haproxy"], timeout=30)
            if not r.ok:
                r = env.run(["systemctl", "restart", "haproxy"], timeout=60)
            if not r.ok:
                problems.append("haproxy did not reload: %s" % (r.err or r.out).strip()[:120])
        else:
            env.run(["systemctl", "disable", "--now", "haproxy"], timeout=60)
        if read_text(env.p(MGMT_JSON)) != current_mgmt:
            env.defer(self.env.restart_mgmt, 2.0)
        return problems

    def _stop_timer(self) -> None:
        self.env.run(["systemctl", "stop", REVERT_UNIT + ".timer", REVERT_UNIT + ".service"], timeout=20)
        self.env.run(["systemctl", "reset-failed", REVERT_UNIT + ".timer", REVERT_UNIT + ".service"], timeout=20)

    def _arm(self, seconds: int) -> None:
        env = self.env
        self._stop_timer()
        script = env.script_path
        if not re.fullmatch(r"/[A-Za-z0-9._/-]{1,200}", script):
            raise EdgeError("bad script path")
        r = env.run(["systemd-run", "--unit=" + REVERT_UNIT, "--collect", "--on-active=%ds" % seconds, "/usr/bin/python3", script, "proxy", "revert", "--auto"], timeout=20)
        if not r.ok:
            raise EdgeError("Could not arm the automatic revert (nothing was applied): %s" % (r.err or r.out).strip()[:160])
        write_json(env.state_path("pending.json"), {"deadline": env.now() + seconds, "seconds": seconds, "kind": "apply"})

    def confirm(self, user: str | None = None, ip: str = "-") -> dict:
        if self.pending() is None:
            raise EdgeError("Nothing waits for confirmation: it may have been reverted already.")
        self._stop_timer()
        unlink(self.env.state_path("pending.json"))
        shutil.rmtree(self.snap_dir(), ignore_errors=True)
        self.env.audit(user, ip, "proxy.confirm", {}, "ok")
        return {"ok": True}

    def revert(self, auto: bool = False, user: str | None = None, ip: str = "-") -> dict:
        env = self.env
        if self.pending() is None and not os.path.isfile(self.snap_dir() + "/meta.json"):
            raise EdgeError("Nothing to revert.")
        self._stop_timer()
        problems = self._restore()
        unlink(env.state_path("pending.json"))
        shutil.rmtree(self.snap_dir(), ignore_errors=True)
        write_json(env.state_path("reverted.json"), {"ts": int(env.now()), "auto": auto, "problems": problems})
        env.audit(user or ("timer" if auto else None), ip, "proxy.revert", {"auto": auto}, "ok" if not problems else "problems: " + "; ".join(problems)[:200])
        return {"ok": not problems, "problems": problems}

    def startup_recovery(self) -> bool:
        """A pending change whose deadline passed while the unit was down (a transient timer does not survive a reboot) is reverted at startup."""
        p = self.pending()
        if p and self.env.now() > p["deadline"]:
            self.revert(auto=True)
            return True
        return False

    # -- listener of the management plane --------------------------------------------------------------------------------------
    def _set_listener(self, st: dict, loopback: bool) -> None:
        path = self.env.p(MGMT_JSON)
        data = read_json(path, {})
        data = data if isinstance(data, dict) else {}
        if loopback:
            if st["previous_bind"] is None and "bind" in data and data["bind"] != MGMT_LOOPBACK:
                st["previous_bind"] = str(data["bind"])
            data["bind"], data["trust_proxy"] = MGMT_LOOPBACK, True
        else:
            if st["previous_bind"]:
                data["bind"] = st["previous_bind"]
            else:
                data.pop("bind", None)
            data.pop("trust_proxy", None)
            st["previous_bind"] = None
        mode = 0o600
        try:
            mode = os.stat(path).st_mode & 0o777
        except OSError:
            pass
        write_file(path, json.dumps(data, indent=1, sort_keys=True) + "\n", mode, 0o755)

    # -- apply (enable / apply / disable) ----------------------------------------------------------------------------------------
    def apply(self, kind: str, body: dict | None = None, user: str | None = None, ip: str = "-", background: bool = True, sink: Callable[[str], None] | None = None,
              seconds: int = REVERT_SECONDS) -> Job:
        if kind not in ("enable", "apply", "disable"):
            raise EdgeError("bad action")
        st = self.state()
        if kind == "apply" and not st["enabled"]:
            raise EdgeError("The HAProxy front is off: enable it first.")
        if kind == "enable" and st["enabled"]:
            raise EdgeError("The HAProxy front is on already: use Apply.")
        if kind == "disable" and not st["enabled"]:
            raise EdgeError("The HAProxy front is off already.")
        cand = self.normalize(body or {}) if kind != "disable" else None
        steps = ["listener", "stop"] if kind == "disable" else self.STEPS
        return self.edge.certs._start("proxy-" + kind, "proxy", steps, user, ip, {"kind": kind, "hostnames": bool(body)},
                                      lambda job: (self._disable_job(job) if kind == "disable" else self._apply_job(job, kind, cand, seconds)), background, sink, "proxy " + kind)

    def _apply_job(self, job: Job, kind: str, cand: dict, seconds: int) -> None:
        env = self.env
        job.step("tools", "Checking HAProxy")
        if not env.which("haproxy"):
            apt_install(env, job, "haproxy", "the front for the management plane and the add-ons")
        if kind == "enable":
            for port in (80, 443, TLS_TERM_PORT):
                if env.port_busy(port) and not self.front_active():
                    raise EdgeError("Port %d is in use by another program. Free it, then enable the front." % port)
        job.step("config", "Building the configuration")
        paths = self.final_paths()
        job.step("check", "Checking it with haproxy -c")
        res = self.check_candidate(cand)
        job.line("haproxy -c: %s" % ("ok" if res["ok"] else "rejected"))
        if not res["ok"]:
            for ln in res["output"].splitlines()[-8:]:
                job.line(ln)
            raise EdgeError("HAProxy rejected the configuration: %s. Nothing was changed." % (([ln for ln in res["output"].splitlines() if ln.strip()] or ["no details"])[-1].strip()[:160]))
        cfg, crt_list, pems = self.build(cand, paths)
        job.step("apply", "Applying it")
        if self.pending() is None:
            self._snapshot()
        pem_dir = env.p(HAPROXY_CERTS)
        mkdirs(pem_dir, 0o755)
        try:
            for p, text in pems.items():
                write_file(p, text, 0o600, 0o755)
            write_file(paths["crt_list"], crt_list, 0o600, 0o755)
            write_file(paths["cfg"], cfg, 0o644, 0o755)
            chk = env.run(["haproxy", "-c", "-f", paths["cfg"]], timeout=30)
            if not chk.ok:
                raise EdgeError("HAProxy rejected the installed configuration: %s" % ((chk.err or chk.out).strip().splitlines() or ["?"])[-1][:160])
            self._arm(seconds)
            if self.front_active():
                r = env.run(["systemctl", "reload", "haproxy"], timeout=30)
            else:
                r = env.run(["systemctl", "enable", "--now", "haproxy"], timeout=60)
                if r.ok:
                    r = env.run(["systemctl", "restart", "haproxy"], timeout=60)
            if not r.ok:
                raise EdgeError("HAProxy did not start or reload: %s" % (r.err or r.out).strip()[:160])
            deadline = env.now() + 8
            while True:
                missing = [p for p in (80, 443, TLS_TERM_PORT) if p not in env.listening_ports()]
                if not missing or env.now() > deadline:
                    break
                env.sleep(0.5)
            if missing:
                raise EdgeError("HAProxy is not listening on port %s." % ", ".join(str(p) for p in missing))
        except EdgeError as e:
            self._stop_timer()
            unlink(env.state_path("pending.json"))
            problems = self._restore()
            shutil.rmtree(self.snap_dir(), ignore_errors=True)
            raise EdgeError("%s The previous configuration was restored%s." % (str(e).rstrip("."), " (but: " + "; ".join(problems) + ")" if problems else ""))
        job.step("listener", "Moving the management plane behind the front" if kind == "enable" else "Saving the hostnames")
        cand["enabled"] = True
        if kind == "enable":
            self._set_listener(cand, True)
            env.defer(env.restart_mgmt, 3.0)
        self.save_state(cand)
        urls = self.urls(cand)
        job.step("confirm", "Waiting for you to confirm")
        job.result = {"kind": kind, "seconds": seconds, "urls": urls, "restart": kind == "enable"}
        job.line("Open %s and press Keep within %d seconds, or everything goes back." % (urls[0] if urls else "https://<this unit>/", seconds))

    def _disable_job(self, job: Job) -> None:
        env = self.env
        st = self.state()
        job.step("listener", "Restoring the direct web address")
        self._stop_timer()
        unlink(env.state_path("pending.json"))
        shutil.rmtree(self.snap_dir(), ignore_errors=True)
        self._set_listener(st, False)
        st["enabled"] = False
        self.save_state(st)
        job.step("stop", "Stopping HAProxy")

        def finish() -> None:
            # HAProxy is stopped by a transient unit: restarting the web service kills this process, so the stop must outlive it
            r = env.run(["systemd-run", "--collect", "--on-active=8s", "systemctl", "disable", "--now", "haproxy"], timeout=20)
            if not r.ok:
                env.run(["systemctl", "disable", "--now", "haproxy"], timeout=60)
            env.restart_mgmt()
        env.defer(finish, 3.0)
        job.result = {"kind": "disable", "restart": True, "urls": ["https://%s:%d/" % (ip if ":" not in ip else "[%s]" % ip, env.mgmt_port) for ip in env.local_ips()[:2]]}

    # -- certificate files changed (renewal): refresh the PEM and reload, without a revert timer ------------------------------------------
    def certs_changed(self, ids: list[str], job: Job | None = None) -> str | None:
        env = self.env
        st = self.state()
        used = {st["mgmt"]["cert"]} | {s["cert"] for s in st["sites"].values()}
        todo = [c for c in ids if c in used]
        if not st["enabled"] or not todo:
            return None
        paths = self.final_paths()
        old = {}
        for cid in todo:
            p = self.edge.certs.store.combined_path(cid)
            old[p] = read_text(p) if os.path.isfile(p) else None
            self.edge.certs.store.write_combined(cid)
        chk = env.run(["haproxy", "-c", "-f", paths["cfg"]], timeout=30)
        r = env.run(["systemctl", "reload", "haproxy"], timeout=30) if chk.ok else None
        if chk.ok and r is not None and r.ok:
            if job:
                job.line("HAProxy reloaded with the new certificate(s)")
            return None
        for p, text in old.items():
            if text is None:
                unlink(p)
            else:
                write_file(p, text, 0o600, 0o755)
        msg = "HAProxy kept the old certificate: %s" % ((chk.err or chk.out) if not chk.ok else (r.err or r.out if r else "")).strip()[:160]
        return msg



# ============================================================================
# 8. The facade, REST routes and the command line
# ============================================================================

class Edge:
    """Certificates and the HAProxy front, one object per management plane."""

    def __init__(self, env: EdgeEnv):
        self.env = env
        self.proxy = ProxyManager(env, self)
        self.certs = CertManager(env, self)


CERT_PATH = r"[a-z0-9_][a-z0-9._-]{0,252}"


def register_routes(host: Any, get_edge: Callable[[Any], Edge]) -> None:
    """Attach /api/certs and /api/proxy to the web module (``host`` provides Route, ROUTES, DYNAMIC_ROUTES, ApiError)."""
    ApiError, Route = host.ApiError, host.Route

    def guarded(fn: Callable[[Any, Edge], Any]) -> Callable[[Any], Any]:
        def inner(req: Any) -> Any:
            edge = get_edge(req.app)
            try:
                return fn(req, edge)
            except NotFound as e:
                raise ApiError(404, str(e))
            except Busy as e:
                raise ApiError(409, str(e))
            except EdgeError as e:
                raise ApiError(400, str(e))
        return inner

    def seg(req: Any, i: int) -> str:
        return req.path.split("/")[i]

    def job_reply(job: Job) -> tuple[int, dict]:
        return 202, {"ok": True, "job": job.view()}

    def certs_list(req: Any, edge: Edge) -> dict:
        d = edge.certs.list()
        d["ok"] = True
        d["proxy"] = {"enabled": edge.proxy.enabled(), "pending": edge.proxy.pending()}
        return d

    def cert_show(req: Any, edge: Edge) -> dict:
        return {"ok": True, "cert": edge.certs.view(seg(req, 3))}

    def job_show(req: Any, edge: Edge) -> dict:
        j = edge.certs.jobs.get(seg(req, 4))
        if not j:
            raise NotFound("no such job")
        return {"ok": True, "job": j.view()}

    def settings(req: Any, edge: Edge) -> dict:
        body = dict(req.body)
        req.audit_note = "settings"
        if body.get("secret"):
            req.body["secret"] = "***"
        d = update_settings(edge.env, body)
        d["ok"] = True
        edge.env.audit(req.user, req.ip, "certs.settings", {k: ("***" if "secret" in k and v else v) for k, v in body.items()}, "ok")
        return d

    def issue(req: Any, edge: Edge) -> Any:
        req.audit_note = "issue"
        return job_reply(edge.certs.issue(dict(req.body), req.user, req.ip))

    def precheck(req: Any, edge: Edge) -> dict:
        ch = req.body.get("challenge", "dns-01")
        if ch not in CHALLENGES:
            raise EdgeError("challenge must be one of %s" % ", ".join(CHALLENGES))
        d = edge.certs.precheck(valid_names(req.body.get("names"), ch), ch)
        d["ok_all"] = d.pop("ok")
        d["ok"] = True
        return d

    def dns(req: Any, edge: Edge) -> dict:
        d = check_dns(edge.env, req.body.get("name", ""))
        d["ok"] = True
        return d

    def secret_check(req: Any, edge: Edge) -> dict:
        d = check_secret(edge.env)
        d["reached"] = d["ok"]
        d["ok"] = True
        return d

    def cert_act(req: Any, edge: Edge) -> Any:
        cid, action = seg(req, 3), seg(req, 4)
        req.audit_note = "%s %s" % (action, cid)
        if action == "renew":
            return job_reply(edge.certs.renew(cid, req.user, req.ip))
        if action == "change":
            return job_reply(edge.certs.issue(dict(req.body), req.user, req.ip, change=cid))
        return edge.certs.remove(cid, req.body.get("confirm", ""), req.user, req.ip)

    def proxy_show(req: Any, edge: Edge) -> dict:
        d = edge.proxy.view()
        d["ok"] = True
        return d

    def proxy_hostnames(req: Any, edge: Edge) -> dict:
        st = edge.proxy.normalize(dict(req.body))
        if edge.proxy.enabled():
            raise EdgeError("The front is on: use Apply, which checks and publishes the change.")
        edge.proxy.save_state(st)
        return {"ok": True, "rows": edge.proxy.rows(st)}

    def proxy_test(req: Any, edge: Edge) -> dict:
        d = edge.proxy.test(dict(req.body) if req.body else None)
        d["ok_check"] = d["ok"]
        d["ok"] = True
        return d

    def proxy_act(req: Any, edge: Edge) -> Any:
        action = seg(req, 3)
        req.audit_note = action
        if action in ("enable", "apply", "disable"):
            return job_reply(edge.proxy.apply(action, dict(req.body), req.user, req.ip))
        if action == "confirm":
            return edge.proxy.confirm(req.user, req.ip)
        return edge.proxy.revert(False, req.user, req.ip)

    R = host.ROUTES
    R[("GET", "/api/certs")] = Route(guarded(certs_list), True, False)
    R[("POST", "/api/certs/settings")] = Route(guarded(settings), True, True)
    R[("POST", "/api/certs/issue")] = Route(guarded(issue), True, True)
    R[("POST", "/api/certs/precheck")] = Route(guarded(precheck), True, False)
    R[("POST", "/api/certs/check-dns")] = Route(guarded(dns), True, False)
    R[("POST", "/api/certs/check-secret")] = Route(guarded(secret_check), True, False)
    R[("GET", "/api/proxy")] = Route(guarded(proxy_show), True, False)
    R[("POST", "/api/proxy/hostnames")] = Route(guarded(proxy_hostnames), True, True)
    R[("POST", "/api/proxy/test")] = Route(guarded(proxy_test), True, False)
    dyn = host.DYNAMIC_ROUTES
    dyn.append(("GET", re.compile(r"/api/certs/job/e[0-9a-f]{12}"), Route(guarded(job_show), True, False)))
    dyn.append(("GET", re.compile(r"/api/certs/" + CERT_PATH), Route(guarded(cert_show), True, False)))
    dyn.append(("POST", re.compile(r"/api/certs/" + CERT_PATH + r"/(?:renew|change|remove)"), Route(guarded(cert_act), True, True)))
    dyn.append(("POST", re.compile(r"/api/proxy/(?:enable|apply|disable|confirm|revert)"), Route(guarded(proxy_act), True, True)))


def fmt_day(ts: float | None) -> str:
    return time.strftime("%Y-%m-%d", time.gmtime(ts)) if ts else "-"


def cli_main(argv: list[str], edge: Edge, which: str, out: Callable[[str], None] = print, err: Callable[[str], None] | None = None, stdin: Any = None) -> int:
    """``synofurb_mgmt.py certs <command>`` / ``proxy <command>``.  Exit codes: 0 ok, 1 the operation failed or another job is running, 2 refused or bad input."""
    import argparse
    err = err or (lambda m: print(m, file=sys.stderr))
    ap = argparse.ArgumentParser(prog="synofurb_mgmt.py " + which, add_help=True)
    sub = ap.add_subparsers(dest="cmd", required=True)

    def add(name: str, *flags: str) -> argparse.ArgumentParser:
        p = sub.add_parser(name)
        p.add_argument("--json", action="store_true")
        for f in flags:
            if f == "id":
                p.add_argument("id")
            elif f == "names":
                p.add_argument("names", nargs="+")
        return p
    if which == "certs":
        add("list")
        add("show", "id")
        p = add("issue", "names")
        p.add_argument("--challenge", default="dns-01", choices=CHALLENGES)
        p.add_argument("--environment", choices=ENVIRONMENTS)
        p.add_argument("--skip-dns-check", action="store_true")
        p = add("change", "id")
        p.add_argument("names", nargs="+")
        p.add_argument("--challenge", default="dns-01", choices=CHALLENGES)
        p.add_argument("--environment", choices=ENVIRONMENTS)
        p = add("renew")
        p.add_argument("--id")
        p.add_argument("--force", action="store_true")
        p = add("remove", "id")
        p.add_argument("--confirm", default="")
        p = add("check-dns")
        p.add_argument("names", nargs="+")
        p = add("settings")
        p.add_argument("--email")
        p.add_argument("--accept-tos", action="store_true", help="accept the Let's Encrypt subscriber agreement")
        p.add_argument("--environment", choices=ENVIRONMENTS)
        p.add_argument("--key-type", choices=KEY_TYPES)
        p.add_argument("--generate-secret", action="store_true", help="create a new random challenge secret (shown once)")
        p.add_argument("--secret-stdin", action="store_true", help="read the challenge secret from standard input")
        p.add_argument("--check-secret", action="store_true", help="ask the service which sub-domain it derives from the secret")
        p.add_argument("--dns-skip-check", choices=("on", "off"))
        p.add_argument("--dns-resolvers")
    else:
        add("status")
        p = add("enable")
        p.add_argument("--seconds", type=int, default=REVERT_SECONDS)
        p = add("apply")
        p.add_argument("--seconds", type=int, default=REVERT_SECONDS)
        add("disable")
        add("test")
        add("confirm")
        p = add("revert")
        p.add_argument("--auto", action="store_true")
        p = add("assign", "id")
        p.add_argument("hostname")
        p.add_argument("cert")
        add("unassign", "id")
    try:
        args = ap.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    cmd = args.cmd

    def jprint(obj: Any) -> None:
        out(json.dumps(obj, indent=1, sort_keys=True))

    def run_job(job: Job) -> int:
        if args.json:
            jprint(job.view())
        else:
            for w in job.warnings:
                err("warning: " + w)
            out("ok" if job.ok else "failed: " + job.error)
        return 0 if job.ok else 1
    sink = None if getattr(args, "json", False) else (lambda m: out(m))
    try:
        if which == "certs":
            c = edge.certs
            if cmd == "list":
                d = c.list()
                if args.json:
                    jprint(d)
                else:
                    out("%-34s %-12s %-10s %-11s %s" % ("ID", "CHALLENGE", "STATE", "EXPIRES", "NAMES"))
                    for v in d["certs"]:
                        out("%-34s %-12s %-10s %-11s %s" % (v["id"], v["challenge"], v["status"], fmt_day(v["expires"]), ", ".join(v["names"])))
                return 0
            if cmd == "show":
                v = c.view(args.id)
                if args.json:
                    jprint(v)
                else:
                    out("%s (%s, %s) %s, expires %s (%s days)" % (v["id"], v["challenge"], v["environment"], v["status"], fmt_day(v["expires"]), v["days_left"]))
                    out("names: " + ", ".join(v["names"]))
                    for cn in v["cnames"]:
                        out("CNAME %s -> %s" % (cn["record"], cn["target"] or "(set the secret first)"))
                    for u in v["used_by"]:
                        out("used by %s (%s)" % (u["hostname"], u["id"]))
                    if v["last_error"]:
                        out("last error: " + v["last_error"])
                return 0
            if cmd in ("issue", "change"):
                body = {"names": args.names, "challenge": args.challenge, "skip_dns_check": args.skip_dns_check if cmd == "issue" else False}
                if args.environment:
                    body["environment"] = args.environment
                job = c.issue(body, user="cli", ip="-", background=False, sink=sink, change=args.id if cmd == "change" else None)
                return run_job(job)
            if cmd == "renew":
                job = c.renew_all("cli", "-", force=args.force, only=args.id, background=False, sink=sink)
                return run_job(job)
            if cmd == "remove":
                d = c.remove(args.id, args.confirm, "cli", "-")
                out(json.dumps(d) if args.json else "removed " + args.id)
                return 0
            if cmd == "check-dns":
                bad = 0
                for n in args.names:
                    d = check_dns(edge.env, n)
                    bad += d["state"] != "ok"
                    if args.json:
                        jprint(d)
                    else:
                        out("%s: %s (%s)%s" % (d["record"], d["state"], d["message"], "" if d["state"] == "ok" else "\n  expected CNAME -> %s" % d["target"]))
                return 0 if not bad else 1
            if cmd == "settings":
                body: dict = {}
                if args.email is not None:
                    body["email"] = args.email
                if args.accept_tos:
                    body["accept_tos"] = True
                for k, v in (("environment", args.environment), ("key_type", args.key_type), ("dns_resolvers", args.dns_resolvers)):
                    if v is not None:
                        body[k] = v
                if args.dns_skip_check:
                    body["dns_skip_check"] = args.dns_skip_check == "on"
                if args.generate_secret:
                    body["generate_secret"] = True
                if args.secret_stdin:
                    body["secret"] = (stdin or sys.stdin).readline().strip()
                d = update_settings(edge.env, body) if body else settings_view(edge.env)
                if args.check_secret:
                    d["check"] = check_secret(edge.env)
                if args.json:
                    jprint(d)
                else:
                    out("email: %s" % (d["email"] or "(not set)"))
                    out("terms accepted: %s" % ("yes" if d["tos_accepted"] else "no"))
                    out("environment: %s, key type %s" % (d["environment"], d["key_type"]))
                    out("challenge secret: %s" % ("set" if d["secret"]["set"] else "not set"))
                    if d["secret"]["target"]:
                        out("CNAME target for every name: %s" % d["secret"]["target"])
                    if d.get("secret_once"):
                        out("NEW SECRET (shown only now, keep it): %s" % d["secret_once"])
                    if d.get("check"):
                        out("service check: %s" % d["check"]["message"])
                return 0
        else:
            p = edge.proxy
            if cmd == "status":
                d = p.view()
                if args.json:
                    jprint(d)
                else:
                    out("front: %s; haproxy %s (%s)" % ("ON" if d["enabled"] else "off", d["haproxy"]["version"] or "not installed", "running" if d["haproxy"]["active"] else "stopped"))
                    for r in d["rows"]:
                        out("%-12s %-32s %s" % (r["id"], r["hostname"] or "-", r["cert"] or "-"))
                    if d["pending"]:
                        out("PENDING: confirm within %d s or it reverts (proxy confirm)" % d["pending"]["seconds_left"])
                    out("recovery: " + d["recovery"])
                return 0
            if cmd in ("enable", "apply", "disable"):
                job = p.apply(cmd, {}, user="cli", ip="-", background=False, sink=sink, seconds=getattr(args, "seconds", REVERT_SECONDS))
                return run_job(job)
            if cmd == "test":
                d = p.test()
                if args.json:
                    jprint({"ok": d["ok"], "output": d["output"], "installed": d["installed"]})
                else:
                    out(d["config"])
                    out("haproxy -c: " + ("not installed" if d["ok"] is None else "ok" if d["ok"] else "REJECTED\n" + d["output"]))
                return 0 if d["ok"] in (True, None) else 1
            if cmd == "confirm":
                p.confirm("cli", "-")
                out("kept")
                return 0
            if cmd == "revert":
                d = p.revert(args.auto, "cli", "-")
                out("reverted" + ("" if d["ok"] else ": " + "; ".join(d["problems"])))
                return 0 if d["ok"] else 1
            if cmd in ("assign", "unassign"):
                body = {"mgmt": {"hostname": "", "cert": ""}} if args.id == "mgmt" and cmd == "unassign" else None
                if args.id == "mgmt":
                    body = {"mgmt": {"hostname": args.hostname if cmd == "assign" else "", "cert": args.cert if cmd == "assign" else ""}}
                else:
                    body = {"sites": {args.id: {"hostname": args.hostname if cmd == "assign" else "", "cert": args.cert if cmd == "assign" else ""}}}
                st = p.normalize(body)
                if p.enabled():
                    job = p.apply("apply", body, user="cli", ip="-", background=False, sink=sink)
                    return run_job(job)
                p.save_state(st)
                out("saved; enable or apply the front to publish it")
                return 0
    except Busy as e:
        err("busy: %s" % e)
        return 1
    except (EdgeError, ValueError) as e:
        err("error: %s" % e)
        return 2
    return 2


# ============================================================================
# 9. Web UI: copy, help topics and the two pages (synofurb_mgmt injects them into the single-page UI)
# ============================================================================
# Word budgets and style are the web design rules of the management plane: every visible word lives in COPY (label <= 4 words, button <= 3,
# tip <= 12, prose <= 12), help is behind info buttons, a control that is off gives its reason as visible text, destructive actions need the typed name,
# icons come from the Lucide sprite only.

def ui_copy(c: Callable[..., dict]) -> tuple[dict, dict, list, list]:
    """(COPY entries, COPY_SCREENS entries, extra COPY_SHELL keys, HELP topics).  ``c`` is the host's copy-entry constructor ``_c(short, kind, tip)``."""
    copy = {
        "nav.certs": c("Certificates"), "nav.proxy": c("Hostnames"),
        # certificates page
        "ce.issue": c("Issue certificate", "button"), "ce.settings": c("Settings", "button"), "ce.expires": c("Expires"), "ce.days": c("days"),
        "ce.empty": c("No certificates yet.", "prose"), "ce.setup": c("Set up Let's Encrypt first.", "safety"),
        "ce.auto": c("Auto-renew", "label", "Renewed twice a day when 30 days or fewer are left."), "ce.busy": c("A job is running", "title"), "ce.view": c("View", "button"),
        "ce.st.valid": c("Valid", "label", "The certificate is valid and renews by itself."), "ce.st.expiring": c("Expires soon", "label", "Renewal should have happened already; see the details."),
        "ce.st.expired": c("Expired", "label", "Browsers no longer trust this certificate."), "ce.st.failed": c("Failed", "label", "The last attempt failed; see the details."),
        "ce.st.working": c("Working", "label", "A job is issuing or renewing it."), "ce.st.none": c("Not issued", "label", "No certificate was obtained yet."),
        "ce.ch.dns-01": c("DNS", "label", "Proves the name with a DNS record; works behind NAT and allows wildcards."),
        "ce.ch.http-01": c("HTTP", "label", "Proves the name on port 80; the name must point at this unit."),
        "ce.ch.tls-alpn-01": c("TLS-ALPN", "label", "Proves the name on port 443; the name must point at this unit."),
        "ce.renew": c("Renew now", "button", "Ask Let's Encrypt for a new certificate now."), "ce.change": c("Change", "button"), "ce.remove": c("Remove", "button"),
        "ce.details": c("Details", "button"), "ce.help": c("Help", "button"), "ce.copy": c("Copy", "button"), "ce.copied": c("Copied.", "prose"),
        # issue wizard
        "ce.issue_t": c("Issue certificate", "title"), "ce.change_t": c("Change {name}", "title"), "ce.names": c("Names", "label", "The first name is the main one; *.name covers one level."),
        "ce.challenge": c("Challenge", "label", "How Let's Encrypt checks that the name is yours."), "ce.environment": c("Environment"),
        "ce.production": c("Production", "label", "Trusted by browsers; Let's Encrypt limits apply."), "ce.staging": c("Staging", "label", "For testing: browsers do not trust it, and limits are generous."),
        "ce.why_names": c("Enter a name first.", "safety"), "ce.why_secret": c("Set the secret first.", "safety"),
        "ce.cname": c("CNAME record"), "ce.checkdns": c("Check DNS", "button", "Look up the record from this unit."), "ce.precheck": c("Pre-check", "button", "Test names and ports from this unit."),
        "ce.reach": c("Reachability"), "ce.port.http-01": c("Port 80 must reach this unit from the Internet.", "prose"), "ce.port.tls-alpn-01": c("Port 443 must reach this unit from the Internet.", "prose"),
        "ce.noinet": c("The Internet side cannot be tested from here.", "prose"), "ce.nat": c("Private address", "label", "Forward the port on your router."),
        "ce.dns.ok": c("Correct", "label", "The record points at the challenge service."), "ce.dns.wrong": c("Wrong target", "label", "The record points somewhere else."),
        "ce.dns.missing": c("Missing", "label", "No CNAME record was found."), "ce.dns.error": c("No answer", "label", "The lookup failed; try again."),
        "ce.dns.nosecret": c("No secret", "label", "Set the secret first."),
        "ce.nosecret": c("No secret set yet.", "safety"), "ce.gensecret": c("Generate secret", "button"), "ce.secret_new": c("Keep this secret. It is shown once.", "safety"),
        # settings
        "ce.settings_t": c("Certificate settings", "title"), "ce.email": c("E-mail", "label", "Let's Encrypt writes here when a certificate cannot renew."),
        "ce.tos": c("I accept the Let's Encrypt terms", "safety"), "ce.terms": c("Terms", "button"), "ce.keytype": c("Key type"),
        "ce.secret": c("Challenge secret", "label", "A password for the challenge service; only its hash goes into DNS."), "ce.secret_set": c("Set"), "ce.secret_none": c("Not set"),
        "ce.secret_own": c("Own secret", "label", "At least 16 characters; 32 or more is better."), "ce.checksvc": c("Check service", "button", "Ask the service which name it derives."),
        "ce.regen_q": c("New secret?", "title"), "ce.regen_b": c("Every CNAME record must then point at the new target.", "prose"), "ce.saved": c("Saved.", "prose"),
        "ce.svc_ok": c("The service agrees.", "prose"), "ce.svc_bad": c("The service answered differently.", "prose"), "ce.target": c("Target"),
        # details, remove, jobs
        "ce.lasterr": c("Last error"), "ce.fp": c("Fingerprint"), "ce.usedby": c("Used by"), "ce.never": c("Nothing yet.", "prose"), "ce.issued": c("Issued"),
        "ce.why_type": c("Type the name first.", "safety"), "ce.remove_q": c("Remove {name}?", "title"), "ce.remove_b": c("The certificate and its key are deleted for good.", "prose"),
        "ce.progress": c("Progress"), "ce.hide": c("Hide", "button"), "ce.done": c("Done.", "prose"), "ce.job_t": c("{name}: {action}", "title"),
        "ce.act.issue": c("issuing"), "ce.act.change": c("changing"), "ce.act.renew": c("renewing"), "ce.act.renew-all": c("renewing"),
        # hostnames page
        "px.on": c("Front on", "label", "HAProxy serves ports 80 and 443 for this unit and its apps."), "px.off": c("Front off", "label", "The management plane answers on its own port."),
        "px.enable": c("Enable front", "button"), "px.apply": c("Apply", "button"), "px.disable": c("Disable front", "button"), "px.check": c("Check", "button", "Test the configuration with haproxy -c; nothing changes."),
        "px.hostname": c("Hostname"), "px.cert": c("Certificate"), "px.backend": c("Backend", "label", "How HAProxy talks to the app on this unit."),
        "px.mgmt": c("Management plane"), "px.none": c("None"), "px.urls": c("Addresses"),
        "px.why_cert": c("Choose a certificate.", "safety"), "px.why_cover": c("The certificate does not cover that name.", "safety"), "px.why_dup": c("Use each hostname once.", "safety"),
        "px.why_name": c("That is not a valid name.", "safety"), "px.nocerts": c("Issue a certificate first.", "safety"), "px.nosites": c("No app has a web page yet.", "prose"),
        "px.enable_q": c("Enable the front?", "title"), "px.apply_q": c("Apply the change?", "title"), "px.disable_q": c("Disable the front?", "title"),
        "px.e1": c("HAProxy takes over ports 80 and 443.", "prose"), "px.e2": c("The management plane then listens on this unit only.", "prose"),
        "px.e3": c("Open the new address and press Keep within two minutes.", "prose"), "px.e4": c("Otherwise everything goes back by itself.", "prose"),
        "px.e5": c("If you are locked out, run this on the console:", "prose"),
        "px.a1": c("The configuration is checked before it is used.", "prose"), "px.d1": c("The management plane listens on port 8443 again.", "prose"),
        "px.d2": c("HAProxy stops. Certificates are kept.", "prose"),
        "px.keep_q": c("Keep this setup?", "title"), "px.in": c("Reverts in"), "px.keep": c("Keep", "button"), "px.revert": c("Revert", "button"), "px.kept": c("Kept.", "prose"),
        "px.reverted": c("Reverted.", "prose"), "px.reopen": c("Open the new address and sign in again.", "prose"), "px.autorev": c("Reverted automatically.", "prose"),
        "px.test_t": c("Configuration check", "title"), "px.test_ok": c("HAProxy accepts the configuration.", "prose"), "px.test_bad": c("HAProxy rejected the configuration.", "prose"),
        "px.test_none": c("HAProxy is installed when you enable the front.", "prose"),
        "px.act.proxy-enable": c("enabling"), "px.act.proxy-apply": c("applying"), "px.act.proxy-disable": c("disabling"),
    }
    screens = {
        "certs": ["nav.certs", "ce.issue", "ce.settings", "col.name", "ce.expires", "col.state", "ce.empty", "ce.setup", "ce.busy", "ce.view"],
        "proxy": ["nav.proxy", "px.enable", "px.apply", "px.disable", "px.check", "col.name", "px.hostname", "px.cert", "px.backend", "px.nocerts", "px.nosites"],
    }
    help_topics = [
        {"id": "certificates", "title": "Certificates", "keywords": "certificate let's encrypt tls https renew staging production wildcard lego acme",
         "body": "Certificates come from Let's Encrypt, which checks that you control a name before it signs. Give an e-mail address and accept the terms once.\n\n"
                 "Issue a certificate for one or more names. The first name is the main one; *.name covers one level below it. Each certificate renews by itself twice a day when 30 days or fewer are left; "
                 "a failure shows up here and on the dashboard.\n\nStaging gives test certificates that browsers do not trust. Use it first: production allows only five identical certificates a week."},
        {"id": "challenges", "title": "Challenges", "keywords": "dns http tls-alpn challenge cname addr.tools secret port 80 443 wildcard nat",
         "body": "DNS: you add one CNAME record per name, _acme-challenge.NAME, pointing at a target made from your secret on challenges.addr.tools. It works behind NAT, needs no open port and allows wildcards. "
                 "The secret is a password: only its hash is public, so keep it long.\n\nHTTP: Let's Encrypt connects to port 80 of the name. TLS-ALPN: it connects to port 443. Both need the name to point at this unit and the port to be "
                 "reachable from the Internet; the HAProxy front answers them. Neither allows wildcards."},
        {"id": "hostnames", "title": "Hostnames", "keywords": "haproxy front hostname proxy sni certificate locked out recovery disable keep revert 443",
         "body": "The front is one HAProxy in front of the management plane and the apps. It serves ports 80 and 443, picks the certificate by name and sends each hostname to its app. "
                 "Any other name or an IP address reaches the management plane, so this unit is always reachable.\n\nEnabling it moves the management plane to this unit only; open the new address and press Keep within two minutes, "
                 "or everything goes back by itself. Apps keep their own ports.\n\nLocked out? On the console run: python3 /opt/synofurb-mgmt/synofurb_mgmt.py proxy disable"},
    ]
    return copy, screens, ["nav.certs", "nav.proxy"], help_topics


UI_NAV = "['certs', 'shield-check'], ['proxy', 'globe'], "

UI_CSS = r"""
.cn{display:flex;flex-direction:column;gap:6px;padding:10px 0;border-top:1px solid var(--border)}.cn:first-of-type{border-top:0}
.cn code{overflow-wrap:anywhere;word-break:break-all;font:400 12px/18px var(--font-mono)}
.cn .row{align-items:flex-start;flex-wrap:nowrap}.cn .row code{flex:1 1 0;min-width:0}.cn .row .btn{flex:none}
.pxrow input.mono{min-width:210px;width:100%}.pxrow select{max-width:100%}
.pre-r{display:flex;flex-direction:column;gap:6px;margin-top:8px}.pre-r .row{align-items:center}
#pend-edge .banner{margin-bottom:0;border-radius:0}
.nw{white-space:nowrap}.row.nw{flex-wrap:nowrap}
"""

UI_JS = r"""
/*PAGE:certs*/
/*copy: ce.st.valid ce.st.expiring ce.st.expired ce.st.failed ce.st.working ce.st.none ce.ch.dns-01 ce.ch.http-01 ce.ch.tls-alpn-01 ce.dns.ok ce.dns.wrong ce.dns.missing ce.dns.error ce.dns.nosecret*/
const CE_KIND = {valid: 'ok', expiring: 'warn', expired: 'err', failed: 'err', working: 'info', none: 'neutral'};
const CE_DNS = {ok: 'ok', wrong: 'err', missing: 'warn', error: 'err', nosecret: 'warn'};
const ceTag = st => tip(badge(CE_KIND[st] || 'neutral', T('ce.st.' + st)), TIP('ce.st.' + st));
const ceChallenge = c => tip(h('span', {class: 'chip'}, T('ce.ch.' + c)), TIP('ce.ch.' + c));
const ceDay = t => (t ? new Date(t * 1000).toISOString().slice(0, 10) : '-');
const ceHelp = (text, label) => popover(T('ce.help'), h('p', {}, text), 'info', label);
const ceCovers = (names, host) => names.some(n => (n.startsWith('*.') ? host.endsWith(n.slice(1)) && host.length > n.length - 1 && !host.slice(0, host.length - n.length + 1).includes('.') : n === host));
function ceCopy(text, label) {
  const b = btn(T('ce.copy'), {size: 'sm', icon: 'copy', onclick: async () => { try { await navigator.clipboard.writeText(text); toast(T('ce.copied')); } catch (e) { toast(text); } }});
  b.setAttribute('aria-label', T('ce.copy') + ': ' + label);
  return b;
}
const dnsTag = r => h('span', {class: 'row'}, tip(badge(CE_DNS[r.state] || 'neutral', T('ce.dns.' + r.state)), TIP('ce.dns.' + r.state)), h('small', {class: 'help', 'data-d': ''}, r.found && r.state === 'wrong' ? r.found : r.message));
/* a job in a dialog: progress, current step, log; polls until it ends (certificates and the front share one job list) */
function ceJob(first, title, done) {
  const bar = h('progress', {max: 100, value: first.progress || 0, 'aria-label': T('ce.progress')}), msg = h('p', {'data-d': '', role: 'status'}, first.message || ''), res = h('div', {}), log = h('pre', {class: 'code log2', 'aria-live': 'off'}, '');
  let timer = null, hideB = null, fails = 0, ended = false;
  const finish = j => { if (!ended) { ended = true; if (done) done(j); } };
  const dlg = dialog(title, {icon: 'refresh-cw', wide: true}, () => h('div', {}, bar, msg, res, log), close => [hideB = h('button', {type: 'button', class: 'btn primary', onclick: close}, T('ce.hide'))]);
  const poll = async () => {
    try {
      const j = (await api('GET', '/api/certs/job/' + first.id)).job; fails = 0;
      bar.value = j.progress; msg.textContent = j.state === 'running' ? j.message : (j.ok ? T('ce.done') : j.error);
      log.textContent = j.log.join('\n'); log.scrollTop = log.scrollHeight;
      if (j.state !== 'running') {
        clearInterval(timer); bar.className = j.ok ? 'ok' : 'err'; hideB.replaceChildren(T('btn.close'));
        if (j.warnings.length) log.textContent += '\n' + j.warnings.join('\n');
        if (j.ok && j.result && j.result.urls && j.result.urls.length) res.replaceChildren(h('p', {role: 'status'}, T('px.reopen')), h('div', {class: 'chips'}, j.result.urls.map(u => h('span', {class: 'chip mono'}, u))));
        toast(j.ok ? T('ce.done') : j.error, !j.ok); finish(j);
      }
    } catch (e) { if (++fails > 2) { clearInterval(timer); msg.textContent = T('px.reopen'); } }
  };
  dlg.addEventListener('close', () => { clearInterval(timer); finish(null); });
  timer = setInterval(poll, 1200); poll();
}
function ceSettings(L, reload) {
  const s = L.settings;
  const email = h('input', {type: 'email', class: 'mono', value: s.email, autocomplete: 'off', spellcheck: 'false', id: 'ce-email'});
  const tos = h('input', {type: 'checkbox', checked: !!s.tos_accepted});
  const env = h('select', {id: 'ce-env'}, ['production', 'staging'].map(x => h('option', {value: x, selected: x === s.environment}, T('ce.' + x))));
  const key = h('select', {id: 'ce-key'}, ['ec256', 'ec384', 'rsa2048', 'rsa4096'].map(x => h('option', {value: x, selected: x === s.key_type}, x)));
  const own = h('input', {type: 'password', class: 'mono', autocomplete: 'new-password', spellcheck: 'false', id: 'ce-own'});
  const state = h('div', {class: 'row'}), shown = h('div', {});
  const drawSecret = () => state.replaceChildren(badge(s.secret.set ? 'ok' : 'warn', s.secret.set ? T('ce.secret_set') : T('ce.secret_none')), s.secret.target ? h('code', {class: 'mono', 'data-d': ''}, s.secret.target) : null);
  drawSecret();
  const gen = btn(T('ce.gensecret'), {icon: 'key-round', size: 'sm'}), chk = btn(T('ce.checksvc'), {icon: 'search', size: 'sm'});
  const doGen = () => act(gen, async () => {
    const d = await api('POST', '/api/certs/settings', {generate_secret: true});
    s.secret = d.secret; drawSecret();
    shown.replaceChildren(banner('ok', T('ce.secret_new'), null), h('div', {class: 'cn'}, h('div', {class: 'row'}, h('code', {class: 'mono'}, d.secret_once), ceCopy(d.secret_once, T('ce.secret')))));
  });
  gen.addEventListener('click', () => { if (s.secret.set) confirmAction({title: T('ce.regen_q'), danger: false, icon: 'key-round', label: T('ce.gensecret'), body: T('ce.regen_b'), onConfirm: async () => { await doGen(); }}); else doGen(); });
  chk.addEventListener('click', () => act(chk, async () => { const r = await api('POST', '/api/certs/check-secret', {}); toast(r.reached ? T('ce.svc_ok') : r.state === 'mismatch' ? T('ce.svc_bad') : r.message, !r.reached); }));
  return dialog(T('ce.settings_t'), {icon: 'settings', wide: true}, () => h('div', {},
    h('div', {class: 'f'}, h('div', {class: 'row'}, h('label', {class: 'lab', for: 'ce-email'}, T('ce.email')), ceHelp(TIP('ce.email'), T('ce.email'))), email),
    h('div', {class: 'row'}, h('label', {class: 'chk'}, tos, h('span', {}, T('ce.tos'))), h('a', {class: 'btn sm', href: s.terms_url, target: '_blank', rel: 'noopener noreferrer'}, ic('external-link'), T('ce.terms'))),
    h('div', {class: 'f'}, h('label', {class: 'lab', for: 'ce-env'}, T('ce.environment')), env),
    h('div', {class: 'f'}, h('label', {class: 'lab', for: 'ce-key'}, T('ce.keytype')), key),
    h('div', {class: 'dsec'}, h('div', {class: 'row'}, h('h4', {}, T('ce.secret')), ceHelp(TIP('ce.secret'), T('ce.secret'))), state, h('div', {class: 'row'}, gen, chk), shown,
      h('div', {class: 'f'}, h('label', {class: 'lab', for: 'ce-own'}, T('ce.secret_own')), own))),
  (close, setErr) => {
    const save = h('button', {type: 'button', class: 'btn primary'}, ic('check'), T('btn.save'));
    save.addEventListener('click', async () => {
      if (save.classList.contains('is-loading')) return;
      busy(save, true); setErr('');
      try {
        const body = {email: email.value.trim(), accept_tos: tos.checked, environment: env.value, key_type: key.value};
        if (own.value) body.secret = own.value;
        await api('POST', '/api/certs/settings', body); toast(T('ce.saved')); close(); reload();
      } catch (e) { setErr(e.message); busy(save, false); }
    });
    return [h('button', {type: 'button', class: 'btn', onclick: close}, T('btn.cancel')), save];
  });
}
function ceIssue(L, cert, reload) {
  const set = L.settings, ready = !!(set.email && set.tos_accepted), edit = !!cert;
  const names = h('input', {class: 'mono', id: 'ce-names', value: cert ? cert.names.join(' ') : '', autocomplete: 'off', spellcheck: 'false'});
  let chal = cert ? cert.challenge : 'dns-01', envn = cert ? cert.environment : set.environment;
  const panel = h('div', {class: 'dsec'}), why = whyLine(), shown = h('div', {});
  let ok = null;
  const parse = () => [...new Set(names.value.split(/[\s,]+/).map(x => x.trim().toLowerCase()).filter(Boolean))];
  const bases = () => [...new Set(parse().map(n => n.replace(/^\*\./, '')))];
  const sync = () => { if (ok) setOff(ok, why, !ready ? T('ce.setup') : !parse().length ? T('ce.why_names') : chal === 'dns-01' && !set.secret.set ? T('ce.why_secret') : null); };
  const genSecret = async () => {
    const d = await api('POST', '/api/certs/settings', {generate_secret: true});
    set.secret = d.secret; draw();
    shown.replaceChildren(banner('ok', T('ce.secret_new'), null), h('div', {class: 'cn'}, h('div', {class: 'row'}, h('code', {class: 'mono'}, d.secret_once), ceCopy(d.secret_once, T('ce.secret')))));
  };
  function draw() {
    panel.replaceChildren();
    if (chal === 'dns-01') {
      panel.append(h('div', {class: 'row'}, h('h4', {}, T('ce.cname')), ceHelp(TIP('ce.ch.dns-01'), T('ce.ch.dns-01'))));
      if (!set.secret.set) {
        const g = btn(T('ce.gensecret'), {size: 'sm', icon: 'key-round'}); g.addEventListener('click', () => act(g, genSecret));
        const own = h('input', {type: 'password', class: 'mono', id: 'ce-own2', autocomplete: 'new-password', spellcheck: 'false', 'aria-label': T('ce.secret_own')});
        const sv = btn(T('btn.save'), {size: 'sm', icon: 'check'});
        sv.addEventListener('click', () => act(sv, async () => { const d = await api('POST', '/api/certs/settings', {secret: own.value}); set.secret = d.secret; draw(); }));
        panel.append(banner('warn', T('ce.nosecret'), null, g), h('div', {class: 'f'}, h('label', {class: 'lab', for: 'ce-own2'}, T('ce.secret_own')), h('div', {class: 'row'}, own, sv)));
      } else {
        const tags = {}, bs = bases();
        for (const b of bs) {
          tags[b] = h('div', {});
          panel.append(h('div', {class: 'cn'}, h('code', {class: 'mono'}, '_acme-challenge.' + b), h('div', {class: 'row'}, ic('arrow-right', 'sm'), h('code', {class: 'mono'}, set.secret.target), ceCopy(set.secret.target, b)), tags[b]));
        }
        if (bs.length) {
          const cb = btn(T('ce.checkdns'), {icon: 'search', size: 'sm'});
          cb.addEventListener('click', () => act(cb, async () => { for (const b of bs) tags[b].replaceChildren(dnsTag(await api('POST', '/api/certs/check-dns', {name: b}))); }));
          panel.append(cb);
        }
      }
    } else {
      const out = h('div', {class: 'pre-r'});
      const pb = btn(T('ce.precheck'), {icon: 'search', size: 'sm'});
      pb.addEventListener('click', () => act(pb, async () => {
        const r = await api('POST', '/api/certs/precheck', {names: parse(), challenge: chal});
        out.replaceChildren(...r.names.map(x => h('div', {class: 'row'}, badge(x.state === 'ok' ? 'ok' : 'warn', x.name), h('small', {class: 'help', 'data-d': ''}, x.message), x.private ? tip(badge('warn', T('ce.nat')), TIP('ce.nat')) : null)),
          ...r.ports.map(x => h('div', {class: 'row'}, badge(x.state === 'ok' ? 'ok' : 'warn', String(x.port)), h('small', {class: 'help', 'data-d': ''}, x.message))), h('small', {class: 'help'}, T('ce.noinet')));
      }));
      panel.append(h('div', {class: 'row'}, h('h4', {}, T('ce.reach')), ceHelp(TIP('ce.ch.' + chal), T('ce.ch.' + chal))), h('p', {'data-d': 'prose'}, T('ce.port.' + chal)), pb, out);
    }
    sync();
  }
  const chalRow = c => { const r = h('input', {type: 'radio', name: 'ce-ch', value: c, checked: c === chal}); r.addEventListener('change', () => { chal = c; draw(); });
    return h('div', {class: 'row'}, h('label', {class: 'chk'}, r, h('span', {}, T('ce.ch.' + c))), ceHelp(TIP('ce.ch.' + c), T('ce.ch.' + c))); };
  const envRow = e => { const r = h('input', {type: 'radio', name: 'ce-env', value: e, checked: e === envn}); r.addEventListener('change', () => { envn = e; });
    return h('div', {class: 'row'}, h('label', {class: 'chk'}, r, h('span', {}, T('ce.' + e))), ceHelp(TIP('ce.' + e), T('ce.' + e))); };
  names.addEventListener('input', () => { if (chal === 'dns-01') draw(); else sync(); });
  return dialog(edit ? T('ce.change_t', {name: cert.id}) : T('ce.issue_t'), {icon: 'shield-check', wide: true}, () => h('div', {},
    h('div', {class: 'f'}, h('div', {class: 'row'}, h('label', {class: 'lab', for: 'ce-names'}, T('ce.names')), ceHelp(TIP('ce.names'), T('ce.names'))), names),
    h('div', {class: 'dsec'}, h('h4', {}, T('ce.challenge')), ['dns-01', 'http-01', 'tls-alpn-01'].map(chalRow)),
    h('div', {class: 'dsec'}, h('h4', {}, T('ce.environment')), ['production', 'staging'].map(envRow)), panel, shown),
  (close, setErr) => {
    ok = h('button', {type: 'button', class: 'btn primary'}, ic('shield-check'), edit ? T('ce.change') : T('ce.issue'));
    ok.addEventListener('click', async () => {
      if (ok.classList.contains('is-loading')) return;
      busy(ok, true); setErr('');
      try {
        const r = await api('POST', edit ? '/api/certs/' + cert.id + '/change' : '/api/certs/issue', {names: parse(), challenge: chal, environment: envn});
        close(); ceJob(r.job, T('ce.job_t', {name: parse()[0], action: T('ce.act.' + (edit ? 'change' : 'issue'))}), reload);
      } catch (e) { setErr(e.message); busy(ok, false); }
    });
    draw();
    return [why, h('button', {type: 'button', class: 'btn', onclick: close}, T('btn.cancel')), ok];
  });
}
/* typed-name confirmation; the button is aria-disabled with its reason beside it (a natively disabled button cannot be focused and says nothing) */
function ceRemove(c, reload) {
  const typed = h('input', {class: 'mono', autocomplete: 'off', spellcheck: 'false', 'aria-label': T('raid.type', {name: c.id})});
  const why = whyLine();
  let ok = null;
  const sync = () => { if (ok) setOff(ok, why, typed.value !== c.id ? T('ce.why_type') : null); };
  typed.addEventListener('input', sync);
  dialog(T('ce.remove_q', {name: c.id}), {danger: true, sub: T('ce.remove_b')}, () => field(T('raid.type', {name: c.id}), typed), (close, setErr) => {
    ok = h('button', {type: 'button', class: 'btn danger'}, ic('trash'), T('ce.remove'));
    ok.addEventListener('click', async () => {
      if (ok.classList.contains('is-loading')) return;
      busy(ok, true); setErr('');
      try { await api('POST', '/api/certs/' + c.id + '/remove', {confirm: typed.value}); close(); reload(); } catch (e) { setErr(e.message); busy(ok, false); }
    });
    sync();
    return [why, h('button', {type: 'button', class: 'btn', onclick: close}, T('btn.cancel')), ok];
  });
}
function ceDetails(c) {
  const kv = (k, v, mono) => h('div', {class: 'row'}, h('b', {}, k), h('span', {class: mono ? 'mono' : '', 'data-d': ''}, v));
  dialog(c.id, {icon: 'shield-check', wide: true}, () => h('div', {},
    h('div', {class: 'row dsec'}, ceTag(c.status), ceChallenge(c.challenge), badge(c.environment === 'staging' ? 'warn' : 'neutral', T('ce.' + c.environment))),
    c.last_error ? banner('err', T('ce.lasterr'), c.last_error) : null,
    h('div', {class: 'dsec'}, kv(T('ce.names'), c.names.join(', '), true), kv(T('ce.issued'), ceDay(c.not_before)), kv(T('ce.expires'), ceDay(c.expires)), c.fingerprint ? kv(T('ce.fp'), c.fingerprint, true) : null),
    c.used_by.length ? h('div', {class: 'dsec'}, h('h4', {}, T('ce.usedby')), h('div', {class: 'chips'}, c.used_by.map(u => h('span', {class: 'chip mono'}, u.hostname)))) : null,
    c.cnames.length && c.cnames[0].target ? h('div', {class: 'dsec'}, h('h4', {}, T('ce.cname')), c.cnames.map(x => h('div', {class: 'cn'}, h('code', {class: 'mono'}, x.record), h('div', {class: 'row'}, ic('arrow-right', 'sm'), h('code', {class: 'mono'}, x.target), ceCopy(x.target, x.record))))) : null),
  close => [h('button', {type: 'button', class: 'btn primary', onclick: close}, T('btn.close'))]);
}
PAGES.certs = async () => {
  const L = await api('GET', '/api/certs'), reload = () => show('certs');
  const ready = !!(L.settings.email && L.settings.tos_accepted);
  const parts = [h('div', {class: 'ph'}, h('div', {class: 'row'}, h('h1', {}, T('nav.certs')), infoBtn('certificates', T('nav.certs')),
    tip(chip(T('ce.auto'), L.renew ? ceDay(L.renew.ts) : '-'), TIP('ce.auto'))),
    h('div', {class: 'row'}, btn(T('ce.settings'), {icon: 'settings', onclick: () => ceSettings(L, reload)}), btn(T('ce.issue'), {kind: 'primary', icon: 'plus', onclick: () => ceIssue(L, null, reload)})))];
  if (L.job) parts.push(banner('neutral', T('ce.busy'), L.job.message, btn(T('ce.view'), {size: 'sm', onclick: () => ceJob(L.job, T('ce.job_t', {name: L.job.subject === '*' ? T('nav.certs') : L.job.subject, action: T((L.job.kind.startsWith('proxy') ? 'px.act.' : 'ce.act.') + L.job.kind)}), reload)})));
  if (!ready) parts.push(banner('warn', T('ce.setup'), null, btn(T('ce.settings'), {size: 'sm', onclick: () => ceSettings(L, reload)})));
  if (!L.certs.length) { parts.push(h('div', {class: 'empty'}, ic('info'), T('ce.empty'))); return h('div', {}, parts); }
  const rows = L.certs.map(c => {
    const act2 = c.status === 'working';
    const nm = h('span', {class: 'anm'}, h('b', {class: 'mono', 'data-d': ''}, c.primary),
      c.names.length > 1 ? h('small', {class: 'adet', 'data-d': ''}, c.names.slice(1).join(', ')) : null, c.last_error ? h('small', {class: 'adet', 'data-d': 'prose'}, c.last_error) : null,
      h('span', {class: 'row'}, ceChallenge(c.challenge), c.environment === 'staging' ? badge('warn', T('ce.staging')) : null));
    return {cls: c.status === 'failed' || c.status === 'expired' ? 'bad' : c.status === 'expiring' ? 'attn' : '', cells: [nm,
      h('span', {class: 'nw'}, ceDay(c.expires), c.days_left != null ? h('small', {class: 'help'}, c.days_left + ' ' + T('ce.days')) : null), ceTag(c.status),
      h('div', {class: 'row nw'}, btnIcon('refresh-cw', T('ce.renew'), () => { if (!act2) act(null, async () => { const r = await api('POST', '/api/certs/' + c.id + '/renew', {}); ceJob(r.job, T('ce.job_t', {name: c.id, action: T('ce.act.renew')}), reload); }); }),
        btnIcon('info', T('ce.details'), () => ceDetails(c)), btnIcon('wrench', T('ce.change'), () => ceIssue(L, c, reload)),
        btnIcon('trash', T('ce.remove'), () => ceRemove(c, reload)))]};
  });
  parts.push(h('section', {class: 'card'}, h('div', {class: 'cb flush'}, table([T('col.name'), T('ce.expires'), T('col.state'), ''], rows))));
  return h('div', {}, parts);
};
/*ENDPAGE*/
/*PAGE:proxy*/
let edgeTick = null;
/* the unconfirmed front change: shown on every page, because the person only reaches it after signing in at the new address */
function edgePending(p) {
  clearInterval(edgeTick); edgeTick = null;
  let el = $('#pend-edge');
  if (!el) { el = h('div', {id: 'pend-edge'}); const a = $('#pend'); if (!a) return; a.after(el); }
  el.replaceChildren();
  if (!p) return;
  const end = Date.now() + p.seconds_left * 1000, left = h('b', {role: 'timer'});
  const keep = btn(T('px.keep'), {kind: 'primary', icon: 'check', onclick: () => act(keep, async () => { await api('POST', '/api/proxy/confirm', {}); toast(T('px.kept')); edgePending(null); if (S.page === 'proxy') show('proxy'); })});
  const rv = btn(T('px.revert'), {icon: 'undo-2', onclick: () => act(rv, async () => { await api('POST', '/api/proxy/revert', {}); toast(T('px.reverted')); edgePending(null); })});
  el.append(banner('warn', T('px.keep_q'), h('span', {}, T('px.in') + ' ', left), rv, keep));
  const upd = () => { const s = Math.max(0, Math.round((end - Date.now()) / 1000)); left.textContent = Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0'); if (!s) { clearInterval(edgeTick); } };
  upd(); edgeTick = setInterval(upd, 1000);
}
function pxConfirm(o) {
  return confirmAction({title: o.title, danger: o.danger, icon: o.icon, label: o.label, body: h('div', {}, o.lines.map(x => h('p', {'data-d': 'prose'}, T(x))), o.cmd ? h('pre', {class: 'code'}, o.cmd) : null), onConfirm: o.onConfirm});
}
PAGES.proxy = async () => {
  const P = await api('GET', '/api/proxy'), reload = () => show('proxy');
  const why = whyLine(), ent = {};
  const parts = [h('div', {class: 'ph'}, h('div', {class: 'row'}, h('h1', {}, T('nav.proxy')), infoBtn('hostnames', T('nav.proxy')),
    tip(badge(P.enabled ? 'ok' : 'neutral', T(P.enabled ? 'px.on' : 'px.off'), P.enabled ? 'shield-check' : 'circle-dashed'), TIP(P.enabled ? 'px.on' : 'px.off'))))];
  if (P.reverted && P.reverted.auto && P.reverted.ts > Date.now() / 1000 - 86400) parts.push(banner('warn', T('px.autorev'), null));
  if (!P.rows.length) { parts.push(h('div', {class: 'empty'}, ic('info'), T('px.nosites'))); }
  const body = () => {
    const out = {mgmt: null, sites: {}};
    for (const r of P.rows) { const e = ent[r.kind + r.id], v = {hostname: e.host.value.trim().toLowerCase(), cert: e.cert.value}; if (r.kind === 'mgmt') out.mgmt = v; else { v.scheme = e.scheme.value; out.sites[r.id] = v; } }
    return out;
  };
  let primary = null;
  const sync = () => {
    let reason = null; const seen = new Set();
    for (const r of P.rows) {
      const e = ent[r.kind + r.id], host = e.host.value.trim().toLowerCase(), cert = P.certs.find(c => c.id === e.cert.value);
      if (!host) continue;
      if (!/^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)+$/.test(host)) reason = reason || T('px.why_name');
      else if (!cert) reason = reason || (P.certs.length ? T('px.why_cert') : T('px.nocerts'));
      else if (!ceCovers(cert.names, host)) reason = reason || T('px.why_cover');
      if (seen.has(host)) reason = reason || T('px.why_dup');
      seen.add(host);
    }
    if (primary) setOff(primary, why, reason);
  };
  const rows = P.rows.map(r => {
    const host = h('input', {class: 'mono', value: r.hostname, placeholder: r.hint ? r.hint + '.example.org' : '', autocomplete: 'off', spellcheck: 'false',
      'aria-label': T('px.hostname') + ': ' + (r.kind === 'mgmt' ? T('px.mgmt') : r.name), oninput: sync});
    const cert = h('select', {'aria-label': T('px.cert') + ': ' + (r.kind === 'mgmt' ? T('px.mgmt') : r.name), onchange: sync}, [h('option', {value: ''}, T('px.none'))].concat(P.certs.map(c => h('option', {value: c.id, selected: c.id === r.cert}, c.id))));
    const scheme = r.kind === 'mgmt' ? h('span', {class: 'mono'}, 'https') : h('select', {'aria-label': T('px.backend') + ': ' + r.name}, ['http', 'https'].map(x => h('option', {value: x, selected: x === r.scheme}, x)));
    ent[r.kind + r.id] = {host, cert, scheme};
    return {cls: 'pxrow', cells: [r.kind === 'mgmt' ? h('b', {}, T('px.mgmt')) : h('span', {class: 'aname'}, h('span', {class: 'aico'}, ic($('#i-' + r.icon) ? r.icon : 'package', 'lg')), h('b', {'data-d': ''}, r.name)), host, cert, scheme]};
  });
  parts.push(h('section', {class: 'card'}, h('div', {class: 'cb flush'}, table([T('col.name'), T('px.hostname'), T('px.cert'), T('px.backend')], rows))));
  if (P.urls.length) parts.push(h('div', {class: 'chips'}, P.urls.map(u => h('span', {class: 'chip mono'}, u))));
  const doJob = path => async () => { const r = await api('POST', path, body()); ceJob(r.job, T('ce.job_t', {name: T('nav.proxy'), action: T('px.act.' + r.job.kind)}), async () => { try { edgePending((await api('GET', '/api/proxy')).pending); } catch (e) { /* the address changed */ } reload(); }); };
  const check = btn(T('px.check'), {icon: 'search', onclick: () => act(check, async () => {
    const r = await api('POST', '/api/proxy/test', body());
    dialog(T('px.test_t'), {icon: r.ok_check === false ? 'octagon-x' : 'check', wide: true}, () => h('div', {}, r.ok_check === null ? h('p', {}, T('px.test_none')) : banner(r.ok_check ? 'ok' : 'err', r.ok_check ? T('px.test_ok') : T('px.test_bad'), null),
      r.output ? h('pre', {class: 'code'}, r.output) : null, h('pre', {class: 'code log2'}, r.config)), close => [h('button', {type: 'button', class: 'btn primary', onclick: close}, T('btn.close'))]);
  })});
  const acts = [check];
  if (!P.enabled) {
    primary = btn(T('px.enable'), {kind: 'primary', icon: 'play', onclick: () => pxConfirm({title: T('px.enable_q'), danger: false, icon: 'play', label: T('px.enable'), lines: ['px.e1', 'px.e2', 'px.e3', 'px.e4', 'px.e5'], cmd: P.recovery, onConfirm: doJob('/api/proxy/enable')})});
  } else {
    primary = btn(T('px.apply'), {kind: 'primary', icon: 'check', onclick: () => pxConfirm({title: T('px.apply_q'), danger: false, icon: 'check', label: T('px.apply'), lines: ['px.a1', 'px.e3', 'px.e4'], onConfirm: doJob('/api/proxy/apply')})});
    acts.push(btn(T('px.disable'), {icon: 'power', onclick: () => pxConfirm({title: T('px.disable_q'), danger: true, icon: 'power', label: T('px.disable'), lines: ['px.d1', 'px.d2'], onConfirm: async () => { const r = await api('POST', '/api/proxy/disable', {}); ceJob(r.job, T('ce.job_t', {name: T('nav.proxy'), action: T('px.act.proxy-disable')}), reload); }})}));
  }
  acts.push(primary);
  parts.splice(1, 0, h('div', {class: 'row'}, acts, why));
  sync();
  return h('div', {}, parts);
};
/*ENDPAGE*/
"""
