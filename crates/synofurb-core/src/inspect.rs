//! Read-only inspection of a running DSM.
//!
//! [`INSPECT_SCRIPT`] is a POSIX `sh` script (DSM ships busybox-ish tools) that
//! prints one pipe-separated record per line; [`parse_inspect`] turns that into
//! [`DeviceFacts`]. The script never writes anything. Keeping the parser pure
//! lets us test it against captured output without a NAS.

use crate::model::*;
use crate::platform;
use anyhow::{Result, bail};

/// Shell helper shared with the stage-2 installer: best-effort disk serial number.
pub const DISK_SERIAL_FN: &str = r#"disk_serial() {
  _d="$1"
  if [ -r "/sys/block/$_d/device/serial" ]; then
    tr -cd '[:alnum:]_-' < "/sys/block/$_d/device/serial"; return
  fi
  if [ -r "/sys/block/$_d/device/vpd_pg80" ]; then
    dd if="/sys/block/$_d/device/vpd_pg80" bs=1 skip=4 2>/dev/null | tr -cd '[:alnum:]_-'; return
  fi
  if command -v udevadm >/dev/null 2>&1; then
    _s=$(udevadm info --query=property --name="/dev/$_d" 2>/dev/null | sed -n 's/^ID_SERIAL_SHORT=//p' | head -n1)
    if [ -n "$_s" ]; then printf '%s' "$_s" | tr -cd '[:alnum:]_-'; return; fi
  fi
  if command -v smartctl >/dev/null 2>&1; then
    smartctl -i "/dev/$_d" 2>/dev/null | sed -n 's/^Serial [Nn]umber: *//p' | head -n1 | tr -cd '[:alnum:]_-'
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
  printf 'DISK|%s|%s|%s|%s|%s|%s|%s|%s\n' "$n" "$model" "$serial" "$size" "${lss:-512}" "${rot:-1}" "${rem:-0}" "$tbl"
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
  printf 'MD|%s|%s|%s|%s|%s|%s|%s|%s\n' "$n" "$(clean < "$m/md/level")" "$(clean < "$m/md/array_state")" \
    "$(cat "$m/size")" "$mem" "$uuid" "$label" "$(clean < "$m/md/metadata_version" 2>/dev/null)"
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn script_has_no_unexpanded_placeholder_and_is_posix_sh_clean() {
        let s = inspect_script();
        assert!(!s.contains("@@"));
        assert!(!s.contains("[["), "no bashisms in a script that runs on DSM");
    }
}
