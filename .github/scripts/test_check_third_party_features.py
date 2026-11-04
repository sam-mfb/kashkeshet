"""check-third-party-features.sh: CI fails when the third-party feature list changes, and says how to update it."""

from pathlib import Path
import subprocess
import unittest

SCRIPTS = Path(__file__).resolve().parent
CHECK = SCRIPTS / "check-third-party-features.sh"
LIST = SCRIPTS.parent / "third-party-features.txt"


def check(output, list_file=LIST):
    """Run the check as CI does, with `output` in place of third-party-features.sh's."""
    return subprocess.run(
        ["bash", str(CHECK), str(list_file)],
        input=output, capture_output=True, text=True,
    )


class CheckThirdPartyFeatures(unittest.TestCase):
    def setUp(self):
        self.kept = LIST.read_text()
        self.lines = self.kept.splitlines(keepends=True)
        self.assertGreater(len(self.lines), 1, "the kept list should name more than one package")

    def assertFailsWith(self, result, *shown):
        self.assertEqual(result.returncode, 1, result.stderr)
        for text in shown:
            self.assertIn(text, result.stdout)
        self.assertIn("update the list in the\nsame pull request and say in its description that the change is intended", result.stderr)
        self.assertIn(f"LC_ALL=C bash .github/scripts/third-party-features.sh > {LIST}", result.stderr)

    def test_the_same_list_passes(self):
        result = check(self.kept)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "")

    def test_a_removed_package_fails_and_shows_it(self):
        removed = self.lines[0]
        result = check("".join(self.lines[1:]))
        self.assertFailsWith(result, "-" + removed)

    def test_an_added_package_fails_and_shows_it(self):
        added = "zzz-new-package v1.0.0 [default]\n"
        result = check(self.kept + added)
        self.assertFailsWith(result, "+" + added)

    def test_a_changed_feature_or_version_fails_and_shows_it(self):
        first = self.lines[0]
        changed = first.replace("]", ",extra-feature]", 1)
        result = check("".join([changed] + self.lines[1:]))
        self.assertFailsWith(result, "-" + first, "+" + changed)

    def test_a_missing_list_file_is_an_error_not_a_pass(self):
        result = check(self.kept, list_file=SCRIPTS / "no-such-list.txt")
        self.assertEqual(result.returncode, 2)
        self.assertIn("no list file", result.stderr)


if __name__ == "__main__":
    unittest.main()
