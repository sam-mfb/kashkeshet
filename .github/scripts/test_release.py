"""Release gates: reject wrong provenance, incomplete archives and missing notes; enforce locks."""

import importlib.util
import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest

SCRIPTS = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("check_release", SCRIPTS / "check-release.py")
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)
TAG_SPEC = importlib.util.spec_from_file_location("check_release_tag", SCRIPTS / "check-release-tag.py")
TAG_CHECK = importlib.util.module_from_spec(TAG_SPEC)
TAG_SPEC.loader.exec_module(TAG_CHECK)
NOTES_SPEC = importlib.util.spec_from_file_location("release_notes", SCRIPTS / "release-notes.py")
NOTES = importlib.util.module_from_spec(NOTES_SPEC)
NOTES_SPEC.loader.exec_module(NOTES)
ROOT = SCRIPTS.parents[1]
# Scratch folders go under the repository's target/tmp, never the system temporary folder.
SCRATCH = ROOT / "target" / "tmp"


def scratch():
    """A new folder under target/tmp, removed when the returned object is cleaned up."""
    SCRATCH.mkdir(parents=True, exist_ok=True)
    return tempfile.TemporaryDirectory(prefix=f"test_release-{os.getpid()}-", dir=SCRATCH)


CHANGELOG = """# Changelog

Notes for each version, newest first.

## 0.2.0 - second

### Agents
- Something new.

## 0.1.0

- The first release.
"""


class ReleaseTags(unittest.TestCase):
    def check(self, version, tag, changelog=CHANGELOG):
        with scratch() as directory:
            manifest = Path(directory) / "Cargo.toml"
            manifest.write_text(f'[package]\nname = "kashkeshet"\nversion = "{version}"\n')
            notes = Path(directory) / "CHANGELOG.md"
            notes.write_text(changelog)
            return TAG_CHECK.check(tag, manifest, notes)

    def test_stable_version_matches_its_tag(self):
        self.assertFalse(self.check("0.1.0", "v0.1.0"))

    def test_candidates_match_and_are_classified_as_prereleases(self):
        for version in ("0.1.0-rc.1", "2.10.3-rc.12", "0.0.0-rc.0"):
            with self.subTest(version=version):
                notes = f"## {version.split('-')[0]}\n- notes\n"
                self.assertTrue(self.check(version, "v" + version, notes))

    def test_mismatched_version_is_refused_including_stable_vs_rc(self):
        for version, tag in [("0.1.0", "v0.1.0-rc.1"), ("0.1.0-rc.1", "v0.1.0"), ("0.1.0", "v0.2.0"), ("0.1.0-rc.1", "v0.1.0-rc.2")]:
            with self.subTest(version=version, tag=tag):
                with self.assertRaisesRegex(ValueError, "does not match Cargo package version"):
                    self.check(version, tag)

    def test_other_tag_forms_and_leading_zeros_are_refused(self):
        for tag in ("0.1.0", "release/v0.1.0", "v0.1", "v01.1.0", "v0.01.0", "v0.1.00", "v0.1.0-rc.01", "v0.1.0-beta.1", "v0.1.0-rc.1-extra", "v0.1.0+build", "v0.1.0\n"):
            with self.subTest(tag=tag):
                with self.assertRaisesRegex(ValueError, "release tag must be"):
                    self.check("0.1.0", tag)

    def test_version_without_notes_is_refused(self):
        for version in ("0.3.0", "0.3.0-rc.1"):
            with self.subTest(version=version):
                with self.assertRaisesRegex(ValueError, "CHANGELOG.md has no section for 0.3.0"):
                    self.check(version, "v" + version)

    def test_candidate_uses_its_versions_notes(self):
        self.assertTrue(self.check("0.2.0-rc.3", "v0.2.0-rc.3"))

    def test_missing_changelog_is_refused(self):
        with scratch() as directory:
            manifest = Path(directory) / "Cargo.toml"
            manifest.write_text('[package]\nname = "kashkeshet"\nversion = "0.1.0"\n')
            with self.assertRaises(OSError):
                TAG_CHECK.check("v0.1.0", manifest, Path(directory) / "CHANGELOG.md")

    def test_repository_changelog_has_notes_for_the_cargo_version(self):
        version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
        TAG_CHECK.check("v" + version, ROOT / "Cargo.toml", ROOT / "CHANGELOG.md")


class ReleaseNotes(unittest.TestCase):
    def test_section_is_the_versions_notes_without_its_heading(self):
        self.assertEqual(NOTES.section(CHANGELOG, "v0.2.0"), "### Agents\n- Something new.")
        self.assertEqual(NOTES.section(CHANGELOG, "v0.1.0"), "- The first release.")

    def test_candidate_tag_reads_its_versions_section(self):
        self.assertEqual(NOTES.section(CHANGELOG, "v0.2.0-rc.1"), NOTES.section(CHANGELOG, "v0.2.0"))

    def test_heading_must_name_the_exact_version(self):
        for changelog in ("## 0.2.0.1\n- no\n", "## v0.2.0\n- no\n", "### 0.2.0\n- no\n", "## 0.2.0-rc.1\n- no\n", "## 0.2.00\n- no\n", "## 10.2.0\n- no\n"):
            with self.subTest(changelog=changelog):
                with self.assertRaisesRegex(ValueError, "no section for 0.2.0"):
                    NOTES.section(changelog, "v0.2.0")

    def test_empty_or_repeated_section_is_refused(self):
        with self.assertRaisesRegex(ValueError, "is empty"):
            NOTES.section("## 0.2.0\n\n## 0.1.0\n- old\n", "v0.2.0")
        with self.assertRaisesRegex(ValueError, "more than one section"):
            NOTES.section("## 0.2.0\n- a\n## 0.2.0 - again\n- b\n", "v0.2.0")

    def test_page_puts_notes_first_and_links_the_verification_steps(self):
        generated = "\n".join([
            "## Release Notes",
            "",
            "dist's copy of the notes",
            "",
            "## Install kashkeshet 0.2.0",
            "",
            "```sh",
            "curl -fsSL https://example.invalid/installer.sh | sh",
            "```",
            "",
            "## Download kashkeshet 0.2.0",
            "",
            "|  File  | Platform | Checksum |",
            "",
            "## Verifying GitHub Artifact Attestations",
            "",
            "The artifacts in this release have attestations generated with GitHub Artifact Attestations.",
            "```sh",
            "gh attestation verify <file-path of downloaded artifact> --repo sam-mfb/kashkeshet",
            "```",
            "",
        ])
        self.assertEqual(
            NOTES.page("v0.2.0", "### Agents\n- Something new.", generated),
            "\n".join([
                "### Agents",
                "- Something new.",
                "",
                "## Install kashkeshet 0.2.0",
                "",
                "```sh",
                "curl -fsSL https://example.invalid/installer.sh | sh",
                "```",
                "",
                "## Download kashkeshet 0.2.0",
                "",
                "|  File  | Platform | Checksum |",
                "",
                "To verify a download, see [Verify a download](https://github.com/sam-mfb/kashkeshet/blob/v0.2.0/docs/releases.md#verify-a-download).",
                "",
            ]),
        )

    def test_named_section_puts_its_name_in_a_heading_and_unnamed_does_not(self):
        self.assertEqual(NOTES.named(CHANGELOG, "v0.2.0"), "# v0.2.0: second\n\n### Agents\n- Something new.")
        self.assertEqual(NOTES.named(CHANGELOG, "v0.2.0-rc.1"), "# v0.2.0-rc.1: second\n\n### Agents\n- Something new.")
        self.assertEqual(NOTES.named(CHANGELOG, "v0.1.0"), "- The first release.")

    def test_named_section_heading_survives_dists_body_and_the_page_builder(self):
        # dist copies the changelog section into its own "Release Notes"; the page drops that copy.
        generated = "\n".join([
            "## Release Notes",
            "",
            "# v0.2.0: second",
            "",
            "dist's copy of the notes",
            "",
            "## Install kashkeshet 0.2.0",
            "",
            "steps",
            "",
        ])
        result = NOTES.page("v0.2.0", NOTES.named(CHANGELOG, "v0.2.0"), generated)
        self.assertTrue(result.startswith("# v0.2.0: second\n\n### Agents\n- Something new.\n\n## Install kashkeshet 0.2.0\n"))
        self.assertEqual(result.count("# v0.2.0: second"), 1)
        self.assertNotIn("dist's copy", result)

    def test_script_starts_the_published_page_with_the_releases_name(self):
        with scratch() as directory:
            generated = Path(directory) / "generated.txt"
            generated.write_text("## Release Notes\n\ncopy\n\n## Install kashkeshet 0.7.0\n\nsteps\n")
            result = subprocess.run(
                [sys.executable, str(SCRIPTS / "release-notes.py"), "v0.7.0", str(generated)],
                check=True, capture_output=True, text=True,
            )
        self.assertEqual(result.stdout.splitlines()[0], "# v0.7.0: bully badger")
        self.assertIn("## Install kashkeshet 0.7.0", result.stdout)

    def test_scratch_folders_are_under_target_tmp(self):
        with scratch() as directory:
            self.assertEqual(Path(directory).parent, ROOT / "target" / "tmp")
        self.assertFalse(Path(directory).exists())

    def test_page_keeps_a_generated_body_with_nothing_to_replace(self):
        generated = "## Install kashkeshet 0.2.0\n\nsteps\n"
        self.assertEqual(
            NOTES.page("v0.2.0-rc.1", "- notes", generated),
            "- notes\n\n## Install kashkeshet 0.2.0\n\nsteps\n\n"
            "To verify a download, see [Verify a download](https://github.com/sam-mfb/kashkeshet/blob/v0.2.0-rc.1/docs/releases.md#verify-a-download).\n",
        )


class ReleaseChecks(unittest.TestCase):
    def setUp(self):
        self.temp = scratch()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.target = "aarch64-unknown-linux-gnu"
        self.commit = "1234567890abcdef" * 2 + "12345678"
        output = self.root / "target" / self.target / "dist/build/kashkeshet-test/output"
        output.parent.mkdir(parents=True)
        output.write_text(f"cargo:rustc-env=KASHKESHET_GIT_COMMIT={self.commit}\n")
        self.output = output
        self.archive()

    def archive(self, commands=("kkt",), embedded=None):
        path = self.root / "target/distrib" / f"kashkeshet-{self.target}.tar.xz"
        path.parent.mkdir(exist_ok=True)
        with tarfile.open(path, "w:xz") as package:
            for command in commands:
                content = (embedded if embedded is not None else self.commit).encode()
                info = tarfile.TarInfo(f"kashkeshet-{self.target}/{command}")
                info.size = len(content)
                info.mode = 0o755
                package.addfile(info, io.BytesIO(content))

    def test_clean_archive_passes(self):
        CHECK.check(self.root, self.target, self.commit)

    def test_dirty_unknown_and_wrong_commits_fail(self):
        for recorded in (self.commit + "-dirty", "unknown", "f" * 40):
            with self.subTest(recorded=recorded):
                self.output.write_text(f"cargo:rustc-env=KASHKESHET_GIT_COMMIT={recorded}\n")
                with self.assertRaises(ValueError):
                    CHECK.check(self.root, self.target, self.commit)

    def test_missing_build_output_fails(self):
        self.output.unlink()
        with self.assertRaises(ValueError):
            CHECK.check(self.root, self.target, self.commit)

    def test_missing_kkt_fails(self):
        self.archive(commands=("kashkeshet",))
        with self.assertRaises(KeyError):
            CHECK.check(self.root, self.target, self.commit)

    def test_archive_from_another_build_fails(self):
        self.archive(embedded="unknown")
        with self.assertRaises(ValueError):
            CHECK.check(self.root, self.target, self.commit)

    def test_dirty_binary_fails_even_with_clean_build_output(self):
        self.archive(embedded=self.commit + "-dirty")
        with self.assertRaisesRegex(ValueError, "binary records a dirty"):
            CHECK.check(self.root, self.target, self.commit)

    def test_cargo_wrapper_locks_builds_and_preserves_arguments(self):
        cargo = self.root / "real-cargo"
        cargo.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
        cargo.chmod(0o700)
        env = dict(os.environ, KKT_RELEASE_CARGO=str(cargo))
        for args, expected in [
            (["build", "--target", self.target], ["build", "--target", self.target, "--locked"]),
            (["metadata", "--format-version=1"], ["metadata", "--format-version=1", "--locked"]),
            (["fetch"], ["fetch", "--locked"]),
            (["--version"], ["--version"]),
        ]:
            with self.subTest(args=args):
                result = subprocess.check_output(
                    ["sh", str(SCRIPTS / "cargo-locked.sh"), *args], env=env, text=True
                )
                self.assertEqual(result.splitlines(), expected)


if __name__ == "__main__":
    unittest.main()
