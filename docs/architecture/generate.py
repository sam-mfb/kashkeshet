#!/usr/bin/env python3
"""Regenerates the architecture map's data, `map-data.js`, from the code.

Run from anywhere in the repository: `python3 docs/architecture/generate.py`.
With `--check`, it writes nothing, and fails if `map-data.js` differs from
what it would write in anything but the commit it names (CI runs this).
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
- Contracts, slots and the role of each arrow (implements, uses, wires):
  rustdoc's JSON again. Trait impls come from a second build with private
  items, so an impl by a private type counts; the page names such a type
  only as "a type of its own". Slots are public parameters and fields
  typed as a workspace trait, or as a type that holds one. See the README.
- Each crate's concern, its layer and what it must not do: written by
  hand in `concerns.json`, from CLAUDE.md and the crates' own docs.
- How each program assembles the server: written by hand in
  `assembly.json`, whose every item is checked against the public items.
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


def rustdoc(package, lib, feature, private=False, bin_target=None):
    """rustdoc's JSON for `package`'s library, with `feature` (a feature, or
    several joined by commas) turned on. With `private`, private items are
    in it too: read only for which traits the crate implements, never
    shown."""
    env = dict(os.environ, RUSTC_BOOTSTRAP="1")
    target = ["--bin", bin_target] if bin_target else ["--lib"]
    args = ["cargo", "rustdoc", "-q", "--locked", "-p", package, *target,
            "--target-dir", str(TARGET)]
    if feature:
        args += ["--features", feature]
    args += ["--", "-Z", "unstable-options", "--output-format", "json"]
    if private:
        args += ["--document-private-items"]
    run(args, env)
    name = bin_target.replace("-", "_") if bin_target else lib
    return json.loads((TARGET / "doc" / f"{name}.json").read_text())


def public_items(doc):
    """Every public item of the crate in `doc`, by its path in the crate."""
    index = doc["index"]
    items = {}
    ids = {}

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
        ids[path] = item["id"]

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
    return crate_doc, items, ids


# ---- Contracts: traits, their implementations, and the slots that take them ----


def crate_of(doc, crate_id):
    """The library name of the crate `crate_id` names in `doc`."""
    if crate_id == 0:
        return doc["index"][str(doc["root"])]["name"]
    return doc["external_crates"].get(str(crate_id), {}).get("name")


def resolve(doc, item_id):
    """(library, last path segment, kind) of the item `item_id` names."""
    entry = doc["paths"].get(str(item_id))
    if not entry:
        return None
    return crate_of(doc, entry["crate_id"]), entry["path"][-1], entry["kind"]


def generic_bounds(generics):
    """Each generic parameter's trait bounds, as trait IDs."""
    bounds = {}
    for param in generics.get("params", []):
        kind = param["kind"]
        if "type" in kind:
            bounds[param["name"]] = [b["trait_bound"]["trait"]["id"] for b in kind["type"]["bounds"]
                                     if "trait_bound" in b]
    for pred in generics.get("where_predicates", []):
        bound = pred.get("bound_predicate")
        if bound and isinstance(bound["type"], dict) and "generic" in bound["type"]:
            bounds.setdefault(bound["type"]["generic"], []).extend(
                b["trait_bound"]["trait"]["id"] for b in bound["bounds"] if "trait_bound" in b)
    return bounds


def type_refs(ty, bounds):
    """The traits a type stands for (`dyn T`, `impl T`, a generic bound by T)
    and the named types in it, as two lists of item IDs."""
    traits, types = [], []

    def visit(t):
        if not isinstance(t, dict):
            return
        kind, value = next(iter(t.items()))
        if kind == "dyn_trait":
            traits.extend(x["trait"]["id"] for x in value["traits"])
        elif kind == "impl_trait":
            traits.extend(b["trait_bound"]["trait"]["id"] for b in value if "trait_bound" in b)
        elif kind == "generic":
            traits.extend(bounds.get(value, []))
        elif kind == "resolved_path":
            types.append(value["id"])
            args = value.get("args") or {}
            for arg in (args.get("angle_bracketed") or {}).get("args", []):
                if "type" in arg:
                    visit(arg["type"])
        elif kind == "borrowed_ref":
            visit(value["type"])
        elif kind == "slice":
            visit(value)
        elif kind == "array":
            visit(value["type"])
        elif kind == "tuple":
            for part in value:
                visit(part)

    visit(ty)
    return traits, types


def signatures(doc, items, ids):
    """What the crate's public functions, methods and fields take and give:
    for each, the traits and the types in its parameters or field, and the
    types it returns. Keys are (owner path, parameter or field name)."""
    index = doc["index"]
    takes, gives = [], []

    def function(owner, item):
        f = item["inner"]["function"]
        bounds = generic_bounds(f["generics"])
        for name, ty in f["sig"]["inputs"]:
            if name == "self":
                continue
            traits, types = type_refs(ty, bounds)
            takes.append({"at": owner, "name": name, "kind": "param", "traits": traits, "types": types})
        if f["sig"].get("output") is not None:
            _, types = type_refs(f["sig"]["output"], bounds)
            entry = {"at": owner, "types": types}
            # `-> Self` (or `Result<Self, _>`) gives back the owner type.
            if '"generic": "Self"' in json.dumps(f["sig"]["output"]):
                entry["self"] = True
            gives.append(entry)

    for path, item_id in ids.items():
        item = index.get(str(item_id))
        if item is None or item.get("crate_id") != 0:
            continue
        kind = next(iter(item["inner"]))
        if kind == "function":
            function(path, item)
        elif kind in ("struct", "enum"):
            inner = item["inner"][kind]
            shape = inner.get("kind")
            if kind == "struct" and isinstance(shape, dict) and "plain" in shape:
                for field_id in shape["plain"]["fields"]:
                    field = index.get(str(field_id))
                    if field and field.get("visibility") == "public":
                        traits, types = type_refs(field["inner"]["struct_field"], {})
                        takes.append({"at": path, "name": field["name"], "kind": "field",
                                      "traits": traits, "types": types})
            for impl_id in inner.get("impls", []):
                impl = index.get(str(impl_id))
                if not impl or impl["inner"]["impl"].get("trait") is not None:
                    continue
                for method_id in impl["inner"]["impl"]["items"]:
                    method = index.get(str(method_id))
                    if method and method.get("visibility") == "public" and "function" in method["inner"]:
                        function(f"{path}::{method['name']}", method)
    resolve_ids = lambda xs: [r for r in (resolve(doc, x) for x in xs) if r]
    for entry in takes:
        entry["traits"] = resolve_ids(entry["traits"])
        entry["types"] = resolve_ids(entry["types"])
    for entry in gives:
        entry["types"] = resolve_ids(entry["types"])
        if entry.pop("self", False) and "::" in entry["at"]:
            entry["types"].append((crate_of(doc, 0), entry["at"].rsplit("::", 1)[0].split("::")[-1], "struct"))
    return takes, gives


def trait_impls(doc):
    """Every impl in the crate of a trait: (trait's library, trait name, the
    implementing type's name or None when it is not a named type)."""
    found = []
    for item in doc["index"].values():
        if item.get("crate_id") != 0 or "impl" not in item["inner"]:
            continue
        impl = item["inner"]["impl"]
        if impl.get("trait") is None or impl.get("is_synthetic") or impl.get("blanket_impl"):
            continue
        trait = resolve(doc, impl["trait"]["id"])
        if not trait:
            continue
        target = impl["for"]
        name = None
        if isinstance(target, dict) and "resolved_path" in target:
            resolved = resolve(doc, target["resolved_path"]["id"])
            if resolved and resolved[0] == crate_of(doc, 0):
                name = resolved[1]
        found.append((trait[0], trait[1], name))
    return found


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
    contracts_raw = {}
    for name in sorted(packages):
        p = packages[name]
        lib = lib_names.get(name)
        bins = [t["name"] for t in p["targets"] if "bin" in t["kind"]]
        doc_summary, items = "", {}
        if lib:
            print(f"rustdoc {name}", file=sys.stderr)
            doc = rustdoc(name, lib, None)
            doc_summary, items, ids = public_items(doc)
            takes, gives = signatures(doc, items, ids)
            features = sorted(f for f in p["features"] if f != "default")
            for feature in features:
                print(f"rustdoc {name} --features {feature}", file=sys.stderr)
                feature_doc = rustdoc(name, lib, feature)
                _, with_feature, feature_ids = public_items(feature_doc)
                more_takes, more_gives = signatures(feature_doc, with_feature, feature_ids)
                takes += [t for t in more_takes if t not in takes]
                gives += [g for g in more_gives if g not in gives]
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
        if lib:
            print(f"rustdoc {name} (private items, for its trait impls)", file=sys.stderr)
            impls = trait_impls(rustdoc(name, lib, ",".join(features), private=True))
        else:
            takes, gives, impls = [], [], []
        # A program's own code implements contracts too (kkt-keymode's key
        # files are kkt-admission's `KeyTexts`).
        for bin_name in bins:
            print(f"rustdoc {name} --bin {bin_name} (private items, for its trait impls)", file=sys.stderr)
            impls += [i for i in trait_impls(rustdoc(name, lib, None, private=True, bin_target=bin_name))
                      if i not in impls]
        contracts_raw[name] = {"takes": takes, "gives": gives, "impls": impls}
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
                # Every path the consumer names, for which constructors it
                # calls (see add_contracts).
                "_paths": sorted(used),
            })

    missing = sorted(set(packages) - set(concerns["crates"]))
    if missing:
        sys.exit(f"concerns.json has no entry for: {', '.join(missing)}")
    stale = sorted(set(concerns["crates"]) - set(packages))
    if stale:
        sys.exit(f"concerns.json names crates that are not in the workspace: {', '.join(stale)}")

    package_of_lib = {lib: name for name, lib in lib_names.items()}
    add_contracts(crates, edges, items_of, contracts_raw, package_of_lib)
    assembly = json.loads((HERE / "assembly.json").read_text())
    check_assembly(assembly, items_of, packages, {c["name"]: c["slots"] for c in crates})

    commit = run(["git", "rev-parse", "--short", "HEAD"]).strip()
    out = HERE / "map-data.js"
    text = render(commit, crates, edges, concerns, assembly)
    if "--check" in sys.argv[1:]:
        current = out.read_text() if out.exists() else ""
        if without_commit(current) != without_commit(text):
            sys.exit("docs/architecture/map-data.js is out of date: run "
                     "python3 docs/architecture/generate.py and commit the result")
        print("docs/architecture/map-data.js is current", file=sys.stderr)
        return
    out.write_text(text)
    print(f"wrote {out.relative_to(ROOT)}: {len(crates)} crates, {len(edges)} arrows, "
          f"{sum(len(c['items']) for c in crates)} public items, at {commit}", file=sys.stderr)


def render(commit, crates, edges, concerns, assembly):
    graph = {"commit": commit, "crates": crates, "edges": edges}
    return (
        "// Generated by docs/architecture/generate.py from cargo metadata, rustdoc's\n"
        "// JSON and the crates' sources; the concerns come from concerns.json and the\n"
        "// programs' assembly flows from assembly.json, both written by hand.\n"
        "// Do not edit: run the script again.\n"
        f"window.KKT_GRAPH = {json.dumps(graph, indent=1, sort_keys=True)};\n"
        f"window.KKT_CONCERNS = {json.dumps(concerns, indent=1, sort_keys=True)};\n"
        f"window.KKT_ASSEMBLY = {json.dumps(assembly, indent=1, sort_keys=True)};\n"
    )


ROLE_ORDER = ["implements", "uses", "wires"]


def add_contracts(crates, edges, items_of, raw, package_of_lib):
    """Adds to each crate the contracts it defines and implements and the
    slots it takes, and to each arrow its roles. Everything here comes from
    rustdoc's JSON (`raw`), matched to the public items by name."""
    by_name = {c["name"]: c for c in crates}

    def public(package, name, kinds):
        """The public path of the item called `name` in `package`."""
        for path, entry in items_of.get(package, {}).items():
            if path.split("::")[-1] == name and entry["kind"] in kinds:
                return path
        return None

    # Which traits each crate implements, and for which public type.
    implementations = []
    for package, data in raw.items():
        for trait_lib, trait_name, type_name in data["impls"]:
            owner = package_of_lib.get(trait_lib)
            if owner is None:
                continue
            trait_path = public(owner, trait_name, {"trait"})
            if trait_path is None:
                continue
            type_path = public(package, type_name, {"struct", "enum"}) if type_name else None
            implementations.append({"crate": package, "type": type_path,
                                    "trait_crate": owner, "trait": trait_path})
    implementations = [dict(t) for t in {tuple(sorted(i.items())) for i in implementations}]
    implementations.sort(key=lambda i: (i["trait_crate"], i["trait"], i["crate"], i["type"] or ""))

    def is_type(package, path):
        return items_of.get(package, {}).get(path, {}).get("kind") in ("struct", "enum")

    # Types that hold an implementation: a public struct or enum whose `new`
    # or public field takes a trait of a workspace crate (`Database::new(
    # backend: impl Backend)`, `Parts.clock`), or a type that does
    # (`Db::new(database: Database)`).
    def ws_traits(entry):
        out = []
        for lib, name, kind in entry["traits"]:
            owner = package_of_lib.get(lib)
            if owner and kind == "trait":
                path = public(owner, name, {"trait"})
                if path:
                    out.append({"crate": owner, "trait": path})
        return out

    holders = {}
    for _ in range(3):
        for package, data in raw.items():
            for entry in data["takes"]:
                if entry["kind"] == "field":
                    owner_type = entry["at"]
                elif entry["at"].endswith("::new"):
                    owner_type = entry["at"][: -len("::new")]
                else:
                    continue
                if not is_type(package, owner_type):
                    continue
                held = ws_traits(entry)
                for lib, name, kind in entry["types"]:
                    holder = holders.get((package_of_lib.get(lib), public(package_of_lib.get(lib), name, {"struct", "enum"})))
                    if holder:
                        held += holder
                if held:
                    key = (package, owner_type)
                    known = holders.setdefault(key, [])
                    for h in held:
                        if h not in known:
                            known.append(h)

    for c in crates:
        data = raw.get(c["name"], {"takes": [], "gives": []})
        slots = []
        for entry in data["takes"]:
            traits = ws_traits(entry)
            holds = []
            for lib, name, kind in entry["types"]:
                pkg = package_of_lib.get(lib)
                path = public(pkg, name, {"struct", "enum"}) if pkg else None
                if path and (pkg, path) in holders and (pkg, path) != (c["name"], entry["at"].rsplit("::", 1)[0]):
                    holds.append({"crate": pkg, "type": path, "traits": holders[(pkg, path)]})
            if traits or holds:
                slot = {"at": entry["at"], "name": entry["name"], "kind": entry["kind"], "traits": traits}
                if holds:
                    slot["holds"] = holds
                if slot not in slots:
                    slots.append(slot)
        c["slots"] = sorted(slots, key=lambda s: (s["at"], s["name"]))
        c["implements"] = [i for i in implementations if i["crate"] == c["name"]]
        c["contracts"] = []
        for item in c["items"]:
            if item["kind"] != "trait":
                continue
            c["contracts"].append({
                "trait": item["path"],
                "implementors": [{"crate": i["crate"], "type": i["type"]} for i in implementations
                                 if i["trait_crate"] == c["name"] and i["trait"] == item["path"]],
            })
        # What the crate's public functions give back: for wiring below.
        c["_gives"] = data["gives"]

    implementing = {(i["crate"], i["type"]) for i in implementations if i["type"]}
    # A type's constructors: its public functions that give back the type
    # itself (`KeyMode::new`, `Remote::from_client`).
    constructors = {}
    for c in crates:
        lib = next((l for l, pkg in package_of_lib.items() if pkg == c["name"]), None)
        for entry in c["_gives"]:
            if "::" not in entry["at"]:
                continue
            owner_type, method = entry["at"].rsplit("::", 1)
            if is_type(c["name"], owner_type) and any(
                    l == lib and n == owner_type.split("::")[-1] for l, n, _ in entry["types"]):
                constructors.setdefault((c["name"], owner_type), set()).add(method)
    factories = {}
    for c in crates:
        for entry in c["_gives"]:
            owner_type = entry["at"].rsplit("::", 1)[0] if "::" in entry["at"] else None
            if owner_type is None or not is_type(c["name"], owner_type):
                continue
            # A type that holds what it gives back hands it out; it does
            # not build it (`Db` and its `Database`).
            if (c["name"], owner_type) in holders:
                continue
            for lib, name, kind in entry["types"]:
                pkg = package_of_lib.get(lib)
                path = public(pkg, name, {"struct", "enum"}) if pkg else None
                if pkg and pkg != c["name"] and (path and ((pkg, path) in holders or (pkg, path) in implementing)):
                    made = factories.setdefault((c["name"], owner_type), [])
                    if {"crate": pkg, "type": path} not in made:
                        made.append({"crate": pkg, "type": path})
    for c in crates:
        del c["_gives"]
        # Its public types that build another crate's implementation or
        # holder (`SqlitePlace` gives a `kkt_db::Database`).
        c["builds"] = [{"type": t, "gives": made} for (pkg, t), made in sorted(factories.items())
                       if pkg == c["name"]]
        c["holders"] = [{"type": t, "holds": held} for (pkg, t), held in sorted(holders.items())
                        if pkg == c["name"]]

    def plain_config(package, path):
        entry = items_of[package].get(path, {})
        return entry.get("kind") == "struct" and entry.get("fields") and not entry.get("methods")

    for e in edges:
        roles = []
        implemented = [i for i in implementations if i["crate"] == e["from"] and i["trait_crate"] == e["to"]]
        if implemented and e["kind"] == "normal":
            roles.append({"role": "implements", "detail": [
                f"{i['type'] or 'a type of its own'} implements {i['trait']}" for i in implemented]})
        traits_implemented = {i["trait"] for i in implemented}
        wiring, other = [], []
        for path in e["items"]:
            if path in traits_implemented:
                continue
            key = (e["to"], path)
            # Wiring: building an implementation of a contract, a factory of
            # one, or a type that holds one, by calling one of its
            # constructors (or naming it, for a type that has none, such as
            # the unit struct `SystemClock`). Naming one that someone else
            # built, or calling its other functions (`KeyMode::admin_operator`),
            # is using it.
            made = constructors.get(key)
            built = any(f"{path}::{m}" in e["_paths"] for m in made) if made else True
            if (key in implementing or key in factories or key in holders) and built:
                if key in holders and not made:
                    other.append(path)
                    continue
                wiring.append(path)
            else:
                other.append(path)
        if wiring:
            # What goes with building an implementation is part of wiring it:
            # plain settings structs, and the traits the wired types
            # implement (a `dyn Clock` slot filled with `SystemClock`).
            wired_traits = {i["trait"] for i in implementations
                            if i["crate"] == e["to"] and i["type"] in wiring and i["trait_crate"] == e["to"]}
            other = [p for p in other if not plain_config(e["to"], p) and p not in wired_traits]
            roles.append({"role": "wires", "detail": wiring})
        if other or not roles:
            roles.append({"role": "uses", "detail": other})
        roles.sort(key=lambda r: ROLE_ORDER.index(r["role"]))
        e["roles"] = roles
        e["role"] = roles[0]["role"]
        del e["_paths"]


def check_assembly(assembly, items_of, packages, slots_of):
    """Fails if assembly.json names a program, an item or a slot that is not
    there."""
    problems = []
    for program, flow in assembly["programs"].items():
        for wire in flow.get("wiring", []):
            impl = wire.get("impl")
            if impl:
                crate, _, path = impl.partition("::")
                if crate not in items_of or path not in items_of[crate]:
                    problems.append(f"{program}: wiring names no public item {path} in {crate}")
            elif not wire.get("own"):
                problems.append(f"{program}: a wiring entry has neither impl nor own")
            crate, _, at = wire["slot"].partition("::")
            if not any(s["at"] == at and s["name"] == wire["param"] for s in slots_of.get(crate, [])):
                problems.append(f"{program}: no slot {at} ({wire['param']}) in {crate}")
    for program, flow in assembly["programs"].items():
        if program not in packages:
            problems.append(f"no package {program}")
        for step in flow["steps"] + flow.get("also", []):
            for ref in step.get("items", []):
                crate, _, path = ref.partition("::")
                if crate not in items_of or path not in items_of[crate]:
                    problems.append(f"{program}: no public item {path} in {crate}")
    if problems:
        sys.exit("assembly.json is out of date:\n  " + "\n  ".join(problems))


def without_commit(text):
    """`text` with the commit it was generated at taken out."""
    return re.sub(r'^ "commit": "[^"]*",$', ' "commit": "",', text, count=1, flags=re.M)


if __name__ == "__main__":
    main()
