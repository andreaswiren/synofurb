#!/usr/bin/env bash
# Portable archive with the bare binary:  synofurb-<version>-<os>-<arch>.tar.gz (Linux, macOS) or .zip (Windows).
#
#   scripts/package-portable.sh --target x86_64-unknown-linux-gnu [--out dist/release]
#
# Layout (flat, no top-level directory):   synofurb[.exe]  LICENSE  README.txt  [synofurb.png on Linux]
# Reproducible: fixed timestamps (SOURCE_DATE_EPOCH, default 0), owner root, mode 0755 for the binary.
set -euo pipefail
# shellcheck source=./package-lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"

TARGET=""
OUT="$PKG_ROOT/dist/release"
while [ $# -gt 0 ]; do
    case "$1" in
        --target) TARGET="${2:?--target needs a value}"; shift 2 ;;
        --out) OUT="${2:?--out needs a value}"; shift 2 ;;
        -h | --help) sed -n '2,8p' "$0"; exit 0 ;;
        *) pkg_die "unknown option: $1" ;;
    esac
done
[ -n "$TARGET" ] || pkg_die "--target is required"

VERSION="$(pkg_version)"
OS="$(pkg_triple_os "$TARGET")"
ARCH="$(pkg_triple_arch "$TARGET")"
BIN_DIR="$PKG_ROOT/target/$TARGET/release"
if [ "$OS" = windows ]; then BIN="$BIN_DIR/synofurb.exe"; EXT=zip; else BIN="$BIN_DIR/synofurb"; EXT=tar.gz; fi
[ -f "$BIN" ] || pkg_die "$BIN not found: build first with  cargo build --release --locked -p synofurb --target $TARGET"

mkdir -p "$OUT"
DEST="$OUT/$(pkg_asset_name "$VERSION" "$OS" "$ARCH" "$EXT")"
ICON=""
if [ "$OS" = linux ]; then ICON="$PKG_ROOT/packaging/icons/512x512.png"; fi

PY="$(pkg_python)"
"$PY" - "$EXT" "$DEST" "$BIN" "$PKG_ROOT/LICENSE" "$VERSION" "$OS" "$ARCH" "$ICON" <<'PYEOF'
import gzip, io, os, sys, tarfile, zipfile

ext, dest, binary, license_file, version, os_name, arch, icon = sys.argv[1:9]
epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
readme = f"""synofurb {version} ({os_name} {arch}) - portable build

Run `synofurb` (no arguments opens the wizard; `synofurb --help` lists the commands).
Reinstalling a NAS repartitions its disks: make a verified backup first.
Project: https://github.com/andreaswiren/synofurb  -  MIT License
""".encode()
binname = "synofurb.exe" if os_name == "windows" else "synofurb"
members = [(binname, open(binary, "rb").read(), 0o755), ("LICENSE", open(license_file, "rb").read(), 0o644), ("README.txt", readme, 0o644)]
if icon:
    members.append(("synofurb.png", open(icon, "rb").read(), 0o644))

if ext == "zip":
    ts = max(epoch, 315532800)  # zip cannot store dates before 1980
    t = __import__("time").gmtime(ts)[:6]
    with zipfile.ZipFile(dest, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, data, mode in members:
            zi = zipfile.ZipInfo(name, date_time=t)
            zi.compress_type = zipfile.ZIP_DEFLATED
            zi.external_attr = (0o100000 | mode) << 16
            zf.writestr(zi, data)
else:
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.PAX_FORMAT) as tf:
        for name, data, mode in members:
            ti = tarfile.TarInfo(name)
            ti.size, ti.mode, ti.mtime = len(data), mode, epoch
            ti.uid = ti.gid = 0
            ti.uname = ti.gname = "root"
            tf.addfile(ti, io.BytesIO(data))
    with open(dest, "wb") as fh:
        with gzip.GzipFile(filename="", mode="wb", fileobj=fh, mtime=epoch, compresslevel=9) as gz:
            gz.write(raw.getvalue())
print(dest)
PYEOF
pkg_log "wrote $(basename "$DEST") ($(pkg_sha256 "$DEST"))"
