# shellcheck shell=sh disable=SC2034
# Common helpers for the synofurb boot agent (sourced by the other scripts, busybox ash).

export PATH=/usr/bin:/usr/sbin:/bin:/sbin
SF_RUN=/run/synofurb
SF_LOG=/var/log/agent.log
SF_CONF=/etc/synofurb-agent.conf
SF_DROPBEAR_LOG=/var/log/dropbear.log
mkdir -p "$SF_RUN" /var/log 2>/dev/null

uptime_s() { read -r _u _ < /proc/uptime; echo "${_u%%.*}"; }

# log LEVEL TEXT...    -> /var/log/agent.log only
log() {
    _lvl=$1; shift
    printf '[%6s] %-5s %s\n' "$(uptime_s)" "$_lvl" "$*" >> "$SF_LOG" 2>/dev/null
}

# note TEXT...         -> agent.log AND the kernel log at KERN_CRIT, which the kernel prints on
#                         every console it registered (VGA, ttyS*, uart8250 mmio ports...).
note() {
    log note "$*"
    printf '<2>synofurb-agent: %s\n' "$*" > /dev/kmsg 2>/dev/null \
        || printf 'synofurb-agent: %s\n' "$*" > /dev/console 2>/dev/null
}

# Defaults first, then the overlay's /etc/synofurb-agent.conf (shell syntax) on top.
load_conf() {
    AGENT_HOSTNAME=
    AGENT_TIMEOUT_MIN=20
    AGENT_WATCHDOG_MODE=login            # login: any successful SSH login disarms; claim: only `sf-boot claim`
    AGENT_SSH_AUTHORIZED_KEYS=
    AGENT_ONE_TIME_PASSWORD_HASH=
    AGENT_PASSWORD_REUSABLE=0            # 1: keep password login after its first successful use
    AGENT_AUTORUN_URL=
    AGENT_AUTORUN_SHA256=
    AGENT_AUTORUN_ALLOW_UNVERIFIED=0
    AGENT_AUTORUN_WAIT_S=180
    AGENT_NET_MODE=dhcp
    AGENT_NET_IFACE=auto
    AGENT_STATIC_ADDRESS=
    AGENT_STATIC_GATEWAY=
    AGENT_STATIC_DNS=
    AGENT_STATIC_ADDRESS6=
    AGENT_STATIC_GATEWAY6=
    AGENT_BEACON_PORT=9988
    AGENT_SSH_PORT=22
    AGENT_CONSOLE_SHELL=1
    AGENT_MODULES=
    if [ -f "$SF_CONF" ]; then
        if sh -n "$SF_CONF" 2>/dev/null; then
            # shellcheck disable=SC1090
            . "$SF_CONF"
        else
            log error "$SF_CONF has a syntax error - using defaults"
        fi
    fi
    case $AGENT_TIMEOUT_MIN in ''|*[!0-9]*) log error "bad AGENT_TIMEOUT_MIN='$AGENT_TIMEOUT_MIN', using 20"; AGENT_TIMEOUT_MIN=20;; esac
}

# Ethernet ports: eth*/en* of ARPHRD_ETHER, not wireless.
eth_ifaces() {
    for _p in /sys/class/net/*; do
        _n=${_p##*/}
        case $_n in eth*|en*) ;; *) continue;; esac
        [ -d "$_p/wireless" ] && continue
        read -r _t < "$_p/type" 2>/dev/null || continue
        [ "$_t" = 1 ] || continue
        echo "$_n"
    done
}

mac_of() { cat "/sys/class/net/$1/address" 2>/dev/null; }

# first global IPv4 of an interface (or of any, when none is given)
ip4_of() {
    ip -4 -o addr show ${1:+dev "$1"} scope global 2>/dev/null | awk '{ sub(/\/.*/, "", $4); print $4; exit }'
}

rebuild_resolv() {
    mkdir -p "$SF_RUN/resolv"
    cat "$SF_RUN"/resolv/* 2>/dev/null | awk '!seen[$0]++' > /etc/resolv.conf.new \
        && mv /etc/resolv.conf.new /etc/resolv.conf
}

# watchdog / claim state ------------------------------------------------------------------
wd_set_deadline() { echo $(( $(uptime_s) + $1 )) > "$SF_RUN/wd_deadline"; }
wd_login_count() { grep -c 'auth succeeded' "$SF_DROPBEAR_LOG" 2>/dev/null || true; }

# Set the boot-menu default on the DOM (DSM / debian / agent). The script comes from the DOM
# component; exit status 64 means "not implemented" (the build-time stub) and is never retried.
# --yes: no terminal prompt; --skip-auto-backup: the one-time full-DOM backup belongs to the installer
# running under DSM - in this RAM-only system it would only fill memory and vanish on reboot.
dom_set_default() {
    [ -x /usr/bin/synofurb-dom ] || { log error "synofurb-dom missing"; return 64; }
    timeout 120 synofurb-dom --yes --skip-auto-backup default "$1" >> "$SF_LOG" 2>&1 </dev/null
}

# Reboot for real: busybox reboot signals init; if the platform does not reset within 20 s, use sysrq.
do_reboot() {
    sync
    reboot &
    sleep 20
    note "reboot did not complete - forcing it through sysrq"
    echo b > /proc/sysrq-trigger
}

claim_unit() { # claim_unit REASON
    [ -e "$SF_RUN/claimed" ] && return 0
    echo "$1 $(uptime_s)" > "$SF_RUN/claimed"
    note "unit CLAIMED ($1): safety watchdog disarmed"
}
