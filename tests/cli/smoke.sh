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

# ------------------------------------------------------------------------------------------------ manage: a fake appliance behind a real sshd
# A second sshd whose ForceCommand runs every command through a wrapper that sends calls to /opt/synofurb-mgmt/synofurb_mgmt.py to the
# repository's script with `--root <sandbox> --dry-run` and moves /opt/synofurb-mgmt into the sandbox. So `manage health|version|update`
# really talk SSH, upload a tarball, stream the real updater's NDJSON and read the real health JSON, without touching this machine.
# (dry-run: systemctl is not called, so a real update ends in the health wait failing and the rollback path; the happy path is covered
# by the unit tests with a scripted unit.)
REPO_SCRIPT="${SYNOFURB_MGMT_SCRIPT:-$REPO/apps/mgmt-plane/synofurb_mgmt.py}"   # override to test another copy
if [ -z "$sshd_bin" ] || ! command -v ssh-keygen >/dev/null 2>&1; then
    skip "manage against a fake appliance" "sshd / ssh-keygen not installed"
elif [ "$(id -u)" != 0 ]; then
    skip "manage against a fake appliance" "need root"
elif [ "$HAVE_PY" = 0 ] || [ ! -f "$REPO_SCRIPT" ]; then
    skip "manage against a fake appliance" "python3 or $REPO_SCRIPT missing"
else
    F="$WORK/fakenas"
    FIX="$F/root"
    mkdir -p "$F" "$FIX/opt/synofurb-mgmt" /run/sshd
    printf '# old copy\n' >"$FIX/opt/synofurb-mgmt/synofurb_mgmt.py"
    printf '0.3.0\n' >"$FIX/opt/synofurb-mgmt/VERSION"
    cat >"$F/wrapper.sh" <<WRAP
#!/bin/sh
cmd=\$(printf '%s' "\$SSH_ORIGINAL_COMMAND" | sed -e 's#python3 /opt/synofurb-mgmt/synofurb_mgmt.py#python3 -I $REPO_SCRIPT --root $FIX --dry-run --health-timeout 1#g' -e 's#/opt/synofurb-mgmt#$FIX/opt/synofurb-mgmt#g')
exec /bin/sh -c "\$cmd"
WRAP
    chmod 755 "$F/wrapper.sh"
    ssh-keygen -q -t ed25519 -N '' -f "$F/hostkey" && ssh-keygen -q -t ed25519 -N '' -f "$F/id" && cp "$F/id.pub" "$F/authorized_keys"
    MPORT=$((20000 + RANDOM % 20000))
    cat >"$F/sshd_config" <<CFG
Port $MPORT
ListenAddress 127.0.0.1
HostKey $F/hostkey
PidFile $F/sshd.pid
AuthorizedKeysFile $F/authorized_keys
PasswordAuthentication no
PubkeyAuthentication yes
PermitRootLogin yes
UsePAM no
StrictModes no
LogLevel ERROR
ForceCommand $F/wrapper.sh
CFG
    if "$sshd_bin" -f "$F/sshd_config" 2>"$F/sshd.err" && sleep 1 && SSHD_PID="$(cat "$F/sshd.pid" 2>/dev/null)" && [ -n "$SSHD_PID" ]; then
        M="--host 127.0.0.1 --port $MPORT --user root --key $F/id"
        # shellcheck disable=SC2086
        cli manage health $M --json
        # no systemd services run here, so the unit's own verdict is warn (8) or crit (9); the document is printed either way
        if [ "$RC" = 8 ] || [ "$RC" = 9 ]; then ok "manage health: exit $RC for a unit with problems"; else bad "manage health: exit $RC, expected 8 or 9"; fi
        json_ok "manage health --json: document shape" "v['schema']==1 and v['health']['schema']==1 and v['overall'] in ('warn','crit') and v['summary']['problems']>=1 and v['web_ui'].endswith(':8443')"
        # shellcheck disable=SC2086
        cli manage health $M
        expect "manage health: readable report" "$RC" "management plane" "Problems:" "Web UI: https://127.0.0.1:8443"
        # shellcheck disable=SC2086
        cli manage version $M --json
        json_ok "manage version --json: installed vs bundled" "v['installed']['version']=='0.3.0' and v['bundled']['version'] and v['update_available'] is True and v['relation']=='nas_older'"
        # shellcheck disable=SC2086
        cli manage update $M --dry-run
        expect "manage update --dry-run changes nothing" 0 "Dry run, nothing was changed" "Update the management plane from 0.3.0"
        [ "$(cat "$FIX/opt/synofurb-mgmt/VERSION")" = "0.3.0" ] && ok "manage update --dry-run left the install alone" || bad "manage update --dry-run changed the install"
        # no terminal and no --yes: refused (3) before anything is changed
        # shellcheck disable=SC2086
        cli manage update $M
        expect "manage update without --yes is refused" 3 "--yes"
        # a real update through ssh: the upload, staging and the real updater's verification, swap and (dry-run) failure and rollback
        # shellcheck disable=SC2086
        cli --log-format json manage update $M --yes
        if [ "$RC" = 1 ] && printf '%s' "$OUTO" | python3 -I -c "
import json,sys
ev=[json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
assert ev[0]['event']=='start' and ev[0]['command']=='manage update', ev[0]
assert ev[-1]['event']=='failure' and ev[-1]['exit_code']==1 and 'Update failed' in ev[-1]['message'], ev[-1]
steps=[e['n'] for e in ev if e['event']=='step']
assert steps[:3]==[1,2,3], steps
logs=' '.join(e['msg'] for e in ev if e['event']=='log')
assert 'NAS verify: ok' in logs and 'NAS swap: ok' in logs and 'NAS health: fail' in logs, logs
res=[e for e in ev if e['event']=='result']
assert res and res[0]['ok'] is False and res[0]['data']['outcome']=='failed', res
"; then ok "manage update over ssh: staged, verified by the real updater, failure reported with exit 1"; else bad "manage update over ssh (exit $RC)"; fi
        ls /tmp/.synofurb-mgmt-stage-* >/dev/null 2>&1 && bad "manage update left a staging folder in /tmp" || ok "manage update cleaned its staging folder"
        # a --from-bundle folder that is not there is rejected before anything is contacted
        # shellcheck disable=SC2086
        cli manage update $M --yes --from-bundle "$F/older"
        expect "manage update --from-bundle with a missing folder is a usage error" 2 "--from-bundle"
        # shellcheck disable=SC2086
        cli manage rollback $M --yes
        # the update above left a previous copy, so the unit tries; its (dry-run) service does not answer, which is reported as a failure
        expect "manage rollback reports what the unit said" 1 "Rollback failed" "NAS rollback"
        # an offline bundle with a mgmt/ folder (what `bundle create` writes; hand-made here because building a real bundle needs the
        # Internet): bundle verify/info see it, and `manage update --from-bundle <bundle>` pushes exactly those files over ssh
        if python3 -I - "$F/bundle" "$REPO/apps/mgmt-plane/tools/make-update-bundle.py" <<'PYB'
import hashlib, json, os, subprocess, sys
d, tool = sys.argv[1], sys.argv[2]
os.makedirs(d + "/mirror/debian/dists/trixie")
open(d + "/mirror/debian/dists/trixie/InRelease", "w").write("Origin: Debian\nSuite: trixie\n")
subprocess.check_call([sys.executable, "-I", tool, d + "/mgmt"], stdout=subprocess.DEVNULL)
files = []
for root, _, names in os.walk(d):
    for n in sorted(names):
        p = os.path.join(root, n)
        data = open(p, "rb").read()
        files.append({"path": os.path.relpath(p, d), "size": len(data), "sha256": hashlib.sha256(data).hexdigest()})
files.sort(key=lambda f: f["path"])
arch = {"installer_kernel": None, "kernel_package_version": None, "kernel_from_backports": False, "packages": 0, "udebs": 0,
        "pool_files": [], "installer_files": [], "agent_files": []}
m = {"format": 1, "tool_version": "0.0.0", "created": "2026-11-01T00:00:00Z", "suite": "trixie", "suites": ["trixie"],
     "kernel": "Stable", "with_firmware": False, "extra_packages": [], "release_dates": {}, "pgp_verified": False,
     "warnings": [], "archs": {"amd64": arch}, "files": files}
json.dump(m, open(d + "/manifest.json", "w"), indent=1)
PYB
        then
            cli bundle verify "$F/bundle"
            expect "bundle verify: the mgmt/ folder is checked" 0 "management app" "in mgmt/ verified" "bundle OK"
            cli bundle info "$F/bundle"
            expect "bundle info names the management app" 0 "Mgmt app:  synofurb-mgmt"
            # shellcheck disable=SC2086
            cli manage update $M --dry-run --from-bundle "$F/bundle"
            expect "manage update --from-bundle <bundle> --dry-run" 0 "Dry run, nothing was changed" "$F/bundle/mgmt"
            # shellcheck disable=SC2086
            cli manage update $M --dry-run --from-bundle "$F/bundle/mgmt"
            expect "manage update --from-bundle <bundle>/mgmt --dry-run" 0 "Dry run, nothing was changed" "$F/bundle/mgmt"
            # shellcheck disable=SC2086
            cli manage update $M --yes --force --from-bundle "$F/bundle"
            # no systemd here: the real updater verifies and swaps, then the (dry-run) health wait fails and it rolls back (exit 1)
            expect "manage update from a bundle over ssh reaches the real updater" 1 "NAS verify: ok" "NAS swap: ok"
            ls /tmp/.synofurb-mgmt-stage-* >/dev/null 2>&1 && bad "bundle update left a staging folder in /tmp" || ok "bundle update cleaned its staging folder"
            printf '# tampered\n' >>"$F/bundle/mgmt/synofurb_mgmt.py"
            cli bundle verify "$F/bundle"
            expect "bundle verify finds a changed management app" 1 "CHANGED  mgmt/synofurb_mgmt.py" "MGMT"
            # shellcheck disable=SC2086
            cli manage update $M --yes --force --from-bundle "$F/bundle"
            expect "manage update refuses a tampered bundle before connecting" 2 "--from-bundle" "does not match"
        else
            skip "manage update --from-bundle <bundle>" "could not build the sample bundle"
        fi
        kill "$SSHD_PID" 2>/dev/null; SSHD_PID=""
    else
        skip "manage against a fake appliance" "sshd would not start here: $(head -c 200 "$F/sshd.err" 2>/dev/null)"
    fi
fi

# the commands that need no unit at all
cli manage open --host 192.168.1.20
expect "manage open prints the address" 0 "https://192.168.1.20:8443"
cli --log-format json manage open --host nas.lan
if [ "$HAVE_PY" = 1 ]; then
    if printf '%s' "$OUTO" | python3 -I -c "
import json,sys
ev=[json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
assert ev[0]['event']=='start' and ev[-1]['event']=='result' and ev[-1]['data']['url']=='https://nas.lan:8443', ev
"; then ok "manage open: NDJSON events"; else bad "manage open: NDJSON events"; fi
fi
cli manage health --host 127.0.0.1 --port 1
expect "manage health without a password and without a terminal is a usage error" 2 "SYNOFURB_PASSWORD"
cli_env SYNOFURB_PASSWORD=hunter2-not-printed -- manage health --host 127.0.0.1 --port 1
expect "manage health on a closed port is an error (1) that never prints the password" 1 "error:"
case "$OUT" in *hunter2-not-printed*) bad "the password was printed" ;; *) ok "the password was not printed" ;; esac

echo
printf 'passed: %s  failed: %s  skipped: %s\n' "$PASS" "$FAIL" "$SKIP"
[ "$FAIL" = 0 ]
