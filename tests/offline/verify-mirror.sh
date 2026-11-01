#!/bin/sh
# shellcheck disable=SC2086,SC2002,SC2012,SC2010,SC2016
# Proves that an offline bundle, served by synofurb's mirror route, is complete by running the REAL tools on it.
#
#   verify-mirror.sh MODE ARCH BASE_URL WORKDIR CHROOT_SETUP KERNEL SUITE [EXTRA_PACKAGES]
#
#   MODE      chroot   debootstrap + the real payload/chroot-setup.sh package steps inside a chroot (root, host arch only)
#             foreign  debootstrap --foreign (downloads + unpacks) and an apt simulation of every apt step; any arch
#   BASE_URL  http://127.0.0.1:PORT/<token>/mirror/debian   (the served bundle; nothing else is reachable)
#   KERNEL    newest | stable
#
# The only Internet access is a one-off HTTPS download of Debian's own archive keyring (the trust anchor for the
# signature check of the bundle's InRelease). Exits non-zero on the first problem.
set -eu

MODE=$1 ARCH=$2 URL=$3 WORK=$4 SCRIPT=$5 KERNEL_CHANNEL=$6 SUITE=$7 EXTRA=${8:-}
HERE=$(cd "$(dirname "$0")" && pwd)
HOSTPORT=${URL#http://}; HOSTPORT=${HOSTPORT%%/*}
DIR=/${URL#http://*/}                       # /<token>/mirror/debian
T=$WORK/target-$ARCH
A=$WORK/apt-$ARCH
LOG=$WORK/log-$ARCH.txt
mkdir -p "$WORK"
say() { printf '\n==== [%s %s] %s\n' "$MODE" "$ARCH" "$*"; }
fail() { printf '\nFAILED [%s %s]: %s\n' "$MODE" "$ARCH" "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------------------------------------------
# mounts: never delete a tree that still has bind mounts
# ---------------------------------------------------------------------------------------------------------------
mounted_below() { awk -v p="$1/" 'index($2, p) == 1 || $2 == substr(p, 1, length(p)-1) { print $2 }' /proc/mounts; }
cleanup() {
  if [ -d "$T" ]; then
    for m in $(mounted_below "$T" | sort -r); do umount "$m" 2>/dev/null || umount -l "$m" 2>/dev/null || true; done
    if [ -n "$(mounted_below "$T")" ]; then
      echo "WARNING: $T still has mounts, NOT deleting it:" >&2; mounted_below "$T" >&2
      return
    fi
    [ "${KEEP:-0}" = 1 ] || rm -rf "$T"
  fi
  [ "${KEEP:-0}" = 1 ] || rm -rf "$A"
}
trap cleanup EXIT INT TERM
if [ -d "$T" ]; then [ -z "$(mounted_below "$T")" ] || fail "$T has stale mounts from an earlier run; unmount them first"; rm -rf "$T"; fi
rm -rf "$A"; mkdir -p "$A"

http_ok() { curl -sfI -o /dev/null --max-time 30 "$1"; }

# ---------------------------------------------------------------------------------------------------------------
# 1. signature chain: Debian's own keyring (fetched from the official site over HTTPS, NOT from the bundle) must
#    verify the InRelease the bundle serves
# ---------------------------------------------------------------------------------------------------------------
say "InRelease signatures against Debian's archive keyring"
mkdir -p "$A/kr" "$A/inrel"
curl -sf --max-time 120 -o "$A/Packages.gz" "$URL/dists/$SUITE/main/binary-all/Packages.gz" || fail "cannot read the served index"
KDEB=$(gzip -dc "$A/Packages.gz" | awk -v RS= '/^Package: debian-archive-keyring\n/ { for (i=1;i<=NF;i++) if ($i=="Filename:") print $(i+1) }')
[ -n "$KDEB" ] || fail "debian-archive-keyring not in the served index"
curl -sf --max-time 120 -o "$A/kr.deb" "https://deb.debian.org/debian/$KDEB" || fail "cannot download the keyring from deb.debian.org (needed as an independent trust anchor)"
dpkg-deb -x "$A/kr.deb" "$A/kr"
KR=$A/kr/usr/share/keyrings/debian-archive-keyring.pgp
[ -f "$KR" ] || KR=$A/kr/usr/share/keyrings/debian-archive-keyring.gpg
[ -f "$KR" ] || fail "no keyring in debian-archive-keyring"
for s in "$SUITE" "$SUITE-backports"; do
  curl -sf -o "$A/inrel/$s" "$URL/dists/$s/InRelease" || { [ "$s" = "$SUITE-backports" ] && [ "$KERNEL_CHANNEL" != newest ] && continue; fail "InRelease of $s not served"; }
  gpgv --keyring "$KR" "$A/inrel/$s" 2>&1 | sed 's/^/    /' || true
  gpgv --keyring "$KR" "$A/inrel/$s" >/dev/null 2>&1 || fail "InRelease of $s is NOT validly signed by Debian's keyring"
  echo "  $s/InRelease: good signature"
  if curl -sf -o "$A/inrel/$s.Release" "$URL/dists/$s/Release" && curl -sf -o "$A/inrel/$s.Release.gpg" "$URL/dists/$s/Release.gpg"; then
    gpgv --keyring "$KR" "$A/inrel/$s.Release.gpg" "$A/inrel/$s.Release" >/dev/null 2>&1 || fail "Release.gpg of $s does not verify"
    echo "  $s/Release + Release.gpg: good signature"
  fi
done

# ---------------------------------------------------------------------------------------------------------------
# 2. what the script installs, taken from the script text itself
# ---------------------------------------------------------------------------------------------------------------
JOINED=$(sed -e ':a' -e '/\\$/N; s/\\\n/ /; ta' "$SCRIPT")
words_of() { printf '%s\n' "$JOINED" | sed -n "s/^[[:space:]]*apt_install \(.*\)\$/\1/p" | tr ' ' '\n' | grep -v '^\$' | grep -v '^"' | grep -v '^$' || true; }
ALL_BOOT=$(printf '%s\n' "$JOINED" | sed -n 's/.*BOOTPKG="\([^"]*\)".*/\1/p' | tr ' ' '\n' | sort -u)
# Every firmware type installs its own bootloader (grub-pc and grub-efi-amd64 conflict), so they are checked one by one.
case "$ARCH" in
  amd64) KERNEL=linux-image-amd64; ARCH_BOOT="grub-pc grub-efi-amd64 efibootmgr"
         GROUPS_LIST="grub-pc|grub-efi-amd64 efibootmgr|intel-microcode|amd64-microcode" ;;
  arm64) KERNEL=linux-image-arm64; ARCH_BOOT="grub-efi-arm64 efibootmgr u-boot-tools"
         GROUPS_LIST="grub-efi-arm64 efibootmgr|u-boot-tools" ;;
  armhf) KERNEL=linux-image-armmp; ARCH_BOOT="grub-efi-arm efibootmgr u-boot-tools"
         GROUPS_LIST="grub-efi-arm efibootmgr|u-boot-tools|haveged" ;;
  *) fail "unknown arch $ARCH" ;;
esac
FIRST_GROUP=${GROUPS_LIST%%|*}
for b in $ARCH_BOOT; do printf '%s\n' "$ALL_BOOT" | grep -qx "$b" || fail "BOOTPKG of the script does not know $b"; done
TOOLS=$(words_of | sort -u | tr '\n' ' ')
echo "packages named in apt_install lines of the script: $(printf '%s' "$TOOLS" | wc -w)"

# ---------------------------------------------------------------------------------------------------------------
# 3. debootstrap exactly as the generated stage-2 script calls it
# ---------------------------------------------------------------------------------------------------------------
say "debootstrap --variant=minbase --include=ca-certificates,locales,dbus ($ARCH) from $URL"
FOREIGN=""; [ "$MODE" = foreign ] && FOREIGN=--foreign
debootstrap $FOREIGN --arch="$ARCH" --variant=minbase --components=main,contrib,non-free-firmware --keyring="$KR" \
  --include=ca-certificates,locales,dbus "$SUITE" "$T" "$URL" > "$LOG" 2>&1 || { tail -40 "$LOG" >&2; fail "debootstrap failed (see $LOG)"; }
tail -3 "$LOG"
echo "  debootstrap OK, $(du -sm "$T" | cut -f1) MiB unpacked"

# ---------------------------------------------------------------------------------------------------------------
# 4a. chroot mode: run the real package steps of payload/chroot-setup.sh
# ---------------------------------------------------------------------------------------------------------------
if [ "$MODE" = chroot ]; then
  [ "$(id -u)" = 0 ] || fail "chroot mode needs root"
  say "real chroot-setup.sh (package steps) against the offline mirror only"
  mount --bind /dev "$T/dev"; mount -t devpts devpts "$T/dev/pts"; mount -t proc proc "$T/proc"; mount -t sysfs sysfs "$T/sys"
  printf '#!/bin/sh\nexit 101\n' > "$T/usr/sbin/policy-rc.d"; chmod 755 "$T/usr/sbin/policy-rc.d"   # no daemons in a test chroot
  P=$T/opt/synofurb-install
  mkdir -p "$P"
  # up to (not including) the storage/users/network steps: hostname, apt sources, locale, kernel, boot + tool packages
  sed '/^# --- storage config/,$d' "$SCRIPT" > "$P/chroot-setup-head.sh"
  # the closing block of the offline branch (official sources) runs separately below
  {
    printf '%s\n' 'set -eu' '. /opt/synofurb-install/env.sh' 'say() { echo "[chroot] $*"; }' 'KR=/usr/share/keyrings/debian-archive-keyring.pgp'
    printf '%s\n' '[ -f "$KR" ] || KR=/usr/share/keyrings/debian-archive-keyring.gpg'
    awk 'prev ~ /^if \[ "\$OFFLINE" = 1 \]; then$/ && /The install is done/ { cap = 1; print prev } cap { print } cap && /^fi$/ { exit } { prev = $0 }' "$SCRIPT"
  } > "$P/chroot-setup-tail.sh"
  grep -q 'deb.debian.org/debian-security' "$P/chroot-setup-tail.sh" || fail "could not extract the closing offline block of the script"
  cat > "$P/env.sh" <<ENV
NAS_HOSTNAME='testnas'
TIMEZONE='UTC'
LOCALE='en_US.UTF-8'
KEYMAP='us'
MIRROR_HOST='$HOSTPORT'
MIRROR_DIR='$DIR'
SUITE='$SUITE'
ARCH='$ARCH'
FIRMWARE='Bios'
KERNEL_CHANNEL='$KERNEL_CHANNEL'
MICROCODE_PKG='intel-microcode'
DOM_BOOT='0'
EXTRA_PACKAGES='$EXTRA'
OFFLINE='1'
OFFLINE_SUITES='$SUITE$( [ "$KERNEL_CHANNEL" = newest ] && printf ' %s-backports' "$SUITE")'
ENV
  CH="env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin HOME=/root chroot $T"
  $CH sh /opt/synofurb-install/chroot-setup-head.sh > "$WORK/chroot-$ARCH.log" 2>&1 || { tail -60 "$WORK/chroot-$ARCH.log" >&2; fail "chroot-setup.sh package steps failed (see $WORK/chroot-$ARCH.log)"; }
  grep -E '^\[chroot\]' "$WORK/chroot-$ARCH.log" | sed 's/^/  /' | head -40
  grep -q 'WARNING: kernel from' "$WORK/chroot-$ARCH.log" && fail "the kernel fell back to the stable kernel: the backports kernel could not be installed"
  echo "  package steps OK"

  say "what the install-time apt sources looked like"
  cat "$T/etc/apt/sources.list.d/debian.sources" | sed 's/^/    /'
  grep -q "127.0.0.1" "$T/etc/apt/sources.list.d/debian.sources" || fail "apt does not use the local mirror"
  grep -qE 'deb\.debian\.org|security' "$T/etc/apt/sources.list.d/debian.sources" && fail "install-time sources mention an online mirror"
  [ -f "$T/etc/apt/apt.conf.d/99synofurb-offline" ] || fail "no temporary offline apt settings"

  say "installed state"
  ls "$T"/boot/vmlinuz-* "$T"/boot/initrd.img-* | sed 's/^/    /'
  if [ "$KERNEL_CHANNEL" = newest ]; then ls "$T"/boot/vmlinuz-* | grep -q 'vmlinuz-7\.' || fail "no 7.x kernel installed"; fi
  $CH dpkg -l | awk '/^ii/ { n++ } END { print "    " n " packages installed" }'
  $CH dpkg --audit | head -5
  $CH apt-get check >/dev/null 2>&1 || fail "apt-get check reports a broken system"
  for p in $TOOLS; do $CH dpkg -s "$p" >/dev/null 2>&1 || fail "$p from the script's package list is not installed"; done
  echo "  every package named in the script's apt_install lines is installed"

  say "other firmware types: bootloader / microcode packages resolve and are all served"
  OLDIFS=$IFS; IFS='|'
  for g in $GROUPS_LIST; do
    IFS=$OLDIFS
    $CH apt-get -y --no-install-recommends --print-uris install $g 2>/dev/null | sed -n "s/^'\([^']*\)'.*/\1/p" > "$A/uris-extra.txt" || true
    $CH apt-get -s -y --no-install-recommends install $g > "$A/sim-extra.txt" 2>&1 || { cat "$A/sim-extra.txt" >&2; fail "cannot resolve '$g' from the mirror"; }
    n=0
    while read -r u; do http_ok "$u" || fail "not served: $u"; n=$((n+1)); done < "$A/uris-extra.txt"
    echo "  '$g': resolves; $n more package files, all served"
    IFS='|'
  done
  IFS=$OLDIFS

  say "closing block of the script: official sources for the installed system"
  $CH sh /opt/synofurb-install/chroot-setup-tail.sh
  cat "$T/etc/apt/sources.list.d/debian.sources" | sed 's/^/    /'
  grep -q 'deb.debian.org/debian-security' "$T/etc/apt/sources.list.d/debian.sources" || fail "official security source missing"
  grep -q "$SUITE-updates" "$T/etc/apt/sources.list.d/debian.sources" || fail "official updates source missing"
  grep -q '127.0.0.1' "$T/etc/apt/sources.list.d/debian.sources" && fail "the local mirror is still in the installed system's sources"
  [ ! -e "$T/etc/apt/apt.conf.d/99synofurb-offline" ] || fail "temporary offline apt settings were not removed"
  if [ "$KERNEL_CHANNEL" = newest ]; then grep -q "$SUITE-backports" "$T/etc/apt/sources.list.d/debian.sources" || fail "backports source missing for the newest kernel"; fi
  echo "  OK"
  say "PASS chroot $ARCH"
  exit 0
fi

# ---------------------------------------------------------------------------------------------------------------
# 4b. foreign mode: simulate the apt steps with apt's own resolver on a synthetic dpkg status
# ---------------------------------------------------------------------------------------------------------------
say "apt simulation of chroot-setup.sh for $ARCH against the offline mirror"
SRC="deb [signed-by=$KR] http://$HOSTPORT$DIR $SUITE main contrib non-free-firmware"
[ "$KERNEL_CHANNEL" = newest ] && SRC="$SRC
deb [signed-by=$KR] http://$HOSTPORT$DIR $SUITE-backports main contrib non-free-firmware"
mkdir -p "$A/state/lists/partial" "$A/cache/archives/partial"
printf '%s\n' "$SRC" > "$A/sources.list"
: > "$A/status"
cat > "$A/apt.conf" <<CONF
Dir::State "$A/state";
Dir::State::status "$A/status";
Dir::Cache "$A/cache";
Dir::Etc::sourcelist "$A/sources.list";
Dir::Etc::sourceparts "/dev/null";
Dir::Etc::preferences "/dev/null";
Dir::Etc::preferencesparts "/dev/null";
APT::Architecture "$ARCH";
APT::Architectures "$ARCH";
Acquire::Languages "none";
Acquire::Check-Valid-Until "false";
Acquire::Check-Date "false";
Acquire::http::Proxy "false";
APT::Install-Recommends "false";
APT::Get::Assume-Yes "true";
Debug::NoLocking "true";
CONF
APTG="env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin APT_CONFIG=$A/apt.conf apt-get"
$APTG update > "$A/update.txt" 2>&1 || { cat "$A/update.txt" >&2; fail "apt-get update against the offline mirror failed"; }
grep -E '^(Err|E:)' "$A/update.txt" && fail "apt-get update reported errors"
tail -3 "$A/update.txt" | sed 's/^/  /'
APTC="env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin APT_CONFIG=$A/apt.conf apt-cache"
$APTC dumpavail > "$A/avail.txt"

# step A = debootstrap set + every apt_install except the kernel
REQUIRED=$(awk -v RS= '/\nPriority: required\n/ || /^Priority: required\n/ || /\nEssential: yes\n/ { for (i=1;i<=NF;i++) if ($i=="Package:") print $(i+1) }' "$A/avail.txt" | sort -u | tr '\n' ' ')
STEP_A="$REQUIRED apt usr-is-merged ca-certificates locales dbus debian-archive-keyring tzdata debconf locales initramfs-tools $FIRST_GROUP $TOOLS $EXTRA"
$APTG -s install $STEP_A > "$A/simA.txt" 2>&1 || { tail -30 "$A/simA.txt" >&2; fail "apt cannot resolve step A (base + tools) from the offline mirror"; }
$APTG --print-uris install $STEP_A 2>/dev/null | sed -n "s/^'\([^']*\)'.*/\1/p" > "$A/urisA.txt"
sed -n 's/^Inst \([^ ]*\) .*/\1/p' "$A/simA.txt" > "$A/namesA.txt"
echo "  step A: apt would install $(wc -l < "$A/namesA.txt") packages"
$(command -v python3) -I "$HERE/mkstatus.py" "$A/avail.txt" "$A/namesA.txt" > "$A/status"

# step B = the kernel on top of step A (apt-get install -t <suite>-backports, as the script does)
if [ "$KERNEL_CHANNEL" = newest ]; then TARGET_REL="-t $SUITE-backports"; else TARGET_REL=""; fi
$APTG -s $TARGET_REL install "$KERNEL" > "$A/simB.txt" 2>&1 || { tail -30 "$A/simB.txt" >&2; fail "apt cannot resolve the kernel step"; }
$APTG --print-uris $TARGET_REL install "$KERNEL" 2>/dev/null | sed -n "s/^'\([^']*\)'.*/\1/p" > "$A/urisB.txt"
echo "  step B: kernel step would install: $(sed -n 's/^Inst \([^ ]*\) (\([^ ]*\) .*/\1 \2/p' "$A/simB.txt" | tr '\n' ',')"
if [ "$KERNEL_CHANNEL" = newest ]; then grep -qE '^Inst linux-image-[0-9]+\.[0-9]+.*bpo' "$A/simB.txt" || fail "the kernel step does not select a backports kernel"; fi

# step C = every other firmware type's bootloader / microcode, each on top of step A
: > "$A/urisC.txt"
OLDIFS=$IFS; IFS='|'
for g in $GROUPS_LIST; do
  IFS=$OLDIFS
  $APTG -s install $g > "$A/simC.txt" 2>&1 || { tail -30 "$A/simC.txt" >&2; fail "apt cannot resolve '$g'"; }
  $APTG --print-uris install $g 2>/dev/null | sed -n "s/^'\([^']*\)'.*/\1/p" >> "$A/urisC.txt"
  IFS='|'
done
IFS=$OLDIFS

# every URI apt would download must be served
sort -u "$A/urisA.txt" "$A/urisB.txt" "$A/urisC.txt" > "$A/uris.txt"
n=0
while read -r u; do
  http_ok "$u" || fail "apt would download $u but the offline mirror does not serve it"
  n=$((n+1))
done < "$A/uris.txt"
echo "  all $n package files apt would download are served"

# the debootstrap --foreign above already downloaded + verified + unpacked the base for this architecture
[ -d "$T/usr" ] || fail "debootstrap --foreign produced no root file system"
say "PASS foreign $ARCH"
