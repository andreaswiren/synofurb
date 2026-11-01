#!/usr/bin/env bash
# .rpm from the prebuilt binary with rpmbuild (apt-get install rpm):  synofurb-<version>-linux-<arch>.rpm
#
#   scripts/package-rpm.sh --target x86_64-unknown-linux-gnu [--out dist/release]
#
# Spec: packaging/linux/synofurb.spec.  A pre-release version (0.2.0-rc.1) becomes RPM version 0.2.0~rc.1.
set -euo pipefail
# shellcheck source=./package-lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"

TARGET=""
OUT="$PKG_ROOT/dist/release"
while [ $# -gt 0 ]; do
    case "$1" in
        --target) TARGET="${2:?--target needs a value}"; shift 2 ;;
        --out) OUT="${2:?--out needs a value}"; shift 2 ;;
        -h | --help) sed -n '2,7p' "$0"; exit 0 ;;
        *) pkg_die "unknown option: $1" ;;
    esac
done
[ -n "$TARGET" ] || pkg_die "--target is required"
[ "$(pkg_triple_os "$TARGET")" = linux ] || pkg_die "rpm is for Linux targets"
command -v rpmbuild >/dev/null 2>&1 || pkg_die "rpmbuild not found (apt-get install -y rpm)"

VERSION="$(pkg_version)"
ARCH="$(pkg_triple_arch "$TARGET")"
BIN="$PKG_ROOT/target/$TARGET/release/synofurb"
[ -f "$BIN" ] || pkg_die "$BIN not found: build first"

RPM_VERSION="${VERSION//-/\~}"
RPM_VERSION="${RPM_VERSION//+/.}"
TOP="$PKG_ROOT/dist/rpmbuild-$ARCH"
rm -rf "$TOP"
mkdir -p "$TOP"/{BUILD,RPMS,SOURCES,SPECS,SRPMS,BUILDROOT} "$OUT"
cp "$PKG_ROOT/packaging/linux/synofurb.spec" "$TOP/SPECS/"

rpmbuild -bb "$TOP/SPECS/synofurb.spec" \
    --target "${ARCH}-linux" \
    --define "_topdir $TOP" \
    --define "pkg_version $RPM_VERSION" \
    --define "bin_path $BIN" \
    --define "icon_path $PKG_ROOT/packaging/icons/512x512.png" \
    --define "desktop_path $PKG_ROOT/packaging/linux/synofurb.desktop" \
    --define "license_path $PKG_ROOT/LICENSE" \
    >&2

built=("$TOP"/RPMS/*/synofurb-*.rpm)
if [ "${#built[@]}" != 1 ] || [ ! -f "${built[0]}" ]; then pkg_die "rpmbuild did not produce exactly one rpm"; fi
DEST="$OUT/$(pkg_asset_name "$VERSION" linux "$ARCH" rpm)"
cp "${built[0]}" "$DEST"
pkg_log "wrote $(basename "$DEST") ($(pkg_sha256 "$DEST"))"
