"""What sort of crate each crate is, and the checks a map's kinds must pass.

The kinds are written by hand in concerns.json (for the current map) and
proposed.json (for the proposal); this module only checks them against
what the map shows, and reports a crate whose kind does not fit rather than
changing it. generate.py and propose.py both use it.
"""

import re

# "contract and effect" is a merged crate (phase 13): a contract in the open
# and its one implementation behind `real`, which only programs turn on.
MERGED = "contract and effect"
# "logic and effect" (phase 14): logic in the open and the effect that only
# carries out its plans behind `real`, which only programs turn on, so the
# logic builds and tests without the effect.
LOGIC_EFFECT = "logic and effect"
KINDS = ["contract", MERGED, "logic", LOGIC_EFFECT, "effect", "composition", "test support", "mixed"]

# Third-party crates that reach the machine: a crate that uses one does
# I/O of its own.
IO_CRATES = {"rusqlite", "reqwest", "rand", "libc", "tracing-subscriber"}

# The standard library's machine work, as paths under `std`: files,
# the environment, processes, sockets, the terminal, and reading the
# clock. Value types such as `net::SocketAddr` or `process::ExitCode` are
# not machine work, and nor is reading the command line (`env::args`):
# parsing its own arguments is the program's.
STD_IO = ["fs", "env", "os::unix::fs", "process::Command", "process::Stdio", "process::exit", "process::id",
          "net::TcpListener", "net::TcpStream", "net::UdpSocket", "io::stdin", "io::stdout", "io::stderr",
          "io::IsTerminal", "time::SystemTime::now", "time::Instant::now", "thread::sleep",
          "path::absolute"]
# Macros that write to the terminal.
PRINT_MACROS = ["print", "println", "eprint", "eprintln"]
# Methods of `Path` and `PathBuf` that ask the file system about a path:
# whether it is there, what it is, where it really leads. The scan reads
# text, not types, so it counts a call with no arguments (`p.exists()`),
# as these methods take none, or one spelled through the type
# (`Path::exists(p)`). A call with arguments (`host.exists(p)`) is some
# other type's method, such as an effect crate's, and is not counted.
PATH_PROBES = ["exists", "try_exists", "is_file", "is_dir", "is_symlink", "metadata",
               "symlink_metadata", "canonicalize", "read_dir", "read_link"]


ARGUMENTS = ["env::args", "env::args_os"]


def is_std_io(path):
    """Whether a path under `std` is machine work."""
    if any(path == p or path.startswith(p + "::") for p in ARGUMENTS):
        return False
    return any(path == p or path.startswith(p + "::") for p in STD_IO)


def path_probes(text):
    """The `Path` methods in PATH_PROBES that `text` calls, as `Path::name`."""
    names = "|".join(PATH_PROBES)
    found = set(re.findall(rf"\.\s*({names})\s*\(\s*\)", text))
    found |= set(re.findall(rf"\b(?:Path|PathBuf)\s*::\s*({names})\s*\(", text))
    return sorted(f"Path::{m}" for m in found)


def unknown(crates, kind_of):
    """Crates with no kind, or a kind this module does not know."""
    return sorted(c["name"] for c in crates if kind_of.get(c["name"]) not in KINDS)


def is_contract(kind):
    """Whether a crate of this kind holds a contract that any crate may
    implement: a contract crate, or a merged crate's open contract."""
    return kind in ("contract", MERGED)


def reaches_effect(edge, kind_of):
    """Whether an arrow depends on an effect: on an effect crate, or on a
    merged crate's effect, by turning its `real` on. Depending on a merged
    crate without `real` is depending on its contract or logic alone."""
    kind = kind_of.get(edge["to"])
    return kind == "effect" or (kind in (MERGED, LOGIC_EFFECT) and "real" in edge.get("features", []))


def slot_feature(crate, slot):
    """The feature a slot is behind: the one the map records for it, or
    else its owner's. A slot on `Type::new` or a field of `Type` is behind
    whatever `Type` is behind (a map built before slots recorded their
    feature, such as the proposal's base)."""
    if slot.get("feature"):
        return slot["feature"]
    features = {i["path"]: i.get("feature") for i in crate.get("items", [])}
    owner = slot["at"] if slot.get("kind") == "field" else slot["at"].rsplit("::", 1)[0]
    return features.get(slot["at"]) or features.get(owner)


def beyond_values(crate, edges, skip_real=False):
    """What a contract crate does beyond pure behaviour on its own values:
    behaviour through an injected dependency (a slot outside `testing`),
    and wiring other crates' implementations. A contract may compute on
    its own value types (Time arithmetic, query checks, search matching);
    an effect is checked apart. With `skip_real`, a slot behind `real` is
    the implementation's, not the contract's (a merged crate)."""
    found = []
    for slot in crate["slots"]:
        if slot["at"].startswith("testing::") or not slot["traits"]:
            continue
        if skip_real and slot_feature(crate, slot) == "real":
            continue
        traits = ", ".join(t["trait"] for t in slot["traits"])
        found.append(f"{slot['at']} takes {slot['name']} ({traits})")
    for e in edges:
        if e["from"] == crate["name"] and e["kind"] == "normal" and any(r["role"] == "wires" for r in e["roles"]):
            found.append(f"wires {e['to']}'s implementations")
    return sorted(set(found))


def turns_on_real(crate, edges):
    """Whether `crate` has a `real` feature, or turns one on in a crate it
    depends on in normal code."""
    if "real" in crate["features"]:
        return True
    return any(e["from"] == crate["name"] and e["kind"] == "normal" and "real" in e.get("features", [])
               for e in edges)


def io_outside_real(crate):
    """The I/O dependencies a crate has other than optional ones that only
    its `real` turns on."""
    optional = crate.get("optional_third_party", {})
    return sorted(d for d in IO_CRATES & set(crate["third_party"]) if optional.get(d) != ["real"])


def merged_mismatches(crate, edges, crates_by_name):
    """Where a crate marked "contract and effect" is not one. It differs
    from "mixed" in two things that must hold together: its effect is the
    implementation of its own contract, behind `real`, and only programs
    turn that `real` on."""
    name, found = crate["name"], []
    does = beyond_values(crate, edges, skip_real=True)
    if does:
        found.append(f"its contract does more than compute on its own values: {'; '.join(does)}")
    if "real" not in crate["features"]:
        found.append("it has no `real` feature")
    io = io_outside_real(crate)
    if io:
        found.append(f"it has I/O dependencies outside `real`: {', '.join(io)}")
    if not any(i["trait_crate"] == name and i.get("type") and not i["type"].startswith("testing::")
               for i in crate.get("implements", [])):
        found.append("nothing in it implements its own contract")
    found += others_turn_on_real(name, edges, crates_by_name)
    return [f"{name} is marked {MERGED} but {f}" for f in found]


def others_turn_on_real(name, edges, crates_by_name):
    """A merged crate's `real` is for programs only: the crates other than
    programs that turn it on, as a finding."""
    on = sorted(e["from"] for e in edges
                if e["to"] == name and e["kind"] == "normal" and "real" in e.get("features", [])
                and not crates_by_name.get(e["from"], {}).get("bins"))
    return [f"crates other than programs turn its `real` on: {', '.join(on)}"] if on else []


def logic_effect_mismatches(crate, edges, crates_by_name):
    """Where a crate marked "logic and effect" is not one. Its logic, outside
    `real`, is logic: no machine work in its own code, no I/O dependency,
    and no other crate's `real`. Its effect is behind its own `real`, in
    modules only that feature brings, and only programs turn that on."""
    name, found = crate["name"], []
    if "real" not in crate["features"]:
        found.append("it has no `real` feature")
    io = io_outside_real(crate)
    if io:
        found.append(f"it has I/O dependencies outside `real`: {', '.join(io)}")
    behind = set(crate.get("real_files", []))
    work = sorted(f for f in crate.get("machine_work", {}) if f not in behind)
    if work:
        found.append(f"it does machine work outside `real`, in {', '.join(work)}")
    others = sorted(e["to"] for e in edges
                    if e["from"] == name and e["kind"] == "normal" and "real" in e.get("features", []))
    if others:
        found.append(f"it turns on another crate's `real`: {', '.join(others)}")
    found += others_turn_on_real(name, edges, crates_by_name)
    return [f"{name} is marked {LOGIC_EFFECT} but {f}" for f in found]


def check(crates, edges, kind_of, mixed_allowed):
    """(mismatches, violations): where a crate's kind and what the map
    shows of it disagree, and, when `mixed_allowed` is false, every crate
    that is still mixed."""
    mismatches, violations = [], []
    for c in crates:
        kind = kind_of[c["name"]]
        if kind == "contract":
            does = beyond_values(c, edges)
            if does:
                mismatches.append(f"{c['name']} is marked contract but does more than compute on its own values: {'; '.join(does)}")
            if turns_on_real(c, edges) or IO_CRATES & set(c["third_party"]):
                mismatches.append(f"{c['name']} is marked contract but has an effect: a `real` feature or an I/O dependency")
        if kind == MERGED:
            mismatches += merged_mismatches(c, edges, {x["name"]: x for x in crates})
        if kind == LOGIC_EFFECT:
            mismatches += logic_effect_mismatches(c, edges, {x["name"]: x for x in crates})
        if kind == "logic" and turns_on_real(c, edges):
            mismatches.append(f"{c['name']} is marked logic but turns on `real`")
        # Named tool wrappers remain effects even though they also own their
        # tool's decisions, tested over a private runner (Sam, phase 12).
        # The contract stays effect-free; implementing it directly is allowed.
        if kind == "effect" and not turns_on_real(c, edges) and not IO_CRATES & set(c["third_party"]):
            mismatches.append(f"{c['name']} is marked effect but has no `real` feature and no I/O dependency")
        if kind == "composition":
            # Composition only wires and adapts: machine work in its own
            # code, or a third-party crate that does it, is an effect of
            # its own.
            work = sorted(c.get("machine_work", {}))
            io = sorted(IO_CRATES & set(c["third_party"]))
            if work or io:
                parts = ([f"in {', '.join(work)}"] if work else []) + ([f"with {', '.join(io)}"] if io else [])
                text = f"{c['name']} is marked composition but does machine work itself: {'; '.join(parts)}"
                (mismatches if mixed_allowed else violations).append(text)
        if kind == "composition" and not c["bins"] and "real" in c["features"]:
            text = f"{c['name']} is marked composition but its own `real` feature touches the machine"
            (mismatches if mixed_allowed else violations).append(text)
        if kind == "mixed" and not mixed_allowed:
            violations.append(f"{c['name']} is still mixed: a contract or logic together with a real effect")
    return mismatches, violations
