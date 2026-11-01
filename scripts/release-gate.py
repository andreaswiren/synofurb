#!/usr/bin/env python3
"""synofurb release gate: no release build runs unless the current design has been approved.

Product rule: two independent, harsh reviewers (the `ux-critic` and the `ui-critic` Claude subagents in
.claude/agents/) must have approved EXACTLY the design that is about to be released.  "Exactly" is enforced by
a content hash (the *design hash*) over every design-relevant file; changing any of those files changes the hash
and invalidates all earlier approvals.

Files that feed the design hash (paths are relative to the repository root, POSIX separators):

  design/mockups/**                          every file (the withdrawn v1 mockups; kept in the hash)
  design/mockups-v2/**                       the v2 desktop mockup and its checks
  design/v2/**                               v2 design system, tokens, UX spec
  design/DESIGN-SYSTEM*                      DESIGN-SYSTEM.md, or a DESIGN-SYSTEM*/ directory (every file in it)
  design/screenshots/**                      every file (if the directory exists)
  apps/mgmt-plane/**/*.html|css|js|svg       standalone web UI files of the management plane (if any)
  apps/mgmt-plane/synofurb_mgmt.py#INDEX_HTML  the single-page UI that is embedded in the Python file, extracted
                                             with `ast` (so backend-only edits do NOT invalidate approvals)
  crates/synofurb/src/ui/**                  GPUI UI sources (if present)
  crates/synofurb/src/gui.rs, gui/**         legacy/transition GUI sources (if present)

Excluded everywhere: __pycache__, *.pyc, .DS_Store, Thumbs.db, and design/approvals/** itself.
Text files (no NUL byte in the first 8 KiB) are hashed with CRLF normalised to LF so Windows (autocrlf)
and Linux checkouts produce the same hash.

design_hash = sha256 over the UTF-8 listing  "<sha256-of-file>  <path>\\n"  sorted by path.

Required approvals (in design/approvals/):

  ux-critic-<design_hash>.json   and   ui-critic-<design_hash>.json

each with verdict "approved", `reviewed_at` not older than 30 days (and not in the future), a valid schema and an
EMPTY `blocking_findings` list.  See docs/DESIGN-GATE.md and design/approvals/README.md.

Exit status: 0 = gate passed (or --dry-run, or a valid signed override); 1 = gate failed; 2 = usage/internal error.
"""

from __future__ import annotations

import argparse
import ast
import datetime as dt
import hashlib
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable

CRITICS = ("ux-critic", "ui-critic")
MAX_AGE_DAYS = 30
MAX_FUTURE_SKEW = dt.timedelta(minutes=10)
MIN_CRITERION_SCORE = 6.0  # an approving verdict may not contain a criterion scored below this
OVERRIDE_MAX_DAYS = 14
OVERRIDE_FILE = "OVERRIDE.md"

REQUIRED_SCORES = {
    "ux-critic": (
        "task_success",
        "destructive_safety",
        "error_recovery",
        "information_architecture",
        "accessibility",
        "copy",
    ),
    "ui-critic": (
        "consistency",
        "contrast",
        "hierarchy",
        "density",
        "states",
        "theming",
        "platform_conformance",
        "implementation_fidelity",
    ),
}

HASH_RE = re.compile(r"^[0-9a-f]{64}$")
EXCLUDED_NAMES = {".DS_Store", "Thumbs.db"}
EXCLUDED_DIRS = {"__pycache__"}
MGMT_UI_SUFFIXES = {".html", ".htm", ".css", ".js", ".svg"}
MGMT_PY = "apps/mgmt-plane/synofurb_mgmt.py"
MGMT_PY_VAR = "INDEX_HTML"


# --------------------------------------------------------------------------------------------------
# design hash
# --------------------------------------------------------------------------------------------------
def _is_text(data: bytes) -> bool:
    return b"\0" not in data[:8192]


def _norm(data: bytes) -> bytes:
    return data.replace(b"\r\n", b"\n") if _is_text(data) else data


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _walk(base: Path, root: Path) -> Iterable[Path]:
    """All files below `base`, deterministic, skipping excluded names. Symlinks are returned as-is."""
    if not base.exists():
        return
    if base.is_file() or base.is_symlink():
        yield base
        return
    for dirpath, dirnames, filenames in os.walk(base, followlinks=False):
        dirnames[:] = sorted(d for d in dirnames if d not in EXCLUDED_DIRS)
        for name in sorted(filenames):
            if name in EXCLUDED_NAMES or name.endswith(".pyc"):
                continue
            yield Path(dirpath) / name


def _entry(path: Path, root: Path) -> tuple[str, str]:
    rel = path.relative_to(root).as_posix()
    if path.is_symlink():
        # never follow: a symlink must not be able to hide a change. Hash the link target text.
        return rel, _sha(b"symlink:" + os.readlink(path).encode("utf-8", "surrogateescape"))
    return rel, _sha(_norm(path.read_bytes()))


def extract_index_html(py_path: Path) -> bytes | None:
    """Return the module-level INDEX_HTML string constant of the management plane, or None."""
    try:
        tree = ast.parse(py_path.read_text(encoding="utf-8"))
    except (OSError, SyntaxError, UnicodeDecodeError):
        return None
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == MGMT_PY_VAR for t in node.targets):
            if isinstance(node.value, ast.Constant) and isinstance(node.value.value, str):
                return node.value.value.replace("\r\n", "\n").encode("utf-8")
    return None


def collect_design_files(root: Path) -> list[tuple[str, str]]:
    """Sorted list of (relative posix path, sha256) of every file that defines the design."""
    root = root.resolve()
    found: dict[str, str] = {}

    def add(path: Path) -> None:
        rel, digest = _entry(path, root)
        found[rel] = digest

    for sub in ("mockups", "mockups-v2", "v2"):
        for p in _walk(root / "design" / sub, root):
            add(p)
    design = root / "design"
    if design.is_dir():
        for child in sorted(design.iterdir()):
            if child.name.startswith("DESIGN-SYSTEM"):
                for p in _walk(child, root):
                    add(p)
    for p in _walk(root / "design" / "screenshots", root):
        add(p)
    mgmt = root / "apps" / "mgmt-plane"
    for p in _walk(mgmt, root):
        if p.suffix.lower() in MGMT_UI_SUFFIXES and "tests" not in p.relative_to(mgmt).parts:
            add(p)
    index_html = extract_index_html(root / MGMT_PY)
    if index_html is not None:
        found[f"{MGMT_PY}#{MGMT_PY_VAR}"] = _sha(index_html)
    for p in _walk(root / "crates" / "synofurb" / "src" / "ui", root):
        add(p)
    gui_rs = root / "crates" / "synofurb" / "src" / "gui.rs"
    if gui_rs.is_file() or gui_rs.is_symlink():
        add(gui_rs)
    for p in _walk(root / "crates" / "synofurb" / "src" / "gui", root):
        add(p)
    return sorted(found.items())


def compute_design_hash(entries: list[tuple[str, str]]) -> str:
    listing = "".join(f"{digest}  {rel}\n" for rel, digest in sorted(entries))
    return _sha(listing.encode("utf-8"))


# --------------------------------------------------------------------------------------------------
# approval validation
# --------------------------------------------------------------------------------------------------
def parse_time(value: object) -> dt.datetime | None:
    if not isinstance(value, str):
        return None
    text = value.strip()
    if text.endswith(("Z", "z")):
        text = text[:-1] + "+00:00"
    try:
        parsed = dt.datetime.fromisoformat(text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        return None
    return parsed


def _validate_findings(name: str, items: object, problems: list[str]) -> None:
    if not isinstance(items, list):
        problems.append(f"`{name}` must be a list")
        return
    for i, item in enumerate(items):
        if not isinstance(item, dict):
            problems.append(f"`{name}[{i}]` must be an object with at least `title` and `evidence`")
            continue
        for key in ("title", "evidence"):
            if not isinstance(item.get(key), str) or not item[key].strip():
                problems.append(f"`{name}[{i}].{key}` must be a non-empty string (findings without evidence are not accepted)")


def validate_approval(data: object, critic: str, design_hash: str, now: dt.datetime, max_age_days: int = MAX_AGE_DAYS) -> list[str]:
    """Return a list of human-readable problems; empty list means this approval is acceptable."""
    problems: list[str] = []
    if not isinstance(data, dict):
        return ["top level must be a JSON object"]
    if data.get("critic") != critic:
        problems.append(f"`critic` is {data.get('critic')!r}, expected {critic!r}")
    verdict = data.get("verdict")
    if verdict not in ("approved", "rejected"):
        problems.append(f"`verdict` is {verdict!r}, must be 'approved' or 'rejected'")
    elif verdict != "approved":
        problems.append("verdict is 'rejected'")
    dh = data.get("design_hash")
    if not isinstance(dh, str) or not HASH_RE.match(dh):
        problems.append("`design_hash` must be 64 lowercase hex characters")
    elif dh != design_hash:
        problems.append(f"`design_hash` inside the file ({dh[:12]}...) does not match the current design hash ({design_hash[:12]}...)")
    reviewed = parse_time(data.get("reviewed_at"))
    if reviewed is None:
        problems.append("`reviewed_at` must be an ISO-8601 timestamp with a time zone, e.g. 2026-11-01T12:00:00Z")
    else:
        if reviewed > now + MAX_FUTURE_SKEW:
            problems.append(f"`reviewed_at` ({data['reviewed_at']}) is in the future")
        elif now - reviewed > dt.timedelta(days=max_age_days):
            age = (now - reviewed).days
            problems.append(f"approval is {age} days old (limit {max_age_days}); run the critic again")
    model = data.get("reviewer_model")
    if not isinstance(model, str) or not model.strip():
        problems.append("`reviewer_model` must be a non-empty string")
    scores = data.get("scores")
    if not isinstance(scores, dict) or not scores:
        problems.append("`scores` must be a non-empty object")
    else:
        for key, value in scores.items():
            if isinstance(value, bool) or not isinstance(value, (int, float)) or not (0 <= value <= 10):
                problems.append(f"`scores.{key}` must be a number between 0 and 10")
        for key in REQUIRED_SCORES.get(critic, ()):
            if key not in scores:
                problems.append(f"`scores.{key}` is missing (required criterion for {critic})")
        if verdict == "approved":
            low = sorted(k for k, v in scores.items() if isinstance(v, (int, float)) and not isinstance(v, bool) and v < MIN_CRITERION_SCORE)
            if low:
                problems.append(f"verdict is 'approved' but criteria scored below {MIN_CRITERION_SCORE:g}: {', '.join(low)} (such a score is itself a blocking finding)")
    blocking = data.get("blocking_findings")
    _validate_findings("blocking_findings", blocking, problems)
    if isinstance(blocking, list) and blocking:
        problems.append(f"{len(blocking)} blocking finding(s) are recorded")
    _validate_findings("non_blocking_findings", data.get("non_blocking_findings"), problems)
    return problems


# --------------------------------------------------------------------------------------------------
# override
# --------------------------------------------------------------------------------------------------
def _kv(text: str, key: str) -> str | None:
    m = re.search(rf"^\s*{re.escape(key)}\s*:\s*(.+?)\s*$", text, re.IGNORECASE | re.MULTILINE)
    return m.group(1) if m else None


def _git(root: Path, *args: str) -> str | None:
    try:
        out = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, timeout=30, check=False)
    except (OSError, subprocess.SubprocessError):
        return None
    return out.stdout.strip() if out.returncode == 0 else None


def override_signature_ok(root: Path, rel: str) -> tuple[bool, str]:
    """The override file must be in a commit with a verified signature.

    CI cannot verify signatures locally (no keys), so the workflow asks the GitHub API and passes
    RELEASE_GATE_OVERRIDE_SIGNATURE=verified. Locally `git log --format=%G?` must say 'G'."""
    if os.environ.get("RELEASE_GATE_OVERRIDE_SIGNATURE", "").strip().lower() == "verified":
        return True, "signature verified by the GitHub API (RELEASE_GATE_OVERRIDE_SIGNATURE=verified)"
    status = _git(root, "log", "-n1", "--format=%G?", "--", rel)
    if status is None:
        return False, "cannot run git to check the commit signature"
    if status == "":
        return False, f"{rel} is not committed (an override must be a signed commit)"
    if status == "G":
        return True, "git reports a good signature on the last commit touching the file"
    return False, f"last commit touching {rel} has signature status {status!r} (need 'G' = good, signed)"


@dataclass
class Override:
    ok: bool
    problems: list[str] = field(default_factory=list)
    maintainer: str = ""
    reason: str = ""
    expires: str = ""
    signature: str = ""


def check_override(root: Path, approvals: Path, design_hash: str, today: dt.date) -> Override | None:
    path = approvals / OVERRIDE_FILE
    if not path.is_file():
        return None
    text = path.read_text(encoding="utf-8", errors="replace")
    ov = Override(ok=False)
    h = (_kv(text, "design_hash") or "").lower()
    if h != design_hash:
        ov.problems.append(f"design_hash in {OVERRIDE_FILE} ({h[:12] or 'missing'}...) does not match the current hash ({design_hash[:12]}...); an override covers exactly one design state")
    ov.maintainer = _kv(text, "maintainer") or ""
    if not re.search(r"\S+.*<[^<>@\s]+@[^<>\s]+>", ov.maintainer):
        ov.problems.append("`maintainer:` must be 'Name <email>'")
    ov.reason = _kv(text, "reason") or ""
    if len(ov.reason) < 30:
        ov.problems.append("`reason:` must explain the override in at least 30 characters")
    ov.expires = _kv(text, "expires") or ""
    granted = _kv(text, "granted") or ""
    try:
        exp = dt.date.fromisoformat(ov.expires)
        gr = dt.date.fromisoformat(granted)
        if exp < today:
            ov.problems.append(f"override expired on {exp}")
        if (exp - gr).days > OVERRIDE_MAX_DAYS or exp < gr:
            ov.problems.append(f"`expires:` must be within {OVERRIDE_MAX_DAYS} days after `granted:`")
        if gr > today + dt.timedelta(days=1):
            ov.problems.append("`granted:` is in the future")
    except ValueError:
        ov.problems.append("`granted:` and `expires:` must be YYYY-MM-DD dates")
    try:
        rel = path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        rel = str(path)
    sig_ok, sig_msg = override_signature_ok(root, rel)
    ov.signature = sig_msg
    if not sig_ok:
        ov.problems.append(sig_msg)
    ov.ok = not ov.problems
    return ov


# --------------------------------------------------------------------------------------------------
# evaluation
# --------------------------------------------------------------------------------------------------
@dataclass
class Result:
    design_hash: str
    files: list[tuple[str, str]]
    problems: dict[str, list[str]] = field(default_factory=dict)  # critic or "design" -> problems
    override: Override | None = None

    @property
    def approved(self) -> bool:
        return not self.problems

    @property
    def status(self) -> str:
        if self.approved:
            return "approved"
        if self.override is not None and self.override.ok:
            return "override"
        return "failed"


def evaluate(root: Path, now: dt.datetime, approvals_dir: Path | None = None, max_age_days: int = MAX_AGE_DAYS) -> Result:
    root = root.resolve()
    approvals = approvals_dir or (root / "design" / "approvals")
    files = collect_design_files(root)
    dh = compute_design_hash(files)
    res = Result(design_hash=dh, files=files)
    if not any(rel.startswith(("design/mockups/", "design/mockups-v2/")) for rel, _ in files):
        res.problems["design"] = ["no files found under design/mockups/ or design/mockups-v2/ - there is no design to approve"]
    for critic in CRITICS:
        path = approvals / f"{critic}-{dh}.json"
        probs: list[str] = []
        if not path.is_file():
            others = sorted(p.name for p in approvals.glob(f"{critic}-*.json")) if approvals.is_dir() else []
            try:
                shown = path.resolve().relative_to(root).as_posix()
            except ValueError:
                shown = str(path)
            probs.append(f"missing {shown}")
            if others:
                probs.append(f"found approval(s) for a DIFFERENT design state (stale, ignored): {', '.join(others)}")
        else:
            try:
                data = json.loads(path.read_text(encoding="utf-8"))
            except (OSError, ValueError) as exc:
                probs.append(f"{path.name} is not valid JSON: {exc}")
            else:
                probs.extend(validate_approval(data, critic, dh, now, max_age_days))
        if probs:
            res.problems[critic] = probs
    if res.problems:
        res.override = check_override(root, approvals, dh, now.date())
    return res


def render(res: Result, dry_run: bool) -> str:
    out: list[str] = [f"design_hash: {res.design_hash}", f"design files hashed: {len(res.files)}"]
    if res.approved:
        out.append("RELEASE GATE: PASSED - both critics approved this exact design state.")
        return "\n".join(out)
    out.append("")
    out.append("RELEASE GATE: NOT APPROVED")
    for who, probs in res.problems.items():
        label = {"design": "design", "ux-critic": "UX critic", "ui-critic": "UI critic"}.get(who, who)
        out.append(f"  [{label}]")
        out.extend(f"    - {p}" for p in probs)
    if res.override is not None:
        out.append("")
        if res.override.ok:
            bar = "!" * 78
            out += [bar, "!!! DESIGN GATE OVERRIDDEN BY A MAINTAINER - THE DESIGN IS NOT APPROVED BY THE CRITICS !!!", bar,
                    f"  maintainer: {res.override.maintainer}", f"  reason:     {res.override.reason}",
                    f"  expires:    {res.override.expires}", f"  signature:  {res.override.signature}"]
        else:
            out.append(f"  [override] {OVERRIDE_FILE} is present but NOT valid:")
            out.extend(f"    - {p}" for p in res.override.problems)
    out.append("")
    out.append("How to fix: run the ux-critic and ui-critic agents (see docs/DESIGN-GATE.md), address every blocking")
    out.append(f"finding, re-run them, and commit design/approvals/ux-critic-{res.design_hash}.json and")
    out.append(f"design/approvals/ui-critic-{res.design_hash}.json.")
    if dry_run:
        out.append("")
        out.append("--dry-run: reporting only; exiting 0 although the gate would FAIL.")
    return "\n".join(out)


def _write_ci_outputs(res: Result, dry_run: bool) -> None:
    out_file = os.environ.get("GITHUB_OUTPUT")
    if out_file:
        status = res.status if not (dry_run and res.status == "failed") else "dry-run-failed"
        with open(out_file, "a", encoding="utf-8") as fh:
            fh.write(f"design_hash={res.design_hash}\ngate_status={status}\n")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write(f"### Design gate: {res.status.upper()}\n\n`design_hash` = `{res.design_hash}`\n\n```\n{render(res, dry_run)}\n```\n")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="synofurb release gate: require UX + UI critic approval of the exact current design.")
    ap.add_argument("--root", default=str(Path(__file__).resolve().parent.parent), help="repository root (default: this repo)")
    ap.add_argument("--approvals-dir", help="default: <root>/design/approvals")
    ap.add_argument("--print-hash", action="store_true", help="print only the design hash and exit 0")
    ap.add_argument("--list", action="store_true", help="list every file that feeds the design hash (sha256  path), then the hash")
    ap.add_argument("--dry-run", action="store_true", help="evaluate and report, but always exit 0")
    ap.add_argument("--max-age-days", type=int, default=MAX_AGE_DAYS)
    ap.add_argument("--now", help="override 'now' (ISO-8601 with zone); for tests")
    args = ap.parse_args(argv)

    root = Path(args.root)
    if not root.is_dir():
        print(f"error: --root {root} is not a directory", file=sys.stderr)
        return 2
    now = parse_time(args.now) if args.now else dt.datetime.now(dt.timezone.utc)
    if now is None:
        print("error: --now must be an ISO-8601 timestamp with a zone", file=sys.stderr)
        return 2

    if args.print_hash or args.list:
        files = collect_design_files(root)
        if args.list:
            for rel, digest in files:
                print(f"{digest}  {rel}")
            print(f"# {len(files)} files")
            print(f"# design_hash {compute_design_hash(files)}")
        else:
            print(compute_design_hash(files))
        return 0

    res = evaluate(root, now, Path(args.approvals_dir) if args.approvals_dir else None, args.max_age_days)
    print(render(res, args.dry_run))
    _write_ci_outputs(res, args.dry_run)
    if res.status in ("approved", "override") or args.dry_run:
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
