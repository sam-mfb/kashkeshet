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

A platform crate's words are its caller's in another way too: a string in
one may name a program or service kkt works with (TOOL) only when the
crate's map description names it, because the crate is for that tool. A
login prompt for Tailscale in the terminal crate fails; `docker compose` in
the Docker crate passes. Test support (files named testing.rs and folders
named testing) is left out of this rule as well, since no person sees it.

It also reads the crate descriptions and layer texts concerns.json gives
the map: a platform crate's description, and the platform layer's text,
must not say kkt or kashkeshet, other crates' names included, and none may
carry the plainest marks of history (HISTORY) or the plainest negatives
(NEGATIVE). A crate's description is one to three short sentences, so one
longer than MAX_DESCRIPTION characters fails.

Each crate's header, the `//!` lines that open its lib.rs or main.rs (for the
root package, its src/lib.rs and each binary's main.rs), keeps
to the same rules: no line of it carries the marks of history, and its
first sentence, which the map shows, says no plain negative. The rest of
a header may say what its code does not do, such as a refusal a rule
makes.

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
HISTORY = re.compile(r"\bwas kkt-|\bformerly\b|ARCHITECTURE\.md|RUST\.md|#\d+|\bphase \d+|"
                     r"\bas before\b|\bused to\b|\bno longer\b|\bestablished\b", re.IGNORECASE)
# A description says what a crate is and does, not what it is not.
NEGATIVE = re.compile(r"\bnot\b|\bnever\b|\bnothing\b|\bcannot\b|n['’]t\b", re.IGNORECASE)
# One to three short sentences: room for the longest descriptions that keep
# the rules, with a margin.
MAX_DESCRIPTION = 360
# The programs and services kkt works with, as a person would see them named.
TOOL = re.compile(
    r"\b(tailscale|docker|compose|podman|launchd|launchctl|systemd|systemctl|journalctl|claude|"
    r"codex|opencode|antigravity|muse code|git|sqlite|xdg-open)\b",
    re.IGNORECASE,
)
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


def is_test_support(path, src):
    relative = path.relative_to(src)
    return path.name == "testing.rs" or "testing" in relative.parts[:-1]


def tool_problems_in(text, where, own):
    """The tools named in one file's strings that are not among `own`, the
    tools its crate's description names: `where:line: …` for each."""
    code = without_test_modules(without_comments(text))
    found = []
    for string in STRING.finditer(code):
        body = string.group(2) if string.group(2) is not None else string.group(3)
        for tool in sorted({t.lower() for t in TOOL.findall(body)} - own):
            line = code.count("\n", 0, string.start()) + 1
            found.append(f"{where}:{line}: a string names {tool}, which the crate's description does not: "
                         f"{string.group(0)[:80]}")
    return found


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


def header(text):
    """The `//!` lines that open a file, without the marks, up to the first
    line of code."""
    lines = []
    for line in text.split("\n"):
        if line.startswith("//!"):
            lines.append(line[3:].strip())
        elif line.strip() and not line.startswith("#!"):
            break
        elif lines and not line.strip():
            break
    return lines


def first_sentence(lines):
    """The first sentence of a header's first paragraph, as the map shows it."""
    paragraph = []
    for line in lines:
        if not line:
            break
        paragraph.append(line)
    text = " ".join(paragraph)
    match = re.match(r"(.+?[.!?])(\s|$)", text)
    return match.group(1) if match else text


def header_problems(root):
    """History in any crate's header, and a negative in its first sentence."""
    concerns = json.loads((root / "docs/architecture/concerns.json").read_text())
    found = []
    for name in sorted(concerns["crates"]):
        places = [root / "crates" / name / "src/lib.rs", root / "crates" / name / "src/main.rs",
                  root / "programs" / name / "src/lib.rs", root / "programs" / name / "src/main.rs"]
        if crate_at_root(root) == name:
            # The root package: its library and its binaries.
            places = [root / "src/lib.rs"] + sorted((root / "src/bin").glob("*/main.rs"))
        for path in [p for p in places if p.exists()]:
            lines = header(path.read_text())
            where = str(path.relative_to(root))
            for number, line in enumerate(lines, start=1):
                history = HISTORY.search(line)
                if history:
                    found.append(f"{where}:{number}: the crate's header gives history: {history.group(0)}")
            negative = NEGATIVE.search(first_sentence(lines))
            if negative:
                found.append(f"{where}: the crate's header begins with what it is not: {negative.group(0)}")
    return found


def crate_at_root(root):
    """The name of the package at the workspace's root, if it has one."""
    manifest = root / "Cargo.toml"
    if not manifest.exists():
        return None
    found = re.search(r'^\[package\][^\[]*?^name\s*=\s*"([^"]+)"', manifest.read_text(), re.MULTILINE | re.DOTALL)
    return found.group(1) if found else None


def problems(root):
    found = description_problems(root) + header_problems(root)
    concerns = json.loads((root / "docs/architecture/concerns.json").read_text())
    for crate in platform_crates(root):
        own = {t.lower() for t in TOOL.findall(concerns["crates"][crate].get("concern", ""))}
        src = root / "crates" / crate / "src"
        for path in sorted(src.rglob("*.rs")):
            if is_test_file(path, src):
                continue
            text, where = path.read_text(), str(path.relative_to(root))
            found += problems_in(text, where)
            if not is_test_support(path, src):
                found += tool_problems_in(text, where, own)
    return found


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    found = problems(root)
    if found:
        print("check-platform-words: a platform crate names kkt (it must know nothing of kkt, "
              "so the caller passes kkt's words in), or a tool its description does not name, "
              "or a crate's map description or header says kkt where it may not, gives history, "
              "says what it is not, or runs long:", file=sys.stderr)
        for line in found:
            print(line, file=sys.stderr)
        sys.exit(1)
    print("check-platform-words: no platform crate names kkt, and the map's descriptions keep to their rules")


if __name__ == "__main__":
    main()
