#!/usr/bin/env bash
# QEMU smoke test of the synofurb boot agent (amd64, TCG - no KVM needed).
#
#   test/smoke.sh [--dist DIR] [--cache DIR] [--only "A B C D" (any subset)] [--keep]
#
# Two guests are booted from the built agent-amd64.vmlinuz/.initrd, each with a per-run overlay appended:
#   A  e1000 NIC, DHCP, key + one-time password, real synofurb-dom (no DOM in QEMU -> must fail gracefully):
#        DHCP v4 lease, SSH key login, sf-boot status/net/claim, kexec --version, UDP beacon (seen on the guest),
#        one-time password, sf-boot kexec of a second kernel from a local HTTP server (hash enforcement included)
#   B  virtio-net NIC, static address, fake synofurb-dom, AGENT_TIMEOUT_MIN=1 and NOBODY logs in:
#        the fail-safe call to synofurb-dom happens before the network is up, the watchdog reboots the guest
#        after ~1 minute (QEMU -no-reboot turns the guest reboot into a QEMU exit); an autorun script with a
#        wrong SHA-256 is refused
#   C  AGENT_WATCHDOG_MODE=claim + a failing autorun script: logins do not disarm, the failed autorun re-arms,
#        `sf-boot claim` disarms, `sf-boot arm 1` re-arms and the watchdog then fires
#   E  console handling: `console=tty0 console=uart8250,io,0x3f8,115200n8` (VGA + an 8250 port given by I/O address,
#        the PC counterpart of the mmio32 form): shells and banners on both
#   D  a good autorun script (hash pinned): it runs, claims the unit (no reboot although timeout is 1 minute);
#        `sf-boot reboot debian` calls synofurb-dom default debian and reboots
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
AGENT=$(dirname "$HERE")
DIST=$AGENT/dist
CACHE=${SYNOFURB_AGENT_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/synofurb-agent}
WORK=$HERE/work
ONLY=""
KEEP=0
PORT_A=${PORT_A:-2222}
PORT_B=${PORT_B:-2223}
HTTP_PORT=${HTTP_PORT:-8099}
BOOT_TIMEOUT=${BOOT_TIMEOUT:-420}

while [[ $# -gt 0 ]]; do
    case $1 in
        --dist) DIST=$(realpath "$2"); shift 2 ;;
        --cache) CACHE=$(realpath "$2"); shift 2 ;;
        --only) ONLY=$2; shift 2 ;;
        --keep) KEEP=1; shift ;;
        -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
done

PASS=0; FAIL=0
ok()   { PASS=$((PASS + 1)); printf '  PASS  %s\n' "$*"; }
bad()  { FAIL=$((FAIL + 1)); printf '  FAIL  %s\n' "$*"; }
check() { # description command...
    local d=$1; shift
    if "$@" >/dev/null 2>&1; then ok "$d"; else bad "$d"; fi
}
note() { printf '\n== %s\n' "$*"; }

for t in qemu-system-x86_64 ssh ssh-keygen python3 cpio gzip openssl; do
    command -v "$t" >/dev/null || { echo "missing test tool: $t" >&2; exit 2; }
done
[[ -f $DIST/agent-amd64.vmlinuz && -f $DIST/agent-amd64.initrd ]] || { echo "build first: ./build.sh amd64 (looked in $DIST)" >&2; exit 2; }

rm -rf "$WORK"; mkdir -p "$WORK"
KEY=$WORK/id_test
ssh-keygen -q -t ed25519 -N '' -C smoke-test -f "$KEY"
SSH_OPTS=(-i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR
          -o ConnectTimeout=8 -o BatchMode=yes -o IdentitiesOnly=yes)
gssh() { local port=$1; shift; ssh "${SSH_OPTS[@]}" -p "$port" root@127.0.0.1 "$@"; }

QPID=""
cleanup() {
    [[ -n $QPID ]] && kill "$QPID" 2>/dev/null
    [[ -n ${HPID:-} ]] && kill "$HPID" 2>/dev/null
    [[ -n ${HSPID:-} ]] && kill "$HSPID" 2>/dev/null
    [[ $KEEP == 1 ]] || true
}
trap cleanup EXIT

# start_guest NAME NIC_MODEL HOSTFWD_TARGET_IP HOSTPORT INITRD
start_guest() {
    local name=$1 nic=$2 gip=$3 port=$4 initrd=$5
    SERIAL=$WORK/serial-$name.log
    : > "$SERIAL"
    qemu-system-x86_64 -machine pc -cpu max -m 512 -smp 2 \
        -display none -monitor none -serial "file:$SERIAL" -no-reboot \
        -kernel "$DIST/agent-amd64.vmlinuz" -initrd "$initrd" \
        -append "${APPEND:-console=ttyS0,115200n8 loglevel=7 panic=-1}" \
        -netdev "user,id=n0,hostfwd=tcp:127.0.0.1:$port-${gip}:22" -device "$nic,netdev=n0,mac=52:54:00:12:34:5${name: -1}" \
        >"$WORK/qemu-$name.out" 2>&1 &
    QPID=$!
    T0=$SECONDS
}
wait_for() { # seconds description command...
    local limit=$1 d=$2; shift 2
    local end=$((SECONDS + limit))
    while (( SECONDS < end )); do
        "$@" >/dev/null 2>&1 && return 0
        kill -0 "$QPID" 2>/dev/null || return 2
        sleep 3
    done
    return 1
}
serial_has() { grep -q -- "$1" "$SERIAL"; }
stop_guest() { kill "$QPID" 2>/dev/null; wait "$QPID" 2>/dev/null; QPID=""; }

# ------------------------------------------------------------------ prepare artifacts
note "preparing test artifacts"
PWHASH=$(openssl passwd -6 'smoke-pass-1')
cat > "$WORK/fake-dom" <<'EOF'
#!/bin/sh
# test double for synofurb-dom: record the call on the kernel console and in /run
echo "<2>FAKE-DOM called: $* (uptime $(cut -d' ' -f1 /proc/uptime))" > /dev/kmsg
echo "$*" >> /run/fake-dom.calls
exit 0
EOF
chmod +x "$WORK/fake-dom"

"$HERE/make-overlay.sh" --out "$WORK/overlay-a.cpio.gz" --hostname smoke-a --pubkey "$KEY.pub" \
    --password-hash "$PWHASH" --timeout-min 20
cat "$DIST/agent-amd64.initrd" "$WORK/overlay-a.cpio.gz" > "$WORK/initrd-a"

mkdir -p "$WORK/http"
printf '#!/bin/sh\necho "<2>AUTORUN-RAN ok host=$(hostname)" > /dev/kmsg\nexit 0\n' > "$WORK/http/autorun-ok.sh"
printf '#!/bin/sh\necho "<2>AUTORUN-RAN fail host=$(hostname)" > /dev/kmsg\nexit 3\n' > "$WORK/http/autorun-fail.sh"
OKSHA=$(sha256sum "$WORK/http/autorun-ok.sh" | cut -d' ' -f1)
FAILSHA=$(sha256sum "$WORK/http/autorun-fail.sh" | cut -d' ' -f1)
AR="http://10.0.2.2:$HTTP_PORT"

"$HERE/make-overlay.sh" --out "$WORK/overlay-b.cpio.gz" --hostname smoke-b --pubkey "$KEY.pub" --timeout-min 1 \
    --net-static 10.0.2.50/24 10.0.2.2 10.0.2.3 --file "/usr/bin/synofurb-dom=$WORK/fake-dom" \
    --autorun-url "$AR/autorun-ok.sh" --autorun-sha256 "$(printf '1%.0s' $(seq 64))"
cat "$DIST/agent-amd64.initrd" "$WORK/overlay-b.cpio.gz" > "$WORK/initrd-b"

"$HERE/make-overlay.sh" --out "$WORK/overlay-c.cpio.gz" --hostname smoke-c --pubkey "$KEY.pub" --timeout-min 5 \
    --watchdog-mode claim --file "/usr/bin/synofurb-dom=$WORK/fake-dom" \
    --autorun-url "$AR/autorun-fail.sh" --autorun-sha256 "$FAILSHA"
cat "$DIST/agent-amd64.initrd" "$WORK/overlay-c.cpio.gz" > "$WORK/initrd-c"

"$HERE/make-overlay.sh" --out "$WORK/overlay-d.cpio.gz" --hostname smoke-d --pubkey "$KEY.pub" --timeout-min 1 \
    --watchdog-mode claim --file "/usr/bin/synofurb-dom=$WORK/fake-dom" \
    --autorun-url "$AR/autorun-ok.sh" --autorun-sha256 "$OKSHA"
cat "$DIST/agent-amd64.initrd" "$WORK/overlay-d.cpio.gz" > "$WORK/initrd-d"

# second kernel for the kexec test: the same kernel, plus an initrd that prints a marker and powers off
BBDEB=$(ls "$CACHE"/pool/*busybox-static_*_amd64.deb 2>/dev/null | head -n1)
[[ -n $BBDEB ]] || { echo "busybox-static .deb not found in $CACHE/pool (run build.sh first)" >&2; exit 2; }
mkdir -p "$WORK/marker/bin" "$WORK/marker/proc" "$WORK/marker/dev" "$WORK/http"
dpkg-deb --fsys-tarfile "$BBDEB" | tar -xO ./usr/bin/busybox > "$WORK/marker/bin/busybox"
chmod +x "$WORK/marker/bin/busybox"
cat > "$WORK/marker/init" <<'EOF'
#!/bin/busybox sh
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t devtmpfs devtmpfs /dev
exec >/dev/console 2>&1
echo "KEXEC-MARKER: second kernel is running, cmdline: $(/bin/busybox cat /proc/cmdline)"
/bin/busybox sleep 1
/bin/busybox poweroff -f
EOF
chmod +x "$WORK/marker/init"
( cd "$WORK/marker" && find . | cpio -o -H newc -R 0:0 --quiet | gzip -9n ) > "$WORK/http/marker.initrd"
cp "$DIST/agent-amd64.vmlinuz" "$WORK/http/vmlinuz"
KSHA=$(sha256sum "$WORK/http/vmlinuz" | cut -d' ' -f1)
ISHA=$(sha256sum "$WORK/http/marker.initrd" | cut -d' ' -f1)
( cd "$WORK/http" && exec python3 -I -m http.server "$HTTP_PORT" --bind 127.0.0.1 ) >"$WORK/http.log" 2>&1 &
HPID=$!
# HTTPS server with a throw-away self-signed certificate (busybox wget must fetch from it - and, as documented,
# will not complain about the unverifiable certificate; that is why --sha256 is mandatory)
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$WORK/tls.key" -out "$WORK/tls.crt" -subj /CN=smoke -days 2 >/dev/null 2>&1
cat > "$WORK/https_server.py" <<PYEOF
import http.server, ssl, sys, os
os.chdir(sys.argv[1])
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(sys.argv[2], sys.argv[3])
srv = http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[4])), http.server.SimpleHTTPRequestHandler)
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
srv.serve_forever()
PYEOF
python3 -I "$WORK/https_server.py" "$WORK/http" "$WORK/tls.crt" "$WORK/tls.key" "$((HTTP_PORT + 1))" >"$WORK/https.log" 2>&1 &
HSPID=$!

# ------------------------------------------------------------------ guest A
PORT=$PORT_A
g() { gssh "$PORT" "$@"; }
# out_has DESC REGEX REMOTE-COMMAND : run the command on the guest, pass if its output matches REGEX
out_has() { local d=$1 re=$2; shift 2; local o; o=$(g "$@" 2>&1); if grep -qE -- "$re" <<<"$o"; then ok "$d"; else bad "$d (got: $(head -c 200 <<<"$o" | tr '\n' '|'))"; fi; }
out_lacks() { local d=$1 re=$2; shift 2; local o; o=$(g "$@" 2>&1); if grep -qE -- "$re" <<<"$o"; then bad "$d (got: $(head -c 200 <<<"$o" | tr '\n' '|'))"; else ok "$d"; fi; }

if [[ -z $ONLY || $ONLY == *A* ]]; then
note "guest A: e1000, DHCP, key + one-time password (port $PORT_A)"
start_guest a e1000 10.0.2.15 "$PORT_A" "$WORK/initrd-a"
wait_for "$BOOT_TIMEOUT" "ssh up" gssh "$PORT_A" true
rc=$?
if [[ $rc -ne 0 ]]; then
    bad "SSH key login within ${BOOT_TIMEOUT}s (rc=$rc)"
    echo "---- serial log tail ----"; tail -n 60 "$SERIAL"
else
    ok "booted and reachable over SSH with the overlay key after $((SECONDS - T0))s (TCG)"
    check "kernel reached the agent init (banner on the serial console)" serial_has "boot agent starting"
    check "DHCP lease announced on the serial console (READY smoke-a ... 10.0.2.15)" serial_has "READY smoke-a.*10.0.2.15"
    out_has "hostname from the overlay" '^smoke-a$' hostname
    sleep 5   # the watchdog polls every 3 s; give it time to notice our first login
    g 'sf-boot status' > "$WORK/status-a.txt" 2>&1
    echo "  --- sf-boot status ---"; sed 's/^/  | /' "$WORK/status-a.txt"
    check "sf-boot status: claimed by ssh-login (our login disarmed the watchdog)" grep -q '^state=claimed' "$WORK/status-a.txt"
    check "sf-boot status: IPv4 lease on eth0" grep -q 'net.eth0.ipv4=10.0.2.15/24' "$WORK/status-a.txt"
    g 'sf-boot status --json' > "$WORK/status-a.json" 2>&1
    check "sf-boot status --json is valid JSON" python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["state"] == "claimed"' "$WORK/status-a.json"
    out_has "kexec --version runs" 'kexec-tools' 'kexec --version'
    echo "  | $(g 'kexec --version 2>&1')"
    out_has "default route via the DHCP router" 'default via 10.0.2.2' 'ip route'
    out_has "resolv.conf has the DHCP DNS server" 'nameserver 10.0.2.3' 'cat /etc/resolv.conf'
    out_has "NIC module e1000 was loaded by the modalias scan" '^e1000 ' lsmod
    out_has "modprobe loop works (needed by synofurb-dom loop mounts)" 'loop-control' 'modprobe loop; ls /dev/loop-control'
    out_has "real synofurb-dom is embedded and runs (sf-boot dom --version)" 'synofurb-dom' 'sf-boot dom --version'
    out_has "e2fsck and fsck.vfat are present" 'fsck.vfat' 'e2fsck -V 2>&1; ls /usr/bin/fsck.vfat'
    echo "  dom default state: $(g 'cat /run/synofurb/dom_default') (QEMU has no Synology DOM, so pending/failed is the expected outcome)"
    out_has "real synofurb-dom failed gracefully without blocking the boot" 'pending|failed' 'cat /run/synofurb/dom_default'
    out_has "dom failure reason is in agent.log" 'no Synology DOM found|synofurb-dom' 'grep -i dom /var/log/agent.log | tail -n 3'

    # --- beacon: listen on the guest itself (broadcasts are looped back to local sockets)
    g 'timeout 12 nc.traditional -l -u -p 9988' > "$WORK/beacon.txt" 2>&1
    echo "  beacon payload: $(head -n1 "$WORK/beacon.txt")"
    check "UDP beacon 'synofurb-agent smoke-a <mac> 10.0.2.15' seen on port 9988" grep -qE '^synofurb-agent smoke-a ([0-9a-f]{2}:){5}[0-9a-f]{2} 10\.0\.2\.15$' "$WORK/beacon.txt"

    # --- IPv6 (QEMU slirp sends router advertisements for fec0::/64)
    echo "  IPv6 on eth0: $(g "ip -6 -o addr show dev eth0 | awk '{print \$4}' | tr '\n' ' '")"
    echo "  dhcp6 log: $(g 'grep dhcp6 /var/log/agent.log | tail -n 3' | tr '\n' '|')"
    out_has "accept_ra=2 on eth0" '^2$' 'cat /proc/sys/net/ipv6/conf/eth0/accept_ra'
    out_has "SLAAC: a non-link-local IPv6 address was configured from the RA" 'inet6 fec0:' 'ip -6 addr show dev eth0'

    # --- one-time password
    ASK=$WORK/askpass.sh; printf '#!/bin/sh\necho smoke-pass-1\n' > "$ASK"; chmod +x "$ASK"
    pw_ssh() { SSH_ASKPASS="$ASK" SSH_ASKPASS_REQUIRE=force DISPLAY=none setsid ssh -o StrictHostKeyChecking=no \
                 -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o PreferredAuthentications=password -o PubkeyAuthentication=no \
                 -o NumberOfPasswordPrompts=1 -o ConnectTimeout=8 -p "$PORT_A" root@127.0.0.1 "$@" 2>/dev/null; }
    r1=$(pw_ssh 'echo pw-ok'); [[ $r1 == pw-ok ]] && ok "password login works the first time" || bad "password login (got '$r1')"
    sleep 6
    r2=$(pw_ssh 'echo pw-ok2'); [[ $r2 == pw-ok2 ]] && bad "password login still works after first use" || ok "password login revoked after its first use (one-time)"
    out_has "key login still works afterwards" 'k-ok' 'echo k-ok'

    g 'cat /var/log/agent.log' > "$WORK/agent-a.log" 2>&1
    g 'cat /var/log/dropbear.log' > "$WORK/dropbear-a.log" 2>&1

    # --- sf-boot kexec: hash enforcement, then a real kexec of a second kernel
    U="http://10.0.2.2:$HTTP_PORT"
    out_has "sf-boot kexec refuses http URLs without --sha256" 'refusing' "sf-boot kexec $U/vmlinuz $U/marker.initrd; echo rc=\$?"
    out_has "sf-boot kexec rejects a wrong kernel hash" 'mismatch' "sf-boot kexec $U/vmlinuz $U/marker.initrd --sha256 kernel=$(printf '0%.0s' $(seq 64)),initrd=$ISHA 2>&1"
    out_has "kexec_loaded is still 0 after the failed attempts" '^kexec_loaded=0' 'sf-boot status'
    out_has "https download works with busybox wget (self-signed cert accepted: no certificate verification!)" "^$ISHA" "wget -q -O /tmp/https.test https://10.0.2.2:$((HTTP_PORT + 1))/marker.initrd && sha256sum /tmp/https.test"
    g "sf-boot kexec $U/vmlinuz $U/marker.initrd --cmdline 'console=ttyS0,115200n8 loglevel=7 sf_marker=1' --sha256 kernel=$KSHA,initrd=$ISHA" > "$WORK/kexec-load.txt" 2>&1
    sed 's/^/  | /' "$WORK/kexec-load.txt"
    out_has "sf-boot kexec loaded the second kernel (kexec_loaded=1)" '^kexec_loaded=1' 'sf-boot status'
    g 'sf-boot exec --detach --delay 2' >/dev/null 2>&1
    end=$((SECONDS + 240))
    while (( SECONDS < end )) && ! serial_has "KEXEC-MARKER"; do sleep 3; done
    check "kexec jumped: second kernel printed its marker (cmdline passed through)" grep -q 'KEXEC-MARKER: second kernel is running, cmdline: .*sf_marker=1' "$SERIAL"
fi
echo "  --- last serial lines of guest A ---"; tail -n 25 "$SERIAL" | sed 's/^/  | /'
stop_guest
fi

# ------------------------------------------------------------------ guest B
if [[ -z $ONLY || $ONLY == *B* ]]; then
note "guest B: virtio-net, static IP, fake synofurb-dom, 1 minute watchdog, nobody logs in (port $PORT_B)"
start_guest b virtio-net-pci 10.0.2.50 "$PORT_B" "$WORK/initrd-b"
end=$((SECONDS + BOOT_TIMEOUT))
while (( SECONDS < end )) && ! serial_has "READY smoke-b"; do sleep 3; kill -0 "$QPID" 2>/dev/null || break; done
check "static address announced on the console (READY ... 10.0.2.50/24)" serial_has "READY smoke-b.*10.0.2.50/24"
check "SSH server answers over the virtio-net NIC (banner only - no login, so the watchdog stays armed)" bash -c "python3 -I -c \"
import socket,sys
s=socket.create_connection(('127.0.0.1',$PORT_B),timeout=10); s.settimeout(10)
b=s.recv(64); sys.exit(0 if b.startswith(b'SSH-2.0-dropbear') else 1)\""
check "FIRST ACTION: fake synofurb-dom called with 'default dsm' (--yes --skip-auto-backup)" grep -q 'FAKE-DOM called: --yes --skip-auto-backup default dsm' "$SERIAL"
dom_line=$(grep -n 'FAKE-DOM called' "$SERIAL" | head -n1 | cut -d: -f1)
ready_line=$(grep -n 'READY smoke-b' "$SERIAL" | head -n1 | cut -d: -f1)
drop_line=$(grep -n 'SSH: root@' "$SERIAL" | head -n1 | cut -d: -f1)
echo "  serial line numbers: fake-dom=${dom_line:-?} ssh-credentials=${drop_line:-?} READY=${ready_line:-?}"
check "dom call precedes network readiness" test "${dom_line:-999999}" -lt "${ready_line:-0}"
check "watchdog armed message" serial_has "watchdog armed: reboot to DSM in 1 min"
end=$((SECONDS + 60)); while (( SECONDS < end )) && ! serial_has "autorun: SHA-256 MISMATCH"; do sleep 3; done
check "autorun script with a wrong SHA-256 is refused (not run)" serial_has "autorun: SHA-256 MISMATCH"
check "refused autorun did not run the script" bash -c "! grep -q AUTORUN-RAN '$SERIAL'"
# wait for the watchdog to reboot the guest (QEMU -no-reboot -> QEMU exits)
end=$((SECONDS + 300))
while (( SECONDS < end )) && kill -0 "$QPID" 2>/dev/null; do sleep 3; done
if kill -0 "$QPID" 2>/dev/null; then bad "guest did not reboot within 300 s of waiting"; else ok "guest rebooted by itself (QEMU exited because of -no-reboot)"; fi
check "serial log: WATCHDOG EXPIRED message" serial_has "WATCHDOG EXPIRED"
check "watchdog re-pointed the DOM default at DSM before rebooting (second fake-dom call)" test "$(grep -c 'FAKE-DOM called' "$SERIAL")" -ge 2
t_armed=$(grep 'watchdog armed' "$SERIAL" | head -n1 | sed -n 's/^\[ *\([0-9.]*\)\].*/\1/p')
t_exp=$(grep 'WATCHDOG EXPIRED' "$SERIAL" | head -n1 | sed -n 's/^\[ *\([0-9.]*\)\].*/\1/p')
if [[ -n $t_armed && -n $t_exp ]]; then
    d=$(awk -v a="$t_armed" -v b="$t_exp" 'BEGIN { printf "%.0f", b - a }')
    echo "  watchdog armed at ${t_armed}s, expired at ${t_exp}s (guest clock) -> ${d}s"
    check "watchdog fired after about 1 minute (55..90 s)" test "$d" -ge 55 -a "$d" -le 90
else
    bad "could not read watchdog timestamps from the serial log"
fi
echo "  --- last serial lines of guest B ---"; tail -n 14 "$SERIAL" | sed 's/^/  | /'
stop_guest
fi

# ------------------------------------------------------------------ guest C
if [[ -z $ONLY || $ONLY == *C* ]]; then
note "guest C: watchdog-mode=claim, failing autorun, claim/arm (port $PORT_A)"
PORT=$PORT_A
start_guest c e1000 10.0.2.15 "$PORT_A" "$WORK/initrd-c"
wait_for "$BOOT_TIMEOUT" "ssh up" gssh "$PORT_A" true || { bad "SSH did not come up"; tail -n 30 "$SERIAL"; }
end=$((SECONDS + 90)); while (( SECONDS < end )) && ! serial_has "AUTORUN-RAN fail"; do sleep 3; done
check "autorun script was downloaded, verified (SHA-256) and run" serial_has "AUTORUN-RAN fail host=smoke-c"
end=$((SECONDS + 30)); while (( SECONDS < end )) && ! serial_has "re-arming the safety watchdog"; do sleep 3; done
check "failed autorun (exit 3) re-armed the watchdog" serial_has "script FAILED (exit 3) - re-arming"
sleep 4
out_has "SSH login did NOT disarm the watchdog in claim mode" '^state=armed' 'sf-boot status'
out_has "watchdog_remaining_s is reported (<= 300)" '^watchdog_remaining_s=([0-9]|[0-9]{2}|[12][0-9]{2}|300)$' 'sf-boot status'
out_has "sf-boot claim disarms" '^state=claimed' 'sf-boot claim >/dev/null; sf-boot status'
out_has "sf-boot arm 1 re-arms (<= 60 s remaining)" '^watchdog_remaining_s=([0-5]?[0-9]|60)$' 'sf-boot arm 1 >/dev/null; sf-boot status'
end=$((SECONDS + 150)); while (( SECONDS < end )) && kill -0 "$QPID" 2>/dev/null; do sleep 3; done
if kill -0 "$QPID" 2>/dev/null; then bad "guest did not reboot after sf-boot arm 1"; else ok "re-armed watchdog rebooted the guest after ~1 minute"; fi
check "WATCHDOG EXPIRED after the re-arm" serial_has "WATCHDOG EXPIRED"
stop_guest
fi

# ------------------------------------------------------------------ guest D
if [[ -z $ONLY || $ONLY == *D* ]]; then
note "guest D: good autorun claims the unit; sf-boot reboot debian (port $PORT_A)"
PORT=$PORT_A
start_guest d e1000 10.0.2.15 "$PORT_A" "$WORK/initrd-d"
wait_for "$BOOT_TIMEOUT" "ssh up" gssh "$PORT_A" true || { bad "SSH did not come up"; tail -n 30 "$SERIAL"; }
end=$((SECONDS + 90)); while (( SECONDS < end )) && ! serial_has "AUTORUN-RAN ok"; do sleep 3; done
check "autorun script ran" serial_has "AUTORUN-RAN ok host=smoke-d"
check "autorun claimed the unit" serial_has "unit CLAIMED (autorun)"
sleep 75   # well past the 1 minute timeout
if kill -0 "$QPID" 2>/dev/null; then ok "claimed unit did NOT reboot after the 1 minute timeout"; else bad "guest rebooted although claimed"; fi
out_has "claim reason is autorun" '^claim_reason=autorun' 'sf-boot status'
g 'sf-boot reboot debian' >"$WORK/reboot-d.txt" 2>&1
end=$((SECONDS + 90)); while (( SECONDS < end )) && kill -0 "$QPID" 2>/dev/null; do sleep 2; done
if kill -0 "$QPID" 2>/dev/null; then bad "sf-boot reboot debian did not reboot"; else ok "sf-boot reboot debian rebooted the guest (QEMU exit)"; fi
check "synofurb-dom was called with 'default debian' first" serial_has "FAKE-DOM called: --yes --skip-auto-backup default debian"
stop_guest
fi

# ------------------------------------------------------------------ guest E
if [[ -z $ONLY || $ONLY == *E* ]]; then
note "guest E: console=tty0 + console=uart8250,io,0x3f8 (port $PORT_A)"
PORT=$PORT_A
APPEND="console=tty0 console=uart8250,io,0x3f8,115200n8 loglevel=7 panic=-1"
start_guest e e1000 10.0.2.15 "$PORT_A" "$WORK/initrd-a"
unset APPEND
wait_for "$BOOT_TIMEOUT" "ssh up" gssh "$PORT_A" true || { bad "SSH did not come up"; tail -n 30 "$SERIAL"; }
out_has "kernel listed both consoles as active" 'tty0 ttyS0' 'cat /sys/class/tty/console/active'
out_has "inittab: shell on tty1 (VGA console)" '^tty1::respawn:' 'cat /etc/inittab'
out_has "inittab: shell on ttyS0 (8250 port named by I/O address)" '^ttyS0::respawn:' 'cat /etc/inittab'
out_has "two console shells are running" '^[2-9]' 'ps w | grep -c "[c]onsole-shell"'
check "banner reached the 8250 serial console (READY line on serial)" serial_has "READY smoke-a"
g 'cat /etc/inittab' | sed 's/^/  | /'
stop_guest
fi

echo
echo "RESULT: $PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
