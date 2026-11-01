"""Unit + HTTP-level tests for synofurb_mgmt (stdlib unittest, no root, no mdadm/lvm/networkd needed).

Run from the mgmt directory:  python3 -m unittest discover -s tests -v
"""
from __future__ import annotations

import http.client
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import synofurb_mgmt as m  # noqa: E402


# ----------------------------------------------------------------------------
# helpers
# ----------------------------------------------------------------------------

class FakeRunner(m.Runner):
    """Scripted Runner: rules are matched by argv prefix (last registered wins)."""

    def __init__(self) -> None:
        self.rules: list = []
        self.calls: list[list[str]] = []
        self.inputs: list = []
        self.avail = {"smartctl", "mdadm"}

    def on(self, prefix, out="", rc=0, err=""):
        self.rules.append((list(prefix), out if callable(out) else m.CompletedResult(rc, out, err)))
        return self

    def run(self, argv, input=None, timeout=60):
        self.calls.append(list(argv))
        self.inputs.append(input)
        for prefix, res in reversed(self.rules):
            if argv[:len(prefix)] == prefix:
                return res(argv) if callable(res) else res
        return m.CompletedResult(0, "", "")

    def which(self, name):
        return "/usr/bin/" + name if name in self.avail else None

    def called(self, *prefix) -> bool:
        return any(c[:len(prefix)] == list(prefix) for c in self.calls)


class FakeAuth:
    def __init__(self, users):
        self.users = users

    def authenticate(self, username, password):
        ok = self.users.get(username) == password
        return ok, "ok" if ok else "bad password"


LSBLK = {"blockdevices": [
    {"name": "nvme0n1", "kname": "nvme0n1", "type": "disk", "size": 256060514304, "fstype": None, "uuid": None, "label": None,
     "mountpoints": [None], "model": "SysNVMe", "serial": "S1", "rota": False, "tran": "nvme", "pkname": None, "children": [
         {"name": "nvme0n1p1", "kname": "nvme0n1p1", "type": "part", "size": 536870912, "fstype": "vfat", "uuid": "ABCD-1234",
          "label": None, "mountpoints": ["/boot/efi"], "model": None, "serial": None, "rota": False, "tran": None, "pkname": "nvme0n1"},
         {"name": "nvme0n1p2", "kname": "nvme0n1p2", "type": "part", "size": 100000000000, "fstype": "LVM2_member", "uuid": "pvuuid",
          "label": None, "mountpoints": [None], "model": None, "serial": None, "rota": False, "tran": None, "pkname": "nvme0n1", "children": [
              {"name": "vg_sys-root", "kname": "dm-0", "type": "lvm", "size": 50000000000, "fstype": "ext4", "uuid": "rootuuid",
               "label": None, "mountpoints": ["/"], "model": None, "serial": None, "rota": False, "tran": None, "pkname": "nvme0n1p2"},
              {"name": "vg_sys-swap", "kname": "dm-1", "type": "lvm", "size": 2000000000, "fstype": "swap", "uuid": "swapuuid",
               "label": None, "mountpoints": ["[SWAP]"], "model": None, "serial": None, "rota": False, "tran": None, "pkname": "nvme0n1p2"}]}]},
    {"name": "sda", "kname": "sda", "type": "disk", "size": 4000787030016, "fstype": None, "uuid": None, "label": None,
     "mountpoints": [None], "model": "WD40", "serial": "A", "rota": True, "tran": "sata", "pkname": None},
    {"name": "sdb", "kname": "sdb", "type": "disk", "size": 4000787030016, "fstype": None, "uuid": None, "label": None,
     "mountpoints": [None], "model": "WD40", "serial": "B", "rota": True, "tran": "sata", "pkname": None},
    {"name": "sdc", "kname": "sdc", "type": "disk", "size": 1000204886016, "fstype": None, "uuid": None, "label": None,
     "mountpoints": [None], "model": "USB", "serial": "C", "rota": True, "tran": "usb", "pkname": None, "children": [
         {"name": "sdc1", "kname": "sdc1", "type": "part", "size": 1000000000000, "fstype": "ntfs", "uuid": "1234567890ABCDEF",
          "label": "Backup", "mountpoints": [None], "model": None, "serial": None, "rota": True, "tran": None, "pkname": "sdc"}]},
    {"name": "sdd", "kname": "sdd", "type": "disk", "size": 2000398934016, "fstype": "linux_raid_member", "uuid": "x", "label": None,
     "mountpoints": [None], "model": "WD20", "serial": "D", "rota": True, "tran": "sata", "pkname": None},
]}
LSBLK_TEXT = json.dumps(LSBLK)
IP_ETH0 = json.dumps([{"ifname": "lo", "operstate": "UNKNOWN", "address": "00:00:00:00:00:00", "mtu": 65536, "addr_info": []},
                      {"ifname": "eth0", "operstate": "UP", "address": "aa:bb:cc:dd:ee:ff", "mtu": 1500,
                       "addr_info": [{"family": "inet", "local": "192.168.1.5", "prefixlen": 24, "scope": "global"}]}])
MDADM_SCAN = "ARRAY /dev/md/media metadata=1.2 name=nas:media UUID=aaaaaaaa:bbbbbbbb:cccccccc:dddddddd\n"


def make_runner() -> FakeRunner:
    r = FakeRunner()
    r.on(["lsblk"], LSBLK_TEXT).on(["ip", "-j", "addr"], IP_ETH0).on(["blkid"], "11111111-2222-4333-8444-555555555555\n")
    r.on(["mdadm", "--detail", "--scan"], MDADM_SCAN)
    r.on(["systemctl", "is-active"], "inactive\n", rc=3)
    return r


class Env:
    """Temp root + app + running ThreadingHTTPServer on 127.0.0.1:<ephemeral> (--no-tls --dry-run equivalent)."""

    def __init__(self, runner=None, users=None, accounts=True):
        self.tmp = tempfile.mkdtemp(prefix="synofurb-test-")
        self.cfg = m.Config(root=self.tmp, dry_run=True, tls=False, fail_delay=0)
        self.runner = runner or make_runner()
        self.app = m.App(self.cfg, self.runner, auth=FakeAuth(users or {"admin": "correct-horse"}))
        self.app.power_delay = 0
        if accounts:
            self.write("/etc/passwd", "root:x:0:0:root:/root:/bin/bash\nadmin:x:1000:1000::/home/admin:/bin/bash\n"
                       "bob:x:1001:1001::/home/bob:/bin/bash\n")
            self.write("/etc/group", "root:x:0:\nsudo:x:27:admin\nusers:x:100:\nadm:x:4:\nadmin:x:1000:\nbob:x:1001:\n")
            self.write("/etc/shadow", "root:*:1:::::\nadmin:$6$x$y:1::::::\nbob:$6$x$y:1::::::\n")
        self.srv = m.MgmtServer(("127.0.0.1", 0), self.app)
        self.port = self.srv.server_address[1]
        self.thread = threading.Thread(target=lambda: self.srv.serve_forever(poll_interval=0.02), daemon=True)
        self.thread.start()

    def write(self, path, text):
        p = self.cfg.p(path)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as fh:
            fh.write(text)

    def read(self, path):
        return m.read_text(self.cfg.p(path))

    def close(self):
        self.srv.shutdown()
        self.srv.server_close()
        shutil.rmtree(self.tmp, ignore_errors=True)


class Client:
    def __init__(self, env: Env):
        self.env, self.cookie, self.csrf = env, None, None

    def raw(self, method, path, body=None, headers=None, data=None):
        c = http.client.HTTPConnection("127.0.0.1", self.env.port, timeout=10)
        h = dict(headers or {})
        if method == "POST":
            h.setdefault("Content-Type", "application/json")
        if data is None and body is not None:
            data = json.dumps(body).encode()
        c.request(method, path, body=data, headers=h)
        r = c.getresponse()
        raw = r.read()
        c.close()
        try:
            obj = json.loads(raw) if raw and "json" in (r.getheader("Content-Type") or "") else raw
        except ValueError:
            obj = raw
        return r.status, obj, r

    def login(self, user="admin", pw="correct-horse"):
        st, obj, r = self.raw("POST", "/api/login", {"username": user, "password": pw})
        if st == 200:
            self.cookie = r.getheader("Set-Cookie").split(";")[0]
            self.csrf = obj["csrf"]
        return st, obj, r

    def call(self, method, path, body=None, csrf=True, headers=None):
        h = {"Cookie": self.cookie} if self.cookie else {}
        if csrf and self.csrf:
            h["X-CSRF-Token"] = self.csrf
        h.update(headers or {})
        return self.raw(method, path, body, h)


class HttpTestCase(unittest.TestCase):
    runner_factory = staticmethod(make_runner)

    def setUp(self):
        self.env = Env(self.runner_factory())
        self.cl = Client(self.env)
        self.runner = self.env.runner
        self.addCleanup(self.env.close)

    def logged_in(self) -> Client:
        st, _, _ = self.cl.login()
        self.assertEqual(st, 200)
        return self.cl


# ----------------------------------------------------------------------------
# validators
# ----------------------------------------------------------------------------

class ValidatorTests(unittest.TestCase):
    def bad(self, fn, *values, **kw):
        for v in values:
            with self.assertRaises(m.ValidationError, msg=repr(v)):
                fn(v, **kw)

    def test_username(self):
        for ok in ("alice", "_svc", "a-b_c9", "x" * 32):
            self.assertEqual(m.valid_username(ok), ok)
        self.bad(m.valid_username, "", "Alice", "9lives", "a b", "a;b", "a\n", "-x", "x" * 33, "../x", None, 5, "root\x00")

    def test_hostname(self):
        self.assertEqual(m.valid_hostname("NAS-01"), "nas-01")
        self.assertEqual(m.valid_hostname("nas.example.org"), "nas.example.org")
        self.bad(m.valid_hostname, "", "-nas", "nas-", "na s", "nas_1", "a..b", "x" * 65, "nas;reboot", "nas\n", "é")

    def test_ifname(self):
        for ok in ("eth0", "enp3s0", "end0", "wlan0.1", "br-lan"):
            self.assertEqual(m.valid_ifname(ok), ok)
        self.bad(m.valid_ifname, "", "../x", "a b", "eth0;x", "x" * 16, "-eth", "eth*", "a/b")

    def test_mountpoint(self):
        for ok in ("/srv/media", "/mnt/usb-1", "/mnt/a/b", "/srv/data2"):
            self.assertEqual(m.valid_mountpoint(ok), ok)
        self.bad(m.valid_mountpoint, "/", "/srv", "/mnt/", "/etc/x", "/srv/../etc", "/srv//x", "/srv/a b", "srv/x",
                 "/srv/x/", "/mnt/..", "/srvx/a", "/srv/a;b", "/srv/.hidden", "/mnt/a/b/c/d", "/srv/$(x)")

    def test_mountpoint_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as t:
            cfg = m.Config(root=t)
            os.makedirs(os.path.join(t, "srv"))
            os.symlink("/etc", os.path.join(t, "srv", "evil"))
            with self.assertRaises(m.ValidationError):
                m.check_no_symlinks(cfg, "/srv/evil/x")
            m.check_no_symlinks(cfg, "/srv/fine")

    def test_device(self):
        for ok in ("/dev/sda", "/dev/sda1", "/dev/nvme0n1p2", "/dev/md/media", "/dev/mapper/vg_x-x", "/dev/vg_x/x", "/dev/md127"):
            self.assertEqual(m.valid_device(ok), ok)
        self.bad(m.valid_device, "sda", "/dev/", "/dev/../etc/passwd", "/dev/sda;rm", "/dev/a/b/c", "/etc/passwd", "/dev/sd a",
                 "/dev/-x", "/dev/sda\n")

    def test_volume_name_and_fs(self):
        self.assertEqual(m.valid_volume_name("media"), "media")
        self.assertEqual(m.valid_volume_name("backup_2"), "backup_2")
        self.bad(m.valid_volume_name, "", "Media", "1x", "a-b", "a b", "x" * 25, "data", "sys", "root", "a/b", "a;b")
        self.assertEqual(m.valid_fs_create("btrfs"), "btrfs")
        self.bad(m.valid_fs_create, "ntfs", "vfat", "zfs", "xfs; reboot")
        self.assertEqual(m.valid_fs_mount("ntfs"), "ntfs")
        self.bad(m.valid_fs_mount, "swap", "crypto_LUKS", "nfs")
        self.assertEqual(m.valid_raid_level("raid10"), "raid10")
        self.bad(m.valid_raid_level, "raid2", "RAID1", "")

    def test_ip_validators(self):
        self.assertEqual(m.valid_ipv4_cidr("192.168.1.10/24"), "192.168.1.10/24")
        self.bad(m.valid_ipv4_cidr, "192.168.1.10", "192.168.1.0/24", "192.168.1.255/24", "0.0.0.0/0", "127.0.0.1/8",
                 "224.0.0.1/24", "1.2.3.4/33", "300.1.1.1/24", "1.2.3.4/24; reboot", "::1/64", "169.254.1.1/16")
        self.assertEqual(m.valid_ipv6_cidr("2001:db8::10/64"), "2001:db8::10/64")
        self.bad(m.valid_ipv6_cidr, "2001:db8::10", "fe80::1/64", "::1/128", "::/0", "ff02::1/64", "1.2.3.4/24", "2001:db8::10/129")
        self.assertEqual(m.valid_dns("1.1.1.1"), "1.1.1.1")
        self.assertEqual(m.valid_dns("2606:4700:4700::1111"), "2606:4700:4700::1111")
        self.bad(m.valid_dns, "0.0.0.0", "dns.example", "1.1.1.1\nDNS=6.6.6.6", "224.0.0.1")
        self.assertEqual(m.valid_gateway("192.168.1.1", 4, "192.168.1.10/24"), "192.168.1.1")
        with self.assertRaises(m.ValidationError):
            m.valid_gateway("10.0.0.1", 4, "192.168.1.10/24")
        with self.assertRaises(m.ValidationError):
            m.valid_gateway("192.168.1.1", 6, "2001:db8::10/64")
        self.assertEqual(m.valid_gateway("fe80::1", 6, "2001:db8::10/64"), "fe80::1")      # link-local allowed
        self.assertEqual(m.valid_search_domain("Home.Example"), "home.example")
        self.bad(m.valid_search_domain, "bad domain", "a..b", "", "-x.org")

    def test_ssh_key_and_password(self):
        key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl user@host"
        self.assertEqual(m.valid_ssh_pubkey(key + "\n"), key)
        self.bad(m.valid_ssh_pubkey, 'command="rm -rf /" ' + key, key + "\nssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ x", "ssh-ed25519 short", "", "hello")
        self.assertEqual(m.valid_password("longenough"), "longenough")
        self.bad(m.valid_password, "short", "x" * 129, "has\nnewline1", "nul\x00byte1234")


# ----------------------------------------------------------------------------
# pure rendering / parsing
# ----------------------------------------------------------------------------

class NetworkdTests(unittest.TestCase):
    def test_dhcp(self):
        t = m.render_networkd("eth0")
        self.assertIn("[Match]\nName=eth0", t)
        for line in ("DHCP=yes", "IPv6AcceptRA=yes", "ClientIdentifier=mac"):
            self.assertIn(line, t)
        self.assertEqual(m.parse_networkd(t)["mode"], "dhcp")

    def test_static_v4(self):
        t = m.render_networkd("eth0", "static", "192.168.1.10/24", "192.168.1.1", dns=["1.1.1.1", "9.9.9.9"], search=["lan", "home.arpa"])
        self.assertIn("DHCP=no", t)
        self.assertIn("IPv6AcceptRA=yes", t)          # SLAAC still works without a static v6 address
        self.assertIn("Address=192.168.1.10/24", t)
        self.assertIn("[Route]\nGateway=192.168.1.1", t)
        self.assertIn("DNS=1.1.1.1\nDNS=9.9.9.9", t)
        self.assertIn("Domains=lan home.arpa", t)
        d = m.parse_networkd(t)
        self.assertEqual((d["mode"], d["ipv4"], d["gw4"], d["dns"], d["search"]),
                         ("static", "192.168.1.10/24", "192.168.1.1", ["1.1.1.1", "9.9.9.9"], ["lan", "home.arpa"]))

    def test_static_dual_stack(self):
        t = m.render_networkd("eth0", "static", "10.0.0.2/24", "10.0.0.1", "2001:db8::2/64", "2001:db8::1")
        self.assertIn("IPv6AcceptRA=no", t)
        self.assertEqual(t.count("[Route]"), 2)
        d = m.parse_networkd(t)
        self.assertEqual((d["ipv6"], d["gw6"]), ("2001:db8::2/64", "2001:db8::1"))

    def test_errors(self):
        with self.assertRaises(m.ValidationError):
            m.render_networkd("eth0", "static", None, None)
        with self.assertRaises(m.ValidationError):
            m.render_networkd("eth0", "bridge")
        with self.assertRaises(m.ValidationError):
            m.render_networkd("eth0\nDHCP=no")


MDSTAT = """Personalities : [raid1] [raid6] [raid5] [raid4]
md0 : active raid1 sdb1[1] sda1[0]
      1046528 blocks super 1.2 [2/2] [UU]
      bitmap: 0/1 pages [0KB], 65536KB chunk

md1 : active raid5 sdd[3] sdc[1](F) sdb[0]
      7813772288 blocks super 1.2 level 5, 512k chunk, algorithm 2 [3/2] [U_U]
      [==>..................]  recovery = 12.6% (493824/3906886144) finish=210.4min speed=52000K/sec

md127 : inactive sde[0](S)
      976630488 blocks super 1.2

unused devices: <none>
"""


class ParserTests(unittest.TestCase):
    def test_mdstat(self):
        a = {x["name"]: x for x in m.parse_mdstat(MDSTAT)}
        self.assertEqual(set(a), {"md0", "md1", "md127"})
        self.assertEqual((a["md0"]["level"], a["md0"]["total"], a["md0"]["active"], a["md0"]["health"]), ("raid1", 2, 2, "UU"))
        self.assertFalse(a["md0"]["degraded"])
        self.assertEqual(a["md0"]["size"], 1046528 * 1024)
        self.assertTrue(a["md1"]["degraded"])
        self.assertTrue(any(x["failed"] and x["name"] == "sdc" for x in a["md1"]["members"]))
        self.assertEqual(a["md1"]["sync"]["action"], "recovery")
        self.assertAlmostEqual(a["md1"]["sync"]["percent"], 12.6)
        self.assertEqual(a["md1"]["sync"]["speed_kbs"], 52000)
        self.assertEqual(a["md127"]["state"], "inactive")
        self.assertTrue(a["md127"]["members"][0]["spare"])
        self.assertEqual(m.parse_mdstat("Personalities :\nunused devices: <none>\n"), [])

    def test_mdadm_scan(self):
        r = m.parse_mdadm_scan(MDADM_SCAN + "garbage\n")
        self.assertEqual(len(r), 1)
        self.assertEqual((r[0]["device"], r[0]["uuid"]), ("/dev/md/media", "aaaaaaaa:bbbbbbbb:cccccccc:dddddddd"))

    def test_lsblk_and_candidates(self):
        devs = m.parse_lsblk(LSBLK_TEXT)
        by = {d["kname"]: d for d in devs}
        self.assertEqual(by["dm-0"]["path"], "/dev/mapper/vg_sys-root")
        self.assertEqual(by["sda"]["path"], "/dev/sda")
        self.assertEqual(by["nvme0n1"]["children"], ["nvme0n1p1", "nvme0n1p2"])
        self.assertEqual(by["sdc1"]["fstype"], "ntfs")
        self.assertEqual(by["nvme0n1p1"]["mountpoints"], ["/boot/efi"])
        self.assertEqual(by["sda"]["mountpoints"], [])
        free, excl = m.candidate_devices(devs)
        self.assertEqual(sorted(d["path"] for d in free), ["/dev/sda", "/dev/sdb"])
        reasons = {d["path"]: d["reason"] for d in excl}
        self.assertIn("system", reasons["/dev/nvme0n1"])
        self.assertIn("RAID member", reasons["/dev/sdd"])
        self.assertIn("ntfs", reasons["/dev/sdc1"])
        self.assertIn("in use", reasons["/dev/sdc"])
        self.assertIn("root/boot", reasons["/dev/nvme0n1p1"])
        self.assertEqual(m.parse_lsblk("not json"), [])

    def test_lvm_findmnt_ip(self):
        rep = json.dumps({"report": [{"vg": [{"vg_name": "vg_data", "vg_size": "1000", "vg_free": "200", "pv_count": "1"}]}]})
        self.assertEqual(m.parse_lvm_report(rep, "vg")[0]["vg_free"], 200)
        self.assertEqual(m.parse_lvm_report("", "vg"), [])
        fm = json.dumps({"filesystems": [{"target": "/", "source": "/dev/mapper/vg_sys-root", "fstype": "ext4", "options": "rw",
                                          "children": [{"target": "/proc", "source": "proc", "fstype": "proc"},
                                                       {"target": "/srv/data", "source": "/dev/mapper/vg_data-data", "fstype": "xfs"}]}]})
        rows = m.parse_findmnt(fm)
        self.assertEqual([r["target"] for r in rows if r["real"]], ["/", "/srv/data"])
        self.assertEqual(m.parse_ip_addr(IP_ETH0)[1]["addrs"][0]["addr"], "192.168.1.5")

    def test_passwd_group_shadow(self):
        pw = m.parse_passwd("root:x:0:0::/root:/bin/bash\nalice:x:1000:1000:A:/home/alice:/bin/zsh\n#c\n")
        gr = m.parse_group("sudo:x:27:alice,bob\nusers:x:100:\n")
        self.assertEqual((pw["alice"].uid, pw["alice"].shell), (1000, "/bin/zsh"))
        self.assertEqual(m.sudo_members(pw, gr), ["alice"])
        sh = m.parse_shadow("alice:!$6$x$y:19000:0:99999:7:::\n")
        self.assertTrue(m.hash_is_locked(sh["alice"]["hash"]))
        self.assertFalse(m.hash_has_password("*"))
        self.assertTrue(m.hash_has_password("$y$j9T$abc$def"))


FSTAB = "# /etc/fstab: static file system information.\nUUID=aaaa / ext4 errors=remount-ro 0 1\n\n# my own comment\n/dev/sdz1 /mnt/old ext4 defaults 0 2\n"


class FstabTests(unittest.TestCase):
    def test_add_preserves_and_is_idempotent(self):
        t1 = m.fstab_add(FSTAB, "UUID=1111", "/srv/media", "xfs", "defaults,nofail", "media")
        self.assertTrue(t1.startswith(FSTAB))
        self.assertIn("# synofurb: media\nUUID=1111 /srv/media xfs defaults,nofail 0 0\n", t1)
        t2 = m.fstab_add(t1, "UUID=1111", "/srv/media", "xfs", "defaults,nofail", "media")
        self.assertEqual(t1, t2)
        t3 = m.fstab_add(t1, "UUID=2222", "/srv/media", "xfs", "defaults,nofail", "media")      # replaces own entry
        self.assertEqual(t3.count("/srv/media"), 1)
        self.assertIn("UUID=2222", t3)
        ents = m.fstab_entries(t3)
        self.assertEqual([e["managed"] for e in ents if e["mountpoint"] == "/srv/media"], [True])

    def test_remove_keeps_unrelated(self):
        t1 = m.fstab_add(FSTAB, "UUID=1111", "/srv/media", "xfs", "defaults,nofail", "media")
        t2, removed = m.fstab_remove(t1, "/srv/media")
        self.assertTrue(removed)
        self.assertEqual(t2, FSTAB)
        t3, removed = m.fstab_remove(t2, "/srv/media")
        self.assertFalse(removed)
        self.assertEqual(t3, FSTAB)
        t4, removed = m.fstab_remove(FSTAB, "/mnt/old", only_managed=True)
        self.assertFalse(removed)
        self.assertEqual(t4, FSTAB)
        t5, removed = m.fstab_remove(FSTAB, "/mnt/old")
        self.assertTrue(removed)
        self.assertIn("# my own comment", t5)
        self.assertNotIn("/mnt/old", t5)

    def test_unmanaged_conflict(self):
        with self.assertRaises(ValueError):
            m.fstab_add(FSTAB, "UUID=1", "/mnt/old", "xfs", "defaults", "x")

    def test_mount_options(self):
        self.assertEqual(m.mount_options("xfs"), ("xfs", "defaults,nofail"))
        self.assertEqual(m.mount_options("ext4", ro=True), ("ext4", "defaults,nofail,ro"))
        self.assertEqual(m.mount_options("btrfs", persist=True), ("btrfs", "defaults,nofail,x-systemd.device-timeout=30"))
        self.assertEqual(m.mount_options("vfat", 1000, 100), ("vfat", "defaults,nofail,uid=1000,gid=100,umask=0022"))
        self.assertEqual(m.mount_options("exfat", 1000, 100)[0], "exfat")
        t, o = m.mount_options("ntfs", 1000, 100, ro=True)
        self.assertEqual(t, "ntfs-3g")
        self.assertIn("uid=1000", o)
        self.assertTrue(o.endswith(",ro"))
        with self.assertRaises(m.ValidationError):
            m.mount_options("reiserfs")


class PlanTests(unittest.TestCase):
    def test_raid1_plan_order(self):
        steps = m.plan_volume_create("media", ["/dev/sda", "/dev/sdb"], "raid1", "xfs", "/srv/media")
        cmds = [s.display() for s in steps]
        order = ["wipefs", "mdadm --create", "mdadm --detail --scan", "update-initramfs", "pvcreate", "vgcreate", "lvcreate", "mkfs.xfs",
                 "[edit /etc/fstab]", "mkdir", "systemctl daemon-reload", "mount /srv/media"]
        pos = [next(i for i, c in enumerate(cmds) if o in c) for o in order]
        self.assertEqual(pos, sorted(pos))
        create = next(s.argv for s in steps if s.argv and s.argv[:2] == ["mdadm", "--create"])
        self.assertEqual(create, ["mdadm", "--create", "/dev/md/media", "--run", "--level=raid1", "--raid-devices=2",
                                  "--metadata=1.2", "--name=media", "/dev/sda", "/dev/sdb"])
        self.assertIn(["vgcreate", "vg_media", "/dev/md/media"], [s.argv for s in steps])
        self.assertIn(["lvcreate", "--yes", "-n", "media", "-l", "100%FREE", "vg_media"], [s.argv for s in steps])
        self.assertIn(["mkfs.xfs", "-f", "-L", "media", "/dev/vg_media/media"], [s.argv for s in steps])
        fst = next(s for s in steps if s.kind == "fstab")
        self.assertIn("nofail", fst.meta["options"])
        self.assertIn("x-systemd.device-timeout=30", fst.meta["options"])

    def test_no_raid_and_sizes(self):
        steps = m.plan_volume_create("tank", ["/dev/sda"], "none", "ext4", "/mnt/tank", 100)
        argvs = [s.argv for s in steps if s.argv]
        self.assertFalse(any(a[0] == "mdadm" for a in argvs))
        self.assertIn(["pvcreate", "--yes", "/dev/sda"], argvs)
        self.assertIn(["lvcreate", "--yes", "-n", "tank", "-L", "100G", "vg_tank"], argvs)
        self.assertIn(["mkfs.ext4", "-F", "-L", "tank", "/dev/vg_tank/tank"], argvs)
        b = m.plan_volume_create("bt", ["/dev/sda"], "none", "btrfs", "/srv/bt")
        self.assertIn(["mkfs.btrfs", "-f", "-L", "bt", "/dev/vg_bt/bt"], [s.argv for s in b])

    def test_raid_count_rules(self):
        devs = ["/dev/sd%s" % c for c in "abcdef"]
        for level, n, ok in (("raid1", 1, False), ("raid1", 2, True), ("raid0", 1, False), ("raid5", 2, False), ("raid5", 3, True),
                             ("raid6", 3, False), ("raid6", 4, True), ("raid10", 3, False), ("raid10", 4, True),
                             ("raid10", 5, False), ("raid10", 6, True), ("none", 0, False), ("none", 3, True)):
            if ok:
                m.plan_volume_create("v", devs[:n], level, "xfs", "/srv/v")
            else:
                with self.assertRaises(m.ValidationError, msg="%s/%d" % (level, n)):
                    m.plan_volume_create("v", devs[:n], level, "xfs", "/srv/v")

    def test_plan_rejects_bad_input(self):
        for kw in ({"name": "a b"}, {"devices": ["/dev/sda", "/dev/sda"]}, {"devices": ["/dev/sda;x"]}, {"fs": "ntfs"},
                   {"mountpoint": "/etc/x"}, {"size_gib": 0}, {"level": "raid7"}):
            args = dict(name="v", devices=["/dev/sda", "/dev/sdb"], level="raid1", fs="xfs", mountpoint="/srv/v", size_gib=None)
            args.update(kw)
            with self.assertRaises(m.ValidationError, msg=str(kw)):
                m.plan_volume_create(**args)

    def test_extend_and_delete_plans(self):
        x = m.plan_volume_extend("vg_data", "data", "xfs", "/srv/data", None)
        self.assertEqual(x[0].argv, ["lvextend", "-l", "+100%FREE", "/dev/vg_data/data"])
        self.assertEqual(x[1].argv, ["xfs_growfs", "/srv/data"])
        self.assertEqual(m.plan_volume_extend("vg_data", "data", "ext4", None, 5)[1].argv, ["resize2fs", "/dev/vg_data/data"])
        self.assertEqual(m.plan_volume_extend("v", "d", "btrfs", "/srv/d", 1)[1].argv, ["btrfs", "filesystem", "resize", "max", "/srv/d"])
        with self.assertRaises(m.ValidationError):
            m.plan_volume_extend("v", "d", "xfs", None, None)
        d = m.plan_volume_delete("vg_x", "x", ["/srv/x"], True, ["/dev/md127"])
        self.assertEqual([s.argv[0] for s in d if s.argv], ["umount", "lvremove", "vgremove", "pvremove"])

    def test_estimate_usable(self):
        g = m.GIB
        self.assertEqual(m.estimate_usable("raid1", [4 * g, 5 * g]), 4 * g)
        self.assertEqual(m.estimate_usable("raid5", [4 * g] * 4), 12 * g)
        self.assertEqual(m.estimate_usable("raid10", [4 * g] * 4), 8 * g)
        self.assertEqual(m.estimate_usable("none", [1 * g, 2 * g]), 3 * g)


# ----------------------------------------------------------------------------
# auth primitives
# ----------------------------------------------------------------------------

class ShadowTests(unittest.TestCase):
    def setUp(self):
        if not m.crypt_available():
            self.skipTest("libcrypt not available")

    def sha512(self, pw, salt="saltsalt1"):
        h = m.crypt_hash(pw, "$6$%s$" % salt)
        self.assertTrue(h and h.startswith("$6$"))
        return h

    def test_verify_sha512(self):
        h = self.sha512("s3cret-pw")
        self.assertTrue(m.verify_shadow_hash("s3cret-pw", h))
        self.assertFalse(m.verify_shadow_hash("s3cret-pX", h))
        self.assertFalse(m.verify_shadow_hash("", h))

    @unittest.skipUnless(shutil.which("openssl"), "openssl missing")
    def test_matches_openssl(self):
        out = subprocess.run(["openssl", "passwd", "-6", "-salt", "abcdefgh", "hunter2hunter2"], capture_output=True, text=True).stdout.strip()
        self.assertTrue(m.verify_shadow_hash("hunter2hunter2", out))
        self.assertFalse(m.verify_shadow_hash("hunter2hunter3", out))

    def test_locked_and_empty_never_verify(self):
        h = self.sha512("pw-pw-pw-pw")
        for stored in ("!" + h, "*", "!", "!!", "", "!*", "x", "plaintext"):
            self.assertFalse(m.verify_shadow_hash("pw-pw-pw-pw", stored), stored)
        self.assertFalse(m.verify_shadow_hash("pw\x00x", h))

    def test_yescrypt_if_supported(self):
        h = m.crypt_hash("yes-pw-yes", "$y$j9T$abcdefgh1234$")
        if not h:
            self.skipTest("libcrypt has no yescrypt")
        self.assertTrue(m.verify_shadow_hash("yes-pw-yes", h))
        self.assertFalse(m.verify_shadow_hash("yes-pw-no", h))

    def test_shadow_auth_backend(self):
        with tempfile.TemporaryDirectory() as t:
            cfg = m.Config(root=t)
            env = Env.__new__(Env)
            env.cfg = cfg
            h = self.sha512("alice-pass-1")
            for path, text in (("/etc/passwd", "root:x:0:0::/root:/bin/sh\nalice:x:1000:1000::/h:/bin/sh\nbob:x:1001:1001::/h:/bin/sh\n"
                                               "carol:x:1002:1002::/h:/bin/sh\n"),
                               ("/etc/group", "root:x:0:\nsudo:x:27:alice,carol\n"),
                               ("/etc/shadow", "root:%s:1::::::\nalice:%s:1::::::\nbob:%s:1::::::\ncarol:!%s:1::::::\n" % (h, h, h, h))):
                os.makedirs(os.path.dirname(cfg.p(path)), exist_ok=True)
                with open(cfg.p(path), "w") as fh:
                    fh.write(text)
            a = m.ShadowAuth(cfg)
            self.assertEqual(a.authenticate("alice", "alice-pass-1"), (True, "ok"))
            self.assertFalse(a.authenticate("alice", "wrong")[0])
            self.assertFalse(a.authenticate("root", "alice-pass-1")[0])        # uid 0 never
            self.assertFalse(a.authenticate("bob", "alice-pass-1")[0])         # not in sudo
            self.assertFalse(a.authenticate("carol", "alice-pass-1")[0])       # locked
            self.assertFalse(a.authenticate("nobody-here", "x")[0])


class SessionAndThrottleTests(unittest.TestCase):
    def test_session_idle_and_absolute_expiry(self):
        now = [1000.0]
        s = m.SessionStore(idle=1800, absolute=43200, time_fn=lambda: now[0])
        sid, sess = s.create("alice", "1.2.3.4")
        self.assertGreaterEqual(len(sid), 43)                       # 256 bit, urlsafe base64
        self.assertEqual(len(sess.csrf), len(m.secrets.token_urlsafe(32)))
        self.assertIsNotNone(s.get(sid))
        now[0] += 1700
        self.assertIsNotNone(s.get(sid))                            # activity refreshes idle timer
        now[0] += 1801
        self.assertIsNone(s.get(sid))                               # idle timeout
        sid2, _ = s.create("alice", "1.2.3.4")
        for _ in range(26):                                         # keep active, hit absolute limit (>12h)
            now[0] += 1700
            if s.get(sid2) is None:
                break
        self.assertIsNone(s.get(sid2))
        self.assertIsNone(s.get("nonsense"))
        self.assertIsNone(s.get(None))

    def test_destroy_and_cap(self):
        s = m.SessionStore()
        sid, _ = s.create("a", "x")
        s.destroy(sid)
        self.assertIsNone(s.get(sid))
        for _ in range(m.SessionStore.MAX_SESSIONS + 10):
            s.create("a", "x")
        self.assertLessEqual(len(s._s), m.SessionStore.MAX_SESSIONS)
        sid, _ = s.create("victim", "x")
        s.destroy_user("victim")
        self.assertIsNone(s.get(sid))

    def test_throttle_backoff(self):
        now = [0.0]
        t = m.LoginThrottle(time_fn=lambda: now[0])
        for _ in range(4):
            t.fail("user:a")
        self.assertEqual(t.retry_after("user:a"), 0)
        t.fail("user:a")
        self.assertTrue(295 <= t.retry_after("user:a") <= 301)
        self.assertEqual(t.retry_after("user:b"), 0)
        now[0] += 301
        self.assertEqual(t.retry_after("user:a"), 0)
        for _ in range(5):
            t.fail("user:a")
        self.assertTrue(t.retry_after("user:a") > 590)               # second lockout doubles to 10 min
        t.success("user:a")
        self.assertEqual(t.retry_after("user:a"), 0)

    def test_redact_and_audit(self):
        r = m.redact({"username": "a", "password": "p", "nested": {"new_password": "q", "ok": 1}, "x": "y" * 400})
        self.assertEqual(r["password"], "***")
        self.assertEqual(r["nested"]["new_password"], "***")
        self.assertEqual(r["nested"]["ok"], 1)
        self.assertTrue(len(r["x"]) < 320)
        with tempfile.TemporaryDirectory() as t:
            log = m.AuditLog(os.path.join(t, "sub", "audit.log"))
            log.log("alice", "1.2.3.4", "/api/x", {"password": "hunter2"}, "ok")
            with open(os.path.join(t, "sub", "audit.log")) as fh:
                line = fh.read()
            self.assertNotIn("hunter2", line)
            self.assertEqual(json.loads(line)["user"], "alice")
            self.assertEqual(os.stat(os.path.join(t, "sub", "audit.log")).st_mode & 0o777, 0o600)


# ----------------------------------------------------------------------------
# runners
# ----------------------------------------------------------------------------

class RunnerTests(unittest.TestCase):
    def test_real_runner_basics(self):
        r = m.RealRunner()
        res = r.run(["echo", "hi"])
        self.assertEqual((res.rc, res.out), (0, "hi\n"))
        self.assertEqual(r.run(["definitely-not-a-command-xyz"]).rc, 127)
        self.assertEqual(r.run(["cat"], input="from stdin").out, "from stdin")
        self.assertEqual(r.run(["sleep", "5"], timeout=1).rc, 124)
        with self.assertRaises(ValueError):
            r.run("echo hi")                       # no strings / shell lines

    def test_dry_run_records_and_never_executes(self):
        d = m.DryRunRunner()
        res = d.run(["mdadm", "--create", "/dev/md/x"], input="secret")
        self.assertTrue(res.ok)
        self.assertEqual(d.calls, [["mdadm", "--create", "/dev/md/x"]])
        self.assertEqual(d.input_sizes, [6])          # content of stdin is not retained
        self.assertTrue(d.run(["blkid"]).out.strip())

    def test_no_tls_requires_dry_run(self):
        self.assertEqual(m.main(["--no-tls"]), 2)


# ----------------------------------------------------------------------------
# HTTP-level tests (real ThreadingHTTPServer on 127.0.0.1, no TLS, dry-run, fake auth)
# ----------------------------------------------------------------------------

class HttpAuthTests(HttpTestCase):
    def test_ui_and_security_headers(self):
        st, body, r = self.cl.raw("GET", "/")
        self.assertEqual(st, 200)
        csp = r.getheader("Content-Security-Policy")
        nonce = csp.split("script-src 'nonce-")[1].split("'")[0]
        self.assertIn('<script nonce="%s">' % nonce, body.decode())
        self.assertNotIn("__NONCE__", body.decode())
        self.assertNotIn("unsafe-inline", csp)
        self.assertEqual(r.getheader("X-Frame-Options"), "DENY")
        self.assertEqual(r.getheader("Referrer-Policy"), "no-referrer")
        self.assertEqual(r.getheader("X-Content-Type-Options"), "nosniff")
        st2, _, r2 = self.cl.raw("GET", "/")
        self.assertNotEqual(csp, r2.getheader("Content-Security-Policy"))         # fresh nonce per response

    def test_401_without_cookie(self):
        for path in ("/api/status", "/api/users", "/api/volumes", "/api/me", "/api/network"):
            st, obj, r = self.cl.raw("GET", path)
            self.assertEqual(st, 401, path)
            self.assertEqual(r.getheader("Cache-Control"), "no-store")
        self.assertEqual(self.cl.raw("POST", "/api/hostname", {"hostname": "x"})[0], 401)

    def test_public_fingerprint_endpoint(self):
        st, obj, _ = self.cl.raw("GET", "/api/tls-fingerprint")
        self.assertEqual(st, 200)
        self.assertIn("sha256", obj)

    def test_login_logout_cookie_flags(self):
        st, obj, r = self.cl.login()
        self.assertEqual(st, 200)
        sc = r.getheader("Set-Cookie")
        self.assertIn("HttpOnly", sc)
        self.assertIn("SameSite=Strict", sc)
        self.assertIn("Path=/", sc)
        self.assertEqual(self.cl.call("GET", "/api/me")[1]["user"], "admin")
        self.assertEqual(self.cl.call("POST", "/api/logout", {})[0], 200)
        self.assertEqual(self.cl.call("GET", "/api/me")[0], 401)

    def test_secure_cookie_name_with_tls(self):
        cfg = m.Config(root=self.env.tmp, tls=True)
        app = m.App(cfg, self.runner, auth=FakeAuth({}))
        self.assertEqual(app.cookie_name, "__Host-synofurb")

    def test_bad_login_and_lockout(self):
        for _ in range(5):
            st, obj, _ = self.cl.login("admin", "wrong")
            self.assertEqual(st, 401)
            self.assertEqual(obj["error"], "Invalid credentials")
        st, obj, r = self.cl.login("admin", "correct-horse")       # right password, but locked out now
        self.assertEqual(st, 429)
        self.assertIsNotNone(r.getheader("Retry-After"))
        audit = self.env.read("/var/log/synofurb-mgmt/audit.log")
        self.assertIn("bad password", audit)
        self.assertIn("locked out", audit)
        self.assertNotIn("wrong", audit.replace("bad password", ""))

    def test_csrf_enforced(self):
        self.logged_in()
        st, obj, _ = self.cl.call("POST", "/api/hostname", {"hostname": "newnas"}, csrf=False)
        self.assertEqual(st, 403)
        st, _, _ = self.cl.call("POST", "/api/hostname", {"hostname": "newnas"}, headers={"X-CSRF-Token": "x" * 43})
        self.assertEqual(st, 403)
        self.assertFalse(self.runner.called("hostnamectl"))
        st, _, _ = self.cl.call("POST", "/api/hostname", {"hostname": "newnas"})
        self.assertEqual(st, 200)
        self.assertTrue(self.runner.called("hostnamectl", "set-hostname", "newnas"))

    def test_origin_mismatch_rejected(self):
        self.logged_in()
        st, _, _ = self.cl.call("POST", "/api/hostname", {"hostname": "newnas"}, headers={"Origin": "https://evil.example"})
        self.assertEqual(st, 403)
        st, _, _ = self.cl.call("POST", "/api/hostname", {"hostname": "newnas"}, headers={"Origin": "null"})
        self.assertEqual(st, 403)
        origin = "http://127.0.0.1:%d" % self.env.port
        st, _, _ = self.cl.call("POST", "/api/hostname", {"hostname": "newnas"}, headers={"Origin": origin})
        self.assertEqual(st, 200)

    def test_body_limits_and_content_type(self):
        self.logged_in()
        big = json.dumps({"hostname": "a" * (64 * 1024 + 10)}).encode()
        st, obj, _ = self.cl.call("POST", "/api/hostname", data=big) if False else self.cl.raw(
            "POST", "/api/hostname", data=big, headers={"Cookie": self.cl.cookie, "X-CSRF-Token": self.cl.csrf})
        self.assertEqual(st, 413)
        st, _, _ = self.cl.call("POST", "/api/hostname", headers={"Content-Type": "text/plain"}, body={"hostname": "x"})
        self.assertEqual(st, 415)
        st, _, _ = self.cl.raw("POST", "/api/hostname", data=b"{not json", headers={"Cookie": self.cl.cookie, "X-CSRF-Token": self.cl.csrf})
        self.assertEqual(st, 400)
        st, _, _ = self.cl.raw("POST", "/api/hostname", data=b"[1,2]", headers={"Cookie": self.cl.cookie, "X-CSRF-Token": self.cl.csrf})
        self.assertEqual(st, 400)

    def test_unknown_route_and_method(self):
        self.assertEqual(self.cl.raw("GET", "/api/nope")[0], 404)
        self.assertEqual(self.cl.raw("PUT", "/api/login", {"a": 1})[0], 405)
        self.assertEqual(self.cl.raw("GET", "/api/login")[0], 405)

    def test_session_expiry_over_http(self):
        self.logged_in()
        for s in self.env.app.sessions._s.values():
            s.last_seen -= 3600
        self.assertEqual(self.cl.call("GET", "/api/me")[0], 401)

    def test_hostname_validation_and_audit(self):
        self.logged_in()
        st, _, _ = self.cl.call("POST", "/api/hostname", {"hostname": "bad host; reboot"})
        self.assertEqual(st, 400)
        self.cl.call("POST", "/api/hostname", {"hostname": "goodnas"})
        recs = [json.loads(x) for x in self.env.read("/var/log/synofurb-mgmt/audit.log").splitlines()]
        last = recs[-1]
        self.assertEqual((last["user"], last["action"], last["result"]), ("admin", "/api/hostname", "ok"))
        self.assertEqual(last["ip"], "127.0.0.1")


class HttpVolumeTests(HttpTestCase):
    def body(self, **kw):
        b = {"name": "media", "level": "raid1", "fs": "xfs", "devices": ["/dev/sda", "/dev/sdb"]}
        b.update(kw)
        return b

    def test_plan_dry_run_executes_nothing(self):
        self.logged_in()
        n = len(self.runner.calls)
        st, obj, _ = self.cl.call("POST", "/api/volumes/plan", self.body())
        self.assertEqual(st, 200, obj)
        self.assertEqual(obj["confirm"], "media")
        self.assertEqual(obj["mountpoint"], "/srv/media")
        cmds = [s["cmd"] for s in obj["steps"]]
        self.assertTrue(any(c.startswith("mdadm --create /dev/md/media") for c in cmds))
        self.assertTrue(any("mkfs.xfs" in c for c in cmds))
        self.assertEqual(obj["usable_bytes"], 4000787030016)
        for c in self.runner.calls[n:]:                              # only read-only inspection happened
            self.assertIn(c[0], ("lsblk", "vgs", "lvs", "pvs", "findmnt"))

    def test_plan_refuses_unsafe_devices(self):
        self.logged_in()
        for devs, frag in ((["/dev/nvme0n1", "/dev/sda"], "system"), (["/dev/nvme0n1p1", "/dev/sda"], "root/boot"),
                           (["/dev/sdc1", "/dev/sda"], "ntfs"), (["/dev/sdd", "/dev/sda"], "RAID member"),
                           (["/dev/sdc", "/dev/sda"], "in use"), (["/dev/sdq", "/dev/sda"], "no such")):
            st, obj, _ = self.cl.call("POST", "/api/volumes/plan", self.body(devices=devs))
            self.assertEqual(st, 400, devs)
            self.assertIn(frag, obj["error"])

    def test_plan_validation_errors(self):
        self.logged_in()
        for kw in ({"name": "x;reboot"}, {"devices": ["/dev/sda"]}, {"level": "raid5"}, {"fs": "ntfs"}, {"mountpoint": "/etc/x"},
                   {"devices": "/dev/sda"}, {"size_gib": 99999}, {"size_gib": "abc"}):
            st, obj, _ = self.cl.call("POST", "/api/volumes/plan", self.body(**kw))
            self.assertEqual(st, 400, kw)

    def test_create_requires_matching_confirm(self):
        self.logged_in()
        for conf in (None, "", "other", "MEDIA"):
            b = self.body()
            if conf is not None:
                b["confirm"] = conf
            st, _, _ = self.cl.call("POST", "/api/volumes/create", b)
            self.assertEqual(st, 400)
        self.assertFalse(self.runner.called("mdadm", "--create", "/dev/md/media", "--run", "--level=raid1", "--raid-devices=2",
                                            "--metadata=1.2", "--name=media", "/dev/sda", "/dev/sdb"))
        self.assertFalse(self.runner.called("wipefs"))

    def test_create_executes_plan(self):
        self.logged_in()
        st, obj, _ = self.cl.call("POST", "/api/volumes/create", self.body(confirm="media"))
        self.assertEqual(st, 200, obj)
        self.assertTrue(all(r["rc"] == 0 for r in obj["results"]))
        self.assertTrue(self.runner.called("wipefs", "-a", "/dev/sda"))
        self.assertTrue(self.runner.called("mkfs.xfs", "-f", "-L", "media", "/dev/vg_media/media"))
        self.assertTrue(self.runner.called("mount", "/srv/media"))
        fstab = self.env.read("/etc/fstab")
        self.assertIn("UUID=11111111-2222-4333-8444-555555555555 /srv/media xfs defaults,nofail,x-systemd.device-timeout=30 0 0", fstab)
        self.assertIn("ARRAY /dev/md/media", self.env.read("/etc/mdadm/mdadm.conf"))
        self.assertTrue(os.path.isdir(self.env.cfg.p("/srv/media")))
        # idempotent config edit: creating the same line again must not duplicate it
        step = m.Step("x", kind="mdconf", argv=["mdadm", "--detail", "--scan", "/dev/md/media"])
        m.exec_step(self.env.app, step)
        self.assertEqual(self.env.read("/etc/mdadm/mdadm.conf").count("ARRAY"), 1)
        audit = self.env.read("/var/log/synofurb-mgmt/audit.log")
        self.assertIn("/api/volumes/create", audit)

    def test_create_stops_at_first_failure(self):
        self.runner.on(["mkfs.xfs"], "", rc=1, err="boom")
        self.logged_in()
        st, obj, _ = self.cl.call("POST", "/api/volumes/create", self.body(confirm="media"))
        self.assertEqual(st, 500)
        self.assertEqual(obj["results"][-1]["err"], "boom")
        self.assertFalse(self.runner.called("mount", "/srv/media"))
        self.assertNotIn("media", self.env.read("/etc/fstab"))

    def test_existing_vg_conflict(self):
        self.runner.on(["vgs"], json.dumps({"report": [{"vg": [{"vg_name": "vg_media", "vg_size": "1", "vg_free": "0"}]}]}))
        self.logged_in()
        self.assertEqual(self.cl.call("POST", "/api/volumes/plan", self.body())[0], 409)

    def test_extend_and_delete(self):
        lvs = json.dumps({"report": [{"lv": [{"lv_name": "media", "vg_name": "vg_media", "lv_size": "100", "lv_path": "/dev/vg_media/media", "lv_attr": "-wi-ao"},
                                              {"lv_name": "root", "vg_name": "vg_sys", "lv_size": "100", "lv_path": "/dev/vg_sys/root", "lv_attr": "x"}]}]})
        vgs = json.dumps({"report": [{"vg": [{"vg_name": "vg_media", "vg_size": "%d" % (10 * m.GIB), "vg_free": "%d" % (4 * m.GIB)}]}]})
        ls = json.loads(LSBLK_TEXT)
        ls["blockdevices"].append({"name": "vg_media-media", "kname": "dm-5", "type": "lvm", "size": 100, "fstype": "xfs", "uuid": "u",
                                   "label": "media", "mountpoints": ["/srv/media"], "pkname": None})
        self.runner.on(["lvs"], lvs).on(["vgs"], vgs).on(["lsblk"], json.dumps(ls))
        self.logged_in()
        st, obj, _ = self.cl.call("POST", "/api/volumes/extend", {"vg": "vg_media", "lv": "media", "add_gib": 10})
        self.assertEqual(st, 400)                                   # more than free
        st, obj, _ = self.cl.call("POST", "/api/volumes/extend", {"vg": "vg_media", "lv": "media", "add_gib": None})
        self.assertEqual(st, 200, obj)
        self.assertTrue(self.runner.called("lvextend", "-l", "+100%FREE", "/dev/vg_media/media"))
        self.assertTrue(self.runner.called("xfs_growfs", "/srv/media"))
        self.assertEqual(self.cl.call("POST", "/api/volumes/extend", {"vg": "vg_nope", "lv": "x"})[0], 404)
        # delete needs a typed name and refuses system volumes
        self.assertEqual(self.cl.call("POST", "/api/volumes/delete", {"vg": "vg_media", "lv": "media", "confirm": "nope"})[0], 400)
        self.assertEqual(self.cl.call("POST", "/api/volumes/delete", {"vg": "vg_sys", "lv": "root", "confirm": "root"})[0], 400)
        self.assertFalse(self.runner.called("lvremove"))
        st, obj, _ = self.cl.call("POST", "/api/volumes/delete", {"vg": "vg_media", "lv": "media", "confirm": "media"})
        self.assertEqual(st, 200, obj)
        self.assertTrue(self.runner.called("umount", "/srv/media"))
        self.assertTrue(self.runner.called("lvremove", "--yes", "/dev/vg_media/media"))


class HttpMountTests(HttpTestCase):
    def test_list_and_mount_ntfs_ro_persist(self):
        self.logged_in()
        st, obj, _ = self.cl.call("GET", "/api/mounts")
        self.assertEqual(st, 200)
        dev = next(d for d in obj["devices"] if d["path"] == "/dev/sdc1")
        self.assertTrue(dev["foreign"] and dev["ro_default"])
        self.assertEqual(dev["suggested"], "/mnt/Backup")
        st, obj, _ = self.cl.call("POST", "/api/mounts/mount", {"device": "/dev/sdc1", "mountpoint": "/mnt/usb", "ro": True, "persist": True})
        self.assertEqual(st, 200, obj)
        fstab = self.env.read("/etc/fstab")
        self.assertIn("UUID=1234567890ABCDEF /mnt/usb ntfs-3g defaults,nofail,uid=1000,gid=1000,umask=0022,ro,x-systemd.device-timeout=30 0 0", fstab)
        self.assertTrue(self.runner.called("mount", "/mnt/usb"))

    def test_mount_not_persistent(self):
        self.logged_in()
        st, obj, _ = self.cl.call("POST", "/api/mounts/mount", {"device": "/dev/sdc1", "mountpoint": "/mnt/usb"})
        self.assertEqual(st, 200, obj)
        # an imported (foreign) volume is read-only unless the caller explicitly says ro=false
        self.assertTrue(self.runner.called("mount", "-t", "ntfs-3g", "-o", "defaults,nofail,uid=1000,gid=1000,umask=0022,ro", "/dev/sdc1", "/mnt/usb"))
        self.assertEqual(self.env.read("/etc/fstab"), "")

    def test_mount_rejections(self):
        self.logged_in()
        for body in ({"device": "/dev/sdc1", "mountpoint": "/etc"}, {"device": "/dev/sdc1", "mountpoint": "/srv/../etc"},
                     {"device": "/dev/sda", "mountpoint": "/mnt/x"},           # no filesystem
                     {"device": "/dev/nvme0n1p1", "mountpoint": "/mnt/x"},     # already mounted
                     {"device": "/dev/sdz9", "mountpoint": "/mnt/x"}, {"device": "sdc1", "mountpoint": "/mnt/x"}):
            st, _, _ = self.cl.call("POST", "/api/mounts/mount", body)
            self.assertIn(st, (400, 404, 409), body)
        self.assertFalse(self.runner.called("mount"))
        os.makedirs(self.env.cfg.p("/mnt"))
        os.symlink("/etc", self.env.cfg.p("/mnt/link"))
        st, _, _ = self.cl.call("POST", "/api/mounts/mount", {"device": "/dev/sdc1", "mountpoint": "/mnt/link/x"})
        self.assertEqual(st, 400)

    def test_unmount_requires_confirm(self):
        self.runner.on(["findmnt"], json.dumps({"filesystems": [{"target": "/mnt/usb", "source": "/dev/sdc1", "fstype": "ntfs3"}]}))
        self.logged_in()
        self.assertEqual(self.cl.call("POST", "/api/mounts/unmount", {"mountpoint": "/mnt/usb"})[0], 400)
        self.assertEqual(self.cl.call("POST", "/api/mounts/unmount", {"mountpoint": "/mnt/usb", "confirm": "/mnt/other"})[0], 400)
        self.assertFalse(self.runner.called("umount"))
        self.assertEqual(self.cl.call("POST", "/api/mounts/unmount", {"mountpoint": "/mnt/usb", "confirm": "/mnt/usb"})[0], 200)
        self.assertTrue(self.runner.called("umount", "/mnt/usb"))
        self.assertEqual(self.cl.call("POST", "/api/mounts/unmount", {"mountpoint": "/mnt/gone", "confirm": "/mnt/gone"})[0], 404)

    def test_activate_runs_scans(self):
        self.logged_in()
        st, obj, _ = self.cl.call("POST", "/api/mounts/activate", {})
        self.assertEqual(st, 200)
        self.assertTrue(self.runner.called("mdadm", "--assemble", "--scan"))
        self.assertTrue(self.runner.called("vgscan"))
        self.assertTrue(self.runner.called("vgchange", "-ay"))
        self.assertTrue(obj["devices"])


class HttpImportedTests(HttpTestCase):
    """Imported-volumes behaviour: read-only default for foreign volumes, writable default for our own."""

    def setUp(self):
        super().setUp()
        ls = json.loads(LSBLK_TEXT)
        ls["blockdevices"] += [
            {"name": "vg1000-lv", "kname": "dm-7", "type": "lvm", "size": 5 * 1024 ** 4, "fstype": "btrfs", "uuid": "bbbb-cccc", "label": "1.42.6",
             "mountpoints": [None], "pkname": None},
            {"name": "vg_media-media", "kname": "dm-8", "type": "lvm", "size": 1024 ** 4, "fstype": "xfs", "uuid": "dddd-eeee", "label": "media",
             "mountpoints": [None], "pkname": None},
            {"name": "sdx1", "kname": "sdx1", "type": "part", "size": 10 ** 10, "fstype": "crypto_LUKS", "uuid": "l", "mountpoints": [None], "pkname": None}]
        self.runner.on(["lsblk"], json.dumps(ls))
        self.logged_in()

    def mount_body(self, device, mp, **kw):
        return dict({"device": device, "mountpoint": mp}, **kw)

    def test_listing_flags_foreign_and_unsupported(self):
        st, obj, _ = self.cl.call("GET", "/api/mounts")
        devs = {d["path"]: d for d in obj["devices"]}
        self.assertTrue(devs["/dev/mapper/vg1000-lv"]["foreign"] and devs["/dev/mapper/vg1000-lv"]["ro_default"])
        self.assertFalse(devs["/dev/mapper/vg_media-media"]["foreign"])
        self.assertFalse(devs["/dev/mapper/vg_media-media"]["ro_default"])
        self.assertIn("/dev/sdx1", {o["path"]: o for o in obj["other"]})
        self.assertIn("/dev/sdd", {o["path"]: o for o in obj["other"]})              # raid member: told to use Activate

    def test_foreign_defaults_to_read_only(self):
        st, obj, _ = self.cl.call("POST", "/api/mounts/mount", self.mount_body("/dev/mapper/vg1000-lv", "/mnt/syno"))
        self.assertEqual(st, 200, obj)
        self.assertTrue(self.runner.called("mount", "-t", "btrfs", "-o", "defaults,nofail,ro", "/dev/mapper/vg1000-lv", "/mnt/syno"))

    def test_foreign_read_write_only_when_explicit(self):
        st, _, _ = self.cl.call("POST", "/api/mounts/mount", self.mount_body("/dev/mapper/vg1000-lv", "/mnt/syno", ro=False))
        self.assertEqual(st, 200)
        self.assertTrue(self.runner.called("mount", "-t", "btrfs", "-o", "defaults,nofail", "/dev/mapper/vg1000-lv", "/mnt/syno"))

    def test_own_volume_defaults_to_read_write(self):
        st, _, _ = self.cl.call("POST", "/api/mounts/mount", self.mount_body("/dev/mapper/vg_media-media", "/srv/media"))
        self.assertEqual(st, 200)
        self.assertTrue(self.runner.called("mount", "-t", "xfs", "-o", "defaults,nofail", "/dev/mapper/vg_media-media", "/srv/media"))

    def test_persisted_read_only_entry(self):
        st, _, _ = self.cl.call("POST", "/api/mounts/mount", self.mount_body("/dev/mapper/vg1000-lv", "/mnt/syno", persist=True))
        self.assertEqual(st, 200)
        self.assertIn("UUID=bbbb-cccc /mnt/syno btrfs defaults,nofail,ro,x-systemd.device-timeout=30 0 0", self.env.read("/etc/fstab"))

    def test_activate_reports_arrays_and_devices(self):
        st, obj, _ = self.cl.call("POST", "/api/mounts/activate", {})
        self.assertEqual(st, 200)
        for k in ("devices", "other", "md", "lvs", "results"):
            self.assertIn(k, obj)


class HttpUserTests(HttpTestCase):
    def test_create_user_password_via_stdin_only(self):
        self.logged_in()
        key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl me@x"
        st, obj, _ = self.cl.call("POST", "/api/users/create", {"username": "dave", "password": "Sup3rSecret!x", "groups": "users, adm",
                                                                 "sudo": True, "ssh_key": key})
        self.assertEqual(st, 200, obj)
        add = next(c for c in self.runner.calls if c[0] == "useradd")
        self.assertEqual(add[-1], "dave")
        self.assertIn("users,adm,sudo", add)
        for c in self.runner.calls:
            self.assertNotIn("Sup3rSecret!x", " ".join(c))
        i = next(i for i, c in enumerate(self.runner.calls) if c == ["chpasswd"])
        self.assertEqual(self.runner.inputs[i], "dave:Sup3rSecret!x\n")
        self.assertNotIn("Sup3rSecret", self.env.read("/var/log/synofurb-mgmt/audit.log"))
        ak = self.env.cfg.p("/home/dave/.ssh/authorized_keys")
        with open(ak) as fh:
            self.assertIn(key, fh.read())
        self.assertEqual(os.stat(ak).st_mode & 0o777, 0o600)
        self.assertEqual(os.stat(os.path.dirname(ak)).st_mode & 0o777, 0o700)

    def test_create_rejections(self):
        self.logged_in()
        for body in ({"username": "Bad Name", "password": "longenough1"}, {"username": "dave", "password": "short"},
                     {"username": "admin", "password": "longenough1"}, {"username": "dave", "password": "longenough1", "groups": "root"},
                     {"username": "dave", "password": "longenough1", "groups": "nonexistentgrp"},
                     {"username": "dave", "password": "longenough1", "ssh_key": "command=\"x\" ssh-rsa AAAA"}):
            st, _, _ = self.cl.call("POST", "/api/users/create", body)
            self.assertIn(st, (400, 409), body)
        self.assertFalse(self.runner.called("useradd"))

    def test_delete_safeguards(self):
        self.logged_in()
        d = lambda **kw: self.cl.call("POST", "/api/users/delete", kw)         # noqa: E731
        self.assertEqual(d(username="root", confirm="root")[0], 400)
        self.assertEqual(d(username="bob")[0], 400)                          # missing confirm
        self.assertEqual(d(username="admin", confirm="admin")[0], 400)       # yourself
        self.assertEqual(d(username="ghost", confirm="ghost")[0], 404)
        self.assertFalse(self.runner.called("userdel"))
        # make bob the second admin, then add a third logged-in actor so "last sudo member" is the blocker
        self.env.write("/etc/group", "root:x:0:\nsudo:x:27:bob\nusers:x:100:\nadm:x:4:\nadmin:x:1000:\nbob:x:1001:\n")
        st, obj, _ = d(username="bob", confirm="bob")
        self.assertEqual(st, 400)
        self.assertIn("last member", obj["error"])
        self.env.write("/etc/group", "root:x:0:\nsudo:x:27:admin,bob\nusers:x:100:\nadm:x:4:\nadmin:x:1000:\nbob:x:1001:\n")
        self.assertEqual(d(username="bob", confirm="bob", remove_home=True)[0], 200)
        self.assertTrue(self.runner.called("userdel", "--remove", "bob"))

    def test_lock_unlock_and_password(self):
        self.logged_in()
        self.assertEqual(self.cl.call("POST", "/api/users/lock", {"username": "admin"})[0], 400)       # self
        self.assertEqual(self.cl.call("POST", "/api/users/lock", {"username": "bob"})[0], 200)
        self.assertTrue(self.runner.called("usermod", "-L", "bob"))
        self.assertEqual(self.cl.call("POST", "/api/users/unlock", {"username": "bob"})[0], 200)
        self.assertTrue(self.runner.called("usermod", "-U", "bob"))
        st, _, _ = self.cl.call("POST", "/api/users/password", {"username": "bob", "password": "n3w-passw0rd"})
        self.assertEqual(st, 200)
        i = [k for k, c in enumerate(self.runner.calls) if c == ["chpasswd"]][-1]
        self.assertEqual(self.runner.inputs[i], "bob:n3w-passw0rd\n")
        self.assertEqual(self.cl.call("POST", "/api/users/password", {"username": "root", "password": "n3w-passw0rd"})[0], 400)

    def test_list_users(self):
        self.logged_in()
        st, obj, _ = self.cl.call("GET", "/api/users")
        self.assertEqual(st, 200)
        names = {u["name"]: u for u in obj["users"]}
        self.assertEqual(set(names), {"root", "admin", "bob"})
        self.assertTrue(names["root"]["root"])
        self.assertTrue(names["admin"]["sudo"])
        self.assertFalse(names["bob"]["sudo"])

    def test_add_ssh_key_dedup(self):
        self.logged_in()
        key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl"
        for _ in range(2):
            self.assertEqual(self.cl.call("POST", "/api/users/ssh-key", {"username": "bob", "ssh_key": key})[0], 200)
        with open(self.env.cfg.p("/home/bob/.ssh/authorized_keys")) as fh:
            self.assertEqual(fh.read().count("AAAAC3"), 1)
        self.assertEqual(self.cl.call("POST", "/api/users/ssh-key", {"username": "bob", "ssh_key": "nope"})[0], 400)


class HttpNetworkTests(HttpTestCase):
    STATIC = {"ifname": "eth0", "mode": "static", "ipv4": "192.168.1.50/24", "gateway4": "192.168.1.1",
              "dns": "1.1.1.1, 9.9.9.9", "search": "lan"}
    NETFILE = "/etc/systemd/network/10-synofurb-eth0.network"

    def test_apply_arms_rollback_then_confirm(self):
        self.logged_in()
        st, obj, _ = self.cl.call("POST", "/api/network/apply", self.STATIC)
        self.assertEqual(st, 200, obj)
        text = self.env.read(self.NETFILE)
        self.assertIn("Address=192.168.1.50/24", text)
        self.assertIn("Name=eth0", text)
        calls = self.runner.calls
        sr = next(c for c in calls if c[0] == "systemd-run")
        self.assertIn("--on-active=90", sr)
        self.assertIn("--unit=synofurb-net-rollback", sr)
        self.assertIn("--net-rollback", sr)
        i_sr = calls.index(sr)
        i_reload = calls.index(["networkctl", "reload"])
        self.assertLess(i_sr, i_reload)                          # timer armed before the link is touched
        self.assertTrue(self.runner.called("networkctl", "reconfigure", "eth0"))
        self.assertTrue(0 < obj["pending"]["seconds_left"] <= 90)
        self.assertEqual(self.cl.call("GET", "/api/me")[1]["net_pending"]["ifname"], "eth0")
        # a second change is refused while one is pending
        self.assertEqual(self.cl.call("POST", "/api/network/apply", self.STATIC)[0], 409)
        # confirm stops the timer and clears the pending state
        st, _, _ = self.cl.call("POST", "/api/network/confirm", {})
        self.assertEqual(st, 200)
        self.assertTrue(self.runner.called("systemctl", "stop", "synofurb-net-rollback.timer"))
        self.assertIsNone(self.cl.call("GET", "/api/me")[1]["net_pending"])
        self.assertEqual(self.cl.call("POST", "/api/network/confirm", {})[0], 409)

    def test_rollback_restores_previous_file_and_parked_files(self):
        original = "# original\n[Match]\nName=eth0\n[Network]\nDHCP=yes\n"
        self.env.write(self.NETFILE, original)
        self.env.write("/etc/systemd/network/05-installer.network", "[Match]\nName=e*\n[Network]\nDHCP=yes\n")
        self.logged_in()
        self.assertEqual(self.cl.call("POST", "/api/network/apply", self.STATIC)[0], 200)
        self.assertFalse(os.path.exists(self.env.cfg.p("/etc/systemd/network/05-installer.network")))     # parked
        self.assertTrue(os.path.exists(self.env.cfg.p("/etc/systemd/network/05-installer.network" + m.DISABLED_SUFFIX)))
        self.assertIn("192.168.1.50", self.env.read(self.NETFILE))
        self.assertTrue(m.rollback_network(self.env.app))          # what the timer's --net-rollback does
        self.assertEqual(self.env.read(self.NETFILE), original)
        self.assertTrue(os.path.exists(self.env.cfg.p("/etc/systemd/network/05-installer.network")))
        self.assertIsNone(m.load_pending(self.env.cfg))
        self.assertFalse(m.rollback_network(self.env.app))         # nothing left to do

    def test_rollback_removes_new_file_when_none_existed(self):
        self.logged_in()
        self.cl.call("POST", "/api/network/apply", {"ifname": "eth0", "mode": "dhcp"})
        self.assertIn("IPv6AcceptRA=yes", self.env.read(self.NETFILE))
        st, _, _ = self.cl.call("POST", "/api/network/rollback", {})
        self.assertEqual(st, 200)
        self.assertFalse(os.path.exists(self.env.cfg.p(self.NETFILE)))

    def test_timer_failure_reverts(self):
        self.runner.on(["systemd-run"], "", rc=1, err="no systemd")
        self.logged_in()
        st, obj, _ = self.cl.call("POST", "/api/network/apply", self.STATIC)
        self.assertEqual(st, 500)
        self.assertFalse(os.path.exists(self.env.cfg.p(self.NETFILE)))
        self.assertIsNone(m.load_pending(self.env.cfg))
        first_net = next(i for i, c in enumerate(self.runner.calls) if c[0] == "networkctl")
        self.assertLess(next(i for i, c in enumerate(self.runner.calls) if c[0] == "systemd-run"), first_net)
        self.assertEqual(sum(1 for c in self.runner.calls if c[:2] == ["networkctl", "reconfigure"]), 1)   # only the revert

    def test_startup_recovery_of_expired_pending(self):
        self.logged_in()
        self.cl.call("POST", "/api/network/apply", self.STATIC)
        p = m.load_pending(self.env.cfg)
        p["deadline"] = 1.0
        m.write_atomic(m.pending_path(self.env.cfg), json.dumps(p))
        m.startup_recovery(self.env.app)
        self.assertIsNone(m.load_pending(self.env.cfg))
        self.assertFalse(os.path.exists(self.env.cfg.p(self.NETFILE)))

    def test_validation(self):
        self.logged_in()
        bad = [dict(self.STATIC, ifname="eth9"), dict(self.STATIC, ifname="lo"), dict(self.STATIC, ifname="eth0;reboot"),
               dict(self.STATIC, gateway4="10.9.9.9"), dict(self.STATIC, ipv4="192.168.1.50"), dict(self.STATIC, dns="1.1.1.1\nDNS=6.6.6.6"),
               dict(self.STATIC, mode="bridge"), dict(self.STATIC, ipv6="fe80::1/64"), dict(self.STATIC, search="bad_domain!")]
        for b in bad:
            st, _, _ = self.cl.call("POST", "/api/network/apply", b)
            self.assertEqual(st, 400, b)
        self.assertFalse(os.path.exists(self.env.cfg.p(self.NETFILE)))
        self.assertFalse(self.runner.called("systemd-run"))

    def test_network_listing(self):
        self.logged_in()
        st, obj, _ = self.cl.call("GET", "/api/network")
        self.assertEqual([i["ifname"] for i in obj["interfaces"]], ["eth0"])
        self.assertFalse(obj["interfaces"][0]["managed"])


class HttpSystemTests(HttpTestCase):
    def test_power_requires_hostname_confirm(self):
        self.logged_in()
        self.assertEqual(self.cl.call("POST", "/api/system/power", {"action": "reboot"})[0], 400)
        self.assertEqual(self.cl.call("POST", "/api/system/power", {"action": "reboot", "confirm": "wrong"})[0], 400)
        self.assertEqual(self.cl.call("POST", "/api/system/power", {"action": "halt", "confirm": m.socket.gethostname()})[0], 400)
        self.assertFalse(self.runner.called("systemctl", "reboot"))
        self.assertEqual(self.cl.call("POST", "/api/system/power", {"action": "reboot", "confirm": m.socket.gethostname()})[0], 200)
        for _ in range(50):
            if self.runner.called("systemctl", "reboot"):
                break
            threading.Event().wait(0.05)
        self.assertTrue(self.runner.called("systemctl", "reboot"))

    def test_restart_allow_list(self):
        self.logged_in()
        self.assertEqual(self.cl.call("POST", "/api/services/restart", {"unit": "networking; reboot"})[0], 400)
        self.assertEqual(self.cl.call("POST", "/api/services/restart", {"unit": "forgevault"})[0], 400)
        self.assertEqual(self.cl.call("POST", "/api/services/restart", {"unit": "ssh"})[0], 200)

    def test_status_services_smart(self):
        self.runner.on(["smartctl"], json.dumps({"smart_status": {"passed": True}, "temperature": {"current": 31}}))
        self.runner.on(["systemctl", "list-units"], "serial-getty@ttyS0.service loaded active running Serial Getty on ttyS0\n")
        self.runner.on(["systemctl", "is-enabled"], "enabled\n").on(["systemctl", "is-active"], "active\n")
        self.logged_in()
        st, obj, _ = self.cl.call("GET", "/api/status")
        self.assertEqual(st, 200)
        for k in ("hostname", "kernel", "arch", "disks", "md", "lvm", "mounts", "interfaces", "memory"):
            self.assertIn(k, obj)
        self.assertEqual(obj["interfaces"][1]["addrs"][0]["addr"], "192.168.1.5")
        self.assertFalse(self.runner.called("smartctl"))             # never during status
        st, obj, _ = self.cl.call("GET", "/api/smart")
        self.assertTrue(obj["available"])
        self.assertEqual({d["device"] for d in obj["disks"]}, {"/dev/nvme0n1", "/dev/sda", "/dev/sdb", "/dev/sdc", "/dev/sdd"})
        self.assertTrue(all(d["passed"] is True for d in obj["disks"]))
        st, obj, _ = self.cl.call("GET", "/api/services")
        self.assertEqual(obj["sshd"], {"enabled": "enabled", "active": "active"})
        self.assertEqual(obj["serial_getty"][0]["unit"], "serial-getty@ttyS0.service")

    def test_smart_unavailable(self):
        self.runner.avail.discard("smartctl")
        self.logged_in()
        self.assertEqual(self.cl.call("GET", "/api/smart")[1], {"ok": True, "available": False, "disks": []})


DOM_STATUS = {
    "tool": "synofurb-dom", "version": "0.4.1", "device": "/dev/synoboot", "device_bytes": 125829120,
    "p1": {"dev": "/dev/synoboot1", "bytes": 52428800, "free_bytes": 31457280}, "p2": {"dev": "/dev/synoboot2", "bytes": 73400320, "free_bytes": 9437184},
    "entries": [{"index": 0, "name": "DEBIAN", "ours": True, "dsm": False}, {"index": 1, "name": "DEBIAN_OLD", "ours": True, "dsm": False},
                {"index": 2, "name": "SYNOLOGY_2", "ours": False, "dsm": True}],
    "default": {"raw": "0", "index": 0, "name": "DEBIAN"}, "files": [], "efi_files": [],
    "backup": {"exists": True, "path": "/srv/data/synofurb/synofurb-dom-first-backup.img"}}
DOM = "/usr/local/sbin/synofurb-dom"


class BootViewTests(unittest.TestCase):
    def test_entries_mapped_to_stable_ids(self):
        v = m.boot_view(DOM_STATUS)
        by = {e["id"]: e for e in v["entries"]}
        self.assertEqual([e["id"] for e in v["entries"]], ["dsm", "debian", "debian-old", "agent"])
        self.assertTrue(by["dsm"]["present"] and by["dsm"]["failsafe"] and by["dsm"]["name"] == "SYNOLOGY_2")
        self.assertTrue(by["debian"]["default"] and not by["dsm"]["default"])
        self.assertFalse(by["agent"]["present"])
        self.assertEqual(v["default_id"], "debian")
        self.assertEqual(v["flash"]["p1"]["free_bytes"], 31457280)
        self.assertEqual(v["backup"], {"exists": True, "path": "/srv/data/synofurb/synofurb-dom-first-backup.img"})
        self.assertEqual(v["other_entries"], [])
        self.assertEqual([e["needs_confirm"] for e in v["entries"]], [True, False, True, True])

    def test_invalid_default_and_foreign_entries(self):
        st = dict(DOM_STATUS, default={"raw": "9", "index": None, "name": ""}, entries=DOM_STATUS["entries"] + [{"index": 3, "name": "EFI_x", "ours": True, "dsm": False}])
        v = m.boot_view(st)
        self.assertFalse(v["default_valid"])
        self.assertIsNone(v["default_id"])
        self.assertEqual(v["other_entries"], ["EFI_x"])


class HttpBootTests(HttpTestCase):
    def setUp(self):
        super().setUp()
        self.runner.on([DOM, "status", "--json"], json.dumps(DOM_STATUS))
        self.logged_in()

    def install_tool(self):
        self.env.write(DOM, "#!/bin/sh\n")

    def test_not_a_boot_flash_unit(self):
        st, obj, _ = self.cl.call("GET", "/api/boot")
        self.assertEqual(st, 200)
        self.assertFalse(obj["available"])
        self.assertIn("not a boot-flash unit", obj["message"])
        self.assertFalse(self.runner.called(DOM))
        self.assertEqual(self.cl.call("POST", "/api/boot/default", {"entry": "dsm", "confirm": "dsm"})[0], 409)

    def test_status_when_tool_present(self):
        self.install_tool()
        st, obj, _ = self.cl.call("GET", "/api/boot")
        self.assertTrue(obj["available"] and obj["default_id"] == "debian")
        self.assertTrue(self.runner.called(DOM, "status", "--json"))
        self.assertEqual(self.cl.call("GET", "/api/status")[1]["boot_flash"], True)

    def test_status_errors_are_reported_not_raised(self):
        self.install_tool()
        self.runner.on([DOM, "status"], "", rc=1, err="no Synology DOM found")
        st, obj, _ = self.cl.call("GET", "/api/boot")
        self.assertEqual(st, 200)
        self.assertIn("no Synology DOM found", obj["error"])
        self.runner.on([DOM, "status"], "not json")
        self.assertIn("unreadable", self.cl.call("GET", "/api/boot")[1]["error"])

    def test_default_needs_typed_name_except_debian(self):
        self.install_tool()
        for body in ({"entry": "dsm"}, {"entry": "dsm", "confirm": "debian"}, {"entry": "dsm", "confirm": "DSM"}, {"entry": "debian-old", "confirm": ""}):
            self.assertEqual(self.cl.call("POST", "/api/boot/default", body)[0], 400, body)
        self.assertFalse(self.runner.called(DOM, "default"))
        st, obj, _ = self.cl.call("POST", "/api/boot/default", {"entry": "dsm", "confirm": "dsm"})
        self.assertEqual(st, 200, obj)
        self.assertTrue(self.runner.called(DOM, "default", "dsm", "--yes"))
        st, _, _ = self.cl.call("POST", "/api/boot/default", {"entry": "debian"})           # no confirm needed for the normal system
        self.assertEqual(st, 200)
        self.assertTrue(self.runner.called(DOM, "default", "debian", "--yes"))

    def test_rejections(self):
        self.install_tool()
        for body in ({"entry": "grub"}, {"entry": "efi:x", "confirm": "efi:x"}, {"entry": "dsm; reboot", "confirm": "dsm; reboot"}, {}):
            self.assertEqual(self.cl.call("POST", "/api/boot/default", body)[0], 400, body)
        st, obj, _ = self.cl.call("POST", "/api/boot/default", {"entry": "agent", "confirm": "agent"})       # not installed on this flash
        self.assertEqual(st, 404)
        self.assertFalse(self.runner.called(DOM, "default"))

    def test_default_failure_is_an_error(self):
        self.install_tool()
        self.runner.on([DOM, "default"], "", rc=1, err="menu entry DEBIAN_OLD does not exist")
        st, obj, _ = self.cl.call("POST", "/api/boot/default", {"entry": "debian-old", "confirm": "debian-old"})
        self.assertEqual(st, 500)
        self.assertIn("does not exist", obj["error"])

    def test_reboot_to_sets_default_then_schedules_reboot(self):
        self.install_tool()
        self.assertEqual(self.cl.call("POST", "/api/boot/reboot-to", {"entry": "dsm"})[0], 400)
        self.assertFalse(self.runner.called("systemd-run"))
        st, obj, _ = self.cl.call("POST", "/api/boot/reboot-to", {"entry": "dsm", "confirm": "dsm"})
        self.assertEqual(st, 200, obj)
        calls = self.runner.calls
        i_def = calls.index([DOM, "default", "dsm", "--yes"])
        i_rb = calls.index(["systemd-run", "--on-active=5", "systemctl", "reboot"])
        self.assertLess(i_def, i_rb)
        audit = self.env.read("/var/log/synofurb-mgmt/audit.log")
        self.assertIn("/api/boot/reboot-to", audit)

    def test_no_reboot_when_default_fails(self):
        self.install_tool()
        self.runner.on([DOM, "default"], "", rc=1, err="refusing")
        self.assertEqual(self.cl.call("POST", "/api/boot/reboot-to", {"entry": "debian"})[0], 500)
        self.assertFalse(self.runner.called("systemd-run"))

    def test_reboot_schedule_failure_reported(self):
        self.install_tool()
        self.runner.on(["systemd-run"], "", rc=1, err="no bus")
        st, obj, _ = self.cl.call("POST", "/api/boot/reboot-to", {"entry": "debian"})
        self.assertEqual(st, 500)
        self.assertIn("could not be scheduled", obj["error"])


CPUINFO_INTEL = ("processor\t: 0\nvendor_id\t: GenuineIntel\nmodel name\t: Intel(R) Celeron(R) CPU J3455 @ 1.50GHz\nmicrocode\t: 0x28\n\n"
                 "processor\t: 1\nvendor_id\t: GenuineIntel\nmodel name\t: Intel(R) Celeron(R) CPU J3455 @ 1.50GHz\nmicrocode\t: 0x28\n")
CPUINFO_AMD = "processor : 0\nvendor_id : AuthenticAMD\nmodel name : AMD Ryzen Embedded V1500B\nmicrocode : 0x8101016\n"
CPUINFO_ARM = "processor : 0\nBogoMIPS : 50.00\nprocessor : 1\nHardware : Marvell Armada 385\n"


class StatusInfoTests(unittest.TestCase):
    def test_cpuinfo_vendor(self):
        i = m.parse_cpuinfo(CPUINFO_INTEL)
        self.assertEqual((i["vendor"], i["threads"], i["microcode_revision"]), ("Intel", 2, "0x28"))
        self.assertIn("J3455", i["model"])
        self.assertEqual(m.parse_cpuinfo(CPUINFO_AMD)["vendor"], "AMD")
        a = m.parse_cpuinfo(CPUINFO_ARM)
        self.assertIsNone(a["vendor"])
        self.assertEqual((a["model"], a["threads"]), ("Marvell Armada 385", 2))
        self.assertEqual(m.parse_cpuinfo("")["threads"], None)

    def test_kernel_flag(self):
        for rel, ge7 in (("6.12.48+deb13-amd64", False), ("6.19.1", False), ("7.0.4-amd64", True), ("7.1", True), ("10.0.0", True), ("garbage", False), ("", False)):
            self.assertEqual(m.kernel_info(rel)["ge7"], ge7, rel)
        k = m.kernel_info("7.0.4-amd64")
        self.assertEqual((k["major"], k["minor"]), (7, 0))

    def test_console_parsing(self):
        c = m.parse_cmdline_consoles("BOOT_IMAGE=/vmlinuz root=/dev/x ro console=tty0 console=ttyS0,115200n8 quiet")
        self.assertEqual([x["device"] for x in c], ["tty0", "ttyS0"])
        self.assertEqual((c[1]["options"], c[1]["serial"], c[0]["serial"]), ("115200n8", True, False))
        self.assertEqual(m.parse_cmdline_consoles("ro quiet console="), [])
        self.assertEqual(m.parse_cmdline_consoles("ro quiet"), [])

    def test_dsm_inspect_parsing_whitelists_keys(self):
        d = m.parse_dsm_inspect("KV|model|DS918+\nKV|serial|SECRET123\nKV|dsm_version|7.1.1-42962\nNIC|eth0|aa:bb\nKV|cpu|Celeron J3455\nKV|hw_version|\njunk\n")
        self.assertEqual(d, {"model": "DS918+", "dsm_version": "7.1.1-42962", "cpu": "Celeron J3455"})
        self.assertNotIn("serial", d)

    def test_unit_info_from_inspect_file(self):
        with tempfile.TemporaryDirectory() as t:
            cfg = m.Config(root=t)
            self.assertIsNone(m.unit_info(cfg)["model"])
            os.makedirs(cfg.p("/var/lib/synofurb"))
            with open(cfg.p(m.DSM_INSPECT_PATH), "w") as fh:
                fh.write("KV|hw_version|DS923+_1\nKV|dsm_version|7.2-64570\n")
            u = m.unit_info(cfg)
            self.assertEqual((u["model"], u["source"], u["dsm_version"]), ("DS923+_1", "dsm-inspect.txt", "7.2-64570"))
            with open(cfg.p(m.DSM_INSPECT_PATH), "w") as fh:
                fh.write("KV|model|DS918+\nKV|hw_version|DS918+_1\n")
            self.assertEqual(m.unit_info(cfg)["model"], "DS918+")

    def test_microcode_state(self):
        app = m.App(m.Config(root="/nonexistent-root"), FakeRunner())
        app.runner.on(["dpkg-query"], "ii \t3.20250211.1\n")
        intel = m.parse_cpuinfo(CPUINFO_INTEL)
        mc = m.microcode_info(app, intel, "x86_64")
        self.assertEqual((mc["package"], mc["installed"], mc["version"], mc["loaded_revision"]), ("intel-microcode", True, "3.20250211.1", "0x28"))
        self.assertTrue(app.runner.called("dpkg-query", "-W", "-f", "${db:Status-Abbrev}\t${Version}\n", "intel-microcode"))
        app.runner.on(["dpkg-query"], "", rc=1, err="no packages found")
        self.assertFalse(m.microcode_info(app, intel, "x86_64")["installed"])
        app.runner.on(["dpkg-query"], "rc \t3.1\n")                     # removed, config files remain
        self.assertFalse(m.microcode_info(app, intel, "x86_64")["installed"])
        self.assertEqual(m.microcode_info(app, m.parse_cpuinfo(CPUINFO_AMD), "x86_64")["package"], "amd64-microcode")
        arm = m.microcode_info(app, m.parse_cpuinfo(CPUINFO_ARM), "armv7l")
        self.assertFalse(arm["applicable"])


class HttpStatusInfoTests(HttpTestCase):
    def test_status_includes_cpu_kernel_microcode_console_unit(self):
        self.runner.on(["dpkg-query"], "ii \t3.1\n")
        self.env.write(m.DSM_INSPECT_PATH, "KV|model|DS918+\nKV|hw_version|DS918+_1\n")
        self.logged_in()
        st, obj, _ = self.cl.call("GET", "/api/status")
        self.assertEqual(st, 200)
        for k in ("cpu", "kernel_info", "microcode", "console", "unit", "version", "boot_flash"):
            self.assertIn(k, obj)
        self.assertEqual(obj["unit"]["model"], "DS918+")
        self.assertEqual(obj["version"], m.VERSION)
        self.assertEqual(obj["kernel_info"]["release"], obj["kernel"])
        self.assertIn("cmdline_consoles", obj["console"])
        self.assertFalse(obj["boot_flash"])


class VersionAndUiTests(HttpTestCase):
    ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

    def test_version_file_is_the_single_source(self):
        with open(os.path.join(self.ROOT, "VERSION")) as fh:
            v = fh.read().strip()
        self.assertRegex(v, r"^\d+\.\d+\.\d+$")
        self.assertEqual(m.VERSION, v)
        self.assertEqual(v, "0.4.0")

    def test_version_in_api_and_footer(self):
        st, obj, _ = self.cl.raw("GET", "/api/tls-fingerprint")
        self.assertEqual(obj["version"], m.VERSION)
        st, body, _ = self.cl.raw("GET", "/")
        self.assertIn("version: '%s'" % m.VERSION, body.decode())                  # shown in the login footer and the sidebar footer
        self.assertIn("'synofurb-mgmt ' + S.version", body.decode())
        self.logged_in()
        self.assertEqual(self.cl.call("GET", "/api/me")[1]["version"], m.VERSION)
        self.assertEqual(self.cl.call("GET", "/api/services")[1]["version"], m.VERSION)

    def test_default_version_pattern(self):
        self.assertRegex(m.DEFAULT_VERSION, r"^\d+\.\d+\.\d+$")

    def test_tokens_match_design_tokens_json(self):
        path = os.path.join(self.ROOT, "..", "..", "design", "mockups", "ui", "tokens.json")
        if not os.path.exists(path):
            self.skipTest("design tokens.json not available")
        with open(path) as fh:
            tok = json.load(fh)
        ours = m.theme_tokens()
        for theme in ("light", "dark"):
            expect = {k.split(".", 2)[2]: v for k, v in tok.items() if k.startswith("color.%s." % theme)}
            self.assertEqual(ours[theme], expect, theme)
        acc = m.accent_tokens()
        for a in m.ACCENTS:
            for theme in ("light", "dark"):
                expect = {k.split(".", 3)[3]: v for k, v in tok.items() if k.startswith("accent.%s.%s." % (a, theme))}
                self.assertEqual(acc[a][theme], expect, (a, theme))
        self.assertEqual(tok["font.ui"].split(",")[0], "Inter")
        norm = lambda x: x.replace('"', "").replace(", ", ",")                    # noqa: E731
        self.assertIn(norm(tok["font.ui"]), norm(m.INDEX_HTML))
        self.assertIn(norm(tok["font.mono"]), norm(m.INDEX_HTML))

    def test_pressed_placeholder_and_border_tokens_exist(self):
        t = m.theme_tokens()
        for theme in ("light", "dark"):
            for name in ("placeholder", "accent_pressed", "danger_pressed", "danger_edge", "border_strong", "ring_inverse",
                         "ok_border", "warn_border", "err_border", "info_border", "accent_border", "log_err_bg", "log_wrn_bg", "scrim"):
                self.assertIn(name, t[theme], (theme, name))
        self.assertNotIn("fg_3", t["light"])                               # replaced by the dedicated placeholder token
        self.assertNotIn("overlay", t["light"])
        css = m.theme_css()
        for var in ("--placeholder:#696969", "--accent-pressed:#541EA0", "--danger-pressed:#94020D", "--danger-edge:#FF9592", "--border-strong:#808080"):
            self.assertIn(var, css)
        html = m.INDEX_HTML
        for use in ("input::placeholder{color:var(--placeholder)", ".btn.primary:active{background:var(--accent-pressed)",
                    ".btn.danger:active{background:var(--danger-pressed)", ".btn:active{background:var(--surface-3)",
                    ".btn.danger{background:var(--danger);color:var(--on-danger);border:2px solid var(--danger-edge)",
                    ".btn.danger:disabled", ".is-loading", ":focus-visible{outline:2px solid var(--ring);outline-offset:2px}",
                    "progress{", "border:1px solid var(--border-strong)", "#pend{position:sticky"):
            self.assertIn(use, html)
        self.assertNotIn("var(--fg-3)", html)
        self.assertNotIn("var(--overlay)", html)

    def test_no_color_mix_gradients_or_inset_shadows(self):
        css = m.INDEX_HTML.split("<style", 1)[1].split("</style>", 1)[0]
        self.assertNotIn("color-mix(", m.INDEX_HTML)
        for bad in ("gradient(", "backdrop-filter", "repeating-", "inset ", "text-transform"):
            self.assertNotIn(bad, css, bad)

    def test_type_floor_and_weights(self):
        css = m.INDEX_HTML.split("<style", 1)[1].split("</style>", 1)[0]
        sizes = [int(x) for x in m.re.findall(r"font(?:-size)?:(?:[^;{}]*?\s)?(\d+(?:\.\d+)?)px", css)]
        self.assertTrue(sizes and min(sizes) >= 12, sorted(set(sizes)))
        for w in m.re.findall(r"font(?:-weight)?:\s*(\d{3})\b", css):
            self.assertIn(w, ("400", "600", "700"))

    def test_phone_layout_targets(self):
        css = m.INDEX_HTML.split("<style", 1)[1].split("</style>", 1)[0]
        phone = css.split("@media (max-width:700px){", 1)[1]
        self.assertIn("height:44px;min-height:44px", phone)               # buttons, selects, inputs
        self.assertIn("min-height:56px", phone)                           # tab bar items
        self.assertIn(".rb .acts .btn{width:100%}", phone)                # Keep / Roll back full width
        self.assertIn("table.st thead{display:none}", phone)              # tables become cards

    def test_ui_is_self_contained_and_csp_clean(self):
        html = m.INDEX_HTML
        self.assertNotRegex(html, r"""\sstyle\s*=""")                        # inline style attributes would be blocked by the nonce CSP
        self.assertNotRegex(html, r"""<(?:script|link|img|iframe)[^>]+(?:src|href)=["']https?:""")
        urls = set(m.re.findall(r"https?://[^\s\"'<>)]+", html))
        allowed = {"http://www.w3.org/2000/svg"} | set(m.re.findall(r"https?://[^\s\"'<>)]+", json.dumps(m._LICENSES)))
        self.assertEqual({u for u in urls if u not in allowed and not u.startswith("https://github.com/andreaswiren/synofurb")}, set())   # only licence texts and outbound links
        self.assertEqual(html.count("<script nonce=\"__NONCE__\">"), 2)
        self.assertIn("<style nonce=\"__NONCE__\">", html)
        for needle in ("Boot entries", "Mounts", "Return to DSM", "Preview only", "Keep", "prefers-reduced-motion", "forced-colors"):
            self.assertIn(needle, html)
        self.assertIn("max-width:700px", html)                                 # phone layout (tokens: layout.phone_breakpoint_px)


class TlsTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("openssl"), "openssl missing")
    def test_self_signed_cert_generation(self):
        with tempfile.TemporaryDirectory() as t:
            cfg = m.Config(root=t)
            cert, key = m.ensure_tls(cfg, m.RealRunner(), "nas-test", ["192.168.1.5", "2001:db8::5", "bogus"])
            self.assertEqual(os.stat(key).st_mode & 0o777, 0o600)
            fp = m.cert_fingerprint(cert)
            self.assertEqual(len(fp.split(":")), 32)
            self.assertEqual(m.ensure_tls(cfg, m.RealRunner(), "other", []), (cert, key))          # reused, not regenerated
            self.assertEqual(m.cert_fingerprint(cert), fp)
            ctx = m.make_ssl_context(cert, key)
            self.assertEqual(ctx.minimum_version, m.ssl.TLSVersion.TLSv1_2)
            txt = subprocess.run(["openssl", "x509", "-in", cert, "-noout", "-text"], capture_output=True, text=True).stdout
            self.assertIn("prime256v1", txt)
            self.assertIn("DNS:nas-test.local", txt)
            self.assertIn("IP Address:192.168.1.5", txt)


if __name__ == "__main__":
    unittest.main()
