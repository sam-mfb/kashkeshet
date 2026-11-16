"""Tests of groups.py: which crates the map draws together."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import groups  # noqa: E402


def crate(name, implements=(), bins=()):
    return {"name": name, "bins": list(bins),
            "implements": [{"trait_crate": t, "trait": "T", "type": None} for t in implements]}


LAYER_OF = {"db": "data", "db-sqlite": "data", "service": "ops", "launchd": "ops", "systemd": "ops",
            "service-os": "ops", "rules": "core", "api-a": "core", "api-b": "core", "web": "surfaces",
            "sim": "dev", "prog": "programs", "far": "foundations", "chain": "data"}
KIND_OF = {"sim": "test support", "prog": "composition"}


class Groups(unittest.TestCase):
    def groups(self, crates):
        return groups.groups(crates, LAYER_OF, KIND_OF)

    def test_an_implementation_goes_under_the_one_contract_in_its_layer(self):
        crates = [crate("db"), crate("db-sqlite", ["db"]), crate("service"),
                  crate("launchd", ["service"]), crate("systemd", ["service"])]
        self.assertEqual(self.groups(crates), {"db": ["db-sqlite"], "service": ["launchd", "systemd"]})

    def test_a_name_alone_groups_nothing(self):
        self.assertEqual(self.groups([crate("service"), crate("service-os")]), {})

    def test_several_contracts_in_its_layer_group_nothing(self):
        crates = [crate("api-a"), crate("api-b"), crate("rules", ["api-a", "api-b"])]
        self.assertEqual(self.groups(crates), {})

    def test_a_contract_in_another_layer_does_not_count(self):
        self.assertEqual(self.groups([crate("db"), crate("far", ["db"])]), {})
        # One in its layer and others elsewhere still groups.
        self.assertEqual(self.groups([crate("db"), crate("web"), crate("db-sqlite", ["db", "web"])]),
                         {"db": ["db-sqlite"]})

    def test_its_own_traits_do_not_count(self):
        self.assertEqual(self.groups([crate("db", ["db"])]), {})

    def test_programs_and_test_support_are_never_grouped(self):
        LAYER_OF_SAME = dict(LAYER_OF, sim="core", prog="core")
        crates = [crate("api-a"), crate("sim", ["api-a"]), crate("prog", ["api-a"], bins=["prog"])]
        self.assertEqual(groups.groups(crates, LAYER_OF_SAME, KIND_OF), {})

    def test_a_partner_is_never_a_head(self):
        crates = [crate("db"), crate("db-sqlite", ["db"]), crate("chain", ["db-sqlite"])]
        self.assertEqual(self.groups(crates), {"db": ["db-sqlite"]})


if __name__ == "__main__":
    unittest.main()
