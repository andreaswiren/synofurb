#!/bin/sh
# synofurb installer for Linux and macOS (POSIX sh): the desktop app, or with --cli the headless command line.
#
#   curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh -s -- --version 0.2.0 --dir ~/.local/bin --run scan
#   curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh -s -- --cli      # servers, SSH sessions, CI
#
# What it does: detects OS/arch, finds the release (latest, or --version), downloads the matching asset plus
# SHA256SUMS and SHA256SUMS.minisig, verifies the minisign signature (when `minisign` is installed) and the
# SHA-256, installs the `synofurb` binary and (Linux) a desktop entry; with --cli it installs `synofurb-cli`
# (no GUI libraries needed) to ~/.local/bin and nothing else.  It never evaluates downloaded data;
# the only things taken from the network are files that are verified before they are used.
#
# Run with --help for all options.  Source of truth for the release layout: docs/RELEASING.md.

set -eu
umask 022

REPO="andreaswiren/synofurb"
API_BASE="${SYNOFURB_API_BASE:-https://api.github.com/repos/${REPO}}"
DL_BASE="${SYNOFURB_DL_BASE:-https://github.com/${REPO}/releases/download}"
# Public key used to verify SHA256SUMS.minisig (the contents of the .pub file's second line).
# The release procedure replaces this placeholder with the real key; see docs/RELEASING.md.
MINISIGN_PUBKEY_BUILTIN="REPLACE_WITH_MINISIGN_PUBLIC_KEY"

VERSION=""
DIR=""
SYSTEM=0
FORMAT="tar"
CLI=0
BIN_NAME="synofurb"
UNINSTALL=0
DRY_RUN=0
INSECURE=0
NO_DESKTOP=0
PUBKEY="${SYNOFURB_MINISIGN_PUBKEY:-$MINISIGN_PUBKEY_BUILTIN}"
MINISIGN_BIN="${SYNOFURB_MINISIGN_BIN:-minisign}"
TMP=""

usage() {
    cat <<'EOF'
synofurb installer (Linux, macOS)

usage: install.sh [options] [--run SUBCOMMAND [ARGS...]]

options:
  --cli                      install the headless command line `synofurb-cli` instead of the desktop app (no GUI libraries,
                             no desktop entry; works on servers and over SSH). Default location ~/.local/bin/synofurb-cli
  --version X.Y.Z            install this release instead of the latest
  --dir DIR                  install the binary into DIR (default: ~/.local/bin; with --system: /usr/local/bin)
  --system                   system-wide install (uses sudo when not root)
  --format tar|appimage|dmg  asset to install. Default tar (bare binary). Linux: tar, appimage. macOS: tar, dmg
  --no-desktop-entry         do not create the Linux desktop entry / icon
  --pubkey KEY               minisign public key (base64 line) to verify SHA256SUMS.minisig with
  --insecure-skip-signature  accept the SHA-256 check alone (proves integrity, NOT authenticity). Needed when
                             minisign is not installed
  --uninstall                remove what this installer put in place (binary, desktop entry, icon, app bundle; with --cli: synofurb-cli)
  --dry-run                  show what would be done; download and change nothing
  -h, --help                 this text
  --run ARGS...              (must come last) after installing, run `synofurb ARGS...` (`synofurb-cli ARGS...` with --cli)

examples:
  curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/install.sh | sh
  curl -fsSL .../install.sh | sh -s -- --version 0.2.0 --dir ~/.local/bin --run scan
  curl -fsSL .../install.sh | sh -s -- --run install --host 192.168.1.20 --user admin --mode keep ...
  curl -fsSL .../install.sh | sh -s -- --cli                     # headless server / SSH session
  curl -fsSL .../install.sh | sh -s -- --cli --run wizard        # install it and start the terminal wizard

environment (mostly for testing/mirrors): SYNOFURB_API_BASE, SYNOFURB_DL_BASE, SYNOFURB_MINISIGN_PUBKEY,
SYNOFURB_MINISIGN_BIN, SYNOFURB_FORCE_OS, SYNOFURB_FORCE_ARCH
EOF
}

say() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}
usage_error() {
    printf 'error: %s\n\n' "$*" >&2
    usage >&2
    exit 2
}
have() { command -v "$1" >/dev/null 2>&1; }

cleanup() {
    if [ -n "$TMP" ] && [ -d "$TMP" ]; then rm -rf "$TMP"; fi
}

# ---------------------------------------------------------------------------------------------- network
CURL_PROTO="--proto =https --tlsv1.2"
WGET_PROTO="--https-only --secure-protocol=TLSv1_2"
case "$API_BASE$DL_BASE" in
    http://*)
        warn "using a plain-http mirror (SYNOFURB_API_BASE / SYNOFURB_DL_BASE); this is meant for tests only"
        CURL_PROTO="--proto =http,https"
        WGET_PROTO=""
        ;;
esac

fetch() { # url dest
    if have curl; then
        # shellcheck disable=SC2086 # CURL_PROTO is a fixed, space-separated option list
        curl -fsSL $CURL_PROTO --retry 3 --connect-timeout 20 -o "$2" "$1" || return 1
    elif have wget; then
        # shellcheck disable=SC2086
        wget -q $WGET_PROTO --tries=3 --timeout=20 -O "$2" "$1" || return 1
    else
        die "need curl or wget"
    fi
}

# ---------------------------------------------------------------------------------------------- helpers
sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | awk '{print $1}'
    elif have shasum; then
        shasum -a 256 "$1" | awk '{print $1}'
    elif have openssl; then
        openssl dgst -sha256 "$1" | awk '{print $NF}'
    else
        die "need sha256sum, shasum or openssl to verify the download"
    fi
}

valid_version() { printf '%s' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+.][0-9A-Za-z.+-]*)?$'; }

json_string() { # key ; reads JSON on stdin, prints the first string value of that key
    sed -n 's/.*"'"$1"'"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
}

detect_os() {
    if [ -n "${SYNOFURB_FORCE_OS:-}" ]; then
        OS="$SYNOFURB_FORCE_OS"
    else
        case "$(uname -s)" in
            Linux) OS=linux ;;
            Darwin) OS=macos ;;
            MINGW* | MSYS* | CYGWIN*) die "this is Windows; use install.ps1 instead: irm https://raw.githubusercontent.com/${REPO}/main/install.ps1 | iex" ;;
            *) die "unsupported operating system: $(uname -s) (supported: Linux, macOS; Windows has install.ps1)" ;;
        esac
    fi
    case "$OS" in linux | macos) ;; *) die "unsupported operating system: $OS" ;; esac
}

detect_arch() {
    if [ -n "${SYNOFURB_FORCE_ARCH:-}" ]; then
        ARCH="$SYNOFURB_FORCE_ARCH"
    else
        m="$(uname -m)"
        if [ "$OS" = macos ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ]; then
            m=arm64 # a Rosetta shell reports x86_64 on Apple silicon; the native build is the right one
        fi
        case "$m" in
            x86_64 | amd64) ARCH=x86_64 ;;
            aarch64 | arm64) ARCH=aarch64 ;;
            *) die "unsupported CPU architecture: $m (published desktop builds: x86_64, aarch64)" ;;
        esac
    fi
    case "$ARCH" in x86_64 | aarch64) ;; *) die "unsupported CPU architecture: $ARCH" ;; esac
    if [ "$OS" = linux ] && [ -z "${SYNOFURB_FORCE_OS:-}" ] && have ldd && ldd --version 2>&1 | grep -qi musl; then
        die "this system uses musl libc (Alpine?); the published Linux builds need glibc"
    fi
}

run_priv() { # run a command with sudo when system-wide and not root
    if [ -n "$SUDO" ]; then sudo "$@"; else "$@"; fi
}

# ---------------------------------------------------------------------------------------------- locations
compute_paths() {
    SUDO=""
    if [ "$SYSTEM" = 1 ]; then
        [ -n "$DIR" ] || DIR=/usr/local/bin
        case "$DIR" in */bin) DATA_DIR="${DIR%/bin}/share" ;; *) DATA_DIR=/usr/local/share ;; esac
        APP_DIR=/Applications
        if [ "$(id -u)" != 0 ]; then
            have sudo || die "--system needs root or sudo"
            SUDO=sudo
        fi
    else
        [ -n "${HOME:-}" ] || die "HOME is not set"
        [ -n "$DIR" ] || DIR="$HOME/.local/bin"
        DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
        APP_DIR="$HOME/Applications"
    fi
    case "$DIR" in
        /*) ;;
        *) die "--dir must be an absolute path (got: $DIR)" ;;
    esac
    # these end up in a .desktop file / shell commands: refuse characters that need escaping
    nl="$(printf '\n_')"
    nl="${nl%_}"
    case "$DIR$DATA_DIR" in
        *\"* | *\\* | *\$* | *\`* | *%* | *"$nl"*) die "install paths must not contain a double quote, backslash, \$, backtick, % or newline" ;;
    esac
    BIN="$DIR/$BIN_NAME"
    DESKTOP_FILE="$DATA_DIR/applications/synofurb.desktop"
    ICON_FILE="$DATA_DIR/icons/hicolor/256x256/apps/synofurb.png"
    APP_BUNDLE="$APP_DIR/synofurb.app"
}

# ---------------------------------------------------------------------------------------------- release lookup
resolve_version() {
    if [ -n "$VERSION" ]; then
        VERSION="${VERSION#v}"
        valid_version "$VERSION" || die "invalid --version: $VERSION (expected e.g. 0.2.0)"
        TAG="v$VERSION"
        return
    fi
    say "Looking up the latest release ..."
    meta="$TMP/latest.json"
    fetch "$API_BASE/releases/latest" "$meta" || die "could not query $API_BASE/releases/latest (no release yet, offline, or GitHub API rate limit). Try --version X.Y.Z"
    TAG="$(json_string tag_name <"$meta")"
    [ -n "$TAG" ] || die "release metadata has no tag_name"
    VERSION="${TAG#v}"
    valid_version "$VERSION" || die "refusing unexpected release tag from the API: $TAG"
    TAG="v$VERSION"
}

select_asset() {
    if [ "$CLI" = 1 ]; then
        # the command line ships as a bare binary in a tar.gz on both Linux and macOS
        [ "$FORMAT" = tar ] || die "--cli is only available as --format tar (got: $FORMAT)"
        EXT="tar.gz"
        ASSET="synofurb-cli-${VERSION}-${OS}-${ARCH}.${EXT}"
        URL_BASE="$DL_BASE/$TAG"
        return
    fi
    case "$OS:$FORMAT" in
        linux:tar | macos:tar) EXT="tar.gz" ;;
        linux:appimage) EXT="AppImage" ;;
        macos:dmg) EXT="dmg" ;;
        *) die "--format $FORMAT is not available on $OS (linux: tar, appimage; macos: tar, dmg)" ;;
    esac
    ASSET="synofurb-${VERSION}-${OS}-${ARCH}.${EXT}"
    URL_BASE="$DL_BASE/$TAG"
}

# ---------------------------------------------------------------------------------------------- verification
print_minisign_help() {
    cat >&2 <<'EOF'

minisign is needed to verify that the download was really published by the synofurb maintainer.
Install it, then run this installer again:
  macOS:           brew install minisign
  Debian/Ubuntu:   sudo apt install minisign
  Fedora:          sudo dnf install minisign
  Arch:            sudo pacman -S minisign
  Alpine:          apk add minisign
  other:           https://jedisct1.github.io/minisign/
Or accept the weaker SHA-256-only check (integrity, not authenticity) with:  --insecure-skip-signature
EOF
}

verify_signature() { # dir containing SHA256SUMS and SHA256SUMS.minisig
    if [ "$INSECURE" = 1 ]; then
        warn "signature verification SKIPPED (--insecure-skip-signature): only the SHA-256 is checked, which cannot detect a tampered release"
        return 0
    fi
    if ! have "$MINISIGN_BIN"; then
        printf 'error: cannot verify the release signature: "%s" is not installed.\n' "$MINISIGN_BIN" >&2
        print_minisign_help
        exit 1
    fi
    if [ "$PUBKEY" = "REPLACE_WITH_MINISIGN_PUBLIC_KEY" ]; then
        die "this installer has no minisign public key built in yet (development copy). Pass --pubkey KEY, or --insecure-skip-signature"
    fi
    fetch "$URL_BASE/SHA256SUMS.minisig" "$1/SHA256SUMS.minisig" || die "could not download $URL_BASE/SHA256SUMS.minisig"
    if "$MINISIGN_BIN" -Vm "$1/SHA256SUMS" -x "$1/SHA256SUMS.minisig" -P "$PUBKEY" >/dev/null 2>&1; then
        say "Signature OK (minisign)."
    else
        die "SIGNATURE VERIFICATION FAILED for SHA256SUMS - the release is not authentic. Nothing was installed."
    fi
}

verify_checksum() { # dir ; verifies $ASSET against SHA256SUMS
    expected="$(awk -v f="$ASSET" '{ n = $2; sub(/^\*/, "", n); if (n == f) { print $1; exit } }' "$1/SHA256SUMS")"
    [ -n "$expected" ] || die "$ASSET is not listed in SHA256SUMS - refusing to install an unlisted file"
    printf '%s' "$expected" | grep -Eq '^[0-9a-fA-F]{64}$' || die "malformed SHA256SUMS entry for $ASSET"
    actual="$(sha256_of "$1/$ASSET")"
    if [ "$(printf '%s' "$expected" | tr 'A-F' 'a-f')" != "$actual" ]; then
        die "SHA-256 MISMATCH for $ASSET (expected $expected, got $actual). The download is corrupt or tampered with. Nothing was installed."
    fi
    say "SHA-256 OK ($actual)."
}

# ---------------------------------------------------------------------------------------------- install steps
check_tar_listing() { # archive ; refuse path traversal, absolute paths and links
    list="$TMP/tar.list"
    tar -tzf "$1" >"$list" 2>/dev/null || die "cannot read $ASSET as a tar.gz archive"
    if grep -Eq '(^/|(^|/)\.\.(/|$))' "$list"; then
        die "archive contains absolute or parent-directory paths - refusing to extract"
    fi
    if tar -tvzf "$1" 2>/dev/null | cut -c1 | grep -Eq '[lhbcp]'; then
        die "archive contains links or special files - refusing to extract"
    fi
}

atomic_install() { # src dest mode
    tmpdest="$(dirname "$2")/.$(basename "$2").new.$$"
    run_priv install -m "$3" "$1" "$tmpdest"
    run_priv mv -f "$tmpdest" "$2"
}

install_tar() {
    archive="$1"
    check_tar_listing "$archive"
    stage="$TMP/stage"
    mkdir -p "$stage"
    tar -xzf "$archive" -C "$stage"
    if [ ! -f "$stage/$BIN_NAME" ] || [ -L "$stage/$BIN_NAME" ]; then
        die "archive does not contain a regular file named '$BIN_NAME' at its top level"
    fi
    run_priv mkdir -p "$DIR"
    atomic_install "$stage/$BIN_NAME" "$BIN" 0755
    say "Installed $BIN"
    if [ "$OS" = macos ]; then xattr -d com.apple.quarantine "$BIN" 2>/dev/null || true; fi
    ICON_SRC=""
    if [ -f "$stage/synofurb.png" ] && [ ! -L "$stage/synofurb.png" ]; then ICON_SRC="$stage/synofurb.png"; fi
    if [ "$OS" = linux ] && [ "$NO_DESKTOP" = 0 ] && [ "$CLI" = 0 ]; then
        create_desktop_entry "$ICON_SRC"
    fi
}

install_appimage() {
    run_priv mkdir -p "$DIR"
    atomic_install "$1" "$BIN" 0755
    say "Installed $BIN (AppImage; needs FUSE 2 or run it with APPIMAGE_EXTRACT_AND_RUN=1)"
    if [ "$NO_DESKTOP" = 0 ]; then create_desktop_entry ""; fi
}

create_desktop_entry() { # icon source (may be empty)
    icon_name="synofurb"
    if [ -n "$1" ]; then
        run_priv mkdir -p "$(dirname "$ICON_FILE")"
        atomic_install "$1" "$ICON_FILE" 0644
    else
        icon_name="utilities-terminal"
    fi
    run_priv mkdir -p "$(dirname "$DESKTOP_FILE")"
    entry="$TMP/synofurb.desktop"
    cat >"$entry" <<EOF
[Desktop Entry]
Type=Application
Name=synofurb
GenericName=Synology to Debian reinstaller
Comment=Reinstall Synology NAS units with plain Debian 13
Exec="$BIN"
Icon=$icon_name
Terminal=false
Categories=System;Utility;Network;
StartupWMClass=synofurb
EOF
    atomic_install "$entry" "$DESKTOP_FILE" 0644
    say "Created desktop entry $DESKTOP_FILE"
    if have update-desktop-database; then run_priv update-desktop-database "$(dirname "$DESKTOP_FILE")" >/dev/null 2>&1 || true; fi
}

install_dmg() {
    have hdiutil || die "hdiutil not found (--format dmg works on macOS only)"
    mnt="$TMP/dmg-mount"
    mkdir -p "$mnt"
    hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$mnt" "$1" >/dev/null || die "could not mount $ASSET"
    app=""
    for candidate in "$mnt"/*.app; do
        if [ -d "$candidate" ]; then
            app="$candidate"
            break
        fi
    done
    if [ -z "$app" ]; then
        hdiutil detach "$mnt" -quiet >/dev/null 2>&1 || true
        die "no .app bundle found inside $ASSET"
    fi
    run_priv mkdir -p "$APP_DIR"
    run_priv rm -rf "$APP_BUNDLE"
    if ! run_priv cp -R "$app" "$APP_BUNDLE"; then
        hdiutil detach "$mnt" -quiet >/dev/null 2>&1 || true
        die "could not copy the app to $APP_DIR"
    fi
    hdiutil detach "$mnt" -quiet >/dev/null 2>&1 || true
    run_priv xattr -dr com.apple.quarantine "$APP_BUNDLE" 2>/dev/null || true
    inner="$APP_BUNDLE/Contents/MacOS/synofurb"
    [ -x "$inner" ] || die "installed app has no Contents/MacOS/synofurb executable"
    run_priv mkdir -p "$DIR"
    run_priv ln -sf "$inner" "$BIN"
    say "Installed $APP_BUNDLE and linked $BIN"
}

# ---------------------------------------------------------------------------------------------- uninstall
do_uninstall() {
    removed=0
    if [ "$CLI" = 1 ]; then
        # only the command-line binary: the desktop app (same prefix, other file) stays
        if [ -e "$BIN" ] || [ -L "$BIN" ]; then
            if [ "$DRY_RUN" = 1 ]; then say "would remove $BIN"; else
                run_priv rm -f "$BIN"
                say "Removed $BIN"
            fi
        else
            say "Nothing to remove (looked for $BIN)."
        fi
        say "Your settings and inventory (e.g. ~/.config/synofurb) were left in place; delete them yourself if you want them gone."
        return
    fi
    for f in "$BIN" "$DESKTOP_FILE" "$ICON_FILE"; do
        if [ -e "$f" ] || [ -L "$f" ]; then
            if [ "$DRY_RUN" = 1 ]; then say "would remove $f"; else
                run_priv rm -f "$f"
                say "Removed $f"
            fi
            removed=1
        fi
    done
    if [ "$OS" = macos ] && [ -d "$APP_BUNDLE" ]; then
        if [ "$DRY_RUN" = 1 ]; then say "would remove $APP_BUNDLE"; else
            run_priv rm -rf "$APP_BUNDLE"
            say "Removed $APP_BUNDLE"
        fi
        removed=1
    fi
    if [ "$removed" = 0 ]; then say "Nothing to remove (looked for $BIN and the desktop entry)."; fi
    say "Your settings and inventory (e.g. ~/.config/synofurb) were left in place; delete them yourself if you want them gone."
}

# ---------------------------------------------------------------------------------------------- main
main() {
    RUN_ARGS_SET=0
    while [ $# -gt 0 ]; do
        case "$1" in
            -h | --help)
                usage
                exit 0
                ;;
            --version)
                [ $# -ge 2 ] || usage_error "--version needs a value"
                VERSION="$2"
                shift 2
                ;;
            --dir)
                [ $# -ge 2 ] || usage_error "--dir needs a value"
                DIR="$2"
                shift 2
                ;;
            --format)
                [ $# -ge 2 ] || usage_error "--format needs a value"
                FORMAT="$2"
                shift 2
                ;;
            --cli)
                CLI=1
                BIN_NAME="synofurb-cli"
                shift
                ;;
            --pubkey)
                [ $# -ge 2 ] || usage_error "--pubkey needs a value"
                PUBKEY="$2"
                shift 2
                ;;
            --system)
                SYSTEM=1
                shift
                ;;
            --no-desktop-entry)
                NO_DESKTOP=1
                shift
                ;;
            --insecure-skip-signature)
                INSECURE=1
                shift
                ;;
            --uninstall)
                UNINSTALL=1
                shift
                ;;
            --dry-run)
                DRY_RUN=1
                shift
                ;;
            --run)
                shift
                [ $# -ge 1 ] || usage_error "--run needs a subcommand, e.g. --run scan"
                RUN_ARGS_SET=1
                break # everything after --run belongs to synofurb
                ;;
            *) usage_error "unknown option: $1" ;;
        esac
    done
    case "$DIR" in "~"*) DIR="$HOME${DIR#\~}" ;; esac # an unexpanded ~ (quoted by the user): expand it ourselves

    detect_os
    detect_arch
    compute_paths

    if [ "$UNINSTALL" = 1 ]; then
        do_uninstall
        exit 0
    fi

    TMP="$(mktemp -d "${TMPDIR:-/tmp}/synofurb-install.XXXXXX")" || die "cannot create a temporary directory"
    trap cleanup EXIT
    trap 'exit 130' INT TERM HUP

    resolve_version
    select_asset

    say "$BIN_NAME $VERSION for $OS/$ARCH ($ASSET)"
    if [ "$DRY_RUN" = 1 ]; then
        say "[dry-run] would download $URL_BASE/$ASSET"
        say "[dry-run] would download $URL_BASE/SHA256SUMS and $URL_BASE/SHA256SUMS.minisig"
        if [ "$INSECURE" = 1 ]; then say "[dry-run] would verify SHA-256 only (--insecure-skip-signature)"; else say "[dry-run] would verify the minisign signature and SHA-256"; fi
        say "[dry-run] would install to $BIN$([ "$SYSTEM" = 1 ] && printf ' (system-wide%s)' "${SUDO:+, via sudo}")"
        if [ "$OS" = linux ] && [ "$NO_DESKTOP" = 0 ] && [ "$CLI" = 0 ]; then say "[dry-run] would create $DESKTOP_FILE"; fi
        if [ "$RUN_ARGS_SET" = 1 ]; then say "[dry-run] would run: $BIN $*"; fi
        say "[dry-run] nothing was downloaded or changed."
        exit 0
    fi

    say "Downloading ..."
    fetch "$URL_BASE/SHA256SUMS" "$TMP/SHA256SUMS" || die "could not download $URL_BASE/SHA256SUMS (does release $TAG exist?)"
    verify_signature "$TMP"
    # fail early and clearly if there is no build for this platform
    if ! grep -qF -- "$ASSET" "$TMP/SHA256SUMS"; then
        die "release $TAG has no asset for $OS/$ARCH ($ASSET). Published builds are listed at https://github.com/${REPO}/releases/tag/$TAG"
    fi
    fetch "$URL_BASE/$ASSET" "$TMP/$ASSET" || die "could not download $URL_BASE/$ASSET"
    verify_checksum "$TMP"

    case "$EXT" in
        tar.gz) install_tar "$TMP/$ASSET" ;;
        AppImage) install_appimage "$TMP/$ASSET" ;;
        dmg) install_dmg "$TMP/$ASSET" ;;
    esac

    case ":${PATH:-}:" in
        *":$DIR:"*) ;;
        *) say "Note: $DIR is not on your PATH. Add it, e.g.:  export PATH=\"$DIR:\$PATH\"" ;;
    esac
    say "Done. Run:  $BIN --help"

    if [ "$RUN_ARGS_SET" = 1 ]; then
        say "Running: $BIN_NAME $*"
        cleanup
        trap - EXIT
        # when this script itself was piped into sh, stdin is the script: give the program the terminal instead
        if [ ! -t 0 ] && (: </dev/tty) 2>/dev/null; then
            status=0
            "$BIN" "$@" </dev/tty || status=$?
        elif [ ! -t 0 ]; then
            status=0
            "$BIN" "$@" </dev/null || status=$?
        else
            status=0
            "$BIN" "$@" || status=$?
        fi
        exit "$status"
    fi
}

main "$@"
exit 0
