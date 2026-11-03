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
/// The reboot needs `synofurb-dom` in the installer image; the app's `build_initrd` always puts it there ([`crate::debian::INSTALLER_DOM_TOOL`]).
pub fn failure_handling(plan: &Plan) -> FailureHandling {
    failure_handling_with_tool(plan, crate::debian::installer_has_dom_tool())
}

/// [`failure_handling`] for an installer image that does or does not contain `synofurb-dom`: without it the script halts and does not
/// reboot by itself, whatever the firmware.
pub fn failure_handling_with_tool(plan: &Plan, installer_has_dom_tool: bool) -> FailureHandling {
    if plan.dom_boot && installer_has_dom_tool {
        FailureHandling {
            reboots_by_itself: true,
            before_ponr_secs: Some(FAIL_REBOOT_BEFORE_PONR_SECS),
            after_ponr_secs: Some(FAIL_REBOOT_AFTER_PONR_SECS),
        }
    } else {
        FailureHandling { reboots_by_itself: false, before_ponr_secs: None, after_ponr_secs: None }
    }
}

/// Room the Debian kernel + initramfs need on the boot flash's ext2 partition, in MiB. Measured 2026-11-03 on a minbase + the
/// package list of `chroot-setup.sh` (`MODULES=list`, `COMPRESS=xz`, intel-microcode for the host CPU): Linux 6.12.107 is 12.1 MB +
/// 13.6 MB = 24.6 MiB, Linux 7.2.11 (trixie-backports) is 14.3 MB + 15.1 MB = 28.0 MiB. Rounded up with headroom for the next point
/// release; the DEBIAN_OLD fallback is dropped by `synofurb-dom` when there is no room for it, so it is not counted here.
pub const DOM_DEBIAN_NEED_MIB: u64 = 32;

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
# Where is synofurb-dom in THIS environment? The installer's PATH may not include /usr/local/sbin, where the app puts it, so look
# at the absolute locations first and only then at PATH. (SYNOFURB_DOM_DIRS: tests.)
SFDOM=
for _d in ${SYNOFURB_DOM_DIRS:-/usr/local/sbin /usr/local/bin /usr/sbin /usr/bin /sbin /bin}; do
  if [ -x "$_d/synofurb-dom" ]; then SFDOM="$_d/synofurb-dom"; break; fi
done
[ -n "$SFDOM" ] || SFDOM=$(command -v synofurb-dom 2>/dev/null || true)
fail() {
  emit "ERROR $*"
  emit "@@FAIL $*"
  diag
  if [ "$DOM_BOOT" = 1 ] && [ -n "$SFDOM" ]; then
    # FAIL-SAFE: this unit boots from its internal flash. Point the default back at DSM and reboot, so a
    # failed or abandoned install never leaves the NAS unreachable.
    emit "FALLBACK: switching the boot flash default back to DSM"
    [ "$DRY" = 1 ] || "$SFDOM" default dsm --yes >> "$LOG" 2>&1 || emit "WARN  could not reset the default boot entry; power-cycle the NAS and pick DSM"
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
  [ "$DOM_BOOT" = 1 ] && emit "WARN  synofurb-dom was not found in the installer, so the boot flash default was NOT reset"
  [ "$DRY" = 1 ] && exit 1
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

# release_storage : stop everything that holds the disks' partitions. The installer's udev rules often assemble DSM's md arrays (and
# activate its LVM volume group) as soon as the disks are seen; repartitioning a disk under a running array fails half-way, and
# zeroing the members of a running array corrupts the array in memory. Rounds, because a volume group must go before its array.
# Returns 1 if an md array or a device-mapper map is still there after the last round.
md_left() { grep '^md[0-9]' "${SYNOFURB_MDSTAT:-/proc/mdstat}" 2>/dev/null | cut -d' ' -f1 | tr '\n' ' '; }
dm_left() { ls "${SYNOFURB_SYSBLOCK:-/sys/block}" 2>/dev/null | grep '^dm-' | tr '\n' ' '; }
release_storage() {
  emit "INFO  releasing: md arrays [$(md_left)] device-mapper [$(dm_left)] volume groups [$(vgs --noheadings -o vg_name 2>/dev/null | tr -d ' ' | tr '\n' ' ')]"
  _round=0
  while [ $_round -lt 5 ]; do
    _round=$((_round + 1))
    command -v vgchange >/dev/null 2>&1 && vgchange -an --noudevsync >> "$LOG" 2>&1
    command -v dmsetup >/dev/null 2>&1 && dmsetup remove_all >> "$LOG" 2>&1
    if command -v mdadm >/dev/null 2>&1; then
      mdadm --stop --scan >> "$LOG" 2>&1
      for _a in $(md_left); do mdadm --stop "/dev/$_a" >> "$LOG" 2>&1; done
    fi
    if [ -z "$(md_left)$(dm_left)" ]; then emit "INFO  released after $_round round(s): nothing holds the disks any more"; return 0; fi
    sleep 1
  done
  return 1
}

# dom_flash_check : the boot flash must have room for Debian's kernel + initramfs BEFORE any disk is wiped; finding out afterwards leaves a
# unit that has neither DSM (its system partition is gone) nor Debian. Counts the fallback files (.old) as free: synofurb-dom drops them
# first. If it is too full, everything synofurb added (the boot agent, 20 MB) is removed: the agent has done its job once the installer runs.
json_p2_free() { printf '%s' "$1" | sed -n 's/.*"p2":{[^}]*"free_bytes":\([0-9][0-9]*\).*/\1/p'; }
json_file_size() { printf '%s' "$1" | tr '{' '\n' | sed -n "s/^\"name\":\"$2\",\"size\":\([0-9][0-9]*\).*/\1/p" | head -n1; }
dom_free_bytes() {
  _st=$("$SFDOM" status --json 2>>"$LOG") || return 1
  _f=$(json_p2_free "$_st")
  [ -n "$_f" ] || return 1
  _o1=$(json_file_size "$_st" 'vmlinuz\.old'); _o2=$(json_file_size "$_st" 'initrd\.img\.old')
  echo $((_f + ${_o1:-0} + ${_o2:-0}))
}
dom_flash_check() { # NEED_MIB
  _need=$(($1 * 1048576))
  _free=$(dom_free_bytes) || fail "preflight: cannot read the boot flash with $SFDOM status --json; nothing was changed"
  log "boot flash: $((_free / 1048576)) MiB free for Debian's kernel + initramfs on the DSM partition, about $1 MiB needed"
  if [ "$_free" -lt "$_need" ]; then
    warn "the boot flash is too full; removing everything synofurb added (boot agent, old Debian entries) to make room - DSM's own files are never touched"
    "$SFDOM" remove --yes >> "$LOG" 2>&1 || fail "preflight: could not clean the boot flash; nothing else was changed"
    _free=$(dom_free_bytes) || fail "preflight: cannot read the boot flash after cleaning it; nothing was changed"
    log "boot flash: $((_free / 1048576)) MiB free after cleaning"
    [ "$_free" -ge "$_need" ] || fail "preflight: the boot flash has only $((_free / 1048576)) MiB free next to DSM's files, Debian's kernel + initramfs need about $1 MiB. Nothing was changed; DSM stays the default"
  fi
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
    if plan.dom_boot {
        // before anything is touched: without the tool the installer can neither fall back to DSM nor register Debian
        writeln!(
            w,
            "[ -n \"$SFDOM\" ] || fail \"preflight: synofurb-dom is not in the installer image, so the boot flash cannot be managed; nothing was changed\""
        )?;
        writeln!(w, "log \"boot flash manager: $SFDOM\"")?;
    }

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

    if plan.dom_boot {
        writeln!(w, "[ \"$DRY\" = 1 ] || dom_flash_check {DOM_DEBIAN_NEED_MIB}")?;
    }

    // -- 3. point of no return ----------------------------------------------
    writeln!(w, "\nstep 3 \"POINT OF NO RETURN: releasing existing RAID/LVM and wiping reclaimed areas\"")?;
    // Stopping DSM's arrays changes nothing on the disks, so a failure here is still "nothing was changed".
    writeln!(w, "emit \"CMD   release_storage (stop old RAID arrays and LVM volume groups)\"")?;
    writeln!(
        w,
        "[ \"$DRY\" = 1 ] || release_storage || fail \"cannot stop the existing RAID arrays/LVM volumes (still active: $(md_left)$(dm_left)); nothing was changed\""
    )?;
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
    let has_data_lv = plan.has_data_volume();
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
        writeln!(w, "run \"$SFDOM\" default debian --yes")?;
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
    // Stop the arrays so the superblocks are written clean and a resync still running (a new RAID5 needs many hours) records where it
    // stands; `reboot -f` would leave them dirty and the new system would start over from the last checkpoint.
    writeln!(w, "try mdadm --stop --scan")?;
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
        assert!(s.contains("DOM_BOOT=1") && s.contains("default dsm --yes"));
        let ponr = s.find("PONR=1").unwrap();
        let fallback = s.find("FALLBACK").unwrap();
        assert!(fallback < ponr, "the fallback handler is defined before the point of no return");
        let default_debian = s.find("run \"$SFDOM\" default debian --yes").unwrap();
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

    // ---- the failure handler in a stand-in installer environment: the tool is NOT on PATH, `sleep` and `reboot` are stubs

    struct Env {
        dir: std::path::PathBuf,
    }

    impl Env {
        fn new(tag: &str, with_tool: bool) -> Env {
            let dir = std::env::temp_dir().join(format!("sf-failenv-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            for d in ["tool", "stubs"] {
                std::fs::create_dir_all(dir.join(d)).unwrap();
            }
            let put = |path: std::path::PathBuf, body: &str| {
                std::fs::write(&path, body).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
                }
            };
            let calls = dir.join("calls");
            // `reboot` never returns on a real unit and the script then sleeps for ever: the stub ends the script at that point
            put(
                dir.join("stubs/sleep"),
                &format!("#!/bin/sh\necho \"sleep $*\" >> {}\n[ \"$1\" = 3600 ] && kill -9 $PPID\nexit 0\n", calls.display()),
            );
            put(dir.join("stubs/reboot"), &format!("#!/bin/sh\necho \"reboot $*\" >> {}\n", calls.display()));
            if with_tool {
                put(dir.join("tool/synofurb-dom"), &format!("#!/bin/sh\necho \"synofurb-dom $*\" >> {}\n", calls.display()));
            }
            Env { dir }
        }

        /// Run the generated installer's preamble (functions only) followed by `tail`.
        fn run(&self, script: &str, tail: &str, dry: bool) -> (i32, String, String) {
            let head = &script[..script.find("\nRUN_ID=").expect("end of the preamble")];
            let f = self.dir.join("harness.sh");
            std::fs::write(&f, format!("{head}\n{tail}\n")).unwrap();
            let out = std::process::Command::new("sh")
                .arg(&f)
                .env_clear()
                .env("PATH", format!("{}:/usr/bin:/bin", self.dir.join("stubs").display()))
                .env("SYNOFURB_LOG", self.dir.join("log"))
                .env("SYNOFURB_LOCK", self.dir.join("lock"))
                .env("SYNOFURB_DRY", if dry { "1" } else { "0" })
                .env("SYNOFURB_DOM_DIRS", self.dir.join("tool"))
                .output()
                .unwrap();
            (
                out.status.code().unwrap_or(-1),
                std::fs::read_to_string(self.dir.join("log")).unwrap_or_default(),
                std::fs::read_to_string(self.dir.join("calls")).unwrap_or_default(),
            )
        }
    }

    impl Drop for Env {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn live_dom_script() -> String {
        let facts = dom_amd_facts();
        let plan = build_plan(&facts, &PlanRequest::default());
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let mut p = params(&plan, &facts, &cfg);
        p.dry_run = false;
        render_stage2(&p).unwrap()
    }

    #[test]
    fn the_failure_handler_finds_synofurb_dom_by_absolute_path_not_only_on_path() {
        let script = live_dom_script();
        // before the point of no return: default reset through the absolute path, 60 s, reboot
        let env = Env::new("pre", true);
        let (_, log, calls) = env.run(&script, "PONR=0\nfail \"boom\"", false);
        let lines: Vec<&str> = calls.lines().take(3).collect();
        assert_eq!(lines, ["synofurb-dom default dsm --yes", "sleep 60", "reboot -f"], "{calls}\n{log}");
        assert!(log.contains("reboots in 60 seconds") && log.contains("FALLBACK"), "{log}");
        // after it: 900 s
        let env = Env::new("post", true);
        let (_, log, calls) = env.run(&script, "PONR=1\nfail \"boom\"", false);
        assert_eq!(
            calls.lines().take(3).collect::<Vec<_>>(),
            ["synofurb-dom default dsm --yes", "sleep 900", "reboot -f"],
            "{calls}\n{log}"
        );
        assert!(log.contains("reboots in 15 minutes"), "{log}");
        // the tool is nowhere: no reboot, no promise, and it says so (dry mode so the harness does not wait for ever)
        let env = Env::new("none", false);
        let (code, log, calls) = env.run(&script, "PONR=0\nfail \"boom\"", true);
        assert_eq!(code, 1);
        assert!(calls.is_empty(), "nothing is run: {calls}");
        assert!(log.contains("will NOT reboot by itself") && log.contains("was not found in the installer"), "{log}");
        assert!(!log.contains("reboots in"), "{log}");
    }

    #[test]
    fn a_boot_flash_install_refuses_to_start_when_the_tool_is_missing_and_the_numbers_follow() {
        let script = live_dom_script();
        assert!(script.contains("preflight: synofurb-dom is not in the installer image"));
        let pre = script.find("preflight: synofurb-dom").unwrap();
        assert!(pre < script.find("PONR=1").unwrap(), "the check comes before the point of no return");
        // dry run of the whole script: with the tool the preflight passes and the log names the tool; without it the run stops at once
        let env = Env::new("pf-yes", true);
        let (code, log, _) = env.run(&format!("{script}\n"), "", true);
        let _ = code; // the harness cuts at RUN_ID, so only the preamble ran; the preflight line is checked below on the full text
        assert!(log.is_empty() || !log.contains("preflight"));
        let facts = dom_amd_facts();
        let plan = build_plan(&facts, &PlanRequest::default());
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let dry = render_stage2(&params(&plan, &facts, &cfg)).unwrap();
        for (tag, with_tool) in [("full-yes", true), ("full-no", false)] {
            let env = Env::new(tag, with_tool);
            let f = env.dir.join("full.sh");
            std::fs::write(&f, &dry).unwrap();
            let out = std::process::Command::new("sh")
                .arg(&f)
                .env_clear()
                .env("PATH", format!("{}:/usr/bin:/bin", env.dir.join("stubs").display()))
                .env("SYNOFURB_LOG", env.dir.join("log"))
                .env("SYNOFURB_LOCK", env.dir.join("lock"))
                .env("SYNOFURB_DRY", "1")
                .env("SYNOFURB_DOM_DIRS", env.dir.join("tool"))
                .output()
                .unwrap();
            let log = std::fs::read_to_string(env.dir.join("log")).unwrap_or_default();
            if with_tool {
                assert!(out.status.success() && log.contains("@@DONE") && log.contains("boot flash manager:"), "{log}");
            } else {
                assert_eq!(out.status.code(), Some(1), "{log}");
                assert!(
                    log.contains("@@FAIL preflight: synofurb-dom is not in the installer image") && !log.contains("@@STEP 3/"),
                    "{log}"
                );
            }
        }
        // the app's text helper: the 60 s / 15 min exist only when the image has the tool
        assert!(failure_handling(&plan).reboots_by_itself);
        let no_tool = failure_handling_with_tool(&plan, false);
        assert_eq!((no_tool.reboots_by_itself, no_tool.before_ponr_secs, no_tool.after_ponr_secs), (false, None, None));
        // and the app really puts the tool into the image
        assert_eq!(crate::debian::INSTALLER_DOM_TOOL, "usr/local/sbin/synofurb-dom");
        assert!(crate::debian::installer_has_dom_tool());
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

    // ---------------------------------------------------------------------------- the owner's real DS918+ (4 x 8 TB, DOM, ttyS2)

    fn ds918_facts() -> DeviceFacts {
        parse_inspect(&fixtures::ds918_two_volumes()).unwrap()
    }

    fn ds918_script(raid: DataRaid, copies: Option<usize>) -> (String, Plan) {
        let facts = ds918_facts();
        let plan = build_plan(&facts, &PlanRequest { data_raid: raid, system_copies: copies, ..Default::default() });
        assert!(plan.is_ok(), "{:?}", plan.errors);
        let mut cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        cfg.serial = facts.serial_console();
        (render_stage2(&params(&plan, &facts, &cfg)).unwrap(), plan)
    }

    #[test]
    fn the_real_ds918_installs_through_the_boot_flash_on_all_four_disks_including_the_removable_flagged_ones() {
        let (s, plan) = ds918_script(DataRaid::Shr1, Some(4));
        sh_syntax_ok(&s);
        assert!(plan.dom_boot && plan.boot.is_none() && plan.esp.is_none(), "DOM unit: no GRUB, no /boot or ESP on the disks");
        assert!(s.contains("DOM_BOOT=1") && !s.contains("bios_grub") && !s.contains("sf-esp"));
        // sda/sdb sit behind the add-in controller and say removable=1: they are wiped, partitioned and in every array like sdc/sdd
        for d in ["sda", "sdb", "sdc", "sdd"] {
            assert!(s.contains(&format!("resolve_disk D_{d} ")), "{d} located");
            assert!(s.contains(&format!("run parted -s \"/dev/$D_{d}\" mklabel gpt")), "{d} gets a new table");
            assert!(s.contains(&format!("zero_ends \"/dev/$D_{d}\"")), "{d} whole-disk metadata wiped");
            for old in [1, 2, 5, 6] {
                assert!(
                    s.contains(&format!("try mdadm --zero-superblock --force \"/dev/$(pdev \"$D_{d}\" {old})\"")),
                    "{d}{old} superblock zeroed"
                );
            }
        }
        let sys = s.lines().find(|l| l.contains("mdadm --create /dev/md/system")).unwrap();
        assert!(sys.contains("--level=1 --raid-devices=4 --metadata=1.2"), "{sys}");
        let data = s.lines().find(|l| l.contains("mdadm --create /dev/md/data1")).unwrap();
        assert!(
            data.contains("--level=5 --raid-devices=4") && data.contains("--bitmap=internal") && !data.contains("assume-clean"),
            "{data}"
        );
        // new arrays carry the new host name, never DSM's `awss01:2` / `awss01:3`
        assert!(sys.contains("--homehost='nas'") && !s.contains("awss01"));
        // the log lists what is destroyed, naming the old arrays
        assert!(s.contains("DESTROYING: Data volume: md2") && s.contains("DESTROYING: Data volume: md3"));
        assert!(s.contains("emit \"@@BEGIN run=$RUN_ID arch=amd64 firmware=Dom"));
    }

    #[test]
    fn the_console_of_the_real_unit_is_the_third_serial_port_everywhere() {
        let facts = ds918_facts();
        assert_eq!(facts.serial_console().kernel_arg(), "console=ttyS2,115200n8", "DSM's own cmdline");
        let plan = build_plan(&facts, &PlanRequest::default());
        let mut cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        cfg.serial = facts.serial_console();
        // installer (d-i): VGA first, serial LAST so it is /dev/console
        let ic = crate::debian::installer_cmdline(&cfg.serial, "192.0.2.10", 18082, false, None);
        assert!(ic.contains("console=tty0 console=ttyS2,115200n8"), "{ic}");
        // the installed system: env.sh feeds chroot-setup.sh (getty + dom.conf kernel arguments)
        let env = crate::payload::render_env(&cfg, &plan, &facts);
        assert!(
            env.contains("SERIAL_TTY='ttyS2'")
                && env.contains("SERIAL_ARG='console=ttyS2,115200n8'")
                && env.contains("SERIAL_SPEED='115200'"),
            "{env}"
        );
        assert!(env.contains("FIRMWARE='Dom'") && env.contains("DOM_BOOT='1'"), "{env}");
        assert!(env.contains("MICROCODE_PKG='intel-microcode'"), "{env}");
        let setup = include_str!("../../../payload/chroot-setup.sh");
        assert!(setup.contains("serial-getty@$SERIAL_TTY.service"), "named port: explicit getty");
        assert!(setup.contains("SERIAL_ARG=\"$SERIAL_ARG\""), "dom.conf carries the kernel argument for the boot flash entry");
        let update = include_str!("../../../payload/files/usr/local/sbin/synofurb-dom-update");
        assert!(update.contains("console=tty0 ${SERIAL_ARG:-}"), "serial argument last: it becomes /dev/console");
        // GRUB (BIOS/UEFI units only): serial unit 2 = COM3 = ttyS2
        assert!(setup.contains("UNIT=${SERIAL_TTY#ttyS}") && setup.contains("--unit=$UNIT"));
    }

    /// Run the preamble's functions (only) followed by `tail` with stub tools first on PATH.
    fn preamble_run(tag: &str, tail: &str, stubs: &[(&str, &str)], env: &[(&str, std::path::PathBuf)]) -> (i32, String, String) {
        let facts = ds918_facts();
        let plan = build_plan(&facts, &PlanRequest::default());
        let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        let mut p = params(&plan, &facts, &cfg);
        p.dry_run = false;
        let script = render_stage2(&p).unwrap();
        let head = &script[..script.find("\nRUN_ID=").unwrap()];
        let dir = std::env::temp_dir().join(format!("sf-pre-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("stubs")).unwrap();
        for (name, body) in stubs {
            let f = dir.join("stubs").join(name);
            std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let f = dir.join("h.sh");
        std::fs::write(&f, format!("{head}\n{tail}\n")).unwrap();
        let mut cmd = std::process::Command::new("sh");
        cmd.arg(&f)
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", dir.join("stubs").display()))
            .env("SYNOFURB_LOG", dir.join("log"))
            .env("SYNOFURB_LOCK", dir.join("lock"))
            .env("SYNOFURB_DRY", "0")
            .env("SYNOFURB_DOM_DIRS", dir.join("stubs"));
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.env("DIR", &dir);
        let out = cmd.output().unwrap();
        let log = std::fs::read_to_string(dir.join("log")).unwrap_or_default();
        let calls = std::fs::read_to_string(dir.join("calls")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        (out.status.code().unwrap_or(-1), format!("{log}{}", String::from_utf8_lossy(&out.stderr)), calls)
    }

    const DOM_STATUS: &str = r#"{"tool":"synofurb-dom","device":"/dev/sde","p1":{"dev":"/dev/sde1","bytes":50331648,"free_bytes":30000000,"fat_volume_id":"10EE-589C"},"p2":{"dev":"/dev/sde2","bytes":96468992,"free_bytes":FREE,"uuid":"45e5b07d"},"entries":[],"files":[{"name":"agent.vmlinuz","size":14329792,"sha256":"x","installed":"d"},{"name":"vmlinuz.old","size":OLDK,"sha256":"y","installed":"d"},{"name":"initrd.img.old","size":OLDI,"sha256":"z","installed":"d"}]}"#;

    /// A stand-in `synofurb-dom`: `status --json` prints the first of status.before/status.after that is not consumed; `remove` switches to "after".
    fn dom_stub(before: &str, after: &str) -> String {
        format!(
            "cd \"$DIR\"\necho \"synofurb-dom $*\" >> calls\ncase \"$1\" in\n status) if [ -e removed ]; then cat <<'EOF'\n{after}\nEOF\nelse cat <<'EOF'\n{before}\nEOF\nfi ;;\n remove) : > removed ;;\n *) ;;\nesac"
        )
    }

    #[test]
    fn the_flash_check_passes_when_debian_fits_and_touches_nothing() {
        let st = DOM_STATUS.replace("FREE", "40000000").replace("OLDK", "0").replace("OLDI", "0");
        let (code, log, calls) = preamble_run(
            "fit",
            "SFDOM=\"$DIR/stubs/synofurb-dom\"\ndom_flash_check 32\nemit DONE-OK",
            &[("synofurb-dom", &dom_stub(&st, &st))],
            &[],
        );
        assert_eq!(code, 0, "{log}");
        assert!(log.contains("38 MiB free") && log.contains("DONE-OK") && !log.contains("too full"), "{log}");
        assert_eq!(calls.lines().collect::<Vec<_>>(), ["synofurb-dom status --json"], "read-only: {calls}");
    }

    #[test]
    fn the_flash_check_counts_the_fallback_kernel_as_free_because_synofurb_dom_drops_it_first() {
        // 5 MB free + 13 MB + 14 MB of .old files = 32 MB: enough for 28 MiB
        let st = DOM_STATUS.replace("FREE", "5000000").replace("OLDK", "13000000").replace("OLDI", "14000000");
        let (code, log, calls) = preamble_run(
            "old",
            "SFDOM=\"$DIR/stubs/synofurb-dom\"\ndom_flash_check 28\nemit DONE-OK",
            &[("synofurb-dom", &dom_stub(&st, &st))],
            &[],
        );
        assert_eq!(code, 0, "{log}");
        assert!(!calls.contains("remove"), "{calls}");
    }

    #[test]
    fn a_full_flash_is_cleaned_of_the_agent_before_the_point_of_no_return_and_a_still_full_one_stops_the_install() {
        let full = DOM_STATUS.replace("FREE", "2000000").replace("OLDK", "0").replace("OLDI", "0");
        let roomy = DOM_STATUS.replace("FREE", "40000000").replace("OLDK", "0").replace("OLDI", "0");
        // the agent (21 MB) is the thing in the way: removing it makes room
        let (code, log, calls) = preamble_run(
            "clean",
            "SFDOM=\"$DIR/stubs/synofurb-dom\"\ndom_flash_check 32\nemit DONE-OK",
            &[("synofurb-dom", &dom_stub(&full, &roomy))],
            &[],
        );
        assert_eq!(code, 0, "{log}");
        assert!(
            calls.lines().any(|l| l == "synofurb-dom remove --yes") && log.contains("too full") && log.contains("DONE-OK"),
            "{log}\n{calls}"
        );
        // even a clean flash is too small: refuse; nothing was written, and the failure path (PONR=0) points the default at DSM again
        let (code, log, calls) = preamble_run(
            "tiny",
            "SFDOM=\"$DIR/stubs/synofurb-dom\"\nSYNOFURB_DRY=0\nDRY=1\ndom_flash_check 32\nemit NOT-REACHED",
            &[("synofurb-dom", &dom_stub(&full, &full))],
            &[],
        );
        assert_eq!(code, 1, "{log}\n{calls}");
        assert!(log.contains("@@FAIL preflight: the boot flash has only 1 MiB free") && !log.contains("NOT-REACHED"), "{log}");
        assert!(log.contains("DSM stays the default"), "{log}");
        // an unreadable flash is also a stop
        let (code, log, _) = preamble_run(
            "broken",
            "SFDOM=\"$DIR/stubs/synofurb-dom\"\nDRY=1\ndom_flash_check 32\nemit NOT-REACHED",
            &[("synofurb-dom", "exit 3")],
            &[],
        );
        assert_eq!(code, 1);
        assert!(log.contains("@@FAIL preflight: cannot read the boot flash"), "{log}");
    }

    #[test]
    fn the_rendered_script_checks_the_flash_and_releases_old_arrays_before_the_point_of_no_return() {
        let (s, _) = ds918_script(DataRaid::Shr1, Some(4));
        let ponr = s.find("emit \"@@POINT_OF_NO_RETURN\"").unwrap();
        let flash = s.find("dom_flash_check 32").unwrap();
        let release = s.find("release_storage ||").unwrap();
        assert!(flash < release && release < ponr, "flash check, then release, then the point of no return");
        assert!(s.find("expect_part").unwrap() < flash, "the layout is verified first");
        assert!(s.find("wget -q -O /dev/null").unwrap() < flash || s.find("Release").unwrap() < flash);
        // the zeroing and partitioning start only after the release
        assert!(release < s.find("mdadm --zero-superblock").unwrap());
        // arrays are stopped cleanly before the reboot, after the file systems are unmounted
        let stop = s.rfind("try mdadm --stop --scan").unwrap();
        assert!(s.rfind("try umount").unwrap_or(0) < stop && stop < s.find("@@DONE").unwrap());
        // no other firmware gets the flash check
        assert!(!render(StorageMode::WipeAll).0.contains("dom_flash_check 32"));
        assert_eq!(DOM_DEBIAN_NEED_MIB, 32, "the measured 28 MiB (Linux 7.2.11) plus headroom; update the doc comment when it changes");
    }

    #[test]
    fn release_storage_stops_old_arrays_until_none_is_left_and_fails_when_one_cannot_be_stopped() {
        // a DSM array that the installer assembled: every `mdadm --stop` of it removes it from the stand-in mdstat
        let mdadm = "cd \"$DIR\"\necho \"mdadm $*\" >> calls\ncase \"$1\" in\n --stop) shift; for a in \"$@\"; do case \"$a\" in --scan) : > mdstat ;; /dev/md*) : > mdstat ;; esac; done ;;\nesac";
        let setup = "printf 'Personalities : [raid1] [raid6] [raid5]\\nmd2 : active raid5 sda5[0] sdb5[1]\\n      11692677888 blocks\\nmd0 : active raid1 sda1[0]\\n' > \"$DIR/mdstat\"";
        let tail = format!("{setup}\nrelease_storage && emit RELEASED || emit STILL-ACTIVE");
        // the stand-in /proc/mdstat and /sys/block live in the temp dir; the helper passes DIR
        let tail = format!("SYNOFURB_MDSTAT=\"$DIR/mdstat\"\nSYNOFURB_SYSBLOCK=\"$DIR/sysblock\"\n{tail}");
        let (code, log, calls) =
            preamble_run("rel-ok", &tail, &[("mdadm", mdadm), ("vgchange", "cd \"$DIR\"; echo \"vgchange $*\" >> calls")], &[]);
        assert_eq!(code, 0, "{log}");
        assert!(log.contains("RELEASED") && !log.contains("STILL-ACTIVE"), "{log}");
        assert!(calls.contains("vgchange -an") && calls.contains("mdadm --stop --scan"), "VG first, then the arrays: {calls}");
        assert!(calls.find("vgchange").unwrap() < calls.find("mdadm").unwrap());
        // an array that refuses to stop (busy): five rounds, then the script says so
        let busy = "cd \"$DIR\"\necho \"mdadm $*\" >> calls\nexit 1";
        let (_, log, calls) = preamble_run("rel-busy", &tail, &[("mdadm", busy)], &[]);
        assert!(log.contains("STILL-ACTIVE"), "{log}");
        assert!(calls.matches("mdadm --stop --scan").count() >= 5, "retried: {calls}");
    }
}
