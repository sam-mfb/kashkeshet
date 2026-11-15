"""Tests of kinds.py's rule checks, which generate.py puts on the page."""

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
    def breaks(self, crates, edges):
        return kinds.rule_breaks(crates, edges, KIND_OF)

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

    def test_implementing_a_contract_is_allowed_and_a_logic_crates_trait_is_not(self):
        on_contract = {"trait_crate": "contract", "trait": "Clock", "type": "Fixed"}
        on_logic = {"trait_crate": "other-logic", "trait": "Disk", "type": None}
        own = {"trait_crate": "logic", "trait": "Own", "type": "Mine"}
        crates = [crate("logic", implements=[on_contract, on_logic, own]), crate("other-logic"),
                  crate("stand-in", implements=[on_logic]), crate("prog", implements=[on_logic])]
        self.assertEqual(self.breaks(crates, []), ["logic's own type implements other-logic::Disk"])

    def test_a_mixed_crate_breaks(self):
        self.assertEqual(kinds.rule_breaks([crate("odd")], [], {"odd": "mixed"}),
                         ["odd is mixed: a contract or logic together with a real effect"])


if __name__ == "__main__":
    unittest.main()
