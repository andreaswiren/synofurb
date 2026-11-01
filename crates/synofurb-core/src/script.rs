//! Renders the stage-2 installer: a POSIX `sh` script that runs inside the Debian
//! installer environment (busybox `ash`), partitions the disks exactly as the
//! [`Plan`] says, bootstraps Debian with `debootstrap` and configures it.
//!
//! The script is straight-line generated code (no JSON parsing, no loops over
//! unknown data), so the log shows every command and the output can be reviewed
//! before anything runs. `SYNOFURB_DRY=1` makes every mutating command a no-op
//! that only logs, which is how the test-suite and `synofurb render --dry-run`
//! exercise it.

use crate::config::{InstallConfig, sh_quote};
use crate::inspect::DISK_SERIAL_FN;
use crate::model::*;
use crate::plan::*;
use crate::timing::{FAIL_REBOOT_AFTER_PONR_SECS, FAIL_REBOOT_BEFORE_PONR_SECS};
use anyhow::{Result, bail};
use std::fmt::Write;

pub struct ScriptParams<'a> {
    pub plan: &'a Plan,
    pub facts: &'a DeviceFacts,
    pub cfg: &'a InstallConfig,
    pub run_id: &'a str,
    /// `http://host:port/token` where the app serves `payload.tar.gz`.
    pub http_base: &'a str,
    pub payload_sha256: &'a str,
    pub log_host: &'a str,
    pub log_tcp_port: u16,
    pub dry_run: bool,
}

/// What the generated installer does when a step fails. The script has **no rescue shell and no SSH server**: the log is on the console
/// and in the event stream, and what happens next depends on the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct FailureHandling {
    /// The unit reboots by itself after a failure (boot-flash units where the installer has `synofurb-dom`). Otherwise the installer halts
    /// and the unit stays as it is until somebody power-cycles it: nothing reboots, there is no countdown.
    pub reboots_by_itself: bool,
    /// Seconds before that reboot when the failure happened before the point of no return.
    pub before_ponr_secs: Option<u32>,
    /// Seconds before that reboot after the point of no return (time to read the log).
    pub after_ponr_secs: Option<u32>,
}

/// Failure behaviour of the script rendered for `plan` (the `60 s` and `15 min` of the app's texts exist only when this says so).
pub fn failure_handling(plan: &Plan) -> FailureHandling {
    if plan.dom_boot {
        FailureHandling {
            reboots_by_itself: true,
            before_ponr_secs: Some(FAIL_REBOOT_BEFORE_PONR_SECS),
            after_ponr_secs: Some(FAIL_REBOOT_AFTER_PONR_SECS),
        }
    } else {
        FailureHandling { reboots_by_itself: false, before_ponr_secs: None, after_ponr_secs: None }
    }
}

/// Udebs the script loads before touching disks (kernel module udebs are virtual `*-modules`).
pub const ANNA_PACKAGES: &[&str] = &[
    "debian-archive-keyring-udeb",
    "parted-udeb",
    "mdadm-udeb",
    "lvm2-udeb",
    "xfsprogs-udeb",
    "e2fsprogs-udeb",
    "dosfstools-udeb",
    "md-modules",
    "xfs-modules",
    "ext4-modules",
    "fat-modules",
    "scsi-modules",
    "sata-modules",
    "crc-modules",
    "usb-storage-modules",
    "usb-modules",
];

const PREAMBLE: &str = r#"#!/bin/sh
# shellcheck shell=dash disable=SC2154
# ---------------------------------------------------------------------------
# synofurb stage-2 installer. GENERATED - review before you trust it.
# Runs inside the Debian installer environment (busybox ash).
# ---------------------------------------------------------------------------
LOG=${SYNOFURB_LOG:-/var/log/synofurb-install.log}
SEQ=0
TOTAL_STEPS=@@TOTAL@@

mkdir -p "$(dirname "$LOG")" 2>/dev/null
: >> "$LOG"

ts() { date '+%H:%M:%S' 2>/dev/null || echo '--:--:--'; }
emit() {
  SEQ=$((SEQ + 1))
  _l="$(ts) #$SEQ $*"
  printf '%s\n' "$_l" >> "$LOG"
  printf '%s\n' "$_l" > /dev/console 2>/dev/null
  logger -t synofurb "$*" 2>/dev/null
  return 0
}
log()  { emit "INFO  $*"; }
warn() { emit "WARN  $*"; }
step() { STEP_NO=$1; shift; emit "@@STEP $STEP_NO/$TOTAL_STEPS $*"; }

diag() {
  emit "---- diagnostics ----"
  { echo '## /proc/mdstat'; cat /proc/mdstat 2>/dev/null
    echo '## /proc/partitions'; cat /proc/partitions 2>/dev/null
    echo '## mounts'; cat /proc/mounts 2>/dev/null
    echo '## lvm'; command -v lvm >/dev/null 2>&1 && { lvm pvs 2>&1; lvm vgs 2>&1; lvm lvs 2>&1; }
    echo '## dmesg (tail)'; dmesg 2>/dev/null | tail -n 80
    echo '## memory'; grep -E 'MemTotal|MemFree' /proc/meminfo
  } >> "$LOG" 2>&1
}

PONR=0
DOM_BOOT=@@DOM@@
fail() {
  emit "ERROR $*"
  emit "@@FAIL $*"
  diag
  if [ "$DOM_BOOT" = 1 ] && command -v synofurb-dom >/dev/null 2>&1; then
    # FAIL-SAFE: this unit boots from its internal flash. Point the default back at DSM and reboot, so a
    # failed or abandoned install never leaves the NAS unreachable.
    emit "FALLBACK: switching the boot flash default back to DSM"
    [ "$DRY" = 1 ] || synofurb-dom default dsm --yes >> "$LOG" 2>&1 || emit "WARN  could not reset the default boot entry; power-cycle the NAS and pick DSM"
    if [ "$PONR" = 0 ]; then
      emit "No disk was changed. The NAS reboots in @@FAIL_PRE@@ seconds and starts what its boot flash defaults to. Full log: $LOG"
      _w=@@FAIL_PRE@@
    else
      emit "Disks were already modified. The NAS reboots in @@FAIL_POST_MIN@@ minutes (time to read the log) and starts what its boot flash defaults to: the DSM loader. DSM's operating system on the reclaimed partitions is gone, so the loader offers a reinstall (find.synology.com, the .pat file for your model). synofurb cannot promise that any data survives. Full log: $LOG"
      _w=@@FAIL_POST@@
    fi
    [ "$DRY" = 1 ] && exit 1
    sleep "$_w"
    reboot -f 2>/dev/null || reboot
  fi
  emit "Installation HALTED. No further changes will be made and the NAS will NOT reboot by itself. Read the log on the console, then power-cycle the NAS. Full log: $LOG"
  sleep 5
  while :; do sleep 3600; done
}

DRY=${SYNOFURB_DRY:-@@DRY@@}
run() {
  emit "CMD   $*"
  [ "$DRY" = 1 ] && return 0
  "$@" >> "$LOG" 2>&1
  _rc=$?
  [ $_rc -eq 0 ] || fail "command failed (rc=$_rc): $*"
}
try() {
  emit "CMD?  $*"
  [ "$DRY" = 1 ] && return 0
  "$@" >> "$LOG" 2>&1
  _rc=$?
  [ $_rc -eq 0 ] || emit "WARN  optional command failed (rc=$_rc): $*"
  return 0
}
# shell snippet run through 'sh -c' with logging
runsh() {
  emit "SH    $*"
  [ "$DRY" = 1 ] && return 0
  sh -c "$*" >> "$LOG" 2>&1
  _rc=$?
  [ $_rc -eq 0 ] || fail "shell snippet failed (rc=$_rc): $*"
}

LOCK=${SYNOFURB_LOCK:-/tmp/synofurb.lock}
[ -e "$LOCK" ] && { emit "WARN  stage-2 already running or finished; refusing to start twice"; while :; do sleep 3600; done; }
: > "$LOCK"

@@DISK_SERIAL_FN@@
pdev() { case "$1" in *[0-9]) echo "${1}p$2" ;; *) echo "${1}$2" ;; esac; }
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  elif busybox sha256sum /dev/null >/dev/null 2>&1; then busybox sha256sum "$1" | cut -d' ' -f1
  else echo UNAVAILABLE; fi
}
wait_dev() {
  [ "$DRY" = 1 ] && return 0
  _i=0
  while [ ! -b "$1" ] && [ $_i -lt 30 ]; do sleep 1; _i=$((_i + 1)); done
  [ -b "$1" ] || { command -v vgmknodes >/dev/null 2>&1 && vgmknodes >/dev/null 2>&1; }
  [ -b "$1" ] || fail "device node $1 did not appear"
}
settle() {
  [ "$DRY" = 1 ] && return 0
  command -v udevadm >/dev/null 2>&1 && udevadm settle --timeout=20 >/dev/null 2>&1
  sleep 1
}
# zero_ends DEVICE : wipe signatures at both ends (md/LVM/fs/GPT metadata live there)
zero_ends() {
  emit "WIPE  start/end of $1"
  [ "$DRY" = 1 ] && return 0
  _sz=$(blockdev --getsize64 "$1" 2>/dev/null || echo 0)
  _mib=$((_sz / 1048576))
  dd if=/dev/zero of="$1" bs=1M count=8 conv=notrunc >> "$LOG" 2>&1
  if [ "$_mib" -gt 16 ]; then
    dd if=/dev/zero of="$1" bs=1M seek=$((_mib - 8)) count=8 conv=notrunc >> "$LOG" 2>&1
  fi
}
zero_range() { # DEVICE SEEK_MIB COUNT_MIB
  emit "WIPE  $1 range ${2}MiB +${3}MiB"
  [ "$DRY" = 1 ] && return 0
  dd if=/dev/zero of="$1" bs=1M seek="$2" count="$3" conv=notrunc >> "$LOG" 2>&1
}

CLAIMED=" "
resolve_disk() { # VAR SERIAL MODEL SECTORS
  _var=$1; _ser=$2; _mod=$3; _sec=$4
  if [ "$DRY" = 1 ]; then eval "$_var=dry_$_var"; return 0; fi
  for _b in /sys/block/*; do
    _n=${_b##*/}
    case "$_n" in loop*|ram*|md*|dm-*|zram*|nbd*|sr*) continue ;; esac
    [ -e "$_b/device" ] || continue
    [ "$(cat "$_b/size" 2>/dev/null)" = "$_sec" ] || continue
    case "$CLAIMED" in *" $_n "*) continue ;; esac
    if [ -n "$_ser" ]; then
      [ "$(disk_serial "$_n")" = "$_ser" ] || continue
    else
      _m=$(sed 's/  */ /g;s/ $//' "$_b/device/model" 2>/dev/null)
      [ "$_m" = "$_mod" ] || continue
    fi
    CLAIMED="$CLAIMED$_n "
    eval "$_var=\$_n"
    return 0
  done
  return 1
}
expect_part() { # DISK NUM START SECTORS
  [ "$DRY" = 1 ] && return 0
  _pd=$(pdev "$1" "$2"); _s="/sys/block/$1/$_pd"
  [ -d "$_s" ] || fail "safety check: expected partition $_pd on $1 is missing - the disk does not match what was inspected"
  if [ "$(cat "$_s/start")" != "$3" ] || [ "$(cat "$_s/size")" != "$4" ]; then
    fail "safety check: partition $_pd on $1 differs from the inspected layout (start/size) - refusing to continue"
  fi
}
expect_nparts() { # DISK COUNT
  [ "$DRY" = 1 ] && return 0
  _c=0
  for _p in /sys/block/"$1"/"$1"*; do [ -f "$_p/partition" ] && _c=$((_c + 1)); done
  [ "$_c" = "$2" ] || fail "safety check: $1 has $_c partitions, expected $2 - the disk does not match what was inspected"
}
"#;

fn blockref(r: &BlockRef) -> String {
    match r {
        BlockRef::Part(d, n) => format!("\"/dev/$(pdev \"$D_{}\" {})\"", ident(d), n),
        BlockRef::Md(name) => format!("/dev/md/{name}"),
    }
}

/// Shell variable suffix for a DSM disk name.
fn ident(disk: &str) -> String {
    disk.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// GPT type flag for a new partition; `None` keeps the default "Linux filesystem" type.
fn flag_for(role: PartRole, in_md: bool) -> Option<&'static str> {
    match role {
        PartRole::BiosGrub => Some("bios_grub"),
        PartRole::Esp => Some("esp"),
        PartRole::Boot => in_md.then_some("raid"),
        PartRole::SystemPv | PartRole::Data => Some(if in_md { "raid" } else { "lvm" }),
    }
}

fn part_in_md(plan: &Plan, disk: &str, num: u32) -> bool {
    plan.arrays.iter().any(|a| a.members.iter().any(|(d, n)| d == disk && *n == num))
}

/// Number of `step()` calls in the rendered script.
const TOTAL_STEPS: u32 = 12;

pub fn render_stage2(p: &ScriptParams) -> Result<String> {
    let plan = p.plan;
    if !plan.is_ok() {
        bail!("refusing to render a plan with errors: {}", plan.errors.join("; "));
    }
    p.cfg.validate()?;
    plan.validate(p.facts).map_err(|e| anyhow::anyhow!("plan failed validation: {}", e.join("; ")))?;
    if p.http_base.contains(['\'', ' ', '\n']) || p.log_host.contains(['\'', ' ', '\n']) {
        bail!("invalid base URL or log host");
    }

    let mut s = String::new();
    s.push_str(
        &PREAMBLE
            .replace("@@TOTAL@@", &TOTAL_STEPS.to_string())
            .replace("@@DRY@@", if p.dry_run { "1" } else { "0" })
            .replace("@@DOM@@", if plan.dom_boot { "1" } else { "0" })
            .replace("@@FAIL_PRE@@", &FAIL_REBOOT_BEFORE_PONR_SECS.to_string())
            .replace("@@FAIL_POST_MIN@@", &(FAIL_REBOOT_AFTER_PONR_SECS / 60).to_string())
            .replace("@@FAIL_POST@@", &FAIL_REBOOT_AFTER_PONR_SECS.to_string())
            .replace("@@DISK_SERIAL_FN@@", DISK_SERIAL_FN),
    );
    let w = &mut s;

    // -- log streaming -------------------------------------------------------
    writeln!(w, "\nRUN_ID={}\nLOGHOST={}\nLOGTCP={}", sh_quote(p.run_id), sh_quote(p.log_host), p.log_tcp_port)?;
    writeln!(w, "if [ \"$DRY\" != 1 ] && [ -n \"$LOGHOST\" ] && command -v nc >/dev/null 2>&1; then")?;
    writeln!(w, "  ( tail -n +1 -F \"$LOG\" 2>/dev/null | nc \"$LOGHOST\" \"$LOGTCP\" ) >/dev/null 2>&1 &")?;
    writeln!(w, "fi")?;
    writeln!(w, "emit \"@@BEGIN run=$RUN_ID arch={} firmware={:?} mode={:?} dry=$DRY\"", plan.arch.debian(), plan.firmware, plan.mode)?;
    writeln!(w, "log \"installer kernel: $(uname -srm)  memory: $(sed -n 's/^MemTotal: *//p' /proc/meminfo)\"")?;
    writeln!(w, "log \"target NAS: {} ({}), DSM {}\"", p.facts.model, p.facts.serial, p.facts.dsm_version)?;

    // -- 1. installer components --------------------------------------------
    writeln!(w, "\nstep 1 \"Loading installer components (RAID, LVM, XFS, debootstrap)\"")?;
    writeln!(w, "try anna-install {}", ANNA_PACKAGES.join(" "))?;
    writeln!(w, "settle")?;
    writeln!(w, "for m in dm_mod raid0 raid1 raid10 raid456 xfs ext4 vfat loop; do try modprobe $m; done")?;
    writeln!(w, "for t in parted mdadm lvm mkfs.xfs mkfs.ext4 mkfs.fat mkswap debootstrap blockdev wget; do")?;
    writeln!(
        w,
        "  command -v $t >/dev/null 2>&1 || [ \"$DRY\" = 1 ] || fail \"required tool '$t' is missing in the installer environment\""
    )?;
    writeln!(w, "done")?;

    // -- 2. preflight --------------------------------------------------------
    writeln!(w, "\nstep 2 \"Preflight: locating disks and verifying they match the inspected NAS\"")?;
    for dp in &plan.disks {
        let id = ident(&dp.disk);
        writeln!(
            w,
            "resolve_disk D_{id} {} {} {} || fail \"cannot find disk '{}' (model {}, serial {}, {} sectors) in the installer\"",
            sh_quote(&dp.ident.serial),
            sh_quote(&dp.ident.model),
            dp.ident.sectors,
            dp.disk,
            dp.ident.model,
            if dp.ident.serial.is_empty() { "unknown" } else { &dp.ident.serial },
            dp.ident.sectors
        )?;
        writeln!(w, "log \"disk {} ({}) -> /dev/$D_{id}\"", dp.disk, dp.ident.model)?;
        writeln!(w, "expect_nparts \"$D_{id}\" {}", dp.expect.len())?;
        for (n, start, sectors) in &dp.expect {
            writeln!(w, "expect_part \"$D_{id}\" {n} {start} {sectors}")?;
        }
    }
    writeln!(w, "log \"downloading payload\"")?;
    writeln!(w, "run wget -q -O /tmp/synofurb-payload.tar.gz {}", sh_quote(&format!("{}/payload.tar.gz", p.http_base)))?;
    writeln!(w, "_got=$(sha256_of /tmp/synofurb-payload.tar.gz)")?;
    writeln!(w, "if [ \"$DRY\" != 1 ]; then")?;
    writeln!(w, "  if [ \"$_got\" = UNAVAILABLE ]; then warn \"no sha256 tool in the installer; payload integrity not verified\";")?;
    writeln!(w, "  elif [ \"$_got\" != {} ]; then fail \"payload checksum mismatch (got $_got)\"; fi", sh_quote(p.payload_sha256))?;
    writeln!(w, "fi")?;
    writeln!(w, "log \"checking the Debian mirror\"")?;
    writeln!(
        w,
        "run wget -q -O /dev/null {}",
        sh_quote(&format!("http://{}{}/dists/{}/Release", p.cfg.mirror_host, p.cfg.mirror_dir, p.cfg.suite))
    )?;
    writeln!(w, "KEYRING=''")?;
    writeln!(
        w,
        "for k in /usr/share/keyrings/debian-archive-keyring.pgp /usr/share/keyrings/debian-archive-keyring.gpg /usr/share/keyrings/archive.gpg; do [ -f \"$k\" ] && KEYRING=$k && break; done"
    )?;
    writeln!(
        w,
        "[ -n \"$KEYRING\" ] || [ \"$DRY\" = 1 ] || fail \"Debian archive keyring not found in the installer; refusing to install unsigned packages\""
    )?;

    // -- 3. point of no return ----------------------------------------------
    writeln!(w, "\nstep 3 \"POINT OF NO RETURN: releasing existing RAID/LVM and wiping reclaimed areas\"")?;
    writeln!(w, "emit \"@@POINT_OF_NO_RETURN\"")?;
    writeln!(w, "PONR=1")?;
    for d in &plan.destroyed {
        writeln!(w, "log {}", sh_quote(&format!("DESTROYING: {d}")))?;
    }
    for d in &plan.preserved {
        writeln!(w, "log {}", sh_quote(&format!("PRESERVING: {} - {}", d.what, d.detail)))?;
    }
    writeln!(w, "try vgchange -an --noudevsync")?;
    writeln!(w, "try mdadm --stop --scan")?;
    for dp in &plan.disks {
        let id = ident(&dp.disk);
        let wipe: Vec<u32> = if dp.new_table { dp.expect.iter().map(|e| e.0).collect() } else { dp.delete.clone() };
        for n in wipe {
            writeln!(w, "try mdadm --zero-superblock --force \"/dev/$(pdev \"$D_{id}\" {n})\"")?;
            writeln!(w, "zero_ends \"/dev/$(pdev \"$D_{id}\" {n})\"")?;
        }
        if dp.new_table {
            writeln!(w, "zero_ends \"/dev/$D_{id}\"")?;
        }
        if let Some(rb) = plan.raw_boot.as_ref().filter(|_| dp.create.iter().any(|c| c.role == PartRole::SystemPv)) {
            writeln!(w, "zero_range \"/dev/$D_{id}\" {} {}", rb.kernel_mib, rb.reserved_mib - rb.kernel_mib)?;
        }
    }

    // -- 4. partitioning -------------------------------------------------------
    writeln!(w, "\nstep 4 \"Partitioning\"")?;
    for dp in &plan.disks {
        let id = ident(&dp.disk);
        if dp.new_table {
            writeln!(w, "run parted -s \"/dev/$D_{id}\" mklabel gpt")?;
        }
        for n in &dp.delete {
            writeln!(w, "run parted -s \"/dev/$D_{id}\" rm {n}")?;
        }
        for c in &dp.create {
            let in_md = part_in_md(plan, &dp.disk, c.number);
            writeln!(w, "run parted -s -a none \"/dev/$D_{id}\" unit s mkpart sf-{} {}s {}s", c.role.label(), c.start, c.end() - 1)?;
            if let Some(flag) = flag_for(c.role, in_md) {
                writeln!(w, "run parted -s \"/dev/$D_{id}\" set {} {} on", c.number, flag)?;
            }
        }
        writeln!(w, "try blockdev --rereadpt \"/dev/$D_{id}\"")?;
    }
    writeln!(w, "settle")?;
    for dp in &plan.disks {
        let id = ident(&dp.disk);
        for c in &dp.create {
            writeln!(w, "wait_dev \"/dev/$(pdev \"$D_{id}\" {})\"", c.number)?;
            writeln!(w, "zero_ends \"/dev/$(pdev \"$D_{id}\" {})\"", c.number)?;
        }
    }

    // -- 5. RAID -----------------------------------------------------------------
    writeln!(w, "\nstep 5 \"Creating RAID arrays ({} array(s))\"", plan.arrays.len())?;
    writeln!(w, "mkdir -p /dev/md")?;
    for a in &plan.arrays {
        let level = if a.level == "linear" { "linear".to_string() } else { a.level.clone() };
        let members: Vec<String> = a.members.iter().map(|(d, n)| blockref(&BlockRef::Part(d.clone(), *n))).collect();
        let bitmap = if a.name.starts_with("data") && a.level != "0" && a.level != "linear" { " --bitmap=internal" } else { "" };
        writeln!(
            w,
            "run mdadm --create /dev/md/{name} --run --force --level={level} --raid-devices={n} --metadata={meta} --name={name} --homehost={host}{bitmap} {members}",
            name = a.name,
            n = a.members.len(),
            meta = a.metadata,
            host = sh_quote(&p.cfg.hostname),
            members = members.join(" ")
        )?;
        writeln!(w, "wait_dev /dev/md/{}", a.name)?;
    }

    // -- 6. LVM ------------------------------------------------------------------
    writeln!(w, "\nstep 6 \"Creating LVM volume groups and logical volumes\"")?;
    for vg in &plan.vgs {
        // One PV per entry; the data group of an SHR-like layout has one per slice array.
        let mut pv_vars = vec![];
        for (i, pv) in vg.pvs.iter().enumerate() {
            let var = if vg.pvs.len() == 1 { format!("PV_{}", vg.name) } else { format!("PV_{}_{}", vg.name, i + 1) };
            writeln!(w, "{var}={}", blockref(pv))?;
            writeln!(w, "run pvcreate -ff -y \"${var}\"")?;
            pv_vars.push(format!("\"${var}\""));
        }
        writeln!(w, "run vgcreate {} {}", vg.name, pv_vars.join(" "))?;
        for lv in &vg.lvs {
            let size = match (lv.mib, lv.vg_percent) {
                (Some(m), _) => format!("-L {m}m"),
                (None, Some(pc)) => format!("-l {pc}%VG"),
                _ => bail!("logical volume {} has no size", lv.name),
            };
            writeln!(w, "run lvcreate --noudevsync -y -n {} {} {}", lv.name, size, vg.name)?;
        }
        writeln!(w, "try vgchange -ay --noudevsync {}", vg.name)?;
        writeln!(w, "try vgmknodes")?;
        for lv in &vg.lvs {
            writeln!(w, "wait_dev /dev/mapper/{}-{}", vg.name, lv.name)?;
        }
    }

    // -- 7. filesystems -----------------------------------------------------------
    writeln!(w, "\nstep 7 \"Creating filesystems (xfs on LVM; ext4 /boot; FAT32 ESP)\"")?;
    if let Some(b) = &plan.boot {
        writeln!(w, "DEV_BOOT={}", blockref(b))?;
        writeln!(w, "run mkfs.ext4 -F -L SF_BOOT \"$DEV_BOOT\"")?;
    }
    if let Some(e) = &plan.esp {
        writeln!(w, "DEV_ESP={}", blockref(e))?;
        writeln!(w, "run mkfs.fat -F 32 -n SF_ESP \"$DEV_ESP\"")?;
    }
    for vg in &plan.vgs {
        for lv in &vg.lvs {
            let dev = format!("/dev/mapper/{}-{}", vg.name, lv.name);
            match lv.fs.as_str() {
                "xfs" => writeln!(w, "run mkfs.xfs -f -L SF_{} {dev}", lv.name.to_uppercase())?,
                "swap" => writeln!(w, "run mkswap -L SF_SWAP {dev}")?,
                other => bail!("unsupported filesystem {other}"),
            }
        }
    }

    // -- 8. mount target -------------------------------------------------------------
    writeln!(w, "\nstep 8 \"Mounting the new system\"")?;
    writeln!(w, "mkdir -p /target")?;
    writeln!(w, "run mount /dev/mapper/vg_sys-root /target")?;
    if plan.boot.is_some() {
        writeln!(w, "run mkdir -p /target/boot")?;
        writeln!(w, "run mount \"$DEV_BOOT\" /target/boot")?;
    }
    if plan.esp.is_some() {
        writeln!(w, "run mkdir -p /target/boot/efi")?;
        writeln!(w, "run mount \"$DEV_ESP\" /target/boot/efi")?;
    }
    let has_data_lv = plan.vgs.iter().any(|v| v.name == "vg_data" && !v.lvs.is_empty());
    if has_data_lv {
        writeln!(w, "run mkdir -p /target/srv/data")?;
        writeln!(w, "run mount /dev/mapper/vg_data-data /target/srv/data")?;
    }

    // -- 9. debootstrap ----------------------------------------------------------------
    writeln!(w, "\nstep 9 \"Installing the Debian {} base system (debootstrap, {})\"", p.cfg.suite, plan.arch.debian())?;
    writeln!(
        w,
        "run debootstrap --arch={arch} --variant=minbase --components=main,contrib,non-free-firmware ${{KEYRING:+--keyring=$KEYRING}} --include=ca-certificates,locales,dbus {suite} /target {mirror}",
        arch = plan.arch.debian(),
        suite = p.cfg.suite,
        mirror = sh_quote(&format!("http://{}{}", p.cfg.mirror_host, p.cfg.mirror_dir)),
    )?;

    // -- 10. fstab, mdadm.conf, payload ------------------------------------------------------
    writeln!(w, "\nstep 10 \"Writing fstab/mdadm.conf and unpacking the configuration payload\"")?;
    writeln!(w, "uuid() {{ blkid -s UUID -o value \"$1\" 2>/dev/null; }}")?;
    writeln!(w, "if [ \"$DRY\" != 1 ]; then")?;
    writeln!(w, "  {{")?;
    writeln!(w, "    echo '# /etc/fstab - written by synofurb'")?;
    writeln!(w, "    echo \"UUID=$(uuid /dev/mapper/vg_sys-root) / xfs defaults 0 1\"")?;
    if plan.boot.is_some() {
        writeln!(w, "    echo \"UUID=$(uuid \"$DEV_BOOT\") /boot ext4 defaults 0 2\"")?;
    }
    if plan.esp.is_some() {
        writeln!(w, "    echo \"UUID=$(uuid \"$DEV_ESP\") /boot/efi vfat umask=0077 0 2\"")?;
    }
    writeln!(w, "    echo \"UUID=$(uuid /dev/mapper/vg_sys-swap) none swap sw 0 0\"")?;
    if has_data_lv {
        writeln!(w, "    echo \"UUID=$(uuid /dev/mapper/vg_data-data) /srv/data xfs defaults,nofail 0 0\"")?;
    }
    writeln!(w, "  }} > /target/etc/fstab")?;
    writeln!(w, "  mkdir -p /target/etc/mdadm")?;
    writeln!(w, "  {{")?;
    writeln!(w, "    echo 'DEVICE partitions'")?;
    writeln!(w, "    echo 'CREATE owner=root group=disk mode=0660 auto=yes'")?;
    writeln!(w, "    echo 'HOMEHOST <system>'")?;
    writeln!(w, "    echo 'MAILADDR root'")?;
    if !plan.arrays.is_empty() {
        writeln!(w, "    mdadm --detail --scan")?;
    }
    writeln!(w, "  }} > /target/etc/mdadm/mdadm.conf")?;
    // Every array we created (system, boot, ESP and each SHR slice) must be listed, or it is not assembled at boot.
    // `--detail --scan` only lists running arrays, so fill any gap from the array itself.
    for a in &plan.arrays {
        writeln!(w, "  grep -Eq '^ARRAY [^ ]*{n}( |$)' /target/etc/mdadm/mdadm.conf || {{", n = a.name)?;
        writeln!(w, "    warn \"mdadm.conf is missing array {n}; adding it\"", n = a.name)?;
        writeln!(w, "    mdadm --detail --brief /dev/md/{n} >> /target/etc/mdadm/mdadm.conf", n = a.name)?;
        writeln!(w, "  }}")?;
    }
    writeln!(w, "  cat /target/etc/fstab /target/etc/mdadm/mdadm.conf >> \"$LOG\"")?;
    writeln!(w, "fi")?;
    writeln!(
        w,
        "if [ \"$DRY\" != 1 ] && [ -n \"$KEYRING\" ] && [ ! -f /target/usr/share/keyrings/debian-archive-keyring.pgp ] && [ ! -f /target/usr/share/keyrings/debian-archive-keyring.gpg ]; then"
    )?;
    writeln!(w, "  log \"target has no archive keyring yet; copying the installer's\"")?;
    writeln!(w, "  mkdir -p /target/usr/share/keyrings && cp \"$KEYRING\" /target/usr/share/keyrings/debian-archive-keyring.gpg")?;
    writeln!(w, "fi")?;
    writeln!(w, "run mkdir -p /target/opt/synofurb-install")?;
    writeln!(w, "run tar -xzf /tmp/synofurb-payload.tar.gz -C /target/opt/synofurb-install")?;
    // runtime values only known inside the installer
    let mut grub_disks = String::new();
    for d in &plan.system_disks {
        let _ = write!(grub_disks, "/dev/$D_{} ", ident(d));
    }
    writeln!(w, "if [ \"$DRY\" != 1 ]; then")?;
    writeln!(w, "  {{")?;
    writeln!(w, "    echo \"SYSTEM_DISKS=\\\"{}\\\"\"", grub_disks.trim_end())?;
    writeln!(w, "    echo \"ROOT_UUID=$(uuid /dev/mapper/vg_sys-root)\"")?;
    writeln!(w, "  }} >> /target/opt/synofurb-install/env.sh")?;
    writeln!(w, "fi")?;
    writeln!(w, "for d in dev proc sys dev/pts; do try mkdir -p /target/$d; done")?;
    writeln!(w, "run mount -o bind /dev /target/dev")?;
    writeln!(w, "run mount -t proc proc /target/proc")?;
    writeln!(w, "run mount -t sysfs sysfs /target/sys")?;
    writeln!(w, "try mount -t devpts devpts /target/dev/pts")?;
    writeln!(w, "try cp /etc/resolv.conf /target/etc/resolv.conf")?;

    // -- 11. configure in chroot -----------------------------------------------------------------
    writeln!(w, "\nstep 11 \"Configuring the system (packages, users, SSH, serial console, network, bootloader)\"")?;
    writeln!(w, "run chroot /target /bin/sh /opt/synofurb-install/chroot-setup.sh")?;
    if plan.raw_boot.is_some() {
        // U-Boot boards cannot find Debian by themselves: the user needs these commands on the serial
        // console BEFORE the reboot (DSM's system partition is gone, so U-Boot's default boot fails).
        writeln!(w, "if [ -f /target/boot/uboot-env.txt ]; then")?;
        writeln!(w, "  emit \"INFO  uboot-env| ==== U-Boot commands for the NAS serial console (also in /boot/uboot-env.txt) ====\"")?;
        writeln!(w, "  while IFS= read -r _l; do emit \"INFO  uboot-env| $_l\"; done < /target/boot/uboot-env.txt")?;
        writeln!(w, "else")?;
        writeln!(
            w,
            "  warn \"no /boot/uboot-env.txt was produced (is the board DTB configured?); Debian will not boot until U-Boot is set up by hand\""
        )?;
        writeln!(w, "fi")?;
    }
    if !plan.kept_arrays.is_empty() {
        writeln!(w, "log \"verifying that preserved arrays were not touched\"")?;
        for name in &plan.kept_arrays {
            let Some(arr) = p.facts.arrays.iter().find(|a| &a.name == name) else { continue };
            for m in &arr.members {
                let Some((d, part)) = p.facts.locate_part(m) else { continue };
                let id = ident(&d.name);
                writeln!(w, "_dev=\"/dev/$(pdev \"$D_{id}\" {})\"", part.number)?;
                writeln!(w, "if [ \"$DRY\" != 1 ]; then")?;
                writeln!(w, "  _got=$(mdadm --examine --export \"$_dev\" 2>/dev/null | sed -n 's/^MD_UUID=//p' | tr 'A-F' 'a-f')")?;
                if arr.uuid.is_empty() {
                    writeln!(w, "  [ -n \"$_got\" ] || fail \"preserved array {name}: no RAID superblock found on $_dev after install\"")?;
                } else {
                    writeln!(
                        w,
                        "  [ \"$_got\" = {} ] || fail \"preserved array {name}: superblock on $_dev is missing or changed (got '$_got')\"",
                        sh_quote(&arr.uuid.to_lowercase())
                    )?;
                }
                writeln!(w, "fi")?;
                writeln!(w, "log \"preserved array {name}: member $_dev superblock intact\"")?;
            }
        }
    }

    // -- 12. finalize ------------------------------------------------------------------------------------------
    writeln!(w, "\nstep 12 \"Finishing: saving logs, unmounting, rebooting\"")?;
    if plan.dom_boot {
        writeln!(w, "log \"making Debian the default entry of the boot flash (DSM stays selectable)\"")?;
        writeln!(w, "run synofurb-dom default debian --yes")?;
    }
    writeln!(w, "run mkdir -p /target/var/log/synofurb")?;
    writeln!(w, "try cp \"$LOG\" /target/var/log/synofurb/install.log")?;
    writeln!(w, "try cp /tmp/synofurb-stage2.sh /target/var/log/synofurb/stage2.sh")?;
    writeln!(w, "sync")?;
    writeln!(
        w,
        "for m in /target/dev/pts /target/dev /target/proc /target/sys /target/srv/data /target/boot/efi /target/boot /target; do try umount $m; done"
    )?;
    writeln!(w, "try vgchange -an --noudevsync")?;
    writeln!(w, "emit \"@@DONE install completed\"")?;
    writeln!(w, "try cp \"$LOG\" /var/log/synofurb-install.final.log")?;
    writeln!(w, "sync; [ \"$DRY\" = 1 ] || sleep 8")?;
    writeln!(w, "[ \"$DRY\" = 1 ] && exit 0")?;
    writeln!(w, "reboot -f 2>/dev/null || reboot")?;
    writeln!(w, "while :; do sleep 3600; done")?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::{fixtures, parse_inspect};

    fn params<'a>(plan: &'a Plan, facts: &'a DeviceFacts, cfg: &'a InstallConfig) -> ScriptParams<'a> {
        ScriptParams {
            plan,
            facts,
            cfg,
            run_id: "test-run",
            http_base: "http://192.168.1.10:8800/tok",
            payload_sha256: "00",
            log_host: "192.168.1.10",
            log_tcp_port: 8801,
            dry_run: true,
        }
    }

    fn render(mode: StorageMode) -> (String, Plan) {
        let facts = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let plan = build_plan(&facts, &PlanRequest { mode, ..Default::default() });
        let cfg = InstallConfig::new("nas", "nasadmin", format!("${}${}${}", "6", "saltsaltsaltsalt", "x".repeat(86)));
        (render_stage2(&params(&plan, &facts, &cfg)).unwrap(), plan)
    }

    fn sh_syntax_ok(script: &str) {
        let dir = std::env::temp_dir().join(format!("sf-sh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(format!("s{}.sh", script.len()));
        std::fs::write(&f, script).unwrap();
        for sh in ["dash", "sh"] {
            if let Ok(out) = std::process::Command::new(sh).arg("-n").arg(&f).output() {
                assert!(out.status.success(), "{sh} -n failed: {}", String::from_utf8_lossy(&out.stderr));
            }
        }
    }

    #[test]
    fn wipe_script_is_valid_sh_and_has_all_steps() {
        let (s, _) = render(StorageMode::WipeAll);
        sh_syntax_ok(&s);
        for n in 1..=TOTAL_STEPS {
            assert!(s.contains(&format!("\nstep {n} ")), "missing step {n}");
        }
        assert!(s.contains("mklabel gpt"));
        assert!(s.contains("mdadm --create /dev/md/system"));
        assert!(s.contains("mdadm --create /dev/md/data --run --force --level=5 --raid-devices=4"));
        assert!(s.contains("--bitmap=internal"));
        assert!(s.contains("mkfs.xfs -f -L SF_ROOT /dev/mapper/vg_sys-root"));
        assert!(s.contains("lvcreate --noudevsync -y -n data -l 80%VG vg_data"));
        assert!(s.contains("set 1 bios_grub on"));
        for ph in ["@@TOTAL@@", "@@DRY@@", "@@DISK_SERIAL_FN@@"] {
            assert!(!s.contains(ph), "unreplaced placeholder {ph}");
        }
        assert!(s.contains("DESTROYING: Data volume: md2"));
    }

    #[test]
    fn point_of_no_return_comes_after_all_safety_checks() {
        let (s, _) = render(StorageMode::WipeAll);
        let ponr = s.find("@@POINT_OF_NO_RETURN").unwrap();
        for needle in ["expect_part \"$D_sda\"", "expect_nparts", "payload checksum mismatch", "archive keyring not found", "Release"] {
            assert!(s.find(needle).unwrap() < ponr, "{needle} must run before the point of no return");
        }
        for needle in ["mklabel gpt", "mdadm --zero-superblock", "parted -s -a none", "mdadm --create"] {
            assert!(s.find(needle).unwrap() > ponr, "{needle} must run after the point of no return");
        }
    }

    #[test]
    fn keep_mode_never_recreates_tables_and_verifies_preserved_arrays() {
        let (s, plan) = render(StorageMode::KeepVolumes);
        sh_syntax_ok(&s);
        assert!(!s.contains("mklabel"), "keep mode must not create a new GPT");
        assert!(s.contains("parted -s \"/dev/$D_sda\" rm 1"));
        assert!(s.contains("parted -s \"/dev/$D_sda\" rm 2"));
        assert!(!s.contains(" rm 5"), "data partition must never be deleted");
        assert!(s.contains("preserved array md2"));
        assert!(s.contains("cccccccc:bbbbbbbb:cccccccc:00000002"));
        assert!(s.contains("PRESERVING: Data volume"));
        // No command that writes or wipes may ever reference the preserved partition 5.
        // (Reading its RAID superblock for verification is fine.)
        for line in s.lines() {
            let mutating =
                line.starts_with("zero_ends") || line.starts_with("zero_range") || line.starts_with("run ") || line.starts_with("try ");
            if mutating && line.contains("pdev") {
                assert!(
                    !line.contains(" 5)") && !line.contains(" 5 "),
                    "preserved partition touched: {line} (kept {})",
                    plan.kept_arrays.join(",")
                );
            }
        }
        assert!(s.contains("mdadm --examine --export"));
    }

    #[test]
    fn refuses_plans_with_errors() {
        let facts = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let plan = build_plan(&facts, &PlanRequest { system_copies: Some(1), ..Default::default() });
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        assert!(render_stage2(&params(&plan, &facts, &cfg)).is_err());
    }

    #[test]
    fn armv7_script_has_no_grub_partitions() {
        let mut facts = parse_inspect(&fixtures::ds_4bay()).unwrap();
        facts.uname_m = "armv7l".into();
        facts.unique = "synology_armada38x_ds116".into();
        facts.arch = Arch::Armhf;
        facts.firmware = Firmware::UBoot;
        facts.mem_kib = 512 * 1024;
        let plan = build_plan(&facts, &PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() });
        assert!(plan.is_ok(), "{:?}", plan.errors);
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let s = render_stage2(&params(&plan, &facts, &cfg)).unwrap();
        sh_syntax_ok(&s);
        assert!(s.contains("--arch=armhf"));
        assert!(!s.contains("bios_grub") && !s.contains("sf-esp") && !s.contains("sf-boot"));
        assert!(s.contains("zero_range"), "raw U-Boot area must be wiped");
        assert!(!s.contains("DEV_BOOT"));
    }

    #[test]
    fn live_script_can_be_forced_into_dry_run_from_the_environment() {
        let facts = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let plan = build_plan(&facts, &PlanRequest::default());
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let mut p = params(&plan, &facts, &cfg);
        p.dry_run = false;
        let s = render_stage2(&p).unwrap();
        assert!(s.contains("DRY=${SYNOFURB_DRY:-0}"));
        let dir = std::env::temp_dir().join(format!("sf-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("live.sh");
        std::fs::write(&script, &s).unwrap();
        let log = dir.join("install.log");
        let out = std::process::Command::new("sh")
            .arg(&script)
            .env("SYNOFURB_LOG", &log)
            .env("SYNOFURB_DRY", "1")
            .env("SYNOFURB_LOCK", dir.join("lock"))
            .output()
            .unwrap();
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(out.status.success(), "{text}");
        assert!(text.contains("dry=1") && text.contains("@@DONE"), "{text}");
    }

    fn dom_amd_facts() -> DeviceFacts {
        let mut s = fixtures::ds_4bay();
        s = s
            .replace("KV|unique|synology_apollolake_918+", "KV|unique|synology_v1000_1621+")
            .replace("KV|model|DS918+", "KV|model|DS1621+");
        s = s
            .replace("KV|uefi|0\n", "KV|uefi|1\nKV|cpu_vendor|AuthenticAMD\n")
            .replace("NIC|eth0", "DOM|262144|65536|196608|1|1\nNIC|eth0");
        parse_inspect(&s).unwrap()
    }

    #[test]
    fn dom_unit_has_no_boot_partitions_a_failsafe_and_switches_default_last() {
        let facts = dom_amd_facts();
        let plan = build_plan(&facts, &PlanRequest::default());
        assert!(plan.is_ok(), "{:?}", plan.errors);
        assert!(plan.dom_boot && plan.boot.is_none() && plan.esp.is_none());
        let roles: Vec<PartRole> = plan.disks[0].create.iter().map(|c| c.role).collect();
        assert!(!roles.iter().any(|r| matches!(r, PartRole::Boot | PartRole::Esp | PartRole::BiosGrub)), "{roles:?}");
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let s = render_stage2(&params(&plan, &facts, &cfg)).unwrap();
        sh_syntax_ok(&s);
        assert!(s.contains("DOM_BOOT=1") && s.contains("synofurb-dom default dsm --yes"));
        let ponr = s.find("PONR=1").unwrap();
        let fallback = s.find("FALLBACK").unwrap();
        assert!(fallback < ponr, "the fallback handler is defined before the point of no return");
        let default_debian = s.find("run synofurb-dom default debian --yes").unwrap();
        assert!(
            s.find("run chroot /target").unwrap() < default_debian && default_debian < s.rfind("reboot -f").unwrap(),
            "default switches only after a successful configuration"
        );
        assert!(!render(StorageMode::WipeAll).0.contains("synofurb-dom default debian"), "non-DOM units are untouched");
    }

    #[test]
    fn the_failure_handler_makes_no_false_promises_and_its_times_come_from_one_place() {
        let facts = dom_amd_facts();
        let plan = build_plan(&facts, &PlanRequest::default());
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let dom = render_stage2(&params(&plan, &facts, &cfg)).unwrap();
        sh_syntax_ok(&dom);
        assert!(dom.contains("_w=60\n") && dom.contains("_w=900\n") && dom.contains("in 15 minutes") && dom.contains("in 60 seconds"));
        assert!(!dom.contains("@@FAIL_"), "every placeholder is replaced");
        let (plain, plain_plan) = render(StorageMode::WipeAll);
        for (name, s) in [("dom", &dom), ("plain", &plain)] {
            let l = s.to_lowercase();
            for bad in ["keeps data volumes", "rescue shell", "data volumes are kept", "recovery / reinstall"] {
                assert!(!l.contains(bad), "{name}: {bad}");
            }
        }
        assert!(dom.contains("cannot promise that any data survives"));
        assert!(plain.contains("will NOT reboot by itself") && plain.contains("power-cycle"));
        // the text helper agrees with the script
        let h = failure_handling(&plan);
        assert_eq!((h.reboots_by_itself, h.before_ponr_secs, h.after_ponr_secs), (true, Some(60), Some(900)));
        let n = failure_handling(&plain_plan);
        assert_eq!((n.reboots_by_itself, n.before_ponr_secs, n.after_ponr_secs), (false, None, None));
    }

    #[test]
    fn uboot_commands_are_printed_before_the_reboot() {
        let mut facts = parse_inspect(&fixtures::ds_4bay()).unwrap();
        facts.uname_m = "armv7l".into();
        facts.unique = "synology_armada38x_ds116".into();
        facts.arch = Arch::Armhf;
        facts.firmware = Firmware::UBoot;
        facts.mem_kib = 512 * 1024;
        let plan = build_plan(&facts, &PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() });
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let s = render_stage2(&params(&plan, &facts, &cfg)).unwrap();
        let chroot = s.find("run chroot /target").unwrap();
        let recipe = s.find("uboot-env|").unwrap();
        let reboot = s.rfind("reboot -f").unwrap();
        assert!(chroot < recipe && recipe < reboot, "recipe must be printed after configuration and before the reboot");
        let (_, x86_plan) = render(StorageMode::WipeAll);
        assert!(x86_plan.raw_boot.is_none());
        assert!(!render(StorageMode::WipeAll).0.contains("uboot-env"));
    }

    // ------------------------------------------------------------------------------------ SHR

    /// Facts with plain disks of the given DATA sizes (GiB), system mirrored on all of them.
    fn shr_setup(data_gib: &[u64], raid: DataRaid) -> (DeviceFacts, Plan) {
        let mut facts = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let names = ["sda", "sdb", "sdc", "sdd"];
        facts.disks.retain(|d| d.name == "synoboot" || names[..data_gib.len()].contains(&d.name.as_str()));
        facts.arrays.clear();
        for d in facts.disks.iter_mut().filter(|d| d.name != "synoboot") {
            let i = names.iter().position(|n| *n == d.name).unwrap();
            d.sectors = (1 + 1 + 1024 + 8192 + data_gib[i] * 1024) * SECTORS_PER_MIB + 34;
        }
        let req = PlanRequest { data_raid: raid, system_copies: Some(data_gib.len()), system_size_mib: 8192, ..Default::default() };
        let plan = build_plan(&facts, &req);
        assert!(plan.is_ok(), "{:?}", plan.errors);
        (facts, plan)
    }

    fn shr_script(data_gib: &[u64], raid: DataRaid) -> (String, Plan) {
        let (facts, plan) = shr_setup(data_gib, raid);
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        (render_stage2(&params(&plan, &facts, &cfg)).unwrap(), plan)
    }

    #[test]
    fn shr_script_creates_one_array_per_slice_and_joins_them_in_one_vg() {
        let (s, _) = shr_script(&[32, 64, 64, 128], DataRaid::Shr1);
        sh_syntax_ok(&s);
        // slice 1: 4 members RAID5, slice 2: 3 members RAID5; the single-disk slice 3 is not created
        let a1 = s.lines().find(|l| l.contains("mdadm --create /dev/md/data1")).expect("data1");
        assert!(a1.contains("--level=5 --raid-devices=4") && a1.contains("--metadata=1.2") && a1.contains("--bitmap=internal"), "{a1}");
        let a2 = s.lines().find(|l| l.contains("mdadm --create /dev/md/data2")).expect("data2");
        assert!(a2.contains("--level=5 --raid-devices=3"), "{a2}");
        assert!(!s.contains("/dev/md/data3") && !s.contains("md/data --"));
        assert!(s.contains("Creating RAID arrays (4 array(s))"), "boot + system + data1 + data2");
        // every PV is initialised, then one vgcreate takes all of them
        assert!(s.contains("PV_vg_data_1=/dev/md/data1") && s.contains("PV_vg_data_2=/dev/md/data2"));
        assert!(s.contains("run pvcreate -ff -y \"$PV_vg_data_1\"") && s.contains("run pvcreate -ff -y \"$PV_vg_data_2\""));
        assert!(s.contains("run vgcreate vg_data \"$PV_vg_data_1\" \"$PV_vg_data_2\""));
        assert!(s.contains("run vgcreate vg_sys \"$PV_vg_sys\""), "single-PV groups keep their form");
        // a plain linear LV over all PVs
        assert!(s.contains("lvcreate --noudevsync -y -n data -l 80%VG vg_data"));
        // creation order: partitions -> arrays -> pvcreate -> vgcreate -> lvcreate -> mkfs
        let pos = |n: &str| s.find(n).unwrap_or_else(|| panic!("{n}"));
        assert!(pos("mkpart sf-data") < pos("mdadm --create /dev/md/data1"));
        assert!(pos("mdadm --create /dev/md/data2") < pos("pvcreate -ff -y \"$PV_vg_data_1\""));
        assert!(pos("pvcreate -ff -y \"$PV_vg_data_2\"") < pos("vgcreate vg_data"));
        assert!(pos("vgcreate vg_data") < pos("lvcreate --noudevsync -y -n data"));
        assert!(pos("lvcreate --noudevsync -y -n data") < pos("mkfs.xfs -f -L SF_DATA"));
        // 4 + 3 data partitions are created (sda, sdb, sdc, sdd carry 1, 2, 2, 2)
        assert_eq!(s.matches("mkpart sf-data").count(), 7);
    }

    #[test]
    fn shr2_script_uses_raid6_and_mdadm_conf_lists_every_array() {
        let (s, plan) = shr_script(&[64, 64, 64, 64], DataRaid::Shr2);
        sh_syntax_ok(&s);
        assert!(s.contains("mdadm --create /dev/md/data1 --run --force --level=6 --raid-devices=4"));
        assert!(s.contains("PV_vg_data=/dev/md/data1") && s.contains("run vgcreate vg_data \"$PV_vg_data\""));
        // mdadm.conf: scan output plus a per-array safety net, all before the initramfs is built in the chroot
        for a in &plan.arrays {
            assert!(s.contains(&format!("grep -Eq '^ARRAY [^ ]*{}( |$)' /target/etc/mdadm/mdadm.conf", a.name)), "{}", a.name);
            assert!(s.contains(&format!("mdadm --detail --brief /dev/md/{} >> /target/etc/mdadm/mdadm.conf", a.name)));
        }
        assert!(s.find("mdadm.conf").unwrap() < s.find("run chroot /target").unwrap());
    }

    #[test]
    fn shr_script_dry_run_executes_end_to_end() {
        let (facts, plan) = shr_setup(&[32, 64, 64, 128], DataRaid::Shr1);
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let s = render_stage2(&params(&plan, &facts, &cfg)).unwrap();
        let dir = std::env::temp_dir().join(format!("sf-shr-dry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("stage2.sh");
        std::fs::write(&script, &s).unwrap();
        let log = dir.join("install.log");
        let out = std::process::Command::new("sh")
            .arg(&script)
            .env("SYNOFURB_LOG", &log)
            .env("SYNOFURB_LOCK", dir.join("lock"))
            .output()
            .expect("sh available");
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(out.status.success(), "dry-run exit status; log:\n{text}");
        assert!(text.contains("@@DONE") && !text.contains("@@FAIL"), "{text}");
        for needle in ["CMD   mdadm --create /dev/md/data1", "CMD   mdadm --create /dev/md/data2", "CMD   vgcreate vg_data"] {
            assert!(text.contains(needle), "{needle} missing in\n{text}");
        }
        assert_eq!(text.matches("CMD   pvcreate -ff -y").count(), 2 + 1, "data1, data2 and the system PV");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_shr_scripts_are_unchanged_in_shape() {
        let (s, _) = render(StorageMode::WipeAll);
        assert!(s.contains("PV_vg_data=/dev/md/data") && s.contains("run vgcreate vg_data \"$PV_vg_data\""));
        assert!(!s.contains("PV_vg_data_1"));
    }

    /// Runs the generated partitioning commands (real `parted`, only the disk variables are bound) on
    /// loop-device images and compares the kernel's view with the plan. Needs root, `losetup` and `parted`;
    /// skipped when they are missing. Does NOT run mdadm/LVM (no md/dm support needed or exercised).
    #[test]
    fn shr_partitioning_on_real_loop_devices_matches_the_plan() {
        let have =
            |t: &str| std::process::Command::new("sh").args(["-c", &format!("command -v {t}")]).output().is_ok_and(|o| o.status.success());
        if !(have("losetup") && have("parted") && std::process::Command::new("id").arg("-u").output().is_ok_and(|o| o.stdout == b"0\n")) {
            eprintln!("skipping: needs root, losetup and parted");
            return;
        }
        // data sizes 4, 4, 8 GiB (sparse images, a few MiB actually written)
        let (facts, plan) = shr_setup(&[4, 4, 8], DataRaid::Shr1);
        let dir = std::env::temp_dir().join(format!("sf-shr-loop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        struct Loops(Vec<String>, std::path::PathBuf);
        impl Drop for Loops {
            fn drop(&mut self) {
                for l in &self.0 {
                    let _ = std::process::Command::new("losetup").args(["-d", l]).status();
                }
                let _ = std::fs::remove_dir_all(&self.1);
            }
        }
        let mut loops = Loops(vec![], dir.clone());
        let mut binds = String::new();
        let mut devs: Vec<(String, String)> = vec![]; // (plan disk, loopN)
        for dp in &plan.disks {
            let img = dir.join(format!("{}.img", dp.disk));
            let sectors = facts.disk(&dp.disk).unwrap().sectors;
            std::fs::File::create(&img).unwrap().set_len(sectors * SECTOR).unwrap();
            let out = std::process::Command::new("losetup").args(["-f", "--show"]).arg(&img).output().unwrap();
            if !out.status.success() {
                eprintln!("skipping: losetup failed: {}", String::from_utf8_lossy(&out.stderr));
                return;
            }
            let dev = String::from_utf8_lossy(&out.stdout).trim().to_string();
            loops.0.push(dev.clone());
            let name = dev.trim_start_matches("/dev/").to_string();
            binds.push_str(&format!("D_{}={}\n", ident(&dp.disk), name));
            devs.push((dp.disk.clone(), name));
        }
        let (facts2, plan2) = (facts, plan);
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let script = render_stage2(&params(&plan2, &facts2, &cfg)).unwrap();
        let part: Vec<&str> = script
            .split("\nstep 4 ")
            .nth(1)
            .unwrap()
            .split("\nsettle\n")
            .next()
            .unwrap()
            .lines()
            .skip(1)
            .filter(|l| l.starts_with("run parted") || l.starts_with("try blockdev"))
            .collect();
        let sh = format!("run() {{ \"$@\" || exit 1; }}\ntry() {{ \"$@\"; return 0; }}\n{binds}{}\n", part.join("\n"));
        let f = dir.join("part.sh");
        std::fs::write(&f, sh).unwrap();
        let out = std::process::Command::new("sh").arg(&f).output().unwrap();
        assert!(
            out.status.success(),
            "partitioning failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // compare with the kernel's idea of every partition
        let mut checked = 0;
        for dp in &plan2.disks {
            let name = &devs.iter().find(|d| d.0 == dp.disk).unwrap().1;
            for c in &dp.create {
                let sys = std::path::Path::new("/sys/block").join(name).join(format!("{name}p{}", c.number));
                let rd = |f: &str| {
                    std::fs::read_to_string(sys.join(f)).unwrap_or_else(|e| panic!("{}: {e}", sys.display())).trim().parse::<u64>().unwrap()
                };
                assert_eq!((rd("start"), rd("size")), (c.start, c.sectors), "{} partition {}", dp.disk, c.number);
                checked += 1;
            }
        }
        // 3 disks x (bios_grub, boot, system) + data partitions of the 4+4+8 layout: 1 per disk
        assert_eq!(checked, 3 * 3 + 3);
        // GPT type flags: the data partitions are raid, as md members need
        let print = std::process::Command::new("parted").args(["-s", &format!("/dev/{}", devs[0].1), "print"]).output().unwrap();
        let print = String::from_utf8_lossy(&print.stdout).to_string();
        assert!(print.contains("sf-data") && print.contains("raid"), "{print}");
    }

    #[test]
    fn dry_run_executes_end_to_end() {
        let (s, _) = render(StorageMode::WipeAll);
        let dir = std::env::temp_dir().join(format!("sf-dry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("stage2.sh");
        std::fs::write(&script, &s).unwrap();
        let log = dir.join("install.log");
        let out = std::process::Command::new("sh")
            .arg(&script)
            .env("SYNOFURB_LOG", &log)
            .env("SYNOFURB_LOCK", dir.join("lock"))
            .output()
            .expect("sh available");
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(out.status.success(), "dry-run exit status; log:\n{text}");
        assert!(text.contains("@@DONE"), "{text}");
        assert!(text.contains("CMD   mdadm --create /dev/md/boot"));
        assert!(!text.contains("@@FAIL"), "{text}");
    }
}
