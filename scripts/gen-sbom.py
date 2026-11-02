#!/usr/bin/env python3
"""Generate the synofurb SBOM (CycloneDX 1.5 + SPDX 2.3, JSON) and a licence report. Python 3 standard library only.

    python3 scripts/gen-sbom.py --out dist/sbom                      # writes the three files below
    python3 scripts/gen-sbom.py --out dist/sbom --timestamp 2026-11-02T00:00:00Z \\
        --commit "$GITHUB_SHA" --agent-manifest dist/agent-manifests --require-agent-manifest --strict
    python3 scripts/gen-sbom.py --check dist/sbom/synofurb-0.2.0-sbom.cdx.json dist/sbom/synofurb-0.2.0-sbom.spdx.json

Output (names are the release asset names, docs/compliance/SBOM.md):
    synofurb-<version>-sbom.cdx.json      CycloneDX 1.5
    synofurb-<version>-sbom.spdx.json     SPDX 2.3
    synofurb-<version>-sbom-report.json   licence-unknown, copyleft (with dependency chains), unresolved versions, counts

Inputs
  * the Rust workspace: `cargo metadata --format-version 1 --locked` (add --offline with --offline here) plus Cargo.lock
    checksums. The resolve covers ALL target platforms (a superset of any single build) and the default features of every
    workspace member (so it includes the GUI). Each crate is marked normal / build / dev from the resolve's dependency kinds.
  * the non-Cargo parts read from the repository: management plane (apps/mgmt-plane), embedded fonts and icons, the
    installers and the payload, the DOM script, build-time inputs of the boot agent (Debian packages: exact versions only
    when a boot/agent/build.sh MANIFEST-<arch>.json is supplied, otherwise listed as UNRESOLVED), the pinned kexec-tools source.
  * what the app downloads at run time (Debian installer, GitHub releases) is listed as external references, not components.

Reproducible: components, edges and properties are sorted; there is no wall-clock time. The only time that appears is the
one given with --timestamp (or SOURCE_DATE_EPOCH). serialNumber and the SPDX namespace are uuid5 of the content.

Exit status: 0 ok, 1 validation failed (or --strict and a gap was found), 2 usage or tool error.
This is an engineering artefact; it is not legal advice and makes no conformity claim (docs/compliance/README.md).
"""
from __future__ import annotations

import argparse
import glob
import hashlib
import json
import os
import re
import struct
import subprocess
import sys
import urllib.parse
import uuid
from collections import deque

try:  # Python >= 3.11
    import tomllib
except ImportError:  # pragma: no cover - fallback parser below
    tomllib = None

SCRIPT_NAME = "synofurb-gen-sbom"
SCRIPT_VERSION = "1"
NAMESPACE = uuid.uuid5(uuid.NAMESPACE_URL, "https://github.com/andreaswiren/synofurb/sbom")
REPO_URL = "https://github.com/andreaswiren/synofurb"
CRATES_REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
SPDX_PLACEHOLDER_TIME = "1970-01-01T00:00:00Z"
CDX_TYPES = {"application", "framework", "library", "container", "platform", "operating-system", "device", "device-driver",
             "firmware", "file", "machine-learning-model", "data"}
CDX_SCOPES = {"required", "optional", "excluded"}
HASH_LEN = {"SHA-256": 64, "SHA-1": 40, "SHA-512": 128, "MD5": 32}

# ----------------------------------------------------------------------------------------------------------------------
# SPDX licence expressions
# ----------------------------------------------------------------------------------------------------------------------
# Identifiers this script knows. An identifier outside this list is NOT rejected (the SPDX list grows); it is reported as
# "unrecognised" so a human looks at it. Add ids here after checking https://spdx.org/licenses/.
KNOWN_IDS = set("""0BSD AFL-2.1 AFL-3.0 AGPL-3.0-only AGPL-3.0-or-later Apache-1.1 Apache-2.0 Artistic-2.0 BSD-1-Clause BSD-2-Clause
BSD-3-Clause BSD-4-Clause BSL-1.0 CC-BY-4.0 CC-BY-SA-4.0 CC0-1.0 CDDL-1.0 CDLA-Permissive-2.0 ECL-2.0 EPL-1.0 EPL-2.0 EUPL-1.2 FTL
GPL-2.0-only GPL-2.0-or-later GPL-3.0-only GPL-3.0-or-later HPND ICU ISC LGPL-2.0-only LGPL-2.0-or-later LGPL-2.1-only
LGPL-2.1-or-later LGPL-3.0-only LGPL-3.0-or-later Libpng MIT MIT-0 MIT-advertising MPL-1.1 MPL-2.0 NCSA OFL-1.1 OpenSSL PostgreSQL
Python-2.0 Unicode-3.0 Unicode-DFS-2016 Unlicense Vim W3C WTFPL X11 Zlib Zlib-acknowledgement bzip2-1.0.6 curl
GPL-2.0 GPL-3.0 LGPL-2.1 LGPL-3.0 AGPL-3.0""".split())  # the last five are deprecated spellings that crates still use
KNOWN_EXCEPTIONS = set("""Classpath-exception-2.0 GCC-exception-3.1 LLVM-exception Linux-syscall-note Autoconf-exception-3.0
OpenSSL-exception Bison-exception-2.2 Font-exception-2.0 GCC-exception-2.0""".split())

COPYLEFT_FAMILIES = (  # (regex on the id, family, what it asks of a distributor of a MIT-licensed binary)
    (re.compile(r"^AGPL-"), "AGPL", "Network copyleft: the corresponding source of the whole combined work must be offered to every user, including users who only reach it over a network. Incompatible with shipping the combined binary as MIT-only."),
    (re.compile(r"^LGPL-"), "LGPL", "Weak copyleft for libraries: ship the licence text and a notice, offer the source of the LGPL library (and any changes to it) and let the user relink or replace it. Statically linked Rust binaries make the relinking part non-trivial (provide object files or a build recipe). Needs a legal decision unless an OR alternative is chosen."),
    (re.compile(r"^GPL-"), "GPL", "Strong copyleft: the combined work as a whole must be offered under GPL-compatible terms with its corresponding source. Cannot be combined into a MIT-only binary; either avoid it or relicense the whole."),
    (re.compile(r"^MPL-"), "MPL", "File-level copyleft: unmodified use needs only the licence text and a pointer to the source of the MPL files; modified MPL files must be published under the MPL. The rest of the MIT code is not affected."),
    (re.compile(r"^(EPL|CPL|CDDL)-"), "EPL/CDDL", "File/module-level copyleft: keep the licence, publish modified files of that component under the same licence."),
    (re.compile(r"^(EUPL|OSL|SSPL|CECILL|MS-RL)"), "other copyleft", "Copyleft licence: read its terms; needs a legal decision."),
    (re.compile(r"^CC-BY-SA"), "CC-BY-SA", "Share-alike: derived works of that data must use the same licence."),
)


class SpdxError(ValueError):
    pass


def normalise_expression(raw: str) -> str:
    """Crates use '/' (and sometimes lower-case) operators: 'MIT/Apache-2.0' -> 'MIT OR Apache-2.0'."""
    s = re.sub(r"\s*/\s*", " OR ", raw.strip())
    s = re.sub(r"\s+", " ", s)
    return re.sub(r"(?i)(?<=\s)(and|or|with)(?=\s)", lambda m: m.group(1).upper(), s)


def parse_expression(raw: str):
    """-> ('id', id, exception|None) | ('and', [..]) | ('or', [..]). Raises SpdxError."""
    tokens = re.findall(r"\(|\)|[^\s()]+", normalise_expression(raw))
    if not tokens:
        raise SpdxError("empty expression")
    pos = 0

    def peek():
        return tokens[pos] if pos < len(tokens) else None

    def take():
        nonlocal pos
        t = peek()
        pos += 1
        return t

    def ident(t):
        if t is None or t in ("(", ")") or t in ("AND", "OR", "WITH") or not re.fullmatch(r"[A-Za-z0-9.+:\-]+", t):
            raise SpdxError("unexpected token %r" % (t,))
        return t

    def atom():
        if peek() == "(":
            take()
            node = p_or()
            if take() != ")":
                raise SpdxError("missing )")
            return node
        i = ident(take())
        exc = None
        if peek() == "WITH":
            take()
            exc = ident(take())
        return ("id", i, exc)

    def p_and():
        items = [atom()]
        while peek() == "AND":
            take()
            items.append(atom())
        return items[0] if len(items) == 1 else ("and", items)

    def p_or():
        items = [p_and()]
        while peek() == "OR":
            take()
            items.append(p_and())
        return items[0] if len(items) == 1 else ("or", items)

    tree = p_or()
    if pos != len(tokens):
        raise SpdxError("trailing tokens: %r" % (tokens[pos:],))
    return tree


def render_expression(tree, parent=None) -> str:
    kind = tree[0]
    if kind == "id":
        return tree[1] + (" WITH " + tree[2] if tree[2] else "")
    joiner = " AND " if kind == "and" else " OR "
    s = joiner.join(render_expression(c, kind) for c in tree[1])
    return "(" + s + ")" if parent is not None and parent != kind else s


def expression_ids(tree):
    if tree[0] == "id":
        yield tree[1].rstrip("+"), tree[2]
    else:
        for c in tree[1]:
            yield from expression_ids(c)


def copyleft_family(lic_id: str):
    base = lic_id.rstrip("+")
    for rx, fam, text in COPYLEFT_FAMILIES:
        if rx.match(base):
            return fam, text
    return None


def copyleft_applies(tree) -> bool:
    """True when the licensee cannot avoid a copyleft choice: a lone copyleft id, an AND containing one, or an OR whose every branch is copyleft."""
    if tree[0] == "id":
        return copyleft_family(tree[1]) is not None
    if tree[0] == "and":
        return any(copyleft_applies(c) for c in tree[1])
    return all(copyleft_applies(c) for c in tree[1])


def mentions_copyleft(tree) -> bool:
    return any(copyleft_family(i) for i, _ in expression_ids(tree))


# ----------------------------------------------------------------------------------------------------------------------
# small helpers
# ----------------------------------------------------------------------------------------------------------------------
def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def read_text(path: str):
    try:
        with open(path, encoding="utf-8") as fh:
            return fh.read()
    except OSError:
        return None


def files_digest(repo: str, rels):
    """(count, sha256 over sorted 'sha256  path' lines) for a list of repo-relative files."""
    lines = sorted("%s  %s" % (sha256_file(os.path.join(repo, r)), r) for r in rels)
    return len(lines), sha256_bytes(("\n".join(lines) + "\n").encode())


def walk_files(repo: str, rel_dir: str):
    base = os.path.join(repo, rel_dir)
    out = []
    for root, dirs, files in os.walk(base):
        dirs[:] = sorted(d for d in dirs if d not in ("__pycache__", ".git"))
        for f in sorted(files):
            if f.endswith((".pyc", ".swp")):
                continue
            out.append(os.path.relpath(os.path.join(root, f), repo).replace(os.sep, "/"))
    return sorted(out)


def font_version(path: str):
    """Version string (name id 5) of an sfnt font, or None. Pure struct parsing; no fontTools."""
    try:
        with open(path, "rb") as fh:
            d = fh.read()
        n = struct.unpack(">H", d[4:6])[0]
        for i in range(n):
            tag, _cs, off, ln = struct.unpack(">4sIII", d[12 + 16 * i:28 + 16 * i])
            if tag != b"name":
                continue
            t = d[off:off + ln]
            _fmt, cnt, so = struct.unpack(">HHH", t[:6])
            for j in range(cnt):
                pid, _eid, _lid, nid, length, o = struct.unpack(">HHHHHH", t[6 + 12 * j:18 + 12 * j])
                if nid == 5 and pid == 3:
                    return t[so + o:so + o + length].decode("utf-16-be").strip()
    except (OSError, struct.error, UnicodeDecodeError):
        return None
    return None


def purl_quote(s: str) -> str:
    return urllib.parse.quote(s, safe="")


def parse_timestamp(arg):
    if arg:
        if not re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?(Z|[+-]\d\d:\d\d)", arg):
            raise SystemExit("--timestamp must be RFC 3339, e.g. 2026-11-02T00:00:00Z")
        return arg
    sde = os.environ.get("SOURCE_DATE_EPOCH", "").strip()
    if sde.isdigit():
        import datetime
        return datetime.datetime.fromtimestamp(int(sde), datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return None


# ----------------------------------------------------------------------------------------------------------------------
# Cargo
# ----------------------------------------------------------------------------------------------------------------------
def run_cargo_metadata(repo: str, offline: bool) -> dict:
    cmd = ["cargo", "metadata", "--format-version", "1", "--locked"] + (["--offline"] if offline else [])
    try:
        p = subprocess.run(cmd, cwd=repo, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    except OSError as e:
        raise SystemExit("cannot run cargo (%s); pass --metadata FILE with the output of `cargo metadata`" % e)
    if p.returncode != 0:
        sys.stderr.write(p.stderr.decode("utf-8", "replace"))
        raise SystemExit("`%s` failed (exit %d)" % (" ".join(cmd), p.returncode))
    return json.loads(p.stdout)


def parse_cargo_lock(text: str) -> dict:
    """{(name, version, source): checksum} from Cargo.lock text. Uses tomllib when present, a tiny parser otherwise."""
    out = {}
    if tomllib is not None:
        for p in tomllib.loads(text).get("package", []):
            if p.get("checksum"):
                out[(p["name"], p["version"], p.get("source", ""))] = p["checksum"]
        return out
    cur = {}

    def flush():
        if cur.get("checksum"):
            out[(cur["name"], cur["version"], cur.get("source", ""))] = cur["checksum"]

    for line in text.splitlines():
        if line.strip() == "[[package]]":
            flush()
            cur = {}
        else:
            m = re.match(r'^(name|version|source|checksum)\s*=\s*"([^"]*)"\s*$', line)
            if m:
                cur[m.group(1)] = m.group(2)
    flush()
    return out


def classify_dependencies(meta: dict) -> dict:
    """{package id: {'kind': normal|build|dev|unknown, 'conditional': bool}} from the resolve's dep_kinds.

    normal  = reachable from a workspace member through normal dependency edges (ships in at least one build)
    build   = only reachable through build-dependencies (build.rs / proc-macro helpers' own build tools)
    dev     = only reachable through dev-dependencies (tests, benches)
    conditional = the crate is only pulled in for some target platforms (cfg(...) dependencies), e.g. Windows-only crates
    """
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    ws = list(meta["workspace_members"])

    def edges(nid):
        for d in nodes[nid]["deps"]:
            for k in d["dep_kinds"]:
                yield d["pkg"], k["kind"], k["target"]

    def closure(seeds, follow):
        seen = set(seeds)
        q = deque(seeds)
        while q:
            cur = q.popleft()
            for pkg, kind, target in edges(cur):
                if follow(kind, target) and pkg not in seen and pkg in nodes:
                    seen.add(pkg)
                    q.append(pkg)
        return seen

    normal = closure(ws, lambda k, t: k is None)
    uncond = closure(ws, lambda k, t: k is None and t is None)
    build_seeds = {p for n in normal for p, k, _ in edges(n) if k == "build"}
    build = closure(build_seeds, lambda k, t: k in (None, "build"))
    dev_seeds = {p for n in ws for p, k, _ in edges(n) if k == "dev"}
    dev = closure(dev_seeds, lambda k, t: k in (None, "build"))
    out = {}
    for nid in nodes:
        if nid in normal:
            out[nid] = {"kind": "normal", "conditional": nid not in uncond}
        elif nid in build:
            out[nid] = {"kind": "build", "conditional": False}
        elif nid in dev:
            out[nid] = {"kind": "dev", "conditional": False}
        else:
            out[nid] = {"kind": "unknown", "conditional": False}
    return out


def dependency_chain(meta: dict, target_id: str, pkg_by_id: dict, prefer_normal=True):
    """Shortest chain of 'name version' strings from a workspace member to target_id (normal edges first)."""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    for only_normal in ((True, False) if prefer_normal else (False,)):
        prev = {}
        q = deque()
        for w in sorted(meta["workspace_members"]):
            prev[w] = None
            q.append(w)
        found = target_id in prev
        while q and not found:
            cur = q.popleft()
            for d in sorted(nodes[cur]["deps"], key=lambda x: x["pkg"]):
                if only_normal and not any(k["kind"] is None for k in d["dep_kinds"]):
                    continue
                if d["pkg"] not in prev:
                    prev[d["pkg"]] = cur
                    if d["pkg"] == target_id:
                        found = True
                        break
                    q.append(d["pkg"])
        if found:
            chain = []
            cur = target_id
            while cur is not None:
                p = pkg_by_id[cur]
                chain.append("%s %s" % (p["name"], p["version"]))
                cur = prev[cur]
            return list(reversed(chain)), ("normal" if only_normal else "build/dev")
    return [], "unreachable"


# ----------------------------------------------------------------------------------------------------------------------
# the neutral model
# ----------------------------------------------------------------------------------------------------------------------
class Component(dict):
    """keys: ref type name version [purl licence licence_name hashes scope props ext description group source_kind cargo_id]"""

    def __init__(self, **kw):
        super().__init__(licence=None, licence_name=None, hashes=[], scope="required", props={}, ext=[], description=None,
                         group=None, purl=None, source_kind="extra", cargo_id=None)
        self.update(kw)


def cargo_purl(pkg: dict) -> str:
    base = "pkg:cargo/%s@%s" % (purl_quote(pkg["name"]), purl_quote(pkg["version"]))
    src = pkg.get("source")
    if src and src != CRATES_REGISTRY and not src.startswith("registry+sparse+https://index.crates.io"):
        if src.startswith("git+"):
            base += "?vcs_url=" + purl_quote(src[4:])
        elif src.startswith("registry"):
            base += "?repository_url=" + purl_quote(src.split("+", 1)[1])
    return base


def cargo_components(meta: dict, lock: dict, ws_licence_fallback="MIT"):
    pkg_by_id = {p["id"]: p for p in meta["packages"]}
    cls = classify_dependencies(meta)
    ws_ids = set(meta["workspace_members"])
    comps, id_to_ref, seen_refs = [], {}, {}
    for pid in sorted(pkg_by_id, key=lambda i: (pkg_by_id[i]["name"], pkg_by_id[i]["version"], i)):
        p = pkg_by_id[pid]
        purl = cargo_purl(p)
        ref = purl
        if ref in seen_refs:  # same name+version from two sources: disambiguate
            ref = "%s#%s" % (purl, sha256_bytes(pid.encode())[:8])
        seen_refs[ref] = pid
        id_to_ref[pid] = ref
        c = Component(ref=ref, type="library", name=p["name"], version=p["version"], purl=purl, cargo_id=pid, source_kind="cargo")
        is_ws = pid in ws_ids
        info = cls.get(pid, {"kind": "unknown", "conditional": False})
        props = {"synofurb:dependency-kind": info["kind"]}
        if is_ws:
            props["synofurb:workspace-member"] = "true"
            if any("bin" in t["kind"] for t in p["targets"]):
                c["type"] = "application"
        else:
            props["synofurb:cargo-source"] = "crates.io" if p.get("source") == CRATES_REGISTRY or (p.get("source") or "").startswith("registry+sparse+https://index.crates.io") else (p.get("source") or "path")
        if info["conditional"]:
            props["synofurb:platform-conditional"] = "true"
        if any("proc-macro" in t["kind"] for t in p["targets"]):
            props["synofurb:proc-macro"] = "true"
        c["scope"] = "required" if info["kind"] in ("normal",) else "excluded"
        raw = p.get("license")
        if raw:
            try:
                tree = parse_expression(raw)
                c["licence"] = render_expression(tree)
                if c["licence"] != raw:
                    props["synofurb:licence-declared-as"] = raw
            except SpdxError as e:
                c["licence_name"] = raw
                props["synofurb:licence-unparsable"] = str(e)
        elif p.get("license_file"):
            c["licence_name"] = "SEE LICENSE FILE: %s" % os.path.basename(p["license_file"])
        elif is_ws:
            c["licence"] = ws_licence_fallback
        c["props"] = props
        if not is_ws:
            chk = lock.get((p["name"], p["version"], p.get("source") or ""))
            if chk:
                c["hashes"] = [("SHA-256", chk)]
            if p.get("source") == CRATES_REGISTRY or (p.get("source") or "").startswith("registry+sparse+https://index.crates.io"):
                c["ext"].append(("distribution", "https://crates.io/api/v1/crates/%s/%s/download" % (p["name"], p["version"]), "crates.io download"))
        if p.get("repository"):
            c["ext"].append(("vcs", p["repository"], ""))
        if p.get("homepage") and p.get("homepage") != p.get("repository"):
            c["ext"].append(("website", p["homepage"], ""))
        if p.get("description"):
            c["description"] = " ".join(p["description"].split())[:300]
        comps.append(c)
    edges = {}
    for n in meta["resolve"]["nodes"]:
        edges[id_to_ref[n["id"]]] = sorted({id_to_ref[d["pkg"]] for d in n["deps"] if d["pkg"] in id_to_ref})
    return comps, edges, id_to_ref, pkg_by_id, cls


# ----------------------------------------------------------------------------------------------------------------------
# non-Cargo components
# ----------------------------------------------------------------------------------------------------------------------
# Licences of Debian packages that the boot-agent build pulls in. DECLARED BY THIS SCRIPT FROM GENERAL KNOWLEDGE, NOT READ
# FROM THE PACKAGES: each is marked synofurb:licence-verified=false. The authority is /usr/share/doc/<pkg>/copyright inside the .deb.
DEBIAN_LICENCES = {
    "busybox-static": "GPL-2.0-only",
    "dropbear-bin": "MIT",
    "kexec-tools": "GPL-2.0-only",
    "e2fsprogs": "GPL-2.0-only",
    "dosfstools": "GPL-3.0-or-later",
    "libc6": "LGPL-2.1-or-later",
    "libgcc-s1": "GPL-3.0-or-later WITH GCC-exception-3.1",
    "zlib1g": "Zlib",
    "liblzma5": "0BSD",
}
AGENT_ROOT_PACKAGES = ("busybox-static", "dropbear-bin", "kexec-tools", "e2fsprogs", "dosfstools", "netcat-traditional")
AGENT_KERNEL_PLACEHOLDERS = ("linux-image-amd64", "linux-image-arm64", "linux-image-armmp")
MGMT_RUNTIME_DEBIAN = "openssl iproute2 util-linux mdadm lvm2 xfsprogs e2fsprogs btrfs-progs ntfs-3g exfatprogs smartmontools openssh-server"
EXTERNAL_REFERENCES = (  # what the app and the scripts fetch at run time: not components of the product
    ("distribution", "https://deb.debian.org/debian", "Debian archive: installer netboot kernel/initrd (crates/synofurb-core/src/debian.rs) and the packages the installer and the offline bundle fetch. Verified at run time: InRelease -> installer SHA256SUMS -> file (PGP only if gpgv and the Debian keyring exist on the user's computer)."),
    ("distribution", "https://security.debian.org/debian-security", "Debian security archive (boot agent build, installed system)."),
    ("distribution", "https://cdimage.debian.org/cdimage/firmware", "Debian non-free firmware archive (optional, SHA256SUMS fetched over HTTPS)."),
    ("distribution", "https://api.github.com/repos/andreaswiren/synofurb/releases", "Update channel read by the in-app updater; assets are verified against SHA256SUMS + SHA256SUMS.minisig (docs/UPDATES.md)."),
    ("distribution", "https://mirrors.edge.kernel.org/pub/linux/utils/kernel/kexec", "kexec-tools source tarball, SHA-256 pinned in scripts/build-kexec.sh."),
    ("other", "https://www.synology.com/", "Synology DSM on the target unit: the product reads and replaces parts of it; it is not a component of synofurb and is not redistributed."),
)


def _grep(text, pattern):
    m = re.search(pattern, text or "", re.M)
    return m.group(1) if m else None


def extra_components(repo: str, product_version: str, manifests: list, require_manifest: bool):
    """-> (components, edges {ref: [refs]}, notes[list of str], unresolved[list of dict])"""
    comps, edges, notes, unresolved = [], {}, [], []
    top = []  # refs the product depends on

    def add(c, product_dep=True):
        comps.append(c)
        if product_dep and c["scope"] != "excluded":
            top.append(c["ref"])
        return c

    # --- management plane -------------------------------------------------------------------------------------------
    mg = os.path.join(repo, "apps/mgmt-plane")
    mg_ver = (read_text(os.path.join(mg, "VERSION")) or "").strip()
    mg_py = os.path.join(mg, "synofurb_mgmt.py")
    if mg_ver and os.path.exists(mg_py):
        import ast
        src = read_text(mg_py)
        imports = set()
        try:
            for n in ast.walk(ast.parse(src)):
                if isinstance(n, ast.Import):
                    imports |= {a.name.split(".")[0] for a in n.names}
                elif isinstance(n, ast.ImportFrom) and n.level == 0 and n.module:
                    imports.add(n.module.split(".")[0])
        except SyntaxError:
            imports = set()
        std = getattr(sys, "stdlib_module_names", None)
        third = sorted(i for i in imports if std is not None and i not in std)
        c = Component(ref="synofurb-mgmt@" + mg_ver, type="application", name="synofurb-mgmt", version=mg_ver, licence="MIT",
                      hashes=[("SHA-256", sha256_file(mg_py))], purl="pkg:generic/synofurb-mgmt@" + purl_quote(mg_ver),
                      description="Single-file web management plane (HTTPS, port 8443) installed on the NAS; runs as root under systemd.")
        c["props"] = {"synofurb:runtime": "python3, standard library only",
                      "synofurb:stdlib-imports": ",".join(sorted(imports)),
                      "synofurb:non-stdlib-imports": ",".join(third) if third else "none",
                      "synofurb:runtime-debian-packages": MGMT_RUNTIME_DEBIAN}
        if third:
            notes.append("apps/mgmt-plane imports modules outside the standard library: %s" % ", ".join(third))
        add(c)
        py = Component(ref="python3-runtime", type="platform", name="python3", version="3.13", purl="pkg:deb/debian/python3?distro=debian-13",
                       licence="Python-2.0", scope="required",
                       description="CPython from the Debian 13 package python3 (installed by the installer, not shipped by synofurb).")
        py["props"] = {"synofurb:licence-verified": "false",
                       "synofurb:version-note": "tested with 3.13 (Debian 13) and 3.12 (CI on ubuntu-24.04); no older minimum has been tested",
                       "synofurb:provided-by": "Debian package python3 on the NAS"}
        add(py, product_dep=False)
        edges[c["ref"]] = [py["ref"]]
    else:
        notes.append("apps/mgmt-plane not found: management plane not listed")

    # --- fonts and icons ------------------------------------------------------------------------------------------------
    font_groups = {}
    for rel in ("crates/synofurb/assets/fonts", "assets/brand/fonts"):
        d = os.path.join(repo, rel)
        if not os.path.isdir(d):
            continue
        for f in sorted(os.listdir(d)):
            if f.endswith((".ttf", ".otf")):
                fam = "Inter" if f.startswith("Inter") else "JetBrains Mono" if f.startswith("JetBrainsMono") else None
                v = font_version(os.path.join(d, f))
                if fam and v:
                    ver = re.match(r"Version\s+([0-9.]+)", v)
                    key = (fam, ver.group(1) if ver else v)
                    font_groups.setdefault(key, []).append(rel + "/" + f)
    for (fam, ver), files in sorted(font_groups.items()):
        used = ("GUI binary (include_bytes!)" if files[0].startswith("crates/") else "source of the woff2 subsets embedded in the management plane (apps/mgmt-plane/tools/build-assets.py) and of the wordmark outlines")
        c = Component(ref="font:%s@%s" % (fam.lower().replace(" ", "-"), ver), type="data", name=fam, version=ver, licence="OFL-1.1",
                      description="%s font files, SIL Open Font License 1.1" % fam)
        c["props"] = {"synofurb:used-in": used, "synofurb:licence-text": "assets/brand/fonts/OFL-%s.txt" % fam.replace(" ", ""),
                      **{"synofurb:file-sha256:" + f: sha256_file(os.path.join(repo, f)) for f in files}}
        c["ext"] = [("vcs", "https://github.com/rsms/inter" if fam == "Inter" else "https://github.com/JetBrains/JetBrainsMono", "")]
        add(c)
    if len({v for (f, v) in font_groups if f == "Inter"}) > 1 or len({v for (f, v) in font_groups if f == "JetBrains Mono"}) > 1:
        notes.append("the repository carries more than one version of the same font family (see the font components): " +
                     "; ".join("%s %s" % k for k in sorted(font_groups)))

    lucide_dir = None
    for cand in sorted(glob.glob(os.path.expanduser("~/.cargo/registry/src/*/gpui-kit-assets-*"))):
        lucide_dir = cand
    lucide_ver = None
    if lucide_dir:
        try:
            lucide_ver = json.loads(read_text(os.path.join(lucide_dir, "lucide.json")) or "{}").get("version")
        except ValueError:
            lucide_ver = None
    c = Component(ref="icons:lucide", type="data", name="lucide-icons", version=lucide_ver or "unrecorded", licence="ISC AND MIT",
                  description="Lucide icon set (ISC; icons derived from Feather are MIT). GUI: via the gpui-kit-assets crate; management plane: a fixed subset inlined as an SVG sprite.")
    c["props"] = {"synofurb:licence-text": "assets/brand/LICENSE-Lucide.txt",
                  "synofurb:version-source": "lucide.json of the gpui-kit-assets crate in the local cargo registry" if lucide_ver else "not recorded: the registry copy of gpui-kit-assets was not available",
                  "synofurb:mgmt-subset-version-recorded": "false"}
    c["ext"] = [("vcs", "https://github.com/lucide-icons/lucide", "")]
    if not lucide_ver:
        unresolved.append({"ref": c["ref"], "name": c["name"], "reason": "Lucide version not recorded (gpui-kit-assets not in the cargo registry on this machine); the management-plane subset's version is never recorded"})
    add(c)

    # --- installers, payload, DOM script --------------------------------------------------------------------------------
    inst = [r for r in ("install.sh", "install.ps1", "apps/mgmt-plane/install.sh") if os.path.exists(os.path.join(repo, r))]
    if inst:
        n, dig = files_digest(repo, inst)
        c = Component(ref="synofurb-installers@" + product_version, type="file", name="synofurb-installers", version=product_version, licence="MIT",
                      description="One-line installers (install.sh, install.ps1, apps/mgmt-plane/install.sh); verify SHA-256 + minisign of the release.")
        c["props"] = {"synofurb:files": ",".join(inst), "synofurb:files-digest-sha256": dig, "synofurb:files-count": str(n)}
        add(c)
    if os.path.isdir(os.path.join(repo, "payload")):
        files = walk_files(repo, "payload")
        n, dig = files_digest(repo, files)
        c = Component(ref="synofurb-payload@" + product_version, type="file", name="synofurb-payload", version=product_version, licence="MIT",
                      description="Files and scripts the installer copies into the new Debian system (sshd drop-in, unattended-upgrades, first-boot, boot-flash update hooks).")
        c["props"] = {"synofurb:files-digest-sha256": dig, "synofurb:files-count": str(n), "synofurb:files": ",".join(files)}
        add(c)
    dom = os.path.join(repo, "boot/dom/synofurb-dom")
    if os.path.exists(dom):
        v = _grep(read_text(dom), r"^VERSION=([0-9][^\s]*)") or "unrecorded"
        c = Component(ref="synofurb-dom@" + v, type="application", name="synofurb-dom", version=v, licence="MIT", hashes=[("SHA-256", sha256_file(dom))],
                      description="Boot-flash (DOM) manager shell script, installed on the NAS and embedded in the boot agent.")
        add(c)

    # --- kexec-tools (pinned source, built by scripts/build-kexec.sh; the binary is NOT in the release) -----------------
    bk = read_text(os.path.join(repo, "scripts/build-kexec.sh"))
    if bk:
        v, h = _grep(bk, r"^VERSION=([0-9][^\s]*)"), _grep(bk, r"^SHA256=([0-9a-f]{64})")
        if v:
            c = Component(ref="kexec-tools-source@" + v, type="library", name="kexec-tools", version=v, licence="GPL-2.0-only", scope="optional",
                          purl="pkg:generic/kexec-tools@" + v + "?download_url=" + purl_quote("https://mirrors.edge.kernel.org/pub/linux/utils/kernel/kexec/kexec-tools-%s.tar.xz" % v),
                          hashes=[("SHA-256", h)] if h else [], description="Source built into a static kexec uploaded to DSM by the app; not part of the release assets unless the maintainer builds and bundles it.")
            c["props"] = {"synofurb:licence-verified": "false", "synofurb:build-script": "scripts/build-kexec.sh",
                          "synofurb:note": "scripts/build-kexec.sh has never been run by the author (header of the script)"}
            add(c)

    # --- boot agent (Debian build-time inputs) ------------------------------------------------------------------------------
    agent_refs = []
    exact = False
    for mpath in manifests:
        try:
            m = json.loads(read_text(mpath) or "")
        except ValueError:
            notes.append("agent manifest %s is not valid JSON; ignored" % mpath)
            continue
        arch, suite = m.get("arch", "unknown"), m.get("suite", "unknown")
        for p in m.get("packages", []):
            name, ver = p.get("name"), p.get("version")
            if not name or not ver:
                continue
            purl = "pkg:deb/debian/%s@%s?arch=%s&distro=debian-13" % (purl_quote(name), purl_quote(ver), arch)
            lic = DEBIAN_LICENCES.get(name)
            if name.startswith(("linux-image-", "linux-modules-")):
                lic = "GPL-2.0-only WITH Linux-syscall-note"
            c = Component(ref=purl, type="firmware" if name.startswith("linux-image-") else "library", name=name, version=ver, purl=purl, licence=lic,
                          hashes=[("SHA-256", p["sha256"])] if re.fullmatch(r"[0-9a-f]{64}", p.get("sha256", "")) else [],
                          description="Debian package unpacked into the boot agent (arch %s, suite %s)" % (arch, suite))
            c["props"] = {"synofurb:agent-arch": arch, "synofurb:debian-suite": suite, "synofurb:agent-version": str(m.get("agent_version", "")),
                          "synofurb:pgp-verified-by-build": str(m.get("pgp_verified", "unknown")).lower(), "synofurb:distributed-in": "synofurb-agent-%s-<version>.{vmlinuz,initrd}" % arch}
            if lic:
                c["props"]["synofurb:licence-verified"] = "false"
            if p.get("source"):
                c["ext"].append(("distribution", p["source"], "as recorded by boot/agent/build.sh"))
            if m.get("dom_stub"):
                c["props"]["synofurb:agent-built-with-dom-stub"] = "true"
                notes.append("agent manifest %s says dom_stub=true: that build must not be shipped" % mpath)
            if c["ref"] not in {x["ref"] for x in comps}:
                add(c)
                agent_refs.append(c["ref"])
                exact = True
    if not exact:
        for name in AGENT_ROOT_PACKAGES + AGENT_KERNEL_PLACEHOLDERS:
            c = Component(ref="pkg:deb/debian/%s?distro=debian-13#unresolved" % name, type="firmware" if name.startswith("linux-image") else "library", name=name,
                          version="UNRESOLVED", licence=DEBIAN_LICENCES.get(name) or ("GPL-2.0-only WITH Linux-syscall-note" if name.startswith("linux-image") else None),
                          description="Boot agent build-time input, fetched from the Debian archive when the release is built (boot/agent/build.sh); exact version only in MANIFEST-<arch>.json.")
            c["props"] = {"synofurb:version-unresolved": "true",
                          "synofurb:version-rule": "highest version in Debian trixie (+ trixie-backports for the kernel, default suite) at build time; dependencies followed except SKIP_DEPS; see boot/agent/build.sh",
                          "synofurb:distributed-in": "synofurb-agent-<arch>-<version>.{vmlinuz,initrd}"}
            if c["licence"]:
                c["props"]["synofurb:licence-verified"] = "false"
            add(c)
            agent_refs.append(c["ref"])
            unresolved.append({"ref": c["ref"], "name": name, "reason": "boot agent Debian package version not pinned: pass the MANIFEST-<arch>.json files of the agent build (--agent-manifest)"})
        msg = "no boot-agent MANIFEST-<arch>.json supplied: agent components are listed UNRESOLVED"
        notes.append(msg)
        if require_manifest:
            notes.append("ERROR: --require-agent-manifest was given but " + msg)

    # --- build-time tools (excluded from the product) ----------------------------------------------------------------------
    cp = None
    for wf in sorted(glob.glob(os.path.join(repo, ".github/workflows/*.yml"))):
        cp = cp or _grep(read_text(wf), r'^\s*CARGO_PACKAGER_VERSION:\s*"?([0-9][0-9.]*)"?')
    if cp:
        c = Component(ref="tool:cargo-packager@" + cp, type="application", name="cargo-packager", version=cp, scope="excluded", purl="pkg:cargo/cargo-packager@" + cp,
                      licence="Apache-2.0 OR MIT", description="Builds the MSI/NSIS/DMG/deb/AppImage installers; build tool, not shipped. License as declared upstream, not verified here.")
        c["props"] = {"synofurb:dependency-kind": "build-tool", "synofurb:licence-verified": "false"}
        add(c, product_dep=False)
    mk = Component(ref="tool:rust-toolchain", type="application", name="rust-toolchain", version=_grep(read_text(os.path.join(repo, "Cargo.toml")), r'^rust-version\s*=\s*"([^"]+)"') or "unrecorded",
                   scope="excluded", licence="MIT OR Apache-2.0", description="Rust compiler; the version is the minimum supported (rust-version); CI builds with the current stable toolchain, which is not recorded.")
    mk["props"] = {"synofurb:dependency-kind": "build-tool", "synofurb:licence-verified": "false", "synofurb:version-note": "minimum supported Rust version; the toolchain used for a release is not recorded"}
    add(mk, product_dep=False)

    # edges: the product depends on `top`; the agent components hang under a pseudo edge list on the product only
    return comps, edges, notes, unresolved, top


def find_manifests(args_paths, repo):
    out = []
    paths = list(args_paths or [])
    if not paths:
        paths = sorted(glob.glob(os.path.join(repo, "dist/agent-*/MANIFEST-*.json")) + glob.glob(os.path.join(repo, "boot/agent/dist/MANIFEST-*.json")))
    for p in paths:
        if os.path.isdir(p):
            out += sorted(glob.glob(os.path.join(p, "**", "MANIFEST-*.json"), recursive=True))
        elif os.path.exists(p):
            out.append(p)
        else:
            raise SystemExit("agent manifest not found: %s" % p)
    return sorted(set(out))


# ----------------------------------------------------------------------------------------------------------------------
# assembling
# ----------------------------------------------------------------------------------------------------------------------
def build_model(repo, meta, lock_text, manifests, require_manifest=False, commit=None):
    lock = parse_cargo_lock(lock_text) if lock_text else {}
    ccomps, cedges, id_to_ref, pkg_by_id, cls = cargo_components(meta, lock)
    ws_names = sorted(pkg_by_id[w]["name"] for w in meta["workspace_members"])
    prod_pkg = next((pkg_by_id[w] for w in meta["workspace_members"] if pkg_by_id[w]["name"] == "synofurb"), pkg_by_id[sorted(meta["workspace_members"])[0]])
    version = prod_pkg["version"]
    xcomps, xedges, notes, unresolved, top = extra_components(repo, version, manifests, require_manifest)
    product = Component(ref="pkg:github/andreaswiren/synofurb@v" + version, type="application", name="synofurb", version=version, licence="MIT",
                        purl="pkg:github/andreaswiren/synofurb@v" + version,
                        description="Desktop tool and command line that reinstall Synology NAS units with plain Debian, with the NAS-side boot agent and management plane.")
    product["ext"] = [("vcs", REPO_URL, ""), ("website", REPO_URL, ""), ("issue-tracker", REPO_URL + "/issues", ""), ("advisories", REPO_URL + "/security/advisories", "")]
    product["props"] = {"synofurb:rust-workspace-members": ",".join(ws_names)}
    if commit:
        product["props"]["synofurb:vcs-commit"] = commit
    comps = sorted(ccomps + xcomps, key=lambda c: (c["name"], c["version"], c["ref"]))
    refs = {c["ref"] for c in comps}
    edges = {r: list(v) for r, v in cedges.items()}
    for r, v in xedges.items():
        edges[r] = sorted(set(edges.get(r, []) + v))
    ws_refs = sorted(id_to_ref[w] for w in meta["workspace_members"])
    agent_and_tools = sorted(set(top))
    edges[product["ref"]] = sorted(set(ws_refs + agent_and_tools))
    for r in refs:
        edges.setdefault(r, [])
    # build tools hang off the product as build edges so that SPDX can say BUILD_DEPENDENCY_OF
    tool_refs = sorted(c["ref"] for c in comps if c["scope"] == "excluded" and c["source_kind"] == "extra")
    edges[product["ref"]] = sorted(set(edges[product["ref"]] + tool_refs))
    return {"product": product, "components": comps, "edges": edges, "notes": notes, "unresolved": unresolved, "meta": meta,
            "id_to_ref": id_to_ref, "pkg_by_id": pkg_by_id, "cls": cls}


# ----------------------------------------------------------------------------------------------------------------------
# CycloneDX
# ----------------------------------------------------------------------------------------------------------------------
def cdx_component(c):
    out = {"type": c["type"], "bom-ref": c["ref"], "name": c["name"], "version": c["version"]}
    if c["group"]:
        out["group"] = c["group"]
    if c["description"]:
        out["description"] = c["description"]
    out["scope"] = c["scope"]
    if c["hashes"]:
        out["hashes"] = [{"alg": a, "content": h} for a, h in c["hashes"]]
    props = dict(c["props"])
    if c["licence"]:
        out["licenses"] = [{"expression": c["licence"]}]
    elif c["licence_name"]:
        out["licenses"] = [{"license": {"name": c["licence_name"]}}]
        props["synofurb:licence-unknown"] = "true"
    else:
        props["synofurb:licence-unknown"] = "true"
    if c["purl"]:
        out["purl"] = c["purl"]
    if c["ext"]:
        out["externalReferences"] = [{"type": t, "url": u, **({"comment": cm} if cm else {})} for t, u, cm in sorted(c["ext"])]
    if props:
        out["properties"] = [{"name": k, "value": v} for k, v in sorted(props.items())]
    return out


def make_cyclonedx(model, timestamp):
    prod = model["product"]
    comps = model["components"]
    doc = {"bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1}
    meta = {}
    if timestamp:
        meta["timestamp"] = timestamp
    meta["tools"] = {"components": [{"type": "application", "name": SCRIPT_NAME, "version": SCRIPT_VERSION, "description": "scripts/gen-sbom.py in the synofurb repository"}]}
    mc = cdx_component(prod)
    mc.pop("scope", None)
    meta["component"] = mc
    meta["properties"] = [
        {"name": "synofurb:sbom-scope", "value": "Rust workspace (all targets, default features, Cargo.lock), management plane, embedded fonts/icons, installers, payload, boot-agent build inputs. NOT covered: system libraries the GUI links dynamically on the user's OS, Debian packages installed on the NAS by the installer, packages the installer downloads at run time, Synology DSM."},
        {"name": "synofurb:statement", "value": "Engineering SBOM, not a conformity claim and not legal advice (docs/compliance/README.md)."},
    ]
    doc["metadata"] = meta
    doc["components"] = [cdx_component(c) for c in comps]
    deps = []
    for ref in sorted(model["edges"]):
        deps.append({"ref": ref, "dependsOn": sorted(set(model["edges"][ref]))})
    doc["dependencies"] = deps
    doc["externalReferences"] = [{"type": t, "url": u, "comment": cm} for t, u, cm in EXTERNAL_REFERENCES]
    doc["compositions"] = [{"aggregate": "incomplete", "assemblies": [prod["ref"]]}]
    ser = "urn:uuid:" + str(uuid.uuid5(NAMESPACE, json.dumps(doc, sort_keys=True)))
    out = {"bomFormat": doc["bomFormat"], "specVersion": doc["specVersion"], "serialNumber": ser}
    out.update({k: v for k, v in doc.items() if k not in out})
    return out


# ----------------------------------------------------------------------------------------------------------------------
# SPDX
# ----------------------------------------------------------------------------------------------------------------------
def spdx_licence(licence, extracted):
    """-> SPDX expression string using only valid ids (unknown ids become LicenseRef-...), or NOASSERTION."""
    if not licence:
        return "NOASSERTION"
    tree = parse_expression(licence)

    def conv(t):
        if t[0] == "id":
            lid, exc = t[1], t[2]
            base = lid.rstrip("+")
            if base not in KNOWN_IDS and not lid.startswith("LicenseRef-"):
                ref = "LicenseRef-" + re.sub(r"[^A-Za-z0-9.\-]", "-", base)
                extracted[ref] = "Identifier %r is not in this script's list of SPDX ids; taken verbatim from the package metadata." % base
                return ("id", ref, None)
            if exc and exc not in KNOWN_EXCEPTIONS:
                return None
            return t
        kids = [conv(c) for c in t[1]]
        if any(k is None for k in kids):
            return None
        return (t[0], kids)

    conv_tree = conv(tree)
    return render_expression(conv_tree) if conv_tree else "NOASSERTION"


def spdx_id(c):
    return "SPDXRef-%s-%s-%s" % ("Package", re.sub(r"[^A-Za-z0-9.\-]", "-", c["name"])[:60], sha256_bytes(c["ref"].encode())[:10])


PURPOSE = {"application": "APPLICATION", "library": "LIBRARY", "firmware": "FIRMWARE", "platform": "OTHER", "data": "OTHER", "file": "FILE"}


def make_spdx(model, timestamp):
    prod = model["product"]
    comps = [prod] + model["components"]
    ids = {c["ref"]: spdx_id(c) for c in comps}
    extracted = {}
    pkgs = []
    for c in comps:
        licence = spdx_licence(c["licence"], extracted)
        p = {"SPDXID": ids[c["ref"]], "name": c["name"], "versionInfo": c["version"], "supplier": "NOASSERTION",
             "downloadLocation": next((u for t, u, _ in c["ext"] if t == "distribution"), REPO_URL + "/releases" if c is prod else "NOASSERTION"),
             "filesAnalyzed": False, "licenseConcluded": "NOASSERTION", "licenseDeclared": licence, "copyrightText": "NOASSERTION",
             "primaryPackagePurpose": PURPOSE.get(c["type"], "OTHER")}
        if c["hashes"]:
            p["checksums"] = [{"algorithm": a.replace("-", ""), "checksumValue": h} for a, h in c["hashes"]]
        hp = next((u for t, u, _ in c["ext"] if t in ("website", "vcs")), None)
        if hp:
            p["homepage"] = hp
        if c["purl"]:
            p["externalRefs"] = [{"referenceCategory": "PACKAGE-MANAGER", "referenceType": "purl", "referenceLocator": c["purl"]}]
        notes = ["%s=%s" % (k.replace("synofurb:", ""), v) for k, v in sorted(c["props"].items()) if k in
                 ("synofurb:dependency-kind", "synofurb:platform-conditional", "synofurb:version-unresolved", "synofurb:licence-verified", "synofurb:proc-macro")]
        if not c["licence"]:
            notes.append("licence-unknown=true")
        if notes:
            p["comment"] = "; ".join(notes)
        if c["description"]:
            p["description"] = c["description"]
        pkgs.append(p)
    pkgs.sort(key=lambda p: (p["name"], p["versionInfo"], p["SPDXID"]))
    rels = [{"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES", "relatedSpdxElement": ids[prod["ref"]]}]
    by_ref = {c["ref"]: c for c in comps}
    for a in sorted(model["edges"]):
        for b in sorted(set(model["edges"][a])):
            kind = by_ref[b]["props"].get("synofurb:dependency-kind", "normal")
            if kind in ("build", "build-tool"):
                rels.append({"spdxElementId": ids[b], "relationshipType": "BUILD_DEPENDENCY_OF", "relatedSpdxElement": ids[a]})
            elif kind == "dev":
                rels.append({"spdxElementId": ids[b], "relationshipType": "DEV_DEPENDENCY_OF", "relatedSpdxElement": ids[a]})
            else:
                rels.append({"spdxElementId": ids[a], "relationshipType": "DEPENDS_ON", "relatedSpdxElement": ids[b]})
    rels.sort(key=lambda r: (r["spdxElementId"], r["relationshipType"], r["relatedSpdxElement"]))
    name = "synofurb-%s" % prod["version"]
    doc = {"spdxVersion": "SPDX-2.3", "dataLicense": "CC0-1.0", "SPDXID": "SPDXRef-DOCUMENT", "name": name,
           "creationInfo": {"created": timestamp or SPDX_PLACEHOLDER_TIME, "creators": ["Tool: %s-%s" % (SCRIPT_NAME, SCRIPT_VERSION)],
                            "comment": "Engineering SBOM, not a conformity claim and not legal advice. " +
                                       ("" if timestamp else "No --timestamp was given: 'created' is the placeholder 1970-01-01T00:00:00Z (SPDX requires the field).")},
           "documentDescribes": [ids[prod["ref"]]], "packages": pkgs, "relationships": rels}
    if extracted:
        doc["hasExtractedLicensingInfos"] = [{"licenseId": k, "extractedText": v, "name": k[len("LicenseRef-"):]} for k, v in sorted(extracted.items())]
    ns = "https://github.com/andreaswiren/synofurb/spdx/%s-%s" % (name, uuid.uuid5(NAMESPACE, json.dumps(doc, sort_keys=True)))
    out = {k: doc[k] for k in ("spdxVersion", "dataLicense", "SPDXID", "name")}
    out["documentNamespace"] = ns
    out.update({k: v for k, v in doc.items() if k not in out})
    return out


# ----------------------------------------------------------------------------------------------------------------------
# report
# ----------------------------------------------------------------------------------------------------------------------
def make_report(model):
    comps = model["components"]
    meta, pkg_by_id, cls = model["meta"], model["pkg_by_id"], model["cls"]
    ref_to_id = {v: k for k, v in model["id_to_ref"].items()}
    rep = {"product": "synofurb", "version": model["product"]["version"], "generator": "%s %s" % (SCRIPT_NAME, SCRIPT_VERSION)}
    cargo = [c for c in comps if c["source_kind"] == "cargo"]
    kinds = {}
    for c in cargo:
        kinds[c["props"]["synofurb:dependency-kind"]] = kinds.get(c["props"]["synofurb:dependency-kind"], 0) + 1
    rep["counts"] = {"components_total": len(comps), "cargo_packages": len(cargo), "cargo_by_kind": dict(sorted(kinds.items())),
                     "cargo_platform_conditional": sum(1 for c in cargo if c["props"].get("synofurb:platform-conditional") == "true"),
                     "non_cargo_components": len(comps) - len(cargo), "workspace_members": len(meta["workspace_members"])}
    rep["licence_unknown"] = sorted(({"ref": c["ref"], "name": c["name"], "version": c["version"], "declared": c["licence_name"]} for c in comps if not c["licence"]), key=lambda x: (x["name"], x["version"]))
    rep["licence_declared_by_generator_unverified"] = sorted(({"name": c["name"], "version": c["version"], "licence": c["licence"]} for c in comps if c["props"].get("synofurb:licence-verified") == "false" and c["licence"]), key=lambda x: (x["name"], x["version"]))
    unrec = {}
    for c in comps:
        if c["licence"]:
            for i, exc in expression_ids(parse_expression(c["licence"])):
                if i not in KNOWN_IDS and not i.startswith("LicenseRef-"):
                    unrec.setdefault(i, []).append("%s %s" % (c["name"], c["version"]))
                if exc and exc not in KNOWN_EXCEPTIONS:
                    unrec.setdefault("exception:" + exc, []).append("%s %s" % (c["name"], c["version"]))
    rep["unrecognised_spdx_ids"] = {k: sorted(v) for k, v in sorted(unrec.items())}
    rep["unresolved_versions"] = sorted(model["unresolved"], key=lambda x: x["name"])
    hist = {}
    for c in comps:
        key = c["licence"] or "(unknown)"
        hist[key] = hist.get(key, 0) + 1
    rep["licence_expressions"] = dict(sorted(hist.items(), key=lambda kv: (-kv[1], kv[0])))
    copyleft = []
    for c in comps:
        if not c["licence"]:
            continue
        tree = parse_expression(c["licence"])
        if not mentions_copyleft(tree):
            continue
        fams = sorted({copyleft_family(i)[0] for i, _ in expression_ids(tree) if copyleft_family(i)})
        text = " ".join(sorted({copyleft_family(i)[1] for i, _ in expression_ids(tree) if copyleft_family(i)}))
        entry = {"name": c["name"], "version": c["version"], "licence": c["licence"], "families": fams,
                 "kind": c["props"].get("synofurb:dependency-kind", "extra"),
                 "unavoidable": copyleft_applies(tree), "obligation": text}
        if c["source_kind"] == "cargo":
            chain, via = dependency_chain(meta, c["cargo_id"], pkg_by_id)
            entry["chain"], entry["chain_via"] = chain, via
            if c["props"].get("synofurb:platform-conditional") == "true":
                entry["platform_conditional"] = True
        else:
            entry["chain"] = ["synofurb", "%s %s" % (c["name"], c["version"])]
            entry["chain_via"] = "extra component"
        copyleft.append(entry)
    rep["copyleft"] = sorted(copyleft, key=lambda e: (not e["unavoidable"], e["name"], e["version"]))
    rep["notes"] = list(model["notes"])
    return rep


def print_summary(rep, out=None):
    out = out or sys.stdout
    c = rep["counts"]
    out.write("components: %d total = %d cargo (%s; %d platform-conditional) + %d other\n" % (
        c["components_total"], c["cargo_packages"], ", ".join("%s %d" % kv for kv in c["cargo_by_kind"].items()), c["cargo_platform_conditional"], c["non_cargo_components"]))
    out.write("licence unknown: %d\n" % len(rep["licence_unknown"]))
    for u in rep["licence_unknown"]:
        out.write("  - %s %s\n" % (u["name"], u["version"]))
    out.write("licence declared by this script, not verified: %d\n" % len(rep["licence_declared_by_generator_unverified"]))
    out.write("unrecognised SPDX ids: %s\n" % (", ".join(rep["unrecognised_spdx_ids"]) or "none"))
    out.write("unresolved versions: %d\n" % len(rep["unresolved_versions"]))
    out.write("copyleft mentioned in %d component(s):\n" % len(rep["copyleft"]))
    for e in rep["copyleft"]:
        out.write("  - %s %s  [%s, %s%s]  %s  via %s%s\n" % (e["name"], e["version"], e["kind"], "APPLIES" if e["unavoidable"] else "avoidable (OR)",
                  ", only on some platforms" if e.get("platform_conditional") else "", e["licence"], " > ".join(e["chain"]),
                  " (build/dev edges)" if e.get("chain_via") == "build/dev" else ""))
    for n in rep["notes"]:
        out.write("note: %s\n" % n)


# ----------------------------------------------------------------------------------------------------------------------
# validation ("schema-lite")
# ----------------------------------------------------------------------------------------------------------------------
def check_cyclonedx(doc) -> tuple:
    """-> (errors, warnings)"""
    err, warn = [], []

    def need(cond, msg):
        if not cond:
            err.append(msg)

    need(doc.get("bomFormat") == "CycloneDX", "bomFormat must be CycloneDX")
    need(doc.get("specVersion") == "1.5", "specVersion must be 1.5")
    need(isinstance(doc.get("version"), int) and doc.get("version", 0) >= 1, "version must be an integer >= 1")
    need(bool(re.fullmatch(r"urn:uuid:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", str(doc.get("serialNumber", "")))), "serialNumber must be urn:uuid:<uuid>")
    md = doc.get("metadata") or {}
    mc = md.get("component") or {}
    for k in ("name", "version", "type", "bom-ref"):
        need(mc.get(k), "metadata.component.%s is required" % k)
    if md.get("timestamp") is not None:
        need(bool(re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?(Z|[+-]\d\d:\d\d)", str(md["timestamp"]))), "metadata.timestamp is not RFC 3339")
    refs, seen = set(), set()
    if mc.get("bom-ref"):
        refs.add(mc["bom-ref"])
        seen.add(mc["bom-ref"])
    comps = doc.get("components")
    need(isinstance(comps, list) and comps, "components must be a non-empty list")

    def walk(cs):
        for c in cs or []:
            yield c
            yield from walk(c.get("components"))

    unknown_ok = 0
    for c in walk(comps if isinstance(comps, list) else []):
        label = "%s@%s" % (c.get("name"), c.get("version"))
        need(c.get("type") in CDX_TYPES, "%s: invalid type %r" % (label, c.get("type")))
        need(bool(c.get("name")), "component without name: %r" % (c.get("bom-ref"),))
        need(bool(c.get("version")), "%s: component without version" % label)
        r = c.get("bom-ref")
        need(bool(r), "%s: bom-ref missing" % label)
        if r:
            need(r not in seen, "duplicate bom-ref %s" % r)
            seen.add(r)
            refs.add(r)
        if "scope" in c:
            need(c["scope"] in CDX_SCOPES, "%s: invalid scope %r" % (label, c["scope"]))
        if "purl" in c:
            need(str(c["purl"]).startswith("pkg:"), "%s: purl must start with pkg:" % label)
        for h in c.get("hashes", []):
            ln = HASH_LEN.get(h.get("alg"))
            need(ln is not None and re.fullmatch(r"[0-9a-f]{%d}" % ln, str(h.get("content", ""))) is not None, "%s: bad hash %r" % (label, h))
        lic = c.get("licenses")
        marked = any(p.get("name") == "synofurb:licence-unknown" and p.get("value") == "true" for p in c.get("properties", []))
        if lic:
            ok = all(("expression" in x and isinstance(x["expression"], str)) or ("license" in x and (x["license"].get("id") or x["license"].get("name"))) for x in lic)
            need(ok, "%s: malformed licenses" % label)
            if "synofurb:licence-unknown" in [p.get("name") for p in c.get("properties", [])] and not marked:
                err.append("%s: licence-unknown marker has a value other than true" % label)
            for x in lic:
                if "expression" in x:
                    try:
                        parse_expression(x["expression"])
                    except SpdxError as e:
                        err.append("%s: licence expression %r is not valid SPDX syntax (%s)" % (label, x["expression"], e))
        if not lic or marked:
            if marked:
                unknown_ok += 1
            else:
                err.append("%s: no licence and not listed as licence-unknown" % label)
    deps = doc.get("dependencies")
    need(isinstance(deps, list), "dependencies must be a list")
    seen_dep = set()
    for d in deps or []:
        r = d.get("ref")
        need(r in refs, "dependency ref %r is not a component" % r)
        need(r not in seen_dep, "duplicate dependency entry for %r" % r)
        seen_dep.add(r)
        for t in d.get("dependsOn", []):
            need(t in refs, "%r dependsOn unknown ref %r" % (r, t))
    if mc.get("bom-ref"):
        need(mc["bom-ref"] in seen_dep, "metadata.component has no dependencies entry")
    if unknown_ok:
        warn.append("%d component(s) are listed as licence-unknown" % unknown_ok)
    n_unres = sum(1 for c in walk(comps if isinstance(comps, list) else []) if any(p.get("name") == "synofurb:version-unresolved" for p in c.get("properties", [])))
    if n_unres:
        warn.append("%d component(s) have an UNRESOLVED version" % n_unres)
    return err, warn


def check_spdx(doc) -> tuple:
    err, warn = [], []

    def need(cond, msg):
        if not cond:
            err.append(msg)

    need(doc.get("spdxVersion") == "SPDX-2.3", "spdxVersion must be SPDX-2.3")
    need(doc.get("dataLicense") == "CC0-1.0", "dataLicense must be CC0-1.0")
    need(doc.get("SPDXID") == "SPDXRef-DOCUMENT", "SPDXID must be SPDXRef-DOCUMENT")
    for k in ("name", "documentNamespace"):
        need(doc.get(k), "%s is required" % k)
    ci = doc.get("creationInfo") or {}
    need(bool(re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", str(ci.get("created", "")))), "creationInfo.created must be YYYY-MM-DDThh:mm:ssZ")
    need(bool(ci.get("creators")), "creationInfo.creators is required")
    ids = set()
    for p in doc.get("packages", []):
        label = "%s@%s" % (p.get("name"), p.get("versionInfo"))
        i = p.get("SPDXID", "")
        need(bool(re.fullmatch(r"SPDXRef-[A-Za-z0-9.\-]+", i)), "%s: bad SPDXID %r" % (label, i))
        need(i not in ids, "duplicate SPDXID %s" % i)
        ids.add(i)
        for k in ("name", "downloadLocation", "filesAnalyzed", "licenseConcluded", "licenseDeclared", "copyrightText"):
            need(k in p, "%s: %s is required" % (label, k))
        need(bool(p.get("versionInfo")), "%s: versionInfo missing" % label)
        if p.get("licenseDeclared") == "NOASSERTION":
            warn.append("%s: licenseDeclared NOASSERTION" % label)
        for cs in p.get("checksums", []):
            need(cs.get("algorithm") in ("SHA256", "SHA1", "SHA512", "MD5") and bool(re.fullmatch(r"[0-9a-f]{32,128}", cs.get("checksumValue", ""))), "%s: bad checksum" % label)
    need(bool(ids), "no packages")
    extracted = {x.get("licenseId") for x in doc.get("hasExtractedLicensingInfos", [])}
    for p in doc.get("packages", []):
        for ref in re.findall(r"LicenseRef-[A-Za-z0-9.\-]+", p.get("licenseDeclared", "")):
            need(ref in extracted, "%s: %s is not declared in hasExtractedLicensingInfos" % (p.get("name"), ref))
    for d in doc.get("documentDescribes", []):
        need(d in ids, "documentDescribes unknown %s" % d)
    need(bool(doc.get("documentDescribes")) or any(r.get("relationshipType") == "DESCRIBES" for r in doc.get("relationships", [])), "nothing is described")
    for r in doc.get("relationships", []):
        for k in ("spdxElementId", "relatedSpdxElement"):
            need(r.get(k) in ids or r.get(k) == "SPDXRef-DOCUMENT", "relationship refers to unknown %s" % r.get(k))
    return err, warn


def check_file(path):
    try:
        with open(path, encoding="utf-8") as fh:
            doc = json.load(fh)
    except (OSError, ValueError) as e:
        return ["cannot read %s: %s" % (path, e)], []
    if doc.get("bomFormat") == "CycloneDX":
        return check_cyclonedx(doc)
    if str(doc.get("spdxVersion", "")).startswith("SPDX-"):
        return check_spdx(doc)
    return ["%s: neither CycloneDX nor SPDX" % path], []


# ----------------------------------------------------------------------------------------------------------------------
def load_json(path):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def dump(path, doc):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(doc, fh, indent=2, ensure_ascii=True)
        fh.write("\n")


def main(argv=None) -> int:
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description="Generate or check the synofurb SBOM (CycloneDX 1.5 + SPDX 2.3).")
    ap.add_argument("--repo", default=os.path.dirname(here), help="repository root (default: the parent of scripts/)")
    ap.add_argument("--out", default=None, help="output directory (default dist/sbom under --repo)")
    ap.add_argument("--timestamp", help="RFC 3339 time to record (default: SOURCE_DATE_EPOCH if set, otherwise no time)")
    ap.add_argument("--commit", help="git commit id to record (CI: $GITHUB_SHA)")
    ap.add_argument("--metadata", help="use this `cargo metadata --format-version 1` JSON instead of running cargo")
    ap.add_argument("--cargo-lock", help="Cargo.lock to read checksums from (default: <repo>/Cargo.lock)")
    ap.add_argument("--offline", action="store_true", help="pass --offline to cargo metadata")
    ap.add_argument("--agent-manifest", action="append", help="boot/agent MANIFEST-<arch>.json file or a directory searched recursively (repeatable)")
    ap.add_argument("--require-agent-manifest", action="store_true", help="fail when the boot-agent package versions are not pinned by a manifest")
    ap.add_argument("--strict", action="store_true", help="also fail on unknown licences, unrecognised SPDX ids and unresolved versions")
    ap.add_argument("--format", choices=("both", "cyclonedx", "spdx"), default="both")
    ap.add_argument("--check", nargs="+", metavar="FILE", help="validate existing SBOM file(s) and exit")
    ap.add_argument("--quiet", action="store_true")
    a = ap.parse_args(argv)

    if a.check:
        bad = 0
        for f in a.check:
            e, w = check_file(f)
            for m in e:
                print("%s: ERROR: %s" % (f, m), file=sys.stderr)
            if not a.quiet:
                for m in w[:20]:
                    print("%s: warning: %s" % (f, m), file=sys.stderr)
                if len(w) > 20:
                    print("%s: ... %d more warnings" % (f, len(w) - 20), file=sys.stderr)
            print("%s: %s (%d error(s), %d warning(s))" % (f, "INVALID" if e else "valid", len(e), len(w)))
            bad += bool(e)
        return 1 if bad else 0

    repo = os.path.abspath(a.repo)
    ts = parse_timestamp(a.timestamp)
    meta = load_json(a.metadata) if a.metadata else run_cargo_metadata(repo, a.offline)
    lock_path = a.cargo_lock or os.path.join(repo, "Cargo.lock")
    lock_text = read_text(lock_path)
    if lock_text is None:
        print("warning: %s not found; no checksums" % lock_path, file=sys.stderr)
    model = build_model(repo, meta, lock_text, find_manifests(a.agent_manifest, repo), a.require_agent_manifest, a.commit)
    rep = make_report(model)
    ver = model["product"]["version"]
    out = os.path.abspath(a.out or os.path.join(repo, "dist", "sbom"))
    problems = [n for n in model["notes"] if n.startswith("ERROR")]
    written = []
    if a.format in ("both", "cyclonedx"):
        doc = make_cyclonedx(model, ts)
        e, _w = check_cyclonedx(doc)
        problems += ["CycloneDX: " + m for m in e]
        p = os.path.join(out, "synofurb-%s-sbom.cdx.json" % ver)
        dump(p, doc)
        written.append(p)
    if a.format in ("both", "spdx"):
        doc = make_spdx(model, ts)
        e, _w = check_spdx(doc)
        problems += ["SPDX: " + m for m in e]
        p = os.path.join(out, "synofurb-%s-sbom.spdx.json" % ver)
        dump(p, doc)
        written.append(p)
    p = os.path.join(out, "synofurb-%s-sbom-report.json" % ver)
    dump(p, rep)
    written.append(p)
    if a.strict:
        if rep["licence_unknown"]:
            problems.append("--strict: %d component(s) with unknown licence" % len(rep["licence_unknown"]))
        if rep["unrecognised_spdx_ids"]:
            problems.append("--strict: unrecognised SPDX ids: %s" % ", ".join(rep["unrecognised_spdx_ids"]))
        if rep["unresolved_versions"]:
            problems.append("--strict: %d component(s) with unresolved versions" % len(rep["unresolved_versions"]))
    if not a.quiet:
        print_summary(rep)
        for w in written:
            print("wrote", w)
    for m in problems:
        print("ERROR: " + m, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
