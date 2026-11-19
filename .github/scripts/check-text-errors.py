#!/usr/bin/env python3
"""Errors are typed (RUST.md 8): outside tests, no function's error is
text. This check fails on a function in the Rust files git tracks whose
signature has a `Result` whose error is `String`, or a pair ending in
`String`, as in a closure parameter, unless it is one of the exceptions
below. It also fails on an exception that is no longer found, so the list
stays exact. CLAUDE.md gives each exception's reason.

Test code is left out: files named tests.rs, testing.rs or *_tests.rs,
folders named tests or testing, and `#[cfg(test)]` items and modules,
with the files of a module declared `#[cfg(test)] mod x;`.

Usage: check-text-errors.py [ROOT]   (the workspace root; default: here)
Tested by test_check_text_errors.py.
"""

import re
import subprocess
import sys
from pathlib import Path

# The functions whose error is text by an approved exception, by file.
EXCEPTIONS = {
    # The development-only programs, never released.
    "programs/kkt-keymode/src/main.rs": {
        "parse_args", "run", "settings", "open", "access", "web", "admin", "admin_names",
    },
    "programs/kkt-fake-product/src/main.rs": {"run"},
    "programs/kkt-webapp-preview/src/main.rs": {"run"},
    # kkt-mcp's build script: its error stops the build.
    "crates/kkt-mcp/build.rs": {"write_tool_text", "tool_texts", "tool_text"},
    # Plain HTTP for a program's test client.
    "crates/kkt-agent-client-mcp/src/client/http.rs": {
        "new", "get", "post", "text", "bytes_stream",
    },
    # Private helpers whose text is a status line's words, not a failure.
    "crates/kkt-containerized-service/src/status.rs": {"name_pack", "config_path_text", "flag"},
}

FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
RESULT = re.compile(r"\bResult\s*<")
CFG_TEST = re.compile(r"#\[\s*cfg\s*\((?![^\]]*\bnot\s*\()[^\]]*\btest\b[^\]]*\)\s*\]")
MOD_DECLARED = re.compile(r"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")


def code_only(text):
    """`text` with comments and the insides of strings and characters made
    blank, keeping every newline, so marks in them are not read as code."""
    out, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            end = text.find("\n", i)
            end = n if end == -1 else end
            out.append(" " * (end - i))
            i = end
        elif text.startswith("/*", i):
            end = text.find("*/", i + 2)
            end = n if end == -1 else end + 2
            out.append("".join("\n" if ch == "\n" else " " for ch in text[i:end]))
            i = end
        elif c == "r" and re.match(r'r#*"', text[i:i + 10]) and not (i and (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = re.match(r'r(#*)"', text[i:]).group(1)
            close = '"' + hashes
            end = text.find(close, i + 2 + len(hashes))
            end = n if end == -1 else end + len(close)
            out.append('""' + "".join("\n" if ch == "\n" else " " for ch in text[i + 2:end]))
            i = end
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append('"' + "".join("\n" if ch == "\n" else " " for ch in text[i + 1:j]) + '"')
            i = j + 1
        elif c == "'" and re.match(r"'(\\.|[^\\'])'", text[i:i + 4]):
            length = len(re.match(r"'(\\.|[^\\'])'", text[i:i + 4]).group(0))
            out.append(" " * length)
            i += length
        else:
            out.append(c)
            i += 1
    return "".join(out)


def item_end(code, start):
    """The index just past the item that starts at `start`: at its `;`, or
    at the brace that closes its body, whichever comes first."""
    depth, i = 0, start
    while i < len(code):
        c = code[i]
        if c in "([":
            depth += 1
        elif c in ")]":
            depth -= 1
        elif c == ";" and depth == 0:
            return i + 1
        elif c == "{" and depth == 0:
            level = 0
            while i < len(code):
                if code[i] == "{":
                    level += 1
                elif code[i] == "}":
                    level -= 1
                    if level == 0:
                        return i + 1
                i += 1
            return len(code)
        i += 1
    return len(code)


def without_tests(code):
    """`code` with each `#[cfg(test)]` item made blank, and the names of the
    modules declared `#[cfg(test)] mod x;`."""
    modules = []
    while True:
        found = CFG_TEST.search(code)
        if not found:
            return code, modules
        start = found.start()
        rest = found.end()
        # Further attributes on the same item.
        while True:
            more = re.match(r"\s*#\[", code[rest:])
            if not more:
                break
            bracket = rest + more.end() - 1
            depth, j = 0, bracket
            while j < len(code):
                if code[j] == "[":
                    depth += 1
                elif code[j] == "]":
                    depth -= 1
                    if depth == 0:
                        break
                j += 1
            rest = j + 1
        declared = MOD_DECLARED.match(code[rest:])
        if declared:
            modules.append(declared.group(1))
        end = item_end(code, rest)
        code = code[:start] + "".join("\n" if ch == "\n" else " " for ch in code[start:end]) + code[end:]


def generic_args(code, opening):
    """The top-level arguments of the generic list whose `<` is at
    `opening`, as text."""
    args, depth, i, begun = [], 0, opening, opening + 1
    while i < len(code):
        if code.startswith("->", i):
            i += 2
            continue
        c = code[i]
        if c in "<([":
            depth += 1
        elif c in ">)]":
            depth -= 1
            if depth == 0:
                args.append(code[begun:i])
                # A trailing comma, as rustfmt writes across lines, adds none.
                return [a.strip() for a in args if a.strip()]
        elif c == "," and depth == 1:
            args.append(code[begun:i])
            begun = i + 1
        i += 1
    return [a.strip() for a in args]


def is_text(error):
    """Whether an error type, as written, is `String` or a pair ending in it."""
    error = " ".join(error.split())
    if error in ("String", "std::string::String"):
        return True
    if error.startswith("(") and error.endswith(")"):
        inner = error[1:-1].rstrip(", ")
        return inner.rsplit(",", 1)[-1].strip() in ("String", "std::string::String")
    return False


def text_errors(text):
    """The names of the functions in one file's text whose signature has a
    `Result` with a text error, outside `#[cfg(test)]` items; and the
    modules it declares `#[cfg(test)]`."""
    code, modules = without_tests(code_only(text))
    found = []
    for fn in FN.finditer(code):
        end = item_end(code, fn.end())
        signature = code[fn.end():end]
        body = signature.find("{")
        if body != -1:
            signature = signature[:body]
        for result in RESULT.finditer(signature):
            args = generic_args(signature, result.end() - 1)
            if len(args) >= 2 and is_text(args[-1]):
                found.append(fn.group(1))
                break
    return found, modules


def is_test_path(relative):
    parts = Path(relative).parts
    name = parts[-1]
    if name in ("tests.rs", "testing.rs") or name.endswith("_tests.rs"):
        return True
    return any(part in ("tests", "testing") for part in parts[:-1])


def module_paths(relative, name):
    """Where a module `name` declared in the file `relative` may be."""
    path = Path(relative)
    folder = path.parent if path.name in ("lib.rs", "main.rs", "mod.rs", "build.rs") else path.parent / path.stem
    return [folder / f"{name}.rs", folder / name]


def rust_files(root):
    listed = subprocess.run(["git", "-C", str(root), "ls-files", "*.rs"],
                            capture_output=True, text=True, check=True).stdout.split("\n")
    return sorted(name for name in listed if name)


def scan(files, read):
    """Each (file, function) with a text error, in the files given, reading
    each with `read`, leaving out test files and test modules."""
    test_modules, results = [], {}
    for relative in files:
        if is_test_path(relative):
            continue
        names, modules = text_errors(read(relative))
        results[relative] = names
        for module in modules:
            test_modules += module_paths(relative, module)
    found = []
    for relative, names in results.items():
        path = Path(relative)
        if any(path == m or m in path.parents for m in test_modules):
            continue
        found += [(relative, name) for name in names]
    return found


def problems(found, exceptions):
    """The text errors not excepted, and the exceptions not found."""
    out = []
    for relative, name in found:
        if name not in exceptions.get(relative, set()):
            out.append(f"{relative}: fn {name} returns a text error; give it an error type (RUST.md 8)")
    seen = set(found)
    for relative, names in sorted(exceptions.items()):
        for name in sorted(names):
            if (relative, name) not in seen:
                out.append(f"{relative}: fn {name} is listed as an exception but returns no text "
                           "error; take it off the list")
    return out


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    found = scan(rust_files(root), lambda relative: (root / relative).read_text())
    listed = problems(found, EXCEPTIONS)
    if listed:
        print("check-text-errors: errors are typed outside tests, but for the listed exceptions:",
              file=sys.stderr)
        for line in listed:
            print(line, file=sys.stderr)
        sys.exit(1)
    count = sum(len(names) for names in EXCEPTIONS.values())
    print(f"check-text-errors: no text error outside tests but the {count} listed exceptions")


if __name__ == "__main__":
    main()
