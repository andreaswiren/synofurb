#!/usr/bin/env bash
# Structural check of a built agent - works for every architecture because it never executes the
# target's code (this is the only check possible for arm64/armhf on an x86-64 host).
#
#   test/check-structure.sh <amd64|arm64|armhf> [--dist DIR]
#
# Verifies: kernel file type and release string, initrd layout (uncompressed device-node archive followed by
# one xz/zstd cpio), /init and scripts, ELF machine type of every binary, program interpreter present at its
# exact path, every DT_NEEDED library present, module tree + modules.dep consistency, size budget fields.
set -uo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ARCH=${1:?usage: check-structure.sh <amd64|arm64|armhf> [--dist DIR]}; shift
DIST=$(dirname "$HERE")/dist
[[ ${1:-} == --dist ]] && DIST=$(realpath "$2")
case $ARCH in
    amd64) MACHINE="Advanced Micro Devices X86-64"; INTERP=/lib64/ld-linux-x86-64.so.2; TRIPLET=x86_64-linux-gnu ;;
    arm64) MACHINE="AArch64";                       INTERP=/lib/ld-linux-aarch64.so.1;   TRIPLET=aarch64-linux-gnu ;;
    armhf) MACHINE="ARM";                           INTERP=/lib/ld-linux-armhf.so.3;     TRIPLET=arm-linux-gnueabihf ;;
    *) echo "bad arch"; exit 2 ;;
esac
K=$DIST/agent-$ARCH.vmlinuz; I=$DIST/agent-$ARCH.initrd
[[ -f $K && -f $I ]] || { echo "missing $K or $I" >&2; exit 2; }
PASS=0; FAIL=0
ok()  { PASS=$((PASS + 1)); printf '  PASS  %s\n' "$*"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL  %s\n' "$*"; }
chk() { local d=$1; shift; if "$@" >/dev/null 2>&1; then ok "$d"; else bad "$d"; fi; }

echo "== $ARCH structure check ($DIST)"
echo "  kernel: $(file -b "$K" | cut -c1-200)"
echo "  sizes : vmlinuz $(stat -c %s "$K") B, initrd $(stat -c %s "$I") B, total $(( $(stat -c %s "$K") + $(stat -c %s "$I") )) B"
( cd "$DIST" && sha256sum -c --ignore-missing SHA256SUMS >/dev/null 2>&1 ) && ok "SHA256SUMS matches the files" || bad "SHA256SUMS mismatch"

# --- initrd layout
magic=$(head -c 6 "$I")
[[ $magic == 070701 ]] && ok "initrd starts with a newc cpio archive (device nodes)" || bad "initrd does not start with a newc archive"
off=$(grep -obUaP '\xfd7zXZ\x00' "$I" | head -n1 | cut -d: -f1)
[[ -n $off ]] && ok "compressed (xz) main archive starts at byte $off" || bad "no xz stream found"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
head -c "$off" "$I" > "$T/dev.cpio"
cpio -t < "$T/dev.cpio" 2>/dev/null | tr '\n' ' ' > "$T/dev.lst"
grep -q 'dev/console' "$T/dev.lst" && grep -q 'dev/null' "$T/dev.lst" && ok "first archive provides dev/console and dev/null ($(cat "$T/dev.lst"))" || bad "device node archive wrong: $(cat "$T/dev.lst")"
mkdir "$T/root"
tail -c +$((off + 1)) "$I" | xz -dc | ( cd "$T/root" && cpio -idm --quiet 2>"$T/cpio.err" ) 
chk "main archive unpacks (xz + cpio)" test -f "$T/root/init"
R=$T/root
echo "  files in the main archive: $(find "$R" | wc -l), unpacked size $(du -sb "$R" | cut -f1) B"

# --- scripts and config
for f in init usr/bin/sf-boot usr/lib/synofurb/boot.sh usr/lib/synofurb/lib.sh usr/lib/synofurb/mkinittab usr/lib/synofurb/svc-net \
         usr/lib/synofurb/svc-dropbear usr/lib/synofurb/svc-beacon usr/lib/synofurb/svc-watchdog usr/lib/synofurb/autorun \
         usr/lib/synofurb/udhcpc.script usr/lib/synofurb/announce usr/bin/synofurb-dom etc/synofurb-agent-release; do
    chk "present: $f" test -f "$R/$f"
done
for f in init usr/bin/sf-boot usr/lib/synofurb/boot.sh usr/bin/synofurb-dom; do chk "executable: $f" test -x "$R/$f"; done
grep -q '^AGENT_DOM_STUB=0' "$R/etc/synofurb-agent-release" && ok "real synofurb-dom embedded (not the stub)" || bad "synofurb-dom is the STUB in this build"
chk "/usr/bin/busybox is a static ELF" bash -c "readelf -l '$R/usr/bin/busybox' | grep -q 'Requesting program interpreter' && exit 1; readelf -h '$R/usr/bin/busybox' | grep -q ELF"
napp=$(find "$R/usr/bin" -type l -lname busybox | wc -l)
(( napp > 200 )) && ok "busybox applet links: $napp" || bad "too few applet links: $napp"
chk "usr-merge symlinks (bin sbin lib)" bash -c "[ -L '$R/bin' ] && [ -L '$R/sbin' ] && [ -L '$R/lib' ]"

# --- ELF machine, interpreter, NEEDED
chkelf() {
    local f=$1 m i n missing=""
    m=$(readelf -h "$f" 2>/dev/null | sed -n 's/^ *Machine: *//p')
    [[ $m == "$MACHINE" ]] || { bad "$(basename "$f"): machine '$m' != '$MACHINE'"; return; }
    i=$(readelf -l "$f" 2>/dev/null | sed -n 's/.*Requesting program interpreter: \(.*\)\]/\1/p')
    [[ -z $i || $i == "$INTERP" ]] || { bad "$(basename "$f"): interpreter '$i' != '$INTERP'"; return; }
    for n in $(readelf -d "$f" 2>/dev/null | sed -n 's/.*Shared library: \[\(.*\)\].*/\1/p'); do
        [[ -e $R/usr/lib/$TRIPLET/$n ]] || missing="$missing $n"
    done
    [[ -z $missing ]] && ok "$(basename "$f"): $MACHINE, interp ${i:-none (static)}, all NEEDED libs present" || bad "$(basename "$f"): missing libs:$missing"
}
for b in busybox dropbear dropbearkey kexec e2fsck fsck.fat nc.traditional; do chkelf "$R/usr/bin/$b"; done
for l in "$R/usr/lib/$TRIPLET"/*.so*; do
    m=$(readelf -h "$l" 2>/dev/null | sed -n 's/^ *Machine: *//p')
    [[ $m == "$MACHINE" ]] || bad "library $(basename "$l") has machine '$m'"
done
chk "libraries: all $(ls "$R/usr/lib/$TRIPLET" | wc -l) are $MACHINE" bash -c "! (for l in '$R/usr/lib/$TRIPLET'/*.so*; do readelf -h \"\$l\" | sed -n 's/^ *Machine: *//p'; done | grep -qv '$MACHINE')"
ld=$R/usr$INTERP
chk "program interpreter present at $INTERP (usr-merged: $(readlink "$ld" 2>/dev/null))" test -e "$(dirname "$ld")/$(readlink "$ld")" -o -e "$ld"

# --- kernel modules
KREL=$(sed -n 's/^AGENT_KERNEL=//p' "$R/etc/synofurb-agent-release")
MD=$R/usr/lib/modules/$KREL
chk "module tree for $KREL exists with modules.dep + modules.alias" bash -c "[ -s '$MD/modules.dep' ] && [ -s '$MD/modules.alias' ]"
if file -b "$K" | grep -q 'version '; then
    chk "kernel image reports release $KREL" bash -c "file -b '$K' | grep -qF '$KREL'"
else
    echo "  note  kernel image is compressed/opaque to file(1); release match with the module tree not checked ($(file -b "$K" | cut -c1-80))"
fi
nmod=$(find "$MD" -name '*.ko' | wc -l)
echo "  modules: $nmod .ko files"
bad_ko=0; for ko in $(find "$MD" -name '*.ko'); do
    m=$(readelf -h "$ko" 2>/dev/null | sed -n 's/^ *Machine: *//p'); [[ $m == "$MACHINE" ]] || bad_ko=$((bad_ko + 1))
done
(( bad_ko == 0 )) && ok "every module is $MACHINE" || bad "$bad_ko modules have a different machine type"
missing_dep=0; while IFS=': ' read -r path deps; do
    [[ -e $MD/${path} ]] || missing_dep=$((missing_dep + 1))
    for d in $deps; do [[ -e $MD/$d ]] || missing_dep=$((missing_dep + 1)); done
done < "$MD/modules.dep"
(( missing_dep == 0 )) && ok "modules.dep is self-consistent (every listed file exists)" || bad "$missing_dep dangling modules.dep entries"
for m in $(sed 's/ /\n/g' <<<"${REQUIRE_MODULES:-}"); do chk "module present: $m" bash -c "find '$MD' -name '$m.ko' | grep -q ."; done

echo
echo "RESULT ($ARCH): $PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
