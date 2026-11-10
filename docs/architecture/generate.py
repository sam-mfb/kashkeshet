#!/usr/bin/env python3
"""Regenerates the architecture map's data, `map-data.js`, from the code.

Run from anywhere in the repository: `python3 docs/architecture/generate.py`.
It needs cargo and the pinned toolchain, and builds rustdoc's JSON with
`RUSTC_BOOTSTRAP=1` (rustdoc's JSON output is not yet stable), into a
target folder of its own under `target/`.

Where each part of the map comes from:

- The crates and the arrows between them: `cargo metadata`. An arrow is a
  dependency of one workspace package on another, of kind normal, dev or
  build, with the features it turns on.
- What a crate provides: rustdoc's JSON of its library, which lists only
  public items: modules, types, traits, functions, constants and macros,
  with trait methods, public inherent methods, enum variants and the first
  sentence of each item's documentation, and structs' public fields.
  Methods, variants or fields a feature adds to an item are marked with it. It is built once with the
  crate's default features and once for each of its features, so an item
  that only a feature makes is marked with that feature.
- What flows along an arrow: the paths the consumer's source names in the
  dependency (`use` statements and `crate::path` uses, through aliases),
  matched to the dependency's public items. Uses in test code (`tests/`,
  `examples/`, `*tests.rs`, `#[cfg(test)]` modules and files) count for
  the dev arrow, uses in `build.rs` for the build arrow. A crate that is
  only a dev-dependency can only be used in test code, so its arrow takes
  every use. This reads names, not
  types: an item used only through a method call, a trait's methods or a
  derive does not show.
- Each crate's concern, its layer and what it must not do: written by
  hand in `concerns.json`, from CLAUDE.md and the crates' own docs.
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
TARGET = ROOT / "target" / "architecture-map"
ITEM_KINDS = {
    "struct": "struct",
    "enum": "enum",
    "trait": "trait",
    "function": "fn",
    "type_alias": "type",
    "constant": "const",
    "static": "static",
    "macro": "macro",
    "proc_macro": "macro",
    "union": "union",
    "module": "mod",
}


def run(args, env=None):
    result = subprocess.run(
        args, cwd=ROOT, env=env, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        sys.exit(f"{' '.join(args)} failed:\n{result.stderr}")
    return result.stdout


def metadata():
    return json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked", "--no-deps"]))


def summary(docs):
    """The first sentence of `docs`, as plain text."""
    if not docs:
        return ""
    para = docs.strip().split("\n\n", 1)[0].replace("\n", " ")
    para = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", para)
    para = re.sub(r"\[`([^`]+)`\]", r"`\1`", para)
    para = re.sub(r"\[([^\]]+)\]", r"\1", para)
    match = re.match(r"(.+?[.!?])(\s|$)", para)
    text = match.group(1) if match else para
    return text if len(text) <= 240 else text[:237] + "..."


def rustdoc(package, lib, feature):
    """rustdoc's JSON for `package`'s library, with `feature` turned on."""
    env = dict(os.environ, RUSTC_BOOTSTRAP="1")
    args = ["cargo", "rustdoc", "-q", "--locked", "-p", package, "--lib",
            "--target-dir", str(TARGET)]
    if feature:
        args += ["--features", feature]
    args += ["--", "-Z", "unstable-options", "--output-format", "json"]
    run(args, env)
    return json.loads((TARGET / "doc" / f"{lib}.json").read_text())


def public_items(doc):
    """Every public item of the crate in `doc`, by its path in the crate."""
    index = doc["index"]
    items = {}

    def kind_of(item):
        return next(iter(item["inner"]))

    def methods(item, kind):
        inner = item["inner"][kind]
        names = []
        if kind == "trait":
            for i in inner["items"]:
                sub = index.get(str(i))
                if sub and kind_of(sub) == "function":
                    names.append(sub["name"])
            return names
        for impl_id in inner.get("impls", []):
            impl = index.get(str(impl_id))
            if not impl or impl["inner"]["impl"].get("trait") is not None:
                continue
            for i in impl["inner"]["impl"]["items"]:
                sub = index.get(str(i))
                if sub and kind_of(sub) == "function" and sub.get("visibility") == "public":
                    names.append(sub["name"])
        return names

    def record(path, item, reexport=None):
        # Name where a re-export comes from only when that is another
        # crate: a path inside this crate names a private module.
        if reexport and item.get("crate_id") == 0:
            reexport = None
        kind = kind_of(item)
        entry = {
            "path": path,
            "kind": ITEM_KINDS.get(kind, kind),
            "doc": summary(item.get("docs")),
        }
        if kind in ("struct", "enum", "trait"):
            names = methods(item, kind)
            if names:
                entry["methods"] = sorted(set(names))
        if kind == "struct":
            shape = item["inner"]["struct"]["kind"]
            if isinstance(shape, dict) and "plain" in shape:
                fields = [index[str(f)]["name"] for f in shape["plain"]["fields"]
                          if str(f) in index and index[str(f)].get("visibility") == "public"]
                if fields:
                    entry["fields"] = fields
        if kind == "enum":
            entry["variants"] = [index[str(v)]["name"] for v in item["inner"]["enum"]["variants"]
                                 if str(v) in index]
        if reexport:
            entry["from"] = reexport
        items[path] = entry

    def walk(module_id, prefix, seen):
        if module_id in seen:
            return
        seen.add(module_id)
        module = index[str(module_id)]
        for child_id in module["inner"]["module"]["items"]:
            child = index.get(str(child_id))
            if child is None or child.get("visibility") not in ("public", None):
                continue
            kind = kind_of(child)
            if kind == "use":
                use = child["inner"]["use"]
                if use["is_glob"]:
                    target = index.get(str(use["id"])) if use["id"] is not None else None
                    if target and kind_of(target) == "module":
                        walk(use["id"], prefix, seen)
                    continue
                path = prefix + use["name"]
                target = index.get(str(use["id"])) if use["id"] is not None else None
                if target is not None:
                    if kind_of(target) == "module":
                        items[path] = {"path": path, "kind": "mod",
                                       "doc": summary(target.get("docs"))}
                        if target.get("crate_id") != 0:
                            items[path]["from"] = use["source"]
                        walk(use["id"], path + "::", seen)
                    else:
                        record(path, target, reexport=use["source"])
                else:
                    external = doc["paths"].get(str(use["id"]), {})
                    items[path] = {"path": path, "kind": ITEM_KINDS.get(external.get("kind"), external.get("kind", "item")),
                                   "doc": "", "from": use["source"]}
                continue
            if kind in ("impl",):
                continue
            path = prefix + child["name"]
            if kind == "module":
                items[path] = {"path": path, "kind": "mod", "doc": summary(child.get("docs"))}
                walk(child_id, path + "::", seen)
            elif kind in ITEM_KINDS:
                record(path, child)

    walk(doc["root"], "", set())
    crate_doc = summary(index[str(doc["root"])].get("docs"))
    return crate_doc, items


# ---- What flows along an arrow ----


def rust_files(package_dir, exclude):
    for path in sorted(package_dir.rglob("*.rs")):
        if any(part in ("target",) for part in path.parts):
            continue
        if any(path.is_relative_to(other) for other in exclude):
            continue
        yield path


def test_module_files(package_dir, files):
    """Files declared as `#[cfg(test)] mod name;`: test code."""
    found = set()
    for path in files:
        text = path.read_text()
        for match in re.finditer(r"#\[cfg\(test\)\]\s*mod\s+(\w+)\s*;", text):
            name = match.group(1)
            base = path.parent if path.name in ("lib.rs", "main.rs", "mod.rs") else path.parent / path.stem
            if (base / f"{name}.rs").exists():
                found.add(base / f"{name}.rs")
            # A test module's folder is test code, every file in it.
            if (base / name).is_dir():
                found.update((base / name).rglob("*.rs"))
    return found


def file_kind(package_dir, path, test_files):
    if path in test_files:
        return "dev"
    rel = path.relative_to(package_dir)
    if rel.name == "build.rs" and len(rel.parts) == 1:
        return "build"
    if rel.parts[0] in ("tests", "examples", "benches"):
        return "dev"
    if rel.name == "tests.rs" or rel.name.endswith("_tests.rs"):
        return "dev"
    return "normal"


def split_test_modules(text):
    """`text` without its `#[cfg(test)]` modules, and those modules' text."""
    normal, tests = [], []
    pos = 0
    for match in re.finditer(r"#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{", text):
        if match.start() < pos:
            continue
        depth, i = 1, match.end()
        while i < len(text) and depth:
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
            i += 1
        normal.append(text[pos:match.start()])
        tests.append(text[match.start():i])
        pos = i
    normal.append(text[pos:])
    return "".join(normal), "".join(tests)


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def parse_use_tree(text):
    """The leaves of a `use` tree: (path segments, local name or None)."""
    tokens = re.findall(r"::|[{},*]|\bas\b|\w+", text)
    pos = 0

    def tree(prefix):
        nonlocal pos
        leaves = []
        path = list(prefix)
        while pos < len(tokens):
            token = tokens[pos]
            if token == "::":
                pos += 1
            elif token == "{":
                pos += 1
                while pos < len(tokens) and tokens[pos] != "}":
                    if tokens[pos] == ",":
                        pos += 1
                        continue
                    leaves += tree(path)
                pos += 1
                return leaves
            elif token in (",", "}"):
                break
            elif token == "*":
                pos += 1
                leaves.append((path + ["*"], None))
                return leaves
            elif token == "as":
                pos += 1
                alias = tokens[pos]
                pos += 1
                leaves.append((path, alias))
                return leaves
            else:
                pos += 1
                if token == "self":
                    continue
                path = path + [token]
        if path != list(prefix) or not leaves:
            leaves.append((path, None))
        return leaves

    return tree([])


def uses_in(text, crate_ident):
    """The paths `text` names in the crate `crate_ident`."""
    used = set()
    locals_ = {crate_ident: []}
    for match in re.finditer(r"\buse\s+([^;]+);", text):
        for segments, alias in parse_use_tree(match.group(1)):
            if not segments or segments[0] != crate_ident:
                continue
            rest = segments[1:]
            if rest and rest[-1] != "*":
                used.add("::".join(rest))
            elif rest and rest[-1] == "*":
                used.add("::".join(rest[:-1] + ["*"]) if rest[:-1] else "*")
            local = alias or (rest[-1] if rest and rest[-1] != "*" else None)
            if alias and not rest:
                locals_[alias] = []
            elif local:
                locals_[local] = rest
    for local, prefix in locals_.items():
        # `alias::Item`, or `crate::alias::Item` for an alias made at the
        # crate's root and used from its other files.
        pattern = r"(?:(?<![\w:])crate::|(?<![\w:]))" + re.escape(local) + r"((?:::\w+)+)"
        for match in re.finditer(pattern, text):
            used.add("::".join(prefix + match.group(1).split("::")[1:]))
    return used


def match_items(used, items):
    """The dependency's public items that the paths in `used` name."""
    found = set()
    for path in used:
        if path == "*":
            continue
        segments = path.split("::")
        for end in range(len(segments), 0, -1):
            candidate = "::".join(segments[:end])
            if candidate in items:
                found.add(candidate)
                break
    return sorted(found)


def main():
    meta = metadata()
    concerns = json.loads((HERE / "concerns.json").read_text())
    packages = {p["name"]: p for p in meta["packages"]}
    member_dirs = {name: Path(p["manifest_path"]).parent for name, p in packages.items()}
    lib_names = {}
    for name, p in packages.items():
        for target in p["targets"]:
            if "lib" in target["kind"]:
                lib_names[name] = target["name"]

    crates = []
    items_of = {}
    for name in sorted(packages):
        p = packages[name]
        lib = lib_names.get(name)
        bins = [t["name"] for t in p["targets"] if "bin" in t["kind"]]
        doc_summary, items = "", {}
        if lib:
            print(f"rustdoc {name}", file=sys.stderr)
            doc_summary, items = public_items(rustdoc(name, lib, None))
            for feature in sorted(f for f in p["features"] if f != "default"):
                print(f"rustdoc {name} --features {feature}", file=sys.stderr)
                _, with_feature = public_items(rustdoc(name, lib, feature))
                for path, entry in with_feature.items():
                    if path not in items:
                        entry["feature"] = feature
                        items[path] = entry
                        continue
                    # A feature can also add methods, variants or fields to
                    # an item that is there without it.
                    for part in ("methods", "variants", "fields"):
                        added = sorted(set(entry.get(part, [])) - set(items[path].get(part, [])))
                        if added:
                            items[path].setdefault("feature_" + part, {})[feature] = added
        items_of[name] = items
        meta_kkt = (p.get("metadata") or {}).get("kkt") or {}
        meta_dist = (p.get("metadata") or {}).get("dist") or {}
        third_party = sorted({d["name"] for d in p["dependencies"]
                              if d["name"] not in packages and d["kind"] is None})
        crates.append({
            "name": name,
            "lib": lib,
            "bins": bins,
            "path": str(member_dirs[name].relative_to(ROOT)) or ".",
            "dev_only": bool(meta_kkt.get("dev-only")),
            "released_program": bool(bins) and meta_dist.get("dist") is not False,
            "features": {f: v for f, v in p["features"].items() if f != "default"},
            "crate_doc": doc_summary,
            "third_party": third_party,
            "items": sorted(items.values(), key=lambda e: e["path"]),
        })

    edges = []
    for name in sorted(packages):
        p = packages[name]
        package_dir = member_dirs[name]
        nested = [d for other, d in member_dirs.items()
                  if other != name and d != package_dir and d.is_relative_to(package_dir)]
        texts = {"normal": "", "dev": "", "build": ""}
        files = list(rust_files(package_dir, nested))
        test_files = test_module_files(package_dir, files)
        for path in files:
            text = strip_comments(path.read_text())
            kind = file_kind(package_dir, path, test_files)
            if kind == "normal":
                text, tests = split_test_modules(text)
                texts["dev"] += tests
            texts[kind] += text
        normal_deps = {d["name"] for d in p["dependencies"] if d["kind"] is None}
        for dep in p["dependencies"]:
            if dep["name"] not in packages:
                continue
            kind = dep["kind"] or "normal"
            # A dev-dependency can only be used in test code, wherever that
            # sits (a `#[cfg(test)]` item outside a test module, too); its
            # arrow takes every use, unless the same crate is also a normal
            # dependency, whose uses are the normal arrow's.
            text = texts[kind]
            if kind == "dev" and dep["name"] not in normal_deps:
                text = texts["normal"] + texts["dev"]
            ident = lib_names.get(dep["name"], dep["name"].replace("-", "_"))
            if dep.get("rename"):
                ident = dep["rename"].replace("-", "_")
            used = uses_in(text, ident)
            edges.append({
                "from": name,
                "to": dep["name"],
                "kind": kind,
                "features": dep["features"],
                "optional": dep["optional"],
                "target": dep.get("target"),
                "items": match_items(used, set(items_of[dep["name"]])),
            })

    missing = sorted(set(packages) - set(concerns["crates"]))
    if missing:
        sys.exit(f"concerns.json has no entry for: {', '.join(missing)}")
    stale = sorted(set(concerns["crates"]) - set(packages))
    if stale:
        sys.exit(f"concerns.json names crates that are not in the workspace: {', '.join(stale)}")

    commit = run(["git", "rev-parse", "--short", "HEAD"]).strip()
    graph = {"commit": commit, "crates": crates, "edges": edges}
    out = HERE / "map-data.js"
    out.write_text(
        "// Generated by docs/architecture/generate.py from cargo metadata, rustdoc's\n"
        "// JSON and the crates' sources; the concerns come from concerns.json.\n"
        "// Do not edit: run the script again.\n"
        f"window.KKT_GRAPH = {json.dumps(graph, indent=1, sort_keys=True)};\n"
        f"window.KKT_CONCERNS = {json.dumps(concerns, indent=1, sort_keys=True)};\n"
    )
    print(f"wrote {out.relative_to(ROOT)}: {len(crates)} crates, {len(edges)} arrows, "
          f"{sum(len(c['items']) for c in crates)} public items, at {commit}", file=sys.stderr)


if __name__ == "__main__":
    main()
