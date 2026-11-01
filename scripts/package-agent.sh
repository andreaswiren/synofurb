#!/usr/bin/env bash
# Boot agent assets for one architecture, renamed to the contract:
#   synofurb-agent-<amd64|arm64|armhf>-<version>.vmlinuz  /  .initrd
#
#   scripts/package-agent.sh --arch arm64 [--out dist/release]
#
# Runs boot/agent/build.sh <arch> --version <app version> --out <tmp>, verifies the files against the SHA256SUMS the
# build wrote, then copies them. Host packages: curl dpkg-dev xz-utils cpio kmod binutils gawk gnupg
# debian-archive-keyring zstd  (apt-get install ...).
set -euo pipefail
# shellcheck source=./package-lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"

ARCH=""
OUT="$PKG_ROOT/dist/release"
while [ $# -gt 0 ]; do
    case "$1" in
        --arch) ARCH="${2:?--arch needs a value}"; shift 2 ;;
        --out) OUT="${2:?--out needs a value}"; shift 2 ;;
        -h | --help) sed -n '2,9p' "$0"; exit 0 ;;
        *) pkg_die "unknown option: $1" ;;
    esac
done
case "$ARCH" in amd64 | arm64 | armhf) ;; *) pkg_die "--arch must be amd64, arm64 or armhf" ;; esac
BUILD="$PKG_ROOT/boot/agent/build.sh"
[ -f "$BUILD" ] || pkg_die "$BUILD not found"
VERSION="$(pkg_version)"

TMPOUT="$PKG_ROOT/dist/agent-$ARCH"
rm -rf "$TMPOUT"
mkdir -p "$TMPOUT" "$OUT"
pkg_log "building the $ARCH boot agent"
bash "$BUILD" "$ARCH" --version "$VERSION" --out "$TMPOUT" --work "$PKG_ROOT/dist/agent-work-$ARCH" >&2

for ext in vmlinuz initrd; do
    f="$TMPOUT/agent-$ARCH.$ext"
    [ -s "$f" ] || pkg_die "build.sh did not produce $f"
    want="$(awk -v n="agent-$ARCH.$ext" '{ x = $2; sub(/^\*/, "", x); sub(/^.*\//, "", x); if (x == n) { print $1; exit } }' "$TMPOUT/SHA256SUMS" 2>/dev/null || true)"
    [ -n "$want" ] || pkg_die "agent-$ARCH.$ext is not listed in $TMPOUT/SHA256SUMS"
    [ "$(pkg_sha256 "$f")" = "$want" ] || pkg_die "checksum mismatch for agent-$ARCH.$ext against the build's own SHA256SUMS"
    cp "$f" "$OUT/synofurb-agent-$ARCH-$VERSION.$ext"
    pkg_log "agent-$ARCH.$ext -> synofurb-agent-$ARCH-$VERSION.$ext"
done
