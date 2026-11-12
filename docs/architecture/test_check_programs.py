"""check-programs.py: machine work in a composition crate fails unless the plan's temporary breaks list it."""

import importlib.util
import json
import unittest
from unittest.mock import patch
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



def merged(slots=(), features=("real", "testing"), third_party=(), optional=None, implements=None):
    """A crate marked contract and effect, as the map records it."""
    return {"name": "kkt-tool", "bins": [], "features": {f: [] for f in features},
            "slots": list(slots), "third_party": list(third_party),
            "optional_third_party": optional or {}, "machine_work": {},
            "implements": [{"crate": "kkt-tool", "type": "Tool", "trait_crate": "kkt-tool", "trait": "Run"}]
            if implements is None else implements}


def edge(source, target, features=(), kind="normal"):
    return {"from": source, "to": target, "kind": kind, "features": list(features), "roles": []}


class ContractAndEffect(unittest.TestCase):
    """A merged crate: its contract in the open, its one implementation behind `real` (phase 13)."""

    kinds = check.kinds

    def mismatches(self, crate, edges=(), others=()):
        crates = [crate, *others]
        kind_of = {c["name"]: c.get("kind", self.kinds.MERGED) for c in crates}
        found, _ = self.kinds.check(crates, list(edges), kind_of, mixed_allowed=False)
        return found

    def test_a_well_formed_merged_crate_passes(self):
        slot = {"at": "Tool::new", "name": "browser", "traits": [{"crate": "kkt-browser", "trait": "Browser"}],
                "feature": "real"}
        crate = merged(slots=[slot], third_party=["reqwest"], optional={"reqwest": ["real"]})
        program = {"name": "kkt", "bins": ["kkt"], "kind": "composition", "features": {}, "slots": [],
                   "third_party": [], "machine_work": {}, "implements": []}
        self.assertEqual(self.mismatches(crate, [edge("kkt", "kkt-tool", ["real"])], [program]), [])

    def test_a_slot_outside_real_is_the_contracts_and_fails(self):
        slot = {"at": "Tool::new", "name": "browser", "traits": [{"crate": "kkt-browser", "trait": "Browser"}]}
        self.assertEqual(self.mismatches(merged(slots=[slot])),
                         ["kkt-tool is marked contract and effect but its contract does more than compute on "
                          "its own values: Tool::new takes browser (Browser)"])

    def test_it_needs_a_real_feature(self):
        self.assertIn("kkt-tool is marked contract and effect but it has no `real` feature",
                      self.mismatches(merged(features=["testing"])))

    def test_an_io_dependency_outside_real_fails(self):
        crate = merged(third_party=["reqwest", "rmcp"], optional={"rmcp": ["real"]})
        self.assertEqual(self.mismatches(crate),
                         ["kkt-tool is marked contract and effect but it has I/O dependencies outside `real`: reqwest"])

    def test_only_a_stand_in_implementing_the_contract_fails(self):
        stand_in = [{"crate": "kkt-tool", "type": "testing::Recorder", "trait_crate": "kkt-tool", "trait": "Run"}]
        self.assertEqual(self.mismatches(merged(implements=stand_in)),
                         ["kkt-tool is marked contract and effect but nothing in it implements its own contract"])

    def test_a_library_turning_its_real_on_fails(self):
        user = {"name": "kkt-user", "bins": [], "kind": "logic", "features": {}, "slots": [], "third_party": [],
                "machine_work": {}, "implements": []}
        self.assertIn("kkt-tool is marked contract and effect but crates other than programs turn its "
                      "`real` on: kkt-user",
                      self.mismatches(merged(), [edge("kkt-user", "kkt-tool", ["real"])], [user]))

    def test_only_an_arrow_turning_real_on_reaches_the_effect(self):
        kind_of = {"kkt-tool": self.kinds.MERGED, "kkt-os": "effect", "kkt-api": "contract"}
        self.assertFalse(self.kinds.reaches_effect(edge("a", "kkt-tool"), kind_of))
        self.assertTrue(self.kinds.reaches_effect(edge("a", "kkt-tool", ["real"]), kind_of))
        self.assertTrue(self.kinds.reaches_effect(edge("a", "kkt-os"), kind_of))
        self.assertFalse(self.kinds.reaches_effect(edge("a", "kkt-api", ["testing"]), kind_of))

    def test_a_merged_crates_contract_may_be_implemented_elsewhere(self):
        self.assertTrue(self.kinds.is_contract(self.kinds.MERGED))
        self.assertTrue(self.kinds.is_contract("contract"))
        self.assertFalse(self.kinds.is_contract("effect"))

class ProposedMachineWork(unittest.TestCase):
    """The frozen program work must still have a real destination after a merge."""

    @classmethod
    def setUpClass(cls):
        module_spec = importlib.util.spec_from_file_location(
            "propose", Path(__file__).resolve().parent / "propose.py")
        cls.propose = importlib.util.module_from_spec(module_spec)
        module_spec.loader.exec_module(cls.propose)

    def proposal_with_container_kind(self, kind):
        path = self.propose.HERE / "proposed.json"
        original_read = Path.read_text
        data = json.loads(original_read(path))
        data["kind_edits"]["kkt-container"] = kind

        def read(candidate, *args, **kwargs):
            return json.dumps(data) if candidate == path else original_read(candidate, *args, **kwargs)

        with patch.object(Path, "read_text", read):
            return self.propose.main()

    def test_merged_container_accepts_its_historical_machine_work(self):
        page = self.proposal_with_container_kind("contract and effect")
        self.assertFalse(page["violations"])

    def test_pure_contract_still_refuses_machine_work(self):
        with self.assertRaisesRegex(SystemExit, "sends its machine work to kkt-container, which is not an effect crate"):
            self.proposal_with_container_kind("contract")


if __name__ == "__main__":
    unittest.main()
