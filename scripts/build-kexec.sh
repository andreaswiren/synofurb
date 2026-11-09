#!/bin/sh
# Build a statically linked kexec for a NAS architecture.
#
#   scripts/build-kexec.sh x86_64|aarch64|armv7   [output-dir]
#
# Why: DSM does not ship kexec, and synofurb uploads one to start the Debian installer. It must be
# static (DSM's libc is old and different), and built for the NAS CPU, not for this computer.
#
# Method:
#   x86_64 on an x86_64 Linux computer with gcc (a GitHub runner, a developer machine): a NATIVE static build with gcc and glibc. This is the
#   toolchain kexec-tools is written for. It targets Linux 3.2.0 and up, so it runs on DSM's 3.10 kernel, and needs nothing installed on the NAS.
#   everything else: musl cross compiler via `zig cc` (https://ziglang.org).
#   Do NOT use zig for x86_64: the "purgatory" stub it produces contains an R_X86_64_GOTPC64 relocation that kexec cannot process, and
#   `kexec -l` then fails on the NAS with "Unhandled rela relocation: R_X86_64_29" (seen on a real RS812+ with the 0.2.4 and 0.2.5 releases).
#   scripts/test-kexec.sh reproduces that failure without a NAS and proves a build is free of it.
# The source tarball is pinned by SHA-256 (taken from kernel.org's signed sha256sums.asc).
#
# STATUS: the x86_64 build was run (native gcc 13) and passes scripts/test-kexec.sh. aarch64 and armv7 (zig) do NOT build yet.
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
NATIVE=0
if [ "$ARCH" = x86_64 ] && [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] && command -v gcc >/dev/null 2>&1; then NATIVE=1; fi
if [ "$NATIVE" = 0 ]; then
  command -v zig >/dev/null 2>&1 || { echo "zig is required to cross-build $ARCH (https://ziglang.org/download/)" >&2; exit 1; }
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"
echo "downloading kexec-tools $VERSION"
curl -fsSL -o kt.tar.xz "$URL"
echo "$SHA256  kt.tar.xz" | sha256sum -c -
tar xf kt.tar.xz
cd "kexec-tools-$VERSION"

if [ "$NATIVE" = 1 ]; then
  echo "native static build with $(gcc --version | head -n 1)"
  ./configure "LDFLAGS=-static -s" CFLAGS=-Os --without-lzma --without-zlib --without-xen
else
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
fi
make -j"$(nproc)"

DEST="$OUT/$DEB"
mkdir -p "$DEST"
cp build/sbin/kexec "$DEST/kexec"
chmod 755 "$DEST/kexec"
echo "built $DEST/kexec"
file "$DEST/kexec" || true
