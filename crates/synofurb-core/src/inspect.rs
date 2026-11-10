//! Read-only inspection of a running DSM.
//!
//! [`INSPECT_SCRIPT`] is a POSIX `sh` script (DSM ships busybox-ish tools) that
//! prints one pipe-separated record per line; [`parse_inspect`] turns that into
//! [`DeviceFacts`]. The script never writes anything. Keeping the parser pure
//! lets us test it against captured output without a NAS.

use crate::model::*;
use crate::platform;
use anyhow::{Result, bail};

/// Shell helper shared with the stage-2 installer: best-effort disk serial number. Each source is tried in turn and only a
/// plausible answer (4 characters or more) is taken: on the RS812+ the installer's kernel offers `vpd_pg80` for its SATA disks
/// but it reads as just `-`, and a helper that stopped at the first source that exists never reached `udevadm`, which has the
/// real serial (0.2.9: stage 2 then "could not find" any of the four disks and stopped in its preflight, before changing anything).
pub const DISK_SERIAL_FN: &str = r#"disk_serial() {
  _d="$1"
  if [ -r "/sys/block/$_d/device/serial" ]; then
    _s=$(tr -cd '[:alnum:]_-' < "/sys/block/$_d/device/serial")
    if [ ${#_s} -ge 4 ]; then printf '%s' "$_s"; return; fi
  fi
  if [ -r "/sys/block/$_d/device/vpd_pg80" ]; then
    _s=$(dd if="/sys/block/$_d/device/vpd_pg80" bs=1 skip=4 2>/dev/null | tr -cd '[:alnum:]_-')
    if [ ${#_s} -ge 4 ]; then printf '%s' "$_s"; return; fi
  fi
  if command -v udevadm >/dev/null 2>&1; then
    _s=$(udevadm info --query=property --name="/dev/$_d" 2>/dev/null | sed -n 's/^ID_SERIAL_SHORT=//p' | head -n1 | tr -cd '[:alnum:]_-')
    if [ ${#_s} -ge 4 ]; then printf '%s' "$_s"; return; fi
  fi
  if command -v smartctl >/dev/null 2>&1; then
    _s=$(smartctl -i "/dev/$_d" 2>/dev/null | sed -n 's/^Serial [Nn]umber: *//p' | head -n1 | tr -cd '[:alnum:]_-')
    if [ ${#_s} -ge 4 ]; then printf '%s' "$_s"; return; fi
  fi
}
"#;

pub const INSPECT_SCRIPT: &str = r#"#!/bin/sh
# synofurb inspect - READ ONLY. Prints records, one per line, '|' separated.
PATH=/usr/syno/sbin:/usr/syno/bin:/usr/local/sbin:/usr/local/bin:/sbin:/bin:/usr/sbin:/usr/bin
clean() { tr -d '|\r\n' ; }
kv() { printf 'KV|%s|%s\n' "$1" "$2"; }
@@DISK_SERIAL_FN@@
kv uname_m "$(uname -m | clean)"
kv cmdline "$(clean < /proc/cmdline 2>/dev/null)"
kv kernel "$(uname -r | clean)"
kv id_u "$(id -u)"
kv hostname "$(hostname 2>/dev/null | clean)"
for f in /etc.defaults/synoinfo.conf /etc/synoinfo.conf; do
  [ -r "$f" ] || continue
  u=$(sed -n 's/^unique=//p' "$f" | head -n1 | tr -d '"' | clean)
  [ -n "$u" ] && kv unique "$u" && break
done
for f in /etc.defaults/synoinfo.conf /etc/synoinfo.conf; do
  [ -r "$f" ] || continue
  m=$(sed -n 's/^upnpmodelname=//p' "$f" | head -n1 | tr -d '"' | clean)
  [ -n "$m" ] && kv model "$m" && break
done
if [ -r /proc/sys/kernel/syno_hw_version ]; then kv hw_version "$(clean < /proc/sys/kernel/syno_hw_version)"; fi
if [ -r /proc/sys/kernel/syno_serial ]; then kv serial "$(clean < /proc/sys/kernel/syno_serial)"; fi
if [ -r /etc.defaults/VERSION ]; then
  pv=$(sed -n 's/^productversion=//p' /etc.defaults/VERSION | tr -d '"' | clean)
  bn=$(sed -n 's/^buildnumber=//p' /etc.defaults/VERSION | tr -d '"' | clean)
  kv dsm_version "$pv-$bn"
fi
kv mem_kib "$(sed -n 's/^MemTotal: *\([0-9]*\) kB.*/\1/p' /proc/meminfo)"
kv cpu "$(sed -n -e 's/^model name[^:]*: *//p' -e 's/^Hardware[^:]*: *//p' /proc/cpuinfo | head -n1 | clean)"
kv cpu_vendor "$(sed -n 's/^vendor_id[^:]*: *//p' /proc/cpuinfo | head -n1 | clean)"
if [ -d /sys/firmware/efi ]; then kv uefi 1; else kv uefi 0; fi
if [ -e /sys/kernel/kexec_loaded ]; then kv kexec 1; else kv kexec 0; fi
if [ -r /proc/device-tree/model ]; then kv dt_model "$(tr -d '\000' < /proc/device-tree/model | clean)"; fi

for n in /sys/class/net/*; do
  nm=${n##*/}
  [ "$nm" = lo ] && continue
  [ -e "$n/device" ] || continue
  printf 'NIC|%s|%s\n' "$nm" "$(clean < "$n/address")"
done

for b in /sys/block/*; do
  n=${b##*/}
  case "$n" in loop*|ram*|md*|dm-*|zram*|nbd*|sr*) continue ;; esac
  [ -e "$b/device" ] || continue
  size=$(cat "$b/size" 2>/dev/null)
  [ -n "$size" ] || continue
  model=$(clean < "$b/device/model" 2>/dev/null | sed 's/  */ /g;s/ $//')
  serial=$(disk_serial "$n" | clean)
  rot=$(cat "$b/queue/rotational" 2>/dev/null)
  rem=$(cat "$b/removable" 2>/dev/null)
  lss=$(cat "$b/queue/logical_block_size" 2>/dev/null)
  tbl=none
  sig=$(dd if="/dev/$n" bs=512 skip=1 count=1 2>/dev/null | dd bs=8 count=1 2>/dev/null)
  if [ "$sig" = "EFI PART" ]; then
    tbl=gpt
  else
    mbr=$(dd if="/dev/$n" bs=1 skip=510 count=2 2>/dev/null | od -An -tx1 | tr -d ' \n')
    [ "$mbr" = "55aa" ] && tbl=msdos
  fi
  dpath=$(readlink -f "$b/device" 2>/dev/null | clean)
  printf 'DISK|%s|%s|%s|%s|%s|%s|%s|%s|%s\n' "$n" "$model" "$serial" "$size" "${lss:-512}" "${rot:-1}" "${rem:-0}" "$tbl" "$dpath"
  for p in "$b"/"$n"*; do
    [ -f "$p/partition" ] || continue
    printf 'PART|%s|%s|%s|%s|%s\n' "$n" "${p##*/}" "$(cat "$p/partition")" "$(cat "$p/start")" "$(cat "$p/size")"
  done
done

# Internal boot flash module (DOM): partition 1 = FAT with volume id 10EE-589C, partition 2 = ext2 with Synology's UUID.
if [ -b /dev/synoboot ] && [ -b /dev/synoboot1 ] && [ -b /dev/synoboot2 ]; then
  m1=0; m2=0
  id1=$(dd if=/dev/synoboot1 bs=1 skip=39 count=4 2>/dev/null | od -An -tx1 | tr -d ' \n')
  id1b=$(dd if=/dev/synoboot1 bs=1 skip=67 count=4 2>/dev/null | od -An -tx1 | tr -d ' \n')
  { [ "$id1" = "9c58ee10" ] || [ "$id1b" = "9c58ee10" ]; } && m1=1
  mg=$(dd if=/dev/synoboot2 bs=1 skip=1080 count=2 2>/dev/null | od -An -tx1 | tr -d ' \n')
  uu=$(dd if=/dev/synoboot2 bs=1 skip=1128 count=16 2>/dev/null | od -An -tx1 | tr -d ' \n')
  [ "$mg" = "53ef" ] && [ "$uu" = "45e5b07d47834867a369f99c0cd1e610" ] && m2=1
  printf 'DOM|%s|%s|%s|%s|%s\n' "$(cat /sys/block/synoboot/size)" "$(cat /sys/block/synoboot/synoboot1/size 2>/dev/null)" \
    "$(cat /sys/block/synoboot/synoboot2/size 2>/dev/null)" "$m1" "$m2"
fi

for m in /sys/block/md*; do
  [ -d "$m/md" ] || continue
  n=${m##*/}
  mem=""
  for s in "$m"/slaves/*; do [ -e "$s" ] && mem="$mem,${s##*/}"; done
  mem=${mem#,}
  uuid=""; label=""
  if command -v mdadm >/dev/null 2>&1; then
    det=$(mdadm --detail "/dev/$n" 2>/dev/null)
    uuid=$(printf '%s\n' "$det" | sed -n 's/^ *UUID : *//p' | head -n1 | clean)
    label=$(printf '%s\n' "$det" | sed -n 's/^ *Name : *\([^ ]*\).*/\1/p' | head -n1 | clean)
  fi
  printf 'MD|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s\n' "$n" "$(clean < "$m/md/level")" "$(clean < "$m/md/array_state")" \
    "$(cat "$m/size")" "$mem" "$uuid" "$label" "$(clean < "$m/md/metadata_version" 2>/dev/null)" \
    "$(clean < "$m/md/sync_action" 2>/dev/null)" "$(clean < "$m/md/sync_completed" 2>/dev/null | tr ' ' '_')" "$(clean < "$m/md/degraded" 2>/dev/null)"
done

LVM=$(command -v lvm 2>/dev/null)
if [ -n "$LVM" ]; then
  "$LVM" pvs --noheadings --separator '|' -o pv_name,vg_name 2>/dev/null | tr -d ' ' | sed 's/^/PV|/'
  "$LVM" lvs --noheadings --units b --nosuffix --separator '|' -o vg_name,lv_name,lv_size 2>/dev/null | tr -d ' ' | sed 's/^/LV|/'
fi

while read -r dev mp fs opts rest; do
  case "$mp" in
    /|/volume*|/config|/boot*) ;;
    *) continue ;;
  esac
  case "$fs" in proc|sysfs|tmpfs|devtmpfs|devpts|cgroup*|rootfs|ramfs) continue ;; esac
  df=$(df -P -k "$mp" 2>/dev/null | tail -n1)
  size=$(printf '%s\n' "$df" | awk '{print $2}')
  used=$(printf '%s\n' "$df" | awk '{print $3}')
  printf 'MNT|%s|%s|%s|%s|%s\n' "$dev" "$mp" "$fs" "${size:-0}" "${used:-0}"
done < /proc/mounts
printf 'END|ok\n'
"#;

/// The script with shared helpers spliced in.
pub fn inspect_script() -> String {
    INSPECT_SCRIPT.replace("@@DISK_SERIAL_FN@@", DISK_SERIAL_FN)
}

/// `md/sync_completed` is `<sectors done> / <sectors total>` (spaces may have been turned into `_`) or `none`.
fn parse_sync_permille(v: &str) -> Option<u32> {
    let v = v.replace('_', " ");
    let (done, total) = v.split_once('/')?;
    let (done, total) = (done.trim().parse::<u64>().ok()?, total.trim().parse::<u64>().ok()?);
    (total > 0).then(|| ((done.min(total) as u128 * 1000) / total as u128) as u32)
}

fn num(s: &str) -> u64 {
    s.trim().parse().unwrap_or(0)
}

/// Parse the output of [`inspect_script`].
pub fn parse_inspect(out: &str) -> Result<DeviceFacts> {
    let mut f = DeviceFacts {
        model: String::new(),
        unique: String::new(),
        dsm_version: String::new(),
        serial: String::new(),
        hostname: String::new(),
        uname_m: String::new(),
        kernel: String::new(),
        arch: Arch::Unsupported,
        mem_kib: 0,
        cpu: String::new(),
        firmware: Firmware::Bios,
        kexec: false,
        dt_model: String::new(),
        cpu_vendor: CpuVendor::Other,
        dom: None,
        cmdline: String::new(),
        is_root: false,
        nics: vec![],
        disks: vec![],
        arrays: vec![],
        lvs: vec![],
        mounts: vec![],
    };
    let mut hw_version = String::new();
    let mut vendor_id = String::new();
    let mut uefi = false;
    let mut ended = false;
    let mut pvs: Vec<(String, String)> = vec![];

    for line in out.lines() {
        let line = line.trim_end_matches('\r');
        let c: Vec<&str> = line.split('|').collect();
        match c.first().copied() {
            Some("KV") if c.len() >= 3 => {
                let v = c[2..].join("|");
                match c[1] {
                    "uname_m" => f.uname_m = v,
                    "kernel" => f.kernel = v,
                    "id_u" => f.is_root = v.trim() == "0",
                    "hostname" => f.hostname = v,
                    "unique" => f.unique = v,
                    "model" => f.model = v,
                    "hw_version" => hw_version = v,
                    "serial" => f.serial = v,
                    "dsm_version" => f.dsm_version = v,
                    "mem_kib" => f.mem_kib = num(&v),
                    "cpu" => f.cpu = v,
                    "uefi" => uefi = v.trim() == "1",
                    "kexec" => f.kexec = v.trim() == "1",
                    "dt_model" => f.dt_model = v,
                    "cmdline" => f.cmdline = v,
                    "cpu_vendor" => vendor_id = v,
                    _ => {}
                }
            }
            Some("NIC") if c.len() >= 3 => f.nics.push(Nic { name: c[1].into(), mac: c[2].to_lowercase() }),
            Some("DISK") if c.len() >= 9 => f.disks.push(Disk {
                name: c[1].into(),
                model: c[2].into(),
                serial: c[3].into(),
                sectors: num(c[4]),
                logical_sector: num(c[5]) as u32,
                rotational: c[6].trim() != "0",
                removable: c[7].trim() == "1",
                table: match c[8].trim() {
                    "gpt" => PartTable::Gpt,
                    "msdos" => PartTable::Msdos,
                    _ => PartTable::None,
                },
                parts: vec![],
                transport: c.get(9).map(|p| crate::model::Transport::from_sysfs_path(p)).unwrap_or_default(),
                device_path: c.get(9).map(|p| p.trim().to_string()).unwrap_or_default(),
            }),
            Some("PART") if c.len() >= 6 => {
                if let Some(d) = f.disks.iter_mut().find(|d| d.name == c[1]) {
                    d.parts.push(Part { name: c[2].into(), number: num(c[3]) as u32, start: num(c[4]), sectors: num(c[5]) });
                }
            }
            Some("MD") if c.len() >= 9 => f.arrays.push(MdArray {
                name: c[1].into(),
                level: c[2].into(),
                state: c[3].into(),
                sectors: num(c[4]),
                members: c[5].split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
                uuid: c[6].into(),
                label: c[7].into(),
                metadata: c[8].into(),
                role: ArrayRole::Data,
                vg: None,
                sync_action: c.get(9).map(|v| v.trim().to_string()).unwrap_or_default(),
                sync_permille: c.get(10).and_then(|v| parse_sync_permille(v)),
                degraded: c.get(11).map(|v| num(v) as u32).unwrap_or(0),
            }),
            Some("DOM") if c.len() >= 6 => {
                f.dom = Some(DomFacts {
                    sectors: num(c[1]),
                    p1_sectors: num(c[2]),
                    p2_sectors: num(c[3]),
                    p1_magic: c[4] == "1",
                    p2_magic: c[5] == "1",
                })
            }
            Some("PV") if c.len() >= 3 => pvs.push((c[1].into(), c[2].into())),
            Some("LV") if c.len() >= 4 => {
                f.lvs.push(Lv { vg: c[1].into(), name: c[2].into(), bytes: c[3].split('.').next().map(num).unwrap_or(0) })
            }
            Some("MNT") if c.len() >= 6 => f.mounts.push(Mount {
                device: c[1].into(),
                mountpoint: c[2].into(),
                fstype: c[3].into(),
                size_kib: num(c[4]),
                used_kib: num(c[5]),
            }),
            Some("END") => ended = true,
            _ => {}
        }
    }
    if !ended {
        bail!("inspection output was truncated (no END record); the NAS may have dropped the connection");
    }
    for d in &mut f.disks {
        d.parts.sort_by_key(|p| p.number);
    }

    // Classify arrays by which partition numbers they sit on, and attach VGs.
    let classified: Vec<(ArrayRole, Option<String>)> = f
        .arrays
        .iter()
        .map(|a| {
            let nums: Vec<u32> = a.members.iter().filter_map(|m| f.locate_part(m).map(|(_, p)| p.number)).collect();
            let role = if !nums.is_empty() && nums.iter().all(|n| *n == 1) {
                ArrayRole::DsmSystem
            } else if !nums.is_empty() && nums.iter().all(|n| *n == 2) {
                ArrayRole::DsmSwap
            } else {
                ArrayRole::Data
            };
            let dev = format!("/dev/{}", a.name);
            let vg = pvs.iter().find(|(pv, _)| *pv == dev).map(|(_, vg)| vg.clone()).filter(|v| !v.is_empty());
            (role, vg)
        })
        .collect();
    for (a, (role, vg)) in f.arrays.iter_mut().zip(classified) {
        a.role = role;
        a.vg = vg;
    }

    if f.model.is_empty() {
        f.model = if !hw_version.is_empty() { hw_version } else { platform::model_from_unique(&f.unique).unwrap_or_default() };
    }
    let info = platform::classify(&f.unique, &f.uname_m);
    f.arch = info.arch;
    f.cpu_vendor = CpuVendor::detect(&vendor_id, f.arch);
    f.firmware = if f.dom.as_ref().is_some_and(|d| d.is_editable()) && f.arch == Arch::X86_64 {
        // The firmware always boots the DOM; that is where Debian has to be added.
        Firmware::Dom
    } else if uefi {
        Firmware::Uefi
    } else if f.arch.is_arm() {
        Firmware::UBoot
    } else {
        Firmware::Bios
    };
    Ok(f)
}

#[doc(hidden)]
pub mod fixtures {
    /// A captured-style 4-bay DSM 7 NAS: 4x 4TB, SHR-ish layout (md0 system, md1 swap, md2 data RAID5+LVM).
    pub fn ds_4bay() -> String {
        let mut s = String::new();
        s.push_str("KV|uname_m|x86_64\nKV|kernel|4.4.302+\nKV|id_u|0\nKV|hostname|diskstation\n");
        s.push_str("KV|unique|synology_apollolake_918+\nKV|model|DS918+\nKV|serial|1920PDN123456\n");
        s.push_str("KV|dsm_version|7.2.1-69057\nKV|mem_kib|8051240\nKV|cpu|Intel(R) Celeron(R) CPU J3455 @ 1.50GHz\n");
        s.push_str("KV|uefi|0\nKV|kexec|1\nKV|cmdline|root=/dev/md0 console=ttyS0,115200n8 quiet\n");
        s.push_str("NIC|eth0|00:11:32:AA:BB:01\nNIC|eth1|00:11:32:AA:BB:02\n");
        let sectors_4tb: u64 = 7_814_037_168;
        for (i, d) in ["sda", "sdb", "sdc", "sdd"].iter().enumerate() {
            s.push_str(&format!("DISK|{d}|WDC WD40EFRX-68N32N0|WD-WCC7K{i}000|{sectors_4tb}|512|1|0|gpt\n"));
            s.push_str(&format!("PART|{d}|{d}1|1|8192|16777216\n"));
            s.push_str(&format!("PART|{d}|{d}2|2|16785408|4194304\n"));
            s.push_str(&format!("PART|{d}|{d}5|5|20979712|{}\n", sectors_4tb - 20979712 - 33));
        }
        s.push_str("DISK|synoboot|Disk Module|SYNO0001|245760|512|0|1|msdos\nPART|synoboot|synoboot1|1|2048|241664\n");
        s.push_str("MD|md0|raid1|clean|16776192|sda1,sdb1,sdc1,sdd1|aaaaaaaa:bbbbbbbb:cccccccc:00000000|DiskStation:0|1.2\n");
        s.push_str("MD|md1|raid1|clean|4193280|sda2,sdb2,sdc2,sdd2|bbbbbbbb:bbbbbbbb:cccccccc:00000001|DiskStation:1|1.2\n");
        s.push_str(&format!(
            "MD|md2|raid5|clean|{}|sda5,sdb5,sdc5,sdd5|cccccccc:bbbbbbbb:cccccccc:00000002|DiskStation:2|1.2\n",
            3 * (sectors_4tb - 20979712 - 33)
        ));
        s.push_str("PV|/dev/md2|vg1000\nLV|vg1000|lv|11990000000000\n");
        s.push_str("MNT|/dev/md0|/|ext4|8191000|2400000\nMNT|/dev/mapper/cachedev_0|/volume1|btrfs|11700000000|4200000000\n");
        s.push_str("END|ok\n");
        s
    }
    /// The owner's real DS918+ (4x 8 TB, Synology Hybrid RAID-like: md2 + md3 are PVs of `vg1`, two volumes behind `cachedev_N`).
    /// Disk paths, md layout, LV sizes and mounts are from the unit's own output; the mount sizes are the owner's (volume1 15.99 TiB, volume2 5.83 TiB); usage and serials are sample data.
    /// sda/sdb sit behind an add-in SATA controller and report `removable=1`; md2 was resyncing (90.8 %) when it was captured.
    pub fn ds918_two_volumes() -> String {
        let mut s = String::new();
        s.push_str("KV|uname_m|x86_64\nKV|kernel|4.4.302+\nKV|id_u|0\nKV|hostname|awss01\n");
        s.push_str("KV|unique|synology_apollolake_918+\nKV|model|DS918+\nKV|serial|18A0PDN014408\n");
        s.push_str("KV|dsm_version|7.2.1-69057\nKV|mem_kib|3900000\nKV|cpu|Intel(R) Celeron(R) CPU J3455 @ 1.50GHz\n");
        s.push_str("KV|cpu_vendor|GenuineIntel\nKV|uefi|0\nKV|kexec|1\nKV|cmdline|root=/dev/md0 console=ttyS2,115200n8 quiet\n");
        // two Intel i211 ports (igb)
        s.push_str("NIC|eth0|00:11:32:AA:BB:01\nNIC|eth1|00:11:32:AA:BB:02\n");
        let sectors: u64 = 15_628_053_168;
        let paths = [
            ("sda", "1", "ata3", "0000:01:00.0", "2:0:0:0", "ZA1EJ7BG"),
            ("sdb", "1", "ata4", "0000:01:00.0", "3:0:0:0", "ZA1DWBT2"),
            ("sdc", "0", "ata1", "0000:00:12.0", "0:0:0:0", "ZA1EH7W8"),
            ("sdd", "0", "ata2", "0000:00:12.0", "1:0:0:0", "ZA1EHWQG"),
        ];
        for (d, rem, ata, pci, scsi, serial) in paths {
            let host = scsi.split(':').next().unwrap();
            let dev = if pci == "0000:01:00.0" { format!("pci0000:00/0000:00:13.0/{pci}") } else { format!("pci0000:00/{pci}") };
            s.push_str(&format!(
                "DISK|{d}|ST8000NM0055-1RM112|{serial}|{sectors}|512|1|{rem}|gpt|/sys/devices/{dev}/{ata}/host{host}/target{host}:0:0/{scsi}\n"
            ));
            s.push_str(&format!("PART|{d}|{d}1|1|8192|4980480\nPART|{d}|{d}2|2|4988672|4194304\n"));
            s.push_str(&format!("PART|{d}|{d}5|5|9437184|3897559296\nPART|{d}|{d}6|6|3907000000|11720813056\n"));
        }
        s.push_str("DISK|synoboot|DiskStation|SYNO0001|245760|512|1|1|msdos|/sys/devices/pci0000:00/0000:00:15.0/usb1/1-1/1-1:1.0/host4/target4:0:0/4:0:0:0\n");
        // The boot flash: partition 1 = FAT loader menu, partition 2 = ext2 with zImage/rd.gz. The unit reports it as editable (both
        // magics match). The partition sizes below are SAMPLE values (a 120 MiB flash holds at least 16 + 64 MiB for the editable test
        // plus the DSM files); `synofurb inspect` on the unit prints the real ones, which belong into docs/HARDWARE-TEST-PLAN.md.
        s.push_str("PART|synoboot|synoboot1|1|2048|49152\nPART|synoboot|synoboot2|2|51200|188416\n");
        s.push_str("DOM|245760|49152|188416|1|1\n");
        s.push_str("MD|md0|raid1|clean|4980352|sdd1,sdc1,sdb1,sda1|u0|DiskStation:0|0.9|idle|none|0\n");
        s.push_str("MD|md1|raid1|clean|4194176|sdd2,sdc2,sdb2,sda2|u1|DiskStation:1|0.9|idle|none|0\n");
        s.push_str("MD|md2|raid5|active|11692677888|sdd5,sda5,sdb5,sdc5|u2|awss01:2|1.2|resync|1770222464_/_1948779648|0\n");
        s.push_str("MD|md3|raid5|clean|35162439168|sda6,sdd6,sdc6,sdb6|u3|awss01:3|1.2|idle|none|0\n");
        s.push_str("PV|/dev/md2|vg1\nPV|/dev/md3|vg1\n");
        s.push_str("LV|vg1|syno_vg_reserved_area|12582912\nLV|vg1|volume_1|17581000000000\nLV|vg1|volume_2|6410000000000\n");
        s.push_str("MNT|/dev/md0|/|ext4|2385000|1500000\n");
        s.push_str("MNT|/dev/mapper/cachedev_0|/volume1|ext4|17170000000|9300000000\n");
        s.push_str("MNT|/dev/mapper/cachedev_1|/volume2|btrfs|6260000000|2500000000\n");
        s.push_str("MNT|/dev/mapper/cachedev_1|/volume2/@docker|btrfs|6260000000|2500000000\n");
        s.push_str("MNT|synodedup-fused|/volume2/ActiveBackupforBusiness/ActiveBackupData|fuse.synodedup-fused|0|0\n");
        s.push_str("END|ok\n");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `disk_serial sdb` against a fake `/sys/block` and a fake `udevadm`.
    #[cfg(unix)]
    fn run_disk_serial(vpd_pg80: Option<&[u8]>) -> String {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("sf-serial-{}-{}", std::process::id(), vpd_pg80.map_or(0, |v| v.len())));
        let _ = std::fs::remove_dir_all(&dir);
        let dev = dir.join("sys/block/sdb/device");
        std::fs::create_dir_all(&dev).unwrap();
        if let Some(v) = vpd_pg80 {
            std::fs::write(dev.join("vpd_pg80"), v).unwrap();
        }
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let ud = dir.join("bin/udevadm");
        std::fs::write(&ud, "#!/bin/sh\necho ID_MODEL=X\necho ID_SERIAL_SHORT=WD-WCC4N1XRD3VH\n").unwrap();
        std::fs::set_permissions(&ud, std::fs::Permissions::from_mode(0o755)).unwrap();
        let script = format!("{}\ndisk_serial sdb", DISK_SERIAL_FN.replace("/sys/block", &format!("{}/sys/block", dir.display())));
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .env("PATH", format!("{}/bin:/usr/bin:/bin", dir.display()))
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    #[cfg(unix)]
    #[test]
    fn disk_serial_falls_through_a_source_that_gives_nothing_usable() {
        // the RS812+ installer's kernel: vpd_pg80 exists for the SATA disks but reads as just "-" after its 4-byte header
        assert_eq!(run_disk_serial(Some(&[0x80, 0, 0, 1, b'-'])), "WD-WCC4N1XRD3VH");
        // no such file at all: udevadm answers
        assert_eq!(run_disk_serial(None), "WD-WCC4N1XRD3VH");
        // a real serial page (header, then the serial padded with spaces) is used as it is
        let mut page = vec![0x80, 0, 0, 20];
        page.extend_from_slice(b"      WD-OTHER123456");
        assert_eq!(run_disk_serial(Some(&page)), "WD-OTHER123456");
    }

    #[test]
    fn parses_fixture() {
        let f = parse_inspect(&fixtures::ds_4bay()).unwrap();
        assert_eq!(f.model, "DS918+");
        assert_eq!(f.arch, Arch::X86_64);
        assert_eq!(f.firmware, Firmware::Bios);
        assert!(f.kexec && f.is_root);
        assert_eq!(f.serial_console().kernel_arg(), "console=ttyS0,115200n8");
        assert_eq!(f.disks.len(), 5);
        assert_eq!(f.candidate_disks().len(), 4, "synoboot flash module must not be a candidate");
        assert_eq!(f.arrays.len(), 3);
        assert_eq!(f.arrays[0].role, ArrayRole::DsmSystem);
        assert_eq!(f.arrays[1].role, ArrayRole::DsmSwap);
        assert_eq!(f.arrays[2].role, ArrayRole::Data);
        assert_eq!(f.arrays[2].vg.as_deref(), Some("vg1000"));
        assert_eq!(f.nics[0].mac, "00:11:32:aa:bb:01");
        assert_eq!(f.lvs[0].bytes, 11_990_000_000_000);
        assert_eq!(f.disks[0].parts.len(), 3);
    }

    #[test]
    fn dom_units_boot_through_the_dom_and_amd_is_detected() {
        let mut s = fixtures::ds_4bay();
        s = s
            .replace("KV|unique|synology_apollolake_918+", "KV|unique|synology_v1000_1621+")
            .replace("KV|model|DS918+", "KV|model|DS1621+");
        s = s.replace("KV|uefi|0\n", "KV|uefi|1\nKV|cpu_vendor|AuthenticAMD\n");
        s = s.replace(
            "KV|cmdline|root=/dev/md0 console=ttyS0,115200n8 quiet",
            "KV|cmdline|root=/dev/md0 console=tty0 console=uart8250,mmio32,0xfedc9000,115200n8",
        );
        s = s.replace("NIC|eth0", "DOM|262144|65536|196608|1|1\nNIC|eth0");
        let f = parse_inspect(&s).unwrap();
        assert_eq!(f.cpu_vendor, CpuVendor::Amd);
        assert_eq!(f.firmware, Firmware::Dom, "DOM wins over UEFI: that is what the firmware boots");
        assert!(f.dom.as_ref().unwrap().is_editable());
        assert_eq!(f.serial_console().kernel_arg(), "console=uart8250,mmio32,0xfedc9000,115200n8");
        // a DOM that does not match Synology's loader must NOT be treated as editable
        let bad = s.replace("DOM|262144|65536|196608|1|1", "DOM|262144|65536|196608|1|0");
        assert_ne!(parse_inspect(&bad).unwrap().firmware, Firmware::Dom);
    }

    #[test]
    fn truncated_output_is_rejected() {
        let s = fixtures::ds_4bay().replace("END|ok\n", "");
        assert!(parse_inspect(&s).is_err());
    }

    #[test]
    fn arm_without_efi_is_uboot() {
        let s = "KV|uname_m|aarch64\nKV|unique|synology_rtd1296_ds118\nKV|uefi|0\nEND|ok\n";
        let f = parse_inspect(s).unwrap();
        assert_eq!(f.arch, Arch::Arm64);
        assert_eq!(f.firmware, Firmware::UBoot);
    }

    #[test]
    fn the_real_ds918_keeps_all_four_internal_disks_even_though_two_report_removable() {
        let f = parse_inspect(&fixtures::ds918_two_volumes()).unwrap();
        assert_eq!(f.candidate_disks().len(), 4, "sda/sdb report removable=1 but are SATA disks");
        let names: Vec<&str> = f.candidate_disks().iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["sda", "sdb", "sdc", "sdd"]);
        assert_eq!(f.disks[0].transport, crate::model::Transport::Sata);
        assert!(f.disks[0].removable && !f.disks[2].removable);
        let ex = f.excluded_disks();
        assert_eq!(ex.len(), 1);
        assert_eq!(ex[0].0.name, "synoboot");
        assert_eq!(ex[0].1, crate::model::Exclusion::BootFlash);
    }

    #[test]
    fn usb_and_unknown_removable_disks_stay_excluded_and_old_lines_still_parse() {
        use crate::model::Exclusion;
        // a USB stick (usb transport, even if it does not say removable)
        let usb = fixtures::ds_4bay().replace(
            "DISK|synoboot|Disk Module|SYNO0001|245760|512|0|1|msdos",
            "DISK|sde|SanDisk Ultra|X1|62500000|512|0|0|gpt|/sys/devices/pci0000:00/0000:00:15.0/usb2/2-1/2-1:1.0/host9/target9:0:0/9:0:0:0",
        );
        let f = parse_inspect(&usb).unwrap();
        assert_eq!(f.disk("sde").unwrap().exclusion(), Some(Exclusion::Usb));
        // removable and no transport information (a 9-field line from an older unit script): not silently a target
        let old = fixtures::ds_4bay().replace("WD-WCC7K0000|7814037168|512|1|0|gpt", "WD-WCC7K0000|7814037168|512|1|1|gpt");
        let f = parse_inspect(&old).unwrap();
        assert_eq!(f.disk("sda").unwrap().exclusion(), Some(Exclusion::Removable));
        assert_eq!(f.candidate_disks().len(), 3);
    }

    #[test]
    fn dsm_volumes_are_found_behind_cachedev_devices_and_joined_with_lvm_and_the_arrays() {
        let f = parse_inspect(&fixtures::ds918_two_volumes()).unwrap();
        let v = f.volumes();
        assert_eq!(v.len(), 2, "sub-mounts like /volume2/@docker and FUSE mounts are not volumes");
        assert_eq!(
            (v[0].name.as_str(), v[0].fstype.as_str(), v[0].lv.as_deref(), v[0].vg.as_deref()),
            ("volume1", "ext4", Some("volume_1"), Some("vg1"))
        );
        assert_eq!((v[1].name.as_str(), v[1].fstype.as_str(), v[1].lv.as_deref()), ("volume2", "btrfs", Some("volume_2")));
        assert_eq!(v[0].arrays, ["md2", "md3"]);
        assert_eq!(v[0].disks, ["sda", "sdb", "sdc", "sdd"]);
        assert_eq!(v[0].levels, ["raid5"]);
        assert_eq!(v[0].used_percent(), 54);
    }

    #[test]
    fn a_running_resync_is_reported() {
        let f = parse_inspect(&fixtures::ds918_two_volumes()).unwrap();
        let busy = f.busy_arrays();
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].name, "md2");
        assert_eq!(busy[0].health_note().as_deref(), Some("resyncing 90.8%"));
        assert!(f.arrays.iter().filter(|a| a.name != "md2").all(|a| a.health_note().is_none()));
    }

    #[test]
    fn dm_names_split_with_doubled_dashes() {
        assert_eq!(crate::model::split_dm_name_for_test("vg1-volume_1"), Some(("vg1".into(), "volume_1".into())));
        assert_eq!(crate::model::split_dm_name_for_test("my--vg-lv--x"), Some(("my-vg".into(), "lv-x".into())));
        assert_eq!(crate::model::split_dm_name_for_test("cachedev_0"), None);
    }

    #[test]
    fn script_has_no_unexpanded_placeholder_and_is_posix_sh_clean() {
        let s = inspect_script();
        assert!(!s.contains("@@"));
        assert!(!s.contains("[["), "no bashisms in a script that runs on DSM");
    }
}
