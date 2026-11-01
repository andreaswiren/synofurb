#!/usr/bin/env bash
# macOS installer package (.pkg) wrapping the .app that cargo-packager built:  synofurb-<version>-macos-<arch>.pkg
#
#   scripts/package-cargo-packager.sh --target aarch64-apple-darwin --formats app,dmg   # produces the .app too
#   scripts/package-macos-pkg.sh --target aarch64-apple-darwin
#
# Signing (optional, skipped when absent): APPLE_INSTALLER_IDENTITY = "Developer ID Installer: Name (TEAMID)".
# Notarization (optional): APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID  -> notarytool + stapler.
set -euo pipefail
# shellcheck source=./package-lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"

TARGET=""
OUT="$PKG_ROOT/dist/release"
while [ $# -gt 0 ]; do
    case "$1" in
        --target) TARGET="${2:?--target needs a value}"; shift 2 ;;
        --out) OUT="${2:?--out needs a value}"; shift 2 ;;
        -h | --help) sed -n '2,10p' "$0"; exit 0 ;;
        *) pkg_die "unknown option: $1" ;;
    esac
done
[ -n "$TARGET" ] || pkg_die "--target is required"
[ "$(pkg_triple_os "$TARGET")" = macos ] || pkg_die "pkg is for macOS targets"
command -v pkgbuild >/dev/null 2>&1 || pkg_die "pkgbuild not found (macOS only)"

VERSION="$(pkg_version)"
ARCH="$(pkg_triple_arch "$TARGET")"
APP="$PKG_ROOT/dist/packager-$TARGET/synofurb.app"
[ -d "$APP" ] || pkg_die "$APP not found: run package-cargo-packager.sh with --formats app,dmg first"
mkdir -p "$OUT"
DEST="$OUT/$(pkg_asset_name "$VERSION" macos "$ARCH" pkg)"

args=(--component "$APP" --install-location /Applications --identifier io.github.andreaswiren.synofurb.pkg --version "${VERSION%%[-+]*}")
if [ -n "${APPLE_INSTALLER_IDENTITY:-}" ]; then args+=(--sign "$APPLE_INSTALLER_IDENTITY" --timestamp); else pkg_log "WARNING: APPLE_INSTALLER_IDENTITY not set - the .pkg is UNSIGNED"; fi
pkgbuild "${args[@]}" "$DEST" >&2

if [ -n "${APPLE_INSTALLER_IDENTITY:-}" ] && [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
    pkg_log "notarizing the .pkg (this can take a few minutes)"
    xcrun notarytool submit "$DEST" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait >&2
    xcrun stapler staple "$DEST" >&2
else
    pkg_log "not notarizing the .pkg (needs APPLE_INSTALLER_IDENTITY + APPLE_ID + APPLE_PASSWORD + APPLE_TEAM_ID)"
fi
pkg_log "wrote $(basename "$DEST") ($(pkg_sha256 "$DEST"))"
