#!/usr/bin/env bash
# Validates the release asset directory against the naming contract and writes SHA256SUMS.
#
#   scripts/package-collect.sh --dir dist/release [--version X.Y.Z] [--mgmt-version A.B.C] [--no-sums]
#
# Contract (see docs/RELEASING.md; the in-app updater depends on it):
#   synofurb-<version>-<os>-<arch>.<ext>      os: windows|macos|linux   arch: x86_64|aarch64
#                                             ext: msi exe dmg pkg deb rpm AppImage tar.gz zip
#   synofurb-agent-<amd64|arm64|armhf>-<version>.vmlinuz / .initrd       boot agent
#   synofurb-dom   compat-models.json   synofurb-mgmt-<mgmt-version>.tar.gz
#   SHA256SUMS (written here)   SHA256SUMS.minisig (written by the signing step)
# Fails when a REQUIRED asset is missing or a file does not match the contract. Optional assets (.pkg, Windows
# aarch64) only produce a warning when absent.
set -euo pipefail
# shellcheck source=./package-lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"

DIR=""
VERSION=""
MGMT_VERSION=""
WRITE_SUMS=1
while [ $# -gt 0 ]; do
    case "$1" in
        --dir) DIR="${2:?--dir needs a value}"; shift 2 ;;
        --version) VERSION="${2:?}"; shift 2 ;;
        --mgmt-version) MGMT_VERSION="${2:?}"; shift 2 ;;
        --no-sums) WRITE_SUMS=0; shift ;;
        -h | --help) sed -n '2,15p' "$0"; exit 0 ;;
        *) pkg_die "unknown option: $1" ;;
    esac
done
[ -d "$DIR" ] || pkg_die "--dir must be an existing directory"
[ -n "$VERSION" ] || VERSION="$(pkg_version)"
[ -n "$MGMT_VERSION" ] || MGMT_VERSION="$(pkg_mgmt_version)"

required=()
optional=()
required+=("synofurb-$VERSION-windows-x86_64.msi" "synofurb-$VERSION-windows-x86_64.exe" "synofurb-$VERSION-windows-x86_64.zip")
optional+=("synofurb-$VERSION-windows-aarch64.msi" "synofurb-$VERSION-windows-aarch64.exe" "synofurb-$VERSION-windows-aarch64.zip")
for plat in macos-aarch64 macos-x86_64; do
    required+=("synofurb-$VERSION-$plat.dmg" "synofurb-$VERSION-$plat.tar.gz")
    optional+=("synofurb-$VERSION-$plat.pkg")
done
for plat in linux-x86_64 linux-aarch64; do
    required+=("synofurb-$VERSION-$plat.deb" "synofurb-$VERSION-$plat.rpm" "synofurb-$VERSION-$plat.AppImage" "synofurb-$VERSION-$plat.tar.gz")
done
for a in amd64 arm64 armhf; do required+=("synofurb-agent-$a-$VERSION.vmlinuz" "synofurb-agent-$a-$VERSION.initrd"); done
required+=("synofurb-dom" "compat-models.json" "synofurb-mgmt-$MGMT_VERSION.tar.gz")

fail=0
for f in "${required[@]}"; do
    if [ ! -s "$DIR/$f" ]; then printf 'MISSING (required): %s\n' "$f" >&2; fail=1; fi
done
for f in "${optional[@]}"; do
    if [ ! -s "$DIR/$f" ]; then printf 'warning: optional asset not present: %s\n' "$f" >&2; fi
done

known=("${required[@]}" "${optional[@]}" SHA256SUMS SHA256SUMS.minisig)
shopt -s nullglob dotglob
for path in "$DIR"/*; do
    f="$(basename "$path")"
    [ -f "$path" ] || { printf 'UNEXPECTED non-file entry: %s\n' "$f" >&2; fail=1; continue; }
    ok=0
    for k in "${known[@]}"; do [ "$f" = "$k" ] && ok=1 && break; done
    if [ "$ok" = 0 ]; then printf 'UNEXPECTED file (does not match the asset contract for version %s): %s\n' "$VERSION" "$f" >&2; fail=1; fi
done
[ "$fail" = 0 ] || pkg_die "release asset set is not valid (see above)"

if [ "$WRITE_SUMS" = 1 ]; then
    rm -f "$DIR/SHA256SUMS" "$DIR/SHA256SUMS.minisig"
    (
        cd "$DIR"
        for f in *; do printf '%s  %s\n' "$(pkg_sha256 "$f")" "$f"; done | LC_ALL=C sort -k2
    ) >"$DIR/SHA256SUMS.tmp"
    mv "$DIR/SHA256SUMS.tmp" "$DIR/SHA256SUMS"
    pkg_log "SHA256SUMS written ($(wc -l <"$DIR/SHA256SUMS" | tr -d ' ') files)"
fi
