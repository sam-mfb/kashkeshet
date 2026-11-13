#!/usr/bin/env python3
"""The refactor's scorecard: how far the code has come toward the proposed map.

During the refactor map-data.js stays at the map the proposal was made
from, since propose.py applies the proposal to that map; the proposal itself
is updated as deliberate differences merge (refactor-plan.md). This script
builds the map from the code into target/tmp (generate.py --out), compares it
with the proposed map, and prints:

- crates: the proposed crates that exist, those still missing, and those that
  exist but the proposal removes;
- items: the proposed map's public items found in their proposed crate, out
  of all of them, matched by crate and path;
- arrows: the proposed normal arrows present and missing, and the normal
  arrows in the code that the proposal does not have;
- wiring: what each program, or kkt-server and kkt-agent-commands where
  they wire things themselves, injects into which slot, from the assembly
  data (assembly.json, and the proposal's flows): the proposed wirings
  present and missing, and those the code has that the proposal does not.
  Both lists are written by hand, so this catches the proposal falling
  behind assembly.json, not assembly.json falling behind the code:
  generate.py checks only that the names assembly.json uses exist, and
  many slots that take a trait (most of them pass-through parts) have no
  wiring entry. 100% here does not mean every wiring in the code is
  recorded;
- rules: kinds.py's checks, with the proposed kinds, and the proposal's own
  rules (only programs and compositions depend on effect crates, in code and
  in tests, and only programs turn on a merged crate's `real`, which kinds.py
  checks; a crate implements another's
  contract only if that crate is a contract crate or a merged one, or it is a
  program, a composition or test support), run on the code.

  python3 docs/architecture/progress.py            the scorecard
  python3 docs/architecture/progress.py --list     and every missing or extra entry
  python3 docs/architecture/progress.py --map FILE  use a map already built

Run it before and after a step; the step's pull request records both.
"""

import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
import kinds  # noqa: E402

BUILT = ROOT / "target" / "tmp" / "scorecard" / "map-data.js"


def load(path, names):
    """The objects a generated .js file assigns to window.<name>."""
    text = path.read_text()
    found = {}
    for name in names:
        start = text.index(f"window.{name} = ") + len(f"window.{name} = ")
        found[name], _ = json.JSONDecoder().raw_decode(text, start)
    return found


def build():
    """The map built from the code, written to target/tmp."""
    subprocess.run([sys.executable, "-I", str(HERE / "generate.py"), "--out", str(BUILT)], check=True)
    return BUILT


def wiring_key(injector, w):
    """One wiring, as the scorecard compares it: who injects, what (an
    implementation, or the injector's own type, named by the words before
    its first comma), into which crate, and which slots."""
    what = w.get("impl") or w["own"].split(",")[0]
    return (injector, what, w["crate"], tuple(sorted((t["at"], t["param"]) for t in w.get("to", []))))


def wiring(current, proposed):
    """The wirings of two assemblies, compared: those of `proposed` found in
    `current`, and the missing and extra ones, as text."""
    def of(assembly):
        return {wiring_key(name, w)
                for section in ("programs", "injectors")
                for name, flow in (assembly.get(section) or {}).items()
                for w in flow.get("wiring", [])}

    def text(k):
        injector, what, crate, slots = k
        return f"{injector}: {what} into {crate} ({', '.join(f'{a}.{p}' for a, p in slots)})"
    now, then = of(current), of(proposed)
    return {"present": len(now & then), "total": len(then),
            "missing": sorted(text(k) for k in then - now), "extra": sorted(text(k) for k in now - then)}


def score(current, current_concerns, proposed, proposed_concerns, current_assembly=None, proposed_assembly=None):
    crates = {c["name"]: c for c in current["crates"]}
    goal = {c["name"]: c for c in proposed["crates"]}
    result = {}

    result["crates"] = {
        "present": sorted(set(goal) & set(crates)),
        "missing": sorted(set(goal) - set(crates)),
        "extra": sorted(set(crates) - set(goal)),
    }

    have = {(c["name"], i["path"]) for c in current["crates"] for i in c["items"]}
    want = {(c["name"], i["path"]) for c in proposed["crates"] for i in c["items"]}
    result["items"] = {"found": len(want & have), "total": len(want),
                       "missing": sorted(f"{c}::{p}" for c, p in want - have)}

    def arrows(graph):
        return {(e["from"], e["to"]) for e in graph["edges"] if e["kind"] == "normal"}
    now, then = arrows(current), arrows(proposed)
    result["arrows"] = {"present": len(now & then), "total": len(then),
                        "missing": sorted(f"{a} -> {b}" for a, b in then - now),
                        "extra": sorted(f"{a} -> {b}" for a, b in now - then)}

    # The proposed kind of each crate the code has; a crate the proposal does
    # not have keeps today's.
    kind_of = {n: (proposed_concerns["crates"].get(n) or current_concerns["crates"].get(n) or {}).get("kind")
               for n in crates}
    mismatches, mixed = kinds.check(list(crates.values()), current["edges"], kind_of, mixed_allowed=False)
    allowed = {n for n, c in crates.items() if c["bins"]} | {n for n, k in kind_of.items() if k == "composition"}
    # A merged crate is an effect only to an arrow that turns its `real` on.
    breaks = [f"{e['from']} depends on the effect crate {e['to']}" + (" in its tests" if e["kind"] == "dev" else "")
              for e in current["edges"]
              if kinds.reaches_effect(e, kind_of) and e["kind"] in ("normal", "dev") and e["from"] not in allowed]
    for c in crates.values():
        for r in c["implements"]:
            if r["trait_crate"] == c["name"] or c["name"] in allowed or kind_of.get(c["name"]) == "test support":
                continue
            if not kinds.is_contract(kind_of.get(r["trait_crate"])):
                breaks.append(f"{c['name']}'s {r['type'] or 'own type'} implements {r['trait_crate']}::{r['trait']}")
    result["rules"] = {"kinds": sorted(mismatches + mixed), "proposal": sorted(set(breaks))}
    if current_assembly is not None and proposed_assembly is not None:
        result["wiring"] = wiring(current_assembly, proposed_assembly)
    return result


def show(result, listing):
    c, i, a, r = result["crates"], result["items"], result["arrows"], result["rules"]
    total = len(c["present"]) + len(c["missing"])
    lines = [
        f"crates: {len(c['present'])} of {total} proposed crates exist; "
        f"{len(c['missing'])} missing; {len(c['extra'])} to remove",
        f"items: {i['found']} of {i['total']} proposed items are in their proposed crate",
        f"arrows: {a['present']} of {a['total']} proposed arrows exist; {len(a['missing'])} missing; "
        f"{len(a['extra'])} extra",
        f"rules: {len(r['kinds'])} kind checks fail; {len(r['proposal'])} breaks of the proposal's rules",
    ]
    w = result.get("wiring")
    if w:
        lines.insert(3, f"wiring: {w['present']} of {w['total']} proposed wirings exist; {len(w['missing'])} missing; "
                        f"{len(w['extra'])} extra")
    if listing:
        def section(title, entries):
            lines.append(f"\n{title} ({len(entries)}):")
            lines.extend(f"  {e}" for e in entries)
        section("crates missing", c["missing"])
        section("crates to remove", c["extra"])
        section("items not yet in their proposed crate", i["missing"])
        section("arrows missing", a["missing"])
        section("arrows extra", a["extra"])
        if w:
            section("wirings missing", w["missing"])
            section("wirings extra", w["extra"])
        section("kind checks failing", r["kinds"])
        section("breaks of the proposal's rules", r["proposal"])
    print("\n".join(lines))


def main():
    args = sys.argv[1:]
    built = Path(args[args.index("--map") + 1]) if "--map" in args else build()
    current = load(built, ["KKT_GRAPH", "KKT_CONCERNS", "KKT_ASSEMBLY"])
    proposed = load(HERE / "proposed-data.js", ["KKT_PROPOSED"])["KKT_PROPOSED"]
    result = score(current["KKT_GRAPH"], current["KKT_CONCERNS"], proposed["graph"], proposed["concerns"],
                   current["KKT_ASSEMBLY"], proposed["assembly"])
    show(result, "--list" in args)


if __name__ == "__main__":
    main()
