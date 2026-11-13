"""check-programs.py: machine work in a composition crate fails unless the plan's temporary breaks list it; progress.py compares wiring."""

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

    def test_a_slot_on_a_type_behind_real_is_the_implementations(self):
        # A map from before slots recorded their feature: the owner's decides.
        slot = {"at": "AdminRemote::new", "name": "clock", "kind": "param",
                "traits": [{"crate": "kkt-clock", "trait": "Clock"}]}
        crate = merged(slots=[slot])
        crate["items"] = [{"path": "AdminRemote", "feature": "real"}]
        self.assertEqual(self.mismatches(crate), [])
        crate["items"] = [{"path": "AdminRemote"}]
        self.assertEqual(self.mismatches(crate),
                         ["kkt-tool is marked contract and effect but its contract does more than compute on "
                          "its own values: AdminRemote::new takes clock (Clock)"])

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

def logic_effect(features=("real",), third_party=(), optional=None, machine_work=None, real_files=()):
    """A crate marked logic and effect, as the map records it."""
    return {"name": "kkt-plan", "bins": [], "features": {f: [] for f in features}, "slots": [],
            "third_party": list(third_party), "optional_third_party": optional or {},
            "machine_work": machine_work or {}, "real_files": list(real_files), "implements": []}


class LogicAndEffect(unittest.TestCase):
    """Logic in the open, and the effect that carries out its plans behind `real` (phase 14)."""
    kinds = check.kinds

    def mismatches(self, crate, edges=(), others=()):
        crates = [crate, *others]
        kind_of = {c["name"]: c.get("kind", self.kinds.LOGIC_EFFECT) for c in crates}
        found, _ = self.kinds.check(crates, list(edges), kind_of, mixed_allowed=False)
        return found

    def program(self):
        return {"name": "kkt", "bins": ["kkt"], "kind": "composition", "features": {}, "slots": [],
                "third_party": [], "machine_work": {}, "implements": []}

    def test_a_well_formed_crate_passes(self):
        crate = logic_effect(third_party=["libc"], optional={"libc": ["real"]},
                             machine_work={"src/files.rs": ["fs::write"]}, real_files=["src/files.rs"])
        self.assertEqual(self.mismatches(crate, [edge("kkt", "kkt-plan", ["real"])], [self.program()]), [])

    def test_it_needs_a_real_feature(self):
        self.assertEqual(self.mismatches(logic_effect(features=())),
                         ["kkt-plan is marked logic and effect but it has no `real` feature"])

    def test_machine_work_outside_real_fails(self):
        crate = logic_effect(machine_work={"src/lib.rs": ["fs::read"], "src/files.rs": ["fs::write"]},
                             real_files=["src/files.rs"])
        self.assertEqual(self.mismatches(crate),
                         ["kkt-plan is marked logic and effect but it does machine work outside `real`, in src/lib.rs"])

    def test_an_io_dependency_outside_real_fails(self):
        crate = logic_effect(third_party=["libc", "rand"], optional={"libc": ["real"], "rand": ["real", "testing"]})
        self.assertEqual(self.mismatches(crate),
                         ["kkt-plan is marked logic and effect but it has I/O dependencies outside `real`: rand"])

    def test_turning_on_another_crates_real_fails(self):
        other = {"name": "kkt-os", "bins": [], "kind": "effect", "features": {"real": []}, "slots": [],
                 "third_party": [], "machine_work": {}, "implements": []}
        self.assertIn("kkt-plan is marked logic and effect but it turns on another crate's `real`: kkt-os",
                      self.mismatches(logic_effect(), [edge("kkt-plan", "kkt-os", ["real"])], [other]))

    def test_a_library_turning_its_real_on_fails(self):
        user = {"name": "kkt-user", "bins": [], "kind": "logic", "features": {}, "slots": [], "third_party": [],
                "machine_work": {}, "implements": []}
        self.assertIn("kkt-plan is marked logic and effect but crates other than programs turn its `real` on: kkt-user",
                      self.mismatches(logic_effect(), [edge("kkt-user", "kkt-plan", ["real"])], [user]))

    def test_only_an_arrow_turning_real_on_reaches_the_effect(self):
        kind_of = {"kkt-plan": self.kinds.LOGIC_EFFECT}
        self.assertFalse(self.kinds.reaches_effect(edge("a", "kkt-plan"), kind_of))
        self.assertTrue(self.kinds.reaches_effect(edge("a", "kkt-plan", ["real"]), kind_of))

    def test_its_logic_is_no_contract(self):
        self.assertFalse(self.kinds.is_contract(self.kinds.LOGIC_EFFECT))


generate_spec = importlib.util.spec_from_file_location("generate", Path(__file__).resolve().parent / "generate.py")
generate = importlib.util.module_from_spec(generate_spec)
generate_spec.loader.exec_module(generate)


class FilesBehindReal(unittest.TestCase):
    """generate.py records which files only `real` brings (phase 14)."""

    def test_the_conditions_that_hold_only_with_real(self):
        for cfg in ['feature = "real"', 'any(feature = "real", test)', 'any(test, feature = "real")',
                    'all(unix, feature = "real")']:
            self.assertTrue(generate.behind_real(cfg), cfg)
        for cfg in ['test', 'feature = "testing"', 'any(test, feature = "real", feature = "testing")',
                    'not(feature = "real")', 'all(unix, not(feature = "real"))', 'unix']:
            self.assertFalse(generate.behind_real(cfg), cfg)

    def test_modules_behind_real_and_what_they_declare(self):
        root = Path(__file__).resolve().parents[2] / "target" / "tmp" / "architecture-tests" / "real-files"
        files = {
            "src/lib.rs": '#[cfg(feature = "real")]\npub mod files;\n#[cfg(any(feature = "real", test))]\nmod plan;\nmod open;\n',
            "src/files.rs": "mod inner;\n",
            "src/files/inner.rs": "",
            "src/plan.rs": "",
            "src/open.rs": "",
        }
        for name, text in files.items():
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(text)
        found = generate.real_module_files(root, [root / n for n in files])
        self.assertEqual(sorted(str(f.relative_to(root)) for f in found),
                         ["src/files.rs", "src/files/inner.rs", "src/plan.rs"])


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



progress_spec = importlib.util.spec_from_file_location("progress", Path(__file__).resolve().parent / "progress.py")
progress = importlib.util.module_from_spec(progress_spec)
progress_spec.loader.exec_module(progress)


class Wiring(unittest.TestCase):
    """The scorecard compares what each program injects where, in the code
    and in the proposal."""

    MCP = {"impl": "kkt-agent-client::Mcp", "crate": "kkt-agent-commands", "to": [{"at": "AgentParts", "param": "servers"}]}
    OWN = {"own": "ClientHost, adapting kkt-client-config-os's OsHost", "crate": "kkt-operate",
           "to": [{"at": "OperateParts", "param": "clients"}]}

    def test_the_same_wiring_matches(self):
        both = {"programs": {"kkt": {"wiring": [self.MCP, self.OWN]}}}
        self.assertEqual(progress.wiring(both, both), {"present": 2, "total": 2, "missing": [], "extra": []})

    def test_a_program_type_is_named_by_its_first_words(self):
        reworded = dict(self.OWN, own="ClientHost, worded otherwise")
        got = progress.wiring({"programs": {"kkt": {"wiring": [self.OWN]}}}, {"programs": {"kkt": {"wiring": [reworded]}}})
        self.assertEqual((got["present"], got["missing"], got["extra"]), (1, [], []))

    def test_a_wiring_the_proposal_lacks_is_extra_and_one_the_code_lacks_is_missing(self):
        stale = {"impl": "kkt-agent-client::Remote", "crate": "kkt-watch", "to": [{"at": "run", "param": "server"}]}
        got = progress.wiring({"programs": {"kkt": {"wiring": [self.MCP]}}}, {"programs": {"kkt": {"wiring": [stale]}}})
        self.assertEqual(got["present"], 0)
        self.assertEqual(got["missing"], ["kkt: kkt-agent-client::Remote into kkt-watch (run.server)"])
        self.assertEqual(got["extra"], ["kkt: kkt-agent-client::Mcp into kkt-agent-commands (AgentParts.servers)"])

    def test_injectors_count_and_slots_tell_wirings_apart(self):
        other_slot = dict(self.MCP, to=[{"at": "AgentParts", "param": "clients"}])
        got = progress.wiring({"injectors": {"kkt-server": {"wiring": [self.MCP]}}},
                              {"injectors": {"kkt-server": {"wiring": [other_slot]}}})
        self.assertEqual((got["present"], len(got["missing"]), len(got["extra"])), (0, 1, 1))

if __name__ == "__main__":
    unittest.main()
