#!/usr/bin/env python3
"""A lint exception is `#[expect(..., reason = "...")]`, never `#[allow]`
(RUST.md 9): an `expect` fails once the lint no longer fires, so a stale
exception cannot stay. This check fails on any `allow` attribute in the
Rust files git tracks, `#[allow(..)]`, `#![allow(..)]` and an `allow`
inside `cfg_attr`, and on an `expect` without a reason.

Usage: check-lint-exceptions.py [ROOT]   (the workspace root; default: here)
Tested by test_check_lint_exceptions.py.
"""

import re
import subprocess
import sys
from pathlib import Path

ALLOW = re.compile(r"#!?\[\s*(?:allow\s*\(|cfg_attr\s*\(.*\ballow\s*\()")
# An `expect` attribute, up to its closing `)]`, which may span lines.
EXPECT = re.compile(r"#!?\[\s*expect\s*\((.*?)\)\s*\]", re.DOTALL)


def problems_in(text, where):
    """The allow attributes and reasonless expects in one file's text."""
    found = []
    for number, line in enumerate(text.split("\n"), start=1):
        if ALLOW.search(line) and not line.lstrip().startswith("//"):
            found.append(f"{where}:{number}: use #[expect(.., reason = \"..\")], not allow: {line.strip()[:80]}")
    for attribute in EXPECT.finditer(text):
        if not re.search(r"\breason\s*=", attribute.group(1)):
            number = text.count("\n", 0, attribute.start()) + 1
            found.append(f"{where}:{number}: an expect gives no reason: {attribute.group(0).strip()[:80]}")
    return found


def rust_files(root):
    listed = subprocess.run(["git", "-C", str(root), "ls-files", "*.rs"],
                            capture_output=True, text=True, check=True).stdout.split("\n")
    return [root / name for name in listed if name]


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    found = []
    for path in rust_files(root):
        found += problems_in(path.read_text(), str(path.relative_to(root)))
    if found:
        print("check-lint-exceptions: a lint exception must be #[expect(.., reason = \"..\")]:",
              file=sys.stderr)
        for line in found:
            print(line, file=sys.stderr)
        sys.exit(1)
    print("check-lint-exceptions: every lint exception is an expect with a reason")


if __name__ == "__main__":
    main()
