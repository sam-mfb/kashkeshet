#!/usr/bin/env python3
"""Accept stable/RC tags only, with an exact Cargo package version match and release notes."""

import importlib.util
from pathlib import Path
import re
import sys
import tomllib

NUMBER = r"(?:0|[1-9][0-9]*)"
TAG = re.compile(rf"v{NUMBER}\.{NUMBER}\.{NUMBER}(?:-rc\.{NUMBER})?")
NOTES_SPEC = importlib.util.spec_from_file_location("release_notes", Path(__file__).resolve().parent / "release-notes.py")
NOTES = importlib.util.module_from_spec(NOTES_SPEC)
NOTES_SPEC.loader.exec_module(NOTES)


def check(tag, manifest, changelog):
    if not TAG.fullmatch(tag):
        raise ValueError("release tag must be vX.Y.Z or vX.Y.Z-rc.N (no leading zeros)")
    package = tomllib.loads(manifest.read_text())["package"]
    version = package["version"]
    if tag != f"v{version}":
        raise ValueError(f"tag {tag} does not match Cargo package version {version}; commit the matching Cargo.toml and Cargo.lock before tagging")
    NOTES.section(changelog.read_text(), tag)
    return "-rc." in version


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[2]
    try:
        if len(sys.argv) != 2:
            raise ValueError("usage: check-release-tag.py vX.Y.Z[-rc.N]")
        candidate = check(sys.argv[1], root / "Cargo.toml", root / "CHANGELOG.md")
    except (ValueError, KeyError, OSError) as error:
        sys.exit(str(error))
    print(f"Validated {'release-candidate' if candidate else 'stable'} tag {sys.argv[1]} against Cargo.toml and CHANGELOG.md")
