"""The committed addons.pack.json (embedded into the desktop app and shipped to new units) must equal what the build produces."""
import os
import sys
import unittest

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, ROOT)
import synofurb_addons  # noqa: E402


class PackIsFresh(unittest.TestCase):
    def test_committed_pack_matches_the_add_on_definitions(self):
        with open(os.path.join(ROOT, "addons.pack.json"), encoding="utf-8", newline="") as fh:
            committed = fh.read()
        built = synofurb_addons.build_pack(os.path.join(ROOT, "addons"))
        self.assertEqual(
            committed, built,
            "apps/mgmt-plane/addons.pack.json is stale: regenerate it with\n"
            "  cd apps/mgmt-plane && python3 -c \"import synofurb_addons as a; open('addons.pack.json','w',newline='\\n').write(a.build_pack('addons'))\"")


if __name__ == "__main__":
    unittest.main()
