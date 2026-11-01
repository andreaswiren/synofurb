#!/usr/bin/env python3
"""Generate compat/models.json from compat/source/synology-cpu-table.csv.

The CSV holds the *hardware facts* (copied from Synology's own table, see its header). This script derives the
fields synofurb needs (arch, boot method, vendor, default status, notes) by fixed rules and writes the JSON that the
app embeds (crates/synofurb-core/src/compat.rs, `include_str!`).

models.json is hand-curated too: contributors add `evidence` and change `status` there. Re-running this script
therefore PRESERVES `status` and `evidence` (and `notes` when evidence exists) of models already present in the JSON
and only refreshes hardware fields. Models that exist in the JSON but not in the CSV are an error.

Usage:
    python3 scripts/gen-compat.py            # rewrite compat/models.json
    python3 scripts/gen-compat.py --check    # exit 1 if models.json is not what the script would write

Standard library only.
"""

import csv
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CSV_PATH = ROOT / "compat" / "source" / "synology-cpu-table.csv"
JSON_PATH = ROOT / "compat" / "models.json"

SCHEMA_VERSION = 1

# ---------------------------------------------------------------------------------------------------------------
# Platform rules. Keys are lower-cased Synology package-arch / platform tokens.
#   arch      Debian architecture (or ppc/armel/i386 for dead ends)
#   boot      how the unit boots DSM (dom = internal flash/DOM module, uboot = Synology U-Boot, unknown)
#   vendor    fallback SoC vendor when the CSV has no CPU name (secondary-source rows)
#   kind      x86 | arm-doc (documented Debian path) | arm-exp | dead
# ---------------------------------------------------------------------------------------------------------------
X86_NOTE = (
    "x86-64 board; the standard Debian path (kexec or installer kit, GRUB) is expected to work in theory. "
    "No synofurb test report yet."
)
X86_LEGACY_NOTE = (
    "Older x86-64 board, almost certainly legacy BIOS boot; a serial console is strongly recommended. "
    "Expected to work in theory; no synofurb test report yet."
)
X86_NK_NOTE = (
    "Newer-kernel (nk) x86-64 platform. Expected to work in theory; no synofurb test report yet. "
    "Boot method of this platform has not been confirmed by anyone we know of."
)
ARM_DOC_NOTE = (
    "ARMv7 (armhf) with Synology U-Boot and no UEFI. The Debian wiki (InstallingDebianOn/Synology) documents this "
    "method for the DS218j and RS816 (Armada 385): raw-sector boot, changed once over the serial console. "
    "Nobody has reported a run of synofurb on this unit; a serial console is mandatory. See docs/ARM.md."
)
ARM_EXP_NOTE = (
    "ARM board with Synology U-Boot and no UEFI. No documented Debian path for this SoC; treat as experimental. "
    "A serial console is mandatory. See docs/ARM.md."
)

P = {
    # --- Intel x86-64 -----------------------------------------------------------------------------------------
    "apollolake": dict(arch="amd64", boot="dom", vendor="Intel", kind="x86", note=X86_NOTE),
    "geminilake": dict(arch="amd64", boot="dom", vendor="Intel", kind="x86", note=X86_NOTE),
    "geminilakenk": dict(arch="amd64", boot="unknown", vendor="Intel", kind="x86", note=X86_NK_NOTE),
    "denverton": dict(arch="amd64", boot="dom", vendor="Intel", kind="x86", note=X86_NOTE),
    "broadwell": dict(arch="amd64", boot="dom", vendor="Intel", kind="x86", note=X86_NOTE),
    "broadwellnk": dict(arch="amd64", boot="dom", vendor="Intel", kind="x86", note=X86_NOTE),
    "broadwellnkv2": dict(arch="amd64", boot="unknown", vendor="Intel", kind="x86", note=X86_NK_NOTE),
    "broadwellntbap": dict(
        arch="amd64",
        boot="unknown",
        vendor="Intel",
        kind="x86",
        note="Dual-controller (high availability) unit. synofurb installs onto one node only; untested and "
        "probably not what you want. No synofurb test report yet.",
    ),
    "grantley": dict(arch="amd64", boot="unknown", vendor="Intel", kind="x86", note=X86_NOTE),
    "purley": dict(arch="amd64", boot="unknown", vendor="Intel", kind="x86", note=X86_NOTE),
    "icelaked": dict(arch="amd64", boot="unknown", vendor="Intel", kind="x86", note=X86_NK_NOTE),
    "bromolow": dict(arch="amd64", boot="dom", vendor="Intel", kind="x86", note=X86_LEGACY_NOTE),
    "cedarview": dict(arch="amd64", boot="dom", vendor="Intel", kind="x86", note=X86_LEGACY_NOTE),
    "avoton": dict(arch="amd64", boot="dom", vendor="Intel", kind="x86", note=X86_LEGACY_NOTE),
    "braswell": dict(arch="amd64", boot="dom", vendor="Intel", kind="x86", note=X86_LEGACY_NOTE),
    "x86": dict(arch="amd64", boot="dom", vendor="Intel", kind="x86", note=X86_LEGACY_NOTE),
    # --- AMD x86-64 -------------------------------------------------------------------------------------------
    "v1000": dict(arch="amd64", boot="dom", vendor="AMD", kind="x86", note=X86_NOTE),
    "v1000nk": dict(arch="amd64", boot="unknown", vendor="AMD", kind="x86", note=X86_NK_NOTE),
    "r1000": dict(arch="amd64", boot="dom", vendor="AMD", kind="x86", note=X86_NOTE),
    "r1000nk": dict(arch="amd64", boot="unknown", vendor="AMD", kind="x86", note=X86_NK_NOTE),
    "epyc7002": dict(arch="amd64", boot="unknown", vendor="AMD", kind="x86", note=X86_NOTE),
    "epyc7003": dict(arch="amd64", boot="unknown", vendor="AMD", kind="x86", note=X86_NOTE),
    # --- ARM, documented Debian path (ARMv7) ------------------------------------------------------------------
    "armada38x": dict(arch="armhf", boot="uboot", vendor="Marvell", kind="arm-doc", note=ARM_DOC_NOTE),
    "armada370": dict(arch="armhf", boot="uboot", vendor="Marvell", kind="arm-doc", note=ARM_DOC_NOTE),
    "armadaxp": dict(arch="armhf", boot="uboot", vendor="Marvell", kind="arm-doc", note=ARM_DOC_NOTE),
    # --- ARM, no documented path ------------------------------------------------------------------------------
    "rtd1296": dict(arch="arm64", boot="uboot", vendor="Realtek", kind="arm-exp", note=ARM_EXP_NOTE),
    "rtd1619b": dict(arch="arm64", boot="uboot", vendor="Realtek", kind="arm-exp", note=ARM_EXP_NOTE),
    "armada37xx": dict(arch="arm64", boot="uboot", vendor="Marvell", kind="arm-exp", note=ARM_EXP_NOTE),
    "armada375": dict(arch="armhf", boot="uboot", vendor="Marvell", kind="arm-exp", note=ARM_EXP_NOTE),
    "alpine": dict(arch="armhf", boot="uboot", vendor="Annapurna", kind="arm-exp", note=ARM_EXP_NOTE),
    "alpine4k": dict(arch="armhf", boot="uboot", vendor="Annapurna", kind="arm-exp", note=ARM_EXP_NOTE),
    # --- Dead ends --------------------------------------------------------------------------------------------
    "comcerto2k": dict(
        arch="armhf",
        boot="unknown",
        vendor="Mindspeed",
        kind="dead",
        note="Unsupported: no mainline Linux support for the Mindspeed Comcerto 2000 SoC.",
    ),
    "monaco": dict(
        arch="armhf",
        boot="unknown",
        vendor="STM",
        kind="dead",
        note="Unsupported: STiH412 media SoC; not supported by Debian 13.",
    ),
    "hi3535": dict(
        arch="armhf",
        boot="unknown",
        vendor="HiSilicon",
        kind="dead",
        note="Unsupported: no mainline Linux support for the HiSilicon Hi3535 SoC. The CPU architecture is assumed "
        "from the SoC family; it is not listed on Synology's CPU page.",
    ),
    "qoriq": dict(
        arch="ppc",
        boot="unknown",
        vendor="Freescale",
        kind="dead",
        note="Unsupported: 32-bit PowerPC (Freescale QorIQ). Debian 13 has no 32-bit PowerPC port.",
    ),
    "ppc853x": dict(
        arch="ppc",
        boot="unknown",
        vendor="Freescale",
        kind="dead",
        note="Unsupported: 32-bit PowerPC (Freescale MPC85xx). Debian 13 has no 32-bit PowerPC port.",
    ),
    "ppc854x": dict(
        arch="ppc",
        boot="unknown",
        vendor="Freescale",
        kind="dead",
        note="Unsupported: 32-bit PowerPC (Freescale MPC85xx). Debian 13 has no 32-bit PowerPC port.",
    ),
    "ppc824x": dict(
        arch="ppc",
        boot="unknown",
        vendor="Freescale",
        kind="dead",
        note="Unsupported: 32-bit PowerPC (Freescale MPC824x). Debian 13 has no 32-bit PowerPC port.",
    ),
    "powerpc": dict(
        arch="ppc",
        boot="unknown",
        vendor="unknown",
        kind="dead",
        note="Unsupported: PowerPC. Debian 13 has no 32-bit PowerPC port.",
    ),
    "evansport": dict(
        arch="i386",
        boot="unknown",
        vendor="Intel",
        kind="dead",
        note="Unsupported: 32-bit x86 (Intel Atom CE5335). Debian 13 no longer ships a 32-bit x86 installer.",
    ),
    "88f628x": dict(
        arch="armel",
        boot="uboot",
        vendor="Marvell",
        kind="dead",
        note="Unsupported: ARMv5 (armel) single-core Marvell Kirkwood. Not supported by synofurb.",
    ),
    "88f6281": dict(
        arch="armel",
        boot="uboot",
        vendor="Marvell",
        kind="dead",
        note="Unsupported: ARMv5 (armel) single-core Marvell Kirkwood. Not supported by synofurb.",
    ),
    "88f6282": dict(
        arch="armel",
        boot="uboot",
        vendor="Marvell",
        kind="dead",
        note="Unsupported: ARMv5 (armel) single-core Marvell Kirkwood. Not supported by synofurb.",
    ),
    "88f5281": dict(
        arch="armel",
        boot="uboot",
        vendor="Marvell",
        kind="dead",
        note="Unsupported: ARMv5 (armel) Marvell Orion. Not supported by synofurb.",
    ),
}

# Package-arch strings on the Synology page -> Debian arch, used to cross-check the platform table.
CPU_ARCH_TO_DEBIAN = {
    "x86_64": "amd64",
    "armv8": "arm64",
    "armv7": "armhf",
    "armv5": "armel",
    "ppc": "ppc",
    "powerpc": "ppc",
    "i686": "i386",
}

# Mainline Debian kernels ship a DTB for exactly these Synology boards (see platform::mainline_dtb).
MAINLINE_DTB = {
    "DS213j": "armada-370-synology-ds213j.dtb",
    "DS116": "armada-385-synology-ds116.dtb",
    "DS414": "armada-xp-synology-ds414.dtb",
}

VENDOR_BY_CPU_PREFIX = [
    ("Intel", "Intel"),
    ("AMD", "AMD"),
    ("Realtek", "Realtek"),
    ("Marvell", "Marvell"),
    ("Annapurna", "Annapurna"),
    ("Freescale", "Freescale"),
    ("STM", "STM"),
    ("Mindspeed", "Mindspeed"),
]

# Evidence that exists before the first user report. Only used when a model is not yet in models.json.
SEED_EVIDENCE = {
    "DS923+": dict(
        status="community",
        evidence=[
            {
                "date": None,
                "reporter": "rylos/synodeb project",
                "kind": "third-party",
                "synofurb_version": None,
                "dsm_version": None,
                "result": "success",
                "notes": "Debian booted from the DOM via DOM boot entries, tested by the synodeb project "
                "(not by synofurb)",
                "log_url": None,
            }
        ],
        notes="Community report: Debian booted from the DOM via DOM boot entries, tested by the synodeb project, "
        "not by synofurb. No synofurb run has been reported for this model yet. Standard x86-64 path otherwise.",
    ),
}


def cpu_vendor_class(vendor: str, arch: str) -> str:
    """Filter bucket: intel | amd | arm | other."""
    v = vendor.lower()
    if v == "intel":
        return "intel"
    if v == "amd":
        return "amd"
    if arch in ("arm64", "armhf", "armel"):
        return "arm"
    return "other"


def parse_ram_mb(ram: str):
    """Return stock RAM in MiB when the text names exactly one size, else None."""
    if not ram or " x 2" in ram or "x 2" in ram:
        return None
    sizes = re.findall(r"(\d+)\s*(GB|MB)", ram, flags=re.I)
    if len(sizes) != 1:
        return None
    n, unit = sizes[0]
    return int(n) * (1024 if unit.upper() == "GB" else 1)


def series_of(model: str) -> str:
    m = re.match(r"^([A-Za-z]+)", model)
    return m.group(1).upper() if m else "?"


def year_of(group: str):
    m = re.match(r"^x(\d\d) Series$", group or "")
    return 2000 + int(m.group(1)) if m else None


def to_int(s):
    s = (s or "").strip()
    return int(s) if s.isdigit() else None


def read_csv():
    fetched = None
    lines = []
    with open(CSV_PATH, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#"):
                m = re.search(r"Fetched:\s+(\d{4}-\d{2}-\d{2})", line)
                if m and fetched is None:
                    fetched = m.group(1)
                continue
            lines.append(line)
    return fetched, list(csv.DictReader(lines))


def build_entry(row):
    plat = row["package_arch"].strip().lower()
    if plat not in P:
        sys.exit(f"gen-compat: no platform rule for {plat!r} (model {row['model']}); add it to P")
    rule = P[plat]
    cpu = row["cpu"].strip() or None
    if cpu:
        want = CPU_ARCH_TO_DEBIAN.get(row["cpu_arch"].strip().lower())
        if want != rule["arch"]:
            sys.exit(
                f"gen-compat: {row['model']}: CPU arch {row['cpu_arch']!r} contradicts platform rule {plat} "
                f"({rule['arch']})"
            )
    vendor = rule["vendor"]
    if cpu:
        for prefix, v in VENDOR_BY_CPU_PREFIX:
            if cpu.startswith(prefix):
                vendor = v
                break
    model = row["model"].strip()
    status = "unsupported" if rule["kind"] == "dead" else "untested"
    notes = rule["note"]
    if model in MAINLINE_DTB:
        notes += f" Debian ships a device tree for this board ({MAINLINE_DTB[model]})."
    if row["source"] == "spksrc-wiki":
        notes += " Hardware details come from the SynoCommunity wiki only; the CPU is not listed on Synology's CPU page."
    ram = row["ram"].strip() or None
    cpu_note = ""
    if ram and parse_ram_mb(ram) is None:
        cpu_note = f" Stock RAM is given as '{ram}' on Synology's page, so no single figure is recorded."
    entry = {
        "model": model,
        "series": series_of(model),
        "year": year_of(row["group"]),
        "platform": plat,
        "cpu": cpu,
        "vendor": vendor,
        "cpu_vendor": cpu_vendor_class(vendor, rule["arch"]),
        "arch": rule["arch"],
        "boot": rule["boot"],
        "ram": ram,
        "ram_mb_stock": parse_ram_mb(ram or ""),
        "cores": to_int(row["cores"]),
        "threads": to_int(row["threads"]),
        "status": status,
        "evidence": [],
        "notes": (notes + cpu_note).strip(),
        "source": row["source"].strip(),
    }
    return entry


SERIES_ORDER = ["PAS", "FS", "HD", "SA", "RC", "RS", "DS", "DVA", "EDS", "NVR", "CS", "UC"]


def sort_key(e):
    s = e["series"]
    si = SERIES_ORDER.index(s) if s in SERIES_ORDER else len(SERIES_ORDER)
    digits = re.search(r"\d+", e["model"])
    return (si, s, int(digits.group(0)) if digits else 0, e["model"].lower())


def main():
    check = "--check" in sys.argv[1:]
    fetched, rows = read_csv()
    if not fetched:
        sys.exit("gen-compat: CSV header lacks a 'Fetched: YYYY-MM-DD' line")

    existing = {}
    if JSON_PATH.exists():
        old = json.loads(JSON_PATH.read_text(encoding="utf-8"))
        existing = {m["model"]: m for m in old["models"]}

    entries, seen = [], set()
    for row in rows:
        e = build_entry(row)
        if e["model"] in seen:
            sys.exit(f"gen-compat: duplicate model {e['model']}")
        seen.add(e["model"])
        old = existing.get(e["model"])
        if old is not None:
            e["status"] = old["status"]
            e["evidence"] = old.get("evidence", [])
            if old.get("evidence") or old["status"] not in ("untested", "unsupported"):
                e["notes"] = old.get("notes") or e["notes"]
        elif e["model"] in SEED_EVIDENCE:
            seed = SEED_EVIDENCE[e["model"]]
            e["status"] = seed["status"]
            e["evidence"] = seed["evidence"]
            if seed["notes"]:
                e["notes"] = seed["notes"]
            else:
                e["notes"] = (
                    "Community report: Debian booted from the DOM via DOM boot entries (synodeb project). "
                    "Not yet tested with synofurb itself. " + e["notes"]
                )
        entries.append(e)

    missing = sorted(set(existing) - seen)
    if missing:
        sys.exit(f"gen-compat: models in models.json but not in the CSV: {missing}")

    entries.sort(key=sort_key)
    doc = {
        "schema_version": SCHEMA_VERSION,
        "updated": fetched,
        "hardware_source": "https://kb.synology.com/en-global/DSM/tutorial/What_kind_of_CPU_does_my_NAS_have",
        "models": entries,
    }
    text = json.dumps(doc, indent=2, ensure_ascii=False) + "\n"
    if check:
        if not JSON_PATH.exists() or JSON_PATH.read_text(encoding="utf-8") != text:
            print("compat/models.json is out of date; run python3 scripts/gen-compat.py", file=sys.stderr)
            return 1
        print(f"ok: {len(entries)} models")
        return 0
    JSON_PATH.write_text(text, encoding="utf-8")
    print(f"wrote {JSON_PATH.relative_to(ROOT)}: {len(entries)} models")
    return 0


if __name__ == "__main__":
    sys.exit(main())
