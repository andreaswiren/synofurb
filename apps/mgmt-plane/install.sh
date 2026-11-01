#!/bin/sh
# synofurb management plane installer for an EXISTING Debian/Ubuntu box with systemd (POSIX sh).
#
#   curl -fsSL https://raw.githubusercontent.com/andreaswiren/synofurb/main/apps/mgmt-plane/install.sh | sudo sh
#   curl -fsSL .../install.sh | sudo sh -s -- --uninstall
#
# Installs synofurb-mgmt-<version>.tar.gz from the GitHub release into /opt/synofurb-mgmt, installs and enables
# the systemd unit `synofurb-mgmt.service` (HTTPS on :8443).  The SHA-256 of the download is checked against the
# release's SHA256SUMS, whose minisign signature is verified first.  Idempotent: running it again upgrades or
# re-applies; state in /var/lib/synofurb-mgmt (certificate, sessions) is kept across upgrades and --uninstall.
#
# The management plane has its own version (apps/mgmt-plane/VERSION), independent of the desktop app's.

set -eu
umask 022

REPO="andreaswiren/synofurb"
API_BASE="${SYNOFURB_API_BASE:-https://api.github.com/repos/${REPO}}"
DL_BASE="${SYNOFURB_DL_BASE:-https://github.com/${REPO}/releases/download}"
MINISIGN_PUBKEY_BUILTIN="REPLACE_WITH_MINISIGN_PUBLIC_KEY"

ROOT=""
RELEASE_TAG=""
MGMT_VERSION=""
UNINSTALL=0
PURGE=0
DRY_RUN=0
NO_START=0
INSECURE=0
PUBKEY="${SYNOFURB_MINISIGN_PUBKEY:-$MINISIGN_PUBKEY_BUILTIN}"
MINISIGN_BIN="${SYNOFURB_MINISIGN_BIN:-minisign}"
TMP=""

usage() {
    cat <<'EOF'
synofurb management plane installer (Debian/Ubuntu + systemd)

usage: sudo sh install.sh [options]

options:
  --version X.Y.Z            install this management-plane version (searches the published releases)
  --release vA.B.C           take the management plane shipped with this synofurb release (default: latest)
  --root DIR                 stage everything below DIR instead of /  (no systemctl; for images/chroots/tests)
  --no-start                 install but do not enable/start the service
  --pubkey KEY               minisign public key to verify SHA256SUMS.minisig with
  --insecure-skip-signature  accept the SHA-256 check alone (needed when minisign is not installed)
  --uninstall                stop, disable and remove the service and /opt/synofurb-mgmt (state is kept)
  --purge                    with --uninstall: also delete /var/lib/synofurb-mgmt and /var/log/synofurb-mgmt
  --dry-run                  show what would happen; download and change nothing
  -h, --help                 this text

Needs: python3, tar, curl or wget, sha256sum, and minisign (apt install minisign) unless --insecure-skip-signature.
environment (tests/mirrors): SYNOFURB_API_BASE, SYNOFURB_DL_BASE, SYNOFURB_MINISIGN_PUBKEY, SYNOFURB_MINISIGN_BIN
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
# shellcheck disable=SC2317 # called through the EXIT trap
cleanup() {
    if [ -n "$TMP" ] && [ -d "$TMP" ]; then rm -rf "$TMP"; fi
}

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

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | awk '{print $1}'
    elif have shasum; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        die "need sha256sum to verify the download"
    fi
}

valid_version() { printf '%s' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+.][0-9A-Za-z.+-]*)?$'; }

# ---------------------------------------------------------------------------------------------- release lookup
# Sets TAG and ASSET (validated) and URL_BASE from the release metadata.
find_release() {
    json="$TMP/release.json"
    if [ -n "$RELEASE_TAG" ]; then
        valid_version "${RELEASE_TAG#v}" || die "invalid --release: $RELEASE_TAG (expected e.g. v0.2.0)"
        url="$API_BASE/releases/tags/v${RELEASE_TAG#v}"
    elif [ -n "$MGMT_VERSION" ]; then
        valid_version "$MGMT_VERSION" || die "invalid --version: $MGMT_VERSION (expected e.g. 0.1.0)"
        url="$API_BASE/releases?per_page=100"
    else
        url="$API_BASE/releases/latest"
    fi
    say "Looking up the management plane release ($url) ..."
    fetch "$url" "$json" || die "could not query $url (no release yet, offline, or API rate limit)"
    # every asset download URL in the JSON, one per line
    grep -Eo '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*"' "$json" | sed 's/.*:[[:space:]]*"\(.*\)"$/\1/' >"$TMP/urls" || true
    if [ -n "$MGMT_VERSION" ]; then
        pick="$(grep -E "/synofurb-mgmt-${MGMT_VERSION}\.tar\.gz\$" "$TMP/urls" | head -n 1 || true)"
    else
        pick="$(grep -E '/synofurb-mgmt-[^/]*\.tar\.gz$' "$TMP/urls" | head -n 1 || true)"
    fi
    [ -n "$pick" ] || die "no synofurb-mgmt-*.tar.gz asset found in that release"
    ASSET="${pick##*/}"
    TAG="$(printf '%s' "$pick" | sed -n 's#.*/\([^/]*\)/[^/]*$#\1#p')"  # the path segment before the file name is the release tag"
    # never trust remote strings beyond a strict whitelist; the URL is rebuilt from known parts
    printf '%s' "$ASSET" | grep -Eq '^synofurb-mgmt-[0-9]+\.[0-9]+\.[0-9]+([-+.][0-9A-Za-z.+-]*)?\.tar\.gz$' || die "unexpected asset name in release metadata: $ASSET"
    valid_version "${TAG#v}" || die "unexpected release tag in asset URL: $TAG"
    URL_BASE="$DL_BASE/$TAG"
    NEW_VERSION="${ASSET#synofurb-mgmt-}"
    NEW_VERSION="${NEW_VERSION%.tar.gz}"
}

# ---------------------------------------------------------------------------------------------- verification
verify_signature() {
    if [ "$INSECURE" = 1 ]; then
        warn "signature verification SKIPPED (--insecure-skip-signature): only the SHA-256 is checked, which cannot detect a tampered release"
        return 0
    fi
    if ! have "$MINISIGN_BIN"; then
        printf 'error: cannot verify the release signature: "%s" is not installed.\n  Debian/Ubuntu: apt-get install -y minisign\n  or accept the weaker SHA-256-only check with --insecure-skip-signature\n' "$MINISIGN_BIN" >&2
        exit 1
    fi
    if [ "$PUBKEY" = "REPLACE_WITH_MINISIGN_PUBLIC_KEY" ]; then
        die "this installer has no minisign public key built in yet (development copy). Pass --pubkey KEY, or --insecure-skip-signature"
    fi
    fetch "$URL_BASE/SHA256SUMS.minisig" "$TMP/SHA256SUMS.minisig" || die "could not download $URL_BASE/SHA256SUMS.minisig"
    if "$MINISIGN_BIN" -Vm "$TMP/SHA256SUMS" -x "$TMP/SHA256SUMS.minisig" -P "$PUBKEY" >/dev/null 2>&1; then
        say "Signature OK (minisign)."
    else
        die "SIGNATURE VERIFICATION FAILED for SHA256SUMS - the release is not authentic. Nothing was installed."
    fi
}

verify_checksum() {
    expected="$(awk -v f="$ASSET" '{ n = $2; sub(/^\*/, "", n); if (n == f) { print $1; exit } }' "$TMP/SHA256SUMS")"
    [ -n "$expected" ] || die "$ASSET is not listed in SHA256SUMS - refusing to install an unlisted file"
    printf '%s' "$expected" | grep -Eq '^[0-9a-fA-F]{64}$' || die "malformed SHA256SUMS entry for $ASSET"
    actual="$(sha256_of "$TMP/$ASSET")"
    if [ "$(printf '%s' "$expected" | tr 'A-F' 'a-f')" != "$actual" ]; then
        die "SHA-256 MISMATCH for $ASSET (expected $expected, got $actual). Nothing was installed."
    fi
    say "SHA-256 OK ($actual)."
}

# ---------------------------------------------------------------------------------------------- paths
set_paths() {
    case "$ROOT" in "" | /*) ;; *) die "--root must be an absolute path" ;; esac
    ROOT="${ROOT%/}"
    OPT="$ROOT/opt/synofurb-mgmt"
    UNIT_DIR="$ROOT/etc/systemd/system"
    UNIT="$UNIT_DIR/synofurb-mgmt.service"
    WANTS="$UNIT_DIR/multi-user.target.wants/synofurb-mgmt.service"
    STATE="$ROOT/var/lib/synofurb-mgmt"
    LOGS="$ROOT/var/log/synofurb-mgmt"
    if [ -z "$ROOT" ]; then USE_SYSTEMCTL=1; else USE_SYSTEMCTL=0; fi
    if [ "$NO_START" = 1 ]; then USE_SYSTEMCTL=0; fi
}

installed_version() {
    if [ -f "$OPT/VERSION" ]; then head -n 1 "$OPT/VERSION" | tr -d '[:space:]'; fi
}

# ---------------------------------------------------------------------------------------------- actions
do_uninstall() {
    if [ "$DRY_RUN" = 1 ]; then
        say "[dry-run] would stop and disable synofurb-mgmt, remove $UNIT, $WANTS and $OPT"
        if [ "$PURGE" = 1 ]; then say "[dry-run] would also delete $STATE and $LOGS"; fi
        return 0
    fi
    if [ "$USE_SYSTEMCTL" = 1 ] && have systemctl; then
        systemctl disable --now synofurb-mgmt.service >/dev/null 2>&1 || true
    fi
    rm -f "$WANTS" "$UNIT"
    rm -rf "$OPT"
    if [ "$USE_SYSTEMCTL" = 1 ] && have systemctl; then systemctl daemon-reload >/dev/null 2>&1 || true; fi
    if [ "$PURGE" = 1 ]; then
        rm -rf "$STATE" "$LOGS"
        say "Removed the service, $OPT, $STATE and $LOGS."
    else
        say "Removed the service and $OPT. Kept $STATE (certificate, sessions); use --purge to delete it."
    fi
}

check_archive() {
    tar -tzf "$1" >"$TMP/tar.list" 2>/dev/null || die "cannot read $ASSET as a tar.gz archive"
    if grep -Eq '(^/|(^|/)\.\.(/|$))' "$TMP/tar.list"; then die "archive contains absolute or parent-directory paths - refusing to extract"; fi
    if tar -tvzf "$1" 2>/dev/null | cut -c1 | grep -Eq '[lhbcp]'; then die "archive contains links or special files - refusing to extract"; fi
}

do_install() {
    stage="$TMP/stage"
    mkdir -p "$stage"
    tar -xzf "$TMP/$ASSET" -C "$stage"
    for f in synofurb_mgmt.py synofurb-mgmt.service VERSION; do
        if [ ! -f "$stage/$f" ]; then die "archive is missing $f"; fi
    done
    packaged="$(head -n 1 "$stage/VERSION" | tr -d '[:space:]')"
    [ "$packaged" = "$NEW_VERSION" ] || die "archive VERSION ($packaged) does not match its file name ($NEW_VERSION)"

    old="$(installed_version || true)"
    if [ -n "$old" ] && [ "$old" = "$NEW_VERSION" ] && diff -r "$stage" "$OPT" >/dev/null 2>&1; then
        say "synofurb-mgmt $NEW_VERSION is already installed and unchanged."
    else
        mkdir -p "$ROOT/opt"
        rm -rf "$ROOT/opt/.synofurb-mgmt.new" "$ROOT/opt/.synofurb-mgmt.old"
        cp -R "$stage" "$ROOT/opt/.synofurb-mgmt.new"
        chmod -R go-w "$ROOT/opt/.synofurb-mgmt.new"
        chmod 0755 "$ROOT/opt/.synofurb-mgmt.new/synofurb_mgmt.py"
        if [ -d "$OPT" ]; then mv "$OPT" "$ROOT/opt/.synofurb-mgmt.old"; fi
        mv "$ROOT/opt/.synofurb-mgmt.new" "$OPT"
        rm -rf "$ROOT/opt/.synofurb-mgmt.old"
        if [ -n "$old" ]; then say "Upgraded synofurb-mgmt $old -> $NEW_VERSION in $OPT"; else say "Installed synofurb-mgmt $NEW_VERSION in $OPT"; fi
    fi

    mkdir -p "$UNIT_DIR/multi-user.target.wants"
    cp "$stage/synofurb-mgmt.service" "$UNIT.new"
    chmod 0644 "$UNIT.new"
    mv -f "$UNIT.new" "$UNIT"
    ln -sf /etc/systemd/system/synofurb-mgmt.service "$WANTS" # what `systemctl enable` does
    say "Installed $UNIT (enabled)"

    if [ "$USE_SYSTEMCTL" = 1 ]; then
        have systemctl || die "systemctl not found (this installer needs systemd). Use --no-start to only install the files"
        systemctl daemon-reload
        systemctl enable synofurb-mgmt.service >/dev/null 2>&1 || true
        systemctl restart synofurb-mgmt.service || die "the service failed to start; see: journalctl -u synofurb-mgmt -n 50"
        say "Service started. Open https://<this-host>:8443 (self-signed certificate; compare the fingerprint:"
        say "  curl -k https://localhost:8443/api/tls-fingerprint )"
    else
        say "Not starting the service (--root / --no-start). Enable it with: systemctl enable --now synofurb-mgmt"
    fi
}

# ---------------------------------------------------------------------------------------------- main
main() {
    while [ $# -gt 0 ]; do
        case "$1" in
            -h | --help)
                usage
                exit 0
                ;;
            --version)
                [ $# -ge 2 ] || usage_error "--version needs a value"
                MGMT_VERSION="${2#v}"
                shift 2
                ;;
            --release)
                [ $# -ge 2 ] || usage_error "--release needs a value"
                RELEASE_TAG="$2"
                shift 2
                ;;
            --root)
                [ $# -ge 2 ] || usage_error "--root needs a value"
                ROOT="$2"
                shift 2
                ;;
            --pubkey)
                [ $# -ge 2 ] || usage_error "--pubkey needs a value"
                PUBKEY="$2"
                shift 2
                ;;
            --no-start)
                NO_START=1
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
            --purge)
                PURGE=1
                shift
                ;;
            --dry-run)
                DRY_RUN=1
                shift
                ;;
            *) usage_error "unknown option: $1" ;;
        esac
    done
    if [ -n "$RELEASE_TAG" ] && [ -n "$MGMT_VERSION" ]; then usage_error "use either --release or --version, not both"; fi
    set_paths

    if [ -z "$ROOT" ] && [ "$DRY_RUN" = 0 ]; then
        [ "$(id -u)" = 0 ] || die "run as root: curl -fsSL ... | sudo sh   (or use --root DIR to stage into a directory)"
        [ "$(uname -s)" = Linux ] || die "the management plane runs on Linux only"
        if [ "$NO_START" = 0 ] && [ ! -d /run/systemd/system ]; then die "systemd is not running here. Use --no-start to only install the files"; fi
    fi

    if [ "$UNINSTALL" = 1 ]; then
        do_uninstall
        exit 0
    fi

    have python3 || { [ "$DRY_RUN" = 1 ] || die "python3 is required (apt-get install -y python3)"; }
    have tar || die "tar is required"

    TMP="$(mktemp -d "${TMPDIR:-/tmp}/synofurb-mgmt-install.XXXXXX")" || die "cannot create a temporary directory"
    trap cleanup EXIT
    trap 'exit 130' INT TERM HUP

    find_release
    say "synofurb-mgmt $NEW_VERSION (release $TAG, asset $ASSET)"
    if [ "$DRY_RUN" = 1 ]; then
        say "[dry-run] would download $URL_BASE/$ASSET, $URL_BASE/SHA256SUMS and $URL_BASE/SHA256SUMS.minisig"
        say "[dry-run] would verify signature + SHA-256, then install into $OPT and $UNIT"
        if [ "$USE_SYSTEMCTL" = 1 ]; then say "[dry-run] would run systemctl daemon-reload / enable / restart synofurb-mgmt"; fi
        say "[dry-run] currently installed: $(installed_version || true)"
        say "[dry-run] nothing was downloaded or changed."
        exit 0
    fi

    fetch "$URL_BASE/SHA256SUMS" "$TMP/SHA256SUMS" || die "could not download $URL_BASE/SHA256SUMS"
    verify_signature
    fetch "$URL_BASE/$ASSET" "$TMP/$ASSET" || die "could not download $URL_BASE/$ASSET"
    verify_checksum
    check_archive "$TMP/$ASSET"
    do_install
}

main "$@"
exit 0
