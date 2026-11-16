"""Tests of kinds.py's rule checks, which generate.py puts on the page."""

import importlib.util
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import kinds  # noqa: E402


def crate(name, bins=(), implements=()):
    return {"name": name, "bins": list(bins), "implements": list(implements)}


def edge(source, target, kind="normal", features=()):
    return {"from": source, "to": target, "kind": kind, "features": list(features)}


KIND_OF = {
    "prog": "composition", "logic": "logic", "other-logic": "logic", "fx": "effect",
    "contract": "contract", "stand-in": "test support",
}


class RuleBreaks(unittest.TestCase):
    def breaks(self, crates, edges, layer_of=None):
        return kinds.rule_breaks(crates, edges, KIND_OF, layer_of if layer_of is not None else {})

    def test_a_logic_crate_on_an_effect_crate_breaks_in_code_and_in_tests(self):
        crates = [crate("logic"), crate("fx")]
        self.assertEqual(self.breaks(crates, [edge("logic", "fx")]), ["logic depends on the effect crate fx"])
        self.assertEqual(self.breaks(crates, [edge("logic", "fx", "dev")]),
                         ["logic depends on the effect crate fx in its tests"])

    def test_a_build_dependency_on_an_effect_is_not_a_break(self):
        self.assertEqual(self.breaks([crate("logic"), crate("fx")], [edge("logic", "fx", "build")]), [])

    def test_only_programs_may_depend_on_effects(self):
        crates = [crate("main", bins=["main"]), crate("prog"), crate("fx"), crate("stand-in")]
        self.assertEqual(self.breaks(crates, [edge("main", "fx"), edge("main", "fx", "dev")]), [])
        # A composition, test support or another effect crate may not (phase 16).
        self.assertEqual(self.breaks(crates + [crate("fx2")], [edge("prog", "fx"), edge("stand-in", "fx", "dev"),
                                                               edge("fx2", "fx")]),
                         ["fx2 depends on the effect crate fx", "prog depends on the effect crate fx",
                          "stand-in depends on the effect crate fx in its tests"])

    def test_only_programs_may_depend_on_a_composition_in_code(self):
        crates = [crate("main", bins=["main"]), crate("prog"), crate("fx"), crate("logic")]
        self.assertEqual(self.breaks(crates, [edge("main", "prog"), edge("logic", "prog", "dev"),
                                              edge("logic", "prog", "build")]), [])
        self.assertEqual(self.breaks(crates, [edge("fx", "prog"), edge("logic", "prog")]),
                         ["fx depends on the composition crate prog", "logic depends on the composition crate prog"])

    def test_implementing_a_contract_is_allowed_and_a_logic_crates_trait_is_not(self):
        on_contract = {"trait_crate": "contract", "trait": "Clock", "type": "Fixed"}
        on_logic = {"trait_crate": "other-logic", "trait": "Disk", "type": None}
        own = {"trait_crate": "logic", "trait": "Own", "type": "Mine"}
        crates = [crate("logic", implements=[on_contract, on_logic, own]), crate("other-logic"),
                  crate("stand-in", implements=[on_logic]), crate("prog", implements=[on_logic])]
        self.assertEqual(self.breaks(crates, []), ["logic's own type implements other-logic::Disk"])

    def test_a_platform_crate_depends_in_code_only_on_platform_crates(self):
        layer_of = {"logic": "platform", "other-logic": "platform", "contract": "core", "stand-in": "dev"}
        crates = [crate("logic"), crate("other-logic"), crate("contract"), crate("stand-in")]
        # Tests and build scripts may reach outside the layer, and a crate
        # outside it may depend on a platform crate.
        self.assertEqual(self.breaks(crates, [edge("logic", "other-logic"), edge("logic", "stand-in", "dev"),
                                              edge("logic", "contract", "build"), edge("contract", "logic")],
                                     layer_of), [])
        self.assertEqual(self.breaks(crates, [edge("logic", "contract")], layer_of),
                         ["logic is a platform crate but depends on contract, which is in the core layer"])

    def test_a_mixed_crate_breaks(self):
        self.assertEqual(kinds.rule_breaks([crate("odd")], [], {"odd": "mixed"}, {}),
                         ["odd is mixed: a contract or logic together with a real effect"])


generate_spec = importlib.util.spec_from_file_location("generate", Path(__file__).resolve().parent / "generate.py")
generate = importlib.util.module_from_spec(generate_spec)
generate_spec.loader.exec_module(generate)


def scanned(name, kind, files):
    """A crate as the map records it, its machine work found by generate.py
    in the given sources, the way generate.py finds it."""
    root = Path(__file__).resolve().parents[2] / "target" / "tmp" / "architecture-tests" / name
    for rel, text in files.items():
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        (root / rel).write_text(text)
    paths = [root / rel for rel in files]
    stand_ins = generate.testing_module_files(root, paths)
    work = {}
    for path in paths:
        found = generate.machine_work_in(generate.split_test_modules(path.read_text())[0])
        if found and path not in stand_ins:
            work[str(path.relative_to(root))] = found
    return {"name": name, "bins": [], "features": {}, "slots": [], "third_party": [],
            "machine_work": work, "implements": []}


class KindsFromTheSources(unittest.TestCase):
    """The two additions of phase 16, from source text to kind check."""

    def mismatches(self, crate, kind):
        found, _ = kinds.check([crate], [], {crate["name"]: kind}, mixed_allowed=False)
        return found

    def test_an_effect_crate_with_only_tokio_machine_work_fits(self):
        crate = scanned("kkt-signals-os", "effect", {"src/lib.rs": (
            "pub async fn stopped() -> std::io::Result<()> {\n"
            "    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;\n"
            "    term.recv().await;\n    Ok(())\n}\n")})
        self.assertEqual(crate["machine_work"]["src/lib.rs"][0], "tokio::signal::unix::SignalKind::terminate")
        self.assertEqual(self.mismatches(crate, "effect"), [])
        # Read as logic, the same crate touches the machine.
        self.assertEqual(self.mismatches(crate, "logic"),
                         ["kkt-signals-os is marked logic but touches the machine: in src/lib.rs"])

    def test_a_contract_whose_testing_module_sleeps_fits(self):
        crate = scanned("kkt-api", "contract", {
            "src/lib.rs": '#[cfg(any(test, feature = "testing"))]\npub mod testing;\npub trait Store {}\n',
            "src/testing/mod.rs": "mod conformance;\n",
            "src/testing/conformance.rs": "pub fn wait() { std::thread::sleep(std::time::Duration::from_millis(1)); }\n",
        })
        self.assertEqual(crate["machine_work"], {})
        self.assertEqual(self.mismatches(crate, "contract"), [])


if __name__ == "__main__":
    unittest.main()
