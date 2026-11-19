"""check-release-packages.sh: a workspace package's `testing` feature may be on in tests, never in a released program."""

import os
from pathlib import Path
import shutil
import subprocess
import unittest

SCRIPT = Path(__file__).resolve().parent / "check-release-packages.sh"
ROOT = SCRIPT.parent.parent.parent
SCRATCH = ROOT / "target" / "tmp" / "check-release-packages"


def workspace(name, section):
    """A workspace of a library with a `testing` feature, and a released
    program that turns it on in `section`."""
    dir = SCRATCH / name
    shutil.rmtree(dir, ignore_errors=True)
    (dir / "lib" / "src").mkdir(parents=True)
    (dir / "prog" / "src").mkdir(parents=True)
    (dir / "Cargo.toml").write_text('[workspace]\nmembers = ["lib", "prog"]\nresolver = "2"\n')
    (dir / "lib" / "Cargo.toml").write_text(
        '[package]\nname = "stand-in"\nversion = "0.1.0"\nedition = "2024"\n\n'
        '[features]\ntesting = []\n'
    )
    (dir / "lib" / "src" / "lib.rs").write_text("")
    (dir / "prog" / "Cargo.toml").write_text(
        '[package]\nname = "prog"\nversion = "0.1.0"\nedition = "2024"\n\n'
        f'[{section}]\nstand-in = {{ path = "../lib", features = ["testing"] }}\n'
    )
    (dir / "prog" / "src" / "main.rs").write_text("fn main() {}\n")
    subprocess.run(
        ["cargo", "generate-lockfile", "--offline", "--manifest-path", str(dir / "Cargo.toml")],
        check=True, capture_output=True,
    )
    return dir / "Cargo.toml"


def check(manifest):
    return subprocess.run(
        ["bash", str(SCRIPT)], env={**os.environ, "KKT_MANIFEST": str(manifest)},
        capture_output=True, text=True,
    )


class TestingInRelease(unittest.TestCase):
    def test_testing_through_a_dev_dependency_passes(self):
        done = check(workspace("dev", "dev-dependencies"))
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("no dev-only package and no testing feature in: prog", done.stdout)

    def test_testing_through_a_normal_dependency_fails(self):
        done = check(workspace("normal", "dependencies"))
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("stand-in has its testing feature on in prog", done.stderr)

    def test_testing_through_a_build_dependency_fails(self):
        done = check(workspace("build", "build-dependencies"))
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("stand-in has its testing feature on in prog", done.stderr)


if __name__ == "__main__":
    unittest.main()
