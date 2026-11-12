"""What sort of crate each crate is, and the checks a map's kinds must pass.

The kinds are written by hand in concerns.json (for the current map) and
proposed.json (for the proposal); this module only checks them against
what the map shows, and reports a crate whose kind does not fit rather than
changing it. generate.py and propose.py both use it.
"""

import re

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
            does = beyond_values(c, edges)
            if does:
                mismatches.append(f"{c['name']} is marked contract but does more than compute on its own values: {'; '.join(does)}")
            if turns_on_real(c, edges) or IO_CRATES & set(c["third_party"]):
                mismatches.append(f"{c['name']} is marked contract but has an effect: a `real` feature or an I/O dependency")
        if kind == "logic" and turns_on_real(c, edges):
            mismatches.append(f"{c['name']} is marked logic but turns on `real`")
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
