#!/usr/bin/env python3
"""Build demo/index.html: the REAL management UI (INDEX_HTML from synofurb_mgmt.py) plus a mock API layer.

    python3 tools/build-demo.py            # writes demo/index.html next to this tools/ directory

How it works
  * GET data comes from the real route handlers of synofurb_mgmt.py, run at build time against a canned DryRunRunner
    and a sandbox root for two units (DS918+ Intel, DS1621+ AMD), so shapes and wording are the real ones.
  * The browser side (tools/demo-mock.js) replaces window.fetch for /api/*: it serves that data, simulates every
    POST (nothing real happens), keeps state so the UI reacts (create a volume, change the network, set the boot default...).
  * The output is one self-contained HTML file: no external resources, the UI HTML is unmodified except for the nonce
    placeholder, the page title and one injected <style>/<script> pair in <head>.
"""
from __future__ import annotations

import contextlib
import json
import os
import sys
import tempfile
import types

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, ROOT)
import synofurb_mgmt as m  # noqa: E402

GiB, TiB = 1024 ** 3, 1024 ** 4


# ---------------------------------------------------------------- fixtures --------------------------------------------

def node(name, typ, size, fstype=None, mps=(), pk=None, label=None, uuid=None, model=None, serial=None, tran=None, rota=None, kname=None, children=None):
    d = {"name": name, "kname": kname or name, "type": typ, "size": int(size), "fstype": fstype, "uuid": uuid, "label": label,
         "mountpoints": list(mps) or [None], "model": model, "serial": serial, "rota": rota, "tran": tran, "pkname": pk}
    if children:
        d["children"] = children
    return d


def build_unit(key: str) -> dict:
    """Canned system description of one unit."""
    amd = key == "ds1621"
    host = "ds1621plus" if amd else "ds918plus"
    n, per, model = (6, int(7.28 * TiB), "ST8000VN004-2M2101") if amd else (4, int(3.64 * TiB), "WD40EFRX-68N32N0")
    level, lname = ("raid6", "raid6") if amd else ("raid5", "raid5")
    sys_part = 24 * GiB
    data_part = per - sys_part - 2 * GiB
    md0_size = sys_part - 64 * 1024 ** 2
    md1_size = data_part * (n - 2 if amd else n - 1) - 128 * 1024 ** 2
    root_sz, swap_sz = 16 * GiB, 4 * GiB
    data_sz = int(md1_size * 0.8)
    letters = "abcdef"[:n]

    lvs_sys = [node("vg_sys-root", "lvm", root_sz, "ext4", ["/"], "md0", "root", "6c1f0e0a-0d52-4c5a-9a0e-3b1f0c7d2a11", kname="dm-0"),
               node("vg_sys-swap", "lvm", swap_sz, "swap", ["[SWAP]"], "md0", None, "b9f6a2d4-7c3e-4a77-8d0c-2f5e6a1c9b22", kname="dm-1")]
    md0 = node("md0", "raid1", md0_size, "LVM2_member", pk=None, uuid="x0", kname="md0", children=lvs_sys)
    lv_data = node("vg_data-data", "lvm", data_sz, "xfs", ["/srv/data"], "md1", "data", "0e3a58c1-5f0a-4d2e-b6a7-9d4c1e7b3f33", kname="dm-2")
    md1 = node("md1", level, md1_size, "LVM2_member", uuid="x1", kname="md1", children=[lv_data])
    disks = []
    for i, c in enumerate(letters):
        disks.append(node("sd" + c, "disk", per, model=model, serial=(("ZR5X%04d" % (4100 + i)) if amd else ("WD-WCC7K%dX%03d" % (i, 41 + i))), tran="sata", rota=True, children=[
            node("sd%s1" % c, "part", sys_part, "linux_raid_member", pk="sd" + c, uuid="m0", children=[md0]),
            node("sd%s2" % c, "part", data_part, "linux_raid_member", pk="sd" + c, uuid="m1", children=[md1])]))
    boot = node("synoboot", "disk", 122 * 1024 ** 2, model="DOM", tran=None, rota=False, children=[
        node("synoboot1", "part", 50 * 1024 ** 2, "vfat", pk="synoboot", label="SYNOBOOT"), node("synoboot2", "part", 70 * 1024 ** 2, "ext2", pk="synoboot")])
    extra = []
    if amd:
        extra += [node("nvme0n1", "disk", 465 * GiB, model="WD Red SN700 500GB", serial="23123L800123", tran="nvme", rota=False),
                  node("nvme1n1", "disk", 465 * GiB, model="WD Red SN700 500GB", serial="23123L800124", tran="nvme", rota=False)]
        extra += [node("sdg", "disk", int(0.93 * TiB), model="My Passport 25E1", serial="WX21A12B3C4D", tran="usb", rota=True, children=[
            node("sdg1", "part", int(0.93 * TiB), "ntfs", pk="sdg", label="Backup", uuid="1234567890ABCDEF")])]
    else:
        extra += [node("sde", "disk", int(1.82 * TiB), model="ST2000DM008-2FR102", serial="ZFL1ABCD", tran="usb", rota=True),
                  node("sdg", "disk", int(1.82 * TiB), model="ST2000DM008-2FR102", serial="ZFL1WXYZ", tran="usb", rota=True),
                  node("sdf", "disk", int(0.93 * TiB), model="My Passport 25E1", serial="WX21A12B3C4D", tran="usb", rota=True, children=[
                      node("sdf1", "part", int(0.93 * TiB), "ntfs", pk="sdf", label="Backup", uuid="1234567890ABCDEF")])]
    syno = node("vg1000-lv", "lvm", int(3.1 * TiB), "btrfs", pk="sdh", label="1.42.6-25556", uuid="5d0c4a1e-77b1-4f3b-a2d8-0f1c9e6b7a44", kname="dm-9")
    extra.append(node("sdh", "disk", int(3.64 * TiB), model="WD40EFRX-68WT0N0", serial="WD-WCC4E0XYZ12", tran="sata", rota=True, children=[syno]))
    lsblk = json.dumps({"blockdevices": [boot] + disks + extra})

    def lvm(sec, rows):
        return json.dumps({"report": [{sec: rows}]})
    vg_data_size = md1_size
    vgs = lvm("vg", [{"vg_name": "vg_data", "vg_size": str(vg_data_size), "vg_free": str(vg_data_size - data_sz), "pv_count": "1", "lv_count": "1"},
                     {"vg_name": "vg_sys", "vg_size": str(md0_size), "vg_free": str(md0_size - root_sz - swap_sz), "pv_count": "1", "lv_count": "2"}])
    lvs = lvm("lv", [{"lv_name": "data", "vg_name": "vg_data", "lv_size": str(data_sz), "lv_path": "/dev/vg_data/data", "lv_attr": "-wi-ao----"},
                     {"lv_name": "root", "vg_name": "vg_sys", "lv_size": str(root_sz), "lv_path": "/dev/vg_sys/root", "lv_attr": "-wi-ao----"},
                     {"lv_name": "swap", "vg_name": "vg_sys", "lv_size": str(swap_sz), "lv_path": "/dev/vg_sys/swap", "lv_attr": "-wi-ao----"}])
    pvs = lvm("pv", [{"pv_name": "/dev/md1", "vg_name": "vg_data", "pv_size": str(vg_data_size), "pv_free": str(vg_data_size - data_sz)},
                     {"pv_name": "/dev/md0", "vg_name": "vg_sys", "pv_size": str(md0_size), "pv_free": str(md0_size - root_sz - swap_sz)}])
    used_data = int(data_sz * (0.31 if amd else 0.58))
    findmnt = json.dumps({"filesystems": [{"target": "/", "source": "/dev/mapper/vg_sys-root", "fstype": "ext4", "options": "rw,relatime", "size": root_sz - 400 * 1024 ** 2,
        "used": int(2.6 * GiB), "avail": root_sz - int(3 * GiB), "children": [
            {"target": "/srv/data", "source": "/dev/mapper/vg_data-data", "fstype": "xfs", "options": "rw,noatime,attr2,inode64", "size": data_sz, "used": used_data, "avail": data_sz - used_data},
            {"target": "/boot", "source": "/dev/synoboot2", "fstype": "ext2", "options": "rw,relatime", "size": 66 * 1024 ** 2, "used": 31 * 1024 ** 2, "avail": 35 * 1024 ** 2}]}]})

    def mdline(name, lvl, devs, blocks, health, extra_line=""):
        members = " ".join("%s[%d]" % (d, i) for i, d in enumerate(devs))
        return "%s : active %s %s\n      %d blocks super 1.2 level %s, 512k chunk, algorithm 2 [%d/%d] [%s]\n%s\n" % (
            name, lvl, members, blocks // 1024, lvl[-1], len(devs), len(devs), health, extra_line)
    resync = "" if amd else "      [========>............]  resync = 41.0%% (%d/%d) finish=432.0min speed=88000K/sec" % (md1_size // 2048, md1_size // 1024)
    mdstat = ("Personalities : [raid1] [raid6] [raid5] [raid4]\n" + mdline("md1", level, ["sd%s2" % c for c in reversed(letters)], md1_size, "U" * n, resync)
              + "\n" + mdline("md0", "raid1", ["sd%s1" % c for c in reversed(letters)], md0_size, "U" * n) + "\nunused devices: <none>\n")
    if amd:
        cpuinfo = "".join("processor\t: %d\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen Embedded V1500B with Radeon Vega Gfx\nmicrocode\t: 0x8101016\n\n" % i for i in range(8))
        ip_ifs = [("eth0", "UP", "00:11:32:b7:21:a0", [("inet", "192.168.1.60", 24, "global"), ("inet6", "2001:db8:1::60", 64, "global"), ("inet6", "fe80::211:32ff:feb7:21a0", 64, "link")]),
                  ("eth1", "UP", "00:11:32:b7:21:a1", [("inet", "10.10.0.60", 24, "global"), ("inet6", "fe80::211:32ff:feb7:21a1", 64, "link")]),
                  ("eth2", "DOWN", "00:11:32:b7:21:a2", []), ("eth3", "DOWN", "00:11:32:b7:21:a3", [])]
    else:
        cpuinfo = "".join("processor\t: %d\nvendor_id\t: GenuineIntel\nmodel name\t: Intel(R) Celeron(R) CPU J3455 @ 1.50GHz\nmicrocode\t: 0x28\n\n" % i for i in range(4))
        ip_ifs = [("eth0", "UP", "00:11:32:a1:b2:c3", [("inet", "192.168.1.40", 24, "global"), ("inet6", "2001:db8:1::40", 64, "global"), ("inet6", "fe80::211:32ff:fea1:b2c3", 64, "link")]),
                  ("eth1", "DOWN", "00:11:32:a1:b2:c4", [])]
    ip = json.dumps([{"ifname": "lo", "operstate": "UNKNOWN", "address": "00:00:00:00:00:00", "mtu": 65536, "addr_info": []}] + [
        {"ifname": i, "operstate": st, "address": mac, "mtu": 1500, "link_type": "ether", "flags": [], "addr_info": [
            {"family": f, "local": a, "prefixlen": p, "scope": sc} for f, a, p, sc in ads]} for i, st, mac, ads in ip_ifs])
    cmdline = "BOOT_IMAGE=/vmlinuz root=/dev/mapper/vg_sys-root ro rootdelay=5 panic=60 console=tty0 console=ttyS0,115200n8\n"
    dom = None
    names = [("DEBIAN", True, False), ("DEBIAN_OLD", True, False), ("SYNOLOGY_2", False, True)] + ([("SYNOFURB_BOOT", True, False)] if amd else [])
    dom = {"tool": "synofurb-dom", "version": "0.4.1", "device": "/dev/synoboot", "device_bytes": 122 * 1024 ** 2,
           "p1": {"dev": "/dev/synoboot1", "bytes": 50 * 1024 ** 2, "free_bytes": 31 * 1024 ** 2, "fat_volume_id": "A1B2-C3D4"},
           "p2": {"dev": "/dev/synoboot2", "bytes": 70 * 1024 ** 2, "free_bytes": 9 * 1024 ** 2, "uuid": "u"},
           "entries": [{"index": i, "name": nm, "ours": ours, "dsm": dsm} for i, (nm, ours, dsm) in enumerate(names)],
           "default": {"raw": "0", "index": 0, "name": "DEBIAN"}, "files": [], "efi_files": [],
           "backup": {"exists": True, "path": "/srv/data/synofurb/synofurb-dom-first-backup.img"}}
    smart_note = {} if amd else {"sdc": "2 reallocated sectors"}
    return {
        "key": key, "host": host, "label": "DS1621+ (AMD Ryzen V1500B, 6 bays)" if amd else "DS918+ (Intel Celeron J3455, 4 bays)",
        "kernel": "7.2.3+deb13-amd64" if amd else "7.2.1+deb13-amd64", "os_release": 'PRETTY_NAME="Debian GNU/Linux 13 (trixie)"\nNAME="Debian GNU/Linux"\nVERSION_ID="13"\nVERSION="13 (trixie)"\nID=debian\n',
        "root_total": root_sz - 400 * 1024 ** 2, "root_free": root_sz - 3 * GiB, "uptime": 11 * 86400 + 3 * 3600 + 540 if amd else 3 * 86400 + 4 * 3600 + 120,
        "mem_total": 32 * GiB if amd else int(7.7 * GiB), "mem_avail": 29 * GiB if amd else int(5.6 * GiB), "load": (0.12, 0.18, 0.15) if amd else (0.31, 0.22, 0.18),
        "temp": 47.0 if amd else 51.0, "lsblk": lsblk, "vgs": vgs, "lvs": lvs, "pvs": pvs, "findmnt": findmnt, "mdstat": mdstat, "cpuinfo": cpuinfo, "ip": ip,
        "ip_ifs": [i[0] for i in ip_ifs], "cmdline": cmdline, "dom": dom, "microcode": ("" if amd else "ii \t3.20250211.1\n"),
        "inspect": "KV|model|%s\nKV|hw_version|%s_1\nKV|dsm_version|%s\nKV|cpu|%s\nKV|uefi|0\n" % (
            ("DS1621+", "DS1621+", "7.2.1-69057", "AMD Ryzen Embedded V1500B") if amd else ("DS918+", "DS918+", "7.1.1-42962", "Intel(R) Celeron(R) CPU J3455")),
        "smart_note": smart_note, "per_disk_temp": [36, 38, 41, 37, 39, 40], "n": n, "letters": letters,
    }


# ---------------------------------------------------------------- recording ------------------------------------------

@contextlib.contextmanager
def patched(u: dict):
    saved = (m.read_text, m.read_temps, m.os.getloadavg, m.socket.gethostname, m.os.uname, m.os.statvfs)
    raw = {"/proc/mdstat": u["mdstat"], "/proc/cpuinfo": u["cpuinfo"], "/proc/cmdline": u["cmdline"], "/proc/uptime": "%d 0\n" % u["uptime"],
           "/proc/meminfo": "MemTotal: %d kB\nMemAvailable: %d kB\nSwapTotal: 4194304 kB\nSwapFree: 4190000 kB\n" % (u["mem_total"] // 1024, u["mem_avail"] // 1024)}
    orig = m.read_text
    m.read_text = lambda p: raw[p] if p in raw else orig(p)
    m.read_temps = lambda: [{"zone": "acpitz", "celsius": u["temp"]}]
    m.os.getloadavg = lambda: u["load"]
    m.socket.gethostname = lambda: u["host"]
    m.os.uname = lambda: types.SimpleNamespace(release=u["kernel"], machine="x86_64")
    m.os.statvfs = lambda p: types.SimpleNamespace(f_blocks=u["root_total"] // 4096, f_frsize=4096, f_bavail=u["root_free"] // 4096)
    try:
        yield
    finally:
        m.read_text, m.read_temps, m.os.getloadavg, m.socket.gethostname, m.os.uname, m.os.statvfs = saved


def write(cfg, path, text, mode=0o644):
    m.write_atomic(cfg.p(path), text, mode=mode)


def record(key: str) -> dict:
    u = build_unit(key)
    root = tempfile.mkdtemp(prefix="synofurb-demo-")
    cfg = m.Config(root=root, dry_run=True, tls=False, fail_delay=0)
    write(cfg, "/etc/passwd", "root:x:0:0:root:/root:/bin/bash\nadmin:x:1000:1000:Admin:/home/admin:/bin/bash\nandreas:x:1001:1001:Andreas:/home/andreas:/bin/bash\n"
          "media:x:1002:100::/home/media:/bin/bash\nbackup:x:1003:1003::/home/backup:/bin/bash\n")
    write(cfg, "/etc/group", "root:x:0:\nsudo:x:27:admin,andreas\nusers:x:100:media\nadm:x:4:admin\nadmin:x:1000:\nandreas:x:1001:\nbackup:x:1003:\n")
    write(cfg, "/etc/shadow", "root:*:19000:::::\nadmin:$y$j9T$x$y:19000::::::\nandreas:$y$j9T$x$y:19000::::::\nmedia:$y$j9T$x$y:19000::::::\nbackup:!$y$j9T$x$y:19000::::::\n", 0o600)
    k1 = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl admin@laptop"
    k2 = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA8sEw0uQ1k9Zs1m3xk0n8Qy0TQe5E1o2fW2cY9pQ7kz admin@desktop"
    write(cfg, "/home/admin/.ssh/authorized_keys", k1 + "\n" + k2 + "\n")
    write(cfg, "/home/andreas/.ssh/authorized_keys", k1.replace("admin@laptop", "andreas@work") + "\n")
    write(cfg, m.DSM_INSPECT_PATH, u["inspect"])
    write(cfg, "/etc/os-release", u["os_release"])
    write(cfg, m.DOM_TOOL, "#!/bin/sh\n")
    for t in ("ed25519", "ecdsa", "rsa"):
        write(cfg, "/etc/ssh/ssh_host_%s_key.pub" % t, "ssh-%s AAAA host\n" % t)
    m.write_atomic(m.net_file(cfg, "eth0"), m.render_networkd("eth0", "dhcp"))
    if key == "ds1621":
        m.write_atomic(m.net_file(cfg, "eth1"), m.render_networkd("eth1", "static", "10.10.0.60/24", "10.10.0.1", dns=["10.10.0.1", "9.9.9.9"], search=["lan"]))
    write(cfg, "/etc/fstab", "# /etc/fstab: static file system information.\nUUID=6c1f0e0a-0d52-4c5a-9a0e-3b1f0c7d2a11 / ext4 errors=remount-ro 0 1\n")
    write(cfg, "/etc/fstab", m.fstab_add(m.read_text(cfg.p("/etc/fstab")), "UUID=0e3a58c1-5f0a-4d2e-b6a7-9d4c1e7b3f33", "/srv/data", "xfs", "defaults,nofail,x-systemd.device-timeout=30", "data"))

    def smart(argv):
        dev = argv[-1].rsplit("/", 1)[-1]
        i = "abcdefgh".find(dev[-1]) if dev.startswith("sd") else 0
        note = u["smart_note"].get(dev)
        return m.CompletedResult(4 if note else 0, json.dumps({"smart_status": {"passed": True}, "temperature": {"current": u["per_disk_temp"][i % 6] if dev.startswith("sd") else 33},
                                                              "power_on_time": {"hours": 21000 + 700 * i}, "smartctl": {"messages": [{"string": note}] if note else []}}), "")

    def keygen(argv):
        t = argv[-1].split("ssh_host_")[1].split("_key")[0]
        return m.CompletedResult(0, {"ed25519": "256 SHA256:Zk3a0Qm1bXn8dY2L5vTq9rWc7HfJ4sPoUe6iAgKxN1M root@%s (ED25519)\n",
                                     "ecdsa": "256 SHA256:Pq7sLd2Vn0uYb5TgA1xKc8RmZ3eHj6WoFi4NtXyB9Cw root@%s (ECDSA)\n",
                                     "rsa": "3072 SHA256:Hw5eTn8Yk2QzXb0LcR7pVa3JmUs1DoGf6iAyNtKx4Ec root@%s (RSA)\n"}[t] % u["host"], "")

    def systemctl(argv):
        sub = argv[1]
        if sub == "is-enabled":
            return m.CompletedResult(0, "enabled\n")
        if sub == "is-active":
            return m.CompletedResult(0, "active\n" if argv[-1] == "ssh" else "inactive\n", "")
        if sub == "list-units":
            return m.CompletedResult(0, "serial-getty@ttyS0.service loaded active running Serial Getty on ttyS0\n")
        return m.CompletedResult(0, "")

    runner = m.DryRunRunner(canned={
        "lsblk": u["lsblk"], "ip": u["ip"], "findmnt": u["findmnt"], "vgs": u["vgs"], "lvs": u["lvs"], "pvs": u["pvs"],
        "dpkg-query": u["microcode"] if u["microcode"] else m.CompletedResult(1, "", "dpkg-query: no packages found matching amd64-microcode"),
        "mdadm": "ARRAY /dev/md/1 metadata=1.2 name=%s:1 UUID=3a5f1c77:0b4e9d21:7c8a2e66:91d0f5b3\n" % u["host"], "smartctl": smart, "ssh-keygen": keygen, "systemctl": systemctl,
        "synofurb-dom": lambda a: m.CompletedResult(0, json.dumps(u["dom"]) if "status" in a else "", ""),
    })
    app = m.App(cfg, runner)
    app.fingerprint = "4F:A1:09:3B:77:D2:AA:10:5C:E8:91:0B:C4:6E:32:7F:19:D5:84:A0:3C:B6:52:E1:F8:2D:90:44:7A:CB:C2:7E"
    sess = m.Session("k", "admin", "demo-csrf", "127.0.0.1", 0.0, 0.0)

    def get(path, method="GET", body=None):
        req = m.Request(app, method, path, body or {}, sess, "127.0.0.1")
        res = m.ROUTES[(method, path)].fn(req)
        return res[1] if isinstance(res, tuple) else res

    with patched(u):
        data = {k: get(p) for k, p in (("status", "/api/status"), ("smart", "/api/smart"), ("users", "/api/users"), ("network", "/api/network"),
                                       ("volumes", "/api/volumes"), ("mounts", "/api/mounts"), ("services", "/api/services"), ("boot", "/api/boot"),
                                       ("tls", "/api/tls-fingerprint"), ("me", "/api/me"))}
        data["activate"] = get("/api/mounts/activate", "POST")
    data["meta"] = {"key": key, "label": u["label"], "host": u["host"], "iface_names": u["ip_ifs"], "uptime": u["uptime"], "per_disk_temp": u["per_disk_temp"]}
    return data


def build() -> str:
    data = {"units": {k: record(k) for k in ("ds918", "ds1621")}, "version": m.VERSION}
    blob = json.dumps(data, separators=(",", ":")).replace("</", "<\\/").replace("<!--", "<\\!--")
    with open(os.path.join(HERE, "demo-mock.js"), encoding="utf-8") as fh:
        mock = fh.read().replace("/*__DATA__*/null", blob)
    with open(os.path.join(HERE, "demo-mock.css"), encoding="utf-8") as fh:
        css = fh.read()
    html = m.INDEX_HTML.replace("__NONCE__", "demo").replace("__VERSION__", m.VERSION)
    assert "</head>" in html
    html = html.replace("<title>NAS management</title>", "<title>NAS management (demo)</title>", 1)
    html = html.replace("</head>", '<style nonce="demo">\n%s</style>\n<script nonce="demo">\n%s</script>\n</head>' % (css, mock), 1)
    return html


def main() -> int:
    out_dir = os.path.join(ROOT, "demo")
    os.makedirs(out_dir, exist_ok=True)
    html = build()
    path = os.path.join(out_dir, "index.html")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(html)
    print("wrote %s (%d KiB)" % (path, len(html) // 1024))
    return 0


if __name__ == "__main__":
    sys.exit(main())
