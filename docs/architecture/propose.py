#!/usr/bin/env python3
"""Writes the proposed crate map, `proposed-data.js`: the current map
(`map-data.js`, from generate.py) reorganized by the dependency-injection
rule that `proposed.json` states and applies by hand.

Run it after generate.py: `python3 docs/architecture/propose.py`. With
`--check`, it writes nothing and fails if `proposed-data.js` is out of date.

What it derives, and what is written by hand:

- The table of traits, with each one's consumers, test-only users and
  implementations: derived from the current map. A consumer is a crate
  other than the trait's definer and implementers that takes the trait in
  a public slot or names it in normal code.
- The rule's result for each trait, the new crates, the moves, the traits
  consumers declare for themselves, the implementations that stop
  implementing a trait, the programs' adapters, and the conflicts and
  assumptions: written by hand in proposed.json. The script stops if a
  trait has no decision, or anything named there is not in the map.
- The proposed crates, items, implementations, slots and arrows with their
  roles: derived by applying proposed.json to the current map. Code that
  uses a moved item depends on its new crate; a consumer that declares a
  trait no longer depends on its old crate for it; a crate that stops
  implementing a trait loses that arrow; each adapter makes its program
  implement the consumer's contract and wire the implementation.
- What changed: derived by comparing the two maps.
"""

import copy
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load_current():
    text = (HERE / "map-data.js").read_text()
    grab = lambda name: json.loads(re.search(
        rf"window\.{name} = (.*?);\n(?:window\.|$)", text, re.S).group(1))
    return grab("KKT_GRAPH"), grab("KKT_CONCERNS"), grab("KKT_ASSEMBLY")


def trait_table(graph):
    """Every workspace trait, with its consumers, test-only users and
    implementations in the current map."""
    rows = []
    for crate in graph["crates"]:
        for item in crate["items"]:
            if item["kind"] != "trait":
                continue
            owner, path = crate["name"], item["path"]
            impls = [{"crate": c["name"], "type": i["type"]} for c in graph["crates"]
                     for i in c["implements"] if i["trait_crate"] == owner and i["trait"] == path]
            implementers = {i["crate"] for i in impls}
            in_slots = {c["name"] for c in graph["crates"] for s in c["slots"]
                        if any(t["crate"] == owner and t["trait"] == path for t in s["traits"])}
            named = {e["from"] for e in graph["edges"]
                     if e["to"] == owner and e["kind"] == "normal" and path in e["items"]}
            consumers = sorted((in_slots | named) - {owner} - implementers)
            tests = sorted({e["from"] for e in graph["edges"]
                            if e["to"] == owner and e["kind"] == "dev" and path in e["items"]}
                           - set(consumers) - {owner} - implementers)
            rows.append({"trait": f"{owner}::{path}", "crate": owner, "path": path,
                         "feature": item.get("feature"), "consumers": consumers,
                         "test_only": tests, "implementations": impls})
    return sorted(rows, key=lambda r: r["trait"])


def split(ref):
    crate, _, path = ref.partition("::")
    return crate, path


def propose(graph, concerns, assembly, spec):
    problems = []
    table = trait_table(graph)
    traits = {r["trait"]: r for r in table}
    for name in traits:
        if name not in spec["decisions"]:
            problems.append(f"no decision for {name}")
    for name in spec["decisions"]:
        if name not in traits:
            problems.append(f"a decision for {name}, which is not a trait in the map")

    crates = {c["name"]: copy.deepcopy(c) for c in graph["crates"]}
    programs = {n for n, c in crates.items() if c["bins"]}
    items_of = {n: {i["path"]: i for i in c["items"]} for n, c in crates.items()}
    changed_crates = {}

    def note(crate, text):
        changed_crates.setdefault(crate, []).append(text)

    # New crates.
    kinds = {}
    for new in spec["new_crates"]:
        kinds[new["name"]] = new["kind"]
        crates[new["name"]] = {
            "name": new["name"], "lib": new["name"].replace("-", "_"), "bins": [], "path": "(proposed)",
            "dev_only": False, "released_program": False, "features": {}, "crate_doc": "",
            "third_party": [], "items": [], "slots": [], "implements": [], "contracts": [],
            "builds": [], "holders": [], "proposed": "new", "kind": new["kind"],
        }
        items_of[new["name"]] = {}
        note(new["name"], f"new {new['kind']} crate, from {new['from']}")

    # Moves.
    home = {}
    for move in spec["moves"]:
        crate, path = split(move["item"])
        if path not in items_of.get(crate, {}):
            problems.append(f"move: no item {path} in {crate}")
            continue
        item = items_of[crate].pop(path)
        item = dict(item, moved_from=crate)
        items_of[move["to"]][path] = item
        home[(crate, path)] = move["to"]
        note(crate, f"{path} moves to {move['to']}")

    # Traits consumers declare for themselves.
    declared = {}
    for decl in spec["declared"]:
        crate, path = split(decl["trait"])
        if decl["trait"] not in traits:
            problems.append(f"declared: {decl['trait']} is not a trait")
            continue
        not_consumers = set(decl["by"]) - set(traits[decl["trait"]]["consumers"])
        if not_consumers:
            problems.append(f"declared: {', '.join(sorted(not_consumers))} do not consume {decl['trait']}")
        moving = [path] + decl.get("with", [])
        for consumer in decl["by"]:
            for p in moving:
                if p in items_of[crate]:
                    items_of[consumer][p] = dict(items_of[crate][p], declared_from=crate)
                    declared[(crate, p, consumer)] = True
            note(consumer, f"declares its own {path}")
        if crate not in decl["by"]:
            for p in moving:
                items_of[crate].pop(p, None)
            note(crate, f"no longer declares {path}")

    def where(crate, path, consumer=None):
        """The crate the item `path` of `crate` lives in, as seen by `consumer`."""
        if consumer and (crate, path, consumer) in declared:
            return consumer
        return home.get((crate, path), crate)

    stop = {s["trait"] for s in spec["stop_implementing"]}

    # Implementations.
    records = []
    for c in graph["crates"]:
        for i in c["implements"]:
            ref = f"{i['trait_crate']}::{i['trait']}"
            impl_crate = where(c["name"], i["type"]) if i["type"] else c["name"]
            if ref in stop:
                if i["trait_crate"] != c["name"] or impl_crate != c["name"]:
                    note(impl_crate, f"{i['type'] or 'its own type'} no longer implements {ref}")
                continue
            records.append({"crate": impl_crate, "type": i["type"],
                            "trait_crate": where(i["trait_crate"], i["trait"]), "trait": i["trait"]})
    adapter_list = {}
    for program, adapters in spec["adapters"].items():
        if program not in programs:
            problems.append(f"adapters: {program} is not a program")
        for a in adapters:
            ic, ip = split(a["adapts"])
            if ip not in items_of.get(ic, {}):
                problems.append(f"adapters: {program} adapts {a['adapts']}, which is not in the proposed map")
            # One adapter may meet several contracts (KeyMode, for both
            # consumers' Identify).
            for target in (a["to"] if isinstance(a["to"], list) else [a["to"]]):
                tc, tp = split(target)
                if items_of.get(tc, {}).get(tp, {}).get("kind") != "trait":
                    problems.append(f"adapters: {program} adapts to {target}, which is not a trait in the proposed map")
                records.append({"crate": program, "type": None, "trait_crate": tc, "trait": tp, "adapts": a["adapts"]})
            adapter_list.setdefault(program, []).append(a)
    # Deduplicated, in a fixed order, so the output is the same every run.
    records = sorted((dict(t) for t in {tuple(sorted((k, v or "") for k, v in r.items())) for r in records}),
                     key=lambda r: (r["crate"], r["trait_crate"], r["trait"], r["type"], r.get("adapts", "")))
    for r in records:
        r["type"] = r["type"] or None
        if not r.get("adapts"):
            r.pop("adapts", None)

    for name, c in crates.items():
        c["items"] = sorted(items_of[name].values(), key=lambda i: i["path"])
        c["implements"] = sorted([r for r in records if r["crate"] == name],
                                 key=lambda r: (r["trait_crate"], r["trait"], r.get("type") or ""))
        c["contracts"] = [{"trait": i["path"], "implementors": [
            {"crate": r["crate"], "type": r["type"]} for r in records
            if r["trait_crate"] == name and r["trait"] == i["path"]]}
            for i in c["items"] if i["kind"] == "trait"]
        for slot in c["slots"]:
            slot["traits"] = [{"crate": where(t["crate"], t["trait"], name), "trait": t["trait"]}
                              for t in slot["traits"]]
            for h in slot.get("holds", []):
                h["crate"] = where(h["crate"], h["type"])
        c["builds"] = [b for b in c.get("builds", []) if b["type"] in items_of[name]]
        c["holders"] = [h for h in c.get("holders", []) if h["type"] in items_of[name]]
        if name in changed_crates and not c.get("proposed"):
            c["proposed"] = "changed"
        c["proposed_notes"] = changed_crates.get(name, [])

    # Slots that take one value meeting several contracts (kkt-server's
    # identify, for its own Identify and kkt-mcp's).
    for bound in spec.get("slot_bounds", []):
        crate, at = split(bound["slot"])
        found = [s for s in crates.get(crate, {}).get("slots", []) if s["at"] == at and s["name"] == bound["param"]]
        if not found:
            problems.append(f"slot_bounds: no slot {at} ({bound['param']}) in {crate}")
        for slot in found:
            slot["traits"] = [{"crate": split(t)[0], "trait": split(t)[1]} for t in bound["traits"]]
            slot["note"] = bound["note"]
            for t in bound["traits"]:
                tc, tp = split(t)
                if items_of.get(tc, {}).get(tp, {}).get("kind") != "trait":
                    problems.append(f"slot_bounds: {t} is not a trait in the proposed map")
        note(crate, f"{at} takes one {bound['param']} value meeting " + " and ".join(bound["traits"]))

    # Arrows.
    implemented = {(r["crate"], r["trait_crate"], r["trait"]) for r in records}
    old_implemented = {(c["name"], i["trait_crate"], i["trait"])
                       for c in graph["crates"] for i in c["implements"]}
    grouped = {}
    for e in graph["edges"]:
        role_of = {p: r["role"] for r in e.get("roles", []) for p in r["detail"]}
        for path in e["items"]:
            ref = f"{e['to']}::{path}"
            if (e["from"], e["to"], path) in old_implemented and ref in stop:
                continue
            target = where(e["to"], path, e["from"])
            if target == e["from"]:
                continue
            key = (e["from"], target, e["kind"])
            entry = grouped.setdefault(key, {"items": {}, "features": e["features"], "optional": e["optional"],
                                             "target": e["target"]})
            entry["items"][path] = role_of.get(path, "uses")
        if not e["items"]:
            key = (e["from"], e["to"], e["kind"])
            grouped.setdefault(key, {"items": {}, "features": e["features"], "optional": e["optional"],
                                     "target": e["target"]})
    for r in records:
        if r["crate"] == r["trait_crate"]:
            continue
        key = (r["crate"], r["trait_crate"], "normal")
        entry = grouped.setdefault(key, {"items": {}, "features": [], "optional": False, "target": None})
        if r.get("adapts"):
            ic, ip = split(r["adapts"])
            if ic != r["crate"]:
                wired = grouped.setdefault((r["crate"], ic, "normal"),
                                           {"items": {}, "features": [], "optional": False, "target": None})
                wired["items"][ip] = "wires"
    # A crate whose public slot takes a trait that now lives in another
    # crate depends on that crate for it (kkt-names' handles take Entropy,
    # now in kkt-entropy).
    for name, c in crates.items():
        for slot in c["slots"]:
            for t in slot["traits"]:
                if t["crate"] != name and t["crate"] in crates:
                    entry = grouped.setdefault((name, t["crate"], "normal"),
                                               {"items": {}, "features": [], "optional": False, "target": None})
                    entry["items"].setdefault(t["trait"], "uses")
    edges = []
    for (frm, to, kind), entry in sorted(grouped.items()):
        roles = []
        impl_detail = [f"{r['type'] or (('adapter for ' + r['adapts']) if r.get('adapts') else 'a type of its own')} implements {r['trait']}"
                       for r in records if r["crate"] == frm and r["trait_crate"] == to and kind == "normal"]
        if impl_detail:
            roles.append({"role": "implements", "detail": sorted(impl_detail)})
        implemented_here = {r["trait"] for r in records if r["crate"] == frm and r["trait_crate"] == to}
        uses = sorted(p for p, role in entry["items"].items() if role != "wires" and p not in implemented_here)
        wires = sorted(p for p, role in entry["items"].items() if role == "wires")
        if wires:
            roles.append({"role": "wires", "detail": wires})
        if uses or not roles:
            roles.append({"role": "uses", "detail": uses})
        order = ["implements", "uses", "wires"]
        roles.sort(key=lambda r: order.index(r["role"]))
        edges.append({"from": frm, "to": to, "kind": kind, "features": entry["features"],
                      "optional": entry["optional"], "target": entry["target"],
                      "items": sorted(entry["items"]), "roles": roles, "role": roles[0]["role"]})

    # What changed.
    def key(e):
        return (e["from"], e["to"], e["kind"])
    before = {key(e): e for e in graph["edges"]}
    after = {key(e): e for e in edges}
    changes = {"crates": [{"crate": n, "notes": notes} for n, notes in sorted(changed_crates.items())],
               "added": [], "removed": [], "changed": []}
    for k, e in after.items():
        if k not in before:
            e["proposed"] = "new"
            changes["added"].append({"from": e["from"], "to": e["to"], "kind": e["kind"], "role": e["role"]})
        else:
            old = before[k]
            if sorted(old["items"]) != e["items"] or [r["role"] for r in old["roles"]] != [r["role"] for r in e["roles"]]:
                e["proposed"] = "changed"
                changes["changed"].append({
                    "from": e["from"], "to": e["to"], "kind": e["kind"],
                    "roles_before": [r["role"] for r in old["roles"]], "roles_after": [r["role"] for r in e["roles"]],
                    "items_gone": sorted(set(old["items"]) - set(e["items"])),
                    "items_new": sorted(set(e["items"]) - set(old["items"]))})
    for k, e in before.items():
        if k not in after:
            changes["removed"].append({"from": e["from"], "to": e["to"], "kind": e["kind"], "role": e["role"]})

    # The rule: only programs depend on implementation crates.
    violations = [f"{e['from']} depends on the implementation crate {e['to']}" for e in edges
                  if kinds.get(e["to"]) == "implementation" and e["from"] not in programs and e["kind"] == "normal"]

    proposed_concerns = copy.deepcopy(concerns)
    layers = proposed_concerns["layers"]
    for added in spec["layers_added"]:
        at = next(i for i, l in enumerate(layers) if l["id"] == added["after"]) + 1
        layers.insert(at, {k: v for k, v in added.items() if k != "after"})
    for new in spec["new_crates"]:
        proposed_concerns["crates"][new["name"]] = {"layer": new["layer"], "concern": new["concern"]}

    proposed_assembly = copy.deepcopy(assembly)
    for program, flow in proposed_assembly["programs"].items():
        # The same steps and wiring, with each item in its proposed crate.
        for step in flow["steps"] + flow.get("also", []):
            step["items"] = [f"{where(*split(ref))}::{split(ref)[1]}" for ref in step["items"]]
        for w in flow.get("wiring", []):
            if w.get("impl"):
                c, p = split(w["impl"])
                w["impl"] = f"{where(c, p)}::{p}"
        flow["adapters"] = adapter_list.get(program, [])
        flow["inferred"] = False

    if problems:
        sys.exit("proposed.json does not fit the map:\n  " + "\n  ".join(problems))
    for row in table:
        decision = spec["decisions"][row["trait"]]
        row.update(decision)
        row["rule"] = ("shared contract crate" if len(row["consumers"]) > 2
                       else "declared by its consumers")
    return {
        "graph": {"commit": graph["commit"], "crates": sorted(crates.values(), key=lambda c: c["name"]),
                  "edges": edges},
        "concerns": proposed_concerns,
        "assembly": proposed_assembly,
        "table": table,
        "changes": changes,
        "violations": violations,
        "rule": spec["rule"],
        "conflicts": spec["conflicts"],
        "withdrawn": spec.get("withdrawn", []),
        "assumptions": spec["assumptions"],
        "not_traits": spec["not_traits"],
        "stop_implementing": spec["stop_implementing"],
    }


def main():
    graph, concerns, assembly = load_current()
    spec = json.loads((HERE / "proposed.json").read_text())
    data = propose(graph, concerns, assembly, spec)
    text = ("// Generated by docs/architecture/propose.py from map-data.js and proposed.json.\n"
            "// Do not edit: run the script again.\n"
            f"window.KKT_PROPOSED = {json.dumps(data, indent=1, sort_keys=True)};\n")
    out = HERE / "proposed-data.js"
    if "--check" in sys.argv[1:]:
        if not out.exists() or out.read_text() != text:
            sys.exit("docs/architecture/proposed-data.js is out of date: run python3 docs/architecture/propose.py")
        print("docs/architecture/proposed-data.js is current", file=sys.stderr)
        return
    out.write_text(text)
    g = data["graph"]
    print(f"wrote {out.name}: {len(g['crates'])} crates, {len(g['edges'])} arrows; "
          f"{len(data['changes']['added'])} arrows added, {len(data['changes']['removed'])} removed, "
          f"{len(data['changes']['changed'])} changed; {len(data['violations'])} violations", file=sys.stderr)


if __name__ == "__main__":
    main()
