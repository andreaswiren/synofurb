#!/usr/bin/env bash
# Headless smoke test of the CLI-only program (synofurb-cli): it must start and work with a stripped environment, no display and no GUI
# libraries.
#
#   tests/cli/smoke.sh [path/to/synofurb-cli]        (default: $CARGO_TARGET_DIR/debug/synofurb-cli, else target/debug/synofurb-cli)
#
# What runs (all under `env -i`, PATH without any X/Wayland tool, a throw-away HOME):
#   * the binary links no graphical library (ldd), --help / --version / completions / version --json / compat / scan --no-sweep
#   * plan / inspect / install --dry-run on the built-in sample unit, JSON shapes, exit codes 2/3/4
#   * the wizard driven through a real pty (TERM=dumb, no colour): menu, wrong confirmation phrase (exit 3), a full rehearsal (exit 0)
#   * with sshd + ssh-keygen + root: inspect / plan / dry run / NDJSON event stream against a REAL local sshd (read-only commands only)
# Needs python3 for the JSON and pty checks (skipped with a notice when missing).
# shellcheck disable=SC2015
# (SC2015: `A && ok || bad` is used on purpose; ok never fails.)
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BIN="${1:-${CARGO_TARGET_DIR:-$REPO/target}/debug/synofurb-cli}"
[ -x "$BIN" ] || { echo "synofurb-cli not found at $BIN (build it: cargo build -p synofurb --no-default-features --bin synofurb-cli)"; exit 2; }
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/synofurb-cli-smoke.XXXXXX")"
SSHD_PID=""
cleanup() {
    [ -n "$SSHD_PID" ] && kill "$SSHD_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT
mkdir -p "$WORK/home"

PASS=0; FAIL=0; SKIP=0
ok() { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL %s\n' "$1"; [ -n "${OUT:-}" ] && printf '%s\n' "$OUT" | head -15 | sed 's/^/       | /'; }
skip() { SKIP=$((SKIP + 1)); printf '  skip %s (%s)\n' "$1" "$2"; }

# run the CLI with nothing but a minimal environment; sets OUT (stdout+stderr), OUTO (stdout), RC
cli() {
    OUTO="$(env -i PATH=/usr/bin:/bin HOME="$WORK/home" XDG_DATA_HOME="$WORK/data" XDG_CACHE_HOME="$WORK/cache" "$BIN" "$@" 2>"$WORK/stderr" </dev/null)"
    RC=$?
    OUT="$OUTO"$'\n'"$(cat "$WORK/stderr")"
}
cli_env() { # NAME=value ... -- args
    local envs=()
    while [ "$1" != "--" ]; do envs+=("$1"); shift; done
    shift
    OUTO="$(env -i PATH=/usr/bin:/bin HOME="$WORK/home" XDG_DATA_HOME="$WORK/data" XDG_CACHE_HOME="$WORK/cache" "${envs[@]}" "$BIN" "$@" 2>"$WORK/stderr" </dev/null)"
    RC=$?
    OUT="$OUTO"$'\n'"$(cat "$WORK/stderr")"
}
expect() { # description, expected rc, [needle ...]
    local desc="$1" rc="$2"
    shift 2
    if [ "$RC" != "$rc" ]; then bad "$desc: exit $RC, expected $rc"; return; fi
    for n in "$@"; do
        if ! printf '%s' "$OUT" | grep -qF -- "$n"; then bad "$desc: output lacks: $n"; return; fi
    done
    ok "$desc"
}
HAVE_PY=0
command -v python3 >/dev/null 2>&1 && HAVE_PY=1
json_ok() { # description, jq-ish python expression over variable v (stdin = stdout of last cli call)
    if [ "$HAVE_PY" = 0 ]; then skip "$1" "python3 not installed"; return; fi
    if printf '%s' "$OUTO" | python3 -I -c "import json,sys; v=json.load(sys.stdin); sys.exit(0 if ($2) else 1)" 2>/dev/null; then ok "$1"; else bad "$1 (JSON check failed: $2)"; fi
}

echo "synofurb-cli smoke test: $BIN"

# ------------------------------------------------------------------------------------------------ no GUI, starts anywhere
if command -v ldd >/dev/null 2>&1 && file "$BIN" 2>/dev/null | grep -q ELF; then
    deps="$(ldd "$BIN" 2>&1)"
    if printf '%s' "$deps" | grep -Eiq 'xkbcommon|libxcb|libX11|wayland|vulkan|freetype|fontconfig|libGL|libEGL|libdrm|libgbm|libasound'; then
        OUT="$deps"; bad "links a graphical library"
    else
        ok "links no graphical library (ldd)"
    fi
else
    skip "ldd check" "not an ELF binary or no ldd/file"
fi

cli --help
expect "--help exits 0 and lists the commands" 0 "scan" "inspect" "plan" "install" "wizard" "boot-manager" "recover" "bundle" "completions" "EXIT CODES"
if printf '%s' "$OUTO" | grep -qE '^  gui '; then bad "the headless build must not have a gui subcommand"; else ok "no gui subcommand"; fi
cli --version
expect "--version" 0 "synofurb-cli"
want="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$REPO/Cargo.toml" | head -1)"
[ -n "$want" ] && expect "--version matches Cargo.toml ($want)" 0 "$want"
cli version --json
json_ok "version --json" "v['name']=='synofurb-cli' and v['gui'] is False and v['schema']==1 and 'target' in v"
helpless=""
for sub in scan inspect plan install wizard boot-manager recover backup-dom disk-backup logs bundle fetch-installer self-update config completions version compat inventory; do
    cli "$sub" --help
    if [ "$RC" != 0 ] || ! printf '%s' "$OUTO" | grep -q 'Usage:'; then helpless="$helpless $sub"; fi
done
if [ -z "$helpless" ]; then ok "every subcommand answers --help"; else OUT="no --help for:$helpless"; bad "every subcommand answers --help"; fi
for sh in bash zsh fish powershell; do
    cli completions "$sh"
    if [ "$RC" = 0 ] && [ -n "$OUTO" ] && printf '%s' "$OUTO" | grep -q 'boot-manager'; then ok "completions $sh"; else bad "completions $sh"; fi
done
cli completions bash
printf '%s\n' "$OUTO" >"$WORK/bash-completion.sh"
bash -n "$WORK/bash-completion.sh" && ok "bash completion script parses" || bad "bash completion script has a syntax error"
cli compat DS923+
expect "compat DS923+" 0 "DS923+" "amd64"
cli compat DS923+ --json
json_ok "compat DS923+ --json" "v['found'] is True and v['entry']['model']=='DS923+' and v['schema']==1"
cli compat --json
json_ok "compat --json summary" "v['models']>100 and 'by_status' in v"
cli scan --no-sweep --timeout 1 --json
expect "scan --no-sweep --json works with no display" 0
json_ok "scan --json is a device list" "isinstance(v['devices'], list)"
cli config paths --json
json_ok "config paths --json" "v['logs'].endswith('logs')"

# ------------------------------------------------------------------------------------------------ on the sample unit
cli inspect --facts demo --json
json_ok "inspect --json" "v['confirm_phrase']=='ERASE DS918+' and v['facts']['model']=='DS918+' and v['supported'] is True"
cli plan --facts demo --json
json_ok "plan --json" "v['ok'] is True and v['plan']['mode']=='WipeAll' and len(v['summary'])>0"
cli plan --facts demo --json --mode keep --keep md99
expect "a plan with errors exits 4" 4
cli install --facts demo --dry-run --out "$WORK/review" --hostname vault
expect "install --dry-run --out" 0 "Dry run written to"
for f in stage2.sh preseed.cfg plan.json payload.tar.gz payload-manifest.json payload/env.sh; do
    [ -f "$WORK/review/$f" ] || { bad "dry run did not write $f"; continue; }
done
ok "dry run wrote the review files"
if sh -n "$WORK/review/stage2.sh" 2>/dev/null; then ok "generated stage2.sh is valid sh"; else bad "generated stage2.sh has a syntax error"; fi
grep -q "NAS_HOSTNAME='vault'" "$WORK/review/payload/env.sh" && ok "dry run honours --hostname" || bad "dry run ignored --hostname"
grep -q DRYRUNSALT "$WORK/review/payload/env.sh" && ok "dry run uses a placeholder password hash" || bad "dry run exposes a real password hash"
cli install --host 192.0.2.1
expect "install without the gate exits 3 before touching the network" 3 "refusing to continue" "--have-backup" "--accept-data-loss" "--confirm"
cli install --host 192.0.2.1 --yes --accept-data-loss --have-backup
expect "--yes does not replace the typed phrase" 3 "--confirm"
cli_env SYNOFURB_PASSWORD=x -- install --host 127.0.0.1 --port 1 --have-backup --accept-data-loss --confirm "ERASE DS918+"
expect "a confirmed install to a dead host fails with 1 and changes nothing" 1 "error:"
cli install --host x --password hunter2
expect "passwords on argv are rejected (usage, exit 2)" 2 "unexpected argument"
cli bogus
expect "unknown command exits 2" 2
cli wizard
expect "wizard without a terminal exits 2 and points at the flags" 2 "interactive terminal" "install --host H --dry-run"
cli --log-format json plan --facts /nonexistent
if [ "$RC" = 1 ] && [ "$HAVE_PY" = 1 ] && printf '%s' "$OUTO" | tail -1 | python3 -I -c "import json,sys; v=json.load(sys.stdin); sys.exit(0 if v['event']=='failure' and v['exit_code']==1 else 1)"; then ok "--log-format json reports failures as an NDJSON event"; else bad "NDJSON failure event"; fi
cli_env NO_COLOR=1 TERM=dumb -- plan --facts demo
if printf '%s' "$OUT" | grep -q $'\033'; then bad "escape codes with NO_COLOR/TERM=dumb"; else ok "no escape codes with NO_COLOR/TERM=dumb"; fi

# ------------------------------------------------------------------------------------------------ the wizard through a pty
if [ "$HAVE_PY" = 0 ]; then
    skip "wizard through a pty" "python3 not installed"
else
    for mode in menu refuse rehearse; do
        OUT="$(python3 -I "$HERE/wizard_pty.py" "$BIN" "$mode" 2>&1)"; RC=$?
        if [ "$RC" = 0 ]; then ok "wizard pty: $mode"; else bad "wizard pty: $mode"; fi
    done
fi

# ------------------------------------------------------------------------------------------------ against a real sshd (read-only)
sshd_bin="$(command -v sshd || ls /usr/sbin/sshd 2>/dev/null || true)"
if [ -z "$sshd_bin" ] || ! command -v ssh-keygen >/dev/null 2>&1; then
    skip "real sshd tests" "sshd / ssh-keygen not installed"
elif [ "$(id -u)" != 0 ]; then
    skip "real sshd tests" "need root (the inspection script runs privileged commands)"
else
    S="$WORK/sshd"
    mkdir -p "$S" /run/sshd
    ssh-keygen -q -t ed25519 -N '' -f "$S/hostkey" && ssh-keygen -q -t ed25519 -N '' -f "$S/id" && cp "$S/id.pub" "$S/authorized_keys"
    PORT=$((20000 + RANDOM % 20000))
    cat >"$S/sshd_config" <<CFG
Port $PORT
ListenAddress 127.0.0.1
HostKey $S/hostkey
PidFile $S/sshd.pid
AuthorizedKeysFile $S/authorized_keys
PasswordAuthentication no
PubkeyAuthentication yes
PermitRootLogin yes
UsePAM no
StrictModes no
LogLevel ERROR
CFG
    if "$sshd_bin" -f "$S/sshd_config" 2>"$S/sshd.err" && sleep 1 && SSHD_PID="$(cat "$S/sshd.pid" 2>/dev/null)" && [ -n "$SSHD_PID" ]; then
        T="--host 127.0.0.1 --port $PORT --user root --key $S/id"
        # shellcheck disable=SC2086
        cli inspect $T --json
        json_ok "real sshd: inspect --json (exit $RC)" "v['schema']==1 and v['facts']['arch'] in ('X86_64','Arm64','Armhf')"
        # shellcheck disable=SC2086
        cli inspect $T --save "$WORK/live-facts.json"
        expect "real sshd: inspect --save" 0 "Disks:"
        cli plan --facts "$WORK/live-facts.json" --json
        json_ok "plan from the saved live inspection" "v['schema']==1 and 'plan' in v"
        # shellcheck disable=SC2086
        cli install $T --dry-run --out "$WORK/live-review"
        expect "real sshd: install --dry-run reads the unit and renders" 0 "Nothing was changed on the NAS"
        [ -f "$WORK/live-review/stage2.sh" ] && ok "real sshd: dry run wrote stage2.sh" || bad "real sshd: dry run wrote nothing"
        # shellcheck disable=SC2086
        cli --log-format json backup-dom $T
        # this host has no Synology boot flash: the command must fail cleanly (exit 1) with an NDJSON failure event
        if [ "$HAVE_PY" = 1 ] && [ "$RC" = 1 ] && printf '%s' "$OUTO" | python3 -I -c "
import json,sys
ev=[json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
assert ev[0]['event']=='start' and ev[-1]['event']=='failure' and ev[-1]['exit_code']==1, ev
assert all('ts' in e for e in ev)
assert any(e['event']=='log' and e['level']=='info' for e in ev)
"; then ok "real sshd: NDJSON event stream (start, log lines, failure) for a failing job"; else bad "real sshd: NDJSON stream for backup-dom"; fi
        # shellcheck disable=SC2086
        cli boot-manager status $T --json
        json_ok "real sshd: boot-manager status --json" "v['can_use_boot_manager'] is False and v['reason']"
        # shellcheck disable=SC2086
        cli --log-format json disk-backup $T --out-dir "$WORK/diskbk"
        # on a normal machine this writes the partition-table backup (exit 0); a sandbox that forbids raw disk reads makes it fail (exit 1)
        if [ "$HAVE_PY" = 1 ] && printf '%s' "$OUTO" | python3 -I -c "
import json,sys
ev=[json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
last=ev[-1]
assert last['event'] in ('result','failure'), last
"; then ok "real sshd: disk-backup ends with a result or failure event (exit $RC)"; else bad "real sshd: disk-backup event stream"; fi
        if [ "$RC" = 0 ]; then
            ls "$WORK"/diskbk/*/SHA256SUMS >/dev/null 2>&1 && ok "real sshd: disk-backup wrote SHA256SUMS" || bad "real sshd: disk-backup wrote no SHA256SUMS"
        fi
        kill "$SSHD_PID" 2>/dev/null; SSHD_PID=""
    else
        skip "real sshd tests" "sshd would not start here: $(head -c 200 "$S/sshd.err" 2>/dev/null)"
    fi
fi

echo
printf 'passed: %s  failed: %s  skipped: %s\n' "$PASS" "$FAIL" "$SKIP"
[ "$FAIL" = 0 ]
