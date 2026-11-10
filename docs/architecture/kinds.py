"""What sort of crate each crate is, and the checks a map's kinds must pass.

The kinds are written by hand in concerns.json (for the current map) and
proposed.json (for the proposal); this module only checks them against
what the map shows, and reports a crate whose kind does not fit rather than
changing it. generate.py and propose.py both use it.
"""

KINDS = ["contract", "logic", "effect", "composition", "configuration", "test support", "mixed"]

# Third-party crates that reach the machine: a crate that uses one does
# I/O of its own.
IO_CRATES = {"rusqlite", "reqwest", "rand", "libc"}


def unknown(crates, kind_of):
    """Crates with no kind, or a kind this module does not know."""
    return sorted(c["name"] for c in crates if kind_of.get(c["name"]) not in KINDS)


def behaviour(crate):
    """The items of `crate` that do something, outside `testing`: its
    functions, and the types with methods of their own."""
    found = []
    for item in crate["items"]:
        if item["path"].startswith("testing::") or item.get("feature") == "testing":
            continue
        if item["kind"] == "fn":
            found.append(item["path"])
        elif item["kind"] in ("struct", "enum") and item.get("methods"):
            n = len(item["methods"])
            found.append(f"{item['path']} ({n} method{'' if n == 1 else 's'})")
    return found


def turns_on_real(crate, edges):
    """Whether `crate` has a `real` feature, or turns one on in a crate it
    depends on in normal code."""
    if "real" in crate["features"]:
        return True
    return any(e["from"] == crate["name"] and e["kind"] == "normal" and "real" in e.get("features", [])
               for e in edges)


def check(crates, edges, kind_of, mixed_allowed):
    """(mismatches, violations): where a crate's kind and what the map
    shows of it disagree, and, when `mixed_allowed` is false, every crate
    that is still mixed."""
    mismatches, violations = [], []
    for c in crates:
        kind = kind_of[c["name"]]
        if kind == "contract":
            does = behaviour(c)
            if does:
                shown = ", ".join(does[:4]) + (f", and {len(does) - 4} more" if len(does) > 4 else "")
                mismatches.append(f"{c['name']} is marked contract but has behaviour: {shown}")
        if kind == "logic" and turns_on_real(c, edges):
            mismatches.append(f"{c['name']} is marked logic but turns on `real`")
        if kind == "effect" and not turns_on_real(c, edges) and not IO_CRATES & set(c["third_party"]):
            mismatches.append(f"{c['name']} is marked effect but has no `real` feature and no I/O dependency")
        if kind == "mixed" and not mixed_allowed:
            violations.append(f"{c['name']} is still mixed: a contract or logic together with a real effect")
    return mismatches, violations
