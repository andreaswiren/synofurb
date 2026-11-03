"""Test doubles for synofurb_edge: fake lego / haproxy / systemctl / systemd-run / apt-get / ss / dig executables (real scripts, run through the
module's real ``run_command``, so the exact argv and environment are recorded), a loopback HTTPS server that plays https://challenges.addr.tools,
a loopback DNS server and a fake clock.  Nothing here talks to the Internet, Let's Encrypt, a real HAProxy or a real unit.
"""
from __future__ import annotations

import hashlib
import http.server
import json
import os
import shutil
import socket
import ssl
import struct
import subprocess
import tempfile
import threading
import time
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
US = "\x1f"

# every fake records "name<US>arg<US>arg..." to $FAKE_LOG; behaviour comes from files in $FAKE_DIR (the tests create them)
RECORD = r'''
rec() { printf '%s' "$1" >> "$FAKE_LOG"; shift; for a in "$@"; do printf '\037%s' "$a" >> "$FAKE_LOG"; done; printf '\n' >> "$FAKE_LOG"; }
'''

FAKE_LEGO = "#!/bin/sh\n" + RECORD + r'''
rec lego "$@"
env | grep '^EXEC_' | sort > "$FAKE_DIR/lego.env"
[ -f "$FAKE_DIR/leak.txt" ] && cat "$FAKE_DIR/leak.txt"
if [ -f "$FAKE_DIR/lego.fail" ]; then cat "$FAKE_DIR/lego.fail" >&2; exit 1; fi
P=""; D=""; ALL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --path) P="$2"; shift ;;
    --domains) [ -z "$D" ] && D="$2"; ALL="$ALL,DNS:$2"; shift ;;
  esac
  shift
done
stem=$(printf '%s' "$D" | tr '*' '_')
mkdir -p "$P/certificates"
days=90; [ -f "$FAKE_DIR/days" ] && days=$(cat "$FAKE_DIR/days")
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days "$days" -subj "/CN=$D" -addext "subjectAltName=${ALL#,}" \
  -keyout "$P/certificates/$stem.key" -out "$P/certificates/$stem.crt" 2>/dev/null
echo "obtained a certificate for $D"
exit 0
'''

FAKE_HAPROXY = "#!/bin/sh\n" + RECORD + r'''
rec haproxy "$@"
if [ "$1" = "-v" ]; then echo "HAProxy version 3.0.11-1 2025/01/01 - https://haproxy.org/"; exit 0; fi
if [ "$1" = "-c" ]; then
  n=$(ls "$FAKE_DIR" | grep -c '^checked-')
  cp "$3" "$FAKE_DIR/checked-$n.cfg"
  if [ -f "$FAKE_DIR/haproxy.invalid" ]; then echo "[ALERT]    (1) : config : parsing [$3:7] : unknown keyword 'bogus'" >&2; echo "[ALERT]    (1) : Fatal errors found in configuration." >&2; exit 1; fi
  echo "Configuration file is valid"; exit 0
fi
exit 0
'''

FAKE_SYSTEMCTL = "#!/bin/sh\n" + RECORD + r'''
rec systemctl "$@"
cmd="$1"; shift
case "$cmd" in
  is-active) [ -f "$FAKE_DIR/active.$1" ] && { echo active; exit 0; }; echo inactive; exit 3 ;;
  is-enabled) [ -f "$FAKE_DIR/enabled.$1" ] && { echo enabled; exit 0; }; echo disabled; exit 1 ;;
  reload) [ -f "$FAKE_DIR/reload.fail" ] && { echo "reload failed" >&2; exit 1; }; exit 0 ;;
  restart) [ -f "$FAKE_DIR/start.fail" ] && { echo "start failed" >&2; exit 1; }; touch "$FAKE_DIR/active.$1"; exit 0 ;;
  enable) [ "$1" = "--now" ] && shift; touch "$FAKE_DIR/enabled.$1"; touch "$FAKE_DIR/active.$1"; exit 0 ;;
  disable) [ "$1" = "--now" ] && shift; rm -f "$FAKE_DIR/enabled.$1" "$FAKE_DIR/active.$1"; exit 0 ;;
  *) exit 0 ;;
esac
'''

FAKE_SYSTEMD_RUN = "#!/bin/sh\n" + RECORD + r'''
rec systemd-run "$@"
[ -f "$FAKE_DIR/systemd-run.fail" ] && { echo "Failed to start transient timer" >&2; exit 1; }
exit 0
'''

FAKE_APT = "#!/bin/sh\n" + RECORD + r'''
rec apt-get "$@"
[ -f "$FAKE_DIR/apt.fail" ] && { echo "E: Unable to locate package" >&2; exit 100; }
for last; do :; done
if [ -f "$FAKE_DIR/pkg/$last" ] && [ "$last" != "update" ]; then cp "$FAKE_DIR/pkg/$last" "$FAKE_BIN/$last"; chmod 755 "$FAKE_BIN/$last"; fi
exit 0
'''

FAKE_SS = "#!/bin/sh\n" + RECORD + r'''
rec ss "$@"
if [ -f "$FAKE_DIR/ss.out" ]; then cat "$FAKE_DIR/ss.out"; fi
if [ -f "$FAKE_DIR/active.haproxy" ]; then
  echo "LISTEN 0 4096 0.0.0.0:80 0.0.0.0:*"; echo "LISTEN 0 4096 0.0.0.0:443 0.0.0.0:*"; echo "LISTEN 0 4096 127.0.0.1:8444 0.0.0.0:*"
fi
exit 0
'''

FAKE_DIG = "#!/bin/sh\n" + RECORD + r'''
rec dig "$@"
name=""; for a; do name="$a"; done
[ -f "$FAKE_DIR/dig.fail" ] && { echo ";; connection timed out" >&2; exit 9; }
[ -f "$FAKE_DIR/dig.$name" ] && cat "$FAKE_DIR/dig.$name"
exit 0
'''


class FakeBin:
    """A temp directory with the fake executables; ``calls()`` returns what they were run with."""

    def __init__(self, with_lego: bool = True):
        self.dir = tempfile.mkdtemp(prefix="synofurb-fakebin-")
        self.bin = os.path.join(self.dir, "bin")
        self.state = os.path.join(self.dir, "state")
        os.makedirs(self.bin)
        os.makedirs(os.path.join(self.state, "pkg"))
        self.log = os.path.join(self.state, "calls.log")
        open(self.log, "w").close()
        for name, text in (("haproxy", FAKE_HAPROXY), ("systemctl", FAKE_SYSTEMCTL), ("systemd-run", FAKE_SYSTEMD_RUN), ("apt-get", FAKE_APT), ("ss", FAKE_SS)):
            self._write(self.bin, name, text)
        self._write(os.path.join(self.state, "pkg"), "lego", FAKE_LEGO)
        if with_lego:
            self._write(self.bin, "lego", FAKE_LEGO)
        # the fakes get these through the environment of the child process: run_command passes only a fixed environment, so bake them in
        for name in os.listdir(self.bin):
            self._bake(os.path.join(self.bin, name))
        self._bake(os.path.join(self.state, "pkg", "lego"))

    def _write(self, d: str, name: str, text: str) -> None:
        p = os.path.join(d, name)
        with open(p, "w") as fh:
            fh.write(text)
        os.chmod(p, 0o755)

    def _bake(self, path: str) -> None:
        with open(path) as fh:
            text = fh.read()
        text = text.replace("#!/bin/sh\n", '#!/bin/sh\nFAKE_DIR="%s"; FAKE_LOG="%s"; FAKE_BIN="%s"\n' % (self.state, self.log, self.bin), 1)
        with open(path, "w") as fh:
            fh.write(text)

    def add(self, name: str, text: str) -> None:
        self._write(self.bin, name, text)
        self._bake(os.path.join(self.bin, name))

    def flag(self, name: str, content: str = "1") -> None:
        with open(os.path.join(self.state, name), "w") as fh:
            fh.write(content)

    def unflag(self, name: str) -> None:
        try:
            os.unlink(os.path.join(self.state, name))
        except OSError:
            pass

    def calls(self, name: str | None = None) -> list[list[str]]:
        out = []
        with open(self.log) as fh:
            for ln in fh.read().splitlines():
                parts = ln.split(US)
                if name is None or parts[0] == name:
                    out.append(parts)
        return out

    def argvs(self, name: str) -> list[list[str]]:
        return [c[1:] for c in self.calls(name)]

    def clear(self) -> None:
        open(self.log, "w").close()

    def close(self) -> None:
        shutil.rmtree(self.dir, ignore_errors=True)


def make_selfsigned(directory: str, host: str = "127.0.0.1") -> tuple[str, str]:
    cert, key = os.path.join(directory, "srv.pem"), os.path.join(directory, "srv.key")
    san = "IP:127.0.0.1" if host[0].isdigit() else "DNS:" + host
    subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "30", "-subj", "/CN=" + host,
                    "-addext", "subjectAltName=" + san, "-keyout", key, "-out", cert], check=True, capture_output=True)
    return cert, key


class FakeAddrTools:
    """https://challenges.addr.tools on loopback.  POST secret=...&txt=... -> 201 OK (the TXT is recorded); only secret -> 200 with the sub-domain.
    ``mode``: ok | refuse (403) | busy (503) | wrong (200 with a different sub-domain).  ``requests`` keeps (path, form fields, content type)."""

    def __init__(self, secret: str):
        self.secret, self.mode, self.requests = secret, "ok", []
        self.dir = tempfile.mkdtemp(prefix="synofurb-addr-")
        self.cert, key = make_selfsigned(self.dir)
        outer = self

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                n = int(self.headers.get("Content-Length") or 0)
                form = {k: v[0] for k, v in urllib.parse.parse_qs(self.rfile.read(n).decode(), keep_blank_values=True).items()}
                outer.requests.append({"path": self.path, "form": form, "ctype": self.headers.get("Content-Type")})
                if outer.mode == "busy":
                    return self._send(503, "busy")
                if outer.mode == "refuse" or form.get("secret") != outer.secret:
                    return self._send(403, "unknown secret " + form.get("secret", ""))
                sub = hashlib.sha224(outer.secret.encode()).hexdigest()
                if "txt" not in form:
                    return self._send(200, ("0" * 56 if outer.mode == "wrong" else sub) + ".challenges.addr.tools")
                self._send(201, "OK")

            def _send(self, code, text):
                body = text.encode()
                self.send_response(code)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
        self.srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(self.cert, key)
        self.srv.socket = ctx.wrap_socket(self.srv.socket, server_side=True)
        self.url = "https://127.0.0.1:%d" % self.srv.server_address[1]
        threading.Thread(target=lambda: self.srv.serve_forever(poll_interval=0.02), daemon=True).start()

    def close(self) -> None:
        self.srv.shutdown()
        self.srv.server_close()
        shutil.rmtree(self.dir, ignore_errors=True)


class FakeDns:
    """A UDP DNS server on 127.0.0.1 answering CNAME queries from a dict {name: target} (NXDOMAIN-style empty answer otherwise)."""

    def __init__(self, cnames: dict[str, str]):
        self.cnames = cnames
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.port = self.sock.getsockname()[1]
        self.stop = False
        threading.Thread(target=self._run, daemon=True).start()

    @staticmethod
    def _enc(name: str) -> bytes:
        return b"".join(bytes([len(x)]) + x.encode() for x in name.split(".")) + b"\x00"

    def _run(self) -> None:
        self.sock.settimeout(0.1)
        while not self.stop:
            try:
                data, addr = self.sock.recvfrom(512)
            except (socket.timeout, OSError):
                continue
            txid = data[:2]
            pos, labels = 12, []
            while data[pos]:
                labels.append(data[pos + 1:pos + 1 + data[pos]].decode())
                pos += 1 + data[pos]
            name = ".".join(labels)
            qtype = struct.unpack(">H", data[pos + 1:pos + 3])[0]
            question = data[12:pos + 5]
            target = self.cnames.get(name)
            if target and qtype == 5:
                rd = self._enc(target)
                ans = b"\xc0\x0c" + struct.pack(">HHIH", 5, 1, 60, len(rd)) + rd          # compressed owner name: a pointer to the question
                self.sock.sendto(txid + struct.pack(">HHHHH", 0x8180, 1, 1, 0, 0) + question + ans, addr)
            else:
                self.sock.sendto(txid + struct.pack(">HHHHH", 0x8183, 1, 0, 0, 0) + question, addr)

    def close(self) -> None:
        self.stop = True
        self.sock.close()


class Clock:
    """Real time plus an offset: certificates made by the fake lego carry real dates, so the module's clock must agree with them; sleep() moves the offset."""

    def __init__(self):
        self.offset = 0.0

    @property
    def t(self) -> float:
        return time.time() + self.offset

    @t.setter
    def t(self, value: float) -> None:
        self.offset = value - time.time()

    def now(self) -> float:
        return time.time() + self.offset

    def sleep(self, s: float) -> None:
        self.offset += s
