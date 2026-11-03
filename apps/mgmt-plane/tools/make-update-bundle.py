#!/usr/bin/env python3
"""Build an update directory for `synofurb_mgmt.py update --from DIR`.

    python3 tools/make-update-bundle.py OUT_DIR

OUT_DIR gets synofurb_mgmt.py, synofurb_addons.py, addons.pack.json (the add-on definitions of addons/ as one file), synofurb_edge.py (certificates and the HAProxy front), VERSION, README.md,
synofurb-mgmt.service and MANIFEST.sha256 (sha256sum format, every file listed).
"""
from __future__ import annotations

import hashlib
import os
import shutil
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FILES = ("synofurb_mgmt.py", "VERSION", "README.md", "synofurb-mgmt.service", "synofurb_addons.py", "addons.pack.json", "synofurb_edge.py")


def build_bundle(out: str, root: str = ROOT) -> list[str]:
    os.makedirs(out, exist_ok=True)
    lines = []
    for name in FILES:
        src = os.path.join(root, name)
        dst = os.path.join(out, name)
        if name == "addons.pack.json":                       # the flat bundle cannot carry addons/<id>/*: they travel as one JSON document
            sys.path.insert(0, root)
            import synofurb_addons
            with open(dst, "w", encoding="utf-8", newline="\n") as fh:
                fh.write(synofurb_addons.build_pack(os.path.join(root, "addons")))
        elif os.path.isfile(src):
            shutil.copyfile(src, dst)
        else:
            continue
        with open(dst, "rb") as fh:
            lines.append("%s  %s" % (hashlib.sha256(fh.read()).hexdigest(), name))
    with open(os.path.join(out, "MANIFEST.sha256"), "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines) + "\n")
    return lines


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    print("\n".join(build_bundle(sys.argv[1])))
