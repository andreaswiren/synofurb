#!/usr/bin/env python3
"""Generate THIRD-PARTY-LICENSES.txt for a synofurb release. Python 3 standard library only.

    python3 scripts/gen-licenses.py --out dist/sbom/THIRD-PARTY-LICENSES.txt            # runs cargo metadata --locked
    python3 scripts/gen-licenses.py --out ... --metadata meta.json --offline --strict

What it collects
  * synofurb's own LICENSE, the font licences (assets/brand/fonts/OFL-*.txt) and the Lucide licence (assets/brand/LICENSE-Lucide.txt)
  * for every Rust crate that ships in a build (dependency kind "normal" in scripts/gen-sbom.py, all target platforms, default
    features, proc-macro crates included) the licence / copying / notice files found in the crate source. Source comes from the
    directory `cargo metadata` reports (~/.cargo/registry/src/...), or, when that is missing, from the .crate archive in
    ~/.cargo/registry/cache. NO network access. Identical texts are printed once with the list of crates they apply to.
  * a pointer for the Debian packages inside the boot agent (their texts live in the .deb files; see the SBOM for names and versions).
Crates with no licence file in their source are listed in a "MISSING" section with the SPDX expression they declare. --strict makes
that a non-zero exit. The output is reproducible (sorted, no time). It is a release artefact: do not commit it.
This is an engineering aid, not legal advice (docs/compliance/README.md).
"""
from __future__ import annotations

import argparse
import glob
import hashlib
import importlib.util
import io
import json
import os
import re
import sys
import tarfile

HERE = os.path.dirname(os.path.abspath(__file__))
LICENCE_FILE_RX = re.compile(r"(?i)^(licen[cs]e|copying|unlicen[cs]e|notice|copyright|authors?)([-_. ].*)?$")
LICENCE_DIR_RX = re.compile(r"(?i)^(licen[cs]es?)$")
MAX_TEXT = 200_000  # bytes per file; a longer "licence" file is something else


def load_sbom_module():
    spec = importlib.util.spec_from_file_location("gen_sbom", os.path.join(HERE, "gen-sbom.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["gen_sbom"] = mod
    spec.loader.exec_module(mod)
    return mod


def norm_text(raw: bytes) -> str:
    t = raw.decode("utf-8", "replace").replace("\r\n", "\n").replace("\r", "\n")
    return "\n".join(line.rstrip() for line in t.strip("\n").split("\n")) + "\n"


def licence_files_in_dir(d: str):
    """[(relative name, bytes)] for licence-like files at the top of a crate directory (and in a LICENSES/ directory)."""
    out = []
    try:
        names = sorted(os.listdir(d))
    except OSError:
        return out
    for n in names:
        p = os.path.join(d, n)
        if os.path.isfile(p) and LICENCE_FILE_RX.match(n):
            out.append((n, p))
        elif os.path.isdir(p) and LICENCE_DIR_RX.match(n):
            for m in sorted(os.listdir(p)):
                q = os.path.join(p, m)
                if os.path.isfile(q):
                    out.append((n + "/" + m, q))
    res = []
    for name, p in out:
        try:
            with open(p, "rb") as fh:
                data = fh.read(MAX_TEXT + 1)
        except OSError:
            continue
        if 0 < len(data) <= MAX_TEXT:
            res.append((name, data))
    return res


def licence_files_in_crate_archive(name: str, version: str):
    for arc in sorted(glob.glob(os.path.expanduser("~/.cargo/registry/cache/*/%s-%s.crate" % (name, version)))):
        try:
            with tarfile.open(arc, "r:gz") as tf:
                res = []
                for m in tf.getmembers():
                    parts = m.name.split("/")
                    if not m.isfile() or len(parts) < 2 or parts[0] != "%s-%s" % (name, version):
                        continue
                    rel = "/".join(parts[1:])
                    ok = (len(parts) == 2 and LICENCE_FILE_RX.match(parts[1])) or (len(parts) == 3 and LICENCE_DIR_RX.match(parts[1]))
                    if ok and 0 < m.size <= MAX_TEXT:
                        res.append((rel, tf.extractfile(m).read()))
                return sorted(res)
        except (OSError, tarfile.TarError):
            continue
    return []


def collect(sb, meta, include_build=False):
    cls = sb.classify_dependencies(meta)
    ws = set(meta["workspace_members"])
    wanted = ("normal", "build") if include_build else ("normal",)
    pkgs = [p for p in meta["packages"] if p["id"] not in ws and cls.get(p["id"], {}).get("kind") in wanted]
    pkgs.sort(key=lambda p: (p["name"], p["version"]))
    groups, missing, from_archive = {}, [], 0
    for p in pkgs:
        files = licence_files_in_dir(os.path.dirname(p["manifest_path"]))
        if not files:
            files = licence_files_in_crate_archive(p["name"], p["version"])
            from_archive += bool(files)
        # only keep the licence files that are not just NOTICE-less duplicates of each other for this crate
        seen_here = set()
        for fname, raw in files:
            text = norm_text(raw)
            h = hashlib.sha256(text.encode()).hexdigest()
            if h in seen_here:
                continue
            seen_here.add(h)
            g = groups.setdefault(h, {"text": text, "who": []})
            g["who"].append((p["name"], p["version"], p.get("license") or "", fname))
        if not files:
            missing.append((p["name"], p["version"], (p.get("license") or "(none declared)") + (" | upstream " + p["repository"] if p.get("repository") else "")))
    return pkgs, groups, missing, from_archive


def read_repo_file(repo, rel):
    try:
        with open(os.path.join(repo, rel), "rb") as fh:
            return norm_text(fh.read())
    except OSError:
        return None


def render(sb, repo, version, pkgs, groups, missing):
    L = []
    w = L.append
    bar = "=" * 78
    w("synofurb %s - third-party notices and licence texts" % version)
    w("")
    w("synofurb itself is MIT licensed (part 1). It contains or ships the components below under their own licences.")
    w("Generated by scripts/gen-licenses.py from the Cargo dependency graph (all target platforms, default features) and the")
    w("licence files found in each crate's source. The SBOM (CycloneDX/SPDX) lists the same components with versions and hashes.")
    w("This file is an engineering aid, not legal advice. It makes no conformity claim.")
    w("")
    w(bar)
    w("PART 1 - synofurb")
    w(bar)
    w(read_repo_file(repo, "LICENSE") or "(LICENSE not found in the repository)")
    w(bar)
    w("PART 2 - fonts and icons bundled in the app, the management plane and the artwork")
    w(bar)
    for title, rel in (("Inter (SIL Open Font License 1.1)", "assets/brand/fonts/OFL-Inter.txt"),
                       ("JetBrains Mono (SIL Open Font License 1.1)", "assets/brand/fonts/OFL-JetBrainsMono.txt"),
                       ("Lucide icons (ISC; Feather-derived icons MIT)", "assets/brand/LICENSE-Lucide.txt")):
        w("---- " + title + " ----")
        w(read_repo_file(repo, rel) or "(%s not found in the repository)" % rel)
    w(bar)
    w("PART 3 - Rust crates compiled into synofurb (%d packages, %d distinct licence texts)" % (len(pkgs), len(groups)))
    w(bar)
    w("MPL-2.0 crates (file-level copyleft; the unmodified source of each is available from the crates.io URL in the SBOM): " +
      (", ".join("%s %s" % (p["name"], p["version"]) for p in pkgs if p.get("license") and "MPL-" in p["license"]) or "none"))
    w("")
    order = sorted(groups.values(), key=lambda g: (sorted(g["who"])[0][0], sorted(g["who"])[0][1]))
    for i, g in enumerate(order, 1):
        w("---- licence text %d of %d ----" % (i, len(order)))
        who = sorted(g["who"])
        w("Applies to (%d): %s" % (len(who), ", ".join("%s %s [%s]" % (n, v, lic) if lic else "%s %s" % (n, v) for n, v, lic, _f in who)))
        w("")
        w(g["text"])
    w(bar)
    w("PART 4 - crates with NO licence file in their source (%d)" % len(missing))
    w(bar)
    if missing:
        w("The licence is only known from the crate metadata (SPDX expression). A maintainer must add the text by hand or verify that")
        w("the upstream repository carries one. (Published crates often omit the files that sit at the root of a workspace.)")
        for n, v, lic in sorted(missing):
            w("  %s %s  declared: %s" % (n, v, lic))
    else:
        w("None: every shipped crate carries at least one licence file.")
    w("")
    w(bar)
    w("PART 5 - boot agent (synofurb-agent-<arch>-<version>.vmlinuz / .initrd) and Debian packages")
    w(bar)
    w("The boot agent images contain Debian's own kernel and the Debian packages busybox-static, dropbear-bin, kexec-tools, e2fsprogs,")
    w("dosfstools, netcat-traditional and the libraries they need, unpacked unmodified by boot/agent/build.sh. Exact package names, versions")
    w("and SHA-256 are in the SBOM (built from the build's MANIFEST-<arch>.json). Most of them are GPL-2.0 or GPL-3.0 licensed (the kernel:")
    w("GPL-2.0 with the Linux syscall note). Their licence texts are in the packages (/usr/share/doc/<package>/copyright) and the source")
    w("code of each exact version is available from the Debian archive (apt-get source <package>=<version>, or https://snapshot.debian.org/).")
    w("synofurb does not modify these packages. Whether this reference satisfies the GPL source obligation for a binary distribution on")
    w("GitHub is a question for a lawyer (docs/compliance/SBOM.md, 'Open decisions').")
    w("")
    return "\n".join(L).rstrip("\n") + "\n"


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Generate THIRD-PARTY-LICENSES.txt from the Cargo registry sources.")
    ap.add_argument("--repo", default=os.path.dirname(HERE))
    ap.add_argument("--out", default=None, help="output file (default dist/sbom/THIRD-PARTY-LICENSES.txt under --repo)")
    ap.add_argument("--metadata", help="cargo metadata JSON instead of running cargo")
    ap.add_argument("--offline", action="store_true")
    ap.add_argument("--include-build", action="store_true", help="also include build-dependency crates (not linked into the product)")
    ap.add_argument("--strict", action="store_true", help="exit 1 when a shipped crate has no licence file")
    a = ap.parse_args(argv)
    sb = load_sbom_module()
    repo = os.path.abspath(a.repo)
    meta = sb.load_json(a.metadata) if a.metadata else sb.run_cargo_metadata(repo, a.offline)
    ws = {p["id"]: p for p in meta["packages"]}
    prod = next((ws[w] for w in meta["workspace_members"] if ws[w]["name"] == "synofurb"), ws[sorted(meta["workspace_members"])[0]])
    pkgs, groups, missing, from_archive = collect(sb, meta, a.include_build)
    text = render(sb, repo, prod["version"], pkgs, groups, missing)
    out = os.path.abspath(a.out or os.path.join(repo, "dist", "sbom", "THIRD-PARTY-LICENSES.txt"))
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(text)
    print("crates: %d shipped packages, %d with licence text (%d read from .crate archives), %d without; %d distinct texts" % (
        len(pkgs), len(pkgs) - len(missing), from_archive, len(missing), len(groups)))
    for n, v, lic in sorted(missing):
        print("  MISSING licence file: %s %s (declared %s)" % (n, v, lic))
    print("wrote %s (%d bytes)" % (out, len(text.encode())))
    return 1 if (a.strict and missing) else 0


if __name__ == "__main__":
    sys.exit(main())
