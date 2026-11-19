#!/usr/bin/env python3
"""A lint exception is `#[expect(..., reason = "...")]`, never `#[allow]`
(RUST.md 9): an `expect` fails once the lint no longer fires, so a stale
exception cannot stay. This check fails on any `allow` attribute in the
Rust files git tracks, `#[allow(..)]`, `#![allow(..)]` and an `allow`
inside `cfg_attr`, and on an `expect` without a reason, wherever it is in
the attribute. Each attribute is read whole, across lines, as rustfmt may
split it.

Usage: check-lint-exceptions.py [ROOT]   (the workspace root; default: here)
Tested by test_check_lint_exceptions.py.
"""

import re
import subprocess
import sys
from pathlib import Path

ALLOW = re.compile(r"\ballow\s*\(")
EXPECT = re.compile(r"\bexpect\s*\(")
ATTRIBUTE_START = re.compile(r"#!?\[")


def closing(text, start, open_mark, close_mark):
    """The index just past the mark that closes the one at `start`, skipping
    strings; the end of `text` if it never closes."""
    depth, i = 0, start
    while i < len(text):
        c = text[i]
        if c == '"':
            i += 1
            while i < len(text) and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
        elif c == open_mark:
            depth += 1
        elif c == close_mark:
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return len(text)


def attributes(text):
    """Each attribute in `text`, whole, across lines: (its first line, its
    text). Comment lines are left out, so an example in a doc is not read."""
    code = "\n".join("" if line.lstrip().startswith("//") else line for line in text.split("\n"))
    for found in ATTRIBUTE_START.finditer(code):
        bracket = found.end() - 1
        yield code.count("\n", 0, found.start()) + 1, code[found.start():closing(code, bracket, "[", "]")]


def problems_in(text, where):
    """The allow attributes and reasonless expects in one file's text."""
    found = []
    for number, attribute in attributes(text):
        shown = " ".join(attribute.split())[:80]
        if ALLOW.search(attribute):
            found.append(f"{where}:{number}: use #[expect(.., reason = \"..\")], not allow: {shown}")
        for expect in EXPECT.finditer(attribute):
            arguments = attribute[expect.end() - 1:closing(attribute, expect.end() - 1, "(", ")")]
            if not re.search(r"\breason\s*=", arguments):
                found.append(f"{where}:{number}: an expect gives no reason: {shown}")
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
