#!/usr/bin/env python3
"""Writes the proposed crate map, `proposed-data.js`: the current map
(`map-data.js`, from generate.py) reorganized by the dependency-injection
rule that `proposed.json` states and applies by hand, and, as a separate
layer, item 7 (`config-split.json`: kkt-config broken up by domain), which
Sam deferred.

Run it after generate.py: `python3 docs/architecture/propose.py`. With
`--check`, it writes nothing and fails if `proposed-data.js` is out of date.

What it derives, and what is written by hand:

- The table of traits, with each one's consumers, test-only users and
  implementations: derived from the current map. A consumer is a crate
  other than the trait's definer and implementers that takes the trait in
  a public slot or names it in normal code.
- How the rule applies (new crates, moves, removed and new items, methods
  that move, traits consumers declare, implementations that stop
  implementing a trait, adapters, slot bounds, arrow edits), a decision
  for every trait, the conflicts and assumptions, and a status for each
  (decided by Sam, recommended pending Sam, tentative): written by hand in
  proposed.json, and for item 7 in config-split.json. The script stops if
  a trait has no decision, a kkt-config item is not in config-split.json,
  or anything named is not in the map.
- The proposed crates, items, implementations, slots and arrows with their
  roles and statuses: derived by applying the hand-written files to the
  current map. Code that uses a moved item depends on its new crate; a
  consumer that declares a trait no longer depends on its old crate for
  it, and a trait declared by one consumer only is found there by every
  other crate too; an implementation follows its type and its trait; each
  adapter makes its program implement the consumer's contract and wire the
  implementation.
- What changed: derived by comparing the maps.
"""

import copy
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROLE_ORDER = ["implements", "uses", "wires"]


def load_current():
    text = (HERE / "map-data.js").read_text()
    grab = lambda name: json.loads(re.search(
        rf"window\.{name} = (.*?);\n(?:window\.|$)", text, re.S).group(1))
    return grab("KKT_GRAPH"), grab("KKT_CONCERNS"), grab("KKT_ASSEMBLY")


def split(ref):
    crate, _, path = ref.partition("::")
    return crate, path


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
            tests = sorted(({e["from"] for e in graph["edges"]
                             if e["to"] == owner and e["kind"] == "dev" and path in e["items"]}
                            | {e["from"] for e in graph["edges"]
                               if e["to"] == owner and path in e.get("test_items", [])})
                           - set(consumers) - {owner} - implementers)
            rows.append({"trait": f"{owner}::{path}", "crate": owner, "path": path,
                         "feature": item.get("feature"), "consumers": consumers,
                         "test_only": tests, "implementations": impls})
    return sorted(rows, key=lambda r: r["trait"])


def build(graph, concerns, assembly, spec, problems, extra=None):
    """The map `spec` makes of `graph`. `extra`, for the item 7 layer: more
    moves and items made private, and the parts Limits splits into."""
    extra = extra or {"moves": [], "private": [], "new_items": [], "replace": {}}
    # Code that moves with a moved item takes the items only it uses along.
    from_moves = {}
    for cm in spec.get("code_moves", []):
        for dep, paths in cm["uses"].items():
            for p in paths:
                from_moves[(cm["by"], dep, p)] = (cm["to"], cm["status"])
    crates = {c["name"]: copy.deepcopy(c) for c in graph["crates"]}
    programs = {n for n, c in crates.items() if c["bins"]}
    items_of = {n: {i["path"]: i for i in c["items"]} for n, c in crates.items()}
    notes = {}
    item_status = {}

    def note(crate, text, status):
        notes.setdefault(crate, [])
        if {"text": text, "status": status} not in notes[crate]:
            notes[crate].append({"text": text, "status": status})

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
        note(new["name"], f"new {new['kind']} crate, from {new['from']}", new["status"])

    home = {}
    for move in spec["moves"] + extra["moves"]:
        crate, path = split(move["item"])
        if path not in items_of.get(crate, {}):
            problems.append(f"move: no item {path} in {crate}")
            continue
        if move["to"] not in crates:
            problems.append(f"move: no crate {move['to']}")
            continue
        item = dict(items_of[crate].pop(path), moved_from=crate, status=move["status"])
        items_of[move["to"]][path] = item
        home[(crate, path)] = move["to"]
        item_status[(move["to"], path)] = move["status"]
        note(crate, f"{path} moves to {move['to']}" + (f": {move['note']}" if move.get("note") else ""), move["status"])
        note(move["to"], f"{path} comes here from {crate}" + (", private" if move.get("private") else ""),
             move["status"])

    # A moved type's slots (its constructor's parameters, its fields) go
    # with it.
    for name, c in list(crates.items()):
        keep = []
        for slot in c["slots"]:
            owner_type = slot["at"].split("::")[0]
            target = home.get((name, owner_type))
            if target:
                crates[target]["slots"].append(slot)
            else:
                keep.append(slot)
        c["slots"] = keep

    for add in spec.get("method_adds", []):
        crate, path = split(add["type"])
        item = items_of.get(crate, {}).get(path)
        if not item:
            problems.append(f"method_adds: no item {path} in {crate}")
            continue
        items_of[crate][path] = dict(item, methods=sorted(set(item.get("methods", [])) | set(add["methods"])),
                                     status=add["status"])
        note(crate, f"{path} gains {', '.join(add['methods'])}: {add['note']}", add["status"])

    for edit in spec.get("doc_edits", []):
        crate, path = split(edit["item"])
        item = items_of.get(crate, {}).get(path)
        if not item:
            problems.append(f"doc_edits: no item {path} in {crate}")
            continue
        items_of[crate][path] = dict(item, doc=edit["doc"])

    for rm in spec.get("remove_items", []) + extra["private"]:
        crate, path = split(rm["item"])
        crate = home.get((crate, path), crate)
        if items_of.get(crate, {}).pop(path, None) is None:
            problems.append(f"remove_items: no item {path} in {crate}")
        if rm.get("note"):
            note(crate, f"{path} is no longer public: {rm['note']}", rm["status"])

    for mm in spec.get("method_moves", []):
        crate, path = split(mm["type"])
        item = items_of.get(crate, {}).get(path)
        if not item:
            problems.append(f"method_moves: no item {path} in {crate}")
            continue
        missing = set(mm["methods"]) - set(item.get("methods", []))
        if missing:
            problems.append(f"method_moves: {path} has no {', '.join(sorted(missing))}")
        items_of[crate][path] = dict(item, methods=[m for m in item.get("methods", []) if m not in mm["methods"]],
                                     status=mm["status"])
        note(crate, f"{path}'s {', '.join(mm['methods'])} move to {mm['to']}: {mm['note']}", mm["status"])

    for new in spec.get("new_items", []) + extra["new_items"]:
        if new["crate"] not in crates:
            problems.append(f"new_items: no crate {new['crate']}")
            continue
        items_of[new["crate"]][new["path"]] = {"path": new["path"], "kind": new["kind"], "doc": new["doc"],
                                               "status": new["status"], "proposed_item": True}
        item_status[(new["crate"], new["path"])] = new["status"]
        note(new["crate"], f"new {new['kind']} {new['path']}", new["status"])

    table = {r["trait"]: r for r in trait_table(graph)}
    declared = {}
    redirect = {}
    for decl in spec["declared"]:
        crate, path = split(decl["trait"])
        if decl["trait"] not in table:
            problems.append(f"declared: {decl['trait']} is not a trait")
            continue
        not_consumers = set(decl["by"]) - set(table[decl["trait"]]["consumers"])
        if not_consumers:
            problems.append(f"declared: {', '.join(sorted(not_consumers))} do not consume {decl['trait']}")
        moving = [path] + decl.get("with", [])
        for p in moving:
            if p not in items_of[crate]:
                problems.append(f"declared: no item {p} in {crate}")
        for consumer in decl["by"]:
            for p in moving:
                if p in items_of[crate]:
                    items_of[consumer][p] = dict(items_of[crate][p], declared_from=crate, status=decl["status"])
                    declared[(crate, p, consumer)] = True
                    item_status[(consumer, p)] = decl["status"]
            note(consumer, f"declares its own {path}" + (f" ({decl['note']})" if decl.get("note") else ""),
                 decl["status"])
        if crate not in decl["by"]:
            for p in moving:
                if items_of[crate].pop(p, None) is not None and len(decl["by"]) == 1:
                    redirect[(crate, p)] = decl["by"][0]
            note(crate, f"no longer declares {path}" + (f" or the {len(moving) - 1} types that go with it" if len(moving) > 1 else ""),
                 decl["status"])

    def where(crate, path, consumer=None):
        """The crate the item `path` of `crate` lives in, as seen by `consumer`."""
        if consumer and (crate, path, consumer) in declared:
            return consumer
        if (crate, path) in redirect:
            return redirect[(crate, path)]
        return home.get((crate, path), crate)

    stop = {s["trait"]: s for s in spec["stop_implementing"]}
    records = []
    for c in graph["crates"]:
        for i in c["implements"]:
            ref = f"{i['trait_crate']}::{i['trait']}"
            impl_crate = where(c["name"], i["type"]) if i["type"] else c["name"]
            if ref in stop:
                note(impl_crate, f"{i['type'] or 'its own type'} no longer implements {ref}", stop[ref]["status"])
                continue
            trait_crate = where(i["trait_crate"], i["trait"])
            status = item_status.get((impl_crate, i["type"])) or item_status.get((trait_crate, i["trait"]))
            record = {"crate": impl_crate, "type": i["type"], "trait_crate": trait_crate, "trait": i["trait"]}
            if status:
                record["status"] = status
            records.append(record)
    adapter_list = {}
    for program, adapters in spec["adapters"].items():
        if program not in programs:
            problems.append(f"adapters: {program} is not a program")
        for a in adapters:
            ic, ip = split(a["adapts"])
            if ip not in items_of.get(ic, {}):
                problems.append(f"adapters: {program} adapts {a['adapts']}, which is not in the proposed map")
            for target in (a["to"] if isinstance(a["to"], list) else [a["to"]]):
                tc, tp = split(target)
                if items_of.get(tc, {}).get(tp, {}).get("kind") != "trait":
                    problems.append(f"adapters: {program} adapts to {target}, which is not a trait in the proposed map")
                records.append({"crate": program, "type": None, "trait_crate": tc, "trait": tp,
                                "adapts": a["adapts"], "status": a["status"]})
            adapter_list.setdefault(program, []).append(a)
    unique = {}
    for r in records:
        unique.setdefault((r["crate"], r["trait_crate"], r["trait"], r["type"] or "", r.get("adapts", "")), r)
    records = [unique[k] for k in sorted(unique)]

    for name, c in crates.items():
        c["items"] = sorted(items_of[name].values(), key=lambda i: i["path"])
        c["implements"] = [r for r in records if r["crate"] == name]
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

    for bound in spec.get("slot_bounds", []):
        crate, at = split(bound["slot"])
        found = [s for s in crates.get(crate, {}).get("slots", []) if s["at"] == at and s["name"] == bound["param"]]
        if not found:
            problems.append(f"slot_bounds: no slot {at} ({bound['param']}) in {crate}")
        for slot in found:
            slot["traits"] = [{"crate": split(t)[0], "trait": split(t)[1]} for t in bound["traits"]]
            slot["holds"] = []
            slot["note"] = bound["note"]
            slot["status"] = bound["status"]
        for t in bound["traits"]:
            tc, tp = split(t)
            if items_of.get(tc, {}).get(tp, {}).get("kind") != "trait":
                problems.append(f"slot_bounds: {t} is not a trait in the proposed map")
        note(crate, f"{at} takes one {bound['param']} value meeting " + " and ".join(bound["traits"]), bound["status"])

    # Arrows, item by item.
    old_implemented = {(c["name"], i["trait_crate"], i["trait"]) for c in graph["crates"] for i in c["implements"]}
    grouped = {}

    def entry_for(key, like=None):
        return grouped.setdefault(key, {
            "items": {}, "features": like["features"] if like else [], "optional": like["optional"] if like else False,
            "target": like["target"] if like else None, "statuses": set(), "had_items": False})

    for e in graph["edges"]:
        role_of = {p: r["role"] for r in e.get("roles", []) for p in r["detail"]}
        if not e["items"]:
            entry_for((e["from"], e["to"], e["kind"]), e)
        for path in e["items"]:
            ref = f"{e['to']}::{path}"
            if (e["from"], e["to"], path) in old_implemented and ref in stop:
                continue
            frm, moved_status = from_moves.get((e["from"], e["to"], path), (e["from"], None))
            # An item split into parts (Limits, in item 7): each user takes
            # the parts it uses.
            replaced = extra["replace"].get((e["to"], path))
            if replaced is not None:
                for ref2 in replaced.get(e["from"], []):
                    rc, rp = split(ref2)
                    if rc != frm:
                        entry = entry_for((frm, rc, e["kind"]), e)
                        entry["items"][rp] = role_of.get(path, "uses")
                        entry["had_items"] = True
                        entry["statuses"].add(extra["status"])
                if e["from"] not in replaced:
                    problems.append(f"config-split.json: limits.users does not say what {e['from']} takes of {path}")
                continue
            target = where(e["to"], path, e["from"])
            if target == frm:
                continue
            entry = entry_for((frm, target, e["kind"]), e)
            entry["items"][path] = role_of.get(path, "uses")
            entry["had_items"] = True
            if moved_status:
                entry["statuses"].add(moved_status)
            if target != e["to"]:
                status = item_status.get((target, path)) or item_status.get((e["to"], path))
                if status:
                    entry["statuses"].add(status)
    for name, c in crates.items():
        for slot in c["slots"]:
            for t in slot["traits"]:
                if t["crate"] != name and t["crate"] in crates:
                    entry = entry_for((name, t["crate"], "normal"))
                    entry["items"].setdefault(t["trait"], "uses")
                    entry["had_items"] = True
                    if slot.get("status"):
                        entry["statuses"].add(slot["status"])
    for r in records:
        if r["crate"] == r["trait_crate"]:
            continue
        entry = entry_for((r["crate"], r["trait_crate"], "normal"))
        if r.get("status"):
            entry["statuses"].add(r["status"])
        if r.get("adapts"):
            ic, ip = split(r["adapts"])
            if ic != r["crate"]:
                wired = entry_for((r["crate"], ic, "normal"))
                wired["items"][ip] = "wires"
                wired["had_items"] = True
                wired["statuses"].add(r["status"])
    for edit in spec.get("edge_edits", []):
        key = (edit["from"], edit["to"], edit["kind"])
        if edit.get("remove"):
            if key not in grouped:
                problems.append(f"edge_edits: no arrow {edit['from']} -> {edit['to']} ({edit['kind']}) to remove")
            grouped.pop(key, None)
            records = [r for r in records if not (edit["kind"] == "normal" and r["crate"] == edit["from"]
                                                  and r["trait_crate"] == edit["to"])]
            note(edit["from"], f"no longer depends on {edit['to']} ({edit['kind']}): {edit['note']}", edit["status"])
            continue
        entry = entry_for(key)
        for p in edit.get("drop", []):
            if entry["items"].pop(p, None) is None:
                problems.append(f"edge_edits: {edit['from']} -> {edit['to']} has no item {p} to drop")
        for role, ps in edit.get("add", {}).items():
            for p in ps:
                if p not in items_of.get(edit["to"], {}):
                    problems.append(f"edge_edits: no item {p} in {edit['to']}")
                entry["items"][p] = role
                entry["had_items"] = True
        entry["statuses"].add(edit["status"])
        note(edit["from"], edit["note"], edit["status"])

    edges = []
    for (frm, to, kind), entry in sorted(grouped.items()):
        implemented_here = [r for r in records if r["crate"] == frm and r["trait_crate"] == to and kind == "normal"]
        if entry["had_items"] and not entry["items"] and not implemented_here:
            continue
        roles = []
        if implemented_here:
            roles.append({"role": "implements", "detail": sorted(
                f"{r['type'] or (('adapter for ' + r['adapts']) if r.get('adapts') else 'a type of its own')} implements {r['trait']}"
                for r in implemented_here)})
        traits_here = {r["trait"] for r in implemented_here}
        uses = sorted(p for p, role in entry["items"].items() if role != "wires" and p not in traits_here)
        wires = sorted(p for p, role in entry["items"].items() if role == "wires")
        if wires:
            roles.append({"role": "wires", "detail": wires})
        if uses or not roles:
            roles.append({"role": "uses", "detail": uses})
        roles.sort(key=lambda r: ROLE_ORDER.index(r["role"]))
        edge = {"from": frm, "to": to, "kind": kind, "features": entry["features"], "optional": entry["optional"],
                "target": entry["target"], "items": sorted(entry["items"]), "roles": roles, "role": roles[0]["role"]}
        if entry["statuses"]:
            edge["statuses"] = sorted(entry["statuses"])
        edges.append(edge)

    for name, c in crates.items():
        c["implements"] = [r for r in records if r["crate"] == name]
        if name in notes and not c.get("proposed"):
            c["proposed"] = "changed"
        c["proposed_notes"] = notes.get(name, [])

    violations = [f"{e['from']} depends on the implementation crate {e['to']}" for e in edges
                  if kinds.get(e["to"]) == "implementation" and e["from"] not in programs and e["kind"] == "normal"]

    new_concerns = copy.deepcopy(concerns)
    layers = new_concerns["layers"]
    for added in spec["layers_added"]:
        at = next(i for i, l in enumerate(layers) if l["id"] == added["after"]) + 1
        layers.insert(at, {k: v for k, v in added.items() if k != "after"})
    for new in spec["new_crates"]:
        new_concerns["crates"][new["name"]] = {"layer": new["layer"], "concern": new["concern"], "status": new["status"]}

    new_assembly = copy.deepcopy(assembly)
    for program, flow in new_assembly["programs"].items():
        for step in flow["steps"] + flow.get("also", []):
            step["items"] = [f"{where(*split(ref))}::{split(ref)[1]}" for ref in step["items"]]
        for w in flow.get("wiring", []):
            if w.get("impl"):
                c, p = split(w["impl"])
                w["impl"] = f"{where(c, p)}::{p}"
        flow["adapters"] = adapter_list.get(program, [])
        flow["inferred"] = False

    graph_out = {"commit": graph["commit"], "crates": sorted(crates.values(), key=lambda c: c["name"]), "edges": edges}
    return graph_out, new_concerns, new_assembly, violations


def diff(before, after):
    """What changed from `before` to `after`: crates and arrows."""
    key = lambda e: (e["from"], e["to"], e["kind"])
    b = {key(e): e for e in before["edges"]}
    a = {key(e): e for e in after["edges"]}
    out = {"crates": [{"crate": c["name"], "notes": c["proposed_notes"]} for c in after["crates"] if c.get("proposed_notes")],
           "added": [], "removed": [], "changed": []}
    for k, e in a.items():
        if k not in b:
            e["proposed"] = "new"
            out["added"].append({"from": e["from"], "to": e["to"], "kind": e["kind"], "role": e["role"],
                                 "statuses": e.get("statuses", [])})
        else:
            old = b[k]
            if old["items"] != e["items"] or [r["role"] for r in old["roles"]] != [r["role"] for r in e["roles"]]:
                e["proposed"] = "changed"
                out["changed"].append({
                    "from": e["from"], "to": e["to"], "kind": e["kind"], "statuses": e.get("statuses", []),
                    "roles_before": [r["role"] for r in old["roles"]], "roles_after": [r["role"] for r in e["roles"]],
                    "items_gone": sorted(set(old["items"]) - set(e["items"])),
                    "items_new": sorted(set(e["items"]) - set(old["items"]))})
    for k, e in b.items():
        if k not in a:
            out["removed"].append({"from": e["from"], "to": e["to"], "kind": e["kind"], "role": e["role"]})
    return out


def item7(graph, split_spec, problems):
    """The extra moves of item 7, from config-split.json."""
    cfg = next(c for c in graph["crates"] if c["name"] == "kkt-config")
    listed = [r["item"] for r in split_spec["items"]]
    for i in cfg["items"]:
        if i["path"] not in listed:
            problems.append(f"config-split.json does not list kkt-config's {i['path']}")
    for name in set(listed) - {i["path"] for i in cfg["items"]}:
        problems.append(f"config-split.json lists {name}, which is not in kkt-config")
    status = split_spec["status"]
    moves, private = [], []
    for r in split_spec["items"]:
        if r["to"] == "kkt-config":
            continue
        if r["to"] == "split":
            limits = split_spec["limits"]
            moves.append({"item": f"kkt-config::{r['item']}", "to": limits["gathered_in"], "status": status,
                          "note": limits["note"]})
            continue
        is_private = "private" in r["how"]
        moves.append({"item": f"kkt-config::{r['item']}", "to": r["to"], "status": status, "private": is_private})
        if is_private:
            moved_users = {r.get("user_moves", {}).get(u, u) for u in r["used_by"] + r["test_only"]}
            others = moved_users - {r["to"]}
            if others:
                problems.append(f"config-split.json: {r['item']} goes to {r['to']} as private, but {', '.join(sorted(others))} use it")
            private.append({"item": f"kkt-config::{r['item']}", "status": status})
    new_items = [dict(n, status=status) for n in split_spec["limits"]["parts"]]
    return {"moves": moves, "private": private, "new_items": new_items, "status": status,
            "replace": {("kkt-config", "Limits"): split_spec["limits"]["users"]}}


def main():
    graph, concerns, assembly = load_current()
    spec = json.loads((HERE / "proposed.json").read_text())
    split_spec = json.loads((HERE / "config-split.json").read_text())
    problems = []
    table = trait_table(graph)
    for row in table:
        if row["trait"] not in spec["decisions"]:
            problems.append(f"no decision for {row['trait']}")
    for name in spec["decisions"]:
        if name not in {r["trait"] for r in table}:
            problems.append(f"a decision for {name}, which is not a trait in the map")
    proposed, p_concerns, p_assembly, violations = build(graph, concerns, assembly, spec, problems)
    changes = diff(graph, proposed)
    with7, c7, a7, violations7 = build(graph, concerns, assembly, spec, problems, item7(graph, split_spec, problems))
    changes7 = diff(proposed, with7)
    if problems:
        sys.exit("proposed.json or config-split.json does not fit the map:\n  " + "\n  ".join(sorted(set(problems))))
    for row in table:
        row.update(spec["decisions"][row["trait"]])
        row["rule"] = "shared contract crate" if len(row["consumers"]) > 2 else "declared by its consumers"
    return {
        "graph": proposed, "concerns": p_concerns, "assembly": p_assembly,
        "table": table, "changes": changes, "violations": violations,
        "item7": {"graph": with7, "concerns": c7, "assembly": a7, "changes": changes7,
                  "violations": violations7, "table": split_spec["items"], "about": split_spec["about"],
                  "limits": split_spec["limits"]},
        "statuses": spec["statuses"], "status_legend": spec["status_legend"], "rule": spec["rule"], "refinement": spec["refinement"],
        "decided_items": spec["decided_items"], "conflicts": spec["conflicts"],
        "withdrawn": spec.get("withdrawn", []), "assumptions": spec["assumptions"],
        "not_traits": spec["not_traits"], "stop_implementing": spec["stop_implementing"],
    }


if __name__ == "__main__":
    data = main()
    text = ("// Generated by docs/architecture/propose.py from map-data.js, proposed.json and\n"
            "// config-split.json. Do not edit: run the script again.\n"
            f"window.KKT_PROPOSED = {json.dumps(data, indent=1, sort_keys=True)};\n")
    out = HERE / "proposed-data.js"
    if "--check" in sys.argv[1:]:
        if not out.exists() or out.read_text() != text:
            sys.exit("docs/architecture/proposed-data.js is out of date: run python3 docs/architecture/propose.py")
        print("docs/architecture/proposed-data.js is current", file=sys.stderr)
    else:
        out.write_text(text)
        g, c, g7 = data["graph"], data["changes"], data["item7"]
        print(f"wrote {out.name}: {len(g['crates'])} crates, {len(g['edges'])} arrows; "
              f"{len(c['added'])} added, {len(c['removed'])} removed, {len(c['changed'])} changed; "
              f"{len(data['violations'])} violations. With item 7: {len(g7['graph']['edges'])} arrows, "
              f"{len(g7['changes']['added'])} added, {len(g7['changes']['removed'])} removed, "
              f"{len(g7['changes']['changed'])} changed, {len(g7['violations'])} violations", file=sys.stderr)
