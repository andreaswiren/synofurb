#!/usr/bin/env python3
"""Build a dpkg status file from `apt-cache dumpavail` stanzas.

usage: mkstatus.py AVAIL_FILE NAMES_FILE > status

NAMES_FILE holds one `name` per line (the packages a simulated install would have put on the system). The stanza
of the version that `apt-cache dumpavail` lists first for each name is copied with `Status: install ok installed`.
Used by verify-mirror.sh to simulate the second apt step (kernel) on top of the first for architectures the host
cannot run.
"""
import sys

avail, names = sys.argv[1], set(open(sys.argv[2]).read().split())
seen = set()
for stanza in open(avail, encoding="utf-8", errors="replace").read().split("\n\n"):
    if not stanza.strip():
        continue
    fields = {}
    for line in stanza.splitlines():
        if line[:1] in " \t" or ":" not in line:
            continue
        k, v = line.split(":", 1)
        fields.setdefault(k, v.strip())
    name = fields.get("Package")
    if name in names and name not in seen:
        seen.add(name)
        print("Package: " + name)
        print("Status: install ok installed")
        for line in stanza.splitlines():
            if not line.startswith("Package:"):
                print(line)
        print()
missing = names - seen
if missing:
    print("mkstatus: not in dumpavail: " + " ".join(sorted(missing)), file=sys.stderr)
