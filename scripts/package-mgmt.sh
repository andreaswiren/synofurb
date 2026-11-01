#!/usr/bin/env bash
# Management plane tarball:  synofurb-mgmt-<mgmt-version>.tar.gz  (version = apps/mgmt-plane/VERSION, NOT the app's)
#
#   scripts/package-mgmt.sh [--out dist/release]
#
# Flat archive (extract straight into /opt/synofurb-mgmt):  synofurb_mgmt.py  synofurb-mgmt.service  README.md  VERSION
# Refuses to build when VERSION disagrees with the built-in `DEFAULT_VERSION = "..."` (or literal `VERSION = "..."`) in
# synofurb_mgmt.py, so a release cannot report a different version than the one it ships under.
# Reproducible: fixed mtimes (SOURCE_DATE_EPOCH, default 0), root ownership.
set -euo pipefail
# shellcheck source=./package-lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"

OUT="$PKG_ROOT/dist/release"
while [ $# -gt 0 ]; do
    case "$1" in
        --out) OUT="${2:?--out needs a value}"; shift 2 ;;
        -h | --help) sed -n '2,10p' "$0"; exit 0 ;;
        *) pkg_die "unknown option: $1" ;;
    esac
done

SRC="$PKG_ROOT/apps/mgmt-plane"
MV="$(pkg_mgmt_version)"
printf '%s' "$MV" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]+)?$' || pkg_die "apps/mgmt-plane/VERSION '$MV' is not SemVer"
for f in synofurb_mgmt.py synofurb-mgmt.service README.md; do [ -f "$SRC/$f" ] || pkg_die "$SRC/$f is missing"; done
# the built-in fallback constant: DEFAULT_VERSION = "x" (the plane reads the VERSION file at runtime), or a literal VERSION = "x"
CODE_V="$(sed -n 's/^DEFAULT_VERSION = "\([^"]*\)".*/\1/p' "$SRC/synofurb_mgmt.py" | head -n 1)"
if [ -z "$CODE_V" ]; then CODE_V="$(sed -n 's/^VERSION = "\([^"]*\)".*/\1/p' "$SRC/synofurb_mgmt.py" | head -n 1)"; fi
if [ -z "$CODE_V" ]; then pkg_die "cannot find DEFAULT_VERSION = \"...\" (or VERSION = \"...\") in synofurb_mgmt.py"; fi
if [ "$CODE_V" != "$MV" ]; then
    pkg_die "version mismatch: apps/mgmt-plane/VERSION is '$MV' but synofurb_mgmt.py says '$CODE_V'. Make them equal (see docs/RELEASING.md)."
fi

mkdir -p "$OUT"
DEST="$OUT/synofurb-mgmt-$MV.tar.gz"
PY="$(pkg_python)"
"$PY" - "$DEST" "$SRC" <<'PYEOF'
import gzip, io, os, sys, tarfile

dest, src = sys.argv[1:3]
epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
files = [("synofurb_mgmt.py", 0o755), ("synofurb-mgmt.service", 0o644), ("README.md", 0o644), ("VERSION", 0o644)]
raw = io.BytesIO()
with tarfile.open(fileobj=raw, mode="w", format=tarfile.PAX_FORMAT) as tf:
    for name, mode in files:
        data = open(os.path.join(src, name), "rb").read().replace(b"\r\n", b"\n")
        ti = tarfile.TarInfo(name)
        ti.size, ti.mode, ti.mtime, ti.uid, ti.gid, ti.uname, ti.gname = len(data), mode, epoch, 0, 0, "root", "root"
        tf.addfile(ti, io.BytesIO(data))
with open(dest, "wb") as fh:
    with gzip.GzipFile(filename="", mode="wb", fileobj=fh, mtime=epoch, compresslevel=9) as gz:
        gz.write(raw.getvalue())
PYEOF
pkg_log "wrote $(basename "$DEST") ($(pkg_sha256 "$DEST"))"
