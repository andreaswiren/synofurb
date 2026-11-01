#!/usr/bin/env python3
"""Build an update directory for `synofurb_mgmt.py update --from DIR`.

    python3 tools/make-update-bundle.py OUT_DIR

OUT_DIR gets synofurb_mgmt.py, VERSION, README.md, synofurb-mgmt.service and MANIFEST.sha256 (sha256sum format, every file listed).
"""
from __future__ import annotations

import hashlib
import os
import shutil
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FILES = ("synofurb_mgmt.py", "VERSION", "README.md", "synofurb-mgmt.service")


def build_bundle(out: str, root: str = ROOT) -> list[str]:
    os.makedirs(out, exist_ok=True)
    lines = []
    for name in FILES:
        src = os.path.join(root, name)
        if not os.path.isfile(src):
            continue
        dst = os.path.join(out, name)
        shutil.copyfile(src, dst)
        with open(dst, "rb") as fh:
            lines.append("%s  %s" % (hashlib.sha256(fh.read()).hexdigest(), name))
    with open(os.path.join(out, "MANIFEST.sha256"), "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines) + "\n")
    return lines


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    print("\n".join(build_bundle(sys.argv[1])))
