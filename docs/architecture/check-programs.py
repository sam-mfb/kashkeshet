#!/usr/bin/env python3
"""CI's check that the programs do no machine work (the audit's F1).

It builds the map from the code (generate.py --out, into target/tmp) and
fails on any crate concerns.json marks as composition, the programs
included, whose own code does machine work (kinds.py's reading of the map:
a third-party crate that reaches the machine, or a std path for files, the
environment, processes, sockets, the terminal or the clock, a print
macro, or a `Path` method that asks the file system about a path), or which has a `real` feature of its own without being a program.

Machine work the refactor has not yet moved out is allowed only where the
plan's "Temporary breaks" table lists it, as a row naming the crate and
each file or library, such as:

  | `kkt-keymode` does machine work in `src/main.rs` | before the refactor | 7.5 |

Anything else fails, and so does a listed file or library that no longer
does machine work, so a step that moves the work out also removes the row.

  python3 docs/architecture/check-programs.py              build the map and check
  python3 docs/architecture/check-programs.py --map FILE   check a map already built
"""

import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))
import kinds  # noqa: E402
from progress import load  # noqa: E402

BUILT = ROOT / "target" / "tmp" / "check-programs" / "map-data.js"
PLAN = HERE / "refactor-plan.md"
ROW = re.compile(r"^\| `([^`]+)` does machine work (.*?) \|")


def listed(plan_text):
    """Each crate's machine work the temporary-breaks table allows: the
    files (`in ...`) and the libraries (`with ...`) its rows name."""
    table = plan_text.split("## Temporary breaks", 1)[1].split("\n## ", 1)[0]
    allowed = {}
    for line in table.splitlines():
        m = ROW.match(line)
        if m:
            crate, what = m.group(1), m.group(2)
            files = re.findall(r"`([^`]+)`", what.split(" with ")[0])
            libs = re.findall(r"`([^`]+)`", what.split(" with ", 1)[1]) if " with " in what else []
            entry = allowed.setdefault(crate, {"files": set(), "libs": set()})
            entry["files"].update(files)
            entry["libs"].update(libs)
    return allowed


def problems(graph, concerns, allowed):
    """What fails the check, as lines."""
    out = []
    kind_of = {n: c.get("kind") for n, c in concerns["crates"].items()}
    for c in graph["crates"]:
        if kind_of.get(c["name"]) != "composition":
            continue
        name = c["name"]
        work = set(c.get("machine_work", {}))
        libs = kinds.IO_CRATES & set(c["third_party"])
        ok = allowed.get(name, {"files": set(), "libs": set()})
        for f in sorted(work - ok["files"]):
            out.append(f"{name} does machine work in {f}, which the plan's temporary breaks do not list")
        for lib in sorted(libs - ok["libs"]):
            out.append(f"{name} uses {lib}, which reaches the machine, and the plan's temporary breaks do not list it")
        for f in sorted(ok["files"] - work):
            out.append(f"{name}: the plan lists machine work in {f}, which no longer does any; remove it from the row")
        for lib in sorted(ok["libs"] - libs):
            out.append(f"{name}: the plan lists {lib}, which it no longer uses; remove it from the row")
        if not c["bins"] and "real" in c["features"]:
            out.append(f"{name} is composition but has a `real` feature of its own")
    for name in sorted(set(allowed) - {c["name"] for c in graph["crates"] if kind_of.get(c["name"]) == "composition"}):
        out.append(f"the plan lists machine work for {name}, which is not a composition crate in the map")
    return out


def main():
    args = sys.argv[1:]
    if "--map" in args:
        built = Path(args[args.index("--map") + 1])
    else:
        subprocess.run([sys.executable, "-I", str(HERE / "generate.py"), "--out", str(BUILT)], check=True)
        built = BUILT
    current = load(built, ["KKT_GRAPH", "KKT_CONCERNS"])
    found = problems(current["KKT_GRAPH"], current["KKT_CONCERNS"], listed(PLAN.read_text()))
    if found:
        print("check-programs: the programs do machine work the plan does not allow:")
        print("\n".join(f"  {p}" for p in found))
        sys.exit(1)
    print("check-programs: no program does machine work beyond the plan's temporary breaks")


if __name__ == "__main__":
    main()
