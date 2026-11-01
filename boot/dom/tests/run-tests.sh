#!/usr/bin/env bash
# Automated tests for synofurb-dom.
#
# Builds synthetic 128 MiB DOM images (MBR or GPT; FAT with volume id 10EE-589C;
# ext2 with the Synology UUID holding zImage + rd.gz of random data), points the
# tool at them through SYNOFURB_DOM / SYNOFURB_DOM1 / SYNOFURB_DOM2 and exercises
# every command.  Nothing here touches a real device.
#
# Usage:   tests/run-tests.sh                  (runs under dash)
#          TEST_SH="busybox ash" tests/run-tests.sh   (shell under test)
#          TEST_BB=1 ...   additionally put busybox applets (awk, sed, cut, ...)
#                          first in PATH so busybox userland is exercised too
#          KEEP=1          keep the work directory
# Needs root, loop devices, e2fsprogs, dosfstools, mtools, util-linux, sfdisk.

set -u
HERE=$(cd "$(dirname "$0")" && pwd)
SCRIPT=${SYNOFURB_DOM_SCRIPT:-$HERE/../synofurb-dom}
SH=${TEST_SH:-dash}
BASE_TMP=${TMPDIR:-/tmp}

if [ "$(id -u)" != 0 ]; then echo "run as root" >&2; exit 2; fi
for t in mkfs.vfat mkfs.ext2 mcopy mmd sfdisk debugfs e2fsck fsck.vfat losetup mount xxd; do
  command -v "$t" >/dev/null 2>&1 || { echo "missing tool: $t" >&2; exit 2; }
done

T=$(mktemp -d "$BASE_TMP/synofurb-tests.XXXXXX")
BASES=$T/bases
mkdir -p "$BASES" "$T/dsm"
export MTOOLS_SKIP_CHECK=1

cleanup() {
  for m in $(awk -v t="$T" 'index($2, t) == 1 { print $2 }' /proc/mounts | sort -r); do umount "$m" 2>/dev/null; done
  for l in $(losetup -a 2>/dev/null | grep "$T" | cut -d: -f1); do losetup -d "$l" 2>/dev/null; done
  rm -f /dev/synoboot 2>/dev/null
  [ "${KEEP:-0}" = 1 ] || rm -rf "$T"
}
trap cleanup EXIT

if [ "${TEST_BB:-0}" = 1 ]; then
  mkdir -p "$T/bbbin"
  for a in awk cut tr od sed grep head tail wc df mktemp sha256sum dd cmp readlink date sort uniq cat cp mv rm mkdir rmdir ls id sync sleep touch chmod; do
    ln -sf "$(command -v busybox)" "$T/bbbin/$a"
  done
  # only the tool under test sees the busybox applets, not this harness
  SH="env PATH=$T/bbbin:$PATH $SH"
  echo "tool runs with busybox applets first in PATH ($T/bbbin)"
fi

PASS=0
FAIL=0
FAILED=()
ok() { PASS=$((PASS + 1)); printf '  ok    %s\n' "$1"; }
bad() {
  FAIL=$((FAIL + 1)); FAILED+=("$CURRENT: $1"); printf '  FAIL  %s\n' "$1"
  if [ -s "$OUT" ]; then printf '        last tool output (rc=%s):\n' "$RC"; tail -n 6 "$OUT" | sed 's/^/        | /'; fi
}
assert() { local d=$1; shift; if "$@"; then ok "$d"; else bad "$d"; fi; }
CURRENT=
begin() { CURRENT=$1; printf '\n== %s\n' "$1"; }

eq() { [ "$1" = "$2" ]; }
not() { ! "$@"; }
ne() { [ "$1" != "$2" ]; }
rc_is() { [ "$RC" -eq "$1" ]; }
rc_nz() { [ "$RC" -ne 0 ]; }
out_has() { grep -qE -- "$1" "$OUT"; }
out_lacks() { ! grep -qE -- "$1" "$OUT"; }
sha() { sha256sum "$1" | cut -d' ' -f1; }

# ------------------------------------------------------------ image building

dd_rand() { head -c "$2" /dev/urandom > "$1"; }
dd_rand "$T/dsm/zImage" $((6 * 1048576))
dd_rand "$T/dsm/rd.gz" $((45 * 1048576))
DSM_SMALL=$T/dsm-small
mkdir -p "$DSM_SMALL"
dd_rand "$DSM_SMALL/zImage" $((6 * 1048576))
dd_rand "$DSM_SMALL/rd.gz" $((20 * 1048576))
dd_rand "$T/dsm/boot.efi" 200000

gen_conf() { # variant -> stdout
  local u=45e5b07d-4783-4867-a369-f99c0cd1e610
  case $1 in
    default)
      cat <<EOF
set default='1'
set timeout='3'
set fallback='0'

set menu_color_normal=white/black
set menu_color_highlight=black/white

menuentry "SYNOLOGY_1" {
	search --fs-uuid --no-floppy --set=root $u
	linux /zImage root=/dev/md0 netif_num=4 syno_hw_version=DS3617xs recovery=1 earlyprintk
	initrd /rd.gz
}
menuentry "SYNOLOGY_2" {
	search --fs-uuid --no-floppy --set=root $u
	linux /zImage root=/dev/md0 netif_num=4 syno_hw_version=DS3617xs earlyprintk
	initrd /rd.gz
}
EOF
      ;;
    dsmfirst)
      cat <<EOF
set default='0'
set timeout='3'

menuentry 'SYNOLOGY_2' {
	search --fs-uuid --no-floppy --set=root $u
	linux /zImage root=/dev/md0
	initrd /rd.gz
}
menuentry 'SYNOLOGY_1' {
	search --fs-uuid --no-floppy --set=root $u
	linux /zImage root=/dev/md0 recovery=1
	initrd /rd.gz
}
EOF
      ;;
    three)
      cat <<EOF
# three entries, DSM last, double-quoted default
set timeout=3
set default="2"

menuentry "SYNOLOGY_1" {
	linux /zImage a=1
	initrd /rd.gz
}
menuentry "SYNOLOGY_3" {
	linux /zImage a=3
	initrd /rd.gz
}
menuentry "SYNOLOGY_2" {
	linux /zImage a=2
	initrd /rd.gz
}
EOF
      ;;
    nodefault)
      sed '/^set default/d' < <(gen_conf default)
      ;;
  esac
}

# mkbase NAME TABLE(dos|gpt) P1_MiB P2_MiB FATBITS DSMDIR [CONFVARIANT]
mkbase() {
  local name=$1 table=$2 p1=$3 p2=$4 fatbits=$5 dsm=$6 variant=${7:-default}
  local img=$BASES/$name.img w=$BASES/$name.w
  mkdir -p "$w/rootfs"
  truncate -s 128M "$img"
  local p1s=2048 p1n=$((p1 * 2048)) p2n=$((p2 * 2048))
  local p2s=$((p1s + p1n))
  if [ "$table" = dos ]; then
    printf 'label: dos\nstart=%s, size=%s, type=c\nstart=%s, size=%s, type=83\n' "$p1s" "$p1n" "$p2s" "$p2n" | sfdisk -q "$img" >/dev/null
  else
    printf 'label: gpt\nstart=%s, size=%s, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7\nstart=%s, size=%s, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4\n' "$p1s" "$p1n" "$p2s" "$p2n" | sfdisk -q "$img" >/dev/null
  fi
  truncate -s "${p1}M" "$w/p1.img"
  if [ "$fatbits" = 32 ]; then
    mkfs.vfat -F 32 -s 1 -i 10EE589C "$w/p1.img" >/dev/null
  else
    mkfs.vfat -F 16 -i 10EE589C "$w/p1.img" >/dev/null
  fi
  gen_conf "$variant" > "$w/conf"
  mmd -i "$w/p1.img" ::EFI ::EFI/boot
  mcopy -i "$w/p1.img" "$w/conf" ::EFI/boot/SynoBootLoader.conf
  mcopy -i "$w/p1.img" "$T/dsm/boot.efi" ::EFI/boot/bootx64.efi
  cp "$dsm/zImage" "$dsm/rd.gz" "$w/rootfs/"
  truncate -s "${p2}M" "$w/p2.img"
  mkfs.ext2 -q -F -U 45e5b07d-4783-4867-a369-f99c0cd1e610 -d "$w/rootfs" "$w/p2.img"
  dd if="$w/p1.img" of="$img" bs=512 seek=$p1s conv=notrunc 2>/dev/null
  dd if="$w/p2.img" of="$img" bs=512 seek=$p2s conv=notrunc 2>/dev/null
  printf 'P1S=%s\nP1N=%s\nP2S=%s\nP2N=%s\n' "$p1s" "$p1n" "$p2s" "$p2n" > "$BASES/$name.layout"
  rm -rf "$w"
  fallocate -d "$img" 2>/dev/null || true
}

# Images are built on first use (and the rarely used ones are dropped again)
# to keep the disk footprint small: name -> "table P1MiB P2MiB fatbits dsmdir [conf variant]"
declare -A BASESPEC=(
  [mbr]="dos 20 104 16 $T/dsm"
  [gpt]="gpt 20 104 16 $T/dsm"
  [fat32]="dos 40 80 32 $T/dsm"
  [small]="dos 20 64 16 $T/dsm"
  [dsmfirst]="dos 20 80 16 $DSM_SMALL dsmfirst"
  [three]="dos 20 80 16 $DSM_SMALL three"
  [nodefault]="dos 20 80 16 $DSM_SMALL nodefault"
)
ensure_base() {
  if [ ! -f "$BASES/$1.img" ]; then
    # shellcheck disable=SC2086
    mkbase "$1" ${BASESPEC[$1]}
  fi
}
drop_base() { rm -f "$BASES/$1.img" "$BASES/$1.layout"; }
free_mb=$(df -Pm "$BASE_TMP" | awk 'NR == 2 { print $4 }')
if [ "$free_mb" -lt 1000 ]; then echo "warning: only ${free_mb} MiB free in $BASE_TMP; the tests need about 1000 MiB" >&2; fi

IMG=
P1S=0 P1N=0 P2S=0 P2N=0
STATE=$T/state
TMPD=$T/tmp
mkdir -p "$STATE" "$TMPD"

fresh() { # kind  -> $IMG
  local kind=$1
  ensure_base "$kind"
  # shellcheck disable=SC1090
  . "$BASES/$kind.layout"
  IMG=$T/work-$kind.img
  cp --sparse=always "$BASES/$kind.img" "$IMG"
  rm -rf "$STATE" "$TMPD"
  mkdir -p "$STATE" "$TMPD"
  ORIG_SHA=$(sha "$IMG")
  cp --sparse=always "$IMG" "$T/orig.img"
}

OUT=$T/out
RC=0
run() { # args... ; env: DOMVAR (default SYNOFURB_DOM=$IMG)
  env "SYNOFURB_DOM=${DOM_OVERRIDE:-$IMG}" SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" \
    ${EXTRA_ENV:-} $SH "$SCRIPT" "$@" > "$OUT" 2>&1
  RC=$?
}
run_stdout() { # like run but only stdout is captured (for --json)
  env "SYNOFURB_DOM=${DOM_OVERRIDE:-$IMG}" SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" \
    $SH "$SCRIPT" "$@" > "$OUT" 2>/dev/null
  RC=$?
}
runenv() { # VAR=val... -- args...
  local envs=()
  while [ "$1" != -- ]; do envs+=("$1"); shift; done
  shift
  env "SYNOFURB_DOM=${DOM_OVERRIDE:-$IMG}" SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" \
    "${envs[@]}" $SH "$SCRIPT" "$@" > "$OUT" 2>&1
  RC=$?
}

conf_of() { mcopy -i "$1@@$((P1S * 512))" ::EFI/boot/SynoBootLoader.conf - 2>/dev/null; }
set_conf() { mcopy -n -o -i "$1@@$((P1S * 512))" "$2" ::EFI/boot/SynoBootLoader.conf >/dev/null 2>&1; }
p2_extract() { dd if="$1" of="$T/p2.cur" bs=512 skip="$P2S" count="$P2N" 2>/dev/null; }
p1_extract() { dd if="$1" of="$T/p1.cur" bs=512 skip="$P1S" count="$P1N" 2>/dev/null; }
p2_cat() { p2_extract "$1"; debugfs -R "cat $2" "$T/p2.cur" 2>/dev/null; }
p2_has() { p2_extract "$1"; debugfs -R "stat $2" "$T/p2.cur" 2>&1 | grep -q 'Inode:'; }
p2_sha() { p2_cat "$1" "$2" | sha256sum | cut -d' ' -f1; }
p1_has() { mdir -i "$1@@$((P1S * 512))" "::$2" >/dev/null 2>&1; }
dsm_sums() { echo "$(p2_sha "$1" /zImage) $(p2_sha "$1" /rd.gz)"; }
fscks_clean() { # image
  p1_extract "$1"; p2_extract "$1"
  e2fsck -fn "$T/p2.cur" >/dev/null 2>&1 && fsck.vfat -n "$T/p1.cur" >/dev/null 2>&1
}
default_value() { sed -n "s/^set default=['\"]\{0,1\}\([^'\"]*\).*/\1/p" | head -n 1 | tr -d '\r'; }
# independent re-implementation: index (0-based) of "menuentry NAME"
entry_idx() { # conf-file name
  awk -v n="$2" -v q="'" '/^[ \t]*menuentry/ { s = $0; gsub(/\r/, "", s); if (s ~ ("[\"" q "]" n "[\"" q "]")) { print i+0; exit } i++ }' "$1"
}
n_entries() { grep -c '^[[:space:]]*menuentry' "$1"; }
outside() { sed '/^### synofurb begin/,/^### synofurb end/d' | sed "/^set default=/d"; }
tmpleft() { find "$TMPD" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l; }
mkfile() { head -c "$2" /dev/urandom > "$1"; }

# ---------------------------------------------------------------------- tests

t_static() {
  begin "static checks"
  if command -v shellcheck >/dev/null 2>&1; then
    assert "shellcheck -s dash is clean" shellcheck -s dash "$SCRIPT"
  else
    echo "  skip  shellcheck not installed"
  fi
  assert "dash -n parses" dash -n "$SCRIPT"
  if command -v busybox >/dev/null 2>&1; then
    assert "busybox ash -n parses" busybox ash -n "$SCRIPT"
  fi
  assert "no obvious bashisms (grep for double-bracket, here-string, local, pattern substitution, echo -n)" bash -c '! grep -nE "\[\[|<<<|^\s*local |\$\{[a-zA-Z_]+//|echo -n" "$1"' _ "$SCRIPT"
  assert "version flag works" bash -c "$SH \"$SCRIPT\" --version | grep -q synofurb-dom"
  run bogus
  assert "unknown command fails" rc_nz
  run
  assert "no command prints usage and fails" rc_nz
}

t_status() {
  begin "status (MBR image)"
  fresh mbr
  run status
  assert "status exits 0" rc_is 0
  assert "status lists SYNOLOGY_1" out_has 'SYNOLOGY_1'
  assert "status lists SYNOLOGY_2 as DSM and default" out_has '1  SYNOLOGY_2 \[DSM\].*default'
  assert "status shows default name+index" out_has 'Default: +SYNOLOGY_2 \(index 1\)'
  assert "status shows sizes of both partitions" out_has 'Partition 1:.*Partition 2:|Partition 2:'
  assert "status reports no backup yet" out_has 'none yet'
  assert "status does not modify the image" eq "$(sha "$IMG")" "$ORIG_SHA"
  assert "no temporary files left" eq "$(tmpleft)" 0
  run_stdout status --json
  assert "status --json exits 0" rc_is 0
  assert "status --json is valid JSON" python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$OUT"
  assert "json default name" bash -c "python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if d[\"default\"][\"name\"]==\"SYNOLOGY_2\" and d[\"default\"][\"index\"]==1 else 1)' $OUT"
  assert "json lists 2 entries" bash -c "python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if len(d[\"entries\"])==2 else 1)' $OUT"
  run verify
  assert "verify passes on a pristine image" rc_is 0
  assert "verify says all checks passed" out_has 'all checks passed'
}

t_refusals() {
  begin "refusal on non-matching images"
  fresh mbr
  # a) all zero
  head -c 134217728 /dev/zero > "$IMG"; ORIG_SHA=$(sha "$IMG")
  run install-agent "$T/dsm/zImage" "$T/dsm/boot.efi" --yes
  assert "all-zero image: refused" rc_nz
  assert "all-zero image: unchanged" eq "$(sha "$IMG")" "$ORIG_SHA"
  run status
  assert "all-zero image: status refused too" rc_nz
  # b) wrong FAT volume id
  fresh mbr
  printf '\x00\x00\x00\x00' | dd of="$IMG" bs=1 seek=$((P1S * 512 + 39)) conv=notrunc 2>/dev/null; ORIG_SHA=$(sha "$IMG")
  run install-agent "$T/dsm/zImage" "$T/dsm/boot.efi" --yes
  assert "wrong FAT volume id: refused" rc_nz
  assert "wrong FAT volume id: message names the FAT id" out_has 'FAT volume id'
  assert "wrong FAT volume id: unchanged" eq "$(sha "$IMG")" "$ORIG_SHA"
  # c) wrong ext UUID
  fresh mbr
  printf '\x00' | dd of="$IMG" bs=1 seek=$((P2S * 512 + 1128)) conv=notrunc 2>/dev/null; ORIG_SHA=$(sha "$IMG")
  run default dsm --yes
  assert "wrong ext UUID: refused" rc_nz
  assert "wrong ext UUID: message names the UUID" out_has 'UUID'
  assert "wrong ext UUID: unchanged" eq "$(sha "$IMG")" "$ORIG_SHA"
  # d) wrong ext magic
  fresh mbr
  printf '\x00\x00' | dd of="$IMG" bs=1 seek=$((P2S * 512 + 1080)) conv=notrunc 2>/dev/null; ORIG_SHA=$(sha "$IMG")
  run remove --yes
  assert "no ext magic: refused" rc_nz
  assert "no ext magic: unchanged" eq "$(sha "$IMG")" "$ORIG_SHA"
  # e) no partition table
  fresh mbr
  head -c 512 /dev/zero | dd of="$IMG" bs=512 count=1 conv=notrunc 2>/dev/null; ORIG_SHA=$(sha "$IMG")
  run status
  assert "no partition table: refused" rc_nz
  assert "no partition table: message" out_has 'partition table'
  assert "no partition table: unchanged" eq "$(sha "$IMG")" "$ORIG_SHA"
  # f) P1 smaller than 16 MiB, g) P2 smaller than 64 MiB
  mkbase tinyp1 dos 10 90 16 "$DSM_SMALL"
  fresh tinyp1
  run status
  assert "P1 below 16 MiB: refused" rc_nz
  assert "P1 below 16 MiB: message" out_has 'minimum'
  mkbase tinyp2 dos 20 60 16 "$DSM_SMALL"
  fresh tinyp2
  run install-agent "$T/dsm/zImage" "$T/dsm/boot.efi" --yes
  assert "P2 below 64 MiB: refused" rc_nz
  assert "P2 below 64 MiB: unchanged" eq "$(sha "$IMG")" "$ORIG_SHA"
  # h) pointing at a nonexistent file / partition overrides that do not match
  fresh mbr
  DOM_OVERRIDE=$T/does-not-exist run status
  assert "nonexistent device: refused" rc_nz
  # i) swapped partitions (ext2 passed as P1)
  fresh mbr
  p1_extract "$IMG"; p2_extract "$IMG"
  env SYNOFURB_DOM1="$T/p2.cur" SYNOFURB_DOM2="$T/p1.cur" SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" $SH "$SCRIPT" status > "$OUT" 2>&1
  RC=$?
  assert "swapped partitions: refused" rc_nz
  rm -f "$BASES/tinyp1".* "$BASES/tinyp2".*
}

t_backup_restore() {
  begin "backup / restore round trip"
  fresh mbr
  local bdir=$T/bk
  run backup "$bdir"
  assert "backup exits 0" rc_is 0
  local bf
  bf=$(ls "$bdir"/synofurb-dom-backup-*.img 2>/dev/null | head -n 1)
  assert "backup file exists with the documented name" test -f "$bf"
  assert "backup name matches YYYYMMDD-HHMMSS" bash -c "[[ \"$bf\" =~ synofurb-dom-backup-[0-9]{8}-[0-9]{6}\\.img$ ]]"
  assert "backup is mode 600" eq "$(stat -c %a "$bf")" 600
  assert "backup is bit-identical to the device" eq "$(sha "$bf")" "$ORIG_SHA"
  assert "sha256 sidecar written and valid" bash -c "cd $bdir && sha256sum -c ${bf##*/}.sha256 >/dev/null"
  assert "backup prints its path" out_has "$bdir/synofurb-dom-backup-"
  # modify the image, then restore
  run install-debian "$T/dsm/boot.efi" "$T/dsm/boot.efi" --cmdline "root=/dev/sda1 ro" --yes
  assert "install-debian (to have something to restore from) works" rc_is 0
  assert "image is now different" ne "$(sha "$IMG")" "$ORIG_SHA"
  run restore "$bf"
  assert "restore without --yes is refused" rc_nz
  run restore "$bf" --yes
  assert "restore --yes exits 0" rc_is 0
  assert "restore round trip is bit-identical" eq "$(sha "$IMG")" "$ORIG_SHA"
  # validation
  head -c 1000000 /dev/zero > "$T/small.img"
  run restore "$T/small.img" --yes
  assert "restore of a wrong-size file is refused" rc_nz
  head -c 134217728 /dev/zero > "$T/zero.img"
  run restore "$T/zero.img" --yes
  assert "restore of a non-DOM image of the right size is refused" rc_nz
  assert "device untouched after refused restores" eq "$(sha "$IMG")" "$ORIG_SHA"
  cp "$bf" "$T/bad.img"; cp "$bf.sha256" "$T/bad.img.sha256"
  printf 'X' | dd of="$T/bad.img" bs=1 seek=$((P2S * 512 + 5000000)) conv=notrunc 2>/dev/null
  run restore "$T/bad.img" --yes
  assert "restore with a failing sha256 sidecar is refused" rc_nz
  assert "device still untouched" eq "$(sha "$IMG")" "$ORIG_SHA"
  # partition-table-less image with right size and FAT/ext magics is refused
  cp "$bf" "$T/notable.img"
  head -c 512 /dev/zero | dd of="$T/notable.img" bs=512 count=1 conv=notrunc 2>/dev/null
  run restore "$T/notable.img" --yes
  assert "restore of an image without partition table is refused" rc_nz
  # restore to a damaged device is allowed when the device is named explicitly
  head -c 4194304 /dev/zero | dd of="$IMG" bs=1M count=4 conv=notrunc 2>/dev/null
  run restore "$bf" --yes
  assert "restore onto a corrupted device (explicit SYNOFURB_DOM) works" rc_is 0
  assert "...and is bit-identical again" eq "$(sha "$IMG")" "$ORIG_SHA"
  rm -rf "$bdir"
}

t_install_agent() {
  begin "install-agent: files, entry, idempotency, DSM untouched"
  fresh mbr
  local before_dsm; before_dsm=$(dsm_sums "$IMG")
  mkfile "$T/agent.k" 3000000; mkfile "$T/agent.i" 2000000
  run install-agent "$T/agent.k" "$T/agent.i" --cmdline "console=ttyS0,115200n8 quiet" --yes
  assert "install-agent exits 0" rc_is 0
  assert "agent kernel is on P2 and byte-identical" eq "$(p2_sha "$IMG" /synofurb/agent.vmlinuz)" "$(sha "$T/agent.k")"
  assert "agent initrd is on P2 and byte-identical" eq "$(p2_sha "$IMG" /synofurb/agent.initrd)" "$(sha "$T/agent.i")"
  conf_of "$IMG" > "$T/c1"
  assert "marker block begin present once" eq "$(grep -c '^### synofurb begin (managed by synofurb-dom)$' "$T/c1")" 1
  assert "marker block end present once" eq "$(grep -c '^### synofurb end$' "$T/c1")" 1
  assert "SYNOFURB_BOOT entry present once" eq "$(grep -c 'menuentry "SYNOFURB_BOOT"' "$T/c1")" 1
  assert "entry uses the ext2 UUID search line" grep -q 'search --fs-uuid --no-floppy --set=root 45e5b07d-4783-4867-a369-f99c0cd1e610' "$T/c1"
  assert "entry has linux line with cmdline" grep -qP '^\tlinux /synofurb/agent.vmlinuz console=ttyS0,115200n8 quiet$' "$T/c1"
  assert "entry has initrd line" grep -qP '^\tinitrd /synofurb/agent.initrd$' "$T/c1"
  assert "default is unchanged (still index 1)" eq "$(default_value < "$T/c1")" 1
  conf_of "$T/orig.img" > "$T/c0"
  assert "conf outside the block is byte-identical" eq "$(outside < "$T/c1" | sha256sum)" "$(outside < "$T/c0" | sha256sum)"
  assert "conf outside block: first 'set default' line untouched too" grep -q "^set default='1'$" "$T/c1"
  assert "DSM files zImage/rd.gz untouched" eq "$(dsm_sums "$IMG")" "$before_dsm"
  assert "both filesystems fsck clean" fscks_clean "$IMG"
  assert "automatic first backup was created" test -f "$STATE/synofurb-dom-first-backup.img"
  assert "automatic backup equals the original device" eq "$(sha "$STATE/synofurb-dom-first-backup.img")" "$ORIG_SHA"
  assert "no temporary files left" eq "$(tmpleft)" 0
  run status
  assert "status shows the agent files" out_has 'agent.vmlinuz'
  assert "status marks the entry as ours" out_has 'SYNOFURB_BOOT \[synofurb\]'
  assert "status shows the backup" out_has 'first backup: .*first-backup'
  # idempotency
  local s1; s1=$(sha "$IMG")
  run install-agent "$T/agent.k" "$T/agent.i" --cmdline "console=ttyS0,115200n8 quiet" --yes
  assert "re-run exits 0" rc_is 0
  assert "re-run says nothing to change" out_has 'Nothing to change'
  assert "re-run leaves the device bit-identical" eq "$(sha "$IMG")" "$s1"
  run install-agent "$T/agent.k" "$T/agent.i" --yes
  assert "re-run without --cmdline keeps the cmdline (no change)" eq "$(sha "$IMG")" "$s1"
  # replacing the agent changes files in place, no duplicate entry
  mkfile "$T/agent2.k" 1500000
  run install-agent "$T/agent2.k" "$T/agent.i" --cmdline "console=tty0" --yes
  assert "replacing the agent kernel works" rc_is 0
  conf_of "$IMG" > "$T/c2"
  assert "still exactly one SYNOFURB_BOOT entry" eq "$(grep -c 'menuentry "SYNOFURB_BOOT"' "$T/c2")" 1
  assert "cmdline replaced" grep -qP '^\tlinux /synofurb/agent.vmlinuz console=tty0$' "$T/c2"
  assert "new kernel content in place" eq "$(p2_sha "$IMG" /synofurb/agent.vmlinuz)" "$(sha "$T/agent2.k")"
  assert "DSM files still untouched" eq "$(dsm_sums "$IMG")" "$before_dsm"
  # dry run
  s1=$(sha "$IMG")
  run install-agent "$T/agent.k" "$T/agent.i" --dry-run --yes
  assert "dry-run exits 0" rc_is 0
  assert "dry-run does not write" eq "$(sha "$IMG")" "$s1"
  assert "dry-run says so" out_has 'Dry run'
  # input validation
  run install-agent "$T/nonexistent" "$T/agent.i" --yes
  assert "missing kernel file is refused" rc_nz
  run install-agent "$T/agent.k" "$T/agent.i" --cmdline "a;b" --yes
  assert "cmdline with ';' is refused" rc_nz
  run install-agent "$T/agent.k" "$T/agent.i" --cmdline "$(printf 'a\nb')" --yes
  assert "cmdline with newline is refused" rc_nz
  run install-agent "$T/agent.k" "$T/agent.i" --cmdline "a{b" --yes
  assert "cmdline with '{' is refused" rc_nz
  assert "device unchanged after rejected input" eq "$(sha "$IMG")" "$s1"
}

t_arm_default() {
  begin "--arm and default index math"
  fresh mbr
  mkfile "$T/agent.k" 1000000; mkfile "$T/agent.i" 1000000
  run install-agent "$T/agent.k" "$T/agent.i" --arm --yes
  assert "install-agent --arm exits 0" rc_is 0
  conf_of "$IMG" > "$T/c1"
  assert "default now equals the agent entry index (2)" eq "$(default_value < "$T/c1")" "$(entry_idx "$T/c1" SYNOFURB_BOOT)"
  assert "agent entry index is 2" eq "$(entry_idx "$T/c1" SYNOFURB_BOOT)" 2
  assert "DSM entry still present at index 1" eq "$(entry_idx "$T/c1" SYNOLOGY_2)" 1
  assert "default line otherwise intact (quotes kept)" grep -q "^set default='2'$" "$T/c1"
  run status
  assert "status shows the armed default" out_has 'Default: +SYNOFURB_BOOT \(index 2\)'
  run default dsm --yes
  assert "default dsm exits 0" rc_is 0
  conf_of "$IMG" > "$T/c2"
  assert "default is back at the DSM index" eq "$(default_value < "$T/c2")" 1
  assert "block untouched by default change" eq "$(sed -n '/^### synofurb begin/,/^### synofurb end/p' "$T/c1" | sha256sum)" "$(sed -n '/^### synofurb begin/,/^### synofurb end/p' "$T/c2" | sha256sum)"
  local s1; s1=$(sha "$IMG")
  run default dsm --yes
  assert "default dsm again: nothing to change" out_has 'Nothing to change'
  assert "...and the device is untouched" eq "$(sha "$IMG")" "$s1"
  run default debian --yes
  assert "default debian without a DEBIAN entry is refused" rc_nz
  run default debian-old --yes
  assert "default debian-old without entry is refused" rc_nz
  run default efi:nothing --yes
  assert "default efi:nothing without entry is refused" rc_nz
  run default bogus --yes
  assert "default with unknown target is refused" rc_nz
  run default agent --yes
  assert "default agent exits 0" rc_is 0
  conf_of "$IMG" > "$T/c3"
  assert "default agent -> index 2" eq "$(default_value < "$T/c3")" 2
  assert "fsck clean" fscks_clean "$IMG"

  # DSM first in the file (index 0), single-quoted names, default '0'
  fresh dsmfirst
  run install-agent "$T/agent.k" "$T/agent.i" --yes
  run default agent --yes
  conf_of "$IMG" > "$T/c4"
  assert "dsm-first: default -> agent index" eq "$(default_value < "$T/c4")" "$(entry_idx "$T/c4" SYNOFURB_BOOT)"
  run default dsm --yes
  conf_of "$IMG" > "$T/c5"
  assert "dsm-first: default dsm -> 0" eq "$(default_value < "$T/c5")" 0
  assert "dsm-first: index of SYNOLOGY_2 is 0" eq "$(entry_idx "$T/c5" SYNOLOGY_2)" 0

  # three entries, DSM last, default="2" with double quotes, "set timeout=3" without quotes
  fresh three
  run install-agent "$T/agent.k" "$T/agent.i" --arm --yes
  conf_of "$IMG" > "$T/c6"
  assert "three: DSM index is 2" eq "$(entry_idx "$T/c6" SYNOLOGY_2)" 2
  assert "three: agent index is 3" eq "$(entry_idx "$T/c6" SYNOFURB_BOOT)" 3
  assert "three: default written with the original double quotes" grep -q '^set default="3"$' "$T/c6"
  run default dsm --yes
  conf_of "$IMG" > "$T/c7"
  assert "three: default dsm -> 2 (double quotes kept)" grep -q '^set default="2"$' "$T/c7"
  assert "three: comment line and timeout untouched" grep -q '^set timeout=3$' "$T/c7"

  # no 'set default' line at all: boot loader default is entry 0
  fresh nodefault
  run install-agent "$T/agent.k" "$T/agent.i" --yes
  conf_of "$IMG" > "$T/c8"
  assert "nodefault: install does not invent a default line" not grep -q '^set default' "$T/c8"
  run default agent --yes
  conf_of "$IMG" > "$T/c9"
  assert "nodefault: default agent inserts a default line with the right index" eq "$(default_value < "$T/c9")" "$(entry_idx "$T/c9" SYNOFURB_BOOT)"
  assert "nodefault: rest of the conf identical" eq "$(sed '/^set default/d' "$T/c9" | outside | sha256sum)" "$(outside < "$T/c8" | sha256sum)"
  drop_base dsmfirst; drop_base three; drop_base nodefault
}

t_debian() {
  begin "install-debian: rotation, fallback entry, idempotency"
  fresh mbr
  local before_dsm; before_dsm=$(dsm_sums "$IMG")
  mkfile "$T/k1" 2500000; mkfile "$T/i1" 4000000
  mkfile "$T/k2" 2600000; mkfile "$T/i2" 4100000
  mkfile "$T/k3" 2700000; mkfile "$T/i3" 4200000
  run install-debian "$T/k1" "$T/i1" --yes
  assert "install-debian without --cmdline is refused" rc_nz
  run install-debian "$T/k1" "$T/i1" --cmdline "root=/dev/sda2 ro quiet" --yes
  assert "first install exits 0" rc_is 0
  assert "kernel installed" eq "$(p2_sha "$IMG" /synofurb/vmlinuz)" "$(sha "$T/k1")"
  assert "initrd installed" eq "$(p2_sha "$IMG" /synofurb/initrd.img)" "$(sha "$T/i1")"
  assert "no .old yet" not p2_has "$IMG" /synofurb/vmlinuz.old
  conf_of "$IMG" > "$T/c1"
  assert "DEBIAN entry present" grep -q 'menuentry "DEBIAN"' "$T/c1"
  assert "no DEBIAN_OLD entry yet" not grep -q 'DEBIAN_OLD' "$T/c1"
  assert "DEBIAN linux line carries the cmdline" grep -qP '^\tlinux /synofurb/vmlinuz root=/dev/sda2 ro quiet$' "$T/c1"
  assert "default untouched by install-debian" eq "$(default_value < "$T/c1")" 1
  local s1; s1=$(sha "$IMG")
  run install-debian "$T/k1" "$T/i1" --cmdline "root=/dev/sda2 ro quiet" --yes
  assert "identical re-install is a no-op" out_has 'Nothing to change'
  assert "...device bit-identical" eq "$(sha "$IMG")" "$s1"
  # second install rotates
  run install-debian "$T/k2" "$T/i2" --cmdline "root=/dev/sda2 ro" --yes
  assert "second install exits 0" rc_is 0
  assert "new kernel is current" eq "$(p2_sha "$IMG" /synofurb/vmlinuz)" "$(sha "$T/k2")"
  assert "previous kernel kept as vmlinuz.old" eq "$(p2_sha "$IMG" /synofurb/vmlinuz.old)" "$(sha "$T/k1")"
  assert "previous initrd kept as initrd.img.old" eq "$(p2_sha "$IMG" /synofurb/initrd.img.old)" "$(sha "$T/i1")"
  conf_of "$IMG" > "$T/c2"
  assert "DEBIAN_OLD entry exists exactly once" eq "$(grep -c 'menuentry "DEBIAN_OLD"' "$T/c2")" 1
  assert "DEBIAN_OLD uses the OLD files and the OLD cmdline" grep -qP '^\tlinux /synofurb/vmlinuz.old root=/dev/sda2 ro quiet$' "$T/c2"
  assert "DEBIAN uses the NEW cmdline" grep -qP '^\tlinux /synofurb/vmlinuz root=/dev/sda2 ro$' "$T/c2"
  assert "DEBIAN_OLD initrd line" grep -qP '^\tinitrd /synofurb/initrd.img.old$' "$T/c2"
  # third install: .old is replaced
  run install-debian "$T/k3" "$T/i3" --cmdline "root=/dev/sda2 ro" --yes
  assert "third install exits 0" rc_is 0
  assert "old is now the second kernel" eq "$(p2_sha "$IMG" /synofurb/vmlinuz.old)" "$(sha "$T/k2")"
  assert "DSM files untouched through three installs" eq "$(dsm_sums "$IMG")" "$before_dsm"
  conf_of "$IMG" > "$T/c3"
  conf_of "$T/orig.img" > "$T/c0"
  assert "conf outside block identical after three installs" eq "$(outside < "$T/c3" | sha256sum)" "$(outside < "$T/c0" | sha256sum)"
  assert "exactly one block" eq "$(grep -c '^### synofurb begin' "$T/c3")" 1
  assert "exactly 3 menuentry lines inside the block region or fewer than 6 total" eq "$(n_entries "$T/c3")" 4
  assert "fsck clean" fscks_clean "$IMG"
  run default debian-old --yes
  assert "default debian-old exits 0" rc_is 0
  conf_of "$IMG" > "$T/c4"
  assert "default index == index of DEBIAN_OLD" eq "$(default_value < "$T/c4")" "$(entry_idx "$T/c4" DEBIAN_OLD)"
  run default debian --yes
  conf_of "$IMG" > "$T/c5"
  assert "default index == index of DEBIAN" eq "$(default_value < "$T/c5")" "$(entry_idx "$T/c5" DEBIAN)"
  run verify
  assert "verify passes" rc_is 0
  run_stdout status --json
  assert "status json lists 4 files incl. manifest data" bash -c "python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if len(d[\"files\"])==4 else 1)' $OUT"
}

t_space() {
  begin "free-space check with rotation"
  fresh small
  local before_dsm; before_dsm=$(dsm_sums "$IMG")
  mkfile "$T/ka" 3000000; mkfile "$T/ia" 3000000
  mkfile "$T/kb" 3100000; mkfile "$T/ib" 3100000
  mkfile "$T/kc" 14000000; mkfile "$T/ic" 14000000
  run install-debian "$T/ka" "$T/ia" --cmdline "root=/dev/sda1" --yes
  assert "A fits (6 MB + 1 MiB in about 12 MiB free)" rc_is 0
  run install-debian "$T/kb" "$T/ib" --cmdline "root=/dev/sda1" --yes
  assert "B succeeds after dropping the fallback to make room" rc_is 0
  assert "...with a warning about deleting .old" out_has 'deleting the \.old'
  assert "...vmlinuz.old is gone" not p2_has "$IMG" /synofurb/vmlinuz.old
  conf_of "$IMG" > "$T/c1"
  assert "...and the DEBIAN_OLD entry is gone with it" not grep -q DEBIAN_OLD "$T/c1"
  assert "B is current" eq "$(p2_sha "$IMG" /synofurb/vmlinuz)" "$(sha "$T/kb")"
  local s1; s1=$(sha "$IMG")
  run install-debian "$T/kc" "$T/ic" --cmdline "root=/dev/sda1" --yes
  assert "oversized install fails" rc_nz
  assert "...with a clear space message" out_has 'not enough space on partition 2: need'
  assert "...mentions the 1 MiB margin / sizes" out_has 'need [0-9.]+ MiB'
  assert "...device bit-identical afterwards" eq "$(sha "$IMG")" "$s1"
  assert "DSM files intact" eq "$(dsm_sums "$IMG")" "$before_dsm"
  # oversized with an existing fallback: .old is dropped in the work copy only; device unchanged
  run install-debian "$T/ka" "$T/ia" --cmdline "root=/dev/sda1" --yes
  s1=$(sha "$IMG")
  run install-debian "$T/kc" "$T/ic" --cmdline "root=/dev/sda1" --yes
  assert "oversized with fallback present fails too" rc_nz
  assert "...device (including the old fallback) untouched" eq "$(sha "$IMG")" "$s1"
  run install-agent "$T/kc" "$T/ic" --yes
  assert "oversized agent fails" rc_nz
  assert "...device untouched" eq "$(sha "$IMG")" "$s1"
  # low TMP space
  mkdir -p "$T/smalltmp"
  mount -t tmpfs -o size=20m tmpfs "$T/smalltmp"
  TMPD_SAVE=$TMPD; TMPD=$T/smalltmp
  run install-agent "$T/ka" "$T/ia" --yes
  assert "too little temp space: refused" rc_nz
  assert "...message names SYNOFURB_TMP" out_has 'SYNOFURB_TMP|not enough free space'
  assert "...device untouched" eq "$(sha "$IMG")" "$s1"
  TMPD=$TMPD_SAVE
  umount "$T/smalltmp"
}

t_efi() {
  begin "install-efi"
  fresh mbr
  local before_dsm; before_dsm=$(dsm_sums "$IMG")
  printf 'MZ' > "$T/test.efi"; head -c 100000 /dev/urandom >> "$T/test.efi"
  run install-efi memtest "$T/test.efi" --yes
  assert "install-efi exits 0" rc_is 0
  assert "efi file on P1" p1_has "$IMG" EFI/synofurb/memtest.efi
  conf_of "$IMG" > "$T/c1"
  assert "EFI_memtest entry present" grep -q 'menuentry "EFI_memtest"' "$T/c1"
  assert "chainloader line" grep -qP '^\tchainloader /EFI/synofurb/memtest.efi$' "$T/c1"
  assert "FAT volume id search line" grep -qP '^\tsearch --fs-uuid --no-floppy --set=root 10EE-589C$' "$T/c1"
  assert "entry is flagged as UNVERIFIED in the output" out_has 'UNVERIFIED'
  assert "efi content identical" eq "$(mcopy -i "$IMG@@$((P1S * 512))" ::EFI/synofurb/memtest.efi - | sha256sum | cut -d' ' -f1)" "$(sha "$T/test.efi")"
  assert "DSM files untouched" eq "$(dsm_sums "$IMG")" "$before_dsm"
  local s1; s1=$(sha "$IMG")
  run install-efi memtest "$T/test.efi" --yes
  assert "idempotent" out_has 'Nothing to change'
  assert "...bit-identical" eq "$(sha "$IMG")" "$s1"
  run default efi:memtest --yes
  conf_of "$IMG" > "$T/c2"
  assert "default efi:memtest -> its index" eq "$(default_value < "$T/c2")" "$(entry_idx "$T/c2" EFI_memtest)"
  assert "fsck clean" fscks_clean "$IMG"
  head -c 5000 /dev/urandom > "$T/notefi.bin"
  run install-efi bad "$T/notefi.bin" --yes
  assert "non-PE file is refused" rc_nz
  run install-efi 'bad name' "$T/test.efi" --yes
  assert "name with space is refused" rc_nz
  run install-efi '../x' "$T/test.efi" --yes
  assert "name with path chars is refused" rc_nz
  run install-efi "$(printf 'a%.0s' $(seq 1 40))" "$T/test.efi" --yes
  assert "name over 32 chars is refused" rc_nz
  run remove --yes
  assert "remove exits 0" rc_is 0
  assert "efi file gone" not p1_has "$IMG" EFI/synofurb/memtest.efi
  assert "efi dir gone" not p1_has "$IMG" EFI/synofurb
  assert "fsck clean after remove" fscks_clean "$IMG"
}

t_remove() {
  begin "remove"
  fresh mbr
  local before_dsm; before_dsm=$(dsm_sums "$IMG")
  mkfile "$T/ak" 1000000; mkfile "$T/ai" 1000000
  mkfile "$T/dk" 2000000; mkfile "$T/di" 2000000; mkfile "$T/dk2" 2100000
  printf 'MZ' > "$T/e.efi"; head -c 3000 /dev/urandom >> "$T/e.efi"
  run install-agent "$T/ak" "$T/ai" --arm --yes
  run install-debian "$T/dk" "$T/di" --cmdline "root=/dev/sda1" --yes
  run install-debian "$T/dk2" "$T/di" --cmdline "root=/dev/sda1" --yes
  run install-efi tool "$T/e.efi" --yes
  run status
  assert "everything installed, default is the agent" out_has 'Default: +SYNOFURB_BOOT'
  run remove --yes
  assert "remove exits 0" rc_is 0
  conf_of "$IMG" > "$T/c1"
  conf_of "$T/orig.img" > "$T/c0"
  assert "conf is byte-identical to the original after remove" eq "$(sha "$T/c1")" "$(sha "$T/c0")"
  assert "default back on DSM (index 1)" eq "$(default_value < "$T/c1")" 1
  for f in agent.vmlinuz agent.initrd vmlinuz initrd.img vmlinuz.old initrd.img.old MANIFEST; do
    assert "removed /synofurb/$f" not p2_has "$IMG" /synofurb/$f
  done
  assert "/synofurb directory removed" not p2_has "$IMG" /synofurb
  assert "EFI dir removed" not p1_has "$IMG" EFI/synofurb
  assert "DSM files present and unchanged" eq "$(dsm_sums "$IMG")" "$before_dsm"
  assert "fsck clean" fscks_clean "$IMG"
  local s1; s1=$(sha "$IMG")
  run remove --yes
  assert "remove on a clean image: nothing to change" out_has 'Nothing to change'
  assert "...bit-identical" eq "$(sha "$IMG")" "$s1"
  # remove when the original default was not DSM: default is forced to DSM
  fresh mbr
  sed "s/^set default='1'/set default='0'/" < <(conf_of "$IMG") > "$T/c.mod"
  set_conf "$IMG" "$T/c.mod"
  run install-agent "$T/ak" "$T/ai" --yes
  run remove --yes
  conf_of "$IMG" > "$T/c2"
  assert "remove sets the default to the DSM entry even if it was entry 0" eq "$(default_value < "$T/c2")" 1
  assert "everything else identical to the modified original" eq "$(outside < "$T/c2" | sha256sum)" "$(outside < "$T/c.mod" | sha256sum)"
}

t_crlf() {
  begin "CRLF conf is preserved"
  fresh mbr
  conf_of "$IMG" | sed 's/$/\r/' > "$T/crlf.conf"
  set_conf "$IMG" "$T/crlf.conf"
  cp "$IMG" "$T/orig.img"
  mkfile "$T/ak" 1000000; mkfile "$T/ai" 1000000
  run install-agent "$T/ak" "$T/ai" --cmdline "console=tty0" --arm --yes
  assert "install on CRLF conf exits 0" rc_is 0
  conf_of "$IMG" > "$T/c1"
  assert "no bare-LF line anywhere (every line ends in CRLF)" bash -c "! grep -qvP '\r$' $T/c1"
  assert "our block lines end in CRLF" bash -c "sed -n '/^### synofurb begin/,/^### synofurb end/p' $T/c1 | grep -c -P '\r$' | grep -qx 7"
  assert "default line kept CRLF and quotes" grep -qP "^set default='2'\r$" "$T/c1"
  assert "original lines outside the block byte-identical" eq "$(outside < "$T/c1" | sha256sum)" "$(outside < "$T/crlf.conf" | sha256sum)"
  run default dsm --yes
  run remove --yes
  conf_of "$IMG" > "$T/c2"
  assert "CRLF conf is byte-identical after install+remove" eq "$(sha "$T/c2")" "$(sha "$T/crlf.conf")"
  assert "fsck clean" fscks_clean "$IMG"

  begin "conf without trailing newline"
  fresh mbr
  conf_of "$IMG" | head -c -1 > "$T/nonl.conf"
  set_conf "$IMG" "$T/nonl.conf"
  run default dsm --yes
  assert "default dsm on a conf without final newline exits 0" rc_is 0
  assert "...nothing to change (index already 1)" out_has 'Nothing to change'
  run install-agent "$T/ak" "$T/ai" --yes
  assert "install-agent exits 0" rc_is 0
  conf_of "$IMG" > "$T/c3"
  assert "original bytes preserved up to the block" bash -c "head -c $(wc -c < "$T/nonl.conf") $T/c3 | cmp -s - $T/nonl.conf"
  assert "block starts on its own line" grep -q '^### synofurb begin' "$T/c3"
  # armed default on a no-newline conf -> only the digit changes
  fresh mbr
  conf_of "$IMG" | head -c -1 > "$T/nonl.conf"
  set_conf "$IMG" "$T/nonl.conf"
  sed "s/^set default='1'/set default='0'/" "$T/nonl.conf" | head -c -1 > "$T/nonl0.conf"
  [ "$(tail -c 1 "$T/nonl0.conf" | xxd -p)" = "7d" ] || sed -i '$ s/$//' "$T/nonl0.conf"
  run default dsm --yes
  conf_of "$IMG" > "$T/c4"
  assert "no trailing newline was added" eq "$(tail -c 1 "$T/c4" | xxd -p)" "$(tail -c 1 "$T/nonl.conf" | xxd -p)"
}

t_default_tracking() {
  begin "default follows its entry by name when the block grows"
  fresh mbr
  mkfile "$T/ak" 1000000; mkfile "$T/ai" 1000000; mkfile "$T/dk" 1000000; mkfile "$T/di" 1000000
  run install-agent "$T/ak" "$T/ai" --yes
  # a user adds their own entry AFTER our block and makes it the default
  conf_of "$IMG" > "$T/c1"
  printf 'menuentry "USER_TOOL" {\n\tlinux /zImage x=1\n\tinitrd /rd.gz\n}\n' >> "$T/c1"
  sed -i "s/^set default='1'/set default='3'/" "$T/c1"
  set_conf "$IMG" "$T/c1"
  assert "setup: USER_TOOL is index 3" eq "$(entry_idx "$T/c1" USER_TOOL)" 3
  run install-debian "$T/dk" "$T/di" --cmdline "root=/dev/sda1" --yes
  assert "install-debian exits 0" rc_is 0
  conf_of "$IMG" > "$T/c2"
  assert "USER_TOOL moved to index 4" eq "$(entry_idx "$T/c2" USER_TOOL)" 4
  assert "default still points to USER_TOOL" eq "$(default_value < "$T/c2")" 4
  assert "the user entry text is preserved" grep -q 'linux /zImage x=1' "$T/c2"
  run remove --yes
  conf_of "$IMG" > "$T/c3"
  assert "remove sets default to DSM and USER_TOOL entry stays" eq "$(entry_idx "$T/c3" USER_TOOL)" 2
  assert "default is DSM index" eq "$(default_value < "$T/c3")" 1
  # duplicate entry names outside our block are not duplicated by us
  fresh mbr
  conf_of "$IMG" > "$T/c4"
  printf 'menuentry "DEBIAN" {\n\tlinux /zImage\n}\n' >> "$T/c4"
  set_conf "$IMG" "$T/c4"
  local s1; s1=$(sha "$IMG")
  run install-debian "$T/dk" "$T/di" --cmdline "x" --yes
  assert "a pre-existing DEBIAN entry outside the block makes install refuse" rc_nz
  assert "...device untouched" eq "$(sha "$IMG")" "$s1"
  # unbalanced marker
  fresh mbr
  { conf_of "$IMG"; echo '### synofurb begin (managed by synofurb-dom)'; } > "$T/c5"
  set_conf "$IMG" "$T/c5"
  s1=$(sha "$IMG")
  run install-agent "$T/ak" "$T/ai" --yes
  assert "unterminated block is refused" rc_nz
  assert "...device untouched" eq "$(sha "$IMG")" "$s1"
  # conf without the DSM entry
  fresh mbr
  conf_of "$IMG" | sed 's/SYNOLOGY_2/SYNOLOGY_9/' > "$T/c6"
  set_conf "$IMG" "$T/c6"
  s1=$(sha "$IMG")
  run install-agent "$T/ak" "$T/ai" --yes
  assert "a conf without SYNOLOGY_2 is refused" rc_nz
  assert "...device untouched" eq "$(sha "$IMG")" "$s1"
}

t_rollback() {
  begin "automatic rollback"
  fresh mbr
  mkfile "$T/dk" 2000000; mkfile "$T/di" 2000000
  runenv SYNOFURB_TEST_FAIL_AFTER_WRITE=1 -- install-debian "$T/dk" "$T/di" --cmdline "root=/dev/sda1" --yes
  assert "failure injected after the 1st partition write: exit non-zero" rc_nz
  assert "...says it restored" out_has 'ROLLBACK OK'
  assert "...device bit-identical to the original" eq "$(sha "$IMG")" "$ORIG_SHA"
  assert "...no temp files left" eq "$(tmpleft)" 0
  fresh mbr
  runenv SYNOFURB_TEST_FAIL_AFTER_WRITE=2 -- install-debian "$T/dk" "$T/di" --cmdline "root=/dev/sda1" --yes
  assert "failure injected after BOTH partitions written: exit non-zero" rc_nz
  assert "...rollback ok" out_has 'ROLLBACK OK'
  assert "...both partitions restored (bit-identical)" eq "$(sha "$IMG")" "$ORIG_SHA"
  fresh mbr
  runenv SYNOFURB_TEST_FAIL_AFTER_WRITE=1 -- default agent --yes
  assert "default on missing entry fails before writing" rc_nz
  mkfile "$T/ak" 1000000; mkfile "$T/ai" 1000000
  run install-agent "$T/ak" "$T/ai" --yes
  local s1; s1=$(sha "$IMG")
  runenv SYNOFURB_TEST_FAIL_AFTER_WRITE=1 -- default agent --yes
  assert "P1-only change with injected failure: non-zero" rc_nz
  assert "...restored" out_has 'ROLLBACK OK'
  assert "...bit-identical to the pre-command state" eq "$(sha "$IMG")" "$s1"
  fresh mbr
  runenv SYNOFURB_TEST_FAIL_AFTER_WRITE=1 -- install-agent "$T/ak" "$T/ai" --arm --yes
  assert "an armed agent install that fails leaves default on DSM" eq "$(conf_of "$IMG" | default_value)" 1
  assert "...and the whole image identical" eq "$(sha "$IMG")" "$ORIG_SHA"
  assert "the automatic first backup still exists after a failed write" test -f "$STATE/synofurb-dom-first-backup.img"
  # concurrent change between read and write
  fresh mbr
  cp "$IMG" "$T/pre.img"
  runenv SYNOFURB_TEST_TAMPER=1 -- install-agent "$T/ak" "$T/ai" --yes
  assert "concurrent modification: non-zero" rc_nz
  assert "...message says the device changed" out_has 'changed on the device'
  assert "...P1 is untouched" eq "$(dd if="$IMG" bs=512 skip="$P1S" count="$P1N" 2>/dev/null | sha256sum)" "$(dd if="$T/pre.img" bs=512 skip="$P1S" count="$P1N" 2>/dev/null | sha256sum)"
  assert "...only the tampered byte differs from the original (nothing of ours was written)" eq "$(cmp -l "$IMG" "$T/pre.img" | wc -l)" 1
}

t_mounted() {
  begin "refuse when a partition is mounted"
  fresh mbr
  p1_extract "$IMG"; p2_extract "$IMG"
  local lo
  lo=$(losetup -f --show "$T/p2.cur")
  mkdir -p "$T/mnt"
  mount -t ext2 -o ro "$lo" "$T/mnt"
  local s1; s1=$(sha "$T/p1.cur")$(sha "$lo")
  env SYNOFURB_DOM1="$T/p1.cur" SYNOFURB_DOM2="$lo" SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" $SH "$SCRIPT" default dsm --yes > "$OUT" 2>&1
  RC=$?
  assert "mounted P2 (loop device): write refused" rc_nz
  assert "...message says mounted" out_has 'mounted'
  assert "...partition images unchanged" eq "$(sha "$T/p1.cur")$(sha "$lo")" "$s1"
  env SYNOFURB_DOM1="$T/p1.cur" SYNOFURB_DOM2="$lo" SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" $SH "$SCRIPT" status > "$OUT" 2>&1
  RC=$?
  assert "status (read-only) still works but warns" rc_is 0
  assert "...warning mentions mounted" out_has 'mounted'
  umount "$T/mnt"; losetup -d "$lo"
  # same via whole device: a loop device of the image with a mounted partition copy is not possible without
  # partition nodes, so also check the DOM file being attached to a mounted loop device
  fresh mbr
  lo=$(losetup -f --show -o $((P2S * 512)) --sizelimit $((P2N * 512)) "$IMG")
  mount -t ext2 -o ro "$lo" "$T/mnt"
  run install-agent "$T/dsm/boot.efi" "$T/dsm/boot.efi" --yes
  assert "image file whose loop device is mounted: write refused" rc_nz
  assert "...device unchanged" eq "$(sha "$IMG")" "$ORIG_SHA"
  umount "$T/mnt"; losetup -d "$lo"
}

t_overrides() {
  begin "SYNOFURB_DOM1 / SYNOFURB_DOM2 partition overrides"
  fresh mbr
  p1_extract "$IMG"; p2_extract "$IMG"
  cp "$T/p1.cur" "$T/ov1.img"; cp "$T/p2.cur" "$T/ov2.img"
  mkfile "$T/ak" 1000000; mkfile "$T/ai" 1000000
  env SYNOFURB_DOM1="$T/ov1.img" SYNOFURB_DOM2="$T/ov2.img" SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" $SH "$SCRIPT" install-agent "$T/ak" "$T/ai" --arm --yes > "$OUT" 2>&1
  RC=$?
  assert "install-agent on separate partition files exits 0" rc_is 0
  assert "partition files keep their exact size (no truncation)" eq "$(stat -c %s "$T/ov1.img")$(stat -c %s "$T/ov2.img")" "$(stat -c %s "$T/p1.cur")$(stat -c %s "$T/p2.cur")"
  assert "automatic backup of the partition images exists" test -f "$STATE/synofurb-dom-first-backup.p2.img"
  assert "automatic p2 backup is the original p2" eq "$(sha "$STATE/synofurb-dom-first-backup.p2.img")" "$(sha "$T/p2.cur")"
  assert "fsck ext clean" bash -c 'e2fsck -fn "$1" >/dev/null 2>&1' _ "$T/ov2.img"
  assert "fsck vfat clean" bash -c 'fsck.vfat -n "$1" >/dev/null 2>&1' _ "$T/ov1.img"
  env SYNOFURB_DOM1="$T/ov1.img" SYNOFURB_DOM2="$T/ov2.img" SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" $SH "$SCRIPT" backup "$T/bk2" > "$OUT" 2>&1
  RC=$?
  assert "backup needs the whole device (SYNOFURB_DOM)" rc_nz
}

t_gpt_fat32() {
  begin "GPT table"
  fresh gpt
  local before_dsm; before_dsm=$(dsm_sums "$IMG")
  run status
  assert "GPT image recognised" rc_is 0
  mkfile "$T/ak" 1000000; mkfile "$T/ai" 1000000
  run install-agent "$T/ak" "$T/ai" --arm --yes
  assert "install on GPT exits 0" rc_is 0
  conf_of "$IMG" > "$T/c1"
  assert "GPT: default armed" eq "$(default_value < "$T/c1")" "$(entry_idx "$T/c1" SYNOFURB_BOOT)"
  assert "GPT: partition table left intact" eq "$(sfdisk -l "$IMG" 2>&1 | grep -c "^$IMG")" 2
  assert "GPT: sfdisk --verify finds no errors" bash -c "sfdisk --verify $IMG >/dev/null 2>&1"
  assert "GPT: DSM files untouched" eq "$(dsm_sums "$IMG")" "$before_dsm"
  assert "GPT: bytes outside partitions (table, gaps) unchanged" eq "$(head -c $((P1S * 512)) "$IMG" | sha256sum)" "$(head -c $((P1S * 512)) "$T/orig.img" | sha256sum)"
  run remove --yes
  assert "GPT: remove works" rc_is 0
  assert "GPT: image restored bit-for-bit in the partitions' content area (conf)" eq "$(sha256sum < <(conf_of "$IMG"))" "$(sha256sum < <(conf_of "$T/orig.img"))"

  begin "FAT32 (volume id at offset 67)"
  fresh fat32
  run status
  assert "FAT32 image recognised" rc_is 0
  run install-agent "$T/ak" "$T/ai" --arm --yes
  assert "FAT32: install exits 0" rc_is 0
  assert "FAT32: fsck clean" fscks_clean "$IMG"
  run verify
  assert "FAT32: verify passes" rc_is 0
  drop_base gpt; drop_base fat32
}

t_detect() {
  begin "auto-detection via /dev/synoboot and block device scan"
  fresh mbr
  local lo
  lo=$(losetup -f --show "$IMG")
  ln -sf "$lo" /dev/synoboot
  env SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" $SH "$SCRIPT" status > "$OUT" 2>&1
  RC=$?
  assert "/dev/synoboot (block device without partition nodes) is auto-detected" rc_is 0
  assert "...device shown" out_has "Device: +/dev/synoboot"
  mkfile "$T/ak" 1000000; mkfile "$T/ai" 1000000
  env SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" $SH "$SCRIPT" install-agent "$T/ak" "$T/ai" --arm --yes > "$OUT" 2>&1
  RC=$?
  assert "install through the block device writes through" rc_is 0
  sync; losetup -d "$lo" 2>/dev/null; rm -f /dev/synoboot
  assert "the image file got the agent entry" bash -c "mcopy -i '$IMG@@$((P1S * 512))' ::EFI/boot/SynoBootLoader.conf - | grep -q SYNOFURB_BOOT"
  assert "fsck clean" fscks_clean "$IMG"
  # scan
  fresh mbr
  lo=$(losetup -f --show "$IMG")
  env SYNOFURB_SCAN_ALL=1 SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" $SH "$SCRIPT" status > "$OUT" 2>&1
  RC=$?
  assert "scan of /sys/block finds the DOM" rc_is 0
  assert "...it is the loop device" out_has "Device: +$lo"
  env SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" $SH "$SCRIPT" status > "$OUT" 2>&1
  RC=$?
  assert "without SYNOFURB_SCAN_ALL loop devices are not scanned (no DOM found)" rc_nz
  losetup -d "$lo"
}

t_noroot() {
  begin "root requirement"
  fresh mbr
  chmod 755 "$T" "$BASES" 2>/dev/null; chmod 644 "$IMG"
  if command -v setpriv >/dev/null 2>&1; then
    setpriv --reuid=65534 --regid=65534 --clear-groups env SYNOFURB_DOM="$IMG" SYNOFURB_STATE="$STATE" SYNOFURB_TMP="$TMPD" SYNOFURB_LOCK="$T/lock" $SH "$SCRIPT" status > "$OUT" 2>&1
    RC=$?
    assert "non-root is refused" rc_nz
    assert "...message" out_has 'must be run as root'
  fi
}

t_verify() {
  begin "verify detects damage"
  fresh mbr
  mkfile "$T/dk" 1000000; mkfile "$T/di" 1000000
  run install-debian "$T/dk" "$T/di" --cmdline "x" --yes
  # delete a referenced file behind the tool's back
  p2_extract "$IMG"
  debugfs -w -R "rm /synofurb/initrd.img" "$T/p2.cur" >/dev/null 2>&1
  dd if="$T/p2.cur" of="$IMG" bs=512 seek="$P2S" conv=notrunc 2>/dev/null
  run verify
  assert "verify fails when an entry references a missing file" rc_nz
  assert "...names the file" out_has 'initrd.img'
  # but remove can still clean up (pre-existing problem is tolerated)
  run remove --yes
  assert "remove works on a DOM with a dangling reference" rc_is 0
  run verify
  assert "verify passes after remove" rc_is 0
  # invalid default index
  fresh mbr
  sed "s/^set default='1'/set default='9'/" < <(conf_of "$IMG") > "$T/c.bad"
  set_conf "$IMG" "$T/c.bad"
  run verify
  assert "verify fails on a default that points nowhere" rc_nz
  assert "...says so" out_has 'does not point to an existing entry'
  mkfile "$T/ak" 1000000; mkfile "$T/ai" 1000000
  run install-agent "$T/ak" "$T/ai" --yes
  assert "a write repairs an invalid default (resets to DSM)" rc_is 0
  assert "...default now valid DSM index" eq "$(conf_of "$IMG" | default_value)" 1
  # fsck problem in the original
  fresh mbr
  p2_extract "$IMG"
  debugfs -w -R "set_inode_field <2> mode 0" "$T/p2.cur" >/dev/null 2>&1
  dd if="$T/p2.cur" of="$IMG" bs=512 seek="$P2S" conv=notrunc 2>/dev/null
  local s1; s1=$(sha "$IMG")
  run install-agent "$T/ak" "$T/ai" --yes
  assert "a filesystem with fsck problems is not written to" rc_nz
  assert "...message mentions fsck" out_has 'fsck'
  assert "...device untouched" eq "$(sha "$IMG")" "$s1"
  run verify
  assert "verify reports the fsck problem" rc_nz
}

t_fatmount_ro() {
  begin "FAT 'mount' backend (read-only, FUSE stand-in for the missing vfat driver)"
  if ! command -v fusefat >/dev/null 2>&1 || ! command -v fusermount >/dev/null 2>&1 || [ ! -c /dev/fuse ]; then
    echo "  skip  fusefat/fusermount//dev/fuse not available"
    return
  fi
  if [ -n "${TEST_BB:-}" ]; then
    echo "  skip  busybox ash runs its built-in mount/umount applets, the PATH shim does not apply"
    return
  fi
  local shim=$T/shim
  mkdir -p "$shim"
  cat > "$shim/mount" <<'EOS'
#!/bin/sh
# test shim: vfat loop mounts are served by fusefat (read-only), all else by the real mount
case " $* " in
  *" vfat "*)
    n=$#; i=0
    for a in "$@"; do i=$((i+1)); [ $i -eq $((n-1)) ] && img=$a; [ $i -eq $n ] && mnt=$a; done
    exec fusefat -o ro "$img" "$mnt" >/dev/null ;;
esac
exec /usr/bin/mount "$@"
EOS
  cat > "$shim/umount" <<'EOS'
#!/bin/sh
if awk -v m="$1" '$2 == m && $3 ~ /^fuse/ { f = 1 } END { exit !f }' /proc/mounts; then exec fusermount -u "$1"; fi
exec /usr/bin/umount "$@"
EOS
  chmod +x "$shim/mount" "$shim/umount"
  fresh mbr
  runenv "PATH=$shim:$PATH" SYNOFURB_FAT_BACKEND=mount -- status
  assert "status via the mount backend exits 0" rc_is 0
  assert "...lists the entries read through the mounted FAT" out_has 'SYNOLOGY_2 \[DSM\]'
  assert "...no mtools fallback warning" out_lacks 'using mtools'
  runenv "PATH=$shim:$PATH" SYNOFURB_FAT_BACKEND=mount -- verify
  assert "verify via the mount backend passes" rc_is 0
  assert "...device untouched" eq "$(sha "$IMG")" "$ORIG_SHA"
  assert "...nothing left mounted" eq "$(grep -c "$T" /proc/mounts)" 0
}

for t in ${ONLY:-t_static t_status t_refusals t_backup_restore t_install_agent t_arm_default t_debian t_space t_efi t_remove t_crlf t_default_tracking t_rollback t_mounted t_overrides t_gpt_fat32 t_detect t_noroot t_verify t_fatmount_ro}; do
  "$t"
done

printf '\n=========================================\n'
printf "shell under test: %s   passed: %s   failed: %s\n" "${TEST_SH:-dash}${TEST_BB:+ (busybox applets)}" "$PASS" "$FAIL"
if [ "$FAIL" -ne 0 ]; then
  printf 'Failures:\n'
  printf '  %s\n' "${FAILED[@]}"
  exit 1
fi
exit 0
