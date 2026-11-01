"""Unit + CLI tests for scripts/release-gate.py.  Run: python3 -m unittest discover -s tests/gate -v"""

import datetime as dt
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
GATE = REPO / "scripts" / "release-gate.py"
spec = importlib.util.spec_from_file_location("release_gate", GATE)
gate = importlib.util.module_from_spec(spec)
sys.modules["release_gate"] = gate
spec.loader.exec_module(gate)

NOW = dt.datetime(2026, 11, 1, 12, 0, 0, tzinfo=dt.timezone.utc)


def good(critic: str, dh: str, **over):
    data = {
        "critic": critic,
        "verdict": "approved",
        "design_hash": dh,
        "reviewed_at": "2026-10-30T09:00:00Z",
        "reviewer_model": "claude-opus-test",
        "scores": {k: 8 for k in gate.REQUIRED_SCORES[critic]},
        "blocking_findings": [],
        "non_blocking_findings": [{"title": "nit", "evidence": "design/mockups/ux/a.html:3 label is 11px"}],
    }
    data.update(over)
    return data


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "design" / "mockups" / "ux").mkdir(parents=True)
        (self.root / "design" / "approvals").mkdir(parents=True)
        (self.root / "design" / "mockups" / "ux" / "a.html").write_text("<h1>hi</h1>\n")

    def tearDown(self):
        self.tmp.cleanup()

    def hash(self):
        return gate.compute_design_hash(gate.collect_design_files(self.root))

    def write_approvals(self, **over):
        dh = self.hash()
        for critic in gate.CRITICS:
            (self.root / "design" / "approvals" / f"{critic}-{dh}.json").write_text(json.dumps(good(critic, dh, **over)))
        return dh

    def run_gate(self, *args, env=None):
        e = dict(os.environ)
        e.pop("GITHUB_OUTPUT", None)
        e.pop("GITHUB_STEP_SUMMARY", None)
        e.pop("RELEASE_GATE_OVERRIDE_SIGNATURE", None)
        e.update(env or {})
        return subprocess.run([sys.executable, str(GATE), "--root", str(self.root), "--now", "2026-11-01T12:00:00Z", *args],
                              capture_output=True, text=True, env=e)


class HashTests(Fixture):
    def test_deterministic_and_content_sensitive(self):
        h1 = self.hash()
        self.assertEqual(h1, self.hash())
        (self.root / "design" / "mockups" / "ux" / "a.html").write_text("<h1>changed</h1>\n")
        self.assertNotEqual(h1, self.hash())

    def test_crlf_normalised(self):
        h1 = self.hash()
        (self.root / "design" / "mockups" / "ux" / "a.html").write_bytes(b"<h1>hi</h1>\r\n")
        self.assertEqual(h1, self.hash())

    def test_binary_not_normalised(self):
        p = self.root / "design" / "mockups" / "x.png"
        p.write_bytes(b"\x89PNG\0\r\n\x1a\n")
        h1 = self.hash()
        p.write_bytes(b"\x89PNG\0\n\x1a\n")
        self.assertNotEqual(h1, self.hash())

    def test_path_matters(self):
        h1 = self.hash()
        (self.root / "design" / "mockups" / "ux" / "a.html").rename(self.root / "design" / "mockups" / "ux" / "b.html")
        self.assertNotEqual(h1, self.hash())

    def test_new_file_in_each_included_area_changes_hash(self):
        areas = [
            "design/mockups/ui/d1/new.html",
            "design/mockups-v2/desktop/new.html",
            "design/v2/UX-SPEC.md",
            "design/DESIGN-SYSTEM.md",
            "design/screenshots/win/main.png",
            "apps/mgmt-plane/ui/page.html",
            "crates/synofurb/src/ui/mod.rs",
            "crates/synofurb/src/gui.rs",
        ]
        seen = {self.hash()}
        for rel in areas:
            p = self.root / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("x")
            h = self.hash()
            self.assertNotIn(h, seen, rel)
            seen.add(h)

    def test_excluded_things_do_not_change_hash(self):
        h1 = self.hash()
        for rel in ["design/approvals/x.json", "design/mockups/.DS_Store", "design/mockups/__pycache__/a.pyc",
                    "apps/mgmt-plane/tests/t.html", "apps/mgmt-plane/synofurb_mgmt.pyc", "docs/other.md", "design/notes.txt"]:
            p = self.root / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("x")
        self.assertEqual(h1, self.hash())

    def test_mgmt_index_html_extracted_backend_changes_ignored(self):
        mg = self.root / "apps" / "mgmt-plane"
        mg.mkdir(parents=True)
        py = mg / "synofurb_mgmt.py"
        py.write_text('VERSION = "1"\nINDEX_HTML = r"""<html>A</html>"""\ndef f():\n    return 1\n')
        h1 = self.hash()
        py.write_text('VERSION = "2"\nINDEX_HTML = r"""<html>A</html>"""\ndef f():\n    return 2\n')
        self.assertEqual(h1, self.hash(), "backend-only edit must not invalidate approvals")
        py.write_text('VERSION = "2"\nINDEX_HTML = r"""<html>B</html>"""\n')
        self.assertNotEqual(h1, self.hash(), "UI edit must invalidate approvals")
        self.assertIn("apps/mgmt-plane/synofurb_mgmt.py#INDEX_HTML", dict(gate.collect_design_files(self.root)))

    def test_symlink_is_hashed_not_followed(self):
        target = self.root / "secret.txt"
        target.write_text("one")
        link = self.root / "design" / "mockups" / "link.txt"
        try:
            link.symlink_to(target)
        except OSError:
            self.skipTest("symlinks unavailable")
        h1 = self.hash()
        target.write_text("two")
        self.assertEqual(h1, self.hash())

    def test_print_hash_cli(self):
        r = self.run_gate("--print-hash")
        self.assertEqual(r.returncode, 0)
        self.assertEqual(r.stdout.strip(), self.hash())

    def test_list_cli(self):
        r = self.run_gate("--list")
        self.assertEqual(r.returncode, 0)
        self.assertIn("design/mockups/ux/a.html", r.stdout)
        self.assertIn(self.hash(), r.stdout)

    def test_real_repo_hash_is_stable(self):
        a = subprocess.run([sys.executable, str(GATE), "--print-hash"], capture_output=True, text=True)
        b = subprocess.run([sys.executable, str(GATE), "--print-hash"], capture_output=True, text=True)
        self.assertEqual(a.returncode, 0)
        self.assertRegex(a.stdout.strip(), r"^[0-9a-f]{64}$")
        self.assertEqual(a.stdout, b.stdout)


class GateTests(Fixture):
    def test_fails_when_no_approvals(self):
        r = self.run_gate()
        self.assertEqual(r.returncode, 1)
        self.assertIn("missing design/approvals/ux-critic-", r.stdout)
        self.assertIn("missing design/approvals/ui-critic-", r.stdout)

    def test_dry_run_exits_zero_but_says_failed(self):
        r = self.run_gate("--dry-run")
        self.assertEqual(r.returncode, 0)
        self.assertIn("NOT APPROVED", r.stdout)
        self.assertIn("--dry-run", r.stdout)

    def test_passes_with_valid_approvals(self):
        self.write_approvals()
        r = self.run_gate()
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("PASSED", r.stdout)

    def test_fails_when_no_mockups(self):
        (self.root / "design" / "mockups" / "ux" / "a.html").unlink()
        self.write_approvals()
        r = self.run_gate()
        self.assertEqual(r.returncode, 1)
        self.assertIn("no design to approve", r.stdout)

    def test_only_one_critic_is_not_enough(self):
        dh = self.write_approvals()
        (self.root / "design" / "approvals" / f"ui-critic-{dh}.json").unlink()
        r = self.run_gate()
        self.assertEqual(r.returncode, 1)
        self.assertIn("[UI critic]", r.stdout)
        self.assertNotIn("[UX critic]", r.stdout)

    def test_design_change_invalidates(self):
        old = self.write_approvals()
        (self.root / "design" / "mockups" / "ux" / "a.html").write_text("<h1>edited after approval</h1>")
        r = self.run_gate()
        self.assertEqual(r.returncode, 1)
        self.assertIn("DIFFERENT design state", r.stdout)
        self.assertIn(old[:12], r.stdout)

    def test_rejected_verdict_fails(self):
        self.write_approvals(verdict="rejected")
        r = self.run_gate()
        self.assertEqual(r.returncode, 1)
        self.assertIn("rejected", r.stdout)

    def test_blocking_findings_fail_even_if_verdict_approved(self):
        self.write_approvals(blocking_findings=[{"title": "x", "evidence": "y"}])
        r = self.run_gate()
        self.assertEqual(r.returncode, 1)
        self.assertIn("blocking finding", r.stdout)

    def test_stale_approval_fails(self):
        self.write_approvals(reviewed_at="2026-09-01T00:00:00Z")
        r = self.run_gate()
        self.assertEqual(r.returncode, 1)
        self.assertIn("days old", r.stdout)

    def test_exactly_30_days_ok_31_fails(self):
        dh = self.hash()
        ok = gate.validate_approval(good("ux-critic", dh, reviewed_at="2026-10-02T12:00:00Z"), "ux-critic", dh, NOW)
        bad = gate.validate_approval(good("ux-critic", dh, reviewed_at="2026-10-02T11:59:00Z"), "ux-critic", dh, NOW)
        self.assertEqual(ok, [])
        self.assertTrue(any("days old" in p for p in bad))

    def test_future_timestamp_fails(self):
        self.write_approvals(reviewed_at="2027-01-01T00:00:00Z")
        r = self.run_gate()
        self.assertEqual(r.returncode, 1)
        self.assertIn("future", r.stdout)

    def test_naive_timestamp_fails(self):
        self.write_approvals(reviewed_at="2026-10-30T09:00:00")
        self.assertEqual(self.run_gate().returncode, 1)

    def test_timezone_offset_accepted(self):
        self.write_approvals(reviewed_at="2026-10-30T11:00:00+02:00")
        self.assertEqual(self.run_gate().returncode, 0)

    def test_hash_inside_file_must_match(self):
        dh = self.write_approvals()
        p = self.root / "design" / "approvals" / f"ux-critic-{dh}.json"
        data = json.loads(p.read_text())
        data["design_hash"] = "0" * 64
        p.write_text(json.dumps(data))
        r = self.run_gate()
        self.assertEqual(r.returncode, 1)
        self.assertIn("does not match", r.stdout)

    def test_critic_field_must_match_filename(self):
        dh = self.write_approvals()
        p = self.root / "design" / "approvals" / f"ux-critic-{dh}.json"
        p.write_text(json.dumps(good("ui-critic", dh)))
        self.assertEqual(self.run_gate().returncode, 1)

    def test_invalid_json_fails(self):
        dh = self.write_approvals()
        (self.root / "design" / "approvals" / f"ux-critic-{dh}.json").write_text("{nope")
        r = self.run_gate()
        self.assertEqual(r.returncode, 1)
        self.assertIn("not valid JSON", r.stdout)

    def test_missing_required_score_fails(self):
        scores = {k: 9 for k in gate.REQUIRED_SCORES["ux-critic"] if k != "copy"}
        dh = self.hash()
        probs = gate.validate_approval(good("ux-critic", dh, scores=scores), "ux-critic", dh, NOW)
        self.assertTrue(any("scores.copy" in p for p in probs))

    def test_low_score_with_approval_fails(self):
        dh = self.hash()
        scores = {k: 9 for k in gate.REQUIRED_SCORES["ui-critic"]}
        scores["contrast"] = 3
        probs = gate.validate_approval(good("ui-critic", dh, scores=scores), "ui-critic", dh, NOW)
        self.assertTrue(any("contrast" in p for p in probs))

    def test_score_out_of_range_and_bool(self):
        dh = self.hash()
        scores = {k: 9 for k in gate.REQUIRED_SCORES["ui-critic"]}
        scores["contrast"] = 11
        scores["density"] = True
        probs = gate.validate_approval(good("ui-critic", dh, scores=scores), "ui-critic", dh, NOW)
        self.assertTrue(any("scores.contrast" in p for p in probs))
        self.assertTrue(any("scores.density" in p for p in probs))

    def test_findings_need_evidence(self):
        dh = self.hash()
        probs = gate.validate_approval(good("ux-critic", dh, non_blocking_findings=[{"title": "vague"}]), "ux-critic", dh, NOW)
        self.assertTrue(any("evidence" in p for p in probs))

    def test_reviewer_model_required(self):
        dh = self.hash()
        probs = gate.validate_approval(good("ux-critic", dh, reviewer_model=""), "ux-critic", dh, NOW)
        self.assertTrue(any("reviewer_model" in p for p in probs))

    def test_github_output_written(self):
        out = self.root / "gh_out"
        r = self.run_gate("--dry-run", env={"GITHUB_OUTPUT": str(out)})
        self.assertEqual(r.returncode, 0)
        text = out.read_text()
        self.assertIn(f"design_hash={self.hash()}", text)
        self.assertIn("gate_status=dry-run-failed", text)
        self.write_approvals()
        out.unlink()
        self.run_gate(env={"GITHUB_OUTPUT": str(out)})
        self.assertIn("gate_status=approved", out.read_text())


class OverrideTests(Fixture):
    def write_override(self, **kw):
        fields = {
            "design_hash": self.hash(),
            "maintainer": "Jane Maintainer <jane@example.org>",
            "reason": "Critics unavailable; hotfix for CVE-2026-0001 with no UI change",
            "granted": "2026-11-01",
            "expires": "2026-11-05",
        }
        fields.update(kw)
        (self.root / "design" / "approvals" / "OVERRIDE.md").write_text("# Override\n\n" + "\n".join(f"{k}: {v}" for k, v in fields.items()) + "\n")

    def test_override_without_signature_is_rejected(self):
        self.write_override()
        r = self.run_gate()  # root is not a git repo -> cannot verify
        self.assertEqual(r.returncode, 1)
        self.assertIn("OVERRIDE.md is present but NOT valid", r.stdout)

    def test_override_with_verified_signature_passes_loudly(self):
        self.write_override()
        r = self.run_gate(env={"RELEASE_GATE_OVERRIDE_SIGNATURE": "verified"})
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("DESIGN GATE OVERRIDDEN", r.stdout)
        self.assertIn("jane@example.org", r.stdout)

    def test_override_bound_to_hash(self):
        self.write_override(design_hash="1" * 64)
        r = self.run_gate(env={"RELEASE_GATE_OVERRIDE_SIGNATURE": "verified"})
        self.assertEqual(r.returncode, 1)
        self.assertIn("covers exactly one design state", r.stdout)

    def test_override_expired(self):
        self.write_override(granted="2026-10-01", expires="2026-10-10")
        r = self.run_gate(env={"RELEASE_GATE_OVERRIDE_SIGNATURE": "verified"})
        self.assertEqual(r.returncode, 1)
        self.assertIn("expired", r.stdout)

    def test_override_too_long(self):
        self.write_override(granted="2026-11-01", expires="2026-12-31")
        r = self.run_gate(env={"RELEASE_GATE_OVERRIDE_SIGNATURE": "verified"})
        self.assertEqual(r.returncode, 1)
        self.assertIn("within 14 days", r.stdout)

    def test_override_needs_reason_and_maintainer(self):
        self.write_override(reason="because", maintainer="jane")
        r = self.run_gate(env={"RELEASE_GATE_OVERRIDE_SIGNATURE": "verified"})
        self.assertEqual(r.returncode, 1)
        self.assertIn("reason", r.stdout)
        self.assertIn("maintainer", r.stdout)

    @unittest.skipUnless(shutil.which("git") and shutil.which("ssh-keygen"), "needs git + ssh-keygen")
    def test_override_with_real_signed_commit(self):
        def git(*a, check=True):
            return subprocess.run(["git", "-C", str(self.root), *a], capture_output=True, text=True, check=check)

        key = self.root.parent / (self.root.name + "-key")
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(key)], check=True)
        signers = self.root.parent / (self.root.name + "-signers")
        signers.write_text("jane@example.org " + Path(str(key) + ".pub").read_text())
        try:
            git("init", "-q")
            for k, v in [("user.name", "Jane"), ("user.email", "jane@example.org"), ("gpg.format", "ssh"),
                         ("user.signingkey", str(key) + ".pub"), ("gpg.ssh.allowedSignersFile", str(signers)),
                         ("commit.gpgsign", "false")]:
                git("config", k, v)
            self.write_override()
            git("add", "-A")
            git("commit", "-q", "-m", "unsigned override")
            r = self.run_gate()
            self.assertEqual(r.returncode, 1, "unsigned commit must not be accepted")
            self.assertIn("signature status", r.stdout)
            git("commit", "-q", "--amend", "-S", "-m", "signed override")
            if git("log", "-n1", "--format=%G?", check=False).stdout.strip() != "G":
                self.skipTest("this git/ssh-keygen cannot verify ssh signatures")
            r = self.run_gate()
            self.assertEqual(r.returncode, 0, r.stdout)
            self.assertIn("DESIGN GATE OVERRIDDEN", r.stdout)
        finally:
            for f in (key, Path(str(key) + ".pub"), signers):
                f.unlink(missing_ok=True)


class RealRepoTests(unittest.TestCase):
    def test_gate_fails_today_without_approvals(self):
        """Intended state until the critics have approved: the gate must fail on the real repo."""
        approvals = REPO / "design" / "approvals"
        real = [p for p in approvals.glob("*-critic-*.json")] if approvals.is_dir() else []
        if real or (approvals / "OVERRIDE.md").exists():
            self.skipTest("real approvals exist")
        r = subprocess.run([sys.executable, str(GATE)], capture_output=True, text=True, cwd=str(REPO))
        self.assertEqual(r.returncode, 1, r.stdout)


if __name__ == "__main__":
    unittest.main()
