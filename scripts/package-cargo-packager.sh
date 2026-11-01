#!/usr/bin/env bash
# Builds the platform installers with cargo-packager and renames them to the release asset contract
#   synofurb-<version>-<os>-<arch>.<ext>
#
#   scripts/package-cargo-packager.sh --target x86_64-pc-windows-msvc --formats wix,nsis
#   scripts/package-cargo-packager.sh --target aarch64-apple-darwin   --formats dmg
#   scripts/package-cargo-packager.sh --target x86_64-unknown-linux-gnu --formats deb,appimage
#
# Prerequisites: `cargo build --release --locked -p synofurb --target <triple>` has been run, and cargo-packager
# (cargo install cargo-packager --version 0.11.8 --locked) is on PATH.  Output: dist/release/ (or --out).
# Optional signing (silently skipped when the variables are absent; see docs/RELEASING.md):
#   APPLE_SIGNING_IDENTITY (+ APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD; notarization: APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID)
#   WINDOWS_CERT_THUMBPRINT (certificate already imported into the user's store; WINDOWS_TIMESTAMP_URL optional)
#   --render-only   write packaging/.Packager.generated.toml, print its path, stop (used by the tests)
set -euo pipefail
# shellcheck source=./package-lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"

TARGET=""
FORMATS=""
OUT="$PKG_ROOT/dist/release"
RENDER_ONLY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --target) TARGET="${2:?--target needs a value}"; shift 2 ;;
        --formats) FORMATS="${2:?--formats needs a value}"; shift 2 ;;
        --out) OUT="${2:?--out needs a value}"; shift 2 ;;
        --render-only) RENDER_ONLY=1; shift ;;
        -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
        *) pkg_die "unknown option: $1" ;;
    esac
done
[ -n "$TARGET" ] || pkg_die "--target is required"
[ -n "$FORMATS" ] || pkg_die "--formats is required (comma separated: wix,nsis,dmg,app,deb,appimage)"

VERSION="$(pkg_version)"
OS="$(pkg_triple_os "$TARGET")"
ARCH="$(pkg_triple_arch "$TARGET")"
BIN_DIR="$PKG_ROOT/target/$TARGET/release"

# ---- render the config ---------------------------------------------------------------------------
template="$(cat "$PKG_ROOT/packaging/Packager.toml")"
gen="$template"
gen="${gen//@VERSION@/$VERSION}"
gen="${gen//@BINARIES_DIR@/$(pkg_native_path "$BIN_DIR")}"
macos_line="# (unsigned build: APPLE_SIGNING_IDENTITY not set)"
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
    ident="${APPLE_SIGNING_IDENTITY//\\/\\\\}"
    macos_line="signing-identity = \"${ident//\"/\\\"}\""
fi
win_line="# (unsigned build: WINDOWS_CERT_THUMBPRINT not set)"
if [ -n "${WINDOWS_CERT_THUMBPRINT:-}" ]; then
    printf '%s' "$WINDOWS_CERT_THUMBPRINT" | grep -Eq '^[0-9A-Fa-f]{40}$' || pkg_die "WINDOWS_CERT_THUMBPRINT must be a 40-digit SHA-1 hex thumbprint"
    win_line="certificate-thumbprint = \"$WINDOWS_CERT_THUMBPRINT\""$'\n'"timestamp-url = \"${WINDOWS_TIMESTAMP_URL:-http://timestamp.digicert.com}\""
fi
gen="${gen//# @MACOS_SIGNING@/$macos_line}"
gen="${gen//# @WINDOWS_SIGNING@/$win_line}"
if printf '%s' "$gen" | grep -Eq '@(VERSION|BINARIES_DIR|MACOS_SIGNING|WINDOWS_SIGNING)@'; then pkg_die "unreplaced placeholder in the generated config"; fi
CONFIG="$PKG_ROOT/packaging/.Packager.generated.toml"
printf '%s\n' "$gen" >"$CONFIG"
if [ "$RENDER_ONLY" = 1 ]; then
    printf '%s\n' "$CONFIG"
    exit 0
fi

# ---- run ------------------------------------------------------------------------------------------
[ -d "$BIN_DIR" ] || pkg_die "$BIN_DIR does not exist: build first with  cargo build --release --locked -p synofurb --target $TARGET"
if [ ! -f "$BIN_DIR/synofurb" ] && [ ! -f "$BIN_DIR/synofurb.exe" ]; then pkg_die "no synofurb binary in $BIN_DIR"; fi
command -v cargo-packager >/dev/null 2>&1 || pkg_die "cargo-packager is not installed (cargo install cargo-packager --version 0.11.8 --locked)"

WORK="$PKG_ROOT/dist/packager-$TARGET"
rm -rf "$WORK"
mkdir -p "$WORK" "$OUT"
pkg_log "cargo-packager $FORMATS for $TARGET (version $VERSION)"
APPIMAGE_EXTRACT_AND_RUN=1 cargo-packager --config "$(pkg_native_path "$CONFIG")" --formats "$FORMATS" --target "$TARGET" --out-dir "$(pkg_native_path "$WORK")" -v

# ---- rename to the asset contract ----------------------------------------------------------------
one() { # glob-pattern -> the single matching file in $WORK
    local matches=() f
    for f in "$WORK"/$1; do [ -f "$f" ] && matches+=("$f"); done
    [ "${#matches[@]}" = 1 ] || pkg_die "expected exactly one '$1' in $WORK, found ${#matches[@]}: ${matches[*]:-}"
    printf '%s' "${matches[0]}"
}
emit() { # pattern ext
    local src dest
    src="$(one "$1")"
    dest="$OUT/$(pkg_asset_name "$VERSION" "$OS" "$ARCH" "$2")"
    cp "$src" "$dest"
    pkg_log "$(basename "$src") -> $(basename "$dest")"
}
IFS=, read -r -a fmts <<<"$FORMATS"
for f in "${fmts[@]}"; do
    case "$f" in
        wix) emit '*.msi' msi ;;
        nsis) emit '*-setup.exe' exe ;;
        dmg) emit '*.dmg' dmg ;;
        deb) emit '*.deb' deb ;;
        appimage) emit '*.AppImage' AppImage ;;
        app) ;; # consumed by package-macos-pkg.sh
        *) pkg_die "unknown format in --formats: $f" ;;
    esac
done
