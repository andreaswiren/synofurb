#!/usr/bin/env bash
# synofurb boot agent - build script
#
#   ./build.sh <amd64|arm64|armhf> [--suite trixie|trixie-backports] [--out DIR] [options]
#
# Produces, in --out (default ./dist):
#   agent-<arch>.vmlinuz   the Debian kernel image, unchanged
#   agent-<arch>.initrd    tiny initramfs (cpio+xz): busybox, dropbear, kexec, NIC/storage modules, our scripts
#   SHA256SUMS             (accumulates entries of all architectures built into the same --out)
#   MANIFEST-<arch>.json   versions, sizes, module list, package list
#   initrd-<arch>.lst      listing of the initramfs
#
# Everything is downloaded over HTTPS from the Debian mirror and verified along the chain
#   InRelease (PGP via gpgv when a Debian keyring is installed) -> Packages.xz (SHA256) -> .deb (SHA256)
# and unpacked with dpkg-deb -x. No foreign-architecture code is executed.
#
# Host needs (Debian/Ubuntu: apt-get install curl dpkg-dev xz-utils cpio kmod binutils gawk gnupg
# debian-archive-keyring [zstd]): curl dpkg-deb xz cpio depmod readelf awk sha256sum [gpgv] [zstd]
set -Eeuo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# ------------------------------------------------------------------ configuration
AGENT_VERSION=${AGENT_VERSION:-0.1.0}
SUITE=trixie-backports
OUT=$HERE/dist
WORK=$HERE/work
CACHE=${SYNOFURB_AGENT_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/synofurb-agent}
MIRROR=${DEBIAN_MIRROR:-https://deb.debian.org/debian}
SECMIRROR=${DEBIAN_SECURITY_MIRROR:-https://security.debian.org/debian-security}
DOM_SCRIPT=$HERE/../dom/synofurb-dom
BUDGET_MIB=""   # default per architecture, see below
ALLOW_OVER_BUDGET=0
STRICT_PGP=0
KEEP_WORK=0
XZ_PRESET=${XZ_PRESET:-6}   # -9e gains nothing on this data (measured: 0.05 % smaller, 3x slower)
COMPRESS=xz
EXTRA_MODULES=""
KEYRINGS=()
ARCH=""

die()  { echo "build.sh: ERROR: $*" >&2; exit 1; }
warn() { echo "build.sh: WARNING: $*" >&2; }
info() { echo "build.sh: $*" >&2; }

usage() {
    cat >&2 <<EOF
usage: $0 <amd64|arm64|armhf> [options]
  --suite trixie|trixie-backports   kernel source (default trixie-backports = Linux 7.2.x; trixie = 6.12 LTS)
  --out DIR                         output directory (default $HERE/dist)
  --work DIR                        scratch directory (default $HERE/work)
  --cache DIR                       download cache (default \$XDG_CACHE_HOME/synofurb-agent)
  --budget-mib N                    kernel+initrd budget; default 24 for amd64/armhf, 64 for arm64 (the build FAILS above it)
  --allow-over-budget               write the outputs anyway (still exits non-zero)
  --dom-script FILE                 synofurb-dom to embed (default ../dom/synofurb-dom, else a stub)
  --modules "a b c"                 extra kernel modules (with dependencies) to include
  --compress xz|zstd                initramfs compression (default xz; xz is the tested path)
  --keyring FILE                    extra gpgv keyring for verifying InRelease (repeatable)
  --strict-pgp                      fail if InRelease cannot be PGP-verified (default: warn, hash chain still applies)
  --version V                       agent version string (default $AGENT_VERSION)
  --keep-work                       keep the scratch directory
EOF
    exit "${1:-2}"
}

while [[ $# -gt 0 ]]; do
    case $1 in
        amd64|arm64|armhf) ARCH=$1; shift ;;
        --suite) SUITE=${2:?}; shift 2 ;;
        --out) OUT=$(realpath -m "${2:?}"); shift 2 ;;
        --work) WORK=$(realpath -m "${2:?}"); shift 2 ;;
        --cache) CACHE=$(realpath -m "${2:?}"); shift 2 ;;
        --budget-mib) BUDGET_MIB=${2:?}; shift 2 ;;
        --allow-over-budget) ALLOW_OVER_BUDGET=1; shift ;;
        --dom-script) DOM_SCRIPT=${2:?}; shift 2 ;;
        --modules) EXTRA_MODULES=${2:?}; shift 2 ;;
        --compress) COMPRESS=${2:?}; shift 2 ;;
        --keyring) KEYRINGS+=("${2:?}"); shift 2 ;;
        --strict-pgp) STRICT_PGP=1; shift ;;
        --version) AGENT_VERSION=${2:?}; shift 2 ;;
        --keep-work) KEEP_WORK=1; shift ;;
        -h|--help) usage 0 ;;
        *) echo "unknown argument: $1" >&2; usage ;;
    esac
done
[[ -n $ARCH ]] || usage
case $SUITE in trixie|trixie-backports) ;; *) die "--suite must be trixie or trixie-backports" ;; esac
case $COMPRESS in xz|zstd) ;; *) die "--compress must be xz or zstd" ;; esac

# per-architecture parameters
case $ARCH in
    amd64) FLAVOR=amd64; TRIPLET=x86_64-linux-gnu;       XZ_BCJ="--x86" ;;
    arm64) FLAVOR=arm64; TRIPLET=aarch64-linux-gnu;      XZ_BCJ="" ;;
    armhf) FLAVOR=armmp; TRIPLET=arm-linux-gnueabihf;    XZ_BCJ="" ;;
esac

# The 24 MiB figure is the DOM (x86 internal flash) limit. Debian ships the arm64 kernel as an UNCOMPRESSED Image
# (46.6 MiB for Linux 7.2), which can never fit 24 MiB; arm64 NAS units have no DOM (they boot through U-Boot
# from disk/SPI), so their default budget is 64 MiB. Override with --budget-mib.
if [[ -z $BUDGET_MIB ]]; then
    case $ARCH in arm64) BUDGET_MIB=64 ;; *) BUDGET_MIB=24 ;; esac
fi

for t in curl dpkg-deb xz cpio depmod readelf awk sha256sum sed find tr sort; do
    command -v "$t" >/dev/null 2>&1 || die "missing host tool: $t"
done
[[ $COMPRESS == zstd ]] && { command -v zstd >/dev/null || die "missing host tool: zstd"; }
command -v dpkg >/dev/null || die "missing host tool: dpkg (for --compare-versions)"

# ------------------------------------------------------------------ module selection
# NICs seen in NAS / small-server hardware (spec) + the PHY/MDIO/PTP/CRC helpers (pulled in by modules.dep).
NIC_MODULES="e1000 e1000e igb igc ixgbe ixgbevf i40e r8169 r8152 tg3 bnx2 bnx2x atlantic mlx4_core mlx4_en mlx5_core
sky2 forcedeth alx atl1c atl1e virtio_net vmxnet3 realtek mdio af_packet ipv6"
# The DOM (internal flash module) is reached through USB or SATA, and carries ext2 + FAT partitions:
# synofurb-dom needs those drivers to mount it.
STORAGE_MODULES="loop usb_storage uas sd_mod xhci_pci xhci_hcd ehci_pci ehci_hcd ohci_pci ohci_hcd uhci_hcd ahci libahci
ata_piix ata_generic pata_acpi ext4 ext2 vfat fat nls_cp437 nls_ascii nls_iso8859_1 nls_utf8 crc16 crc32c_generic"
ARM_MODULES="mvneta mvpp2 stmmac stmmac_platform dwmac_* dwc_eth_qos ahci_platform libahci_platform ahci_mvebu sata_mv"
WANTED_MODULES="$NIC_MODULES $STORAGE_MODULES $EXTRA_MODULES"
if [[ $ARCH != amd64 ]]; then WANTED_MODULES="$WANTED_MODULES $ARM_MODULES"; fi
WANTED_MODULES=$(printf '%s' "$WANTED_MODULES" | tr '\n' ' ')

# Debian packages that provide the userland. Their Depends are followed (see SKIP_DEPS); only files that
# the binaries really need (ELF NEEDED entries) are copied.
PKG_ROOTS="busybox-static dropbear-bin kexec-tools e2fsprogs dosfstools netcat-traditional"
SKIP_DEPS='^(debconf|debconf-2\.0|dpkg|logsave|openssl|ca-certificates|init-system-helpers|perl-base|util-linux|gcc-[0-9]+-base|linux-base.*|dracut|initramfs-tools|linux-initramfs-tool|kmod|systemd.*|udev)$'

# ------------------------------------------------------------------ helpers
declare -A SRC_BASE=([trixie]=$MIRROR [trixie-updates]=$MIRROR [trixie-backports]=$MIRROR [trixie-security]=$SECMIRROR)
mkdir -p "$CACHE/idx" "$CACHE/pool" "$OUT"
WORKA=$WORK/$ARCH
rm -rf "$WORKA"; mkdir -p "$WORKA"/{idx,pkgs,kernel,rootfs}
trap '[[ $KEEP_WORK == 1 ]] || rm -rf "$WORKA"' EXIT

fetch_try() { # url dest -> non-zero on failure
    curl -fsSL --retry 3 --retry-delay 2 --connect-timeout 20 -o "$2.part" "$1" 2>/dev/null || { rm -f "$2.part"; return 1; }
    mv "$2.part" "$2"
}
fetch() { fetch_try "$1" "$2" || die "download failed: $1"; }
sha_of() { sha256sum "$1" | cut -d' ' -f1; }
human() { awk -v b="$1" 'BEGIN { printf "%.2f MiB", b / 1048576 }'; }
jstr() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'; }

# verify_release FILE: Valid-Until + PGP (gpgv).
PGP_OK=1
verify_release() {
    local f=$1 vu
    vu=$(sed -n 's/^Valid-Until: //p' "$f" | head -n1)
    if [[ -n $vu ]] && [[ $(date -d "$vu" +%s 2>/dev/null || echo 0) -lt $(date +%s) ]]; then
        die "$f is expired (Valid-Until: $vu) - wrong clock or a stale mirror"
    fi
    if command -v gpgv >/dev/null; then
        local kr=() k out
        for k in "${KEYRINGS[@]}" /usr/share/keyrings/debian-archive-keyring.gpg /usr/share/keyrings/debian-archive-removed-keys.gpg; do
            [[ -f $k ]] && kr+=(--keyring "$k")
        done
        if [[ ${#kr[@]} -gt 0 ]]; then
            out=$(gpgv --status-fd 1 "${kr[@]}" "$f" 2>/dev/null || true)
            if grep -q '^\[GNUPG:\] BADSIG' <<<"$out"; then die "BAD PGP signature on $f"; fi
            if grep -q '^\[GNUPG:\] GOODSIG' <<<"$out"; then return 0; fi
        fi
    fi
    PGP_OK=0
    [[ $STRICT_PGP == 1 ]] && die "cannot PGP-verify $f (install gpgv + debian-archive-keyring, or pass --keyring)"
    return 0
}

release_hash() { # InRelease path -> "sha size"
    awk -v p="$2" '/^SHA256:/{f=1;next} /^[^ ]/{f=0} f && $3==p {print $1, $2; exit}' "$1"
}

# fetch_index DIST: download + verify InRelease and main/binary-ARCH/Packages.xz, convert to TSV
# columns: name version filename sha256 size depends predepends dist   ('-' = empty)
fetch_index() {
    local dist=$1 base=${SRC_BASE[$1]} rel=$WORKA/idx/$1.InRelease want hs sha size xz
    fetch "$base/dists/$dist/InRelease" "$rel"
    verify_release "$rel"
    want="main/binary-$ARCH/Packages.xz"
    hs=$(release_hash "$rel" "$want")
    [[ -n $hs ]] || { warn "no $want in $dist/InRelease - skipping"; : > "$WORKA/idx/$dist.tsv"; return; }
    read -r sha size <<<"$hs"
    xz=$CACHE/idx/$sha.xz
    if [[ ! -f $xz || $(sha_of "$xz") != "$sha" ]]; then
        info "fetching $dist Packages.xz ($(human "$size"))"
        # by-hash first (immune to mirror/CDN skew between InRelease and Packages), plain path as fallback
        fetch_try "$base/dists/$dist/main/binary-$ARCH/by-hash/SHA256/$sha" "$xz" || fetch "$base/dists/$dist/$want" "$xz"
        [[ $(sha_of "$xz") == "$sha" ]] || { rm -f "$xz"; die "SHA256 mismatch for $dist/$want (InRelease says $sha)"; }
    fi
    xz -dc "$xz" | awk -v src="$dist" 'BEGIN { RS = ""; FS = "\n" }
        { delete f
          for (i = 1; i <= NF; i++) { if ($i ~ /^[ \t]/) continue; k = $i; sub(/:.*/, "", k); v = $i; sub(/^[^:]*:[ \t]*/, "", v); f[k] = v }
          if (f["Package"] == "") next
          printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n", f["Package"], f["Version"], (f["Filename"] == "" ? "-" : f["Filename"]),
                 (f["SHA256"] == "" ? "-" : f["SHA256"]), (f["Size"] == "" ? "-" : f["Size"]),
                 (f["Depends"] == "" ? "-" : f["Depends"]), (f["Pre-Depends"] == "" ? "-" : f["Pre-Depends"]), src }' \
        > "$WORKA/idx/$dist.tsv"
    echo "$dist" >> "$WORKA/idx/dists"
}

# pkg_best NAME TSV... -> the TSV line with the highest version
pkg_best() {
    local name=$1; shift
    local best="" bv="" l v
    while IFS= read -r l; do
        v=$(cut -f2 <<<"$l")
        if [[ -z $best ]] || dpkg --compare-versions "$v" gt "$bv"; then best=$l; bv=$v; fi
    done < <(awk -F'\t' -v n="$name" '$1 == n' "$@")
    printf '%s' "$best"
}

# dep_names "Depends" "Pre-Depends" -> package names (first alternative, versions stripped)
dep_names() {
    printf '%s,%s\n' "$1" "$2" | tr ',' '\n' | sed -e 's/|.*//' -e 's/([^)]*)//g' -e 's/\[[^]]*\]//g' \
        -e 's/<[^>]*>//g' -e 's/:[a-z0-9]*$//' -e 's/[[:space:]]//g' | grep -v -e '^$' -e '^-$' || true
}

declare -A PKG_VERSION PKG_SHA PKG_FILE PKG_SRC PKG_SIZE
# fetch_deb NAME TSV... : download + verify + record; prints nothing, sets DEB_PATH
DEB_PATH=""
fetch_deb() {
    local name=$1; shift
    local line n ver fn sha size dep pdep src dest base
    line=$(pkg_best "$name" "$@")
    [[ -n $line ]] || die "package '$name' not found in the $ARCH indexes"
    IFS=$'\t' read -r n ver fn sha size dep pdep src <<<"$line"
    [[ $fn != "-" && $sha != "-" ]] || die "package $name has no Filename/SHA256 in the index"
    dest=$CACHE/pool/$sha-${fn##*/}
    if [[ ! -f $dest || $(sha_of "$dest") != "$sha" ]]; then
        info "downloading $name $ver ($(human "$size"))"
        fetch "${SRC_BASE[$src]}/$fn" "$dest"
        [[ $(sha_of "$dest") == "$sha" ]] || { rm -f "$dest"; die "SHA256 mismatch for $name ($fn); index says $sha"; }
    fi
    PKG_VERSION[$name]=$ver; PKG_SHA[$name]=$sha; PKG_FILE[$name]=$fn; PKG_SRC[$name]=$src; PKG_SIZE[$name]=$size
    DEB_PATH=$dest
}

# ------------------------------------------------------------------ 1. indexes
info "== indexes ($ARCH, kernel suite $SUITE)"
for d in trixie trixie-updates trixie-security; do fetch_index "$d"; done
USER_TSVS=("$WORKA/idx/trixie.tsv" "$WORKA/idx/trixie-updates.tsv" "$WORKA/idx/trixie-security.tsv")
if [[ $SUITE == trixie-backports ]]; then
    fetch_index trixie-backports
    KERNEL_TSVS=("$WORKA/idx/trixie-backports.tsv")
    SDE_REL=$WORKA/idx/trixie-backports.InRelease
else
    KERNEL_TSVS=("${USER_TSVS[@]}")
    SDE_REL=$WORKA/idx/trixie.InRelease
fi
[[ $PGP_OK == 1 ]] || warn "InRelease files were NOT PGP-verified (no gpgv/keyring); integrity rests on HTTPS + the SHA256 chain"
SOURCE_DATE_EPOCH=$(date -d "$(sed -n 's/^Date: //p' "$SDE_REL" | head -n1)" +%s)
export SOURCE_DATE_EPOCH

# ------------------------------------------------------------------ 2. kernel
info "== kernel"
meta=linux-image-$FLAVOR
mline=$(pkg_best "$meta" "${KERNEL_TSVS[@]}")
[[ -n $mline ]] || die "$meta not found in suite $SUITE"
mdeps=$(cut -f6 <<<"$mline")
kabi=$(dep_names "$mdeps" "-" | grep '^linux-image-' | head -n1)
[[ -n $kabi ]] || die "cannot resolve the versioned kernel package from $meta (Depends: $mdeps)"
kline=$(pkg_best "$kabi" "${KERNEL_TSVS[@]}")
[[ -n $kline ]] || die "$kabi not found"
KERNEL_PKGS=("$kabi")
while read -r d; do
    case $d in linux-binary-*|linux-modules-*|linux-image-*) KERNEL_PKGS+=("$d");; esac
done < <(dep_names "$(cut -f6 <<<"$kline")" "$(cut -f7 <<<"$kline")")
info "kernel packages: ${KERNEL_PKGS[*]}"
for p in "${KERNEL_PKGS[@]}"; do
    fetch_deb "$p" "${KERNEL_TSVS[@]}"
    dpkg-deb -x "$DEB_PATH" "$WORKA/kernel"
done
KIMG=$(find "$WORKA/kernel" -type f \( -path '*/boot/vmlinuz-*' -o -name vmlinuz \) | head -n1)
[[ -n $KIMG ]] || die "no vmlinuz found in the kernel packages"
MODROOT=$(find "$WORKA/kernel" -maxdepth 4 -type d -path '*/lib/modules/*' ! -path '*/lib/modules/*/*' | head -n1)
[[ -n $MODROOT ]] || die "no /lib/modules/<release> found in the kernel packages"
KREL=$(basename "$MODROOT")
info "kernel release $KREL, image $(basename "$KIMG") $(human "$(stat -c %s "$KIMG")")"
[[ -e $WORKA/kernel/lib ]] || ln -s usr/lib "$WORKA/kernel/lib"   # depmod -b expects <root>/lib/modules
depmod -b "$WORKA/kernel" -a "$KREL" 2>/dev/null || die "depmod failed on the kernel module tree (kmod without xz support?)"

# dependency closure over modules.dep; names are matched with '-' == '_' and globs
cat > "$WORKA/closure.awk" <<'EOF'
function base(p,  a, n, b) { n = split(p, a, "/"); b = a[n]; sub(/\.ko(\.xz|\.zst|\.gz)?$/, "", b); gsub("-", "_", b); return b }
function add(m,  d, i, nd) {
    if (m in seen) return
    if (!(m in path)) { missing[m] = 1; return }
    seen[m] = 1; nd = split(deps[m], d, " ")
    for (i = 1; i <= nd; i++) add(base(d[i]))
}
BEGIN { FS = ": *" }
{ b = base($1); path[b] = $1; deps[b] = $2 }
END {
    nl = split(wanted, L, /[ \n]+/)
    for (i = 1; i <= nl; i++) {
        w = L[i]; if (w == "") continue; gsub("-", "_", w)
        if (w ~ /[*?]/) { gl = w; gsub(/\*/, ".*", gl); gsub(/\?/, ".", gl); hit = 0; for (m in path) if (m ~ ("^" gl "$")) { add(m); hit = 1 }; if (!hit) missing[w] = 1 }
        else { req[w] = 1; add(w) }
    }
    for (m in seen) print "M " path[m]
    for (m in missing) if (m in req || m ~ /[*?]/) print "X " m
}
EOF
awk -v wanted="$WANTED_MODULES" -f "$WORKA/closure.awk" "$MODROOT/modules.dep" | sort > "$WORKA/closure.txt"
grep '^M ' "$WORKA/closure.txt" | cut -c3- > "$WORKA/modules.sel"
grep '^X ' "$WORKA/closure.txt" | cut -c3- > "$WORKA/modules.builtin_or_absent" || true
[[ -s $WORKA/modules.sel ]] || die "module selection is empty"
info "modules: $(wc -l < "$WORKA/modules.sel") selected (incl. dependencies); not modules in this kernel (built in or absent): $(tr '\n' ' ' < "$WORKA/modules.builtin_or_absent")"

# ------------------------------------------------------------------ 3. userland packages
info "== userland packages"
declare -A SEEN
queue=($PKG_ROOTS)
USER_PKGS=()
while [[ ${#queue[@]} -gt 0 ]]; do
    p=${queue[0]}; queue=("${queue[@]:1}")
    [[ -n ${SEEN[$p]:-} ]] && continue
    SEEN[$p]=1
    line=$(pkg_best "$p" "${USER_TSVS[@]}")
    if [[ -z $line ]]; then
        if [[ $PKG_ROOTS == *"$p"* ]]; then die "package $p not found"; fi
        warn "dependency $p not found (virtual package?) - ignored"; continue
    fi
    USER_PKGS+=("$p")
    while read -r d; do
        [[ $d =~ $SKIP_DEPS ]] && continue
        [[ -z ${SEEN[$d]:-} ]] && queue+=("$d")
    done < <(dep_names "$(cut -f6 <<<"$line")" "$(cut -f7 <<<"$line")")
done
for p in "${USER_PKGS[@]}"; do
    fetch_deb "$p" "${USER_TSVS[@]}"
    mkdir -p "$WORKA/pkgs/$p"
    dpkg-deb -x "$DEB_PATH" "$WORKA/pkgs/$p"
done
info "userland packages: ${USER_PKGS[*]}"

# ------------------------------------------------------------------ 4. assemble the rootfs
info "== rootfs"
ROOT=$WORKA/rootfs
mkdir -p "$ROOT"/{usr/bin,usr/lib,etc,dev,proc,sys,run,tmp,root,var/log,var/run,var/lib/synofurb,mnt}
ln -s usr/bin "$ROOT/bin"; ln -s usr/bin "$ROOT/sbin"; ln -s bin "$ROOT/usr/sbin"; ln -s usr/lib "$ROOT/lib"
chmod 700 "$ROOT/root"; chmod 1777 "$ROOT/tmp"

# our files
cp -a "$HERE/rootfs/." "$ROOT/"
chmod 755 "$ROOT/init" "$ROOT"/usr/bin/* "$ROOT"/usr/lib/synofurb/*
chmod 644 "$ROOT/usr/lib/synofurb/lib.sh"

pkgfile() { # pkg name -> path of the file in the unpacked package
    local f
    f=$(find "$WORKA/pkgs/$1" \( -type f -o -type l \) \( -path '*/bin/*' -o -path '*/sbin/*' \) -name "$2" | head -n1)
    [[ -n $f ]] || die "$2 not found in package $1"
    echo "$f"
}

# busybox + applets
install -m 755 "$(pkgfile busybox-static busybox)" "$ROOT/usr/bin/busybox"
applet_blob() { tr '\0' '\n' < "$1" | awk '
    { l[NR] = $0 }
    END { for (i = 1; i < NR - 2; i++) if (l[i] ~ /\[$/ && l[i+1] == "[[" && l[i+2] != "") { s = i; break }
          if (!s) exit 1
          print "["; for (j = s + 1; j <= NR; j++) { print l[j]; if (l[j] == "zcat") break } }'; }
applet_blob "$ROOT/usr/bin/busybox" | sort -u > "$WORKA/applets.all" || true
napplets=$(wc -l < "$WORKA/applets.all")
(( napplets > 100 )) || die "could not read the applet table from busybox ($napplets entries)"
for a in sh ash init mount umount modprobe insmod udhcpc udhcpc6 ip wget sha256sum sync reboot poweroff halt timeout \
         xargs find awk sed grep cut tr sort head tail cat ls cp mv rm mkdir ln chmod ps kill sleep hostname dmesg \
         lsmod uname tar gzip xz stat nslookup ifconfig route cttyhack setsid; do
    grep -qx -- "$a" "$WORKA/applets.all" || die "busybox lacks the required applet '$a'"
done
grep -vxE 'busybox|dpkg|dpkg-deb|rpm|rpm2cpio|nuke|linuxrc|ssl_client|ubirename|\[\[' "$WORKA/applets.all" > "$WORKA/applets.link"
while read -r a; do
    [[ $a == busybox ]] && continue
    ln -sf busybox "$ROOT/usr/bin/$a"
done < "$WORKA/applets.link"
ln -sf busybox "$ROOT/usr/bin/[["   # keep the bash-style test, harmless
info "busybox: $napplets applets in the binary, $(wc -l < "$WORKA/applets.link") linked"

# other binaries
ELFS=()
inst_bin() { # pkg srcname destname
    install -m 755 "$(pkgfile "$1" "$2")" "$ROOT/usr/bin/${3:-$2}"
    ELFS+=("$ROOT/usr/bin/${3:-$2}")
}
inst_bin dropbear-bin dropbear
inst_bin dropbear-bin dropbearkey
inst_bin kexec-tools kexec
inst_bin e2fsprogs e2fsck
for n in fsck.ext2 fsck.ext3 fsck.ext4; do ln -sf e2fsck "$ROOT/usr/bin/$n"; done
inst_bin dosfstools fsck.fat
ln -sf fsck.fat "$ROOT/usr/bin/fsck.vfat"; ln -sf fsck.fat "$ROOT/usr/bin/dosfsck"
inst_bin netcat-traditional nc.traditional

# shared libraries: exactly what the ELF NEEDED entries (recursively) ask for
declare -A LIBIDX LIB_DONE
while IFS= read -r p; do b=${p##*/}; [[ -n ${LIBIDX[$b]:-} ]] || LIBIDX[$b]=$p
done < <(find "$WORKA/pkgs" \( -type f -o -type l \) \( -name '*.so' -o -name '*.so.*' \))
elf_needed() { readelf -d "$1" 2>/dev/null | sed -n 's/.*Shared library: \[\(.*\)\].*/\1/p'; }
elf_interp() { readelf -l "$1" 2>/dev/null | sed -n 's/.*Requesting program interpreter: \(.*\)\]/\1/p'; }
LIBS_USED=()
install_deps_of() {
    local f=$1 so path real
    while read -r so; do
        [[ -n $so ]] || continue
        [[ -n ${LIB_DONE[$so]:-} ]] && continue
        LIB_DONE[$so]=1
        path=${LIBIDX[$so]:-}
        [[ -n $path ]] || die "shared library $so (needed by ${f##*/}) is in none of the downloaded packages"
        real=$(readlink -f "$path")
        [[ $real == "$WORKA/pkgs/"* && -f $real ]] || die "library $path resolves outside the unpacked package ($real)"
        install -D -m 755 "$real" "$ROOT/usr/lib/$TRIPLET/$so"
        LIBS_USED+=("$so")
        install_deps_of "$real"
    done < <(elf_needed "$f")
}
INTERPS=()
for f in "${ELFS[@]}"; do
    install_deps_of "$f"
    i=$(elf_interp "$f"); [[ -n $i ]] && INTERPS+=("$i")
done
# the program interpreter must exist at the exact path baked into the binaries
for i in $(printf '%s\n' "${INTERPS[@]}" | sort -u); do
    ldso=${i##*/}
    [[ -e $ROOT/usr/lib/$TRIPLET/$ldso ]] || die "dynamic loader $ldso missing (libc6 not pulled in?)"
    dir=$ROOT/usr$(dirname "$i")           # /lib64 -> usr/lib64, /lib -> usr/lib (usr-merge)
    mkdir -p "$dir"
    ln -sfn "$(realpath -m --relative-to="$dir" "$ROOT/usr/lib/$TRIPLET/$ldso")" "$dir/$ldso"
done
[[ -e $ROOT/lib64 || ! -d $ROOT/usr/lib64 ]] || ln -s usr/lib64 "$ROOT/lib64"
info "shared libraries: ${LIBS_USED[*]}"

# synofurb-dom
DOM_STUB=0
if [[ -f $DOM_SCRIPT ]]; then
    install -m 755 "$DOM_SCRIPT" "$ROOT/usr/bin/synofurb-dom"
    shebang=$(head -n1 "$DOM_SCRIPT")
    case $shebang in
        '#!/bin/sh'*|'#!/usr/bin/env sh'*|'#!/bin/ash'*|'#!/bin/busybox'*) ;;
        *) warn "synofurb-dom starts with '$shebang' - the agent has busybox ash only (no bash/python)" ;;
    esac
    info "embedding synofurb-dom from $DOM_SCRIPT"
else
    DOM_STUB=1
    warn "================================================================================"
    warn "$DOM_SCRIPT not found: embedding a STUB synofurb-dom."
    warn "The agent will NOT be able to set the DOM boot default; its fail-safe (boot DSM"
    warn "again) then only works if the DOM component makes the agent a one-shot boot."
    warn "Do not ship this build. Rebuild once boot/dom/synofurb-dom exists."
    warn "================================================================================"
    cat > "$ROOT/usr/bin/synofurb-dom" <<'EOF'
#!/bin/sh
# STUB (placeholder generated by build.sh because boot/dom/synofurb-dom was missing at build time).
# TODO: replace with the real synofurb-dom from the DOM component and rebuild.
echo "synofurb-dom STUB: cannot run 'synofurb-dom $*' - the real script was not included in this agent build" >&2
exit 64
EOF
    chmod 755 "$ROOT/usr/bin/synofurb-dom"
fi

# release file
BUILD_DATE=$(date -u -d "@$SOURCE_DATE_EPOCH" +%Y-%m-%dT%H:%M:%SZ)
cat > "$ROOT/etc/synofurb-agent-release" <<EOF
AGENT_VERSION=$AGENT_VERSION
AGENT_ARCH=$ARCH
AGENT_KERNEL=$KREL
AGENT_SUITE=$SUITE
AGENT_BUILT=$BUILD_DATE
AGENT_DOM_STUB=$DOM_STUB
EOF

# kernel modules: decompress (busybox modprobe/insmod cannot read .ko.xz), depmod
MODDEST=$ROOT/usr/lib/modules/$KREL
while read -r m; do
    d=$MODDEST/$(dirname "$m")
    mkdir -p "$d"
    case $m in
        *.xz) xz -dc "$MODROOT/$m" > "$d/$(basename "${m%.xz}")" ;;
        *.zst) zstd -dc "$MODROOT/$m" > "$d/$(basename "${m%.zst}")" ;;
        *.gz) gzip -dc "$MODROOT/$m" > "$d/$(basename "${m%.gz}")" ;;
        *) cp "$MODROOT/$m" "$d/" ;;
    esac
done < "$WORKA/modules.sel"
depmod -b "$ROOT" -a "$KREL" 2>/dev/null || die "depmod on the final module set failed"
find "$MODDEST" -maxdepth 1 -type f ! -name modules.dep ! -name modules.alias -delete
MODULES_UNCOMP=$(du -sb "$MODDEST" | cut -f1)
cp "$KIMG" "$WORKA/agent.vmlinuz"
rm -rf "$WORKA/kernel"          # ~160-540 MB of unpacked modules, no longer needed

# ------------------------------------------------------------------ 5. initramfs archive
info "== initramfs"
find "$ROOT" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
INITRD_TMP=$WORKA/initrd.cpio
( cd "$ROOT" && find . -mindepth 1 -print0 | LC_ALL=C sort -z | cpio --null -o -H newc -R 0:0 --quiet ) > "$INITRD_TMP"
CPIO_SIZE=$(stat -c %s "$INITRD_TMP")

# /dev/console + /dev/null as real device nodes in a tiny uncompressed first archive (no root needed to build it)
newc_dev_archive() {
    local ino=900000
    entry() { # name mode rdevmajor rdevminor
        local name=$1 mode=$2 maj=$3 min=$4 nlink=1 namesz pad
        namesz=$(( ${#name} + 1 ))
        if (( (mode & 0170000) == 0040000 )); then nlink=2; fi
        printf '070701%08X%08X%08X%08X%08X%08X%08X%08X%08X%08X%08X%08X%08X' \
            "$ino" "$mode" 0 0 "$nlink" "$SOURCE_DATE_EPOCH" 0 0 0 "$maj" "$min" "$namesz" 0
        printf '%s\0' "$name"
        pad=$(( (4 - (110 + namesz) % 4) % 4 ))
        if (( pad > 0 )); then head -c "$pad" /dev/zero; fi
        ino=$((ino + 1))
    }
    entry dev $((8#40755)) 0 0
    entry dev/console $((8#20600)) 5 1
    entry dev/null $((8#20666)) 1 3
    ino=0; entry TRAILER!!! 0 0 0
}
newc_dev_archive > "$WORKA/dev.cpio"

compress_cpio() {
    case $COMPRESS in
        xz) xz -c --check=crc32 $XZ_BCJ --lzma2=preset=$XZ_PRESET,dict=16MiB < "$1" ;;
        zstd) zstd -q -19 -c < "$1" ;;
    esac
}
{ cat "$WORKA/dev.cpio"; compress_cpio "$INITRD_TMP"; } > "$WORKA/agent.initrd"

KSIZE=$(stat -c %s "$WORKA/agent.vmlinuz")
ISIZE=$(stat -c %s "$WORKA/agent.initrd")
TOTAL=$((KSIZE + ISIZE))
BUDGET=$((BUDGET_MIB * 1048576))

# ------------------------------------------------------------------ 6. report
comp_size() { # bytes of a directory/file set when compressed alone with xz -9 (rough share of the final initrd)
    tar -C "$ROOT" -cf - "$@" 2>/dev/null | xz -6 -c 2>/dev/null | wc -c
}
cat <<EOF >&2

==================== size report: $ARCH ($SUITE, kernel $KREL) ====================
  kernel  agent-$ARCH.vmlinuz       $(printf '%10d' "$KSIZE")  $(human "$KSIZE")
  initrd  agent-$ARCH.initrd        $(printf '%10d' "$ISIZE")  $(human "$ISIZE")   (cpio $(human "$CPIO_SIZE") uncompressed, $COMPRESS)
  total                             $(printf '%10d' "$TOTAL")  $(human "$TOTAL")   budget $BUDGET_MIB MiB ($(human "$((BUDGET - TOTAL))") free)
  initrd contents, compressed on their own (xz -6), for orientation:
      kernel modules ($(wc -l < "$WORKA/modules.sel") files, $(human "$MODULES_UNCOMP") raw)  $(human "$(comp_size usr/lib/modules)")
      busybox                                  $(human "$(comp_size usr/bin/busybox)")
      shared libraries                         $(human "$(comp_size usr/lib/$TRIPLET)")
      dropbear/kexec/e2fsck/fsck.fat/nc        $(human "$(comp_size usr/bin/dropbear usr/bin/dropbearkey usr/bin/kexec usr/bin/e2fsck usr/bin/fsck.fat usr/bin/nc.traditional)")
      scripts + config                         $(human "$(comp_size usr/lib/synofurb usr/bin/sf-boot init etc)")
================================================================================
EOF

if (( TOTAL > BUDGET )); then
    echo "build.sh: ERROR: kernel + initrd = $(human "$TOTAL") exceeds the $BUDGET_MIB MiB budget by $(human "$((TOTAL - BUDGET))")." >&2
    echo "  The DOM partition is small and DSM's own files share it. Remove modules (edit NIC_MODULES/STORAGE_MODULES," >&2
    echo "  or try --suite trixie for the smaller 6.12 kernel), or raise --budget-mib if you know the space is there." >&2
    if [[ $ALLOW_OVER_BUDGET != 1 ]]; then exit 1; fi
    warn "--allow-over-budget: writing the outputs anyway"
fi

# ------------------------------------------------------------------ 7. outputs
cp "$WORKA/agent.vmlinuz" "$OUT/agent-$ARCH.vmlinuz"
cp "$WORKA/agent.initrd" "$OUT/agent-$ARCH.initrd"
( cd "$ROOT" && find . -mindepth 1 | LC_ALL=C sort | while read -r f; do
      if [[ -L $f ]]; then printf 'l %s -> %s\n' "${f#./}" "$(readlink "$f")"
      elif [[ -d $f ]]; then printf 'd %s\n' "${f#./}"
      else printf 'f %9d %s\n' "$(stat -c %s "$f")" "${f#./}"; fi
  done ) > "$OUT/initrd-$ARCH.lst"
( cd "$OUT" && { [[ -f SHA256SUMS ]] && grep -v " agent-$ARCH\\." SHA256SUMS || true; sha256sum "agent-$ARCH.vmlinuz" "agent-$ARCH.initrd"; } | LC_ALL=C sort -k2 > SHA256SUMS.new && mv SHA256SUMS.new SHA256SUMS )

join_json() { local first=1 x; printf '['; for x in "$@"; do [[ $first == 1 ]] || printf ', '; first=0; printf '"%s"' "$(jstr "$x")"; done; printf ']'; }
{
    echo "{"
    echo "  \"agent_version\": \"$(jstr "$AGENT_VERSION")\","
    echo "  \"arch\": \"$ARCH\","
    echo "  \"suite\": \"$SUITE\","
    echo "  \"built\": \"$BUILD_DATE\","
    echo "  \"compression\": \"$COMPRESS\","
    echo "  \"dom_stub\": $([[ $DOM_STUB == 1 ]] && echo true || echo false),"
    echo "  \"pgp_verified\": $([[ $PGP_OK == 1 ]] && echo true || echo false),"
    echo "  \"kernel\": { \"release\": \"$(jstr "$KREL")\", \"packages\": $(join_json "${KERNEL_PKGS[@]}"), \"file\": \"agent-$ARCH.vmlinuz\", \"bytes\": $KSIZE },"
    echo "  \"initrd\": { \"file\": \"agent-$ARCH.initrd\", \"bytes\": $ISIZE, \"cpio_bytes\": $CPIO_SIZE },"
    echo "  \"total_bytes\": $TOTAL,"
    echo "  \"budget_bytes\": $BUDGET,"
    echo "  \"busybox_applets\": $napplets,"
    echo "  \"modules\": $(join_json $(sed 's|.*/||; s|\.ko.*||' "$WORKA/modules.sel" | sort)),"
    echo "  \"modules_not_modules_in_this_kernel\": $(join_json $(cat "$WORKA/modules.builtin_or_absent")),"
    echo "  \"libraries\": $(join_json "${LIBS_USED[@]}"),"
    echo "  \"packages\": ["
    first=1
    for p in "${KERNEL_PKGS[@]}" "${USER_PKGS[@]}"; do
        [[ -n ${PKG_VERSION[$p]:-} ]] || continue
        [[ $first == 1 ]] || echo ","
        first=0
        printf '    { "name": "%s", "version": "%s", "sha256": "%s", "source": "%s/%s" }' \
            "$(jstr "$p")" "$(jstr "${PKG_VERSION[$p]}")" "${PKG_SHA[$p]}" "${SRC_BASE[${PKG_SRC[$p]}]}" "${PKG_FILE[$p]}"
    done
    echo
    echo "  ]"
    echo "}"
} > "$OUT/MANIFEST-$ARCH.json"

info "wrote $OUT/agent-$ARCH.vmlinuz $OUT/agent-$ARCH.initrd (+ SHA256SUMS, MANIFEST-$ARCH.json, initrd-$ARCH.lst)"
(( TOTAL > BUDGET )) && exit 1
exit 0
