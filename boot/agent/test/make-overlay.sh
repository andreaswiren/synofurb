#!/usr/bin/env bash
# make-overlay.sh - build the per-unit OVERLAY for the synofurb boot agent.
#
# The overlay is a small gzip-compressed newc cpio archive. Append it to the agent initrd
# (cat agent-amd64.initrd overlay.cpio.gz > initrd-for-this-nas) - the kernel unpacks concatenated
# archives in order, so the overlay's files replace/extend the ones in the base initrd.
#
#   make-overlay.sh --out overlay.cpio.gz [options]
#     --hostname NAME            AGENT_HOSTNAME
#     --timeout-min N            AGENT_TIMEOUT_MIN (0 = never reboot by itself; default 20)
#     --watchdog-mode login|claim
#     --pubkey FILE              add a public key (repeatable) -> /etc/synofurb-authorized_keys
#     --password-hash HASH       AGENT_ONE_TIME_PASSWORD_HASH (crypt(3) hash; one use)
#     --password-reusable        keep password login after first use
#     --autorun-url URL --autorun-sha256 HEX
#     --net-static ADDR/PREFIX GATEWAY [DNS...]   AGENT_NET_MODE=static (see --net-iface)
#     --net-iface auto|ethN|MAC
#     --ssh-port N | --beacon-port N
#     --set VAR=VALUE            any other AGENT_* setting (repeatable)
#     --file DEST=SRC            add a file to the overlay (DEST absolute path in the agent; mode kept)
#     --host-key FILE            dropbear ed25519 host key to pin (-> /etc/dropbear/dropbear_ed25519_host_key)
set -euo pipefail

die() { echo "make-overlay.sh: $*" >&2; exit 1; }
q() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }   # shell-quote a value

OUT=""; declare -a KEYS=() FILES=() SETS=()
HOSTNAME_=""; TIMEOUT=""; WDMODE=""; PWHASH=""; PWREUSE=0; AURL=""; ASHA=""
NETIP=""; NETGW=""; NETDNS=""; NETIF=""; SSHPORT=""; BEACON=""; HOSTKEY=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --out) OUT=${2:?}; shift 2 ;;
        --hostname) HOSTNAME_=${2:?}; shift 2 ;;
        --timeout-min) TIMEOUT=${2:?}; shift 2 ;;
        --watchdog-mode) WDMODE=${2:?}; shift 2 ;;
        --pubkey) KEYS+=("${2:?}"); shift 2 ;;
        --password-hash) PWHASH=${2:?}; shift 2 ;;
        --password-reusable) PWREUSE=1; shift ;;
        --autorun-url) AURL=${2:?}; shift 2 ;;
        --autorun-sha256) ASHA=${2:?}; shift 2 ;;
        --net-static) NETIP=${2:?}; NETGW=${3:?}; shift 3; NETDNS=""
                      while [[ $# -gt 0 && $1 != --* ]]; do NETDNS="$NETDNS $1"; shift; done ;;
        --net-iface) NETIF=${2:?}; shift 2 ;;
        --ssh-port) SSHPORT=${2:?}; shift 2 ;;
        --beacon-port) BEACON=${2:?}; shift 2 ;;
        --set) SETS+=("${2:?}"); shift 2 ;;
        --file) FILES+=("${2:?}"); shift 2 ;;
        --host-key) HOSTKEY=${2:?}; shift 2 ;;
        -h|--help) sed -n '2,40p' "$0"; exit 0 ;;
        *) die "unknown option $1" ;;
    esac
done
[[ -n $OUT ]] || die "--out is required"
case ${WDMODE:-login} in login|claim) ;; *) die "--watchdog-mode must be login or claim" ;; esac
[[ -z $TIMEOUT || $TIMEOUT =~ ^[0-9]+$ ]] || die "--timeout-min must be a number"
[[ -z $HOSTNAME_ || $HOSTNAME_ =~ ^[A-Za-z0-9][A-Za-z0-9-]*$ ]] || die "--hostname: letters, digits and '-' only"
[[ -z $AURL || -n $ASHA ]] || echo "make-overlay.sh: warning: --autorun-url without --autorun-sha256: the agent will refuse to run it" >&2

STAGE=$(mktemp -d); trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/etc"
CONF=$STAGE/etc/synofurb-agent.conf
{
    echo "# synofurb boot agent overlay - generated $(date -u +%FT%TZ)"
    [[ -n $HOSTNAME_ ]] && echo "AGENT_HOSTNAME=$(q "$HOSTNAME_")"
    [[ -n $TIMEOUT ]] && echo "AGENT_TIMEOUT_MIN=$TIMEOUT"
    [[ -n $WDMODE ]] && echo "AGENT_WATCHDOG_MODE=$WDMODE"
    [[ -n $PWHASH ]] && echo "AGENT_ONE_TIME_PASSWORD_HASH=$(q "$PWHASH")"
    [[ $PWREUSE == 1 ]] && echo "AGENT_PASSWORD_REUSABLE=1"
    [[ -n $AURL ]] && echo "AGENT_AUTORUN_URL=$(q "$AURL")"
    [[ -n $ASHA ]] && echo "AGENT_AUTORUN_SHA256=$(q "$ASHA")"
    if [[ -n $NETIP ]]; then
        echo "AGENT_NET_MODE=static"
        echo "AGENT_STATIC_ADDRESS=$(q "$NETIP")"
        echo "AGENT_STATIC_GATEWAY=$(q "$NETGW")"
        echo "AGENT_STATIC_DNS=$(q "${NETDNS# }")"
    fi
    [[ -n $NETIF ]] && echo "AGENT_NET_IFACE=$(q "$NETIF")"
    [[ -n $SSHPORT ]] && echo "AGENT_SSH_PORT=$SSHPORT"
    [[ -n $BEACON ]] && echo "AGENT_BEACON_PORT=$BEACON"
    for s in "${SETS[@]:-}"; do
        [[ -n $s ]] || continue
        [[ $s =~ ^AGENT_[A-Z0-9_]+=.*$ ]] || die "--set expects AGENT_NAME=value, got '$s'"
        echo "${s%%=*}=$(q "${s#*=}")"
    done
} > "$CONF"
chmod 600 "$CONF"

if [[ ${#KEYS[@]} -gt 0 ]]; then
    : > "$STAGE/etc/synofurb-authorized_keys"
    for k in "${KEYS[@]}"; do
        [[ -f $k ]] || die "no such public key file: $k"
        cat "$k" >> "$STAGE/etc/synofurb-authorized_keys"
        [[ $(tail -c1 "$k" | wc -l) -eq 1 ]] || echo >> "$STAGE/etc/synofurb-authorized_keys"
    done
    chmod 644 "$STAGE/etc/synofurb-authorized_keys"
fi
if [[ -n $HOSTKEY ]]; then
    mkdir -p "$STAGE/etc/dropbear"
    install -m 600 "$HOSTKEY" "$STAGE/etc/dropbear/dropbear_ed25519_host_key"
fi
for f in "${FILES[@]:-}"; do
    [[ -n $f ]] || continue
    dest=${f%%=*}; src=${f#*=}
    [[ $dest == /* ]] || die "--file DEST must be an absolute path: $dest"
    [[ -f $src ]] || die "--file: no such file $src"
    install -D -m "$(stat -c %a "$src")" "$src" "$STAGE$dest"
done

find "$STAGE" -exec touch -h -d @0 {} +
( cd "$STAGE" && find . -mindepth 1 -print0 | LC_ALL=C sort -z | cpio --null -o -H newc -R 0:0 --quiet ) | gzip -9n > "$OUT"
echo "overlay written: $OUT ($(stat -c %s "$OUT") bytes)" >&2
