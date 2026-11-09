#!/bin/sh
# Build a statically linked kexec for a NAS architecture.
#
#   scripts/build-kexec.sh x86_64|aarch64|armv7   [output-dir]
#
# Why: DSM does not ship kexec, and synofurb uploads one to start the Debian installer. It must be
# static (DSM's libc is old and different), and built for the NAS CPU, not for this computer.
#
# Method: musl cross compiler via `zig cc` (https://ziglang.org), so one machine can build all three.
# The source tarball is pinned by SHA-256 (taken from kernel.org's signed sha256sums.asc).
#
# STATUS: this script has NOT been run by the author of this repository (the build environment used
# to write it blocks compiling downloaded sources). Build it yourself and inspect the result:
#   file assets/kexec/<arch>/kexec      -> must say "statically linked"
#   assets/kexec/<arch>/kexec --version
set -eu
ARCH=${1:?usage: build-kexec.sh x86_64|aarch64|armv7 [outdir]}
OUT=${2:-assets/kexec}
ORIG=$(pwd)
case "$OUT" in /*) ;; *) OUT="$ORIG/$OUT" ;; esac
VERSION=2.0.32
SHA256=8f81422a5fd2362cf6cb001b511e535565ed0f32c2f4451fb5eb68fed6710a5d
URL=https://mirrors.edge.kernel.org/pub/linux/utils/kernel/kexec/kexec-tools-$VERSION.tar.xz

case "$ARCH" in
  x86_64)  TARGET=x86_64-linux-musl;     HOST=x86_64-linux-musl;   DEB=amd64 ;;
  aarch64) TARGET=aarch64-linux-musl;    HOST=aarch64-linux-musl;  DEB=arm64 ;;
  armv7)   TARGET=arm-linux-musleabihf;  HOST=arm-linux-musleabihf; DEB=armhf ;;
  *) echo "unknown architecture $ARCH" >&2; exit 2 ;;
esac
command -v zig >/dev/null 2>&1 || { echo "zig is required (https://ziglang.org/download/)" >&2; exit 1; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"
echo "downloading kexec-tools $VERSION"
curl -fsSL -o kt.tar.xz "$URL"
echo "$SHA256  kt.tar.xz" | sha256sum -c -
tar xf kt.tar.xz
cd "kexec-tools-$VERSION"

# zig cc reads `-march=x86-64` (which kexec-tools passes when it builds its purgatory stub) as the CPU "x86" and stops with
# "unknown CPU: 'x86'"; zig spells that baseline CPU x86_64, so the wrapper rewrites the flag. It also drops the purgatory's
# `-Wl,-Map=` link-map option, which zig's linker front end does not support.
cat > "$WORK/cc" <<CC
#!/bin/sh
for a in "\$@"; do
  shift
  case "\$a" in
    -march=x86-64) a=-mcpu=x86_64 ;;
    -Wl,-Map=*) continue ;;   # a link map for debugging; zig's linker front end rejects it
  esac
  set -- "\$@" "\$a"
done
exec zig cc -target $TARGET "\$@"
CC
chmod +x "$WORK/cc"
./configure --host="$HOST" CC="$WORK/cc" CFLAGS="-Os" LDFLAGS="-static" --without-lzma --without-zlib --without-xen
make -j"$(nproc)"

DEST="$OUT/$DEB"
mkdir -p "$DEST"
cp build/sbin/kexec "$DEST/kexec"
chmod 755 "$DEST/kexec"
echo "built $DEST/kexec"
file "$DEST/kexec" || true
