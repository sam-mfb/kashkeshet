"""propose.py: late_moves move an item after every earlier move, may give it a new path and a feature, and its users follow it."""

import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("propose", Path(__file__).resolve().parent / "propose.py")
propose = importlib.util.module_from_spec(spec)
spec.loader.exec_module(propose)


def crate(name, items, kind="logic"):
    return {"name": name, "layer": "core", "bins": [], "features": {}, "third_party": [], "slots": [],
            "implements": [], "machine_work": {},
            "items": [{"path": p, "kind": "fn", "doc": ""} for p in items]}


GRAPH = {
    "commit": "test",
    "layers": ["core"],
    "crates": [crate("kkt-a", ["keep", "keys::moved", "plain"]), crate("kkt-b", []), crate("kkt-user", [])],
    "edges": [{"from": "kkt-user", "to": "kkt-a", "kind": "normal", "items": ["keep", "keys::moved", "plain"],
               "role": "uses", "roles": [], "features": [], "optional": False, "target": None, "test_items": []}],
}
CONCERNS = {"layers": [{"name": "core"}], "crates": {n: {"kind": "logic", "concern": "", "must_not": "", "layer": "core"}
                       for n in ("kkt-a", "kkt-b", "kkt-user")}}
ASSEMBLY = {"programs": {}, "injectors": []}


def build(late_moves):
    spec = {"new_crates": [], "moves": [], "declared": [], "stop_implementing": [], "adapters": {},
            "layers_added": [], "late_moves": late_moves}
    problems = []
    graph, _, _, _ = propose.build(GRAPH, CONCERNS, ASSEMBLY, spec, problems)
    return graph, problems


def items(graph, name):
    return {i["path"]: i for c in graph["crates"] if c["name"] == name for i in c["items"]}


def uses(graph, frm, to):
    return sorted(p for e in graph["edges"] if e["from"] == frm and e["to"] == to and e["kind"] == "normal"
                  for p in e["items"])


class LateMoves(unittest.TestCase):
    def test_a_late_move_with_as_gives_the_item_its_new_path_and_its_users_follow(self):
        graph, problems = build([{"item": "kkt-a::keys::moved", "to": "kkt-b", "as": "moved",
                                  "status": "decided", "note": "test"}])
        self.assertEqual(problems, [])
        self.assertIn("moved", items(graph, "kkt-b"))
        self.assertNotIn("keys::moved", items(graph, "kkt-a"))
        self.assertEqual(uses(graph, "kkt-user", "kkt-b"), ["moved"])
        self.assertEqual(uses(graph, "kkt-user", "kkt-a"), ["keep", "plain"])

    def test_a_late_move_without_as_keeps_the_path(self):
        graph, problems = build([{"item": "kkt-a::plain", "to": "kkt-b", "status": "decided", "note": "test"}])
        self.assertEqual(problems, [])
        self.assertIn("plain", items(graph, "kkt-b"))
        self.assertEqual(uses(graph, "kkt-user", "kkt-b"), ["plain"])

    def test_a_late_move_may_put_the_item_behind_a_feature(self):
        graph, problems = build([{"item": "kkt-a::plain", "to": "kkt-b", "feature": "real",
                                  "status": "decided", "note": "test"}])
        self.assertEqual(problems, [])
        self.assertEqual(items(graph, "kkt-b")["plain"].get("feature"), "real")

    def test_a_late_move_to_no_crate_is_a_problem(self):
        _, problems = build([{"item": "kkt-a::plain", "to": "kkt-nowhere", "status": "decided", "note": "test"}])
        self.assertIn("late_moves: no crate kkt-nowhere", problems)

    def test_a_late_move_of_no_item_is_a_problem(self):
        _, problems = build([{"item": "kkt-a::absent", "to": "kkt-b", "status": "decided", "note": "test"}])
        self.assertIn("late_moves: no item absent in kkt-a", problems)


if __name__ == "__main__":
    unittest.main()
