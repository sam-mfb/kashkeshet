"""check-programs.py: machine work in a composition crate fails unless the plan's temporary breaks list it."""

import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "check_programs", Path(__file__).resolve().parent / "check-programs.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

PLAN = """
## Temporary breaks

| Break | Introduced by | Removed by |
|---|---|---|
| `kkt` does machine work in `src/a.rs` and `src/b.rs`, with `libc` | before | 7.3 |
| `kkt-dev` does machine work in `src/main.rs` | before | 7.5 |
| kkt-web depends on kkt-shutdown, an extra arrow | 1.4 | 6.3 |

## Differences from the proposed map

| `kkt` does machine work in `src/c.rs` | not a temporary break | |
"""

CONCERNS = {"crates": {"kkt": {"kind": "composition"}, "kkt-dev": {"kind": "composition"},
                       "kkt-lib": {"kind": "logic"}}}


def crate(name, work=(), third_party=(), bins=True, features=()):
    return {"name": name, "machine_work": {f: [] for f in work}, "third_party": list(third_party),
            "bins": ["x"] if bins else [], "features": list(features)}


class CheckPrograms(unittest.TestCase):
    def test_the_plan_lists_files_and_libraries_by_crate(self):
        self.assertEqual(check.listed(PLAN), {
            "kkt": {"files": {"src/a.rs", "src/b.rs"}, "libs": {"libc"}},
            "kkt-dev": {"files": {"src/main.rs"}, "libs": set()},
        })

    def test_listed_work_passes(self):
        graph = {"crates": [crate("kkt", ["src/a.rs", "src/b.rs"], ["libc"]),
                            crate("kkt-dev", ["src/main.rs"]), crate("kkt-lib", ["src/x.rs"])]}
        self.assertEqual(check.problems(graph, CONCERNS, check.listed(PLAN)), [])

    def test_unlisted_work_fails(self):
        graph = {"crates": [crate("kkt", ["src/a.rs", "src/b.rs", "src/c.rs"], ["libc", "reqwest"]),
                            crate("kkt-dev", ["src/main.rs"])]}
        self.assertEqual(check.problems(graph, CONCERNS, check.listed(PLAN)), [
            "kkt does machine work in src/c.rs, which the plan's temporary breaks do not list",
            "kkt uses reqwest, which reaches the machine, and the plan's temporary breaks do not list it",
        ])

    def test_a_listed_file_that_no_longer_does_work_fails(self):
        graph = {"crates": [crate("kkt", ["src/a.rs"], ["libc"]), crate("kkt-dev")]}
        self.assertEqual(check.problems(graph, CONCERNS, check.listed(PLAN)), [
            "kkt: the plan lists machine work in src/b.rs, which no longer does any; remove it from the row",
            "kkt-dev: the plan lists machine work in src/main.rs, which no longer does any; remove it from the row",
        ])

    def test_a_library_composition_with_its_own_real_feature_fails(self):
        concerns = {"crates": {"kkt-server": {"kind": "composition"}}}
        graph = {"crates": [crate("kkt-server", bins=False, features=["real"])]}
        self.assertEqual(check.problems(graph, concerns, {}),
                         ["kkt-server is composition but has a `real` feature of its own"])


if __name__ == "__main__":
    unittest.main()


class PathProbes(unittest.TestCase):
    """A program asking the file system about a path counts as machine work."""

    def test_a_path_method_with_no_arguments_is_a_probe(self):
        text = "if path.exists() { let real = dir.join(x).canonicalize()?; }\nlet m = p . metadata ( );"
        self.assertEqual(check.kinds.path_probes(text),
                         ["Path::canonicalize", "Path::exists", "Path::metadata"])

    def test_a_call_through_the_type_is_a_probe(self):
        self.assertEqual(check.kinds.path_probes("Path::is_file(&p) && PathBuf::is_dir(&q)"),
                         ["Path::is_dir", "Path::is_file"])

    def test_another_types_method_with_arguments_is_not(self):
        # An effect crate's adapter, called with the path to ask about.
        self.assertEqual(check.kinds.path_probes("self.system.exists(&path).unwrap_or(false); host.is_file(p)"), [])
        self.assertEqual(check.kinds.path_probes("let exists = true; fn is_file() {}"), [])

    def test_std_path_absolute_is_machine_work(self):
        self.assertTrue(check.kinds.is_std_io("path::absolute"))
        self.assertFalse(check.kinds.is_std_io("path::Path"))
