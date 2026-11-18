#!/usr/bin/env python3
"""A platform crate knows nothing of kkt, in what it says as well as in what
it depends on. This check reads the platform crates from
docs/architecture/concerns.json, so a crate that joins the layer is covered
without an edit here, and fails when one's Rust source outside its tests has
`kkt` or `kashkeshet` as a word in a string literal, in the name of a
public item, or in a doc comment (`///` or `//!`). In a doc comment, a
crate's name (`kkt-log`, `kkt_log::Log`) is allowed: a doc link needs it.
Ordinary comments and crate names in code paths are not read.
Tests are left out: files named tests.rs, folders named tests, and modules
under `#[cfg(test)]`.

It also reads the crate descriptions and layer texts concerns.json gives
the map: a platform crate's description, and the platform layer's text,
must not say kkt or kashkeshet, other crates' names included, and none may
carry the plainest marks of history (HISTORY) or the plainest negatives
(NEGATIVE). A crate's description is one to three short sentences, so one
longer than MAX_DESCRIPTION characters fails.

Usage: check-platform-words.py [ROOT]   (the workspace root; default: here)
Tested by test_check_platform_words.py.
"""

import json
import re
import sys
from pathlib import Path

WORD = re.compile(r"\b(kkt|kashkeshet)\b", re.IGNORECASE)
STRING = re.compile(r'r(#*)"(.*?)"\1|"((?:[^"\\]|\\.)*)"', re.DOTALL)
PUBLIC_NAME = re.compile(
    r"\bpub(?:\([^)]*\))?\s+(?:const\s+fn|async\s+fn|unsafe\s+fn|fn|struct|enum|const|static|trait|type|mod)\s+(\w+)"
)
# In a doc comment: kkt or kashkeshet as a word, but not as the start of a
# crate's name, such as `kkt-log` or `kkt_log::Log`.
DOC_WORD = re.compile(r"\b(kkt|kashkeshet)\b(?![-_])", re.IGNORECASE)
DOC_LINE = re.compile(r"^\s*//[/!](.*)$")
HISTORY = re.compile(r"\bwas kkt-|\bformerly\b|ARCHITECTURE\.md|RUST\.md|#\d+|\bphase \d+", re.IGNORECASE)
# A description says what a crate is and does, not what it is not.
NEGATIVE = re.compile(r"\bknows? nothing\b|\bnever\b|\bdoes not\b|\bis not\b|\bdoesn't\b|\bisn't\b", re.IGNORECASE)
# One to three short sentences: room for the longest descriptions that keep
# the rules, with a margin.
MAX_DESCRIPTION = 360
TEST_MODULE = re.compile(r"#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{")


def platform_crates(root):
    """The packages concerns.json puts in the platform layer."""
    concerns = json.loads((root / "docs/architecture/concerns.json").read_text())
    return sorted(name for name, crate in concerns["crates"].items() if crate["layer"] == "platform")


def without_comments(code):
    """`code` with its comments blanked, keeping strings and line numbers."""
    out, i, n = [], 0, len(code)
    while i < n:
        string = STRING.match(code, i) if code[i] in 'r"' else None
        if string and (code[i] == '"' or (i == 0 or not (code[i - 1].isalnum() or code[i - 1] == "_"))):
            out.append(string.group(0))
            i = string.end()
        elif code.startswith("//", i):
            end = code.find("\n", i)
            end = n if end == -1 else end
            i = end
        elif code.startswith("/*", i):
            end = code.find("*/", i + 2)
            end = n if end == -1 else end + 2
            out.append("\n" * code.count("\n", i, end))
            i = end
        elif code[i] == "'" and i + 2 < n and code[i + 2] == "'":
            # A character literal such as '"', which must not open a string.
            out.append(code[i:i + 3])
            i += 3
        elif code[i] == "'" and i + 3 < n and code[i + 1] == "\\" and code[i + 3] == "'":
            # An escaped one, such as '\"'.
            out.append(code[i:i + 4])
            i += 4
        else:
            out.append(code[i])
            i += 1
    return "".join(out)


def without_test_modules(code):
    """`code` with each `#[cfg(test)] mod … { … }` block blanked."""
    while True:
        found = TEST_MODULE.search(code)
        if not found:
            return code
        depth, i = 1, found.end()
        while i < len(code) and depth:
            depth += {"{": 1, "}": -1}.get(code[i], 0)
            i += 1
        code = code[:found.start()] + "\n" * code.count("\n", found.start(), i) + code[i:]


def is_test_file(path, src):
    relative = path.relative_to(src)
    return path.name == "tests.rs" or "tests" in relative.parts[:-1]


def problems_in(text, where):
    """The kkt words in one file's code: `where:line: …` for each."""
    code = without_test_modules(without_comments(text))
    found = []
    for string in STRING.finditer(code):
        body = string.group(2) if string.group(2) is not None else string.group(3)
        if WORD.search(body):
            line = code.count("\n", 0, string.start()) + 1
            found.append(f"{where}:{line}: a string names kkt: {string.group(0)[:80]}")
    for name in PUBLIC_NAME.finditer(code):
        if re.search(r"kkt|kashkeshet", name.group(1), re.IGNORECASE):
            line = code.count("\n", 0, name.start()) + 1
            found.append(f"{where}:{line}: a public item's name names kkt: {name.group(1)}")
    for number, line in enumerate(without_test_modules(text).split("\n"), start=1):
        doc = DOC_LINE.match(line)
        if doc and DOC_WORD.search(doc.group(1)):
            found.append(f"{where}:{number}: a doc comment names kkt: {line.strip()[:80]}")
    return found


def description_problems(root):
    """The crate descriptions that break the map's rules: kkt's words in a
    platform crate's, and history, negatives or too many words in any."""
    concerns = json.loads((root / "docs/architecture/concerns.json").read_text())
    found = []
    for name, crate in sorted(concerns["crates"].items()):
        text = crate.get("concern", "")
        if crate["layer"] == "platform" and WORD.search(text):
            found.append(f"concerns.json: {name}'s description names kkt: {WORD.search(text).group(0)}")
        history = HISTORY.search(text)
        if history:
            found.append(f"concerns.json: {name}'s description gives history: {history.group(0)}")
        negative = NEGATIVE.search(text)
        if negative:
            found.append(f"concerns.json: {name}'s description says what it is not: {negative.group(0)}")
        if len(text) > MAX_DESCRIPTION:
            found.append(f"concerns.json: {name}'s description is {len(text)} characters, "
                         f"more than {MAX_DESCRIPTION}: say it in one to three short sentences")
    # The layers' texts keep to the same rules.
    for layer in concerns.get("layers", []):
        text = layer.get("about", "")
        if layer["id"] == "platform" and WORD.search(text):
            found.append(f"concerns.json: the platform layer's text names kkt: {WORD.search(text).group(0)}")
        if HISTORY.search(text):
            found.append(f"concerns.json: the {layer['id']} layer's text gives history: {HISTORY.search(text).group(0)}")
        if NEGATIVE.search(text):
            found.append(f"concerns.json: the {layer['id']} layer's text says what it is not: {NEGATIVE.search(text).group(0)}")
    return found


def problems(root):
    found = description_problems(root)
    for crate in platform_crates(root):
        src = root / "crates" / crate / "src"
        for path in sorted(src.rglob("*.rs")):
            if is_test_file(path, src):
                continue
            found += problems_in(path.read_text(), str(path.relative_to(root)))
    return found


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    found = problems(root)
    if found:
        print("check-platform-words: a platform crate names kkt (it must know nothing of kkt, "
              "so the caller passes kkt's words in), or a crate's map description says kkt "
              "where it may not, gives history, says what it is not, or runs long:", file=sys.stderr)
        for line in found:
            print(line, file=sys.stderr)
        sys.exit(1)
    print("check-platform-words: no platform crate names kkt, and the map's descriptions keep to their rules")


if __name__ == "__main__":
    main()
