#!/usr/bin/env python3
"""Writes the proposed crate map, `proposed-data.js`: the current map
(`map-data.js`, from generate.py) reorganized by the dependency-injection
rule that `proposed.json` states and applies by hand, with item 7
(`config-split.json`: kkt-config broken up by domain), which Sam accepted.

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
  (decided by Sam, or recommended pending Sam): written by hand in
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
- Each crate's kind: the current map's, from concerns.json, with the
  proposal's kind_edits, new crates and renames. kinds.py checks them, as
  generate.py does for the current map, and a crate still mixed is a
  violation.
- What changed: derived by comparing the maps.
"""

import copy
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
# kinds.py sits beside this script; `python3 -I` leaves this folder off the
# path, so it is put there by name.
sys.path.insert(0, str(HERE))
import kinds as kind_rules  # noqa: E402
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

    kinds = {n: concerns["crates"][n]["kind"] for n in crates}
    for name, kind in spec.get("kind_edits", {}).items():
        if name not in kinds:
            problems.append(f"kind_edits: no crate {name}")
        kinds[name] = kind
    for new in spec["new_crates"]:
        kinds[new["name"]] = new["kind"]
        crates[new["name"]] = {
            "name": new["name"], "lib": new["name"].replace("-", "_"), "bins": [], "path": "(proposed)",
            "dev_only": False, "released_program": False, "features": {}, "crate_doc": "",
            "third_party": [], "items": [], "slots": [], "implements": [], "contracts": [],
            "builds": [], "holders": [], "proposed": "new", "kind": new["kind"],
        }
        if new.get("program"):
            crates[new["name"]]["bins"] = [new["program"]]
            programs.add(new["name"])
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
        if move.get("feature"):
            item["feature"] = move["feature"]
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
            target = home.get((name, slot["at"])) or home.get((name, slot["at"].rsplit("::", 1)[0])) or home.get((name, owner_type))
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
            # The owner keeps the types that go with the trait when it still
            # uses them itself (kkt-store's record types, which the
            # program's adapter maps to kkt-rules' copies).
            keeps = set(decl.get("with", [])) if decl.get("owner_keeps") else set()
            for p in moving:
                if p in keeps:
                    continue
                if items_of[crate].pop(p, None) is not None:
                    redirect[(crate, p)] = list(decl["by"])
            note(crate, f"no longer declares {path}" + (f" or the {len(moving) - 1} types that go with it" if len(moving) > 1 else ""),
                 decl["status"])

    def where(crate, path, consumer=None):
        """The crate the item `path` of `crate` lives in, as seen by `consumer`."""
        if consumer and (crate, path, consumer) in declared:
            return consumer
        if (crate, path) in redirect:
            return redirect[(crate, path)][0]
        return home.get((crate, path), crate)

    def wheres(crate, path, consumer=None):
        """Every crate the item can be found in: a trait declared by two
        consumers is in both."""
        if consumer and (crate, path, consumer) in declared:
            return [consumer]
        if (crate, path) in redirect:
            return redirect[(crate, path)]
        return [home.get((crate, path), crate)]

    stop = {s["trait"]: s for s in spec["stop_implementing"]}
    records = []
    for c in graph["crates"]:
        for i in c["implements"]:
            ref = f"{i['trait_crate']}::{i['trait']}"
            impl_crate = where(c["name"], i["type"]) if i["type"] else c["name"]
            trait_crate = where(i["trait_crate"], i["trait"])
            # The strict rule covers only another crate's contract: a crate
            # may implement its own trait (FixedClock implements Clock).
            if ref in stop and impl_crate != trait_crate:
                note(impl_crate, f"{i['type'] or 'its own type'} no longer implements {ref}", stop[ref]["status"])
                continue
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
                                "adapts": a["adapts"], "status": a["status"], "kind": a.get("kind", "normal")})
            adapter_list.setdefault(program, []).append(a)
    shared_by = {}
    for sa in spec.get("shared_adapters", []):
        if kinds.get(sa["crate"]) != "composition":
            problems.append(f"shared_adapters: {sa['crate']} is not a composition crate")
        if sa["as"] not in items_of.get(sa["crate"], {}):
            problems.append(f"shared_adapters: no item {sa['as']} in {sa['crate']}")
        ic, ip = split(sa["adapts"])
        if ip not in items_of.get(ic, {}):
            problems.append(f"shared_adapters: {sa['adapts']} is not in the proposed map")
        for target in sa["to"]:
            tc, tp = split(target)
            if items_of.get(tc, {}).get(tp, {}).get("kind") != "trait":
                problems.append(f"shared_adapters: {target} is not a trait in the proposed map")
            records.append({"crate": sa["crate"], "type": sa["as"], "trait_crate": tc, "trait": tp,
                            "adapts": sa["adapts"], "status": sa["status"]})
        for user in sa["used_by"]:
            if user not in programs:
                problems.append(f"shared_adapters: {user} is not a program")
            shared_by.setdefault(user, []).append(sa)
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
                held = f"{h['crate']}::{h['type']}"
                traits = []
                for t in h.get("traits", []):
                    ref = f"{t['crate']}::{t['trait']}"
                    swap = next((hb for hb in spec.get("hold_bounds", [])
                                 if hb["type"] == held and hb["replace"] == ref), None)
                    if swap:
                        traits += [{"crate": split(w)[0], "trait": split(w)[1]} for w in swap["with"]]
                    else:
                        traits.append({"crate": where(t["crate"], t["trait"], name), "trait": t["trait"]})
                h["traits"] = [dict(u) for u in {(t["crate"], t["trait"]): t for t in traits}.values()]
                h["crate"] = where(h["crate"], h["type"])
        removed = {split(b["item"])[1] for b in spec.get("builds_removed", []) if split(b["item"])[0] == name}
        c["builds"] = [b for b in c.get("builds", []) if b["type"] in items_of[name] and b["type"] not in removed]
        for b in spec.get("builds_removed", []):
            if split(b["item"])[0] == name:
                note(name, f"{split(b['item'])[1]}: {b['note']}", b["status"])
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

    for hb in spec.get("hold_bounds", []):
        for t in hb["with"]:
            tc, tp = split(t)
            if items_of.get(tc, {}).get(tp, {}).get("kind") != "trait":
                problems.append(f"hold_bounds: {t} is not a trait in the proposed map")
        hc, hp = split(hb["type"])
        note(where(hc, hp), f"{hp} holds a value meeting " + " and ".join(hb["with"]) + f" in place of {hb['replace']}: {hb['note']}",
             hb["status"])

    for new in spec.get("new_slots", []):
        if new["crate"] not in crates:
            problems.append(f"new_slots: no crate {new['crate']}")
            continue
        for t in new["traits"]:
            tc, tp = split(t)
            if items_of.get(tc, {}).get(tp, {}).get("kind") != "trait":
                problems.append(f"new_slots: {t} is not a trait in the proposed map")
        crates[new["crate"]]["slots"].append({
            "at": new["at"], "name": new["name"], "kind": "param", "status": new["status"], "note": new["note"],
            "traits": [{"crate": split(t)[0], "trait": split(t)[1]} for t in new["traits"]], "proposed_slot": True})
        note(new["crate"], f"{new['at']} takes {new['name']}: {new['note']}", new["status"])

    for fm in spec.get("feature_moves", []):
        src, dst = crates.get(fm["from"]), crates.get(fm["to"])
        if not src or not dst or fm["feature"] not in src["features"]:
            problems.append(f"feature_moves: {fm['from']} has no feature {fm['feature']} to move to {fm['to']}")
            continue
        dst["features"][fm["feature"]] = src["features"].pop(fm["feature"])
        note(fm["from"], f"its {fm['feature']} feature moves to {fm['to']}", fm["status"])
        note(fm["to"], f"takes the {fm['feature']} feature from {fm['from']}", fm["status"])

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
            for target in wheres(e["to"], path, e["from"]):
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
                        # Kept per item, so an item dropped later takes its
                        # status with it.
                        entry.setdefault("item_statuses", {})[path] = status
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
        entry = entry_for((r["crate"], r["trait_crate"], r.get("kind", "normal")))
        if r.get("status"):
            entry["statuses"].add(r["status"])
        if r.get("adapts"):
            ic, ip = split(r["adapts"])
            if ic != r["crate"]:
                wired = entry_for((r["crate"], ic, r.get("kind", "normal")))
                wired["items"][ip] = "wires"
                wired["had_items"] = True
                wired["statuses"].add(r["status"])
    # Who takes a shared adapter: its programs wire it, and tests that
    # need a store wire it in theirs.
    for sa in spec.get("shared_adapters", []):
        for user, kind in [(u, "normal") for u in sa["used_by"]] + [(t, "dev") for t in sa["tests"]]:
            entry = entry_for((user, sa["crate"], kind))
            entry["items"][sa["as"]] = "wires"
            entry["had_items"] = True
            entry["statuses"].add(sa["status"])
        note(sa["crate"], f"{sa['as']} adapts {sa['adapts']} to {' and '.join(sa['to'])}, for " +
             ", ".join(sa["used_by"]) + "; and the tests of " + ", ".join(sa["tests"]), sa["status"])
        for t in sa["tests"]:
            note(t, f"its tests wire {sa['crate']}'s {sa['as']} instead of an adapter of their own (item 8)", sa["status"])
        if sa.get("caveat"):
            for crate, text in sa["caveat"].items():
                note(crate, text, sa["status"])

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
        implemented_here = [r for r in records if r["crate"] == frm and r["trait_crate"] == to
                            and r.get("kind", "normal") == kind]
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
        statuses = entry["statuses"] | {st for p, st in entry.get("item_statuses", {}).items() if p in entry["items"]}
        if statuses:
            edge["statuses"] = sorted(statuses)
        edges.append(edge)

    for name, c in crates.items():
        c["implements"] = [r for r in records if r["crate"] == name]
        if name in notes and not c.get("proposed"):
            c["proposed"] = "changed"
        c["proposed_notes"] = notes.get(name, [])

    allowed = programs | {n for n, k in kinds.items() if k == "composition"}
    violations = [f"{e['from']} depends on the effect crate {e['to']}" for e in edges
                  if kinds.get(e["to"]) == "effect" and e["from"] not in allowed and e["kind"] == "normal"]

    new_concerns = copy.deepcopy(concerns)
    layers = new_concerns["layers"]
    for added in spec["layers_added"]:
        at = next(i for i, l in enumerate(layers) if l["id"] == added["after"]) + 1
        layers.insert(at, {k: v for k, v in added.items() if k != "after"})
    for new in spec["new_crates"]:
        new_concerns["crates"][new["name"]] = {"layer": new["layer"], "concern": new["concern"], "status": new["status"]}

    for name, edit in list(spec.get("concern_edits", {}).items()) + list(extra.get("concern_edits", {}).items()):
        if name not in new_concerns["crates"]:
            problems.append(f"concern_edits: no crate {name}")
            continue
        new_concerns["crates"][name] = dict(new_concerns["crates"][name], concern=edit["concern"], status=edit["status"])
        if "must_not" in edit:
            new_concerns["crates"][name]["must_not"] = edit["must_not"]
        if "layer" in edit:
            new_concerns["crates"][name]["layer"] = edit["layer"]

    new_assembly = copy.deepcopy(assembly)
    a_spec = spec.get("assembly", {"edits": {}, "add": {}})
    for program, flow in new_assembly["programs"].items():
        edits = a_spec["edits"].get(program, {})
        dropped = {(w["impl"], w["crate"]) for w in edits.get("wiring_drop", [])}
        present = {(x.get("impl"), x["crate"]) for x in flow.get("wiring", [])}
        if dropped and not dropped & present:
            problems.append(f"assembly: none of {program}'s wiring_drop is in its current wiring")
        flow["wiring"] = [w for w in flow.get("wiring", []) if (w.get("impl"), w["crate"]) not in dropped]
        for step in flow["steps"] + flow.get("also", []):
            step["items"] = [f"{where(*split(ref))}::{split(ref)[1]}" for ref in step["items"]]
        for w in flow.get("wiring", []):
            if w.get("impl"):
                c, p = split(w["impl"])
                w["impl"] = f"{where(c, p)}::{p}"
        for title, new_step in edits.get("steps", {}).items():
            found = [st for st in flow["steps"] if st["title"] == title]
            if not found:
                problems.append(f"assembly: {program} has no step {title}")
            for st in found:
                st.update(new_step)
                st["proposed_step"] = True
        flow["wiring"] += [dict(w) for w in edits.get("wiring_add", [])]
    for program, flow in a_spec["add"].items():
        if program not in programs:
            problems.append(f"assembly: {program} is not a program")
        new_assembly["programs"][program] = copy.deepcopy(flow)
    # Flows name crates as the proposed map does; renames apply below.
    renamed_back = {rn["to"]: rn["from"] for rn in spec.get("renames", [])}

    def split_named(ref):
        c, p = split(ref)
        return renamed_back.get(c, c), p

    for program, flow in new_assembly["programs"].items():
        # Everything a flow names must be in the proposed map.
        for step in flow["steps"] + flow.get("also", []):
            for ref in step["items"]:
                c, p = split_named(ref)
                if p not in items_of.get(c, {}):
                    problems.append(f"assembly: {program}'s step {step['title']} names {ref}, which is not in the proposed map")
        for w in flow.get("wiring", []):
            if w.get("impl"):
                c, p = split_named(w["impl"])
                if p not in items_of.get(c, {}):
                    problems.append(f"assembly: {program} wires {w['impl']}, which is not in the proposed map")
            slots = {(sl["at"], sl["name"]) for sl in crates.get(renamed_back.get(w["crate"], w["crate"]), {}).get("slots", [])}
            for t in w["to"]:
                if (t["at"], t["param"]) not in slots:
                    problems.append(f"assembly: {program} wires into {w['crate']}::{t['at']} ({t['param']}), which is not a slot in the proposed map")
        flow["adapters"] = []
        for a in adapter_list.get(program, []) + [dict(sa, **{"in": sa["crate"]}) for sa in shared_by.get(program, [])]:
            size = 0
            for target in (a["to"] if isinstance(a["to"], list) else [a["to"]]):
                tc, tp = split(target)
                size += len(items_of.get(tc, {}).get(tp, {}).get("methods", []))
            flow["adapters"].append(dict(a, methods=size, large=a.get("large", False) or size >= 20))
        adapted = {a["adapts"]: a for a in flow["adapters"]}
        for w in flow.get("wiring", []):
            a = adapted.get(w.get("impl"))
            if a:
                w["through_adapter"] = a["to"] if isinstance(a["to"], list) else [a["to"]]
        flow["inferred"] = False

    # Renames: the crate and every mention of it.
    for rn in spec.get("renames", []):
        old, new = rn["from"], rn["to"]
        if old not in crates:
            problems.append(f"renames: no crate {old}")
            continue
        c = crates.pop(old)
        c["name"] = new
        c["lib"] = new.replace("-", "_")
        c["renamed_from"] = old
        c["proposed_notes"] = c.get("proposed_notes", []) + [{"text": f"renamed from {old}: {rn['concern']}", "status": rn["status"]}]
        if c.get("proposed") != "new":
            c["proposed"] = "changed"
        kinds[new] = rn["kind"]
        kinds.pop(old, None)
        crates[new] = c
        for other in crates.values():
            for r in other["implements"]:
                for k in ("crate", "trait_crate"):
                    if r.get(k) == old:
                        r[k] = new
            for slot in other["slots"]:
                for t in slot["traits"]:
                    if t["crate"] == old:
                        t["crate"] = new
            for ctr in other["contracts"]:
                for i in ctr["implementors"]:
                    if i["crate"] == old:
                        i["crate"] = new
        for e in edges:
            for k in ("from", "to"):
                if e[k] == old:
                    e[k] = new
            for role in e["roles"]:
                role["detail"] = [d.replace(f"{old}::", f"{new}::") for d in role["detail"]]
        for other in crates.values():
            for r in other["implements"]:
                if r.get("adapts", "").startswith(f"{old}::"):
                    r["adapts"] = new + r["adapts"][len(old):]
        info = new_concerns["crates"].pop(old)
        new_concerns["crates"][new] = dict(info, layer=rn["layer"], concern=rn["concern"], status=rn["status"],
                                           title=new)
        for flow in new_assembly["programs"].values():
            for step in flow["steps"] + flow.get("also", []):
                step["items"] = [f"{new}::{split(r)[1]}" if split(r)[0] == old else r for r in step["items"]]
            for w in flow.get("wiring", []):
                if w.get("impl") and split(w["impl"])[0] == old:
                    w["impl"] = f"{new}::{split(w['impl'])[1]}"
            for a in flow.get("adapters", []):
                if split(a["adapts"])[0] == old:
                    a["adapts"] = f"{new}::{split(a['adapts'])[1]}"
    allowed = programs | {n for n, k in kinds.items() if k == "composition"}
    violations = [f"{e['from']} depends on the effect crate {e['to']}" for e in edges
                  if kinds.get(e["to"]) == "effect" and e["from"] not in allowed and e["kind"] == "normal"]
    # The strict rule: only programs and compositions implement another
    # crate's contract.
    for c in crates.values():
        for r in c["implements"]:
            if r["trait_crate"] != c["name"] and not r.get("adapts") and c["name"] not in programs:
                violations.append(f"{c['name']}'s {r['type'] or 'own type'} implements {r['trait_crate']}::{r['trait']}")

    for name, c in crates.items():
        c["kind"] = kinds[name]
        new_concerns["crates"][name]["kind"] = kinds[name]
    unknown = kind_rules.unknown(crates.values(), kinds)
    if unknown:
        problems.append(f"no kind, or one kinds.py does not know, for: {', '.join(unknown)}")
    kind_mismatches, mixed = kind_rules.check(list(crates.values()), edges, kinds, mixed_allowed=False)
    violations += mixed

    graph_out = {"commit": graph["commit"], "crates": sorted(crates.values(), key=lambda c: c["name"]), "edges": edges,
                 "kind_mismatches": kind_mismatches}
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
        move = {"item": f"kkt-config::{r['item']}", "to": r["to"], "status": status, "private": is_private}
        if "test-visible" in r["how"]:
            move["feature"] = "testing"
        moves.append(move)
        if is_private:
            moved_users = {r.get("user_moves", {}).get(u, u) for u in r["used_by"] + r["test_only"]}
            others = moved_users - {r["to"]}
            if others:
                problems.append(f"config-split.json: {r['item']} goes to {r['to']} as private, but {', '.join(sorted(others))} use it")
            private.append({"item": f"kkt-config::{r['item']}", "status": status})
    new_items = [dict(n, status=status) for n in split_spec["limits"]["parts"]]
    return {"moves": moves, "private": private, "new_items": new_items, "status": status,
            "concern_edits": {n: dict(e, status=status) for n, e in split_spec.get("concern_edits", {}).items()},
            "replace": {("kkt-config", "Limits"): split_spec["limits"]["users"]}}


def renamed_rows(rows, renames):
    """Item 7's table, naming each crate as the proposed map does."""
    new_name = {rn["from"]: rn["to"] for rn in renames}
    out = copy.deepcopy(rows)
    for r in out:
        r["to"] = new_name.get(r["to"], r["to"])
        r["used_by"] = [new_name.get(u, u) for u in r["used_by"]]
        r["test_only"] = [new_name.get(u, u) for u in r["test_only"]]
        if "user_moves" in r:
            r["user_moves"] = {new_name.get(k, k): new_name.get(v, v) for k, v in r["user_moves"].items()}
    return out


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
    # Item 7 is part of the proposal (Sam accepted it, msg 1979).
    proposed, p_concerns, p_assembly, violations = build(graph, concerns, assembly, spec, problems,
                                                         item7(graph, split_spec, problems))
    changes = diff(graph, proposed)
    if problems:
        sys.exit("proposed.json or config-split.json does not fit the map:\n  " + "\n  ".join(sorted(set(problems))))
    for row in table:
        row.update(spec["decisions"][row["trait"]])
        row["rule"] = "shared contract crate" if len(row["consumers"]) > 2 else "declared by its consumers"
    return {
        "graph": proposed, "concerns": p_concerns, "assembly": p_assembly,
        "table": table, "changes": changes, "violations": violations,
        "config_split": {"table": renamed_rows(split_spec["items"], spec.get("renames", [])), "about": split_spec["about"], "limits": split_spec["limits"],
                         "status": split_spec["status"]},
        "statuses": spec["statuses"], "status_legend": spec["status_legend"], "rule": spec["rule"],
        "strict": spec["strict"], "rejected": spec.get("rejected", []),
        "decided_items": spec["decided_items"], "open_questions": spec["open_questions"], "conflicts": spec["conflicts"],
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
        g, c = data["graph"], data["changes"]
        print(f"wrote {out.name}: {len(g['crates'])} crates, {len(g['edges'])} arrows; "
              f"{len(c['added'])} added, {len(c['removed'])} removed, {len(c['changed'])} changed; "
              f"{len(data['violations'])} violations", file=sys.stderr)
