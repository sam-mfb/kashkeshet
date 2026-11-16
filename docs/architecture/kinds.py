"""What sort of crate each crate is, and the checks a map's kinds must pass.

The kinds are written by hand in concerns.json; this module only checks
them against what the map shows, and reports a crate whose kind does not
fit rather than changing it. It also checks the rules that rest on the
kinds (`rule_breaks`). generate.py and check-programs.py use it.
"""

import re

# One kind per crate (phase 16). "mixed" is a crate that is still a
# contract or logic together with a real effect: the current map only.
KINDS = ["contract", "logic", "effect", "composition", "test support", "mixed"]

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
# tokio's machine work, as paths under `tokio`: files, sockets, processes,
# signals, the terminal, and waiting on or reading the clock.
TOKIO_IO = ["fs", "net", "process", "signal", "io::stdin", "io::stdout", "io::stderr",
            "time::sleep", "time::sleep_until", "time::interval", "time::Instant"]
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


def is_tokio_io(path):
    """Whether a path under `tokio` is machine work."""
    return any(path == p or path.startswith(p + "::") for p in TOKIO_IO)


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
    implement."""
    return kind == "contract"


def reaches_effect(edge, kind_of):
    """Whether an arrow depends on an effect: on an effect crate."""
    return kind_of.get(edge["to"]) == "effect"


def beyond_values(crate, edges):
    """What a contract crate does beyond pure behaviour on its own values:
    behaviour through an injected dependency (a slot outside `testing`),
    and wiring other crates' implementations. A contract may compute on
    its own value types (Time arithmetic, query checks, search matching);
    an effect is checked apart."""
    found = []
    for slot in crate["slots"]:
        if slot["at"].startswith("testing::") or not slot["traits"]:
            continue
        traits = ", ".join(t["trait"] for t in slot["traits"])
        found.append(f"{slot['at']} takes {slot['name']} ({traits})")
    for e in edges:
        if e["from"] == crate["name"] and e["kind"] == "normal" and any(r["role"] == "wires" for r in e["roles"]):
            found.append(f"wires {e['to']}'s implementations")
    return sorted(set(found))


def does_io(crate):
    """Where a crate touches the machine: its own code's machine work, by
    file, and the I/O libraries it uses. Empty when it touches nothing."""
    work = sorted(crate.get("machine_work", {}))
    io = sorted(IO_CRATES & set(crate["third_party"]))
    return ([f"in {', '.join(work)}"] if work else []) + ([f"with {', '.join(io)}"] if io else [])


def check(crates, edges, kind_of, mixed_allowed):
    """(mismatches, violations): where a crate's kind and what the map
    shows of it disagree, and, when `mixed_allowed` is false, every crate
    that is still mixed."""
    mismatches, violations = [], []
    for c in crates:
        kind = kind_of[c["name"]]
        io = does_io(c)
        if kind == "contract":
            does = beyond_values(c, edges)
            if does:
                mismatches.append(f"{c['name']} is marked contract but does more than compute on its own values: {'; '.join(does)}")
            if io:
                mismatches.append(f"{c['name']} is marked contract but touches the machine: {'; '.join(io)}")
        if kind == "logic" and io:
            mismatches.append(f"{c['name']} is marked logic but touches the machine: {'; '.join(io)}")
        # An effect crate touches the machine: in its own code, or through
        # an I/O library. Named tool wrappers remain effects even though they
        # also own their tool's decisions, tested over a private runner (Sam,
        # phase 12).
        if kind == "effect" and not io:
            mismatches.append(f"{c['name']} is marked effect but touches the machine nowhere the map can see")
        if kind == "composition" and io:
            # Composition only wires and adapts: machine work in its own
            # code, or a third-party crate that does it, is an effect of
            # its own.
            text = f"{c['name']} is marked composition but does machine work itself: {'; '.join(io)}"
            (mismatches if mixed_allowed else violations).append(text)
        if kind == "mixed" and not mixed_allowed:
            violations.append(f"{c['name']} is still mixed: a contract or logic together with a real effect")
    return mismatches, violations


def rule_breaks(crates, edges, kind_of):
    """Where the code breaks the rules that rest on the kinds: only programs
    depend on an effect crate, in code or in tests (phase 16); only
    programs depend on a composition crate in code (phase 16's follow-up); a crate
    implements another crate's trait only if that crate is a contract, or
    the implementer is a program, a composition or test support; and no
    crate is mixed. Sorted text, one line per break."""
    programs = {c["name"] for c in crates if c["bins"]}
    allowed = programs | {n for n, k in kind_of.items() if k == "composition"}
    breaks = [f"{e['from']} depends on the effect crate {e['to']}" + (" in its tests" if e["kind"] == "dev" else "")
              for e in edges
              if reaches_effect(e, kind_of) and e["kind"] in ("normal", "dev") and e["from"] not in programs]
    breaks += [f"{e['from']} depends on the composition crate {e['to']}"
               for e in edges
               if kind_of.get(e["to"]) == "composition" and e["kind"] == "normal" and e["from"] not in programs]
    for c in crates:
        for r in c["implements"]:
            if r["trait_crate"] == c["name"] or c["name"] in allowed or kind_of.get(c["name"]) == "test support":
                continue
            if not is_contract(kind_of.get(r["trait_crate"])):
                breaks.append(f"{c['name']}'s {r['type'] or 'own type'} implements {r['trait_crate']}::{r['trait']}")
        if kind_of.get(c["name"]) == "mixed":
            breaks.append(f"{c['name']} is mixed: a contract or logic together with a real effect")
    return sorted(set(breaks))
