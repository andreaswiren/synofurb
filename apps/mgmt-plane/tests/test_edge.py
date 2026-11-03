"""Certificates and the HAProxy front (synofurb_edge.py + the glue in synofurb_mgmt.py).

Nothing here talks to Let's Encrypt, a real DNS zone, a real lego, a real HAProxy or a real systemd: lego, haproxy, systemctl, systemd-run, apt-get, ss and dig are
fake executables (tests/edge_fakes.py) that record the exact argv and environment they were run with; challenges.addr.tools is a loopback HTTPS server; DNS is a loopback
UDP server.  The module's real ``run_command`` runs them, so what the tests assert is the command line the module really builds.
"""
from __future__ import annotations

import hashlib
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(ROOT))
sys.path.insert(0, ROOT)
sys.path.insert(0, HERE)
import synofurb_edge as E  # noqa: E402
import synofurb_mgmt as m  # noqa: E402
from edge_fakes import FakeAddrTools, FakeBin, FakeDns, Clock, FAKE_DIG, make_selfsigned  # noqa: E402
from test_mgmt import Client, Env  # noqa: E402

SECRET = "correct-horse-battery-staple-0123456789-xyz"
HAVE_OPENSSL = bool(shutil.which("openssl"))


def jload(path):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def slurp(path):
    with open(path, encoding="utf-8") as fh:
        return fh.read()


def mode(path):
    return stat.S_IMODE(os.stat(path).st_mode)


class Fx:
    """One temp root + fake executables + an Edge around them."""

    def __init__(self, with_lego=True, sites=None, haproxy_dir=True):
        self.tmp = tempfile.mkdtemp(prefix="synofurb-edge-")
        self.fb = FakeBin(with_lego)
        self.fb.add("dig", FAKE_DIG)
        self.clock = Clock()
        self.audits: list = []
        self.restarts: list = []
        self.hosts: dict = {}
        self.sites = sites if sites is not None else []
        self.env = E.EdgeEnv(root=self.tmp, bin_dir=self.fb.bin, state_dir="/var/lib/synofurb-mgmt", script_path="/opt/synofurb-mgmt/synofurb_mgmt.py",
                             audit=lambda *a: self.audits.append(a), now=self.clock.now, sleep=self.clock.sleep, resolve_host=lambda n: self.hosts.get(n, []),
                             local_ips=lambda: ["192.168.1.5"], sites=lambda: self.sites, restart_mgmt=lambda: self.restarts.append(self.clock.now()), mgmt_port=8443)
        self.edge = E.Edge(self.env)
        if haproxy_dir:
            os.makedirs(self.p("/etc/haproxy"))
        d = self.p("/var/lib/synofurb-mgmt/tls")                       # the management plane's own certificate (what the front uses as its default)
        os.makedirs(d)
        if HAVE_OPENSSL:
            c, k = make_selfsigned(d, "localhost")
            shutil.move(c, d + "/cert.pem")
            shutil.move(k, d + "/key.pem")

    def p(self, path):
        return self.env.p(path)

    def read(self, path):
        return E.read_text(self.p(path))

    def configure(self, environment="staging", secret=SECRET):
        return E.update_settings(self.env, {"email": "admin@example.org", "accept_tos": True, "environment": environment, "secret": secret})

    def cname_ok(self, *names):
        target = E.cname_target(E.load_secret(self.env)) + "."
        for n in names:
            self.fb.flag("dig." + E.challenge_record(n), target + "\n")

    def issue(self, names=("example.org",), challenge="dns-01", environment="staging", **kw):
        if challenge == "dns-01":
            self.cname_ok(*names)
        return self.edge.certs.issue(dict(names=list(names), challenge=challenge, environment=environment, **kw), "tester", "10.0.0.1", background=False)

    def close(self):
        self.fb.close()
        shutil.rmtree(self.tmp, ignore_errors=True)


@unittest.skipUnless(HAVE_OPENSSL, "openssl missing")
class Base(unittest.TestCase):
    def setUp(self):
        self.fx = Fx()
        self.addCleanup(self.fx.close)
        self.fx.configure()


# ============================================================================================================================
# names, secrets, certificates
# ============================================================================================================================

class NameTests(unittest.TestCase):
    def test_valid_names(self):
        self.assertEqual(E.valid_domain("Example.ORG"), "example.org")
        self.assertEqual(E.valid_domain("a.b.example.org."), "a.b.example.org")
        self.assertEqual(E.valid_domain("*.example.org", wildcard=True), "*.example.org")
        self.assertEqual(E.valid_domain("münchen.de"), "xn--mnchen-3ya.de")                  # punycode, never raw unicode
        self.assertEqual(E.valid_domain("xn--mnchen-3ya.de"), "xn--mnchen-3ya.de")

    def test_hostile_names_are_refused(self):
        bad = ["", "example", "*.example.org", "a*.example.org", "foo.*.example.org", "-a.example.org", "a-.example.org", "a..example.org", "a b.example.org",
               "a/b.example.org", "../etc/passwd", "a.example.org/../x", "a.example.org\nbind :22", "a.example.org\r\nx", "x;y.example.org", "a_b.example.org", "$(id).example.org",
               "`id`.example.org", "1.2.3.4", "::1", "example.local", "host.localhost", "x.home.arpa", "a" * 64 + ".example.org", ("a." * 130) + "org", "exa\x00mple.org", None, 5, ["a.example.org"],
               "a.example.org:443", "user@example.org", "‮example.org"]
        for v in bad:
            with self.assertRaises(E.EdgeError, msg=repr(v)):
                E.valid_domain(v)

    def test_names_list(self):
        self.assertEqual(E.valid_names("A.example.org, b.example.org  a.example.org", "http-01"), ["a.example.org", "b.example.org"])
        with self.assertRaises(E.EdgeError):
            E.valid_names(["*.example.org"], "http-01")                                          # wildcard needs DNS
        with self.assertRaises(E.EdgeError):
            E.valid_names([], "dns-01")
        with self.assertRaises(E.EdgeError):
            E.valid_names(["a%d.example.org" % i for i in range(E.MAX_NAMES + 1)], "dns-01")

    def test_cert_id_never_leaves_the_directory(self):
        self.assertEqual(E.cert_id_for("example.org"), "example.org")
        self.assertEqual(E.cert_id_for("*.example.org"), "_wildcard.example.org")
        for bad in ("../x", "a/b", ".hidden.example.org", "x..y", "", "/etc/passwd", "A.example.org", "a.example.org/"):
            with self.assertRaises(E.EdgeError, msg=bad):
                E.valid_cert_id(bad)

    def test_wildcard_matching(self):
        self.assertTrue(E.name_matches("*.example.org", "a.example.org"))
        self.assertFalse(E.name_matches("*.example.org", "a.b.example.org"))
        self.assertFalse(E.name_matches("*.example.org", "example.org"))
        self.assertTrue(E.covers(["example.org", "*.example.org"], "example.org"))
        self.assertFalse(E.covers(["a.example.org"], "b.example.org"))

    def test_challenge_record_shares_wildcard_and_base(self):
        self.assertEqual(E.challenge_record("*.example.org"), "_acme-challenge.example.org")
        self.assertEqual(E.challenge_record("example.org"), "_acme-challenge.example.org")


class Sha224Tests(unittest.TestCase):
    VECTORS = {"": "d14a028c2a3a2bc9476102bb288234c415a2b01f828ea62ac5b3e42f", "abc": "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7",
               "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq": "75388b16512776cc5dba5da1fd890150b0c6455cb4f58b1952522525"}      # FIPS 180-4 examples

    def test_vectors(self):
        for text, want in self.VECTORS.items():
            self.assertEqual(E.addr_hash(text), want)
            self.assertEqual(want, hashlib.sha224(text.encode()).hexdigest())

    def test_target_shape(self):
        t = E.cname_target(SECRET)
        h, _, rest = t.partition(".")
        self.assertEqual(rest, "challenges.addr.tools")
        self.assertRegex(h, r"^[0-9a-f]{56}$")                                                   # lowercase hex, exactly 56 characters
        self.assertEqual(h, hashlib.sha224(SECRET.encode()).hexdigest())

    def test_secret_generation_and_validation(self):
        a, b = E.generate_secret(), E.generate_secret()
        self.assertNotEqual(a, b)
        self.assertGreaterEqual(len(a), 32)
        self.assertRegex(a, r"^[A-Za-z0-9_-]+$")
        self.assertEqual(E.valid_secret(a), a)
        for bad in ("short", "has space in it 1234567890", "x" * 129, "line\nbreak-1234567890", 5, None):
            with self.assertRaises(E.EdgeError):
                E.valid_secret(bad)


class CertParseTests(unittest.TestCase):
    @unittest.skipUnless(HAVE_OPENSSL, "openssl missing")
    def test_san_validity_fingerprint(self):
        with tempfile.TemporaryDirectory() as d:
            subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "45", "-subj", "/CN=example.org",
                            "-addext", "subjectAltName=DNS:example.org,DNS:*.example.org", "-keyout", d + "/k", "-out", d + "/c"], check=True, capture_output=True)
            pem = slurp(d + "/c")
            info = E.parse_cert(pem)
            fp = subprocess.run(["openssl", "x509", "-in", d + "/c", "-noout", "-fingerprint", "-sha256"], capture_output=True, text=True).stdout.split("=")[1].strip()
        self.assertEqual(info["sans"], ["example.org", "*.example.org"])
        self.assertEqual(info["cn"], "example.org")
        self.assertEqual(info["fingerprint"], fp)
        self.assertAlmostEqual((info["not_after"] - info["not_before"]) / 86400, 45, delta=0.01)

    def test_garbage_is_refused(self):
        for bad in ("", "not a pem", "-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----", "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----"):
            with self.assertRaises(E.EdgeError):
                E.parse_cert(bad)


# ============================================================================================================================
# settings and the secret
# ============================================================================================================================

class SettingsTests(Base):
    def test_files_are_private_and_the_secret_is_not_in_the_view(self):
        view = E.settings_view(self.fx.env)
        self.assertNotIn(SECRET, json.dumps(view))
        self.assertTrue(view["secret"]["set"])
        self.assertEqual(view["secret"]["target"], E.cname_target(SECRET))
        for rel in ("settings.json", "addr-tools.json"):
            self.assertEqual(mode(self.fx.env.edge_path(rel)), 0o600, rel)
        self.assertEqual(mode(self.fx.env.edge_path("")), 0o700)
        self.assertEqual(E.load_secret(self.fx.env), SECRET)
        self.assertNotIn(SECRET, self.fx.read("/etc/synofurb/edge/settings.json"))

    def test_generate_shows_the_secret_once(self):
        d = E.update_settings(self.fx.env, {"generate_secret": True})
        self.assertGreaterEqual(len(d["secret_once"]), 32)
        self.assertEqual(E.load_secret(self.fx.env), d["secret_once"])
        again = E.settings_view(self.fx.env)
        self.assertNotIn("secret_once", again)
        self.assertNotIn(d["secret_once"], json.dumps(again))
        self.assertEqual(again["secret"]["target"], E.cname_target(d["secret_once"]))

    def test_validation(self):
        for body in ({"email": "nope"}, {"email": "a b@example.org"}, {"environment": "prod"}, {"key_type": "dsa"}, {"secret": "short"}, {"dns_resolvers": "not-an-ip"},
                     {"dns_resolvers": "1.1.1.1,2.2.2.2,3.3.3.3,4.4.4.4"}, {"addr_tools_url": "http://challenges.addr.tools"}, {"addr_tools_url": "https://x/y z"},
                     {"addr_tools_ca": "/etc/passwd"}, {"addr_tools_ca": "/etc/synofurb/edge/../../passwd"}, {"dns_skip_check": "yes"}):
            with self.assertRaises(E.EdgeError, msg=body):
                E.update_settings(self.fx.env, body)

    def test_terms_must_be_accepted_to_issue(self):
        fx = Fx()
        self.addCleanup(fx.close)
        with self.assertRaises(E.EdgeError) as cm:
            fx.edge.certs.issue({"names": ["example.org"], "challenge": "dns-01"}, background=False)
        self.assertIn("accept the Let's Encrypt terms", str(cm.exception))


class AddrToolsTests(Base):
    def setUp(self):
        super().setUp()
        self.srv = FakeAddrTools(SECRET)
        self.addCleanup(self.srv.close)
        shutil.copy(self.srv.cert, self.fx.env.edge_path("ca.pem"))
        E.update_settings(self.fx.env, {"addr_tools_url": self.srv.url, "addr_tools_ca": "/etc/synofurb/edge/ca.pem"})

    def test_sanity_request_sends_only_the_secret(self):
        r = E.check_secret(self.fx.env)
        self.assertTrue(r["ok"], r)
        self.assertEqual(self.srv.requests[-1]["form"], {"secret": SECRET})                      # nothing is published by the check
        self.assertEqual(self.srv.requests[-1]["ctype"], "application/x-www-form-urlencoded")

    def test_sanity_request_failures(self):
        self.srv.mode = "wrong"
        self.assertEqual(E.check_secret(self.fx.env)["state"], "mismatch")
        self.srv.mode = "refuse"
        r = E.check_secret(self.fx.env)
        self.assertEqual((r["state"], r["status"]), ("refused", 403))
        self.assertNotIn(SECRET, json.dumps(r))
        self.srv.close()
        r = E.check_secret(self.fx.env)
        self.assertEqual(r["state"], "unreachable")
        self.assertNotIn(SECRET, json.dumps(r))

    def adapter(self, *args):
        path = E.ensure_adapter(self.fx.env)
        return subprocess.run([sys.executable, path, *args], capture_output=True, text=True, timeout=60)

    def test_adapter_file_is_private_and_holds_no_secret(self):
        path = E.ensure_adapter(self.fx.env)
        self.assertEqual(mode(path), 0o700)
        text = slurp(path)
        self.assertNotIn(SECRET, text)
        self.assertNotIn(E.addr_hash(SECRET), text)
        self.assertIn(self.fx.env.edge_path("addr-tools.json"), text)
        compile(text, "adapter", "exec")
        self.assertEqual(E.ensure_adapter(self.fx.env), path)                                    # idempotent

    def test_present_posts_secret_and_txt_and_wants_201(self):
        r = self.adapter("present", "--", "_acme-challenge.example.org.", "LHDhK3oGRvkiefQnx7OOczTY5Tic_xZ6HcMOc_gmtoM")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.srv.requests[-1]["form"], {"secret": SECRET, "txt": "LHDhK3oGRvkiefQnx7OOczTY5Tic_xZ6HcMOc_gmtoM"})
        r = self.adapter("present", "_acme-challenge.example.org.", "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG")          # lego without the "--"
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(len(self.srv.requests), 2)
        self.assertNotIn(SECRET, r.stdout + r.stderr)

    def test_cleanup_and_timeout_do_nothing(self):
        for args in (("cleanup", "--", "_acme-challenge.example.org.", "v" * 43), ("timeout",)):
            self.assertEqual(self.adapter(*args).returncode, 0)
        self.assertEqual(self.srv.requests, [])

    def test_other_codes_fail_without_leaking_the_secret(self):
        for mode_ in ("refuse", "busy"):
            self.srv.mode = mode_
            r = self.adapter("present", "--", "_acme-challenge.example.org.", "v" * 43)
            self.assertEqual(r.returncode, 1, mode_)
            self.assertNotIn(SECRET, r.stdout + r.stderr)
        self.srv.mode = "refuse"
        self.assertIn("403", self.adapter("present", "x.org", "v" * 43).stderr)

    def test_bad_input_and_missing_configuration(self):
        self.assertEqual(self.adapter("present", "--", "fqdn", "bad value with spaces").returncode, 2)
        self.assertEqual(self.adapter("bogus").returncode, 2)
        self.assertEqual(self.adapter().returncode, 2)
        os.unlink(self.fx.env.edge_path("addr-tools.json"))
        self.assertEqual(self.adapter("present", "--", "fqdn", "v" * 43).returncode, 1)


# ============================================================================================================================
# DNS
# ============================================================================================================================

class DnsTests(Base):
    def test_check_dns_states_through_dig(self):
        fx, target = self.fx, E.cname_target(SECRET)
        r = E.check_dns(fx.env, "example.org")
        self.assertEqual((r["state"], r["record"], r["target"]), ("missing", "_acme-challenge.example.org", target))
        fx.fb.flag("dig._acme-challenge.example.org", target.upper() + ".\n")
        self.assertEqual(E.check_dns(fx.env, "*.example.org")["state"], "ok")                  # wildcard and base share the record; case does not matter
        fx.fb.flag("dig._acme-challenge.example.org", "0" * 56 + ".challenges.addr.tools.\n")
        r = E.check_dns(fx.env, "example.org")
        self.assertEqual((r["state"], r["found"]), ("wrong", "0" * 56 + ".challenges.addr.tools"))
        fx.fb.flag("dig.fail")
        self.assertEqual(E.check_dns(fx.env, "example.org")["state"], "error")
        argv = fx.fb.argvs("dig")[-1]
        self.assertEqual(argv, ["+short", "+time=3", "+tries=1", "CNAME", "_acme-challenge.example.org"])

    def test_check_dns_without_secret(self):
        fx = Fx()
        self.addCleanup(fx.close)
        self.assertEqual(E.check_dns(fx.env, "example.org")["state"], "nosecret")

    def test_dns_names_are_validated_before_any_command(self):
        with self.assertRaises(E.EdgeError):
            E.check_dns(self.fx.env, "x.org; rm -rf /")
        self.assertEqual(self.fx.fb.argvs("dig"), [])

    def test_builtin_udp_resolver_when_dig_is_missing(self):
        fx = Fx()
        self.addCleanup(fx.close)
        os.unlink(os.path.join(fx.fb.bin, "dig"))
        E.update_settings(fx.env, {"secret": SECRET})
        dns = FakeDns({"_acme-challenge.example.org": E.cname_target(SECRET)})
        self.addCleanup(dns.close)
        orig = E.system_resolvers
        E.system_resolvers = lambda env: ["127.0.0.1:%d" % dns.port]
        try:
            self.assertEqual(E.check_dns(fx.env, "example.org")["state"], "ok")                # a compressed answer name is followed
            self.assertEqual(E.check_dns(fx.env, "other.org")["state"], "missing")
        finally:
            E.system_resolvers = orig

    def test_dns_answer_parser_handles_compression(self):
        q = b"\x12\x34\x81\x80\x00\x01\x00\x01\x00\x00\x00\x00" + b"\x03www\x07example\x03org\x00\x00\x05\x00\x01"
        rd = b"\x06target\x07example\x03org\x00"
        ans = b"\xc0\x0c\x00\x05\x00\x01\x00\x00\x00\x3c" + bytes([0, len(rd)]) + rd
        r = E.parse_dns_answer(q + ans, 0x1234)
        self.assertEqual(r["answers"], [("www.example.org", 5, "target.example.org")])
        with self.assertRaises(ValueError):
            E.parse_dns_answer(q + ans, 0x9999)


# ============================================================================================================================
# lego: the argv, the environment, issue / change / renew / remove
# ============================================================================================================================

class LegoArgvTests(unittest.TestCase):
    def argv(self, **kw):
        base = dict(command="run", environment="staging", email="a@example.org", path="/etc/synofurb/lego/staging", names=["example.org", "*.example.org"], challenge="dns-01", key_type="ec256")
        base.update(kw)
        return E.lego_argv("/usr/bin/lego", **base)

    def test_dns(self):
        self.assertEqual(self.argv(), ["/usr/bin/lego", "--server", "https://acme-staging-v02.api.letsencrypt.org/directory", "--email", "a@example.org", "--accept-tos",
                                       "--path", "/etc/synofurb/lego/staging", "--key-type", "ec256", "--domains", "example.org", "--domains", "*.example.org", "--dns", "exec", "run"])

    def test_dns_options_and_renew(self):
        a = self.argv(command="renew", environment="production", dns_skip_check=True, resolvers=["1.1.1.1", "9.9.9.9:53"], days=30)
        self.assertEqual(a[2], "https://acme-v02.api.letsencrypt.org/directory")
        self.assertEqual(a[-9:], ["--dns", "exec", "--dns.disable-cp", "--dns.resolvers", "1.1.1.1", "--dns.resolvers", "9.9.9.9:53", "renew", "--days", "30"][-9:])
        self.assertEqual(a[-3:], ["renew", "--days", "30"])

    def test_http_and_tls_alpn(self):
        a = self.argv(challenge="http-01", names=["example.org"], http_listen="127.0.0.1:5080")
        self.assertEqual(a[-4:], ["--http", "--http.port", "127.0.0.1:5080", "run"])
        a = self.argv(challenge="tls-alpn-01", names=["example.org"], alpn_listen=":443")
        self.assertEqual(a[-4:], ["--tls", "--tls.port", ":443", "run"])

    def test_nothing_unvalidated_gets_through(self):
        for kw in ({"names": ["bad name.org"]}, {"names": ["x.org\n--dns"]}, {"email": "a@b"}, {"email": "a@example.org --server x"}, {"challenge": "dns-02"}, {"environment": "prod"},
                   {"command": "revoke"}, {"key_type": "rsa1"}):
            with self.assertRaises(E.EdgeError, msg=kw):
                self.argv(**kw)

    def test_env_names_the_adapter_and_carries_no_secret(self):
        env = E.lego_env("/usr/local/sbin/synofurb-acme-addr-tools")
        self.assertEqual(env["EXEC_PATH"], "/usr/local/sbin/synofurb-acme-addr-tools")
        self.assertTrue(all(k.startswith("EXEC_") for k in env))
        self.assertNotIn("secret", json.dumps(env).lower())


class IssueTests(Base):
    def test_issue_dns_wildcard_end_to_end(self):
        fx = self.fx
        job = fx.issue(["example.org", "*.example.org"])
        self.assertEqual((job.ok, job.state), (True, "done"), job.error)
        # the exact command line and environment of lego
        argv = fx.fb.argvs("lego")[-1]
        self.assertEqual(argv, ["--server", E.ACME_URLS["staging"], "--email", "admin@example.org", "--accept-tos", "--path", fx.p("/etc/synofurb/lego/staging"), "--key-type", "ec256",
                                "--domains", "example.org", "--domains", "*.example.org", "--dns", "exec", "run"])
        env = slurp(os.path.join(fx.fb.state, "lego.env"))
        self.assertIn("EXEC_PATH=" + fx.p(E.ADAPTER_PATH), env)
        self.assertNotIn(SECRET, env + json.dumps(fx.fb.calls()))                                 # the secret is in no argv and no environment
        # the store: directories 0700, files 0600
        d = fx.p("/etc/synofurb/certs/example.org")
        self.assertEqual(mode(d), 0o700)
        for f in ("fullchain.pem", "privkey.pem", "meta.json"):
            self.assertEqual(mode(d + "/" + f), 0o600, f)
        meta = jload(d + "/meta.json")
        self.assertEqual((meta["primary"], meta["names"], meta["challenge"], meta["environment"]), ("example.org", ["example.org", "*.example.org"], "dns-01", "staging"))
        self.assertGreater(meta["expires"], fx.clock.now() + 80 * 86400)
        self.assertEqual(meta["last_error"], "")
        # the combined PEM for HAProxy: chain first, then the key
        combined = fx.read("/etc/haproxy/certs/example.org.pem")
        self.assertIn("BEGIN CERTIFICATE", combined)
        self.assertLess(combined.index("BEGIN CERTIFICATE"), combined.index("PRIVATE KEY"))
        self.assertEqual(mode(fx.p("/etc/haproxy/certs/example.org.pem")), 0o600)
        # the audit trail and the renewal timer
        kinds = [a[2] for a in fx.audits]
        self.assertIn("certs.job.start", kinds)
        self.assertIn("certs.job.issue", kinds)
        self.assertTrue(os.path.isfile(fx.p("/etc/systemd/system/synofurb-certs-renew.timer")))
        self.assertIn(["enable", "--now", "synofurb-certs-renew.timer"], fx.fb.argvs("systemctl"))

    def test_staging_and_production_have_their_own_lego_account_folder(self):
        fx = self.fx
        fx.issue(["a.example.org"], environment="staging")
        fx.issue(["b.example.org"], environment="production")
        paths = [a[a.index("--path") + 1] for a in fx.fb.argvs("lego")]
        self.assertEqual(paths, [fx.p("/etc/synofurb/lego/staging"), fx.p("/etc/synofurb/lego/production")])
        self.assertTrue(os.path.isfile(fx.p("/etc/synofurb/lego/staging/certificates/a.example.org.crt")))
        self.assertEqual(mode(fx.p("/etc/synofurb/lego")), 0o700)

    def test_wildcard_primary_uses_a_safe_id_and_lego_stem(self):
        fx = self.fx
        fx.issue(["*.example.org", "example.org"])
        self.assertTrue(os.path.isfile(fx.p("/etc/synofurb/certs/_wildcard.example.org/fullchain.pem")))
        self.assertTrue(os.path.isfile(fx.p("/etc/synofurb/lego/staging/certificates/_.example.org.crt")))

    def test_http01_and_alpn_use_the_right_listeners(self):
        fx = self.fx
        fx.issue(["a.example.org"], "http-01")
        self.assertEqual(fx.fb.argvs("lego")[-1][-4:], ["--http", "--http.port", ":80", "run"])                 # front off: lego listens itself
        fx.issue(["b.example.org"], "tls-alpn-01")
        self.assertEqual(fx.fb.argvs("lego")[-1][-4:], ["--tls", "--tls.port", ":443", "run"])
        st = fx.edge.proxy.state()
        st["enabled"] = True
        fx.edge.proxy.save_state(st)
        fx.fb.flag("active.haproxy")
        fx.issue(["c.example.org"], "http-01")
        self.assertEqual(fx.fb.argvs("lego")[-1][-4:], ["--http", "--http.port", "127.0.0.1:5080", "run"])      # behind HAProxy: loopback port
        fx.issue(["d.example.org"], "tls-alpn-01")
        self.assertEqual(fx.fb.argvs("lego")[-1][-4:], ["--tls", "--tls.port", "127.0.0.1:5443", "run"])

    def test_port_in_use_is_refused_before_lego_runs(self):
        fx = self.fx
        fx.fb.flag("ss.out", "LISTEN 0 4096 0.0.0.0:80 0.0.0.0:*\n")
        job = fx.issue(["a.example.org"], "http-01")
        self.assertFalse(job.ok)
        self.assertIn("Port 80 is in use", job.error)
        self.assertEqual(fx.fb.argvs("lego"), [])

    def test_dns_challenge_needs_the_cname_first(self):
        fx = self.fx
        job = fx.edge.certs.issue({"names": ["example.org"], "challenge": "dns-01", "environment": "staging"}, background=False)
        self.assertFalse(job.ok)
        self.assertIn("Create the CNAME record first", job.error)
        self.assertIn("_acme-challenge.example.org", job.error)
        self.assertEqual(fx.fb.argvs("lego"), [])
        job = fx.edge.certs.issue({"names": ["example.org"], "challenge": "dns-01", "environment": "staging", "skip_dns_check": True}, background=False)
        self.assertTrue(job.ok, job.error)
        self.assertEqual(len(fx.fb.argvs("lego")), 1)

    def test_dns_challenge_needs_a_secret(self):
        fx = Fx()
        self.addCleanup(fx.close)
        E.update_settings(fx.env, {"email": "a@example.org", "accept_tos": True})
        job = fx.edge.certs.issue({"names": ["example.org"], "challenge": "dns-01"}, background=False)
        self.assertIn("Set the challenge secret first", job.error)

    def test_lego_is_installed_on_demand(self):
        fx = Fx(with_lego=False)
        self.addCleanup(fx.close)
        fx.configure()
        job = fx.issue(["example.org"])
        self.assertTrue(job.ok, job.error)
        apt = fx.fb.argvs("apt-get")
        self.assertEqual(apt[0][-1], "update")
        self.assertEqual(apt[1][-3:], ["install", "--no-install-recommends", "lego"])
        self.assertNotIn("sh", [a[0] for a in apt])

    def test_offline_unit_gets_a_clear_message(self):
        fx = Fx(with_lego=False)
        self.addCleanup(fx.close)
        fx.configure()
        fx.fb.flag("apt.fail")
        job = fx.issue(["example.org"])
        self.assertFalse(job.ok)
        self.assertIn("needs the Debian mirror and Internet access", job.error)

    def test_failures_are_explained_and_remembered(self):
        fx = self.fx
        cases = [("acme: error: 429 :: POST :: urn:ietf:params:acme:error:rateLimited :: too many certificates already issued", "limit"),
                 ("error: one or more domains had a problem: [x.org] acme: error: 400 :: urn:ietf:params:acme:error:dns :: DNS problem: NXDOMAIN looking up A for x.org", "does not exist"),
                 ("Timeout during connect (likely firewall problem)", "port 80"),
                 ("listen tcp :80: bind: address already in use", "already in use"),
                 ("[x.org] time limit exceeded: last error: NS ns1 did not return the expected TXT record", "did not appear in time")]
        for n, (out, want) in enumerate(cases):
            fx.fb.flag("lego.fail", out + "\n")
            job = fx.issue(["f%d.example.org" % n])
            self.assertFalse(job.ok)
            self.assertIn(want, job.error)
            meta = jload(fx.p("/etc/synofurb/certs/f%d.example.org/meta.json" % n))
            self.assertIn(want, meta["last_error"])
            self.assertIsNone(meta["expires"])
            self.assertEqual(fx.edge.certs.view("f%d.example.org" % n)["status"], "failed")

    def test_the_job_log_never_shows_the_secret(self):
        fx = self.fx
        fx.fb.flag("leak.txt", "debug: using secret %s here\n" % SECRET)
        job = fx.issue(["example.org"])
        self.assertTrue(job.ok)
        log = "\n".join(job.view()["log"])
        self.assertNotIn(SECRET, log)
        self.assertIn("***", log)
        self.assertNotIn(SECRET, json.dumps(fx.audits))
        self.assertNotIn(SECRET, fx.read("/var/lib/synofurb-mgmt/edge/jobs.json"))

    def test_hostile_names_never_reach_lego(self):
        fx = self.fx
        for bad in ("x.org\n--server http://evil", "a b.example.org", "../../etc/passwd", "$(id).example.org", "x.org;id"):
            with self.assertRaises(E.EdgeError, msg=bad):
                fx.edge.certs.issue({"names": [bad], "challenge": "dns-01"}, background=False)
        self.assertEqual(fx.fb.argvs("lego"), [])
        self.assertEqual(os.listdir(fx.p("/etc/synofurb/certs")) if os.path.isdir(fx.p("/etc/synofurb/certs")) else [], [])

    def test_issue_twice_needs_change_or_renew(self):
        fx = self.fx
        fx.issue(["example.org"])
        with self.assertRaises(E.EdgeError) as cm:
            fx.issue(["example.org"])
        self.assertIn("exists already", str(cm.exception))

    def test_change_keeps_the_primary_and_runs_lego_run(self):
        fx = self.fx
        fx.issue(["example.org"])
        fx.cname_ok("example.org")
        job = fx.edge.certs.issue({"names": ["example.org", "www.example.org"], "challenge": "http-01", "environment": "staging"}, background=False, change="example.org")
        self.assertTrue(job.ok, job.error)
        self.assertEqual(fx.edge.certs.store.meta("example.org")["names"], ["example.org", "www.example.org"])
        self.assertEqual(fx.edge.certs.store.meta("example.org")["challenge"], "http-01")
        with self.assertRaises(E.EdgeError):
            fx.edge.certs.issue({"names": ["other.example.org"], "challenge": "dns-01"}, background=False, change="example.org")

    def test_a_failed_change_keeps_the_working_certificate(self):
        fx = self.fx
        fx.issue(["example.org"])
        before = fx.read("/etc/synofurb/certs/example.org/fullchain.pem")
        fx.fb.flag("lego.fail", "boom\n")
        job = fx.edge.certs.issue({"names": ["example.org", "x.example.org"], "challenge": "http-01", "environment": "staging"}, background=False, change="example.org")
        self.assertFalse(job.ok)
        self.assertEqual(fx.read("/etc/synofurb/certs/example.org/fullchain.pem"), before)
        v = fx.edge.certs.view("example.org")
        self.assertEqual(v["names"], ["example.org"])
        self.assertTrue(v["last_error"])

    def test_remove_needs_the_typed_name_and_refuses_while_in_use(self):
        fx = self.fx
        fx.issue(["example.org"])
        with self.assertRaises(E.EdgeError):
            fx.edge.certs.remove("example.org", "example")
        st = fx.edge.proxy.state()
        st["mgmt"] = {"hostname": "example.org", "cert": "example.org"}
        fx.edge.proxy.save_state(st)
        with self.assertRaises(E.EdgeError) as cm:
            fx.edge.certs.remove("example.org", "example.org")
        self.assertIn("used by example.org", str(cm.exception))
        fx.edge.proxy.save_state(E.json.loads(json.dumps(E.PROXY_DEFAULTS)))
        fx.edge.certs.remove("example.org", "example.org", "tester", "-")
        self.assertFalse(os.path.exists(fx.p("/etc/synofurb/certs/example.org")))
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/certs/example.org.pem")))
        self.assertFalse(os.path.exists(fx.p("/etc/synofurb/lego/staging/certificates/example.org.crt")))
        with self.assertRaises(E.NotFound):
            fx.edge.certs.remove("example.org", "example.org")

    def test_one_job_at_a_time(self):
        fx = self.fx

        class Held:
            def acquire(self, *a):
                return None

            def holder(self):
                return {"kind": "update", "label": "from github"}

            def release(self, t):
                pass
        fx.env.job_lock = Held()
        with self.assertRaises(E.Busy) as cm:
            fx.issue(["example.org"])
        self.assertIn("Another job is running (update from github)", str(cm.exception))
        self.assertEqual(fx.fb.argvs("lego"), [])


class RateLimitTests(Base):
    def test_duplicate_limit_is_checked_before_lego(self):
        fx = self.fx
        for _ in range(5):
            fx.edge.certs.record_event(["example.org"], "production", True)
        job = fx.issue(["example.org"], environment="production")
        self.assertFalse(job.ok)
        self.assertIn("5 identical certificates per week", job.error)
        self.assertEqual(fx.fb.argvs("lego"), [])
        job = fx.issue(["example.org"], environment="staging")                                      # staging has its own, generous limits
        self.assertTrue(job.ok, job.error)

    def test_warning_at_four_and_expiry_after_a_week(self):
        fx = self.fx
        for _ in range(4):
            fx.edge.certs.record_event(["a.example.org"], "production", True)
        job = fx.issue(["a.example.org"], environment="production")
        self.assertTrue(job.ok, job.error)
        self.assertTrue(any("4 of 5" in w for w in job.warnings))
        fx.clock.t += 8 * 86400
        self.assertEqual(fx.edge.certs.rate_check(["a.example.org"], "production"), [])

    def test_failed_validation_limit(self):
        fx = self.fx
        for _ in range(5):
            fx.edge.certs.record_event(["b.example.org"], "production", False)
        job = fx.issue(["b.example.org"], environment="production")
        self.assertIn("5 failed attempts per hour", job.error)
        fx.clock.t += 3700
        self.assertEqual(fx.edge.certs.rate_check(["b.example.org"], "production"), [])


class PrecheckTests(Base):
    def test_http_precheck_reports_resolution_ports_and_nat(self):
        fx = self.fx
        fx.hosts = {"a.example.org": ["192.168.1.5"], "b.example.org": ["203.0.113.9"], "c.example.org": ["10.0.0.7"]}
        r = fx.edge.certs.precheck(["a.example.org", "b.example.org", "c.example.org", "d.example.org"], "http-01")
        by = {x["name"]: x for x in r["names"]}
        self.assertTrue(by["a.example.org"]["local"])
        self.assertFalse(by["b.example.org"]["local"])
        self.assertTrue(by["c.example.org"]["private"])
        self.assertEqual(by["d.example.org"]["state"], "missing")
        self.assertEqual(r["ports"][0]["port"], 80)
        self.assertEqual(r["ports"][0]["state"], "ok")
        self.assertIn("cannot be tested from here", r["note"])
        self.assertFalse(r["ok"])
        fx.fb.flag("ss.out", "LISTEN 0 4096 0.0.0.0:443 0.0.0.0:*\n")
        self.assertEqual(fx.edge.certs.precheck(["a.example.org"], "tls-alpn-01")["ports"][0]["state"], "busy")

    def test_dns_precheck_is_the_cname_check(self):
        fx = self.fx
        fx.cname_ok("a.example.org")
        r = fx.edge.certs.precheck(["a.example.org", "b.example.org"], "dns-01")
        self.assertEqual([x["state"] for x in r["names"]], ["ok", "missing"])
        self.assertFalse(r["ok"])


# ============================================================================================================================
# renewal
# ============================================================================================================================

class RenewalTests(Base):
    def issue_with_days(self, name, days):
        self.fx.fb.flag("days", str(days))
        self.fx.issue([name])
        self.fx.fb.unflag("days")

    def test_threshold_logic(self):
        fx = self.fx
        self.issue_with_days("far.example.org", 80)
        self.issue_with_days("near.example.org", 29)
        self.issue_with_days("edge.example.org", 32)
        c = fx.edge.certs
        self.assertFalse(c.due("far.example.org")[0])
        self.assertTrue(c.due("near.example.org")[0])
        self.assertFalse(c.due("edge.example.org")[0])
        self.assertTrue(c.due("far.example.org", force=True)[0])
        fx.clock.t += 3 * 86400                                                                    # the 32-day certificate is now inside the window
        self.assertTrue(c.due("edge.example.org")[0])

    def test_renew_all_renews_only_what_is_due_and_records_the_run(self):
        fx = self.fx
        self.issue_with_days("far.example.org", 80)
        self.issue_with_days("near.example.org", 10)
        fx.fb.clear()
        fx.fb.flag("days", "90")
        job = fx.edge.certs.renew_all("timer", "-", background=False)
        self.assertTrue(job.ok, job.error)
        argvs = fx.fb.argvs("lego")
        self.assertEqual(len(argvs), 1)
        a = argvs[0]
        self.assertEqual(a[-3:], ["renew", "--days", "30"])                                         # lego renews: files exist, so `renew`, not `run`
        self.assertIn("near.example.org", a)
        self.assertGreater(fx.edge.certs.store.meta("near.example.org")["expires"], fx.clock.now() + 80 * 86400)
        self.assertIsNotNone(fx.edge.certs.store.meta("near.example.org")["renewed"])
        rec = jload(fx.p("/var/lib/synofurb-mgmt/edge/renew.json"))
        self.assertEqual((rec["ok"], rec["renewed"], rec["failed"]), (True, 1, 0))
        self.assertEqual({r["id"]: r["result"] for r in rec["results"]}, {"far.example.org": "skipped", "near.example.org": "renewed"})

    def test_a_failed_renewal_is_recorded_and_not_retried_at_once(self):
        fx = self.fx
        self.issue_with_days("near.example.org", 10)
        fx.fb.flag("lego.fail", "Timeout during connect\n")
        job = fx.edge.certs.renew_all("timer", "-", background=False)
        self.assertFalse(job.ok)
        self.assertIn("1 of 1 renewals failed", job.error)
        meta = fx.edge.certs.store.meta("near.example.org")
        self.assertTrue(meta["last_error"])
        self.assertEqual(fx.edge.certs.view("near.example.org")["status"], "failed")
        fx.fb.clear()
        job = fx.edge.certs.renew_all("timer", "-", background=False)                               # six hours have not passed
        self.assertTrue(job.ok)
        self.assertEqual(fx.fb.argvs("lego"), [])
        fx.clock.t += 7 * 3600
        job = fx.edge.certs.renew_all("timer", "-", background=False)
        self.assertEqual(len(fx.fb.argvs("lego")), 1)
        self.assertFalse(job.ok)
        fx.fb.unflag("lego.fail")
        job = fx.edge.certs.renew_all("timer", "-", force=True, background=False)                    # force ignores the backoff
        self.assertTrue(job.ok, job.error)
        self.assertEqual(fx.edge.certs.store.meta("near.example.org")["last_error"], "")

    def test_renew_now_forces_it(self):
        fx = self.fx
        self.issue_with_days("far.example.org", 80)
        job = fx.edge.certs.renew("far.example.org", "tester", "-", background=False)
        self.assertTrue(job.ok, job.error)
        self.assertEqual(fx.fb.argvs("lego")[-1][-3:], ["renew", "--days", "36500"])

    def test_haproxy_is_reloaded_only_when_a_used_certificate_changed(self):
        fx = self.fx
        self.issue_with_days("used.example.org", 10)
        self.issue_with_days("unused.example.org", 10)
        st = fx.edge.proxy.state()
        st["enabled"], st["mgmt"] = True, {"hostname": "used.example.org", "cert": "used.example.org"}
        fx.edge.proxy.save_state(st)
        fx.fb.flag("active.haproxy")
        E.write_file(fx.p("/etc/haproxy/haproxy.cfg"), "# current\n", 0o644)
        fx.fb.clear()
        job = fx.edge.certs.renew_all("timer", "-", background=False, only="unused.example.org", force=True)
        self.assertTrue(job.ok)
        self.assertEqual(fx.fb.argvs("haproxy"), [])
        self.assertNotIn(["reload", "haproxy"], fx.fb.argvs("systemctl"))
        job = fx.edge.certs.renew_all("timer", "-", background=False, only="used.example.org", force=True)
        self.assertTrue(job.ok)
        self.assertEqual(fx.fb.argvs("haproxy")[-1], ["-c", "-f", fx.p("/etc/haproxy/haproxy.cfg")])
        self.assertIn(["reload", "haproxy"], fx.fb.argvs("systemctl"))

    def test_a_bad_reload_keeps_the_old_certificate_file(self):
        fx = self.fx
        self.issue_with_days("used.example.org", 10)
        st = fx.edge.proxy.state()
        st["enabled"], st["mgmt"] = True, {"hostname": "used.example.org", "cert": "used.example.org"}
        fx.edge.proxy.save_state(st)
        fx.fb.flag("active.haproxy")
        E.write_file(fx.p("/etc/haproxy/haproxy.cfg"), "# current\n", 0o644)
        old = fx.read("/etc/haproxy/certs/used.example.org.pem")
        fx.fb.flag("haproxy.invalid")
        job = fx.edge.certs.renew_all("timer", "-", background=False, force=True)
        self.assertEqual(fx.read("/etc/haproxy/certs/used.example.org.pem"), old)
        self.assertTrue(any("kept the old certificate" in w for w in job.warnings))
        self.assertNotIn(["reload", "haproxy"], fx.fb.argvs("systemctl"))

    def test_timer_units(self):
        service, timer = E.renewal_units("/opt/synofurb-mgmt/synofurb_mgmt.py")
        self.assertIn("ExecStart=/usr/bin/python3 /opt/synofurb-mgmt/synofurb_mgmt.py certs renew\n", service)
        self.assertIn("Type=oneshot", service)
        self.assertIn("OnCalendar=*-*-* 03,15:00:00", timer)                                      # twice a day
        self.assertRegex(timer, r"RandomizedDelaySec=\d+h")
        self.assertIn("Persistent=true", timer)
        self.assertIn("WantedBy=timers.target", timer)
        with self.assertRaises(E.EdgeError):
            E.renewal_units("/opt/x; rm -rf /")

    def test_timer_is_installed_once(self):
        fx = self.fx
        fx.issue(["a.example.org"])
        n = len([a for a in fx.fb.argvs("systemctl") if a[0] == "enable"])
        fx.fb.flag("enabled.synofurb-certs-renew.timer")
        fx.issue(["b.example.org"])
        self.assertEqual(len([a for a in fx.fb.argvs("systemctl") if a[0] == "enable"]), n)
        self.assertEqual(fx.edge.certs.timer_state(), {"installed": True, "enabled": True})


class HealthTests(Base):
    def test_absent_without_certificates(self):
        fx = Fx()
        self.addCleanup(fx.close)
        self.assertEqual(E.health_section(fx.env), (None, [], []))

    def test_thresholds(self):
        fx = self.fx
        for name, days in (("ok.example.org", 60), ("soon.example.org", 13), ("crit.example.org", 2)):
            fx.fb.flag("days", str(days))
            fx.issue([name])
        sect, warn, crit = E.health_section(fx.env)
        self.assertEqual(sect["count"], 3)
        self.assertEqual(sect["min_days"], 1)                                                       # 2 days minus the seconds since it was issued
        self.assertEqual(sorted(x["id"] for x in sect["expiring"]), ["crit.example.org", "soon.example.org"])
        self.assertEqual(warn, ["certificate soon.example.org expires in 12 days"])
        self.assertEqual(crit, ["certificate crit.example.org expires in 1 days"])
        fx.clock.t += 5 * 86400
        self.assertIn("certificate crit.example.org expired", E.health_section(fx.env)[2])

    def test_failed_renewal_and_failed_issue(self):
        fx = self.fx
        fx.fb.flag("days", "20")
        fx.issue(["a.example.org"])
        meta = fx.edge.certs.store.meta("a.example.org")
        meta["last_error"] = "boom"
        fx.edge.certs.store.save_meta("a.example.org", meta)
        fx.fb.flag("lego.fail", "x\n")
        fx.issue(["b.example.org"])
        sect, warn, crit = E.health_section(fx.env)
        self.assertEqual(sorted(sect["failed"]), ["a.example.org", "b.example.org"])
        self.assertIn("certificate a.example.org: renewal failed", warn)
        self.assertIn("certificate b.example.org: issue failed", warn)

    def test_health_json_keeps_its_schema_and_gains_a_section(self):
        fx = self.fx
        fx.fb.flag("days", "2")
        fx.issue(["a.example.org"])
        cfg = m.Config(root=fx.tmp, dry_run=True, tls=False)
        app = m.App(cfg, m.DryRunRunner())
        app._edge = fx.edge
        doc = m.collect_health(app, quick=True, budget_s=5)
        self.assertEqual(doc["schema"], 1)
        self.assertEqual(doc["certs"]["count"], 1)
        self.assertEqual(doc["overall"], "crit")
        self.assertIn("certificate a.example.org expires in 1 days", doc["warnings"])
        json.dumps(doc)
        for key in ("version", "hostname", "arrays", "disks", "volumes", "services", "overall", "warnings"):
            self.assertIn(key, doc)
        app2 = m.App(m.Config(root=tempfile.mkdtemp(), dry_run=True, tls=False), m.DryRunRunner())
        self.assertNotIn("certs", m.collect_health(app2, quick=True, budget_s=5))                  # no certificates: the old document, byte for byte in shape


class StoreSecurityTests(Base):
    def test_store_refuses_traversal_and_links(self):
        fx = self.fx
        store = fx.edge.certs.store
        for bad in ("../x", "a/b", "/etc", ".x.example.org"):
            with self.assertRaises(E.EdgeError):
                store.dir(bad)
        fx.issue(["example.org"])
        os.symlink("/tmp", fx.p("/etc/synofurb/certs/evil.example.org"))
        self.assertEqual(store.ids(), ["example.org"])                                              # a link is never listed

    def test_module_has_no_shell(self):
        src = open(os.path.join(ROOT, "synofurb_edge.py"), encoding="utf-8").read()
        code = "\n".join(ln for ln in src.splitlines() if not ln.lstrip().startswith("#"))
        for needle in ("shell=True", "os.system", "os.popen", "commands.getoutput"):
            self.assertNotIn(needle, code.replace("shell=False", ""), needle)
        self.assertEqual(code.count("subprocess.Popen"), 1)                                         # one place starts processes
        self.assertIn("shell=False", code)
        self.assertNotIn("import synofurb_mgmt", src)


# ============================================================================================================================
# the HAProxy front: rendering
# ============================================================================================================================

PATHS = {"cfg": "/etc/haproxy/haproxy.cfg", "crt_list": "/etc/haproxy/synofurb-crt-list.txt", "pem_dir": "/etc/haproxy/certs", "default_pem": "/etc/haproxy/certs/_default.pem"}
FULL = {"mgmt_port": 8443, "mgmt": {"hostname": "nas.example.org", "cert": "_wildcard.example.org"},
        "sites": [{"id": "plex", "hostname": "plex.other.net", "cert": "plex.other.net", "port": 32400, "scheme": "http"},
                  {"id": "frigate", "hostname": "frigate.example.org", "cert": "_wildcard.example.org", "port": 8971, "scheme": "https"}],
        "certs": {"plex.other.net": ["plex.other.net"], "_wildcard.example.org": ["nas.example.org", "frigate.example.org"]}}
MINIMAL = {"mgmt_port": 8443, "mgmt": {"hostname": "", "cert": ""}, "sites": [], "certs": {}}


class RenderTests(unittest.TestCase):
    def golden(self, name):
        return slurp(os.path.join(HERE, "golden", name))

    def test_golden_full(self):
        cfg, crt = E.render_haproxy(FULL, PATHS)
        self.assertEqual(cfg, self.golden("haproxy-full.cfg"))
        self.assertEqual(crt, self.golden("haproxy-full.crt-list"))

    def test_golden_minimal(self):
        cfg, crt = E.render_haproxy(MINIMAL, PATHS)
        self.assertEqual(cfg, self.golden("haproxy-minimal.cfg"))
        self.assertEqual(crt, self.golden("haproxy-minimal.crt-list"))

    def test_sni_and_alpn_routing_text(self):
        cfg, crt = E.render_haproxy(FULL, PATHS)
        self.assertIn("use_backend be_acme_alpn if { req.ssl_alpn acme-tls/1 }", cfg)                  # TLS-ALPN-01 validation goes to lego
        self.assertIn("tcp-request content accept if { req.ssl_hello_type 1 }", cfg)
        self.assertIn("use_backend be_acme_http if acme_challenge", cfg)                                  # HTTP-01
        self.assertIn("http-request redirect scheme https code 301 unless acme_challenge", cfg)
        self.assertIn("bind 127.0.0.1:8444 accept-proxy ssl crt-list /etc/haproxy/synofurb-crt-list.txt alpn h2,http/1.1", cfg)
        self.assertIn("use_backend be_addon_frigate if { req.hdr(host),field(1,:),lower -m str frigate.example.org }", cfg)
        self.assertIn("server frigate 127.0.0.1:8971 ssl verify none", cfg)                               # https backend: re-encrypted on loopback
        self.assertIn("server plex 127.0.0.1:32400\n", cfg)                                               # http backend: plain
        self.assertIn("default_backend be_mgmt", cfg)                                                     # never locked out: anything else is the management plane
        self.assertLess(cfg.index("be_acme_alpn if"), cfg.index("default_backend be_tls_terminator"))
        lines = crt.splitlines()
        self.assertEqual(lines[2], "/etc/haproxy/certs/_default.pem")                                     # the first certificate is the default
        self.assertIn("/etc/haproxy/certs/_wildcard.example.org.pem frigate.example.org nas.example.org", lines)   # the SNI names of a certificate: only the assigned hostnames

    def test_the_management_plane_listens_where_it_was_told(self):
        spec = dict(MINIMAL, mgmt_port=9443)
        self.assertIn("server mgmt 127.0.0.1:9443 ssl verify none", E.render_haproxy(spec, PATHS)[0])

    def test_order_does_not_change_the_output(self):
        a = E.render_haproxy(FULL, PATHS)
        spec = dict(FULL, sites=list(reversed(FULL["sites"])))
        self.assertEqual(a, E.render_haproxy(spec, PATHS))

    def test_template_injection_is_impossible(self):
        def spec(**kw):
            d = json.loads(json.dumps(FULL))
            d.update(kw)
            return d
        evil_names = ["a.example.org\n    bind 0.0.0.0:22", "a.example.org }\n", "a b.example.org", "a.example.org;x", "a.example.org # c", "{ x }.example.org", "*.example.org", "a.example.org\r\nacl"]
        for host in evil_names:
            for where in ("mgmt", "site", "cert"):
                d = spec()
                if where == "mgmt":
                    d["mgmt"] = {"hostname": host, "cert": "_wildcard.example.org"}
                elif where == "site":
                    d["sites"][0]["hostname"] = host
                else:
                    d["certs"]["_wildcard.example.org"] = [host]
                with self.assertRaises(E.EdgeError, msg=(where, host)):
                    E.render_haproxy(d, PATHS)
        for site in ({"id": "x\nbind :22", "hostname": "x.example.org", "cert": "c.example.org", "port": 80, "scheme": "http"},
                     {"id": "ok", "hostname": "x.example.org", "cert": "c.example.org", "port": "80\nbind", "scheme": "http"},
                     {"id": "ok", "hostname": "x.example.org", "cert": "c.example.org", "port": 70000, "scheme": "http"},
                     {"id": "ok", "hostname": "x.example.org", "cert": "c.example.org", "port": 80, "scheme": "http\n"}):
            with self.assertRaises((E.EdgeError, ValueError), msg=site):
                E.render_haproxy(spec(sites=[site]), PATHS)
        for cid in ("../x", "a b", "x\n"):
            with self.assertRaises(E.EdgeError):
                E.render_haproxy(spec(certs={cid: ["a.example.org"]}), PATHS)
        with self.assertRaises(E.EdgeError):
            E.render_haproxy(dict(FULL, mgmt_port=0), PATHS)
        with self.assertRaises(E.EdgeError):
            E.render_haproxy(FULL, dict(PATHS, crt_list="/etc/haproxy/x\nbind"))
        d = spec()
        d["sites"].append(dict(d["sites"][0], id="other"))                                                # the same hostname twice
        with self.assertRaises(E.EdgeError):
            E.render_haproxy(d, PATHS)

    def test_no_dollar_or_format_leaks(self):
        cfg, crt = E.render_haproxy(FULL, PATHS)
        for text in (cfg, crt):
            self.assertNotIn("{}", text)
            self.assertNotRegex(text, r"%\((?!src)")
            self.assertNotIn("None", text)


# ============================================================================================================================
# the HAProxy front: hostnames, apply, confirm, revert, disable
# ============================================================================================================================

SITES = [{"id": "frigate", "name": "Frigate NVR", "icon": "camera", "port": 8971, "scheme": "https", "path": "/", "hint": "frigate"},
         {"id": "plex", "name": "Plex Media Server", "icon": "package", "port": 32400, "scheme": "http", "path": "/web", "hint": "plex"}]
BODY = {"mgmt": {"hostname": "nas.example.org", "cert": "_wildcard.example.org"}, "sites": {"frigate": {"hostname": "frigate.example.org", "cert": "_wildcard.example.org"}}}
MGMT_JSON = {"update_pubkey": "RWTestKey", "idle_timeout": 1800}


@unittest.skipUnless(HAVE_OPENSSL, "openssl missing")
class ProxyBase(unittest.TestCase):
    def setUp(self):
        self.fx = Fx(sites=[dict(x) for x in SITES])
        self.addCleanup(self.fx.close)
        fx = self.fx
        fx.configure()
        fx.issue(["*.example.org", "example.org"])
        E.write_json(fx.p("/etc/synofurb/mgmt.json"), MGMT_JSON, 0o644)
        self.px = fx.edge.proxy

    def enable(self, body=None, **kw):
        return self.px.apply("enable", BODY if body is None else body, "tester", "10.0.0.1", background=False, **kw)

    def mgmt_json(self):
        return jload(self.fx.p("/etc/synofurb/mgmt.json"))

    def systemctl(self):
        return self.fx.fb.argvs("systemctl")

    def haproxy_ctl(self):
        """systemctl calls that start, reload or enable haproxy (not the renewal timer)."""
        return [a for a in self.systemctl() if a[0] in ("reload", "restart", "enable") and a[-1] == "haproxy"]


class HostnameTests(ProxyBase):
    def test_valid_assignment_and_rows(self):
        st = self.px.normalize(BODY)
        self.assertEqual(st["mgmt"], {"hostname": "nas.example.org", "cert": "_wildcard.example.org"})
        self.assertEqual(st["sites"]["frigate"], {"hostname": "frigate.example.org", "cert": "_wildcard.example.org", "scheme": ""})
        rows = self.px.rows(st)
        self.assertEqual([r["id"] for r in rows], ["mgmt", "frigate", "plex"])
        self.assertEqual(rows[1]["scheme"], "https")                                                      # the manifest's web scheme is the default backend scheme
        self.assertEqual(rows[2]["hint"], "plex")

    def test_refusals(self):
        n = self.px.normalize
        for body, want in (({"mgmt": {"hostname": "nas.example.org", "cert": ""}}, "choose a certificate"),
                           ({"mgmt": {"hostname": "other.net", "cert": "_wildcard.example.org"}}, "does not cover"),
                           ({"mgmt": {"hostname": "a.b.example.org", "cert": "_wildcard.example.org"}}, "does not cover"),          # a wildcard covers one level
                           ({"mgmt": {"hostname": "nas.example.org", "cert": "nope.example.org"}}, "not issued"),
                           ({"sites": {"ghost": {"hostname": "g.example.org", "cert": "_wildcard.example.org"}}}, "no web page"),
                           ({"sites": {"frigate": {"hostname": "x.example.org", "cert": "_wildcard.example.org", "scheme": "ftp"}}}, "scheme"),
                           ({"mgmt": {"hostname": "nas.example.org\nbind :22", "cert": "_wildcard.example.org"}}, "not a valid name"),
                           ({"mgmt": {"hostname": "nas.example.org", "cert": "../x"}}, "bad certificate name"),
                           ({"mgmt": {"hostname": "*.example.org", "cert": "_wildcard.example.org"}}, "wildcard"),
                           ({"mgmt": {"hostname": "same.example.org", "cert": "_wildcard.example.org"}, "sites": {"frigate": {"hostname": "same.example.org", "cert": "_wildcard.example.org"}}}, "only once")):
            with self.assertRaises(E.EdgeError, msg=body) as cm:
                n(body)
            self.assertIn(want, str(cm.exception))

    def test_a_failed_issue_cannot_be_assigned(self):
        fx = self.fx
        fx.fb.flag("lego.fail", "boom\n")
        fx.issue(["broken.example.org"])
        with self.assertRaises(E.EdgeError) as cm:
            self.px.normalize({"mgmt": {"hostname": "broken.example.org", "cert": "broken.example.org"}})
        self.assertIn("not issued yet", str(cm.exception))

    def test_save_while_off_persists_only_the_state(self):
        self.px.save_state(self.px.normalize(BODY))
        self.assertEqual(self.px.state()["mgmt"]["hostname"], "nas.example.org")
        self.assertFalse(os.path.exists(self.fx.p("/etc/haproxy/haproxy.cfg")))
        self.assertEqual(mode(self.fx.env.edge_path("proxy.json")), 0o600)

    def test_candidate_check_touches_no_live_file(self):
        fx = self.fx
        res = self.px.check_candidate(self.px.normalize(BODY))
        self.assertTrue(res["ok"])
        self.assertIn("use_backend be_addon_frigate", res["config"])
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/haproxy.cfg")))
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/synofurb-crt-list.txt")))
        self.assertEqual(fx.fb.argvs("systemctl"), [x for x in fx.fb.argvs("systemctl")])
        checked = [a for a in fx.fb.argvs("haproxy") if a[:2] == ["-c", "-f"]]
        self.assertEqual(len(checked), 1)
        self.assertNotEqual(checked[0][2], fx.p("/etc/haproxy/haproxy.cfg"))                          # it validated a private copy
        self.assertEqual(os.listdir(fx.env.state_path("")).count("snap"), 0)
        self.assertEqual([n for n in os.listdir(fx.env.state_path("")) if n.startswith("check-")], [])   # and removed it


class ApplyTests(ProxyBase):
    def test_enable_end_to_end(self):
        fx, px = self.fx, self.px
        fx.fb.clear()
        job = self.enable()
        self.assertTrue(job.ok, job.error)
        cfg = fx.read("/etc/haproxy/haproxy.cfg")
        self.assertTrue(cfg.startswith(E.MARKER))
        self.assertIn("use_backend be_mgmt if { req.hdr(host),field(1,:),lower -m str nas.example.org }", cfg)
        self.assertIn("be_addon_frigate", cfg)
        crt = fx.read("/etc/haproxy/synofurb-crt-list.txt").splitlines()
        self.assertEqual(crt[2], fx.p("/etc/haproxy/certs/_default.pem"))
        self.assertTrue(crt[3].endswith("/_wildcard.example.org.pem frigate.example.org nas.example.org"))
        for name in ("_default.pem", "_wildcard.example.org.pem"):
            self.assertEqual(mode(fx.p("/etc/haproxy/certs/" + name)), 0o600)
        self.assertIn("PRIVATE KEY", fx.read("/etc/haproxy/certs/_default.pem"))
        # validated twice: a private copy first, then the installed file
        checks = [a for a in fx.fb.argvs("haproxy") if a[:2] == ["-c", "-f"]]
        self.assertEqual(len(checks), 2)
        self.assertEqual(checks[1][2], fx.p("/etc/haproxy/haproxy.cfg"))
        # the revert timer is armed BEFORE haproxy is started
        calls = [c[0] + " " + " ".join(c[1:3]) for c in fx.fb.calls()]
        arm = next(i for i, c in enumerate(calls) if c.startswith("systemd-run"))
        start = next(i for i, c in enumerate(calls) if c.startswith("systemctl enable"))
        self.assertLess(arm, start)
        self.assertEqual(fx.fb.argvs("systemd-run")[0], ["--unit=synofurb-edge-revert", "--collect", "--on-active=120s", "/usr/bin/python3", "/opt/synofurb-mgmt/synofurb_mgmt.py", "proxy", "revert", "--auto"])
        self.assertIn(["enable", "--now", "haproxy"], self.systemctl())
        # the management plane moves to loopback, trusts the front, and keeps its other settings
        mj = self.mgmt_json()
        self.assertEqual((mj["bind"], mj["trust_proxy"], mj["update_pubkey"], mj["idle_timeout"]), ("127.0.0.1", True, "RWTestKey", 1800))
        self.assertEqual(len(fx.restarts), 1)
        self.assertTrue(px.state()["enabled"])
        pend = px.pending()
        self.assertEqual(pend["seconds"], 120)
        self.assertIn(pend["seconds_left"], (119, 120))
        self.assertEqual(job.result["urls"][0], "https://nas.example.org/")
        self.assertIn("https://192.168.1.5/", job.result["urls"])
        self.assertTrue(any(a[2] == "certs.job.proxy-enable" for a in fx.audits))

    def test_haproxy_is_installed_on_demand(self):
        fx = self.fx
        os.unlink(os.path.join(fx.fb.bin, "haproxy"))
        shutil.copy(os.path.join(HERE, "..", "tests", "edge_fakes.py"), fx.fb.state + "/unused")
        # a fake package: apt "installs" it by copying the script
        from edge_fakes import FAKE_HAPROXY
        fx.fb.add("haproxy-src", FAKE_HAPROXY)
        shutil.move(os.path.join(fx.fb.bin, "haproxy-src"), os.path.join(fx.fb.state, "pkg", "haproxy"))
        job = self.enable()
        self.assertTrue(job.ok, job.error)
        self.assertTrue(any(a[-3:] == ["install", "--no-install-recommends", "haproxy"] for a in fx.fb.argvs("apt-get")))

    def test_haproxy_rejects_the_candidate_nothing_changes(self):
        fx = self.fx
        E.write_file(fx.p("/etc/haproxy/haproxy.cfg"), "# the previous configuration\n", 0o644)
        fx.fb.flag("haproxy.invalid")
        job = self.enable()
        self.assertFalse(job.ok)
        self.assertIn("HAProxy rejected the configuration", job.error)
        self.assertIn("Nothing was changed", job.error)
        self.assertEqual(fx.read("/etc/haproxy/haproxy.cfg"), "# the previous configuration\n")        # the previous config is kept
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/synofurb-crt-list.txt")))
        self.assertFalse(self.haproxy_ctl())                                                            # never reloaded or started
        self.assertEqual(fx.fb.argvs("systemd-run"), [])                                                # no timer armed
        self.assertEqual(self.mgmt_json(), MGMT_JSON)
        self.assertEqual(fx.restarts, [])
        self.assertFalse(self.px.state()["enabled"])
        self.assertIsNone(self.px.pending())

    def test_a_rejected_installed_file_is_rolled_back_too(self):
        fx = self.fx
        E.write_file(fx.p("/etc/haproxy/haproxy.cfg"), "# previous\n", 0o644)
        fx.fb.add("haproxy", "#!/bin/sh\n[ \"$1\" = -v ] && { echo 'HAProxy version 3.0.11'; exit 0; }\ncase \"$3\" in /etc/*|*/etc/haproxy/*) echo '[ALERT] x' >&2; exit 1;; esac\nexit 0\n")
        job = self.enable()
        self.assertFalse(job.ok)
        self.assertIn("rejected the installed configuration", job.error)
        self.assertEqual(fx.read("/etc/haproxy/haproxy.cfg"), "# previous\n")
        self.assertFalse(self.haproxy_ctl())

    def test_reload_failure_restores_everything(self):
        fx = self.fx
        E.write_file(fx.p("/etc/haproxy/haproxy.cfg"), "# running\n", 0o644)
        E.write_file(fx.p("/etc/haproxy/certs/old.pem"), "OLD PEM\n", 0o600)
        fx.fb.flag("active.haproxy")
        fx.fb.flag("enabled.haproxy")
        fx.fb.flag("reload.fail")
        job = self.px.apply("enable", BODY, "tester", "-", background=False)
        self.assertFalse(job.ok)
        self.assertIn("HAProxy did not start or reload", job.error)
        self.assertIn("previous configuration was restored", job.error)
        self.assertEqual(fx.read("/etc/haproxy/haproxy.cfg"), "# running\n")
        self.assertEqual(fx.read("/etc/haproxy/certs/old.pem"), "OLD PEM\n")
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/certs/_default.pem")))
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/synofurb-crt-list.txt")))
        self.assertIn(["stop", "synofurb-edge-revert.timer", "synofurb-edge-revert.service"], self.systemctl())     # the timer is disarmed
        self.assertIsNone(self.px.pending())
        self.assertEqual(self.mgmt_json(), MGMT_JSON)
        self.assertFalse(self.px.state()["enabled"])
        self.assertFalse(os.path.exists(self.px.snap_dir()))

    def test_first_start_failure_stops_the_service_again(self):
        fx = self.fx
        fx.fb.flag("start.fail")
        job = self.enable()
        self.assertFalse(job.ok)
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/haproxy.cfg")))
        self.assertIn(["disable", "--now", "haproxy"], self.systemctl())
        self.assertEqual(self.mgmt_json(), MGMT_JSON)

    def test_missing_ports_revert(self):
        fx = self.fx
        fx.fb.add("ss", "#!/bin/sh\nexit 0\n")                                                          # haproxy "started" but listens nowhere
        job = self.enable()
        self.assertFalse(job.ok)
        self.assertIn("not listening on port", job.error)
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/haproxy.cfg")))

    def test_an_unarmed_timer_means_nothing_is_applied(self):
        fx = self.fx
        fx.fb.flag("systemd-run.fail")
        job = self.enable()
        self.assertFalse(job.ok)
        self.assertIn("Could not arm the automatic revert", job.error)
        self.assertFalse(self.haproxy_ctl())
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/haproxy.cfg")))
        self.assertEqual(self.mgmt_json(), MGMT_JSON)

    def test_busy_ports_are_refused_before_anything_is_written(self):
        fx = self.fx
        fx.fb.flag("ss.out", "LISTEN 0 4096 0.0.0.0:443 0.0.0.0:*\n")
        job = self.enable()
        self.assertFalse(job.ok)
        self.assertIn("Port 443 is in use", job.error)
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/haproxy.cfg")))
        self.assertEqual(fx.fb.argvs("systemd-run"), [])

    def test_enable_twice_and_apply_while_off_are_refused(self):
        with self.assertRaises(E.EdgeError):
            self.px.apply("apply", {}, background=False)
        with self.assertRaises(E.EdgeError):
            self.px.apply("disable", {}, background=False)
        self.enable()
        with self.assertRaises(E.EdgeError):
            self.enable()

    def test_empty_front_is_allowed_and_keeps_the_default_backend(self):
        job = self.enable(body={})
        self.assertTrue(job.ok, job.error)
        cfg = self.fx.read("/etc/haproxy/haproxy.cfg")
        self.assertIn("default_backend be_mgmt", cfg)
        self.assertNotIn("be_addon_", cfg)


class ConfirmRevertTests(ProxyBase):
    def test_confirm_keeps_it(self):
        fx = self.fx
        self.enable()
        self.px.confirm("tester", "-")
        self.assertIsNone(self.px.pending())
        self.assertFalse(os.path.exists(self.px.snap_dir()))
        self.assertIn(["stop", "synofurb-edge-revert.timer", "synofurb-edge-revert.service"], self.systemctl())
        self.assertTrue(self.px.state()["enabled"])
        self.assertIn("be_mgmt", fx.read("/etc/haproxy/haproxy.cfg"))
        with self.assertRaises(E.EdgeError):
            self.px.confirm()
        with self.assertRaises(E.EdgeError):
            self.px.revert()

    def test_revert_returns_to_the_direct_listener(self):
        fx = self.fx
        self.enable()
        fx.restarts.clear()
        res = self.px.revert(auto=True)
        self.assertTrue(res["ok"], res)
        self.assertEqual(self.mgmt_json(), MGMT_JSON)                                                   # bind and trust_proxy are gone again
        for path in ("/etc/haproxy/haproxy.cfg", "/etc/haproxy/synofurb-crt-list.txt", "/etc/haproxy/certs/_default.pem"):
            self.assertFalse(os.path.exists(fx.p(path)), path)
        self.assertTrue(os.path.exists(fx.p("/etc/haproxy/certs/_wildcard.example.org.pem")))           # the certificate store's own PEM stays
        self.assertIn(["disable", "--now", "haproxy"], self.systemctl())                                # it was off before
        self.assertEqual(len(fx.restarts), 1)                                                           # the web service is restarted to listen directly again
        self.assertIsNone(self.px.pending())
        self.assertFalse(self.px.state()["enabled"])
        rec = jload(fx.p("/var/lib/synofurb-mgmt/edge/reverted.json"))
        self.assertTrue(rec["auto"])
        self.assertTrue(any(a[2] == "proxy.revert" and a[0] == "timer" for a in fx.audits))

    def test_revert_restores_the_previous_bytes_and_reloads_a_running_haproxy(self):
        fx = self.fx
        E.write_file(fx.p("/etc/haproxy/haproxy.cfg"), "# hand written\nglobal\n", 0o644)
        fx.fb.flag("active.haproxy")
        fx.fb.flag("enabled.haproxy")
        self.enable()
        self.assertIn("Managed by synofurb-mgmt", fx.read("/etc/haproxy/haproxy.cfg"))
        fx.fb.clear()
        self.px.revert()
        self.assertEqual(fx.read("/etc/haproxy/haproxy.cfg"), "# hand written\nglobal\n")
        self.assertIn(["reload", "haproxy"], self.systemctl())
        self.assertNotIn(["disable", "--now", "haproxy"], self.systemctl())

    def test_a_configuration_that_was_not_ours_is_kept_once(self):
        fx = self.fx
        E.write_file(fx.p("/etc/haproxy/haproxy.cfg"), "# the administrator's own\nglobal\n", 0o644)
        self.enable()
        orig = fx.env.edge_path("haproxy.cfg.original")
        self.assertEqual(slurp(orig), "# the administrator's own\nglobal\n")
        self.assertEqual(mode(orig), 0o600)
        self.px.confirm()
        self.px.apply("disable", {}, "tester", "-", background=False)
        self.enable()                                                                                     # the file on disk is ours now: the original is not replaced by it
        self.assertEqual(slurp(orig), "# the administrator's own\nglobal\n")

    def test_a_second_apply_keeps_the_first_snapshot(self):
        fx = self.fx
        self.enable()
        job = self.px.apply("apply", {"sites": {"plex": {"hostname": "plex.example.org", "cert": "_wildcard.example.org"}}}, "tester", "-", background=False)
        self.assertTrue(job.ok, job.error)
        self.assertIn("be_addon_plex", fx.read("/etc/haproxy/haproxy.cfg"))
        self.assertEqual(len(fx.restarts), 1)                                                           # applying again does not restart the web service
        self.px.revert()
        self.assertEqual(self.mgmt_json(), MGMT_JSON)
        self.assertFalse(os.path.exists(fx.p("/etc/haproxy/haproxy.cfg")))                              # back to the state before the FIRST apply

    def test_startup_recovery_reverts_an_expired_pending_change(self):
        fx = self.fx
        self.enable()
        self.assertFalse(self.px.startup_recovery())                                                    # the timer still has time
        fx.clock.t += 121
        self.assertTrue(self.px.startup_recovery())
        self.assertEqual(self.mgmt_json(), MGMT_JSON)
        self.assertIsNone(self.px.pending())
        self.assertFalse(self.px.startup_recovery())

    def test_previous_custom_bind_is_remembered(self):
        fx = self.fx
        E.write_json(fx.p("/etc/synofurb/mgmt.json"), dict(MGMT_JSON, bind="192.168.1.5"), 0o644)
        self.enable()
        self.assertEqual(self.mgmt_json()["bind"], "127.0.0.1")
        self.px.confirm()
        job = self.px.apply("disable", {}, "tester", "-", background=False)
        self.assertTrue(job.ok, job.error)
        self.assertEqual(self.mgmt_json(), dict(MGMT_JSON, bind="192.168.1.5"))


class DisableTests(ProxyBase):
    def test_disable_restores_the_direct_listener_and_stops_haproxy_later(self):
        fx = self.fx
        self.enable()
        self.px.confirm()
        fx.restarts.clear()
        job = self.px.apply("disable", {}, "tester", "10.0.0.1", background=False)
        self.assertTrue(job.ok, job.error)
        self.assertEqual(self.mgmt_json(), MGMT_JSON)
        self.assertFalse(self.px.state()["enabled"])
        self.assertEqual(self.px.state()["mgmt"]["hostname"], "nas.example.org")                        # the hostnames are kept for the next enable
        # HAProxy is stopped by a transient unit that outlives the restart of the web service
        self.assertEqual(fx.fb.argvs("systemd-run")[-1], ["--collect", "--on-active=8s", "systemctl", "disable", "--now", "haproxy"])
        self.assertEqual(len(fx.restarts), 1)
        self.assertEqual(job.result["urls"], ["https://192.168.1.5:8443/"])
        self.assertTrue(os.path.exists(fx.p("/etc/synofurb/certs/_wildcard.example.org/fullchain.pem")))     # certificates stay

    def test_the_recovery_command_is_documented_in_the_view(self):
        v = self.px.view()
        self.assertEqual(v["recovery"], "python3 /opt/synofurb-mgmt/synofurb_mgmt.py proxy disable")
        self.assertEqual(v["mgmt_port"], 8443)

    def test_disable_from_the_cli_runs_immediately(self):
        fx = self.fx
        self.enable()
        self.px.confirm()
        out, err = io.StringIO(), io.StringIO()
        rc = E.cli_main(["disable"], fx.edge, "proxy", out=lambda s: out.write(s + "\n"), err=lambda s: err.write(s + "\n"))
        self.assertEqual(rc, 0, err.getvalue())
        self.assertEqual(self.mgmt_json(), MGMT_JSON)
        self.assertIn("ok", out.getvalue())


class TrustProxyTests(unittest.TestCase):
    def test_forwarded_address(self):
        f = m.forwarded_ip
        self.assertEqual(f("127.0.0.1", "203.0.113.7", True), "203.0.113.7")
        self.assertEqual(f("::1", "2001:db8::1", True), "2001:db8::1")
        self.assertEqual(f("127.0.0.1", "1.1.1.1, 203.0.113.7", True), "203.0.113.7")                   # the last hop is the one HAProxy wrote
        self.assertEqual(f("127.0.0.1", "203.0.113.7", False), "127.0.0.1")                              # not trusted unless mgmt.json says so
        self.assertEqual(f("192.168.1.9", "203.0.113.7", True), "192.168.1.9")                           # only a loopback peer is believed
        for bad in ("evil", "1.2.3.4; drop", "", "999.1.1.1", "a, b"):
            self.assertEqual(f("127.0.0.1", bad, True), "127.0.0.1")
        self.assertEqual(f("127.0.0.1", None, True), "127.0.0.1")

    def test_config_reads_trust_proxy_from_the_file_only(self):
        with tempfile.TemporaryDirectory() as d:
            cfg = m.Config(root=d)
            self.assertFalse(cfg.trust_proxy)
            E.write_json(os.path.join(d, "etc/synofurb/mgmt.json"), {"trust_proxy": True, "bind": "127.0.0.1"}, 0o644)
            cfg.load_file()
            self.assertTrue(cfg.trust_proxy)
            self.assertEqual(cfg.bind, "127.0.0.1")
            cfg2 = m.Config(root=d)
            E.write_json(os.path.join(d, "etc/synofurb/mgmt.json"), {"trust_proxy": "yes"}, 0o644)
            cfg2.load_file()
            self.assertFalse(cfg2.trust_proxy)


# ============================================================================================================================
# REST routes and the command line
# ============================================================================================================================

@unittest.skipUnless(HAVE_OPENSSL, "openssl missing")
class ApiTests(unittest.TestCase):
    def setUp(self):
        self.fx = Fx(sites=[dict(x) for x in SITES])
        self.addCleanup(self.fx.close)
        self.env = Env()
        self.addCleanup(self.env.close)
        self.env.app._edge = self.fx.edge
        self.c = Client(self.env)

    def login(self):
        self.assertEqual(self.c.login()[0], 200)

    def wait(self, job_id, timeout=30):
        import time
        end = time.time() + timeout
        while time.time() < end:
            st, obj, _ = self.c.call("GET", "/api/certs/job/" + job_id)
            self.assertEqual(st, 200)
            if obj["job"]["state"] != "running":
                return obj["job"]
            time.sleep(0.05)
        self.fail("job did not end")

    def test_everything_needs_a_session(self):
        for method, path in (("GET", "/api/certs"), ("GET", "/api/proxy"), ("POST", "/api/certs/issue"), ("POST", "/api/certs/settings"), ("POST", "/api/proxy/enable"),
                             ("POST", "/api/certs/example.org/remove"), ("GET", "/api/certs/example.org"), ("POST", "/api/proxy/confirm"), ("GET", "/api/certs/job/e0123456789ab")):
            st, obj, _ = self.c.call(method, path, {} if method == "POST" else None)
            self.assertEqual(st, 401, (method, path))

    def test_csrf_is_required_for_changes(self):
        self.login()
        st, obj, _ = self.c.call("POST", "/api/certs/settings", {"email": "a@example.org"}, csrf=False)
        self.assertEqual(st, 403)
        st, obj, _ = self.c.call("POST", "/api/proxy/enable", {}, csrf=False)
        self.assertEqual(st, 403)
        st, obj, _ = self.c.call("POST", "/api/certs/issue", {"names": ["a.example.org"]}, headers={"Origin": "https://evil.example"})
        self.assertEqual(st, 403)

    def test_settings_issue_list_show_renew_remove(self):
        self.login()
        st, obj, _ = self.c.call("POST", "/api/certs/settings", {"email": "admin@example.org", "accept_tos": True, "generate_secret": True, "environment": "staging"})
        self.assertEqual(st, 200, obj)
        secret = obj["secret_once"]
        self.fx.cname_ok("example.org")
        st, obj, _ = self.c.call("POST", "/api/certs/issue", {"names": ["example.org"], "challenge": "dns-01"})
        self.assertEqual(st, 202, obj)
        job = self.wait(obj["job"]["id"])
        self.assertTrue(job["ok"], job)
        st, obj, _ = self.c.call("GET", "/api/certs")
        self.assertEqual(st, 200)
        self.assertEqual([c["id"] for c in obj["certs"]], ["example.org"])
        self.assertEqual(obj["certs"][0]["status"], "valid")
        self.assertEqual(obj["certs"][0]["cnames"][0]["target"], E.cname_target(secret))
        self.assertNotIn(secret, json.dumps(obj))
        st, obj, _ = self.c.call("GET", "/api/certs/example.org")
        self.assertEqual((st, obj["cert"]["primary"]), (200, "example.org"))
        st, obj, _ = self.c.call("POST", "/api/certs/example.org/renew", {})
        self.assertEqual(st, 202)
        self.assertTrue(self.wait(obj["job"]["id"])["ok"])
        st, obj, _ = self.c.call("POST", "/api/certs/example.org/remove", {"confirm": "wrong"})
        self.assertEqual(st, 400)
        st, obj, _ = self.c.call("POST", "/api/certs/example.org/remove", {"confirm": "example.org"})
        self.assertEqual((st, obj["ok"]), (200, True))
        # the audit log has the facts and never the secret
        audit = slurp(self.env.cfg.p(self.env.cfg.audit_path))
        self.assertIn("/api/certs/issue", audit)
        self.assertIn("/api/certs/example.org/remove", audit)
        self.assertNotIn(secret, audit)
        self.assertNotIn(secret, json.dumps(self.fx.audits))

    def test_errors_are_400_404_409_and_hostile_ids_never_match(self):
        self.login()
        st, obj, _ = self.c.call("POST", "/api/certs/issue", {"names": ["a.example.org"]})
        self.assertEqual(st, 400)
        self.assertIn("accept the Let's Encrypt terms", obj["error"])
        self.fx.configure()
        st, obj, _ = self.c.call("POST", "/api/certs/issue", {"names": ["bad name"], "challenge": "dns-01"})
        self.assertEqual(st, 400)
        self.assertEqual(self.c.call("GET", "/api/certs/nothing.example.org")[0], 404)
        self.assertEqual(self.c.call("POST", "/api/certs/nothing.example.org/renew", {})[0], 404)
        for path in ("/api/certs/..%2Fetc", "/api/certs/a/b", "/api/certs/.hidden", "/api/certs/job/xyz"):
            self.assertIn(self.c.call("GET", path)[0], (404, 405), path)

        class Held:
            def acquire(self, *a):
                return None

            def holder(self):
                return {"kind": "addon", "label": "install plex"}

            def release(self, t):
                pass
        self.fx.env.job_lock = Held()
        self.fx.cname_ok("a.example.org")
        st, obj, _ = self.c.call("POST", "/api/certs/issue", {"names": ["a.example.org"], "challenge": "dns-01"})
        self.assertEqual(st, 409)
        self.assertIn("Another job is running (addon install plex)", obj["error"])

    def test_read_only_checks(self):
        self.login()
        self.fx.configure()
        self.fx.cname_ok("a.example.org")
        st, obj, _ = self.c.call("POST", "/api/certs/check-dns", {"name": "a.example.org"})
        self.assertEqual((st, obj["state"]), (200, "ok"))
        st, obj, _ = self.c.call("POST", "/api/certs/check-dns", {"name": "a b"})
        self.assertEqual(st, 400)
        self.fx.hosts = {"a.example.org": ["192.168.1.5"]}
        st, obj, _ = self.c.call("POST", "/api/certs/precheck", {"names": ["a.example.org"], "challenge": "http-01"})
        self.assertEqual((st, obj["names"][0]["local"], obj["ports"][0]["port"]), (200, True, 80))
        st, obj, _ = self.c.call("POST", "/api/certs/precheck", {"names": ["*.example.org"], "challenge": "http-01"})
        self.assertEqual(st, 400)

    def test_proxy_flow(self):
        self.login()
        fx = self.fx
        fx.configure()
        fx.issue(["*.example.org", "example.org"])
        st, obj, _ = self.c.call("GET", "/api/proxy")
        self.assertEqual(st, 200)
        self.assertFalse(obj["enabled"])
        self.assertEqual([r["id"] for r in obj["rows"]], ["mgmt", "frigate", "plex"])
        self.assertEqual([c["id"] for c in obj["certs"]], ["_wildcard.example.org"])
        st, obj, _ = self.c.call("POST", "/api/proxy/test", BODY)
        self.assertEqual((st, obj["ok_check"]), (200, True))
        self.assertIn("be_addon_frigate", obj["config"])
        st, obj, _ = self.c.call("POST", "/api/proxy/enable", {"mgmt": {"hostname": "nas.example.org", "cert": "nope"}})
        self.assertEqual(st, 400)
        E.write_json(fx.p("/etc/synofurb/mgmt.json"), MGMT_JSON, 0o644)
        st, obj, _ = self.c.call("POST", "/api/proxy/enable", BODY)
        self.assertEqual(st, 202, obj)
        job = self.wait(obj["job"]["id"])
        self.assertTrue(job["ok"], job)
        st, me, _ = self.c.call("GET", "/api/me")
        self.assertEqual(me["edge_pending"]["seconds"], 120)                                              # the banner on every page
        self.assertEqual(self.c.call("POST", "/api/proxy/hostnames", BODY)[0], 400)                       # on: must use Apply
        st, obj, _ = self.c.call("POST", "/api/proxy/confirm", {})
        self.assertEqual((st, obj["ok"]), (200, True))
        self.assertIsNone(self.c.call("GET", "/api/me")[1]["edge_pending"])
        st, obj, _ = self.c.call("POST", "/api/proxy/revert", {})
        self.assertEqual(st, 400)
        st, obj, _ = self.c.call("POST", "/api/proxy/disable", {})
        self.assertEqual(st, 202)
        self.assertTrue(self.wait(obj["job"]["id"])["ok"])
        self.assertFalse(self.c.call("GET", "/api/proxy")[1]["enabled"])

    def test_client_address_behind_the_front(self):
        self.env.cfg.trust_proxy = True
        self.login()
        st, obj, _ = self.c.call("GET", "/api/certs", headers={"X-Forwarded-For": "203.0.113.7"})
        st, obj, _ = self.c.call("POST", "/api/certs/settings", {"email": "x@example.org"}, headers={"X-Forwarded-For": "203.0.113.7"})
        self.assertEqual(st, 200)
        last = [json.loads(x) for x in slurp(self.env.cfg.p(self.env.cfg.audit_path)).splitlines()][-1]
        self.assertEqual((last["ip"], last["action"]), ("203.0.113.7", "/api/certs/settings"))
        self.env.cfg.trust_proxy = False
        self.c.call("POST", "/api/certs/settings", {"email": "y@example.org"}, headers={"X-Forwarded-For": "203.0.113.7"})
        last = [json.loads(x) for x in slurp(self.env.cfg.p(self.env.cfg.audit_path)).splitlines()][-1]
        self.assertEqual(last["ip"], "127.0.0.1")


@unittest.skipUnless(HAVE_OPENSSL, "openssl missing")
class CliTests(unittest.TestCase):
    def setUp(self):
        self.fx = Fx(sites=[dict(x) for x in SITES])
        self.addCleanup(self.fx.close)

    def run_cli(self, which, *argv, stdin=None):
        out, err = [], []
        rc = E.cli_main(list(argv), self.fx.edge, which, out=out.append, err=err.append, stdin=io.StringIO(stdin) if stdin is not None else None)
        return rc, "\n".join(out), "\n".join(err)

    def test_settings_issue_list_show_renew_remove(self):
        fx = self.fx
        rc, out, err = self.run_cli("certs", "settings", "--email", "admin@example.org", "--accept-tos", "--environment", "staging", "--generate-secret")
        self.assertEqual(rc, 0, err)
        secret = re.search(r"NEW SECRET \(shown only now, keep it\): (\S+)", out).group(1)
        self.assertIn("CNAME target for every name: " + E.cname_target(secret), out)
        rc, out, err = self.run_cli("certs", "settings")
        self.assertNotIn(secret, out)                                                                   # shown once
        self.assertIn("challenge secret: set", out)
        rc, out, err = self.run_cli("certs", "check-dns", "example.org")
        self.assertEqual(rc, 1)
        self.assertIn("expected CNAME -> " + E.cname_target(secret), out)
        fx.cname_ok("example.org")
        self.assertEqual(self.run_cli("certs", "check-dns", "example.org")[0], 0)
        rc, out, err = self.run_cli("certs", "issue", "example.org", "--challenge", "dns-01")
        self.assertEqual(rc, 0, err + out)
        self.assertIn("== Asking Let's Encrypt (staging, dns-01)", out)
        self.assertNotIn(secret, out)
        rc, out, err = self.run_cli("certs", "list")
        self.assertIn("example.org", out)
        rc, out, err = self.run_cli("certs", "list", "--json")
        self.assertEqual(json.loads(out)["certs"][0]["id"], "example.org")
        rc, out, err = self.run_cli("certs", "show", "example.org")
        self.assertIn("CNAME _acme-challenge.example.org -> " + E.cname_target(secret), out)
        self.assertEqual(self.run_cli("certs", "renew")[0], 0)                                           # nothing is due: fine
        self.assertEqual(len(fx.fb.argvs("lego")), 1)
        rc, out, err = self.run_cli("certs", "renew", "--id", "example.org", "--force")
        self.assertEqual(rc, 0, err)
        self.assertEqual(len(fx.fb.argvs("lego")), 2)
        rc, out, err = self.run_cli("certs", "remove", "example.org")
        self.assertEqual(rc, 2)
        self.assertIn("type the certificate name", err)
        self.assertEqual(self.run_cli("certs", "remove", "example.org", "--confirm", "example.org")[0], 0)

    def test_secret_from_stdin_and_exit_codes(self):
        rc, out, err = self.run_cli("certs", "settings", "--secret-stdin", stdin=SECRET + "\n")
        self.assertEqual(rc, 0, err)
        self.assertEqual(E.load_secret(self.fx.env), SECRET)
        self.assertEqual(self.run_cli("certs", "settings", "--email", "nope")[0], 2)
        self.assertEqual(self.run_cli("certs", "show", "nothing.example.org")[0], 2)
        self.assertEqual(self.run_cli("certs", "bogus")[0], 2)
        self.assertEqual(self.run_cli("proxy", "enable", "--seconds", "x")[0], 2)

    def test_busy_is_exit_1(self):
        fx = self.fx
        fx.configure()
        fx.cname_ok("example.org")

        class Held:
            def acquire(self, *a):
                return None

            def holder(self):
                return {"kind": "update", "label": "x"}
        fx.env.job_lock = Held()
        rc, out, err = self.run_cli("certs", "issue", "example.org")
        self.assertEqual(rc, 1)
        self.assertIn("busy", err)

    def test_proxy_commands(self):
        fx = self.fx
        fx.configure()
        fx.issue(["*.example.org", "example.org"])
        E.write_json(fx.p("/etc/synofurb/mgmt.json"), MGMT_JSON, 0o644)
        rc, out, err = self.run_cli("proxy", "status")
        self.assertIn("front: off", out)
        self.assertIn("recovery: python3 /opt/synofurb-mgmt/synofurb_mgmt.py proxy disable", out)
        rc, out, err = self.run_cli("proxy", "assign", "frigate", "frigate.example.org", "_wildcard.example.org")
        self.assertEqual(rc, 0, err)
        self.assertIn("saved", out)
        self.assertEqual(self.run_cli("proxy", "assign", "frigate", "other.net", "_wildcard.example.org")[0], 2)
        rc, out, err = self.run_cli("proxy", "test")
        self.assertEqual(rc, 0)
        self.assertIn("use_backend be_addon_frigate", out)
        rc, out, err = self.run_cli("proxy", "enable")
        self.assertEqual(rc, 0, err + out)
        self.assertEqual(self.run_cli("proxy", "enable")[0], 2)
        rc, out, err = self.run_cli("proxy", "status", "--json")
        self.assertTrue(json.loads(out)["enabled"])
        self.assertEqual(self.run_cli("proxy", "confirm")[0], 0)
        self.assertEqual(self.run_cli("proxy", "confirm")[0], 2)
        rc, out, err = self.run_cli("proxy", "unassign", "frigate")
        self.assertEqual(rc, 0, err + out)
        self.assertNotIn("be_addon_frigate", fx.read("/etc/haproxy/haproxy.cfg"))
        rc, out, err = self.run_cli("proxy", "revert")                                                  # the unassign was an apply: it waits for Keep, so it can be reverted
        self.assertEqual(rc, 0, err)
        self.assertIn("be_addon_frigate", fx.read("/etc/haproxy/haproxy.cfg"))
        self.assertEqual(self.run_cli("proxy", "revert")[0], 2)                                         # nothing left to revert
        self.assertEqual(self.run_cli("proxy", "disable")[0], 0)

    def test_failed_haproxy_check_is_exit_1_and_shown(self):
        fx = self.fx
        fx.fb.flag("haproxy.invalid")
        rc, out, err = self.run_cli("proxy", "test")
        self.assertEqual(rc, 1)
        self.assertIn("REJECTED", out)

    def test_dry_run_wiring_through_main(self):
        with tempfile.TemporaryDirectory() as d:
            import contextlib
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = m.main(["--dry-run", "--root", d, "certs", "list", "--json"])
            self.assertEqual(rc, 0)
            self.assertEqual(json.loads(buf.getvalue())["certs"], [])
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = m.main(["--dry-run", "--root", d, "proxy", "status", "--json"])
            self.assertEqual(rc, 0)
            self.assertFalse(json.loads(buf.getvalue())["enabled"])
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(m.main(["--dry-run", "--root", d, "certs", "nope"]), 2)


# ============================================================================================================================
# the update bundle contract, everywhere the module is listed
# ============================================================================================================================

class BundleContractTests(unittest.TestCase):
    def read(self, rel):
        return slurp(os.path.join(REPO, rel))

    def test_python_contract(self):
        self.assertIn("synofurb_edge.py", m.BUNDLE_FILES)
        self.assertTrue(m.bundle_member_ok("synofurb_edge.py"))
        self.assertFalse(m.bundle_member_ok("synofurb_edge.pyc"))
        self.assertEqual(m.BUNDLE_FILES[-1], "synofurb_edge.py")
        spec = __import__("importlib.util").util.spec_from_file_location("mk", os.path.join(ROOT, "tools", "make-update-bundle.py"))
        mk = __import__("importlib.util").util.module_from_spec(spec)
        spec.loader.exec_module(mk)
        self.assertEqual(tuple(mk.FILES), tuple(m.BUNDLE_FILES))                                          # one order, everywhere

    def test_verify_bundle_accepts_the_module_and_still_refuses_strangers(self):
        with tempfile.TemporaryDirectory() as d:
            for name, text in (("synofurb_mgmt.py", "x = 1\n"), ("VERSION", "1.0.0\n"), ("README.md", "r\n"), ("synofurb_edge.py", "y = 2\n")):
                E.write_file(os.path.join(d, name), text, 0o644)
            m.write_manifest(d)
            files, version = m.verify_bundle(d)
            self.assertIn("synofurb_edge.py", files)
            E.write_file(os.path.join(d, "evil.py"), "z\n", 0o644)
            m.write_manifest(d, m.BUNDLE_FILES + ("evil.py",))
            # an unlisted-in-contract file in the manifest is refused
            with self.assertRaises(m.ValidationError):
                m.verify_bundle(d)

    def test_the_real_tree_builds_a_bundle_the_unit_accepts(self):
        spec = __import__("importlib.util").util.spec_from_file_location("mk", os.path.join(ROOT, "tools", "make-update-bundle.py"))
        mk = __import__("importlib.util").util.module_from_spec(spec)
        spec.loader.exec_module(mk)
        with tempfile.TemporaryDirectory() as d:
            lines = mk.build_bundle(d)
            self.assertIn("synofurb_edge.py", [ln.split("  ")[1] for ln in lines])
            files, _ = m.verify_bundle(d)
            self.assertEqual(files["synofurb_edge.py"], hashlib.sha256(open(os.path.join(ROOT, "synofurb_edge.py"), "rb").read()).hexdigest())

    def test_packaging_script_and_installer_and_rust_list_it(self):
        self.assertIn("synofurb_edge.py", self.read("scripts/package-mgmt.sh"))
        self.assertIn('("synofurb_edge.py", 0o644)', self.read("scripts/package-mgmt.sh"))
        self.assertIn("synofurb_edge.py", self.read("payload/chroot-setup.sh"))
        rs = self.read("crates/synofurb-core/src/manage.rs")
        self.assertIn('("synofurb_edge.py", false, false),', rs)                                          # MGMT_FILES
        self.assertIn('("synofurb_edge.py", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/mgmt-plane/synofurb_edge.py"))),', rs)       # EMBEDDED
        self.assertIn("synofurb-mgmt.service synofurb_addons.py addons.pack.json synofurb_edge.py; do", rs)                                             # staging script copy loop
        self.assertIn('[ -f "$NEW/synofurb_edge.py" ] && chmod 0644 "$NEW/synofurb_edge.py"', rs)                                                        # and its chmod
        self.assertIn('mgmt_asset!("mgmt/synofurb_edge.py", "synofurb_edge.py")', self.read("crates/synofurb-core/src/payload.rs"))
        self.assertIn('"mgmt/synofurb_edge.py"', self.read("crates/synofurb-core/src/payload.rs"))
        self.assertIn('"synofurb_edge.py"', self.read("crates/synofurb-core/src/manage/tests.rs"))


# ============================================================================================================================
# the web UI: copy contract, and the two pages in a real browser against the demo
# ============================================================================================================================

class UiContractTests(unittest.TestCase):
    def test_pages_copy_and_help_are_wired(self):
        src = m._UI_TEMPLATE[m._UI_TEMPLATE.rindex("<script nonce"):]
        for page in ("certs", "proxy"):
            self.assertIn("/*PAGE:%s*/" % page, src)
            self.assertIn("PAGES.%s = " % page, src)
            self.assertIn(page, m.COPY_SCREENS)
        for k in ("nav.certs", "nav.proxy"):
            self.assertIn(k, m.COPY_SHELL)
        self.assertIn("['certs', 'shield-check'], ['proxy', 'globe']", src)
        ids = {t["id"] for t in m.HELP_TOPICS}
        for need in ("certificates", "challenges", "hostnames"):
            self.assertIn(need, ids)
        used = set(re.findall(r"""\bT\('([a-z0-9_.-]+)'""", src)) | set(re.findall(r"""TIP\('([a-z0-9_.-]+)'""", src))
        used |= {"ce.st." + k for k in ("valid", "expiring", "expired", "failed", "working", "none")} | {"ce.ch." + k for k in E.CHALLENGES} | {"ce.dns." + k for k in ("ok", "wrong", "missing", "error", "nosecret")}
        used |= {"ce.act." + k for k in ("issue", "change", "renew", "renew-all")} | {"px.act.proxy-" + k for k in ("enable", "apply", "disable")} | {"ce.port." + k for k in ("http-01", "tls-alpn-01")}
        used |= {"ce.production", "ce.staging"}
        self.assertEqual(sorted(k for k in used if k not in m.COPY and not k.endswith(".")), [])

    def test_copy_obeys_the_word_budgets(self):
        lim = {"title": 4, "label": 4, "button": 3, "safety": 12, "tip": 12, "prose": 12}
        for k, e in m.COPY.items():
            if k.startswith(("ce.", "px.")) or k in ("nav.certs", "nav.proxy"):
                self.assertLessEqual(len(e["short"].split()), lim[e["kind"]], k)
                if "tip" in e:
                    self.assertTrue(e["tip"].endswith("."), k)
                    self.assertLessEqual(len(e["tip"].split()), 25, k)
        for t in m.HELP_TOPICS:
            if t["id"] in ("certificates", "challenges", "hostnames"):
                self.assertLessEqual(len(t["body"].split()), 150, t["id"])
                self.assertLessEqual(len(t["body"].split("\n\n")), 4, t["id"])
                self.assertLessEqual(len(t["title"].split()), 4)

    def test_the_hostnames_help_names_the_recovery_command(self):
        body = next(t["body"] for t in m.HELP_TOPICS if t["id"] == "hostnames")
        self.assertIn("synofurb_mgmt.py proxy disable", body)
        self.assertIn("two minutes", body)

    def test_no_secret_value_is_ever_part_of_the_page(self):
        self.assertNotIn(SECRET, m.INDEX_HTML)
        self.assertNotIn("innerHTML", E.UI_JS)                                                           # nothing the server sends is parsed as HTML
        self.assertNotIn("eval(", E.UI_JS)


@unittest.skipUnless(shutil.which("node"), "node missing")
class BrowserTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = __import__("importlib.util").util.spec_from_file_location("build_demo", os.path.join(ROOT, "tools", "build-demo.py"))
        mod = __import__("importlib.util").util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        cls.tmp = tempfile.TemporaryDirectory()
        cls.demo = os.path.join(cls.tmp.name, "index.html")
        with open(cls.demo, "w", encoding="utf-8") as fh:
            fh.write(mod.build())
        env = dict(os.environ)
        if shutil.which("npm"):
            env["NODE_PATH"] = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() + os.pathsep + env.get("NODE_PATH", "")
        r = subprocess.run(["node", os.path.join(HERE, "ui_edge.js"), "file://" + cls.demo], capture_output=True, text=True, timeout=420, env=env)
        cls.out, cls.skip = None, None
        if r.returncode == 77 or "Cannot find module 'playwright'" in r.stderr:
            cls.skip = "playwright or chromium not available"
        else:
            assert r.returncode == 0, r.stderr[-3000:]
            cls.out = json.loads(r.stdout.strip().splitlines()[-1])

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def setUp(self):
        if self.skip:
            self.skipTest(self.skip)

    def test_no_errors_no_stray_text_every_off_control_gives_its_reason(self):
        self.assertEqual(self.out["errors"], [])
        self.assertEqual(self.out["problems"], [])                                                       # null/undefined/NaN, hover-only reasons, overflow, in every state

    def test_list(self):
        f = self.out["flows"]
        rows = {r["name"]: r for r in f["rows"]}
        self.assertEqual(rows["nas.example.org"]["tags"], ["Valid"])
        self.assertEqual(rows["nas.example.org"]["challenge"], "DNS")
        self.assertEqual(rows["media.example.org"]["tags"], ["Expires soon"])
        self.assertEqual(rows["old.example.org"]["tags"], ["Expired"])
        self.assertEqual(rows["new.example.org"]["tags"], ["Failed"])
        self.assertEqual(rows["stage.example.org"]["tags"], ["Staging", "Valid"])
        self.assertIn("Create the CNAME record first", rows["new.example.org"]["detail"])                # why it failed is text under the name
        self.assertEqual(rows["old.example.org"]["buttons"], ["Renew now", "Details", "Change", "Remove"])
        self.assertTrue(f["autoRenewChip"].startswith("Auto-renew"))
        self.assertFalse(f["listPhoneOverflow"])

    def test_setup_is_required_and_says_so(self):
        f = self.out["flows"]
        self.assertIn("Set up Let's Encrypt first.", f["unsetBanner"])
        self.assertEqual(f["ownSecretField"], 1)                                                         # the wizard has the secret field, not only Settings
        self.assertEqual(f["ownSecretSaved"], 1)                                                         # a short secret is refused, a good one shows the CNAME at once
        self.assertEqual(f["unsetReason"], {"off": True, "native": False, "text": "Set up Let's Encrypt first.", "visible": True, "focusable": True})
        self.assertTrue(f["unsetFocus"] and f["unsetClickIgnored"])

    def test_dns_wizard(self):
        f = self.out["flows"]
        self.assertEqual(f["emptyNamesReason"]["text"], "Enter a name first.")
        self.assertTrue(f["emptyNamesReason"]["visible"])
        self.assertEqual(f["challengeHelps"][:5], ["Help: Names", "Help: DNS", "Help: HTTP", "Help: TLS-ALPN", "Help: Production"])      # one info button each, with its own name
        self.assertEqual([c["record"] for c in f["cnames"]], ["_acme-challenge.new.example.org", "_acme-challenge.old.example.org"])
        self.assertTrue(all(re.fullmatch(r"[0-9a-f]{1,56}\.challenges\.addr\.tools", c["target"]) for c in f["cnames"]))
        self.assertEqual(f["copyLabels"], ["Copy: new.example.org", "Copy: old.example.org"])
        self.assertTrue(f["issueOnWithName"])
        self.assertEqual(f["dnsStates"], ["Missing", "Wrong target"])
        self.assertEqual(f["jobTitle"], "files.example.org: issuing")
        self.assertEqual(f["issuedRows"], 3)

    def test_http_wizard(self):
        f = self.out["flows"]
        self.assertEqual(f["httpHint"], "Port 80 must reach this unit from the Internet.")
        self.assertIn("files.example.org", f["precheck"])
        self.assertIn("The Internet side cannot be tested from here.", f["precheck"])
        self.assertIn("a wildcard (*.example.org) needs the DNS challenge", f["wildcardRefused"])

    def test_settings_and_the_secret(self):
        f = self.out["flows"]
        self.assertRegex(f["settingsTarget"], r"^[0-9a-f]{56}\.challenges\.addr\.tools$")
        self.assertTrue(f["regenAsks"])                                                                  # replacing the secret asks first
        self.assertTrue(f["secretShown"])

    def test_details_remove_renew(self):
        f = self.out["flows"]
        self.assertIn("old.example.org", f["details"])
        self.assertIn("Last error", f["details"])
        self.assertEqual(f["removeOff"], {"off": True, "native": False, "text": "Type the name first.", "visible": True, "focusable": True})     # typed name needed, reason visible
        self.assertTrue(f["removeOn"])
        self.assertEqual(f["removedRows"], 0)
        self.assertEqual(f["renewTitle"], "nas.example.org: renewing")

    def test_hostnames(self):
        f = self.out["flows"]
        self.assertEqual(f["noCertReason"], {"off": True, "native": False, "text": "Choose a certificate.", "visible": True, "focusable": True})
        self.assertTrue(f["coveredOn"] and f["wildcardOk"])
        self.assertEqual(f["coverReason"]["text"], "The certificate does not cover that name.")
        self.assertEqual(f["dupReason"], "Use each hostname once.")
        self.assertIn("HAProxy accepts the configuration.", f["checkDialog"])
        for needle in ("HAProxy takes over ports 80 and 443.", "listens on this unit only", "press Keep within two minutes", "goes back by itself", "proxy disable"):
            self.assertIn(needle, f["enableText"])
        self.assertEqual(f["addresses"], ["https://nas.example.org/", "https://192.168.1.40/"])
        self.assertIn("Keep this setup?", f["pending"])
        self.assertEqual(f["pendingButtons"], ["Revert", "Keep"])
        self.assertTrue(f["keptGone"] and f["applyShown"])
        self.assertIn("listens on port 8443 again", f["disableText"])
        self.assertEqual(f["offAgain"], 1)

    def test_banner_on_every_page_and_phone_width(self):
        f = self.out["flows"]
        self.assertIn("Keep this setup?", f["bannerOnOtherPage"])
        self.assertTrue(f["revertedGone"])
        self.assertFalse(f["proxyPhoneOverflow"])
        self.assertFalse(f["dialogPhoneOverflow"])
        self.assertFalse(f["issuePhone"]["overflow"])
        self.assertEqual(f["issuePhone"]["cnameOutside"], 0)                                             # an 80 character CNAME target wraps instead of leaving the dialog


if __name__ == "__main__":
    unittest.main()
