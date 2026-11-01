#!/usr/bin/env bash
# End-to-end tests for install.sh and apps/mgmt-plane/install.sh against a local fake GitHub release server.
#   tests/install/run-tests.sh            (needs python3, curl or wget, tar; minisign for the signature tests)
# Never touches the real HOME or the real system: HOME, XDG_DATA_HOME and --root/--dir point into a temp dir.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
INSTALL_SH="$REPO/install.sh"
MGMT_SH="$REPO/apps/mgmt-plane/install.sh"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/synofurb-install-tests.XXXXXX")"
SERVER_PID=""
cleanup() {
    [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

PASS=0
FAIL=0
SKIP=0
CURRENT=""
OUT=""
RC=0

ok() { PASS=$((PASS + 1)); printf '  ok   %s\n' "$CURRENT"; }
bad() {
    FAIL=$((FAIL + 1))
    printf '  FAIL %s: %s\n' "$CURRENT" "$*"
    printf '%s\n' "$OUT" | sed 's/^/       | /' | head -25
}
skip() { SKIP=$((SKIP + 1)); printf '  skip %s (%s)\n' "$CURRENT" "$*"; }
begin() { CURRENT="$1"; FAILED_HERE=0; }
check() { # description, condition-result(0=true)
    if [ "$2" -ne 0 ]; then bad "$1"; FAILED_HERE=1; fi
}
finish() { [ "${FAILED_HERE:-0}" = 0 ] && ok; }
expect_rc() { check "exit status $RC, expected $1" "$([ "$RC" = "$1" ]; echo $?)"; }
expect_out() { check "output lacks: $1" "$(printf '%s' "$OUT" | grep -qF -- "$1"; echo $?)"; }
expect_not_out() { check "output unexpectedly contains: $1" "$(printf '%s' "$OUT" | grep -qF -- "$1" && echo 1 || echo 0)"; }
exists() { [ -e "$1" ] || [ -L "$1" ]; }
expect_exists() { check "missing: $1" "$(exists "$1"; echo $?)"; }
expect_absent() { check "should not exist: $1" "$(exists "$1" && echo 1 || echo 0)"; }

# ---------------------------------------------------------------- server + keys
WWW="$WORK/www"
mkdir -p "$WWW"
python3 "$HERE/fake-server.py" "$WWW" "$WORK/port" &
SERVER_PID=$!
for _ in $(seq 1 50); do [ -s "$WORK/port" ] && break; sleep 0.1; done
[ -s "$WORK/port" ] || { echo "fake server did not start"; exit 1; }
PORT="$(cat "$WORK/port")"
BASE="http://127.0.0.1:$PORT"
export SYNOFURB_API_BASE="$BASE/api"
export SYNOFURB_DL_BASE="$BASE/dl"

HAVE_MINISIGN=0
SEC="$WORK/test.key"
PUB=""
if command -v minisign >/dev/null 2>&1; then
    minisign -G -W -f -p "$WORK/test.pub" -s "$SEC" >/dev/null 2>&1 && HAVE_MINISIGN=1
    PUB="$(sed -n 2p "$WORK/test.pub")"
    minisign -G -W -f -p "$WORK/other.pub" -s "$WORK/other.key" >/dev/null 2>&1
    OTHER_PUB="$(sed -n 2p "$WORK/other.pub")"
fi

release() { # [fake_release.py options...] ; signs when minisign is available
    sign=()
    [ "$HAVE_MINISIGN" = 1 ] && sign=(--seckey "$SEC")
    python3 "$HERE/fake_release.py" --www "$WWW" --base "$BASE" "${sign[@]}" "$@" || { echo "fake_release failed"; exit 1; }
}

new_env() { # fresh HOME for every test
    T="$WORK/t$((++N))"
    mkdir -p "$T/home"
    HOME_DIR="$T/home"
}
N=0

inst() { # run install.sh with isolated HOME; OUT/RC are set
    OUT="$(HOME="$HOME_DIR" XDG_DATA_HOME="" SYNOFURB_TEST_LOG="$T/run.log" SYNOFURB_FORCE_OS="${FORCE_OS-linux}" SYNOFURB_FORCE_ARCH="${FORCE_ARCH-x86_64}" \
        sh "$INSTALL_SH" "$@" 2>&1 </dev/null)"
    RC=$?
}
mgmt() {
    OUT="$(sh "$MGMT_SH" "$@" 2>&1 </dev/null)"
    RC=$?
}
SIG="--insecure-skip-signature"
[ "$HAVE_MINISIGN" = 1 ] && SIG="--pubkey ${PUB}"
# shellcheck disable=SC2206
SIGARGS=($SIG)

echo "install.sh"
# ------------------------------------------------------------------------------------------------
begin "--help and bad options"
new_env
inst --help; expect_rc 0; expect_out "usage: install.sh"
inst --bogus; expect_rc 2; expect_out "unknown option"
inst --version; expect_rc 2; expect_out "needs a value"
inst --run; expect_rc 2
finish

begin "happy path: latest release, tar.gz, SHA-256 + signature, desktop entry, icon"
if [ "$HAVE_MINISIGN" = 0 ]; then skip "minisign not installed"; else
    new_env; release --platforms linux-x86_64
    inst "${SIGARGS[@]}"
    expect_rc 0; expect_out "Signature OK"; expect_out "SHA-256 OK"
    expect_exists "$HOME_DIR/.local/bin/synofurb"
    check "binary not executable" "$([ -x "$HOME_DIR/.local/bin/synofurb" ]; echo $?)"
    expect_exists "$HOME_DIR/.local/share/applications/synofurb.desktop"
    expect_exists "$HOME_DIR/.local/share/icons/hicolor/256x256/apps/synofurb.png"
    check "desktop Exec wrong" "$(grep -q "^Exec=\"$HOME_DIR/.local/bin/synofurb\"" "$HOME_DIR/.local/share/applications/synofurb.desktop"; echo $?)"
    check "installed binary does not run" "$("$HOME_DIR/.local/bin/synofurb" --version | grep -q 'synofurb 0.2.0'; echo $?)"
    expect_absent "$HOME_DIR/.local/bin/.synofurb.new"
    finish
fi

begin "explicit --version, --dir with a space, SHA-256 only (--insecure-skip-signature)"
new_env; release --platforms linux-x86_64 --version 0.3.1
inst --version v0.3.1 --dir "$T/my bin" --insecure-skip-signature
expect_rc 0; expect_out "synofurb 0.3.1 for linux/x86_64"; expect_out "signature verification SKIPPED"
expect_exists "$T/my bin/synofurb"
inst --version 0.3.1 --dir "$T/my bin" --insecure-skip-signature   # idempotent re-install
expect_rc 0
finish

begin "minisign missing: refuses, explains how to install, proceeds only with the flag"
new_env; release --platforms linux-x86_64
OUT="$(HOME="$HOME_DIR" SYNOFURB_MINISIGN_BIN=/nonexistent/minisign SYNOFURB_FORCE_OS=linux SYNOFURB_FORCE_ARCH=x86_64 sh "$INSTALL_SH" --pubkey RWQexample 2>&1 </dev/null)"; RC=$?
expect_rc 1; expect_out "minisign"; expect_out "brew install minisign"; expect_out "--insecure-skip-signature"
expect_absent "$HOME_DIR/.local/bin/synofurb"
OUT="$(HOME="$HOME_DIR" SYNOFURB_MINISIGN_BIN=/nonexistent/minisign SYNOFURB_FORCE_OS=linux SYNOFURB_FORCE_ARCH=x86_64 sh "$INSTALL_SH" --insecure-skip-signature 2>&1 </dev/null)"; RC=$?
expect_rc 0; expect_exists "$HOME_DIR/.local/bin/synofurb"
finish

begin "no built-in public key and none given: refuses (development copy)"
if [ "$HAVE_MINISIGN" = 0 ]; then skip "minisign not installed"; else
    new_env; release --platforms linux-x86_64
    inst; expect_rc 1; expect_out "no minisign public key"
    finish
fi

begin "wrong public key / forged signature is rejected"
if [ "$HAVE_MINISIGN" = 0 ]; then skip "minisign not installed"; else
    new_env; release --platforms linux-x86_64
    inst --pubkey "$OTHER_PUB"; expect_rc 1; expect_out "SIGNATURE VERIFICATION FAILED"
    expect_absent "$HOME_DIR/.local/bin/synofurb"
    release --platforms linux-x86_64 --mutate sums-after-sign
    inst --pubkey "$PUB"; expect_rc 1; expect_out "SIGNATURE VERIFICATION FAILED"
    expect_absent "$HOME_DIR/.local/bin/synofurb"
    finish
fi

begin "checksum mismatch: nothing installed"
new_env; release --platforms linux-x86_64 --mutate asset
inst "${SIGARGS[@]}"; expect_rc 1; expect_out "SHA-256 MISMATCH"
expect_absent "$HOME_DIR/.local/bin/synofurb"; expect_absent "$HOME_DIR/.local/share/applications/synofurb.desktop"
finish

begin "asset not listed in SHA256SUMS is refused"
new_env; release --platforms linux-x86_64 --mutate unlisted
inst "${SIGARGS[@]}"; expect_rc 1; expect_out "no asset for linux/x86_64"
expect_absent "$HOME_DIR/.local/bin/synofurb"
finish

begin "wrong arch: clear error"
new_env; release --platforms linux-x86_64
FORCE_ARCH=aarch64 inst "${SIGARGS[@]}"; expect_rc 1; expect_out "no asset for linux/aarch64"
FORCE_ARCH=riscv64 inst "${SIGARGS[@]}"; expect_rc 1; expect_out "unsupported CPU architecture"
FORCE_OS=freebsd inst "${SIGARGS[@]}"; expect_rc 1; expect_out "unsupported operating system"
expect_absent "$HOME_DIR/.local/bin/synofurb"
finish

begin "missing release (404) and malicious tag from the API"
new_env; release --platforms linux-x86_64
inst --version 9.9.9 "${SIGARGS[@]}"; expect_rc 1; expect_out "does release v9.9.9 exist"
inst --version "1.0; rm -rf /" ; expect_rc 1; expect_out "invalid --version"
release --platforms linux-x86_64 --mutate bad-tag
inst "${SIGARGS[@]}"; expect_rc 1; expect_out "refusing unexpected release tag"
expect_absent /tmp/pwned
finish

begin "archive with path traversal / links is rejected"
new_env; release --platforms linux-x86_64 --mutate evil-tar
inst "${SIGARGS[@]}"; expect_rc 1; expect_out "absolute or parent-directory"
expect_absent "$HOME_DIR/.local/bin/synofurb"; expect_absent "$T/evil.txt"
release --platforms linux-x86_64 --mutate link-tar
inst "${SIGARGS[@]}"; expect_rc 1; expect_out "links or special files"
release --platforms linux-x86_64 --mutate no-binary
inst "${SIGARGS[@]}"; expect_rc 1; expect_out "does not contain a regular file named 'synofurb'"
finish

begin "--dry-run changes nothing and downloads no asset"
new_env; release --platforms linux-x86_64
inst --dry-run --dir "$T/bin"; expect_rc 0; expect_out "[dry-run] would download"; expect_out "synofurb-0.2.0-linux-x86_64.tar.gz"; expect_out "nothing was downloaded or changed"
expect_absent "$T/bin"; expect_absent "$HOME_DIR/.local"
inst --dry-run --version 0.2.0 --run scan --json; expect_rc 0; expect_out "would run:"; expect_out "scan --json"
FORCE_OS=macos FORCE_ARCH=aarch64 inst --dry-run --format dmg --version 0.2.0; expect_rc 0; expect_out "synofurb-0.2.0-macos-aarch64.dmg"
finish

begin "--uninstall removes binary, desktop entry and icon; --dry-run --uninstall only reports"
new_env; release --platforms linux-x86_64
inst --insecure-skip-signature; expect_rc 0
inst --uninstall --dry-run; expect_rc 0; expect_out "would remove"; expect_exists "$HOME_DIR/.local/bin/synofurb"
inst --uninstall; expect_rc 0; expect_out "Removed"
expect_absent "$HOME_DIR/.local/bin/synofurb"; expect_absent "$HOME_DIR/.local/share/applications/synofurb.desktop"
expect_absent "$HOME_DIR/.local/share/icons/hicolor/256x256/apps/synofurb.png"
inst --uninstall; expect_rc 0; expect_out "Nothing to remove"
finish

begin "--run passes the subcommand and its arguments to the installed binary"
new_env; release --platforms linux-x86_64
inst --insecure-skip-signature --run scan --json --no-sweep; expect_rc 0
check "binary not called with the right arguments" "$(grep -qx 'scan --json --no-sweep' "$T/run.log"; echo $?)"
OUT="$(HOME="$HOME_DIR" SYNOFURB_TEST_EXIT=7 SYNOFURB_FORCE_OS=linux SYNOFURB_FORCE_ARCH=x86_64 sh "$INSTALL_SH" --insecure-skip-signature --run scan 2>&1 </dev/null)"; RC=$?
expect_rc 7   # the exit status of synofurb is propagated
finish

begin "works when piped into sh (curl | sh) incl. --run"
new_env; release --platforms linux-x86_64
OUT="$(HOME="$HOME_DIR" SYNOFURB_TEST_LOG="$T/run.log" SYNOFURB_FORCE_OS=linux SYNOFURB_FORCE_ARCH=x86_64 sh -s -- --insecure-skip-signature --dir "$T/pbin" --run plan --host 10.0.0.2 <"$INSTALL_SH" 2>&1)"; RC=$?
expect_rc 0; expect_exists "$T/pbin/synofurb"
check "piped --run arguments lost" "$(grep -qx 'plan --host 10.0.0.2' "$T/run.log"; echo $?)"
finish

begin "--system with --dir as root-or-sudo-less test target; XDG_DATA_HOME honoured for user installs"
new_env; release --platforms linux-x86_64
OUT="$(HOME="$HOME_DIR" XDG_DATA_HOME="$T/xdg" SYNOFURB_FORCE_OS=linux SYNOFURB_FORCE_ARCH=x86_64 sh "$INSTALL_SH" --insecure-skip-signature 2>&1 </dev/null)"; RC=$?
expect_rc 0; expect_exists "$T/xdg/applications/synofurb.desktop"
inst --uninstall >/dev/null
if [ "$(id -u)" = 0 ]; then
    inst --system --dir "$T/sys/usr/local/bin" --insecure-skip-signature; expect_rc 0
    expect_exists "$T/sys/usr/local/bin/synofurb"; expect_exists "$T/sys/usr/local/share/applications/synofurb.desktop"
    inst --system --dir "$T/sys/usr/local/bin" --uninstall; expect_rc 0
    expect_absent "$T/sys/usr/local/bin/synofurb"
fi
finish

begin "--no-desktop-entry and relative --dir"
new_env; release --platforms linux-x86_64
inst --insecure-skip-signature --no-desktop-entry; expect_rc 0; expect_absent "$HOME_DIR/.local/share/applications/synofurb.desktop"
inst --insecure-skip-signature --dir relative/bin; expect_rc 1; expect_out "absolute path"
inst --insecure-skip-signature --dir '/tmp/we"ird'; expect_rc 1; expect_out "must not contain"
finish

begin "macOS: bare tar.gz install and --format errors"
new_env; release --platforms macos-aarch64
FORCE_OS=macos FORCE_ARCH=aarch64 inst --insecure-skip-signature; expect_rc 0
expect_exists "$HOME_DIR/.local/bin/synofurb"; expect_absent "$HOME_DIR/.local/share/applications/synofurb.desktop"
FORCE_OS=macos FORCE_ARCH=aarch64 inst --insecure-skip-signature --format appimage; expect_rc 1; expect_out "not available on macos"
FORCE_OS=macos FORCE_ARCH=aarch64 inst --uninstall; expect_rc 0
finish

begin "Linux AppImage format"
new_env; release --platforms linux-x86_64
inst --insecure-skip-signature --format appimage; expect_rc 0; expect_out "AppImage"
expect_exists "$HOME_DIR/.local/bin/synofurb"
finish

begin "--cli: installs only synofurb-cli (no desktop entry, no icon, no desktop binary), SHA-256 + signature"
new_env; release --platforms linux-x86_64
inst --cli "${SIGARGS[@]}"
expect_rc 0; expect_out "synofurb-cli 0.2.0 for linux/x86_64"; expect_out "synofurb-cli-0.2.0-linux-x86_64.tar.gz"; expect_out "SHA-256 OK"
[ "$HAVE_MINISIGN" = 1 ] && expect_out "Signature OK"
expect_exists "$HOME_DIR/.local/bin/synofurb-cli"
check "cli binary not executable" "$([ -x "$HOME_DIR/.local/bin/synofurb-cli" ]; echo $?)"
expect_absent "$HOME_DIR/.local/bin/synofurb"
expect_absent "$HOME_DIR/.local/share/applications/synofurb.desktop"; expect_absent "$HOME_DIR/.local/share/icons"
check "installed cli does not run" "$("$HOME_DIR/.local/bin/synofurb-cli" --version | grep -q 'synofurb-cli 0.2.0'; echo $?)"
expect_out "synofurb-cli --help"
finish

begin "--cli: explicit --version, --dir, idempotent; coexists with the desktop install; --uninstall --cli removes only the CLI"
new_env; release --platforms linux-x86_64 --version 0.3.1
inst --cli --version v0.3.1 --dir "$T/bin" --insecure-skip-signature; expect_rc 0; expect_exists "$T/bin/synofurb-cli"
inst --cli --version 0.3.1 --dir "$T/bin" --insecure-skip-signature; expect_rc 0
inst --version 0.3.1 --dir "$T/bin" --insecure-skip-signature; expect_rc 0; expect_exists "$T/bin/synofurb"
inst --cli --dir "$T/bin" --uninstall --dry-run; expect_rc 0; expect_out "would remove $T/bin/synofurb-cli"; expect_exists "$T/bin/synofurb-cli"
inst --cli --dir "$T/bin" --uninstall; expect_rc 0; expect_out "Removed $T/bin/synofurb-cli"
expect_absent "$T/bin/synofurb-cli"; expect_exists "$T/bin/synofurb"
inst --cli --dir "$T/bin" --uninstall; expect_rc 0; expect_out "Nothing to remove"
finish

begin "--cli: --dry-run names the cli asset and no desktop entry; --run passes arguments to synofurb-cli; macOS and wrong arch"
new_env; release --platforms linux-x86_64,macos-aarch64
inst --cli --dry-run --dir "$T/bin"; expect_rc 0; expect_out "synofurb-cli-0.2.0-linux-x86_64.tar.gz"; expect_not_out "desktop entry"; expect_not_out "synofurb-0.2.0-linux-x86_64.tar.gz"
expect_absent "$T/bin"
inst --cli --insecure-skip-signature --run scan --json --no-sweep; expect_rc 0; expect_out "Running: synofurb-cli scan --json --no-sweep"
check "synofurb-cli not called with the right arguments" "$(grep -qx 'scan --json --no-sweep' "$T/run.log"; echo $?)"
FORCE_OS=macos FORCE_ARCH=aarch64 inst --cli --insecure-skip-signature; expect_rc 0; expect_exists "$HOME_DIR/.local/bin/synofurb-cli"
FORCE_ARCH=aarch64 inst --cli --insecure-skip-signature; expect_rc 1; expect_out "no asset for linux/aarch64"; expect_out "synofurb-cli-0.2.0-linux-aarch64.tar.gz"
inst --cli --format appimage --insecure-skip-signature; expect_rc 1; expect_out "--cli is only available as --format tar"
finish

begin "--cli: tampered / unlisted / hostile archives are refused and nothing is installed"
new_env; release --platforms linux-x86_64 --mutate cli-asset
inst --cli "${SIGARGS[@]}"; expect_rc 1; expect_out "SHA-256 MISMATCH"; expect_absent "$HOME_DIR/.local/bin/synofurb-cli"
release --platforms linux-x86_64 --mutate cli-unlisted
inst --cli "${SIGARGS[@]}"; expect_rc 1; expect_out "no asset for linux/x86_64"; expect_absent "$HOME_DIR/.local/bin/synofurb-cli"
release --platforms linux-x86_64 --mutate cli-evil-tar
inst --cli "${SIGARGS[@]}"; expect_rc 1; expect_out "absolute or parent-directory"; expect_absent "$HOME_DIR/.local/bin/synofurb-cli"; expect_absent "$T/evil-cli.txt"
release --platforms linux-x86_64 --mutate cli-link-tar
inst --cli "${SIGARGS[@]}"; expect_rc 1; expect_out "links or special files"
release --platforms linux-x86_64 --mutate cli-no-binary
inst --cli "${SIGARGS[@]}"; expect_rc 1; expect_out "does not contain a regular file named 'synofurb-cli'"
if [ "$HAVE_MINISIGN" = 1 ]; then
    release --platforms linux-x86_64 --mutate sums-after-sign
    inst --cli --pubkey "$PUB"; expect_rc 1; expect_out "SIGNATURE VERIFICATION FAILED"; expect_absent "$HOME_DIR/.local/bin/synofurb-cli"
fi
finish

begin "--cli works piped into sh (curl | sh -s -- --cli --run ...)"
new_env; release --platforms linux-x86_64
OUT="$(HOME="$HOME_DIR" SYNOFURB_TEST_LOG="$T/run.log" SYNOFURB_FORCE_OS=linux SYNOFURB_FORCE_ARCH=x86_64 sh -s -- --cli --insecure-skip-signature --dir "$T/pbin" --run wizard --demo <"$INSTALL_SH" 2>&1)"; RC=$?
expect_rc 0; expect_exists "$T/pbin/synofurb-cli"
check "piped --run arguments lost" "$(grep -qx 'wizard --demo' "$T/run.log"; echo $?)"
finish

begin "macOS dmg install with a fake hdiutil"
new_env; release --platforms macos-aarch64
mkdir -p "$T/fakebin" "$T/fakedmg/synofurb.app/Contents/MacOS"
printf '#!/bin/sh\necho app\n' >"$T/fakedmg/synofurb.app/Contents/MacOS/synofurb"; chmod +x "$T/fakedmg/synofurb.app/Contents/MacOS/synofurb"
cat >"$T/fakebin/hdiutil" <<HD
#!/bin/sh
case "\$1" in
  attach) while [ \$# -gt 0 ]; do [ "\$1" = -mountpoint ] && mp="\$2"; shift; done; cp -R "$T/fakedmg/synofurb.app" "\$mp/";;
  detach) exit 0;;
esac
HD
chmod +x "$T/fakebin/hdiutil"
PATH="$T/fakebin:$PATH" FORCE_OS=macos FORCE_ARCH=aarch64 inst --insecure-skip-signature --format dmg; expect_rc 0
expect_exists "$HOME_DIR/Applications/synofurb.app/Contents/MacOS/synofurb"
expect_exists "$HOME_DIR/.local/bin/synofurb"
FORCE_OS=macos FORCE_ARCH=aarch64 inst --uninstall; expect_rc 0
expect_absent "$HOME_DIR/Applications/synofurb.app"; expect_absent "$HOME_DIR/.local/bin/synofurb"
finish

begin "release procedure: a copy with the real public key pasted in verifies without --pubkey"
if [ "$HAVE_MINISIGN" = 0 ]; then skip "minisign not installed"; else
    new_env; release --platforms linux-x86_64
    sed "s|^MINISIGN_PUBKEY_BUILTIN=.*|MINISIGN_PUBKEY_BUILTIN=\"$PUB\"|" "$INSTALL_SH" >"$T/install-keyed.sh"
    OUT="$(HOME="$HOME_DIR" SYNOFURB_FORCE_OS=linux SYNOFURB_FORCE_ARCH=x86_64 sh "$T/install-keyed.sh" 2>&1 </dev/null)"; RC=$?
    expect_rc 0; expect_out "Signature OK"; expect_exists "$HOME_DIR/.local/bin/synofurb"
    sed "s|^MINISIGN_PUBKEY_BUILTIN=.*|MINISIGN_PUBKEY_BUILTIN=\"$PUB\"|" "$MGMT_SH" >"$T/mgmt-keyed.sh"
    OUT="$(sh "$T/mgmt-keyed.sh" --root "$T/rk" 2>&1 </dev/null)"; RC=$?
    expect_rc 0; expect_out "Signature OK"
    finish
fi

echo "apps/mgmt-plane/install.sh"
# ------------------------------------------------------------------------------------------------
begin "mgmt: stage into --root, enable unit, idempotent re-run"
new_env; release --platforms linux-x86_64 --mgmt-version 0.1.0
R="$T/root"
mgmt --root "$R" "${SIGARGS[@]}"; expect_rc 0; expect_out "SHA-256 OK"; expect_out "Installed synofurb-mgmt 0.1.0"
expect_exists "$R/opt/synofurb-mgmt/synofurb_mgmt.py"; expect_exists "$R/opt/synofurb-mgmt/VERSION"
expect_exists "$R/etc/systemd/system/synofurb-mgmt.service"
check "unit not enabled (wants symlink)" "$([ -L "$R/etc/systemd/system/multi-user.target.wants/synofurb-mgmt.service" ]; echo $?)"
check "script not executable" "$([ -x "$R/opt/synofurb-mgmt/synofurb_mgmt.py" ]; echo $?)"
mgmt --root "$R" "${SIGARGS[@]}"; expect_rc 0; expect_out "already installed and unchanged"
finish

begin "mgmt: upgrade replaces files and keeps state"
new_env; release --platforms linux-x86_64 --mgmt-version 0.1.0
R="$T/root"; mgmt --root "$R" "${SIGARGS[@]}"; expect_rc 0
mkdir -p "$R/var/lib/synofurb-mgmt"; echo cert >"$R/var/lib/synofurb-mgmt/cert.pem"
release --platforms linux-x86_64 --version 0.4.0 --mgmt-version 0.2.0
mgmt --root "$R" "${SIGARGS[@]}"; expect_rc 0; expect_out "Upgraded synofurb-mgmt 0.1.0 -> 0.2.0"
check "VERSION not updated" "$(grep -qx 0.2.0 "$R/opt/synofurb-mgmt/VERSION"; echo $?)"
expect_exists "$R/var/lib/synofurb-mgmt/cert.pem"
finish

begin "mgmt: --version searches releases, --release picks a tag"
new_env; release --platforms linux-x86_64 --mgmt-version 0.1.0
mgmt --root "$T/r1" --version 0.1.0 --dry-run; expect_rc 0; expect_out "synofurb-mgmt 0.1.0 (release v0.2.0"
mgmt --root "$T/r1" --release v0.2.0 --dry-run; expect_rc 0
mgmt --root "$T/r1" --version 7.7.7 --dry-run; expect_rc 1; expect_out "no synofurb-mgmt"
mgmt --root "$T/r1" --version 0.1.0 --release v0.2.0 --dry-run; expect_rc 2
expect_absent "$T/r1"
finish

begin "mgmt: checksum mismatch, bad signature, bad archives"
new_env; release --platforms linux-x86_64 --mutate asset
# the asset mutated above is the desktop tar; build a fresh tree whose mgmt tar is mismatched via sums-after-sign
if [ "$HAVE_MINISIGN" = 1 ]; then
    release --platforms linux-x86_64 --mutate sums-after-sign
    mgmt --root "$T/r2" --pubkey "$PUB"; expect_rc 1; expect_out "SIGNATURE VERIFICATION FAILED"; expect_absent "$T/r2/opt"
fi
release --platforms linux-x86_64 --mutate mgmt-wrong-version
mgmt --root "$T/r3" --insecure-skip-signature; expect_rc 1; expect_out "does not match its file name"; expect_absent "$T/r3/opt"
release --platforms linux-x86_64 --mutate mgmt-no-version
mgmt --root "$T/r4" --insecure-skip-signature; expect_rc 1; expect_out "missing VERSION"
# tamper with the mgmt asset itself
release --platforms linux-x86_64
printf 'x' >>"$WWW/dl/v0.2.0/synofurb-mgmt-0.1.0.tar.gz"
mgmt --root "$T/r5" --insecure-skip-signature; expect_rc 1; expect_out "SHA-256 MISMATCH"; expect_absent "$T/r5/opt"
finish

begin "mgmt: minisign missing is an error unless skipped explicitly"
new_env; release --platforms linux-x86_64
OUT="$(SYNOFURB_MINISIGN_BIN=/nonexistent/minisign sh "$MGMT_SH" --root "$T/r6" --pubkey RWQexample 2>&1 </dev/null)"; RC=$?
expect_rc 1; expect_out "not installed"; expect_absent "$T/r6/opt"
finish

begin "mgmt: --uninstall keeps state, --purge removes it, --dry-run touches nothing"
new_env; release --platforms linux-x86_64
R="$T/root"; mgmt --root "$R" --insecure-skip-signature; expect_rc 0
mkdir -p "$R/var/lib/synofurb-mgmt"; echo cert >"$R/var/lib/synofurb-mgmt/cert.pem"
mgmt --root "$R" --uninstall --dry-run; expect_rc 0; expect_exists "$R/opt/synofurb-mgmt"
mgmt --root "$R" --uninstall; expect_rc 0
expect_absent "$R/opt/synofurb-mgmt"; expect_absent "$R/etc/systemd/system/synofurb-mgmt.service"
expect_absent "$R/etc/systemd/system/multi-user.target.wants/synofurb-mgmt.service"; expect_exists "$R/var/lib/synofurb-mgmt/cert.pem"
mgmt --root "$R" --uninstall --purge; expect_rc 0; expect_absent "$R/var/lib/synofurb-mgmt"
mgmt --root "$R" --uninstall; expect_rc 0   # idempotent
finish

begin "mgmt: refuses to run as non-root without --root (outside dry-run)"
if [ "$(id -u)" = 0 ]; then skip "running as root"; else
    new_env; release --platforms linux-x86_64
    mgmt --insecure-skip-signature; expect_rc 1; expect_out "run as root"
    finish
fi

echo "install.ps1 (pwsh on this host; msiexec, registry and shortcuts are not exercised)"
# ------------------------------------------------------------------------------------------------
PWSH="$(command -v pwsh || true)"
cat >"$WORK/msiexec" <<'FM'
#!/bin/sh
printf '%s\n' "$*" >> "$MSILOG"
exit "${MSI_EXIT:-0}"
FM
chmod +x "$WORK/msiexec"
ps() { # run install.ps1 in test mode; OUT/RC are set
    OUT="$(SYNOFURB_PS_TEST=1 SYNOFURB_FORCE_ARCH="${PS_ARCH-x86_64}" SYNOFURB_MSIEXEC="${FAKE_MSIEXEC-}" HOME="$HOME_DIR" \
        "$PWSH" -NoProfile -NonInteractive -File "$REPO/install.ps1" "$@" 2>&1 </dev/null)"
    RC=$?
}
if [ -z "$PWSH" ]; then
    begin "install.ps1 tests"; skip "pwsh not installed"
else
    begin "ps1: parses without errors"
    OUT="$(PS1="$REPO/install.ps1" "$PWSH" -NoProfile -Command '$e=$null;$t=$null;[void][System.Management.Automation.Language.Parser]::ParseFile($env:PS1,[ref]$t,[ref]$e); $e | ForEach-Object { $_.ToString() }; exit $e.Count' 2>&1)"; RC=$?
    expect_rc 0
    finish

    begin "ps1: -DryRun (latest via API, MSI default, -Portable, -User, -Run array)"
    new_env; release --platforms windows-x86_64
    ps -DryRun; expect_rc 0; expect_out "synofurb-0.2.0-windows-x86_64.msi"; expect_out "msiexec /i"; expect_out "nothing was downloaded or changed"
    ps -DryRun -Portable -Version v0.2.0; expect_rc 0; expect_out "synofurb-0.2.0-windows-x86_64.zip"; expect_out "would extract"
    OUT="$(PS1="$REPO/install.ps1" SYNOFURB_PS_TEST=1 SYNOFURB_FORCE_ARCH=x86_64 "$PWSH" -NoProfile -NonInteractive -Command '& $env:PS1 -DryRun -User -Run scan,--json' 2>&1)"; RC=$?
    expect_rc 0; expect_out "Start Menu shortcut"; expect_out "would run: synofurb scan --json"
    ps -DryRun -Dir "$T/d dir" -Version 0.2.0; expect_rc 0; expect_out "INSTALLDIR="
    finish

    begin "ps1: scriptblock form  & ([scriptblock]::Create(...)) -DryRun"
    new_env; release --platforms windows-x86_64
    OUT="$(PS1="$REPO/install.ps1" SYNOFURB_PS_TEST=1 SYNOFURB_FORCE_ARCH=x86_64 "$PWSH" -NoProfile -NonInteractive -Command '& ([scriptblock]::Create((Get-Content -Raw -LiteralPath $env:PS1))) -DryRun -Version 0.2.0 -Run scan' 2>&1)"; RC=$?
    expect_rc 0; expect_out "would run: synofurb scan"
    finish

    begin "ps1: portable install with SHA-256 only, then -Uninstall"
    new_env; release --platforms windows-x86_64
    ps -Portable -Dir "$T/pdir" -InsecureSkipSignature; expect_rc 0; expect_out "SHA-256 OK"; expect_out "signature verification SKIPPED"
    expect_exists "$T/pdir/synofurb.exe"; expect_exists "$T/pdir/LICENSE"
    ps -Uninstall -Dir "$T/pdir"; expect_rc 0; expect_absent "$T/pdir"
    ps -Uninstall -Dir "$T/pdir"; expect_rc 0; expect_out "Nothing to remove"
    finish

    begin "ps1: no minisign -> loud warning but SHA-256 install; -RequireSignature refuses"
    new_env; release --platforms windows-x86_64
    OUT="$(SYNOFURB_PS_TEST=1 SYNOFURB_FORCE_ARCH=x86_64 SYNOFURB_MINISIGN_BIN=/nonexistent/minisign "$PWSH" -NoProfile -NonInteractive -File "$REPO/install.ps1" -Portable -Dir "$T/pdir" 2>&1 </dev/null)"; RC=$?
    expect_rc 0; expect_out "minisign is not installed"; expect_exists "$T/pdir/synofurb.exe"
    OUT="$(SYNOFURB_PS_TEST=1 SYNOFURB_FORCE_ARCH=x86_64 SYNOFURB_MINISIGN_BIN=/nonexistent/minisign "$PWSH" -NoProfile -NonInteractive -File "$REPO/install.ps1" -Portable -Dir "$T/pdir2" -RequireSignature 2>&1 </dev/null)"; RC=$?
    expect_rc 1; expect_absent "$T/pdir2"
    finish

    begin "ps1: minisign signature verified / forged / wrong key"
    if [ "$HAVE_MINISIGN" = 0 ]; then skip "minisign not installed"; else
        new_env; release --platforms windows-x86_64
        ps -Portable -Dir "$T/pdir" -PublicKey "$PUB" -RequireSignature; expect_rc 0; expect_out "Signature OK"; expect_exists "$T/pdir/synofurb.exe"
        ps -Portable -Dir "$T/pdir3" -PublicKey "$OTHER_PUB"; expect_rc 1; expect_out "SIGNATURE VERIFICATION FAILED"; expect_absent "$T/pdir3"
        release --platforms windows-x86_64 --mutate sums-after-sign
        ps -Portable -Dir "$T/pdir4" -PublicKey "$PUB"; expect_rc 1; expect_out "SIGNATURE VERIFICATION FAILED"; expect_absent "$T/pdir4"
        finish
    fi

    begin "ps1: a copy with the real public key pasted in verifies without -PublicKey"
    if [ "$HAVE_MINISIGN" = 0 ]; then skip "minisign not installed"; else
        new_env; release --platforms windows-x86_64
        sed "s|^\( *\)\$BuiltinKey = .*|\1\$BuiltinKey = '$PUB'|" "$REPO/install.ps1" >"$T/install-keyed.ps1"
        OUT="$(SYNOFURB_PS_TEST=1 SYNOFURB_FORCE_ARCH=x86_64 "$PWSH" -NoProfile -NonInteractive -File "$T/install-keyed.ps1" -Portable -Dir "$T/pk" -RequireSignature 2>&1 </dev/null)"; RC=$?
        expect_rc 0; expect_out "Signature OK"; expect_exists "$T/pk/synofurb.exe"
        finish
    fi

    begin "ps1: checksum mismatch, wrong arch, unknown version, zip-slip, missing exe"
    new_env; release --platforms windows-x86_64 --mutate asset --tamper-ext zip
    ps -Portable -Dir "$T/pdir" -InsecureSkipSignature; expect_rc 1; expect_out "SHA-256 MISMATCH"; expect_absent "$T/pdir"
    release --platforms windows-x86_64
    PS_ARCH=aarch64 ps -Portable -Dir "$T/pdir" -InsecureSkipSignature; expect_rc 1; expect_out "no asset for windows/aarch64"
    PS_ARCH=x86 ps -Portable -Dir "$T/pdir" -InsecureSkipSignature; expect_rc 1; expect_out "unsupported CPU architecture"
    ps -Portable -Dir "$T/pdir" -InsecureSkipSignature -Version 9.9.9; expect_rc 1; expect_out "download failed"
    ps -Portable -Dir "$T/pdir" -InsecureSkipSignature -Version 'x;y'; expect_rc 1; expect_out "invalid -Version"
    ps -Portable -Dir relative/dir -InsecureSkipSignature; expect_rc 1; expect_out "absolute path"
    release --platforms windows-x86_64 --mutate zip-slip
    ps -Portable -Dir "$T/pdir" -InsecureSkipSignature; expect_rc 1; expect_out "escapes the install folder"; expect_absent "$T/pdir"
    release --platforms windows-x86_64 --mutate no-binary
    ps -Portable -Dir "$T/pdir" -InsecureSkipSignature; expect_rc 1; expect_out "does not contain synofurb.exe"
    finish

    begin "ps1: -Cli -DryRun, install with SHA-256 only, -AddToPath is Windows-only, -Uninstall -Cli"
    new_env; release --platforms windows-x86_64
    ps -Cli -DryRun -Dir "$T/cdir"; expect_rc 0; expect_out "synofurb-cli 0.2.0 for windows/x86_64"; expect_out "synofurb-cli-0.2.0-windows-x86_64.zip"; expect_out "would extract to $T/cdir"; expect_not_out "msiexec"
    OUT="$(PS1="$REPO/install.ps1" SYNOFURB_PS_TEST=1 SYNOFURB_FORCE_ARCH=x86_64 "$PWSH" -NoProfile -NonInteractive -Command '& $env:PS1 -Cli -DryRun -Run wizard,--demo' 2>&1)"; RC=$?
    expect_rc 0; expect_out "would run: synofurb-cli wizard --demo"
    ps -Cli -Portable -DryRun; expect_rc 1; expect_out "do not combine"
    ps -Cli -InsecureSkipSignature -Dir "$T/cdir"; expect_rc 0; expect_out "SHA-256 OK"; expect_exists "$T/cdir/synofurb-cli.exe"; expect_exists "$T/cdir/LICENSE"
    expect_absent "$T/cdir/synofurb.exe"
    ps -Cli -Uninstall -Dir "$T/cdir" -DryRun; expect_rc 0; expect_out "would remove"; expect_exists "$T/cdir"
    ps -Cli -Uninstall -Dir "$T/cdir"; expect_rc 0; expect_absent "$T/cdir"
    ps -Cli -Uninstall -Dir "$T/cdir"; expect_rc 0; expect_out "Nothing to remove"
    finish

    begin "ps1: -Cli refuses a tampered / zip-slip / binary-less archive and a missing arch"
    new_env; release --platforms windows-x86_64 --mutate cli-asset
    ps -Cli -InsecureSkipSignature -Dir "$T/cdir"; expect_rc 1; expect_out "SHA-256 MISMATCH"; expect_absent "$T/cdir"
    release --platforms windows-x86_64 --mutate cli-zip-slip
    ps -Cli -InsecureSkipSignature -Dir "$T/cdir"; expect_rc 1; expect_out "escapes the install folder"; expect_absent "$T/cdir"
    release --platforms windows-x86_64 --mutate cli-no-binary
    ps -Cli -InsecureSkipSignature -Dir "$T/cdir"; expect_rc 1; expect_out "does not contain synofurb-cli.exe"
    release --platforms windows-x86_64
    PS_ARCH=aarch64 ps -Cli -InsecureSkipSignature -Dir "$T/cdir"; expect_rc 1; expect_out "no asset for windows/aarch64"; expect_out "synofurb-cli-0.2.0-windows-aarch64.zip"
    if [ "$HAVE_MINISIGN" = 1 ]; then
        ps -Cli -Dir "$T/cdir2" -PublicKey "$PUB" -RequireSignature; expect_rc 0; expect_out "Signature OK"; expect_exists "$T/cdir2/synofurb-cli.exe"
    fi
    finish

    begin "ps1: MSI path calls msiexec /i ... /qn with INSTALLDIR and reports failure"
    new_env; release --platforms windows-x86_64
    MSILOG="$T/msi.log" FAKE_MSIEXEC="$WORK/msiexec" ps -InsecureSkipSignature -Dir "$T/pf"; expect_rc 0
    check "msiexec not called as expected" "$(grep -q -- "^/i .*synofurb-0.2.0-windows-x86_64.msi /qn /norestart /L\*v .* INSTALLDIR=$T/pf\$" "$T/msi.log"; echo $?)"
    # (a POSIX exit status is 8 bits, so real codes such as 1603/3010 cannot be faked here; any non-zero value exercises the failure path)
    MSI_EXIT=5 MSILOG="$T/msi.log" FAKE_MSIEXEC="$WORK/msiexec" ps -InsecureSkipSignature; expect_rc 1; expect_out "msiexec exit code 5"
    finish

    begin "ps1: piped into iex (irm | iex form) installs the MSI with defaults"
    new_env; release --platforms windows-x86_64
    OUT="$(PS1="$REPO/install.ps1" MSILOG="$T/msi.log" SYNOFURB_MSIEXEC="$WORK/msiexec" SYNOFURB_PS_TEST=1 SYNOFURB_FORCE_ARCH=x86_64 SYNOFURB_MINISIGN_BIN=/nonexistent "$PWSH" -NoProfile -NonInteractive -Command 'Get-Content -Raw -LiteralPath $env:PS1 | iex' 2>&1)"; RC=$?
    expect_rc 0; expect_out "SHA-256 OK"
    check "msiexec not invoked via iex" "$(grep -q '^/i ' "$T/msi.log"; echo $?)"
    finish
fi

echo
printf 'passed: %s  failed: %s  skipped: %s\n' "$PASS" "$FAIL" "$SKIP"
[ "$FAIL" = 0 ]
