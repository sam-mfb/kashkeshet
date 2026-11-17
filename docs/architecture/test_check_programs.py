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


class KindsAndRules(unittest.TestCase):
    def test_a_map_with_none_passes(self):
        self.assertEqual(check.kinds_and_rules({"kind_mismatches": [], "rule_breaks": []}), [])

    def test_a_kind_that_does_not_fit_or_a_broken_rule_fails(self):
        graph = {"kind_mismatches": ["kkt-x is marked logic but touches the machine: with libc"],
                 "rule_breaks": ["kkt-y depends on the effect crate kkt-z"]}
        self.assertEqual(check.kinds_and_rules(graph), [
            "kind: kkt-x is marked logic but touches the machine: with libc",
            "rule: kkt-y depends on the effect crate kkt-z",
        ])


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



def one(name="kkt-x", kind="logic", machine_work=None, third_party=(), slots=()):
    """A crate of one kind, as the map records it."""
    return {"name": name, "kind": kind, "bins": [], "features": {}, "slots": list(slots),
            "third_party": list(third_party), "machine_work": machine_work or {}, "implements": []}


def edge(source, target, features=(), kind="normal"):
    return {"from": source, "to": target, "kind": kind, "features": list(features), "roles": []}


class OneKindPerCrate(unittest.TestCase):
    """Each kind against what the map shows: a contract or logic crate
    touches nothing, an effect crate touches the machine."""
    kinds = check.kinds

    def mismatches(self, *crates):
        kind_of = {c["name"]: c["kind"] for c in crates}
        found, _ = self.kinds.check(list(crates), [], kind_of, mixed_allowed=False)
        return found

    def test_contract_and_logic_crates_that_touch_nothing_pass(self):
        self.assertEqual(self.mismatches(one("kkt-api", "contract"), one("kkt-plan", "logic")), [])

    def test_a_contract_or_logic_crate_that_touches_the_machine_fails(self):
        for kind in ["contract", "logic"]:
            with self.subTest(kind=kind):
                self.assertEqual(self.mismatches(one("kkt-x", kind, machine_work={"src/lib.rs": ["fs::write"]})),
                                 [f"kkt-x is marked {kind} but touches the machine: in src/lib.rs"])
                self.assertEqual(self.mismatches(one("kkt-x", kind, third_party=["libc", "serde"])),
                                 [f"kkt-x is marked {kind} but touches the machine: with libc"])

    def test_an_effect_crate_touches_the_machine_in_its_code_or_through_a_library(self):
        self.assertEqual(self.mismatches(one("kkt-a-os", "effect", machine_work={"src/lib.rs": ["fs::write"]}),
                                         one("kkt-b-os", "effect", third_party=["rand"])), [])

    def test_an_effect_crate_that_touches_nothing_the_map_can_see_fails(self):
        self.assertEqual(self.mismatches(one("kkt-os", "effect", third_party=["serde"])),
                         ["kkt-os is marked effect but touches the machine nowhere the map can see"])

    def test_only_an_effect_crate_is_reached_as_an_effect(self):
        kind_of = {"kkt-os": "effect", "kkt-api": "contract", "kkt-plan": "logic"}
        self.assertTrue(self.kinds.reaches_effect(edge("a", "kkt-os"), kind_of))
        self.assertFalse(self.kinds.reaches_effect(edge("a", "kkt-api", ["testing"]), kind_of))
        self.assertFalse(self.kinds.reaches_effect(edge("a", "kkt-plan"), kind_of))

    def test_only_a_contract_crates_traits_are_for_others_to_implement(self):
        self.assertTrue(self.kinds.is_contract("contract"))
        for kind in ["logic", "effect", "composition"]:
            self.assertFalse(self.kinds.is_contract(kind), kind)

    def test_the_merged_kinds_are_gone(self):
        self.assertNotIn("contract and effect", self.kinds.KINDS)
        self.assertNotIn("logic and effect", self.kinds.KINDS)


class TokioWork(unittest.TestCase):
    """tokio's files, sockets, processes, signals, terminal and clock are
    machine work, like the standard library's."""

    def test_tokio_machine_work(self):
        for path in ["net::TcpListener", "signal::ctrl_c", "signal::unix::Signal", "fs::read", "process::Command",
                     "io::stdout", "time::sleep", "time::Instant::now"]:
            self.assertTrue(check.kinds.is_tokio_io(path), path)
        for path in ["sync::Mutex", "task::spawn", "select", "time::Duration", "io::AsyncWriteExt"]:
            self.assertFalse(check.kinds.is_tokio_io(path), path)


generate_spec = importlib.util.spec_from_file_location("generate", Path(__file__).resolve().parent / "generate.py")
generate = importlib.util.module_from_spec(generate_spec)
generate_spec.loader.exec_module(generate)


class StandIns(unittest.TestCase):
    """generate.py leaves out of a crate's machine work the files only tests
    or `testing` bring: a stand-in's work is test support's."""

    def test_the_conditions_that_hold_only_for_tests_or_testing(self):
        # A bare `test` holds only for tests too: its modules and its
        # single items are test code.
        for cfg in ['test', 'feature = "testing"', 'any(test, feature = "testing")', 'any(feature = "testing", test)']:
            self.assertTrue(generate.behind_testing(cfg), cfg)
        for cfg in ['unix', 'not(feature = "testing")', 'any(unix, feature = "testing")']:
            self.assertFalse(generate.behind_testing(cfg), cfg)

    def test_modules_behind_testing_and_what_they_declare(self):
        root = Path(__file__).resolve().parents[2] / "target" / "tmp" / "architecture-tests" / "testing-files"
        files = {
            "src/lib.rs": '#[cfg(any(test, feature = "testing"))]\npub mod testing;\n#[cfg(feature = "testing")]\nmod fake;\nmod open;\n',
            "src/testing/mod.rs": "mod conformance;\n",
            "src/testing/conformance.rs": "",
            "src/fake.rs": "",
            "src/open.rs": "",
        }
        for name, text in files.items():
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(text)
        found = generate.testing_module_files(root, [root / n for n in files])
        self.assertEqual(sorted(str(f.relative_to(root)) for f in found),
                         ["src/fake.rs", "src/testing/conformance.rs", "src/testing/mod.rs"])


class AssemblyWiring(unittest.TestCase):
    """generate.py's check of assembly.json's wiring: names that exist, no
    value of the receiving crate's own type recorded as an injection, and
    what an injector passes on joined to the slot a program filled."""

    ITEMS = {"kkt-rules": {"Db": {}, "Startup": {}}, "kkt-db-sqlite": {"SqlitePlace": {}},
             "kkt-terminal": {"Terminal": {}}, "kkt": {}}
    SLOTS = {"kkt-rules": [{"at": "Db::new", "name": "database"}, {"at": "Startup", "name": "db"}],
             "kkt-setup": [{"at": "ask::Prompter::new", "name": "lines"}]}
    PACKAGES = {"kkt", "kkt-rules", "kkt-db-sqlite", "kkt-terminal", "kkt-setup"}

    def check(self, wiring):
        assembly = {"programs": {"kkt": {"steps": [], "wiring": wiring}}}
        with self.assertRaises(SystemExit) as stop:
            generate.check_assembly(assembly, self.ITEMS, self.PACKAGES, self.SLOTS)
        return str(stop.exception)

    def passes(self, wiring):
        assembly = {"programs": {"kkt": {"steps": [], "wiring": wiring}}}
        generate.check_assembly(assembly, self.ITEMS, self.PACKAGES, self.SLOTS)

    def test_another_crates_implementation_and_an_adapter_pass(self):
        self.passes([
            {"impl": "kkt-db-sqlite::SqlitePlace", "crate": "kkt-rules", "to": [{"at": "Db::new", "param": "database"}]},
            {"own": "TerminalLines", "supplier": "kkt-terminal", "supplier_item": "Terminal",
             "crate": "kkt-setup", "to": [{"at": "ask::Prompter::new", "param": "lines"}]},
            {"own": "ServeReads", "supplier": "kkt", "crate": "kkt-setup", "to": [{"at": "ask::Prompter::new", "param": "lines"}]},
        ])

    def run_with_injector(self, passes_on):
        """kkt fills kkt-operate's lines; kkt-operate passes something on."""
        slots = dict(self.SLOTS, **{"kkt-operate": [{"at": "OperateParts", "name": "lines"},
                                                    {"at": "OperateParts", "name": "clock"}]})
        assembly = {
            "programs": {"kkt": {"steps": [], "wiring": [
                {"own": "TerminalLines", "supplier": "kkt-terminal", "crate": "kkt-operate",
                 "to": [{"at": "OperateParts", "param": "lines"}]}]}},
            "injectors": {"kkt-operate": {"wiring": [dict(
                {"passes_on": "the lines it was given", "crate": "kkt-setup",
                 "to": [{"at": "ask::Prompter::new", "param": "lines"}]}, **passes_on)]}},
        }
        generate.check_assembly(assembly, self.ITEMS, self.PACKAGES | {"kkt-operate"}, slots)

    def test_passing_on_names_the_slot_a_program_filled(self):
        self.run_with_injector({"from": [{"at": "OperateParts", "param": "lines"}]})

    def refused(self, passes_on):
        with self.assertRaises(SystemExit) as stop:
            self.run_with_injector(passes_on)
        return str(stop.exception)

    def test_passing_on_without_from_is_refused(self):
        self.assertIn('passing on the lines it was given into kkt-setup says no "from"', self.refused({}))

    def test_from_must_name_a_slot_of_the_injector(self):
        self.assertIn('"from" names no slot OperateParts (screen) of kkt-operate',
                      self.refused({"from": [{"at": "OperateParts", "param": "screen"}]}))

    def test_from_must_name_a_slot_some_program_fills(self):
        self.assertIn("no program fills OperateParts (clock)",
                      self.refused({"from": [{"at": "OperateParts", "param": "clock"}]}))

    def test_the_receiving_crates_own_type_is_refused(self):
        self.assertIn("kkt-rules::Db is kkt-rules's own type, not an injection into it",
                      self.check([{"impl": "kkt-rules::Db", "crate": "kkt-rules", "to": [{"at": "Startup", "param": "db"}]}]))

    def test_a_value_of_a_holder_type_the_receiver_names_is_refused(self):
        slots = dict(self.SLOTS, **{"kkt-mcp": [
            {"at": "endpoint", "name": "fmt", "traits": [],
             "holds": [{"crate": "kkt-result-format", "type": "Formatter", "traits": [{"crate": "kkt-clock", "trait": "Clock"}]}]}]})
        items = dict(self.ITEMS, **{"kkt-result-format": {"Formatter": {}}})
        assembly = {"programs": {"kkt": {"steps": [], "wiring": [
            {"impl": "kkt-result-format::Formatter", "crate": "kkt-mcp", "to": [{"at": "endpoint", "param": "fmt"}]}]}}}
        with self.assertRaises(SystemExit) as stop:
            generate.check_assembly(assembly, items, self.PACKAGES | {"kkt-mcp", "kkt-result-format"}, slots)
        self.assertIn("kkt-result-format::Formatter is a value of a type kkt-mcp names directly", str(stop.exception))

    def test_a_factory_for_the_slots_contract_passes(self):
        slots = dict(self.SLOTS, **{"kkt-rules": [
            {"at": "Db::new", "name": "database", "traits": [{"crate": "kkt-db", "trait": "Backend"}], "holds": []}]})
        self.passes_with(slots, [{"impl": "kkt-db-sqlite::SqlitePlace", "crate": "kkt-rules",
                                  "to": [{"at": "Db::new", "param": "database"}]}])

    def passes_with(self, slots, wiring):
        assembly = {"programs": {"kkt": {"steps": [], "wiring": wiring}}}
        generate.check_assembly(assembly, self.ITEMS, self.PACKAGES, slots)

    def foreign_check(self, impl, foreign, foreign_impls, items):
        slots = dict(self.SLOTS, **{"kkt-log-tracing": [
            {"at": "init_logging", "name": "log_file", "traits": [], "foreign": foreign}]})
        assembly = {"programs": {"kkt": {"steps": [], "wiring": [
            {"impl": impl, "crate": "kkt-log-tracing", "to": [{"at": "init_logging", "param": "log_file"}]}]}}}
        generate.check_assembly(assembly, dict(self.ITEMS, **items), self.PACKAGES | {"kkt-log-tracing", "kkt-rolling-file"},
                                slots, foreign_impls)

    def test_a_slot_typed_as_an_outside_trait_takes_what_rustdoc_shows_meeting_it(self):
        write = ["std::io::Write"]
        items = {"kkt-rolling-file": {"RotatingFile": {"kind": "struct"}, "Plain": {"kind": "struct"},
                                      "open": {"kind": "fn"}}}
        meets = {"kkt-rolling-file": [{"type": "RotatingFile", "trait": "std::io::Write"}]}
        self.foreign_check("kkt-rolling-file::RotatingFile", write, meets, items)
        with self.assertRaises(SystemExit) as stop:
            self.foreign_check("kkt-rolling-file::Plain", write, meets, items)
        self.assertIn("cannot verify that kkt-rolling-file::Plain meets kkt-log-tracing's init_logging (log_file), "
                      "which takes std::io::Write", str(stop.exception))
        # A public function fills a slot that takes a function trait, and
        # nothing else does.
        self.foreign_check("kkt-rolling-file::open", ["core::ops::function::Fn"], {}, items)
        with self.assertRaises(SystemExit):
            self.foreign_check("kkt-rolling-file::open", write, {}, items)

    def test_an_adapter_needs_a_supplier_package_and_its_item(self):
        lines = [{"at": "ask::Prompter::new", "param": "lines"}]
        self.assertIn("names no supplier package (kkt-terminal's Terminal)",
                      self.check([{"own": "TerminalLines", "supplier": "kkt-terminal's Terminal", "crate": "kkt-setup", "to": lines}]))
        self.assertIn("no public item Screen in the supplier kkt-terminal",
                      self.check([{"own": "TerminalLines", "supplier": "kkt-terminal", "supplier_item": "Screen",
                                   "crate": "kkt-setup", "to": lines}]))

    def test_an_entry_is_exactly_one_kind_of_injection(self):
        self.assertIn("needs one of impl, own or passes_on",
                      self.check([{"crate": "kkt-setup", "to": [{"at": "ask::Prompter::new", "param": "lines"}]}]))


class TestCodeSplit(unittest.TestCase):
    """generate.py's split of a file into its real code and its test code:
    anything under #[cfg(test)] or the crate's own `testing` feature is test
    code."""

    def split(self, text):
        return generate.split_test_modules(text)

    def test_testing_items_of_every_shape_go_to_the_tests(self):
        text = """use kkt_log::{Log, Level};
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(feature = "testing")]
pub use store::testing;
pub struct Db {
    backend: Backend,
    #[cfg(any(test, feature = "testing"))]
    faults: Option<kkt_db::testing::FaultyBackend>,
}
impl Db {
    #[cfg(any(test, feature = "testing"))]
    pub fn recent(&self) -> Result<Vec<Message>, Error> {
        kkt_log::testing::CapturedLog::new()
    }
    pub fn real(&self) -> Level { Level::Info }
}
#[cfg(test)]
mod tests {
    use kkt_clock::testing::FixedClock;
}
"""
        normal, tests = self.split(text)
        for gone in ("pub mod testing", "pub use store::testing", "FaultyBackend", "CapturedLog", "FixedClock"):
            self.assertNotIn(gone, normal)
            self.assertIn(gone, tests)
        for kept in ("use kkt_log::{Log, Level};", "backend: Backend,", "pub fn real(&self) -> Level { Level::Info }", "pub struct Db {"):
            self.assertIn(kept, normal)
        # The struct and the impl still close.
        self.assertEqual(normal.count("{"), normal.count("}"))

    def test_a_condition_that_holds_only_under_test_is_test_code(self):
        for cond in ("test", 'feature = "testing"', "all(test, unix)", 'all(unix, any(test, feature = "testing"))',
                     'any(test, feature = "testing")', "any(test, all(test, windows))"):
            self.assertTrue(generate.behind_testing(cond), cond)
        for cond in ("unix", "not(test)", "any(test, unix)", 'feature = "real"', "all(unix, windows)", "all()", "any()"):
            self.assertFalse(generate.behind_testing(cond), cond)
        normal, tests = self.split("#[cfg(all(test, unix))]\nmod tests {\n    use kkt_scratch::Dir;\n}\nfn real() {}\n")
        self.assertIn("kkt_scratch", tests)
        self.assertNotIn("kkt_scratch", normal)
        self.assertIn("fn real", normal)

    def test_other_conditions_stay_real_code(self):
        text = """#[cfg(unix)]
fn on_unix() { std::fs::read("x"); }
#[cfg(feature = "real")]
mod real;
"""
        normal, tests = self.split(text)
        self.assertEqual(tests, "")
        self.assertIn("on_unix", normal)
        self.assertIn("mod real;", normal)


class TestingOnlyDependency(unittest.TestCase):
    """A normal dependency that only the package's own `testing` feature
    turns on is a test dependency."""

    FEATURES = {"testing": ["dep:kkt-search", "dep:rusqlite", "kkt-clock/testing"], "real": ["dep:tokio"],
                "both": ["dep:kkt-names"], "more": ["dep:kkt-names"]}

    def only(self, dep):
        return generate.testing_only_dependency(self.FEATURES, dep)

    def test_optional_and_turned_on_only_by_testing(self):
        self.assertTrue(self.only({"name": "kkt-search", "optional": True}))

    def test_not_optional_is_real_code(self):
        # kkt-clock is a plain dependency whose own testing feature is asked for.
        self.assertFalse(self.only({"name": "kkt-clock", "optional": False}))

    def test_turned_on_by_another_feature_is_real_code(self):
        self.assertFalse(self.only({"name": "tokio", "optional": True}))
        self.assertFalse(self.only({"name": "kkt-names", "optional": True}))


class FactoryGives(unittest.TestCase):
    """generate.py's rule for what makes a type a factory: a public function
    that gives back another crate's holder, implementation or contract."""

    LIBS = {"kkt_db": "kkt-db", "kkt_db_sqlite": "kkt-db-sqlite", "kkt_rules": "kkt-rules"}
    PUBLIC = {("kkt-db", "Backend", "trait"): "Backend", ("kkt-db", "Value", "struct"): "Value",
              ("kkt-rules", "Db", "struct"): "Db"}

    def gives(self, types, holders=(), implementing=()):
        def public(pkg, name, kinds):
            return next((p for (c, n, k), p in self.PUBLIC.items() if c == pkg and n == name and k in kinds), None)
        return generate.factory_gives("kkt-db-sqlite", types, self.LIBS, public, set(holders), set(implementing))

    def test_another_crates_contract_as_a_trait_object_makes_a_factory(self):
        # SqlitePlace::open gives an Arc<dyn kkt_db::Backend>: the rule missed it.
        self.assertEqual(self.gives([("kkt_db", "Backend", "trait")]), [{"crate": "kkt-db", "type": "Backend"}])

    def test_another_crates_holder_makes_a_factory(self):
        self.assertEqual(self.gives([("kkt_rules", "Db", "struct")], holders={("kkt-rules", "Db")}),
                         [{"crate": "kkt-rules", "type": "Db"}])

    def test_a_plain_value_or_its_own_type_does_not(self):
        self.assertEqual(self.gives([("kkt_db", "Value", "struct"), ("kkt_db_sqlite", "SqlitePlace", "struct")]), [])


if __name__ == "__main__":
    unittest.main()
