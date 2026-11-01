#!/bin/sh
# Offline-mode integration test: builds a REAL bundle from deb.debian.org, serves it with synofurb's own server code
# and runs the real debootstrap / apt-get against it (see docs/OFFLINE.md, "How completeness is proven").
#
#   tests/offline/run.sh [WORKDIR]
#
#   SYNOFURB_OFFLINE_ARCHS   comma list, default amd64            (e.g. amd64,arm64,armhf)
#   SYNOFURB_OFFLINE_KERNEL  newest (default) | stable
#   SYNOFURB_OFFLINE_FIRMWARE=1   also fetch the ~470 MiB installer firmware cpio (default: skipped to save disk)
#   KEEP=1                   keep the bundle and the chroots afterwards
#
# Needs: Internet, root (chroot + mount) for the host architecture, `apt-get install debootstrap gpgv curl`.
# Disk: about 0.5 GiB per architecture for the bundle, plus about 2 GiB for the chroot of the host architecture.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${1:-${TMPDIR:-/tmp}/synofurb-offline-test}
export SYNOFURB_OFFLINE_DIR="$WORK/bundle" SYNOFURB_OFFLINE_WORK="$WORK/work"
export SYNOFURB_OFFLINE_ARCHS="${SYNOFURB_OFFLINE_ARCHS:-amd64}"
mkdir -p "$WORK/work"

# the architecture the host can really run gets the full chroot install, the others the apt simulation
case "$(uname -m)" in
  x86_64) HOSTARCH=amd64 ;; aarch64) HOSTARCH=arm64 ;; armv7l) HOSTARCH=armhf ;; *) HOSTARCH=none ;;
esac
MODES=""
for a in $(echo "$SYNOFURB_OFFLINE_ARCHS" | tr ',' ' '); do
  if [ "$a" = "$HOSTARCH" ] && [ "$(id -u)" = 0 ]; then MODES="$MODES,$a=chroot"; else MODES="$MODES,$a=foreign"; fi
done
export SYNOFURB_OFFLINE_MODES="${MODES#,}"

cleanup() { [ "${KEEP:-0}" = 1 ] || { [ -z "$(awk -v p="$WORK/" 'index($2,p)==1' /proc/mounts)" ] && rm -rf "$WORK"; }; }
trap cleanup EXIT

cd "$ROOT"
cargo test -p synofurb-core --lib live_create_bundle -- --ignored --nocapture
cargo test -p synofurb-core --lib live_udeb_closure_is_served -- --ignored --nocapture
cargo test -p synofurb-core --lib live_real_tools -- --ignored --nocapture
echo "offline integration test: PASS ($SYNOFURB_OFFLINE_ARCHS)"
