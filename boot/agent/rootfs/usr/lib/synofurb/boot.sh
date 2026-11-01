#!/bin/sh
# synofurb boot agent - main boot sequence. Started by busybox init as ::sysinit: (init waits for it),
# so it must return quickly; long-lived work is done by the svc-* scripts that init respawns afterwards.
. /usr/lib/synofurb/lib.sh
load_conf

note "boot agent starting: $(head -n1 /etc/synofurb-agent-release 2>/dev/null) kernel $(uname -r)"
log info "cmdline: $(cat /proc/cmdline)"
echo pending > "$SF_RUN/dom_default"

# --- 1. FAIL-SAFE: the first thing the agent does is point the DOM's boot default back at DSM.
# The DOM block device only shows up once the storage drivers load, so this keeps retrying in the
# background (every 3 s, up to 2 minutes) while the rest of the boot proceeds.
(
    n=0
    while [ "$n" -lt 40 ]; do
        dom_set_default dsm
        rc=$?
        if [ "$rc" -eq 0 ]; then
            echo ok > "$SF_RUN/dom_default"
            note "boot default set to DSM (synofurb-dom default dsm, attempt $((n + 1)))"
            exit 0
        fi
        if [ "$rc" -eq 64 ]; then
            echo missing > "$SF_RUN/dom_default"
            note "WARNING: synofurb-dom is a STUB in this build - the boot default was NOT changed"
            exit 1
        fi
        n=$((n + 1))
        log info "synofurb-dom default dsm failed (rc=$rc), attempt $n"
        sleep 3
    done
    echo failed > "$SF_RUN/dom_default"
    note "WARNING: could not set the DOM boot default to DSM after $n attempts; see sf-boot log"
) &

# --- 2. drivers: load everything the hardware's modalias strings ask for (busybox modprobe reads
# modules.alias), twice, because PHY/mdio devices only appear after their NIC driver bound.
load_modules() {
    find /sys/devices -name modalias -type f -exec cat {} + 2>/dev/null | sort -u \
        | xargs modprobe -qba 2>>"$SF_LOG"
}
for m in af_packet ipv6 $AGENT_MODULES; do modprobe -q "$m" 2>/dev/null; done
load_modules
sleep 1
load_modules
log info "modules: $(lsmod | awk 'NR>1{printf "%s ", $1}')"

# --- 3. identity
first=$(eth_ifaces | head -n1)
if [ -z "$AGENT_HOSTNAME" ]; then
    suffix=$(mac_of "$first" | tr -d ':' | cut -c7-12)
    AGENT_HOSTNAME="synofurb-${suffix:-agent}"
fi
case $AGENT_HOSTNAME in
    *[!A-Za-z0-9-]*|-*|'') AGENT_HOSTNAME="synofurb-agent" ;;
esac
hostname "$AGENT_HOSTNAME"
echo "$AGENT_HOSTNAME" > /etc/hostname
printf '127.0.0.1 localhost %s\n::1 localhost\n' "$AGENT_HOSTNAME" > /etc/hosts

# --- 4. SSH credentials
umask 077
mkdir -p /root/.ssh /etc/dropbear
chmod 700 /root /root/.ssh
{
    printf '%s\n' "$AGENT_SSH_AUTHORIZED_KEYS"
    [ -f /etc/synofurb-authorized_keys ] && cat /etc/synofurb-authorized_keys
} | awk 'NF && $1 !~ /^#/' > /root/.ssh/authorized_keys
chmod 600 /root/.ssh/authorized_keys
nkeys=$(wc -l < /root/.ssh/authorized_keys)

if [ -n "$AGENT_ONE_TIME_PASSWORD_HASH" ]; then
    printf 'root:%s:19000:0:99999:7:::\n' "$AGENT_ONE_TIME_PASSWORD_HASH" > /etc/shadow
    touch "$SF_RUN/password_enabled"
    [ "$AGENT_PASSWORD_REUSABLE" = 1 ] || touch "$SF_RUN/password_onetime"
else
    printf 'root:*:19000:0:99999:7:::\n' > /etc/shadow
fi
chmod 600 /etc/shadow
umask 022

for t in ed25519 ecdsa; do
    k=/etc/dropbear/dropbear_${t}_host_key
    if [ ! -s "$k" ]; then
        if [ "$t" = ecdsa ]; then dropbearkey -t ecdsa -s 256 -f "$k" >/dev/null 2>>"$SF_LOG"
        else dropbearkey -t "$t" -f "$k" >/dev/null 2>>"$SF_LOG"; fi
    fi
done
fp=$(dropbearkey -y -f /etc/dropbear/dropbear_ed25519_host_key 2>/dev/null | awk '/^Fingerprint/{print $2}')
note "SSH: root@<ip> port $AGENT_SSH_PORT, $nkeys authorized key(s), password login $( [ -e "$SF_RUN/password_enabled" ] && echo enabled || echo off ); host key ed25519 $fp"
if [ "$nkeys" -eq 0 ] && [ ! -e "$SF_RUN/password_enabled" ]; then
    note "WARNING: no SSH key and no password configured - nobody can log in over the network (overlay missing?)"
fi

# --- 5. kernel knobs
for f in /proc/sys/net/ipv6/conf/all/accept_ra /proc/sys/net/ipv6/conf/default/accept_ra; do echo 2 > "$f" 2>/dev/null; done
echo 1 > /proc/sys/net/ipv4/conf/all/arp_announce 2>/dev/null
mkdir -p "$SF_RUN/net" "$SF_RUN/resolv"
: > /etc/resolv.conf

note "init done: network comes up in the background (svc-net), watchdog $( [ "$AGENT_TIMEOUT_MIN" -eq 0 ] && echo DISABLED || echo "${AGENT_TIMEOUT_MIN} min" )"
exit 0
