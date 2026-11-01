#!/usr/bin/env python3
"""Builds a fake GitHub release tree for the installer tests.

  <www>/api/releases/latest                 release JSON (tag_name, assets[].browser_download_url)
  <www>/api/releases/tags/v<ver>            same
  <www>/api/releases                        list with that one release
  <www>/dl/v<ver>/<assets>, SHA256SUMS, SHA256SUMS.minisig
"""
import argparse
import hashlib
import io
import json
import shutil
import subprocess
import tarfile
import zipfile
from pathlib import Path

FAKE_BIN = """#!/bin/sh
# fake synofurb used by the installer tests
printf '%s\\n' "$*" >> "${SYNOFURB_TEST_LOG:-/dev/null}"
case "${1:-}" in --version) echo "synofurb @VER@" ;; esac
exit "${SYNOFURB_TEST_EXIT:-0}"
"""
FAKE_CLI_BIN = """#!/bin/sh
# fake synofurb-cli used by the installer tests
printf '%s\\n' "$*" >> "${SYNOFURB_TEST_LOG:-/dev/null}"
case "${1:-}" in --version) echo "synofurb-cli @VER@" ;; esac
exit "${SYNOFURB_TEST_EXIT:-0}"
"""
PNG = bytes.fromhex("89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c6360000002000001e221bc330000000049454e44ae426082")


def add(tf: tarfile.TarFile, name: str, data: bytes, mode: int = 0o644, typ=tarfile.REGTYPE, linkname: str = "") -> None:
    ti = tarfile.TarInfo(name)
    ti.size = len(data) if typ == tarfile.REGTYPE else 0
    ti.mode = mode
    ti.mtime = 0
    ti.type = typ
    ti.linkname = linkname
    tf.addfile(ti, io.BytesIO(data) if typ == tarfile.REGTYPE else None)


def desktop_tar(path: Path, version: str, mutate: str) -> None:
    with tarfile.open(path, "w:gz") as tf:
        if mutate != "no-binary":
            add(tf, "synofurb", FAKE_BIN.replace("@VER@", version).encode(), 0o755)
        add(tf, "synofurb.png", PNG)
        add(tf, "LICENSE", b"MIT\n")
        if mutate == "evil-tar":
            add(tf, "../evil.txt", b"pwned")
        if mutate == "link-tar":
            add(tf, "sneaky", b"", typ=tarfile.SYMTYPE, linkname="/etc/passwd")


def cli_tar(path: Path, version: str, mutate: str) -> None:
    """The headless build: a flat archive with `synofurb-cli`, LICENSE and README.txt (no icon, no desktop files)."""
    with tarfile.open(path, "w:gz") as tf:
        if mutate != "cli-no-binary":
            add(tf, "synofurb-cli", FAKE_CLI_BIN.replace("@VER@", version).encode(), 0o755)
        add(tf, "LICENSE", b"MIT\n")
        add(tf, "README.txt", b"synofurb-cli\n")
        if mutate == "cli-evil-tar":
            add(tf, "../evil-cli.txt", b"pwned")
        if mutate == "cli-link-tar":
            add(tf, "sneaky", b"", typ=tarfile.SYMTYPE, linkname="/etc/passwd")


def mgmt_tar(path: Path, version: str, mutate: str) -> None:
    with tarfile.open(path, "w:gz") as tf:
        add(tf, "synofurb_mgmt.py", b"#!/usr/bin/env python3\nprint('fake mgmt')\n", 0o755)
        add(tf, "synofurb-mgmt.service", b"[Unit]\nDescription=fake\n[Service]\nExecStart=/usr/bin/python3 /opt/synofurb-mgmt/synofurb_mgmt.py\n[Install]\nWantedBy=multi-user.target\n")
        add(tf, "README.md", b"fake\n")
        if mutate != "mgmt-no-version":
            add(tf, "VERSION", (version if mutate != "mgmt-wrong-version" else "9.9.9").encode() + b"\n")


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--www", required=True)
    ap.add_argument("--version", default="0.2.0")
    ap.add_argument("--platforms", default="linux-x86_64")
    ap.add_argument("--mgmt-version", default="0.1.0")
    ap.add_argument("--seckey", help="minisign secret key (unencrypted); omit for no signature")
    ap.add_argument("--base", required=True, help="http://127.0.0.1:PORT")
    ap.add_argument("--mutate", default="none")
    ap.add_argument("--tamper-ext", default="msi", help="with --mutate asset on windows platforms: msi or zip")
    a = ap.parse_args()

    www = Path(a.www)
    if www.exists():
        shutil.rmtree(www)
    tag = f"v{a.version}"
    dl = www / "dl" / tag
    dl.mkdir(parents=True)
    (www / "api" / "releases" / "tags").mkdir(parents=True)

    names = []
    for plat in filter(None, a.platforms.split(",")):
        if plat.startswith("windows"):
            msi = f"synofurb-{a.version}-{plat}.msi"
            (dl / msi).write_bytes(b"not a real msi")
            zname = f"synofurb-{a.version}-{plat}.zip"
            with zipfile.ZipFile(dl / zname, "w") as zf:
                if a.mutate != "no-binary":
                    zf.writestr("synofurb.exe", "MZ fake exe " + a.version)
                zf.writestr("LICENSE", "MIT\n")
                if a.mutate == "zip-slip":
                    zf.writestr("../evil.txt", "pwned")
            names += [msi, zname]
            cz = f"synofurb-cli-{a.version}-{plat}.zip"
            with zipfile.ZipFile(dl / cz, "w") as zf:
                if a.mutate != "cli-no-binary":
                    zf.writestr("synofurb-cli.exe", "MZ fake cli exe " + a.version)
                zf.writestr("LICENSE", "MIT\n")
                if a.mutate == "cli-zip-slip":
                    zf.writestr("../evil-cli.txt", "pwned")
            names.append(cz)
            continue
        name = f"synofurb-{a.version}-{plat}.tar.gz"
        desktop_tar(dl / name, a.version, a.mutate)
        names.append(name)
        cname = f"synofurb-cli-{a.version}-{plat}.tar.gz"
        cli_tar(dl / cname, a.version, a.mutate)
        names.append(cname)
        if plat.startswith("linux"):
            ai = f"synofurb-{a.version}-{plat}.AppImage"
            (dl / ai).write_bytes(FAKE_BIN.replace("@VER@", a.version).encode())
            names.append(ai)
        if plat.startswith("macos"):
            (dl / f"synofurb-{a.version}-{plat}.dmg").write_bytes(b"not a real dmg")
            names.append(f"synofurb-{a.version}-{plat}.dmg")
    mgmt = f"synofurb-mgmt-{a.mgmt_version}.tar.gz"
    mgmt_tar(dl / mgmt, a.mgmt_version, a.mutate)
    names.append(mgmt)

    sums = "".join(f"{sha(dl / n)}  {n}\n" for n in sorted(names) if not (a.mutate == "unlisted" and n.endswith(".tar.gz") and n.startswith("synofurb-0")))
    (dl / "SHA256SUMS").write_text(sums)
    if a.seckey:
        subprocess.run(["minisign", "-S", "-s", a.seckey, "-m", str(dl / "SHA256SUMS"), "-x", str(dl / "SHA256SUMS.minisig")],
                       check=True, capture_output=True, input=b"\n")
    if a.mutate == "asset":  # tamper with the asset after the sums were computed and signed
        first = a.platforms.split(",")[0]
        p = dl / f"synofurb-{a.version}-{first}.{'zip' if first.startswith('windows') and a.tamper_ext == 'zip' else 'msi' if first.startswith('windows') else 'tar.gz'}"
        p.write_bytes(p.read_bytes() + b"\0tampered")
    if a.mutate == "cli-asset":  # tamper with the CLI archive after the sums were computed and signed
        first = a.platforms.split(",")[0]
        p = dl / f"synofurb-cli-{a.version}-{first}.{'zip' if first.startswith('windows') else 'tar.gz'}"
        p.write_bytes(p.read_bytes() + b"\0tampered")
    if a.mutate == "cli-unlisted":  # the CLI archive exists but SHA256SUMS does not list it
        sums = "".join(l for l in sums.splitlines(keepends=True) if "synofurb-cli-" not in l)
        (dl / "SHA256SUMS").write_text(sums)
        if a.seckey:
            subprocess.run(["minisign", "-S", "-s", a.seckey, "-m", str(dl / "SHA256SUMS"), "-x", str(dl / "SHA256SUMS.minisig")],
                           check=True, capture_output=True, input=b"\n")
    if a.mutate == "sums-after-sign":  # valid-looking sums, but the signature no longer matches
        (dl / "SHA256SUMS").write_text(sums + f"{'0' * 64}  something-else.zip\n")

    shown_tag = "v1.0; touch /tmp/pwned" if a.mutate == "bad-tag" else tag
    release = {
        "tag_name": shown_tag,
        "name": f"synofurb {a.version}",
        "draft": False,
        "prerelease": False,
        "assets": [{"name": n, "browser_download_url": f"{a.base}/dl/{tag}/{n}"} for n in names + ["SHA256SUMS"]],
    }
    text = json.dumps(release, indent=2)
    (www / "api" / "releases" / "latest").write_text(text)
    (www / "api" / "releases" / "tags" / tag).write_text(text)
    # `GET /api/releases?per_page=100` is served from the directory index file
    (www / "api" / "releases" / "index.html").write_text(json.dumps([release], indent=2))


if __name__ == "__main__":
    main()
