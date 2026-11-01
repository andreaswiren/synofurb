"""RAID / SHR management tests: parsers (realistic mdadm and mdstat output), inventory, plans, safety refusals, SHR slice vectors, HTTP routes."""
from __future__ import annotations

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import synofurb_mgmt as m  # noqa: E402
from test_mgmt import Client, Env, FakeRunner, HttpTestCase, LSBLK, make_runner  # noqa: E402

GIB, MIB = m.GIB, m.MIB


# ---------------------------------------------------------------------------------------------------------------------
# realistic fixtures
# ---------------------------------------------------------------------------------------------------------------------

DETAIL_CLEAN = """/dev/md1:
           Version : 1.2
     Creation Time : Mon Oct 28 10:12:44 2024
        Raid Level : raid5
        Array Size : 11706543104 (10.90 TiB 11.99 TB)
     Used Dev Size : 3902181376 (3.63 TiB 4.00 TB)
      Raid Devices : 4
     Total Devices : 4
       Persistence : Superblock is persistent

     Intent Bitmap : Internal

       Update Time : Tue Oct 29 08:00:01 2024
             State : clean
    Active Devices : 4
   Working Devices : 4
    Failed Devices : 0
     Spare Devices : 0

            Layout : left-symmetric
        Chunk Size : 512K

Consistency Policy : bitmap

              Name : ds918plus:1  (local to host ds918plus)
              UUID : 3a5f1c77:0b4e9d21:7c8a2e66:91d0f5b3
            Events : 12345

    Number   Major   Minor   RaidDevice State
       0       8        2        0      active sync   /dev/sda2
       1       8       18        1      active sync   /dev/sdb2
       2       8       34        2      active sync   /dev/sdc2
       3       8       50        3      active sync   /dev/sdd2
"""

DETAIL_DEGRADED = """/dev/md1:
           Version : 1.2
        Raid Level : raid5
        Array Size : 11706543104 (10.90 TiB 11.99 TB)
     Used Dev Size : 3902181376 (3.63 TiB 4.00 TB)
      Raid Devices : 4
     Total Devices : 4
     Intent Bitmap : Internal
             State : clean, degraded, recovering
    Active Devices : 3
   Working Devices : 4
    Failed Devices : 0
     Spare Devices : 1
            Layout : left-symmetric
        Chunk Size : 512K
    Rebuild Status : 41% complete
              Name : ds918plus:1  (local to host ds918plus)
              UUID : 3a5f1c77:0b4e9d21:7c8a2e66:91d0f5b3
            Events : 12399

    Number   Major   Minor   RaidDevice State
       0       8        2        0      active sync   /dev/sda2
       1       8       18        1      active sync   /dev/sdb2
       4       8       66        2      spare rebuilding   /dev/sde2
       3       8       50        3      active sync   /dev/sdd2
"""

DETAIL_RESHAPE = """/dev/md1:
        Raid Level : raid5
        Array Size : 11706543104 (10.90 TiB 11.99 TB)
     Used Dev Size : 3902181376 (3.63 TiB 4.00 TB)
      Raid Devices : 5
     Total Devices : 5
             State : clean, reshaping
    Active Devices : 5
   Working Devices : 5
    Failed Devices : 0
     Spare Devices : 0
    Reshape Status : 12% complete
     Delta Devices : 1, (4->5)
              Name : ds918plus:1  (local to host ds918plus)
              UUID : 3a5f1c77:0b4e9d21:7c8a2e66:91d0f5b3

    Number   Major   Minor   RaidDevice State
       0       8        2        0      active sync   /dev/sda2
       1       8       18        1      active sync   /dev/sdb2
       2       8       34        2      active sync   /dev/sdc2
       3       8       50        3      active sync   /dev/sdd2
       4       8       66        4      active sync   /dev/sde2
"""

DETAIL_CHECK = """/dev/md0:
           Version : 1.2
        Raid Level : raid1
        Array Size : 25165824 (24.00 GiB 25.77 GB)
      Raid Devices : 2
     Total Devices : 2
             State : active, checking
    Active Devices : 2
   Working Devices : 2
    Failed Devices : 0
     Spare Devices : 0
      Check Status : 30% complete
              Name : ds918plus:0  (local to host ds918plus)
              UUID : aaaa:bbbb:cccc:dddd

    Number   Major   Minor   RaidDevice State
       0       8        1        0      active sync   /dev/sda1
       1       8       17        1      active sync   /dev/sdb1
"""

DETAIL_FOREIGN = """/dev/md2:
           Version : 1.2
        Raid Level : raid5
        Array Size : 7813772288 (7.28 TiB 8.00 TB)
     Used Dev Size : 3906886144 (3.64 TiB 4.00 TB)
      Raid Devices : 3
     Total Devices : 3
             State : clean
    Active Devices : 3
   Working Devices : 3
    Failed Devices : 0
     Spare Devices : 0
              Name : DiskStation:2
              UUID : 11112222:33334444:55556666:77778888
            Events : 4242

    Number   Major   Minor   RaidDevice State
       0       8       37        0      active sync   /dev/sdf5
       1       8       53        1      active sync   /dev/sdg5
       2       8       69        2      active sync   /dev/sdh5
"""

MDSTAT_RESHAPE = """Personalities : [raid1] [raid6] [raid5] [raid4]
md1 : active raid5 sde2[4] sdd2[3] sdc2[2] sdb2[1] sda2[0]
      11706543104 blocks super 1.2 level 5, 512k chunk, algorithm 2 [5/5] [UUUUU]
      [==>..................]  reshape = 12.3% (480000000/3902181376) finish=900.5min speed=63000K/sec
      bitmap: 0/30 pages [0KB], 65536KB chunk

unused devices: <none>
"""

MDSTAT_CHECK = """Personalities : [raid1]
md0 : active raid1 sdb1[1] sda1[0]
      25165824 blocks super 1.2 [2/2] [UU]
      [======>..............]  check = 30.0% (7549747/25165824) finish=2.1min speed=140000K/sec
      bitmap: 1/1 pages [4KB], 65536KB chunk

unused devices: <none>
"""

MDSTAT_DEGRADED = """Personalities : [raid5]
md1 : active raid5 sde2[4] sdd2[3] sdb2[1] sda2[0]
      11706543104 blocks super 1.2 level 5, 512k chunk, algorithm 2 [4/3] [UU_U]
      [========>............]  recovery = 41.0% (1600000000/3902181376) finish=432.0min speed=88000K/sec

unused devices: <none>
"""

MDSTAT_FOREIGN = """Personalities : [raid5]
md2 : active raid5 sdh5[2] sdg5[1] sdf5[0]
      7813772288 blocks super 1.2 level 5, 64k chunk, algorithm 2 [3/3] [UUU]

unused devices: <none>
"""


def ld(name, typ, size, fs=None, mps=(), pk=None, model=None, serial=None, children=(), kname=None):
    return {"name": name, "kname": kname or name, "path": ("/dev/mapper/" + name) if typ == "lvm" else "/dev/" + (kname or name), "type": typ, "size": size,
            "fstype": fs, "uuid": None, "label": None, "mountpoints": list(mps), "model": model, "serial": serial, "rota": True, "tran": "sata", "pkname": pk,
            "children": list(children)}


def devs_for(disks=("sda", "sdb", "sdc", "sdd", "sde")):
    """A flat lsblk list: data disks with partition 2 (member) and the md/LVM stack above sda2..sdd2."""
    out = [ld("md1", "raid5", 11706543104 * 1024 // 1024, "LVM2_member", children=["dm-0"], kname="md1"),
           ld("vg_data-data", "lvm", 9 * 1024 ** 4, "xfs", ["/srv/data"], pk="md1", kname="dm-0")]
    for d in disks:
        member = d != "sde"
        out.append(ld(d, "disk", 4000787030016, model="WD40EFRX", serial="S-" + d, children=[d + "2"]))
        out.append(ld(d + "2", "part", 3900 * GIB, "linux_raid_member" if member else None, pk=d, children=["md1"] if member else []))
    return out


PVS = [{"pv_name": "/dev/md1", "vg_name": "vg_data", "pv_size": 11706543104 * 1024, "pv_free": 2 * 1024 ** 4}]
LVS = [{"lv_name": "data", "vg_name": "vg_data", "lv_size": 9 * 1024 ** 4, "lv_path": "/dev/vg_data/data", "lv_attr": "-wi-ao----"}]


def inventory(mdstat_text, detail_texts, sysfs=None, devs=None, pvs=PVS, lvs=LVS, host="ds918plus"):
    stat = m.parse_mdstat(mdstat_text)
    det = {n: m.parse_mdadm_detail(t) for n, t in detail_texts.items()}
    return m.build_array_inventory(stat, det, sysfs or {}, devs or devs_for(), pvs, lvs, host)


def by(arrs, name):
    return next(a for a in arrs if a["name"] == name)


# ---------------------------------------------------------------------------------------------------------------------
# parsers
# ---------------------------------------------------------------------------------------------------------------------

class DetailParserTests(unittest.TestCase):
    def test_clean_raid5(self):
        d = m.parse_mdadm_detail(DETAIL_CLEAN)
        self.assertEqual((d["device"], d["level"], d["raid_devices"], d["active_devices"], d["spare_devices"]), ("/dev/md1", "raid5", 4, 4, 0))
        self.assertEqual(d["array_size"], 11706543104 * 1024)                     # mdadm prints KiB
        self.assertEqual(d["used_dev_size"], 3902181376 * 1024)
        self.assertEqual((d["state"], d["state_flags"]), ("clean", ["clean"]))
        self.assertEqual((d["chunk_bytes"], d["layout"], d["bitmap"]), (512 * 1024, "left-symmetric", "internal"))
        self.assertEqual((d["name_host"], d["name_label"], d["name_local"]), ("ds918plus", "1", True))
        self.assertEqual(d["uuid"], "3a5f1c77:0b4e9d21:7c8a2e66:91d0f5b3")
        self.assertEqual([(x["slot"], x["device"], x["state"]) for x in d["members"]], [(i, "/dev/sd%s2" % c, "active") for i, c in enumerate("abcd")])
        self.assertIsNone(d["progress"])

    def test_degraded_rebuilding_with_spare(self):
        d = m.parse_mdadm_detail(DETAIL_DEGRADED)
        self.assertEqual(d["state_flags"], ["clean", "degraded", "recovering"])
        self.assertEqual(d["progress"], {"kind": "recovery", "percent": 41.0})
        st = {x["device"]: x["state"] for x in d["members"]}
        self.assertEqual(st["/dev/sde2"], "rebuilding")
        self.assertEqual((d["active_devices"], d["working_devices"], d["spare_devices"]), (3, 4, 1))

    def test_removed_slot_without_device(self):
        txt = DETAIL_DEGRADED.replace("       4       8       66        2      spare rebuilding   /dev/sde2\n", "       -       0        0        2      removed\n")
        d = m.parse_mdadm_detail(txt)
        rem = [x for x in d["members"] if x["state"] == "removed"]
        self.assertEqual((rem[0]["device"], rem[0]["slot"]), (None, 2))

    def test_faulty_member(self):
        txt = DETAIL_CLEAN.replace("active sync   /dev/sdc2", "faulty   /dev/sdc2")
        self.assertEqual({x["device"]: x["state"] for x in m.parse_mdadm_detail(txt)["members"]}["/dev/sdc2"], "faulty")

    def test_reshape_and_check(self):
        d = m.parse_mdadm_detail(DETAIL_RESHAPE)
        self.assertEqual(d["progress"], {"kind": "reshape", "percent": 12.0})
        self.assertEqual((d["raid_devices"], d["delta_devices"]), (5, "1, (4->5)"))
        self.assertIn("reshaping", d["state_flags"])
        c = m.parse_mdadm_detail(DETAIL_CHECK)
        self.assertEqual(c["progress"], {"kind": "check", "percent": 30.0})
        self.assertEqual(c["level"], "raid1")

    def test_foreign_name_is_not_local(self):
        d = m.parse_mdadm_detail(DETAIL_FOREIGN)
        self.assertEqual((d["name_host"], d["name_label"], d["name_local"]), ("DiskStation", "2", False))

    def test_garbage(self):
        d = m.parse_mdadm_detail("mdadm: cannot open /dev/md9: No such file or directory\n")
        self.assertEqual(d["members"], [])
        self.assertIsNone(d["level"])

    def test_member_state_names(self):
        for raw, want in (("active sync", "active"), ("active sync set-A", "active"), ("spare", "spare"), ("spare rebuilding", "rebuilding"),
                          ("faulty", "faulty"), ("removed", "removed"), ("active sync writemostly", "active"), ("replacement", "replacement")):
            self.assertEqual(m.norm_member_state(raw), want, raw)


class MdstatTests(unittest.TestCase):
    def test_reshape_progress_and_bitmap(self):
        a = m.parse_mdstat(MDSTAT_RESHAPE)[0]
        self.assertEqual((a["name"], a["level"], a["total"], a["active"], a["health"]), ("md1", "raid5", 5, 5, "UUUUU"))
        self.assertEqual(a["sync"], {"action": "reshape", "percent": 12.3, "finish_min": 900.5, "speed_kbs": 63000})
        self.assertEqual(a["bitmap"], {"pages_used": 0, "pages": 30, "chunk_kb": 65536})
        self.assertFalse(a["degraded"])

    def test_check_state(self):
        a = m.parse_mdstat(MDSTAT_CHECK)[0]
        self.assertEqual((a["sync"]["action"], a["sync"]["percent"], a["sync"]["speed_kbs"]), ("check", 30.0, 140000))
        self.assertEqual(a["bitmap"]["pages_used"], 1)

    def test_degraded_recovery(self):
        a = m.parse_mdstat(MDSTAT_DEGRADED)[0]
        self.assertTrue(a["degraded"])
        self.assertEqual((a["active"], a["total"], a["health"], a["sync"]["action"]), (3, 4, "UU_U", "recovery"))

    def test_sysfs_reader(self, ):
        import tempfile
        with tempfile.TemporaryDirectory() as t:
            base = os.path.join(t, "block", "md1", "md")
            os.makedirs(os.path.join(base, "bitmap"))
            for f, v in (("sync_action", "check\n"), ("mismatch_cnt", "128\n"), ("sync_completed", "3750000 / 12500000\n"), ("sync_speed", "98000\n"),
                         ("sync_speed_min", "1000 (local)\n"), ("sync_speed_max", "system\n"), ("degraded", "0\n"), ("array_state", "active\n"), ("stripe_cache_size", "256\n")):
                open(os.path.join(base, f), "w").write(v)
            open(os.path.join(base, "bitmap", "location"), "w").write("+8\n")
            s = m.read_md_sysfs(m.Config(sys_root=t), "md1")
            self.assertEqual((s["sync_action"], s["mismatch_cnt"], s["sync_completed"], s["sync_speed"]), ("check", 128, 30.0, 98000))
            self.assertEqual((s["sync_speed_min"], s["sync_speed_max"], s["stripe_cache_size"], s["bitmap_location"]), (1000, "system", 256, "+8"))
            self.assertEqual(m.read_md_sysfs(m.Config(sys_root=t), "md9"), {"mismatch_cnt": None, "sync_completed": None, "sync_speed": None, "degraded": None,
                                                                              "stripe_cache_size": None, "sync_speed_min": None, "sync_speed_max": None})
        self.assertIsNone(m.parse_sync_completed("none"))
        self.assertIsNone(m.parse_sync_completed("delayed"))


# ---------------------------------------------------------------------------------------------------------------------
# inventory
# ---------------------------------------------------------------------------------------------------------------------

class InventoryTests(unittest.TestCase):
    def test_clean_array_with_uses(self):
        a = by(inventory(MDSTAT_RESHAPE.replace("reshape", "idle"), {"md1": DETAIL_CLEAN}), "md1")
        self.assertEqual((a["level"], a["state"], a["degraded"], a["busy"], a["foreign"]), ("raid5", "clean", False, False, False))
        self.assertEqual(a["vgs"], ["vg_data"])
        self.assertEqual(a["lvs"], ["vg_data/data"])
        self.assertEqual(a["mounts"], ["/srv/data"])
        self.assertTrue(a["in_use"])
        self.assertEqual([m_["disk"] for m_ in a["members"]][:2], ["sda", "sdb"])
        self.assertEqual(a["members"][0]["model"], "WD40EFRX")
        self.assertEqual(a["label"], "1")

    def test_degraded_rebuild_progress_and_spare(self):
        a = by(inventory(MDSTAT_DEGRADED, {"md1": DETAIL_DEGRADED}, {"md1": {"sync_action": "recover", "mismatch_cnt": 0, "sync_speed": 91000}}), "md1")
        self.assertTrue(a["degraded"] and a["busy"])
        self.assertEqual(a["state"], "degraded")
        self.assertEqual(a["sync"]["action"], "recovery")
        self.assertEqual(a["sync"]["speed_kbs"], 88000)
        self.assertEqual([x["state"] for x in a["members"]].count("rebuilding"), 1)
        self.assertEqual(a["mismatch_cnt"], 0)

    def test_reshape_and_check_are_busy(self):
        r = by(inventory(MDSTAT_RESHAPE, {"md1": DETAIL_RESHAPE}), "md1")
        self.assertTrue(r["busy"])
        self.assertEqual(r["sync"]["action"], "reshape")
        self.assertEqual(r["raid_devices"], 5)
        c = by(inventory(MDSTAT_CHECK, {"md0": DETAIL_CHECK}, devs=[]), "md0")
        self.assertTrue(c["busy"])
        self.assertEqual(c["sync"]["action"], "check")

    def test_sysfs_alone_reveals_a_check(self):
        text = MDSTAT_CHECK.replace("      [======>..............]  check = 30.0% (7549747/25165824) finish=2.1min speed=140000K/sec\n", "")
        a = by(inventory(text, {}, {"md0": {"sync_action": "check", "sync_completed": 12.5, "sync_speed": 5000, "mismatch_cnt": 0}}, devs=[]), "md0")
        self.assertTrue(a["busy"])
        self.assertEqual((a["sync"]["action"], a["sync"]["percent"]), ("check", 12.5))

    def test_foreign_by_host_and_vg(self):
        devs = devs_for() + [ld("md2", "raid5", 1, "LVM2_member", kname="md2")]
        pvs = PVS + [{"pv_name": "/dev/md2", "vg_name": "vg1000", "pv_size": 1, "pv_free": 0}]
        a = by(inventory(MDSTAT_FOREIGN, {"md2": DETAIL_FOREIGN}, devs=devs, pvs=pvs), "md2")
        self.assertTrue(a["foreign"])
        self.assertTrue(any("DiskStation" in r for r in a["foreign_reasons"]))
        self.assertTrue(any("vg1000" in r for r in a["foreign_reasons"]))

    def test_own_array_is_not_foreign_even_without_detail(self):
        a = by(inventory(MDSTAT_DEGRADED, {}), "md1")
        self.assertFalse(a["foreign"])

    def test_inactive_array(self):
        a = by(inventory("Personalities : [raid5]\nmd127 : inactive sdb[1](S) sda[0](S)\n      976630488 blocks super 1.2\n\nunused devices: <none>\n", {}, devs=[]), "md127")
        self.assertTrue(a["inactive"])
        self.assertEqual(a["state"], "inactive")


# ---------------------------------------------------------------------------------------------------------------------
# SHR slices
# ---------------------------------------------------------------------------------------------------------------------

class ShrSliceTests(unittest.TestCase):
    VECTORS = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "shr_vectors.json")))["vectors"]

    def test_vectors(self):
        for v in self.VECTORS:
            res = m.shr_slices({k: x * MIB for k, x in v["disks"].items()}, v["mode"])
            self.assertEqual(res["usable_bytes"], v["usable"] * MIB, v["name"])
            got = [{"members": s["members"], "length": s["length"] // MIB, "level": s["level"], "usable": s["usable"] // MIB} for s in res["slices"]]
            self.assertEqual(got, [{"members": s["members"], "length": s["length"], "level": s["level"], "usable": s["usable"]} for s in v["slices"]], v["name"])
            if "warnings" in v:
                self.assertEqual(len(res["warnings"]), v["warnings"] + sum(1 for s in v["slices"] if s["level"] is None), v["name"])

    def test_spec_examples_totals(self):
        T = 1000 * GIB
        self.assertEqual(m.shr_slices({"a": 4 * T, "b": 4 * T, "c": 8 * T}, "shr1")["usable_bytes"], 8 * T)
        self.assertEqual(m.shr_slices({"a": 2 * T, "b": 4 * T, "c": 4 * T, "d": 8 * T}, "shr1")["usable_bytes"], 10 * T)
        self.assertEqual(m.shr_slices({"a": T, "b": 2 * T, "c": 2 * T, "d": 3 * T}, "shr1")["usable_bytes"], 5 * T)

    def test_unusable_slice_is_reported_never_used(self):
        r = m.shr_slices({"a": 4000 * GIB, "b": 4000 * GIB, "c": 8000 * GIB}, "shr1")
        top = r["slices"][-1]
        self.assertTrue(top["unusable"])
        self.assertEqual((top["level"], top["usable"]), (None, 0))
        self.assertTrue(any("unused" in w for w in r["warnings"]))

    def test_minimum_disks_and_mode(self):
        for sizes, mode in (({"a": GIB * 100}, "shr1"), ({"a": 1, "b": 2, "c": 3}, "shr2"), ({"a": GIB, "b": GIB}, "raid5")):
            with self.assertRaises(m.ValidationError):
                m.shr_slices(sizes, mode)

    def test_shr2_fallback_warns(self):
        r = m.shr_slices({"a": 4000 * GIB, "b": 4000 * GIB, "c": 8000 * GIB, "d": 8000 * GIB}, "shr2")
        self.assertEqual(r["slices"][1]["level"], "raid1")
        self.assertIn("not 2", r["slices"][1]["note"])
        self.assertTrue(any("not 2" in w for w in r["warnings"]))

    def test_member_order_follows_input(self):
        r = m.shr_slices({"/dev/sdz": 4000 * GIB, "/dev/sda": 4000 * GIB, "/dev/sdm": 8000 * GIB}, "shr1")
        self.assertEqual(r["slices"][0]["members"], ["/dev/sdz", "/dev/sda", "/dev/sdm"])

    def test_part_paths(self):
        self.assertEqual(m.part_path("/dev/sda", 2), "/dev/sda2")
        self.assertEqual(m.part_path("/dev/nvme0n1", 1), "/dev/nvme0n1p1")

    def test_shr_create_plan_steps(self):
        T = 1000 * GIB
        disks = {"/dev/sda": 4 * T + 2 * MIB, "/dev/sdb": 4 * T + 2 * MIB, "/dev/sdc": 8 * T + 2 * MIB}
        p = m.plan_shr_create("media", disks, "shr1", "btrfs", "/srv/media")
        cmds = [s.display() for s in p.steps]
        self.assertEqual(p.confirm, "media")
        self.assertEqual(p.info["usable_bytes"], 8 * T)
        self.assertIn("parted -s /dev/sda mklabel gpt", cmds)
        self.assertEqual(sum(1 for c in cmds if "mkpart" in c), 3)                    # one partition per disk: slice 2 (one member) is never created
        create = [s.argv for s in p.steps if s.argv and s.argv[:2] == ["mdadm", "--create"]]
        self.assertEqual(len(create), 1)
        self.assertIn("--level=raid5", create[0])
        self.assertIn("--name=media_1", create[0])
        self.assertEqual(create[0][-3:], ["/dev/sda1", "/dev/sdb1", "/dev/sdc1"])
        self.assertIn(["vgcreate", "vg_media", "/dev/md/media_1"], [s.argv for s in p.steps])
        self.assertTrue(any(w.startswith("ALL DATA") for w in p.warnings))
        self.assertTrue(any("unused" in w for w in p.warnings))
        order = ["wipefs", "mklabel", "mkpart", "mdadm --create", "update-initramfs", "pvcreate", "vgcreate", "lvcreate", "mkfs.btrfs", "mount /srv/media"]
        pos = [next(i for i, c in enumerate(cmds) if o in c) for o in order]
        self.assertEqual(pos, sorted(pos))

    def test_shr_create_two_arrays(self):
        T = 1000 * GIB
        disks = {"/dev/sda": 2 * T + 2 * MIB, "/dev/sdb": 4 * T + 2 * MIB, "/dev/sdc": 4 * T + 2 * MIB, "/dev/sdd": 8 * T + 2 * MIB}
        p = m.plan_shr_create("tank", disks, "shr1", "xfs", "/srv/tank")
        self.assertEqual(p.info["usable_bytes"], 10 * T)
        names = [a for s in p.steps if s.argv and s.argv[:2] == ["mdadm", "--create"] for a in s.argv if a.startswith("--name=")]
        self.assertEqual(names, ["--name=tank_1", "--name=tank_2"])
        pv = next(s.argv for s in p.steps if s.argv and s.argv[0] == "vgcreate")
        self.assertEqual(pv, ["vgcreate", "vg_tank", "/dev/md/tank_1", "/dev/md/tank_2"])
        # sdd (the largest) carries two partitions, sda one
        mk = [s.argv for s in p.steps if s.argv and "mkpart" in s.argv]
        self.assertEqual(sum(1 for a in mk if a[4] == "/dev/sdd"), 2)
        self.assertEqual(sum(1 for a in mk if a[4] == "/dev/sda"), 1)

    def test_shr_create_rejects(self):
        T = 1000 * GIB
        with self.assertRaises(m.ValidationError):
            m.plan_shr_create("x;y", {"/dev/sda": T, "/dev/sdb": T}, "shr1", "xfs", "/srv/x")
        with self.assertRaises(m.ValidationError):
            m.plan_shr_create("x", {"/dev/sda": T, "/dev/sdb": T}, "shr1", "xfs", "/etc/x")
        with self.assertRaises(m.ValidationError):
            m.plan_shr_create("x", {"/dev/sda": T, "/dev/sdb": T}, "shr1", "xfs", "/srv/x", 99999)


class ShrGrowTests(unittest.TestCase):
    T = 1000 * GIB

    def layout(self, mode="shr1"):
        """media volume built from 4, 4, 8 TB: one RAID5 slice of 4 TB on all three, the 8 TB disk has 4 TB free."""
        T = self.T
        return {"volume": "media", "mode": mode, "disks": {
            "/dev/sda": {"size": 4 * T, "parts": [(1, "/dev/sda1", 4 * T)]}, "/dev/sdb": {"size": 4 * T, "parts": [(1, "/dev/sdb1", 4 * T)]},
            "/dev/sdc": {"size": 8 * T, "parts": [(1, "/dev/sdc1", 4 * T)]}},
            "slices": [{"index": 1, "array": "md2", "label": "media_1", "level": "raid5", "length": 4 * T, "members": {"sda": "/dev/sda1", "sdb": "/dev/sdb1", "sdc": "/dev/sdc1"},
                        "raid_devices": 3, "degraded": False, "busy": False}]}

    def test_add_equal_disk_grows_slice_one(self):
        p = m.plan_shr_add_disk(self.layout(), "/dev/sdd", 4 * self.T + 2 * MIB)
        cmds = [s.display() for s in p.steps]
        self.assertTrue(any(c.startswith("parted -s -a optimal /dev/sdd mkpart media_1") for c in cmds))
        self.assertIn("mdadm --manage /dev/md2 --add /dev/sdd1", cmds)
        self.assertTrue(any(c.startswith("mdadm --grow /dev/md2 --raid-devices=4") for c in cmds))
        self.assertFalse(any("--create" in c for c in cmds))
        self.assertEqual(p.info["joined_slices"], [1])
        self.assertEqual(p.info["new_slices"], [])
        self.assertEqual(p.confirm, "media")
        self.assertEqual(p.follow_ups[0].argv[0], "pvresize")

    def test_add_big_disk_joins_and_creates_a_mirror_of_the_tails(self):
        p = m.plan_shr_add_disk(self.layout(), "/dev/sdd", 8 * self.T + 2 * MIB)
        creates = [s.argv for s in p.steps if s.argv and s.argv[:2] == ["mdadm", "--create"]]
        self.assertEqual(len(creates), 1)
        self.assertIn("--name=media_2", creates[0])
        self.assertIn("--level=raid1", creates[0])
        self.assertEqual(creates[0][-2:], ["/dev/sdc2", "/dev/sdd2"])
        self.assertIn(["vgextend", "vg_media", "/dev/md/media_2"], [s.argv for s in p.steps])
        self.assertEqual(p.info["new_slices"], ["media_2"])

    def test_add_small_disk_makes_no_change_when_nothing_fits(self):
        with self.assertRaises(m.SafetyError):
            m.plan_shr_add_disk(self.layout(), "/dev/sdd", GIB // 2)

    def test_add_small_disk_pairs_with_the_free_tail(self):
        p = m.plan_shr_add_disk(self.layout(), "/dev/sdd", self.T // 2)
        c = next(s.argv for s in p.steps if s.argv and s.argv[:2] == ["mdadm", "--create"])
        self.assertIn("--name=media_2", c)
        self.assertEqual(c[-2:], ["/dev/sdc2", "/dev/sdd1"])
        self.assertEqual(p.info["joined_slices"], [])

    def test_mirror_slice_becomes_raid5_with_third_disk(self):
        lay = self.layout()
        lay["disks"].pop("/dev/sdb")
        lay["slices"][0].update(level="raid1", members={"sda": "/dev/sda1", "sdc": "/dev/sdc1"}, raid_devices=2)
        p = m.plan_shr_add_disk(lay, "/dev/sdd", 4 * self.T + 2 * MIB)
        cmds = [s.display() for s in p.steps]
        i = [c.startswith("mdadm --grow /dev/md2 --level=5") for c in cmds].index(True)
        self.assertIn("mdadm --manage /dev/md2 --add /dev/sdd1", cmds[i:])
        self.assertTrue(any("--raid-devices=3" in c for c in cmds))

    def test_refuses_while_degraded_or_busy(self):
        for key in ("degraded", "busy"):
            lay = self.layout()
            lay["slices"][0][key] = True
            with self.assertRaises(m.SafetyError):
                m.plan_shr_add_disk(lay, "/dev/sdd", 4 * self.T + 2 * MIB)

    def test_refuses_disk_already_in_volume(self):
        with self.assertRaises(m.SafetyError):
            m.plan_shr_add_disk(self.layout(), "/dev/sda", 4 * self.T)

    def test_replace_healthy_disk_uses_replace_with(self):
        p = m.plan_shr_replace_disk(self.layout(), "/dev/sda", "/dev/sdd", 4 * self.T + 2 * MIB, False)
        cmds = [s.display() for s in p.steps]
        self.assertIn("mdadm --manage /dev/md2 --add /dev/sdd1", cmds)
        self.assertIn("mdadm --manage /dev/md2 --replace /dev/sda1 --with /dev/sdd1", cmds)
        self.assertEqual([s.display() for s in p.follow_ups], ["mdadm --manage /dev/md2 --remove /dev/sda1"])

    def test_replace_failed_disk_removes_then_adds(self):
        p = m.plan_shr_replace_disk(self.layout(), "/dev/sda", "/dev/sdd", 4 * self.T + 2 * MIB, True)
        cmds = [s.display() for s in p.steps if s.argv and s.argv[0] == "mdadm"]
        self.assertEqual(cmds, ["mdadm --manage /dev/md2 --fail /dev/sda1", "mdadm --manage /dev/md2 --remove /dev/sda1", "mdadm --manage /dev/md2 --add /dev/sdd1"])

    def test_replace_refuses_smaller_disk_and_degraded(self):
        with self.assertRaises(m.SafetyError):
            m.plan_shr_replace_disk(self.layout(), "/dev/sda", "/dev/sdd", 3 * self.T, False)
        lay = self.layout()
        lay["slices"][0]["degraded"] = True
        with self.assertRaises(m.SafetyError):
            m.plan_shr_replace_disk(lay, "/dev/sda", "/dev/sdd", 4 * self.T + 2 * MIB, False)

    def test_layout_from_inventory(self):
        devs = []
        for d in "abc":
            devs.append(ld("sd" + d, "disk", 4 * self.T, model="M", serial="s" + d, children=["sd%s1" % d]))
            devs.append(ld("sd%s1" % d, "part", 4 * self.T, "linux_raid_member", pk="sd" + d))
        det = DETAIL_CLEAN.replace("ds918plus:1", "ds918plus:media_1").replace("raid_dummy", "")
        det = det.replace("/dev/sda2", "/dev/sda1").replace("/dev/sdb2", "/dev/sdb1").replace("/dev/sdc2", "/dev/sdc1").replace("      Raid Devices : 4", "      Raid Devices : 3")
        arrs = m.build_array_inventory(m.parse_mdstat(MDSTAT_FOREIGN.replace("md2", "md3").replace("sdh5", "sdc1").replace("sdg5", "sdb1").replace("sdf5", "sda1")),
                                       {"md3": m.parse_mdadm_detail(det.replace("/dev/md1", "/dev/md3"))}, {}, devs,
                                       [{"pv_name": "/dev/md3", "vg_name": "vg_media", "pv_size": 1, "pv_free": 0}], [], "ds918plus")
        lay = m.shr_layout_from_arrays(arrs, "media")
        self.assertEqual([s["array"] for s in lay["slices"]], ["md3"])
        self.assertEqual(set(lay["disks"]), {"sda", "sdb", "sdc"})
        self.assertEqual(lay["mode"], "shr1")


# ---------------------------------------------------------------------------------------------------------------------
# operation plans and safety rules (pure)
# ---------------------------------------------------------------------------------------------------------------------

def arr(level="raid5", n=4, degraded=False, busy=False, **kw):
    members = [{"device": "/dev/sd%s2" % c, "name": "sd%s2" % c, "slot": i, "state": "active", "size": 3900 * GIB, "disk": "sd" + c} for i, c in enumerate("abcdefgh"[:n])]
    a = {"name": "md1", "device": "/dev/md1", "label": "1", "level": level, "degraded": degraded, "busy": busy, "sync": {"action": "resync"} if busy else None,
         "raid_devices": n, "members": members, "spares": [], "failed": [], "used_dev_size": 3900 * GIB, "mounts": [], "lvs": [], "vgs": [], "holds_system": False,
         "inactive": False, "foreign": False, "foreign_reasons": []}
    a.update(kw)
    return a


class PlanTests(unittest.TestCase):
    def test_create_plan_order_and_options(self):
        p = m.plan_raid_create("scratch", "raid5", ["/dev/sda", "/dev/sdb", "/dev/sdc"], ["/dev/sdd"], 512, True, {d: 4000 * GIB for d in ("/dev/sda", "/dev/sdb", "/dev/sdc")})
        cmds = [s.display() for s in p.steps]
        create = next(s.argv for s in p.steps if s.argv[:2] == ["mdadm", "--create"])
        self.assertEqual(create, ["mdadm", "--create", "/dev/md/scratch", "--run", "--level=raid5", "--raid-devices=3", "--metadata=1.2", "--name=scratch", "--spare-devices=1",
                                  "--chunk=512", "--bitmap=internal", "/dev/sda", "/dev/sdb", "/dev/sdc", "/dev/sdd"])
        self.assertEqual(p.info["usable_bytes"], 8000 * GIB)
        self.assertEqual(p.info["tolerance"], 1)
        pos = [next(i for i, c in enumerate(cmds) if o in c) for o in ("wipefs", "mdadm --create", "mdadm --detail --scan", "update-initramfs")]
        self.assertEqual(pos, sorted(pos))
        self.assertEqual(p.confirm, "scratch")

    def test_create_with_volume(self):
        p = m.plan_raid_create("media", "raid1", ["/dev/sda", "/dev/sdb"], [], None, True, {}, {"fs": "ext4", "mountpoint": "/srv/media"})
        argvs = [s.argv for s in p.steps if s.argv]
        self.assertIn(["pvcreate", "--yes", "/dev/md/media"], argvs)
        self.assertIn(["vgcreate", "vg_media", "/dev/md/media"], argvs)
        self.assertTrue(any(a[0] == "mkfs.ext4" for a in argvs))

    def test_create_validation(self):
        for kw in ({"label": "A b"}, {"level": "raid7"}, {"devices": ["/dev/sda"], "level": "raid1"}, {"devices": ["/dev/sda", "/dev/sda"], "level": "raid1"},
                   {"level": "raid10", "devices": ["/dev/sda", "/dev/sdb", "/dev/sdc"]}, {"level": "raid0", "spares": ["/dev/sdc"]}, {"chunk": 100}):
            a = dict(label="x", level="raid1", devices=["/dev/sda", "/dev/sdb"], spares=[], chunk=None)
            a.update(kw)
            with self.assertRaises(m.ValidationError, msg=str(kw)):
                m.plan_raid_create(a["label"], a["level"], a["devices"], a["spares"], a["chunk"], True, {})

    def test_add_spare(self):
        p = m.plan_add_spare(arr(), "/dev/sde2", {"size": 4000 * GIB})
        self.assertEqual([s.argv for s in p.steps], [["mdadm", "--manage", "/dev/md1", "--add", "/dev/sde2"]])
        self.assertEqual(p.confirm, "md1")

    def test_fail_and_remove_rules(self):
        self.assertEqual(m.plan_fail(arr(), "/dev/sdb2").steps[0].argv, ["mdadm", "--manage", "/dev/md1", "--fail", "/dev/sdb2"])
        with self.assertRaises(m.SafetyError):
            m.plan_fail(arr(degraded=True, members=[dict(m_, state="faulty" if i == 0 else "active") for i, m_ in enumerate(arr()["members"])]), "/dev/sdb2")   # would kill raid5
        with self.assertRaises(m.SafetyError):
            m.plan_fail(arr(level="raid0"), "/dev/sda2")
        with self.assertRaises(m.SafetyError):
            m.plan_fail(arr(busy=True), "/dev/sda2")
        with self.assertRaises(m.ValidationError):
            m.plan_fail(arr(), "/dev/sdz9")
        with self.assertRaises(m.SafetyError):
            m.plan_remove(arr(), "/dev/sda2")                               # working members must be failed first
        faulty = arr(members=[dict(m_, state="faulty") if i == 0 else m_ for i, m_ in enumerate(arr()["members"])])
        self.assertEqual(m.plan_remove(faulty, "/dev/sda2").steps[0].argv, ["mdadm", "--manage", "/dev/md1", "--remove", "/dev/sda2"])

    def test_raid6_and_raid1_fail_limits(self):
        r6 = arr("raid6", 4)
        m.plan_fail(r6, "/dev/sda2")
        r6d = arr("raid6", 4, members=[dict(m_, state="faulty") if i < 2 else m_ for i, m_ in enumerate(arr("raid6", 4)["members"])])
        with self.assertRaises(m.SafetyError):
            m.plan_fail(r6d, "/dev/scc2".replace("scc", "sdc"))
        r1 = arr("raid1", 2)
        m.plan_fail(r1, "/dev/sda2")
        self.assertEqual((m.min_active("raid5", 4), m.min_active("raid6", 6), m.min_active("raid1", 3), m.min_active("raid0", 2)), (3, 4, 1, 2))
        self.assertEqual((m.fault_tolerance("raid5", 4), m.fault_tolerance("raid6", 6), m.fault_tolerance("raid1", 3), m.fault_tolerance("raid0", 2), m.fault_tolerance("raid10", 4)), (1, 2, 2, 0, 1))

    def test_replace_healthy_uses_replace_with(self):
        p = m.plan_replace(arr(), "/dev/sdb2", "/dev/sde2", {})
        self.assertEqual([s.argv for s in p.steps], [["mdadm", "--manage", "/dev/md1", "--add", "/dev/sde2"], ["mdadm", "--manage", "/dev/md1", "--replace", "/dev/sdb2", "--with", "/dev/sde2"]])
        self.assertEqual(p.follow_ups[0].argv, ["mdadm", "--manage", "/dev/md1", "--remove", "/dev/sdb2"])
        self.assertEqual(p.info["mode"], "healthy")

    def test_replace_faulty_removes_and_adds(self):
        a = arr(degraded=True, members=[dict(m_, state="faulty") if i == 1 else m_ for i, m_ in enumerate(arr()["members"])])
        p = m.plan_replace(a, "/dev/sdb2", "/dev/sde2", {})
        self.assertEqual([s.argv[3] for s in p.steps], ["--remove", "--add"])
        self.assertEqual(p.info["mode"], "failed")

    def test_replace_refusals(self):
        with self.assertRaises(m.SafetyError):
            m.plan_replace(arr(degraded=True), "/dev/sdb2", "/dev/sde2", {})        # healthy-replace on a degraded array
        with self.assertRaises(m.SafetyError):
            m.plan_replace(arr(busy=True), "/dev/sdb2", "/dev/sde2", {})
        with self.assertRaises(m.SafetyError):
            m.plan_replace(arr("raid0", 2), "/dev/sda2", "/dev/sde2", {})

    def test_grow_add_disk_raid5(self):
        p = m.plan_grow(arr(), ["/dev/sde2"], False, "/var/lib/synofurb-mgmt", [])
        self.assertEqual(p.steps[0].argv, ["mdadm", "--manage", "/dev/md1", "--add", "/dev/sde2"])
        self.assertEqual(p.steps[1].argv, ["mdadm", "--grow", "/dev/md1", "--raid-devices=5", "--backup-file=/var/lib/synofurb-mgmt/md-md1.backup"])
        self.assertTrue(p.info["after_reshape"])

    def test_grow_raid1_has_no_backup_file(self):
        p = m.plan_grow(arr("raid1", 2), ["/dev/sdc2"], False, "/s", [])
        self.assertEqual(p.steps[-1].argv, ["mdadm", "--grow", "/dev/md1", "--raid-devices=3"])

    def test_grow_size_max_and_followups(self):
        p = m.plan_grow(arr(), [], True, "/s", [m.Step("x", ["pvresize", "/dev/md1"])])
        self.assertEqual([s.argv for s in p.steps], [["mdadm", "--grow", "/dev/md1", "--size=max"]])
        self.assertEqual(p.follow_ups[0].argv, ["pvresize", "/dev/md1"])

    def test_grow_refusals(self):
        for a, add, mx in ((arr(busy=True), ["/dev/sde2"], False), (arr(degraded=True), ["/dev/sde2"], False), (arr("raid0", 2), ["/dev/sde2"], False),
                           (arr("raid10", 4), ["/dev/sde2"], False), (arr(), [], False), (arr(), ["/dev/sde2"], True)):
            with self.assertRaises(m.ValidationError, msg=a["level"]):
                m.plan_grow(a, add, mx, "/s", [])

    def test_after_grow_followups(self):
        a = arr(lvs=["vg_data/data"], vgs=["vg_data"])
        devs = [ld("vg_data-data", "lvm", 1, "xfs", ["/srv/data"], kname="dm-0")]
        steps = m.after_grow_follow_ups(a, LVS, devs)
        self.assertEqual([s.argv for s in steps], [["pvresize", "/dev/md1"], ["lvextend", "-l", "+100%FREE", "/dev/vg_data/data"], ["xfs_growfs", "/srv/data"]])

    def test_change_level_table(self):
        p = m.plan_change_level(arr("raid5", 4), "raid6", ["/dev/sde2"], "/s")
        self.assertEqual(p.steps[-1].argv, ["mdadm", "--grow", "/dev/md1", "--level=6", "--raid-devices=5", "--backup-file=/s/md-md1.backup"])
        self.assertEqual(p.steps[0].argv[3], "--add")
        p = m.plan_change_level(arr("raid6", 5), "raid5", [], "/s")
        self.assertIn("--raid-devices=4", p.steps[-1].argv)
        p = m.plan_change_level(arr("raid1", 2), "raid5", [], "/s")
        self.assertEqual(p.steps[-1].argv[:4], ["mdadm", "--grow", "/dev/md1", "--level=5"])
        self.assertNotIn("--raid-devices", " ".join(p.steps[-1].argv))
        p = m.plan_change_level(arr("raid1", 2), "raid5", ["/dev/sdc2"], "/s")
        self.assertIn("--raid-devices=3", p.steps[-1].argv)
        p = m.plan_change_level(arr("raid0", 2), "raid5", ["/dev/sdc2"], "/s")
        self.assertIn("--raid-devices=3", p.steps[-1].argv)

    def test_change_level_refusals(self):
        for a, tgt, add in ((arr("raid5", 4), "raid1", []), (arr("raid10", 4), "raid5", []), (arr("raid5", 4), "raid6", []),
                            (arr("raid1", 3), "raid5", []), (arr("raid5", 4, degraded=True), "raid6", ["/dev/sde2"]), (arr("raid5", 4, busy=True), "raid6", ["/dev/sde2"]),
                            (arr("raid5", 4), "raid0", [])):
            with self.assertRaises(m.SafetyError, msg="%s -> %s" % (a["level"], tgt)):
                m.plan_change_level(a, tgt, add, "/s")

    def test_scrub(self):
        p = m.plan_scrub(arr("raid1", 2), "check")
        self.assertEqual((p.steps[0].kind, p.steps[0].meta), ("sysfs", {"path": "/sys/block/md1/md/sync_action", "value": "check"}))
        self.assertIsNone(p.confirm)
        self.assertEqual(p.steps[0].display(), "echo check > /sys/block/md1/md/sync_action")
        self.assertTrue(m.plan_scrub(arr("raid5"), "repair").info["writes"])
        m.plan_scrub(arr(busy=True), "idle")                                         # stopping is always allowed
        for a, act in ((arr(busy=True), "check"), (arr("raid0", 2), "check"), (arr(degraded=True), "repair"), (arr(inactive=True), "check")):
            with self.assertRaises(m.SafetyError):
                m.plan_scrub(a, act)
        with self.assertRaises(m.ValidationError):
            m.plan_scrub(arr(), "format")

    def test_stop_refusals(self):
        self.assertEqual(m.plan_stop(arr()).steps[0].argv, ["mdadm", "--stop", "/dev/md1"])
        for kw in ({"mounts": ["/srv/data"]}, {"lvs": ["vg_data/data"]}, {"holds_system": True}, {"busy": True}):
            with self.assertRaises(m.SafetyError, msg=str(kw)):
                m.plan_stop(arr(**kw))

    def test_tuning(self):
        p = m.plan_tuning(arr(), {"speed_min": 50000, "speed_max": "system", "stripe_cache": 1024, "bitmap": "none"})
        self.assertEqual([(s.kind, s.meta.get("value")) for s in p.steps if s.kind == "sysfs"], [("sysfs", "50000"), ("sysfs", "system"), ("sysfs", "1024")])
        self.assertEqual(p.steps[-1].argv, ["mdadm", "--grow", "/dev/md1", "--bitmap=none"])
        for body in ({}, {"speed_min": -1}, {"speed_min": "fast"}, {"speed_max": 10 ** 9}, {"stripe_cache": 3}, {"speed_min": True}):
            with self.assertRaises(m.ValidationError, msg=str(body)):
                m.plan_tuning(arr(), body)
        with self.assertRaises(m.ValidationError):
            m.plan_tuning(arr("raid1", 2), {"stripe_cache": 256})
        with self.assertRaises(m.SafetyError):
            m.plan_tuning(arr("raid0", 2), {"bitmap": "internal"})

    def test_scrub_schedule_units(self):
        p = m.plan_tuning(None, {"scrub_schedule": "enable"})
        kinds = [s.kind for s in p.steps]
        self.assertEqual(kinds[:2], ["unitfile", "unitfile"])
        self.assertIn("OnCalendar=monthly", p.steps[1].meta["content"])
        self.assertIn("--raid-scrub", p.steps[0].meta["content"])
        self.assertEqual(p.steps[-1].argv, ["systemctl", "enable", "--now", "synofurb-raid-scrub.timer"])
        self.assertEqual(m.plan_tuning(None, {"scrub_schedule": "disable"}).steps[0].argv[:2], ["systemctl", "disable"])

    def test_assemble_plan(self):
        self.assertEqual(m.plan_assemble().steps[0].argv, ["mdadm", "--assemble", "--scan"])

    def test_level_transition_table_is_only_what_mdadm_supports(self):
        self.assertEqual(sorted(m.LEVEL_TRANSITIONS), [("raid0", "raid5"), ("raid1", "raid5"), ("raid5", "raid6"), ("raid6", "raid5")])

    def test_sysfs_step_refuses_odd_paths(self):
        app = m.App(m.Config(root="/nonexistent-root"), FakeRunner())
        for path, val in (("/etc/passwd", "check"), ("/sys/block/md1/md/sync_action", "check; reboot"), ("/sys/block/md1/../md0/md/sync_action", "idle"),
                          ("/sys/block/sda/md/sync_action", "idle")):
            r = m.exec_step(app, m.Step("x", None, kind="sysfs", meta={"path": path, "value": val}))
            self.assertFalse(r.ok, path)
        r = m.exec_step(app, m.Step("x", None, kind="unitfile", meta={"path": "/etc/cron.d/evil", "content": "x"}))
        self.assertFalse(r.ok)


# ---------------------------------------------------------------------------------------------------------------------
# HTTP routes
# ---------------------------------------------------------------------------------------------------------------------

def raid_lsblk():
    ls = json.loads(json.dumps(LSBLK))
    ls["blockdevices"] = [d for d in ls["blockdevices"]]
    return ls


class RaidHttp(HttpTestCase):
    """A healthy RAID5 md1 (sda2..sdd2) on vg_data, plus free disks sde/sdg and a foreign DSM array md2."""
    DETAIL = {"md1": DETAIL_CLEAN, "md2": DETAIL_FOREIGN}
    MDSTAT = "Personalities : [raid5]\n" + MDSTAT_DEGRADED.split("\n", 1)[1].split("\n\nunused")[0].replace("sde2[4] ", "").replace("[4/3] [UU_U]", "[4/4] [UUUU]").replace("sdb2[1] sda2[0]", "sdc2[2] sdb2[1] sda2[0]").split("\n      [")[0] + "\n\n" + MDSTAT_FOREIGN.split("\n", 1)[1]

    def lsblk_json(self):
        devs = []
        md1 = {"name": "md1", "kname": "md1", "type": "raid5", "size": 11706543104 * 1024, "fstype": "LVM2_member", "mountpoints": [None], "pkname": None, "children": [
            {"name": "vg_data-data", "kname": "dm-0", "type": "lvm", "size": 9 * 1024 ** 4, "fstype": "xfs", "mountpoints": ["/srv/data"], "pkname": "md1"}]}
        for c in "abcd":
            devs.append({"name": "sd" + c, "kname": "sd" + c, "type": "disk", "size": 4000787030016, "fstype": None, "mountpoints": [None], "model": "WD40EFRX", "serial": "S" + c, "rota": True, "tran": "sata", "pkname": None, "children": [
                {"name": "sd%s2" % c, "kname": "sd%s2" % c, "type": "part", "size": 3900 * GIB, "fstype": "linux_raid_member", "mountpoints": [None], "pkname": "sd" + c, "children": [md1]}]})
        for c in "efg":
            devs.append({"name": "sd" + c, "kname": "sd" + c, "type": "disk", "size": 4000787030016, "fstype": None, "mountpoints": [None], "model": "ST4000", "serial": "S" + c, "rota": True, "tran": "sata", "pkname": None})
        md2 = {"name": "md2", "kname": "md2", "type": "raid5", "size": 1, "fstype": "LVM2_member", "mountpoints": [None], "pkname": None, "children": []}
        for c in "hij":
            devs.append({"name": "sd" + c, "kname": "sd" + c, "type": "disk", "size": 4000787030016, "fstype": None, "mountpoints": [None], "pkname": None, "children": [
                {"name": "sd%s5" % c, "kname": "sd%s5" % c, "type": "part", "size": 3900 * GIB, "fstype": "linux_raid_member", "mountpoints": [None], "pkname": "sd" + c, "children": [md2]}]})
        return json.dumps({"blockdevices": devs})

    def setUp(self):
        super().setUp()
        cfg = self.env.cfg
        cfg.proc_root, cfg.sys_root = os.path.join(self.env.tmp, "proc"), os.path.join(self.env.tmp, "sys")
        os.makedirs(cfg.proc_root)
        open(os.path.join(cfg.proc_root, "mdstat"), "w").write(self.MDSTAT)
        for n in ("md1", "md2"):
            base = os.path.join(cfg.sys_root, "block", n, "md")
            os.makedirs(base)
            for f, v in (("sync_action", "idle"), ("mismatch_cnt", "0"), ("degraded", "0"), ("array_state", "clean")):
                open(os.path.join(base, f), "w").write(v + "\n")
        r = self.runner
        r.on(["lsblk"], self.lsblk_json())
        r.on(["mdadm", "--detail", "/dev/md1"], DETAIL_CLEAN).on(["mdadm", "--detail", "/dev/md2"], DETAIL_FOREIGN)
        r.on(["pvs"], json.dumps({"report": [{"pv": [{"pv_name": "/dev/md1", "vg_name": "vg_data", "pv_size": "1", "pv_free": "0"}, {"pv_name": "/dev/md2", "vg_name": "vg1000", "pv_size": "1", "pv_free": "0"}]}]}))
        r.on(["lvs"], json.dumps({"report": [{"lv": [{"lv_name": "data", "vg_name": "vg_data", "lv_size": "1", "lv_path": "/dev/vg_data/data", "lv_attr": "-wi-ao----"}]}]}))
        r.on(["systemctl", "is-enabled"], "enabled\n").on(["systemctl", "is-active"], "active\n")
        self.logged_in()
        self._socket = m.socket.gethostname
        m.socket.gethostname = lambda: "ds918plus"
        self.addCleanup(lambda: setattr(m.socket, "gethostname", self._socket))

    def post(self, op, body=None, plan=False):
        if plan:
            return self.cl.call("POST", "/api/raid/plan", dict(body or {}, op=op))
        return self.cl.call("POST", "/api/raid/" + op, body or {})

    def sysfs(self, name, f):
        return open(os.path.join(self.env.cfg.sys_root, "block", name, "md", f)).read()

    def test_inventory_route(self):
        st, o, _ = self.cl.call("GET", "/api/raid")
        self.assertEqual(st, 200, o)
        arrs = {a["name"]: a for a in o["arrays"]}
        self.assertEqual(set(arrs), {"md1", "md2"})
        self.assertFalse(arrs["md1"]["foreign"])
        self.assertTrue(arrs["md2"]["foreign"])
        self.assertEqual(arrs["md1"]["lvs"], ["vg_data/data"])
        self.assertEqual(arrs["md1"]["mismatch_cnt"], 0)
        self.assertEqual({c["path"] for c in o["candidates"]}, {"/dev/sde", "/dev/sdf", "/dev/sdg"} & {c["path"] for c in o["candidates"]})
        self.assertTrue(o["available"])
        self.assertEqual(o["scrub_timer"]["enabled"], "enabled")
        self.assertIn("/dev/sde", {c["path"] for c in o["candidates"]})

    def test_unauthenticated(self):
        st, _, _ = Client(self.env).raw("GET", "/api/raid")
        self.assertEqual(st, 401)

    def test_no_mdadm(self):
        self.runner.avail.discard("mdadm")
        self.assertEqual(self.post("scrub", {"array": "md1"})[0], 409)
        self.assertFalse(self.cl.call("GET", "/api/raid")[1]["available"])

    def test_plan_is_read_only(self):
        n = len(self.runner.calls)
        st, o, _ = self.post("add-spare", {"array": "md1", "device": "/dev/sde"}, plan=True)
        self.assertEqual(st, 200, o)
        self.assertEqual(o["confirm"], "md1")
        self.assertEqual(o["steps"][0]["cmd"], "mdadm --manage /dev/md1 --add /dev/sde")
        for c in self.runner.calls[n:]:
            self.assertIn(c[0], ("lsblk", "mdadm", "pvs", "lvs", "vgs", "systemctl"))
            self.assertFalse(c[:3] == ["mdadm", "--manage", "/dev/md1"])

    def test_add_spare_needs_typed_confirm_then_runs(self):
        body = {"array": "md1", "device": "/dev/sde"}
        for c in (None, "", "md2", "MD1"):
            self.assertEqual(self.post("add-spare", dict(body, confirm=c) if c is not None else body)[0], 400)
        self.assertFalse(self.runner.called("mdadm", "--manage", "/dev/md1", "--add"))
        st, o, _ = self.post("add-spare", dict(body, confirm="md1"))
        self.assertEqual(st, 200, o)
        self.assertTrue(self.runner.called("mdadm", "--manage", "/dev/md1", "--add", "/dev/sde"))
        self.assertIn("/api/raid/add-spare", self.env.read("/var/log/synofurb-mgmt/audit.log"))

    def test_device_rules(self):
        for dev in ("/dev/sda2", "/dev/nvme0n1", "/dev/sdz", "sde", "/dev/sde;reboot", "/dev/sdh5"):
            st, _, _ = self.post("add-spare", {"array": "md1", "device": dev, "confirm": "md1"})
            self.assertIn(st, (400, 409), dev)                                   # member of an array / system disk / unknown / malformed
        self.assertFalse(self.runner.called("mdadm", "--manage"))
        self.assertEqual(self.post("add-spare", {"array": "md99", "device": "/dev/sde", "confirm": "md99"})[0], 404)
        self.assertEqual(self.post("add-spare", {"array": "../x", "device": "/dev/sde"})[0], 400)

    def test_fail_refused_when_it_would_lose_data(self):
        open(os.path.join(self.env.cfg.proc_root, "mdstat"), "w").write("Personalities : [raid5]\nmd1 : active raid5 sdd2[3] sdb2[1] sda2[0]\n      11706543104 blocks super 1.2 level 5, 512k chunk, algorithm 2 [4/3] [UU_U]\n\nunused devices: <none>\n")
        self.runner.on(["mdadm", "--detail", "/dev/md1"], DETAIL_DEGRADED.replace("spare rebuilding   /dev/sde2", "removed").replace("       4       8       66        2      removed",
                                                                                       "       -       0        0        2      removed").replace("State : clean, degraded, recovering", "State : clean, degraded").replace("    Rebuild Status : 41% complete\n", ""))
        st, o, _ = self.post("fail", {"array": "md1", "device": "/dev/sda2", "confirm": "md1"})
        self.assertEqual(st, 409, o)
        self.assertIn("unreadable", o["error"])
        self.assertFalse(self.runner.called("mdadm", "--manage", "/dev/md1", "--fail"))

    def test_busy_array_refuses_changes_but_allows_idle(self):
        open(os.path.join(self.env.cfg.proc_root, "mdstat"), "w").write(MDSTAT_RESHAPE + "\nmd2 : active raid5 sdj5[2] sdi5[1] sdh5[0]\n      7813772288 blocks super 1.2 [3/3] [UUU]\n")
        self.runner.on(["mdadm", "--detail", "/dev/md1"], DETAIL_RESHAPE)
        for op, body in (("grow", {"add": ["/dev/sde"]}), ("change-level", {"target": "raid6", "add": ["/dev/sde"]}), ("replace", {"old": "/dev/sda2", "new": "/dev/sde"}),
                         ("scrub", {"action": "check"}), ("stop", {})):
            st, o, _ = self.post(op, dict(body, array="md1", confirm="md1"))
            self.assertEqual(st, 409, (op, o))
        st, o, _ = self.post("scrub", {"array": "md1", "action": "idle"})
        self.assertEqual(st, 200, o)
        self.assertEqual(self.sysfs("md1", "sync_action"), "idle\n")

    def test_scrub_writes_sysfs_without_confirmation(self):
        st, o, _ = self.post("scrub", {"array": "md1", "action": "check"})
        self.assertEqual(st, 200, o)
        self.assertEqual(self.sysfs("md1", "sync_action"), "check\n")
        self.assertEqual(self.post("scrub", {"array": "md1", "action": "format"})[0], 400)

    def test_replace_flow(self):
        st, o, _ = self.post("replace", {"array": "md1", "old": "/dev/sdb2", "new": "/dev/sde", "confirm": "md1"})
        self.assertEqual(st, 200, o)
        calls = self.runner.calls
        i = calls.index(["mdadm", "--manage", "/dev/md1", "--add", "/dev/sde"])
        j = calls.index(["mdadm", "--manage", "/dev/md1", "--replace", "/dev/sdb2", "--with", "/dev/sde"])
        self.assertLess(i, j)
        self.assertEqual(o["follow_ups"][0]["cmd"], "mdadm --manage /dev/md1 --remove /dev/sdb2")

    def test_grow_with_followups_offered(self):
        st, o, _ = self.post("grow", {"array": "md1", "add": ["/dev/sde"]}, plan=True)
        self.assertEqual(st, 200, o)
        cmds = [s["cmd"] for s in o["steps"]]
        self.assertIn("mdadm --grow /dev/md1 --raid-devices=5 --backup-file=/var/lib/synofurb-mgmt/md-md1.backup", cmds)
        self.assertEqual(o["follow_ups"][0]["cmd"], "pvresize /dev/md1")
        self.assertTrue(o["after_reshape"])
        self.assertEqual(self.post("grow", {"array": "md1", "add": ["/dev/sde"], "confirm": "md1"})[0], 200)
        # follow-up route refuses while busy and works once idle
        self.assertEqual(self.post("after-grow", {"array": "md1", "confirm": "md1"})[0], 200)
        self.assertTrue(self.runner.called("pvresize", "/dev/md1"))

    def test_stop_refused_for_array_in_use(self):
        st, o, _ = self.post("stop", {"array": "md1", "confirm": "md1"})
        self.assertEqual(st, 409)
        self.assertIn("mounted", o["error"])
        self.assertFalse(self.runner.called("mdadm", "--stop"))

    def test_foreign_array_rules(self):
        st, o, _ = self.post("add-spare", {"array": "md2", "device": "/dev/sde"}, plan=True)
        self.assertEqual(st, 200, o)
        self.assertTrue(o["needs_foreign_ack"])
        self.assertEqual(o["confirm"], "md2")
        self.assertIn("not created by synofurb", o["warnings"][0])
        # typed confirmation alone is not enough
        st, o, _ = self.post("add-spare", {"array": "md2", "device": "/dev/sde", "confirm": "md2"})
        self.assertEqual(st, 400)
        self.assertIn("not mine", o["error"])
        self.assertFalse(self.runner.called("mdadm", "--manage", "/dev/md2"))
        st, o, _ = self.post("add-spare", {"array": "md2", "device": "/dev/sde", "confirm": "md2", "foreign_ack": True})
        self.assertEqual(st, 200, o)
        self.assertTrue(self.runner.called("mdadm", "--manage", "/dev/md2", "--add", "/dev/sde"))

    def test_foreign_array_scrub_and_assemble_are_free(self):
        self.assertEqual(self.post("scrub", {"array": "md2", "action": "check"})[0], 200)
        self.assertEqual(self.sysfs("md2", "sync_action"), "check\n")
        self.assertEqual(self.post("assemble", {})[0], 200)
        self.assertTrue(self.runner.called("mdadm", "--assemble", "--scan"))
        self.assertEqual(self.post("scrub", {"array": "md2", "action": "idle"})[0], 200)
        st, o, _ = self.post("scrub", {"array": "md2", "action": "repair"})                 # repair writes: needs the acknowledgement
        self.assertEqual(st, 400)
        self.assertEqual(self.post("scrub", {"array": "md2", "action": "repair", "confirm": "md2", "foreign_ack": True})[0], 200)

    def test_create_plan_and_run(self):
        body = {"name": "scratch", "level": "raid1", "devices": ["/dev/sde", "/dev/sdf"]}
        st, o, _ = self.post("create", body, plan=True)
        self.assertEqual(st, 200, o)
        self.assertEqual(o["confirm"], "scratch")
        self.assertTrue(any(s["cmd"].startswith("mdadm --create /dev/md/scratch") for s in o["steps"]))
        self.assertEqual(self.post("create", dict(body, confirm="nope"))[0], 400)
        self.assertFalse(self.runner.called("wipefs"))
        st, o, _ = self.post("create", dict(body, confirm="scratch"))
        self.assertEqual(st, 200, o)
        self.assertTrue(self.runner.called("wipefs", "-a", "/dev/sde"))
        self.assertTrue(self.runner.called("update-initramfs", "-u"))

    def test_create_rejects_used_devices_and_bad_counts(self):
        for body in ({"name": "x", "level": "raid5", "devices": ["/dev/sde", "/dev/sdf"]}, {"name": "x", "level": "raid1", "devices": ["/dev/sda2", "/dev/sde"]},
                     {"name": "x", "level": "raid1", "devices": ["/dev/nvme0n1p1", "/dev/sde"]}, {"name": "md1", "level": "raid1", "devices": ["/dev/sde", "/dev/sdf"]},
                     {"name": "x", "level": "raid1", "devices": ["/dev/sde", "/dev/sdf"], "spares": ["/dev/sde"]}, {"name": "x", "level": "none", "devices": ["/dev/sde", "/dev/sdf"]}):
            st, _, _ = self.post("create", dict(body, confirm="x"))
            self.assertIn(st, (400, 409), body)
        self.assertFalse(self.runner.called("mdadm", "--create"))

    def test_shr_plan_route(self):
        st, o, _ = self.post("shr-create", {"name": "media", "mode": "shr1", "devices": ["/dev/sde", "/dev/sdf", "/dev/sdg"], "fs": "btrfs"}, plan=True)
        self.assertEqual(st, 200, o)
        self.assertEqual(len(o["shr"]["slices"]), 1)
        self.assertEqual(o["shr"]["slices"][0]["level"], "raid5")
        self.assertEqual(o["confirm"], "media")
        self.assertTrue(any("mklabel" in s["cmd"] for s in o["steps"]))
        self.assertEqual(self.post("shr-create", {"name": "media", "mode": "shr1", "devices": ["/dev/sde", "/dev/sdf"]}, plan=True)[0], 200)
        self.assertEqual(self.post("shr-create", {"name": "media", "mode": "shr2", "devices": ["/dev/sde", "/dev/sdf"]}, plan=True)[0], 400)    # SHR-2 needs 4
        self.assertEqual(self.post("shr-create", {"name": "media", "mode": "shr1", "devices": ["/dev/sde", "/dev/sda2"]}, plan=True)[0], 400)    # a partition
        self.assertEqual(self.post("shr-add-disk", {"volume": "media", "device": "/dev/sde"}, plan=True)[0], 404)                              # no such SHR volume

    def test_shr_create_executes_with_typed_name(self):
        body = {"name": "media", "mode": "shr1", "devices": ["/dev/sde", "/dev/sdf", "/dev/sdg"], "fs": "xfs"}
        self.assertEqual(self.post("shr-create", dict(body, confirm="x"))[0], 400)
        st, o, _ = self.post("shr-create", dict(body, confirm="media"))
        self.assertEqual(st, 200, o)
        self.assertTrue(self.runner.called("parted", "-s", "/dev/sde", "mklabel", "gpt"))
        self.assertTrue(any(c[:2] == ["mdadm", "--create"] and "--level=raid5" in c for c in self.runner.calls))

    def test_volume_create_accepts_shr(self):
        st, o, _ = self.cl.call("POST", "/api/volumes/plan", {"name": "media", "level": "shr1", "fs": "xfs", "devices": ["/dev/sde", "/dev/sdf", "/dev/sdg"]})
        self.assertEqual(st, 200, o)
        self.assertEqual(o["shr"]["mode"], "shr1")
        self.assertTrue(any(s["cmd"].startswith("mdadm --create /dev/md/media_1") for s in o["steps"]))
        st, o, _ = self.cl.call("POST", "/api/volumes/plan", {"name": "media", "level": "shr1", "fs": "xfs", "devices": ["/dev/sde", "/dev/sdf", "/dev/sdb2"]})
        self.assertEqual(st, 400)
        st, o, _ = self.cl.call("POST", "/api/volumes/create", {"name": "media", "level": "shr2", "fs": "xfs", "devices": ["/dev/sde", "/dev/sdf", "/dev/sdg"], "confirm": "media"})
        self.assertEqual(st, 400)                                         # SHR-2 needs 4 disks

    def test_tuning_and_schedule(self):
        st, o, _ = self.post("tuning", {"array": "md1", "speed_min": 40000, "speed_max": 200000})
        self.assertEqual(st, 200, o)
        self.assertEqual(self.sysfs("md1", "sync_speed_min"), "40000\n")
        self.assertEqual(self.sysfs("md1", "sync_speed_max"), "200000\n")
        self.assertEqual(self.post("tuning", {"array": "md1", "speed_min": "fast"})[0], 400)
        st, o, _ = self.post("tuning", {"scrub_schedule": "enable"})
        self.assertEqual(st, 200, o)
        self.assertIn("OnCalendar=monthly", self.env.read("/etc/systemd/system/synofurb-raid-scrub.timer"))
        self.assertTrue(self.runner.called("systemctl", "enable", "--now", "synofurb-raid-scrub.timer"))

    def test_change_level_route(self):
        st, o, _ = self.post("change-level", {"array": "md1", "target": "raid6", "add": ["/dev/sde"]}, plan=True)
        self.assertEqual(st, 200, o)
        self.assertTrue(any("--level=6" in s["cmd"] for s in o["steps"]))
        self.assertEqual(self.post("change-level", {"array": "md1", "target": "raid1"}, plan=True)[0], 409)

    def test_monthly_scrub_helper(self):
        n = m.raid_scrub_all(self.env.app)
        self.assertGreaterEqual(n, 1)
        self.assertEqual(self.sysfs("md1", "sync_action"), "check\n")


class StatusIntegration(unittest.TestCase):
    def test_volume_view_uses_proc_root(self):
        import tempfile
        with tempfile.TemporaryDirectory() as t:
            os.makedirs(os.path.join(t, "proc"))
            open(os.path.join(t, "proc", "mdstat"), "w").write(MDSTAT_CHECK)
            app = m.App(m.Config(root=t, proc_root=os.path.join(t, "proc")), FakeRunner())
            self.assertEqual(app.mdstat()[0]["sync"]["action"], "check")


# ---------------------------------------------------------------------------------------------------------------------
# UI copy module: word budgets (design/BRIEF-v2.md) and wiring
# ---------------------------------------------------------------------------------------------------------------------

def words(text):
    return len([w for w in text.replace("{n}", "N").replace("{name}", "NAME").split() if w])


class CopyBudgetTests(unittest.TestCase):
    LIMITS = {"title": 4, "label": 4, "button": 3, "safety": 12, "tip": 12}

    def test_every_string_obeys_its_kind(self):
        for key, e in m.COPY.items():
            kind = e["kind"]
            self.assertIn(kind, ("title", "label", "button", "tip", "prose", "safety"), key)
            if kind in self.LIMITS:
                self.assertLessEqual(words(e["short"]), self.LIMITS[kind], "%s: %r" % (key, e["short"]))
            if kind == "prose":
                self.assertLessEqual(words(e["short"]), 12, key)
            if "tip" in e:
                self.assertLessEqual(words(e["tip"]), 25, "%s tip" % key)
                self.assertTrue(e["tip"].endswith("."), key)
                self.assertEqual(e["tip"].count(". "), 0, "one sentence only: %s" % key)

    def test_buttons_start_with_a_verb_or_are_short(self):
        for key, e in m.COPY.items():
            if e["kind"] == "button":
                self.assertLessEqual(words(e["short"]), 3, key)

    def test_default_screens_stay_under_60_words_of_prose(self):
        for screen, keys in m.COPY_SCREENS.items():
            prose = sum(words(m.COPY[k]["short"]) for k in keys if m.COPY[k]["kind"] == "prose")
            self.assertLessEqual(prose, 60, screen)
            for k in keys:
                self.assertIn(k, m.COPY)
            for k in keys:                                          # page titles at most 4 words, labels at most 4
                if m.COPY[k]["kind"] == "title":
                    self.assertLessEqual(words(m.COPY[k]["short"]), 4)

    def test_help_topics(self):
        ids = [t["id"] for t in m.HELP_TOPICS]
        self.assertEqual(len(ids), len(set(ids)))
        for t in m.HELP_TOPICS:
            self.assertLessEqual(words(t["title"]), 4, t["id"])
            self.assertLessEqual(words(t["body"]), 150, t["id"])
            self.assertLessEqual(len(t["body"].split("\n\n")), 4, t["id"])
        for needed in ("raid-levels", "shr", "scrub", "replace", "grow", "foreign"):
            self.assertIn(needed, ids)

    def test_every_copy_key_used_by_the_ui_exists(self):
        import re
        html = m.INDEX_HTML
        used = set(re.findall(r"""\bT\('([a-z0-9_.]+)'""", html)) | set(re.findall(r"""TIP\('([a-z0-9_.]+)'""", html))
        used |= {"state." + k for k in ("clean", "degraded", "rebuilding", "resyncing", "reshaping", "checking", "repairing", "inactive")}
        used |= {"ms." + k for k in ("active", "spare", "faulty", "rebuilding", "removed", "replacement")}
        used |= {"lv." + k for k in ("raid0", "raid1", "raid5", "raid6", "raid10", "shr1", "shr2")}
        missing = sorted(k for k in used if k not in m.COPY and not k.endswith("."))
        self.assertEqual(missing, [])
        for hid in re.findall(r"infoBtn\('([a-z0-9-]+)'", html):
            self.assertIn(hid, [t["id"] for t in m.HELP_TOPICS])

    def test_ui_has_raid_and_help_pages_and_accessible_widgets(self):
        html = m.INDEX_HTML
        for needle in ("PAGES.raid", "PAGES.help", "['raid', 'RAID', 'layers']", "role: 'tooltip'", "role: 'menu'", "'aria-haspopup': 'menu'", "role: 'menuitem'", "ArrowDown",
                       "scrollbar-width:thin", "::-webkit-scrollbar-thumb", "function shrSlices", "Open RAID"):
            self.assertIn(needle, html)
        self.assertNotIn("show('storage')", html.split("PAGES.dashboard", 1)[1].split("/* ---------- Storage", 1)[0])    # the dashboard badge links to RAID now

    def test_js_shr_matches_python_vectors(self):
        """Run the browser implementation of the slice algorithm against the shared vectors (skipped when node is missing)."""
        import shutil
        import subprocess
        import re
        node = shutil.which("node")
        if not node:
            self.skipTest("node not available")
        src = re.search(r"const GIBN = .*?\n(function shrSlices.*?\n\})\n", m.INDEX_HTML, re.S)
        self.assertIsNotNone(src)
        vec = os.path.join(os.path.dirname(os.path.abspath(__file__)), "shr_vectors.json")
        js = ("const GIBN=1073741824,MIBN=1048576;" + src.group(1) + ";const V=JSON.parse(require('fs').readFileSync(process.argv[1])).vectors;let bad=[];"
              "for(const v of V){const d={};for(const[k,x]of Object.entries(v.disks))d[k]=x*MIBN;const r=shrSlices(d,v.mode);"
              "const got=JSON.stringify(r.slices.map(s=>({m:s.members,l:s.length/MIBN,lv:s.level,u:s.usable/MIBN})));"
              "const exp=JSON.stringify(v.slices.map(s=>({m:s.members,l:s.length,lv:s.level,u:s.usable})));"
              "if(got!==exp||r.usable!==v.usable*MIBN)bad.push(v.name)}console.log(JSON.stringify(bad));")
        out = subprocess.run([node, "-e", js, vec], capture_output=True, text=True, timeout=30)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(json.loads(out.stdout.strip().splitlines()[-1]), [])


if __name__ == "__main__":
    unittest.main()
