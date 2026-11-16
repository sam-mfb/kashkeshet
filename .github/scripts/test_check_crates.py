"""check-crates.jq: which packages may depend on an effect crate, the rules for turning on another workspace package's testing feature, no `real` feature, and package names."""

import json
from pathlib import Path
import subprocess
import unittest

RULES = Path(__file__).resolve().parent / "check-crates.jq"


def package(name, targets=("lib",), dependencies=(), features=None, dev_only=False):
    """A package as `cargo metadata --no-deps` describes it, with only the fields the rules read."""
    return {
        "name": name,
        "targets": [{"name": name, "kind": [kind]} for kind in targets],
        "dependencies": list(dependencies),
        "features": features or {},
        "metadata": {"kkt": {"dev-only": True}} if dev_only else None,
    }


def dependency(name, kind=None, features=(), rename=None, target=None):
    return {"name": name, "kind": kind, "features": list(features), "rename": rename, "target": target}


def exception(package, dependency):
    """One entry of effect-exceptions.json."""
    return {"package": package, "dependency": dependency, "reason": "a test reason"}


def problems(*packages, effects=(), effect_exceptions=()):
    result = subprocess.run(
        ["jq", "-r", "--argjson", "effects", json.dumps(list(effects)),
         "--argjson", "effect_exceptions", json.dumps(list(effect_exceptions)), "-f", str(RULES)],
        input=json.dumps({"packages": list(packages)}),
        check=True, capture_output=True, text=True,
    )
    return result.stdout.splitlines()


def effect_rule(package, dependency, section):
    return (f"{package}: depends on the effect crate {dependency} in {section}; only a program "
            "(a package with a binary target) may, unless listed in effect-exceptions.json")


class CheckCrates(unittest.TestCase):
    def test_a_program_may_depend_on_an_effect_crate_in_any_section(self):
        self.assertEqual(problems(
            package("kashkeshet", targets=("lib", "bin", "test"), dependencies=[
                dependency("kkt-os"),
                dependency("kkt-os", kind="dev"),
                dependency("kkt-os", kind="build"),
                dependency("kkt-b", kind="dev", features=["testing"]),
                dependency("kkt-scratch", kind="dev"),
            ]),
            package("kkt-os"),
            package("kkt-b"),
            package("kkt-scratch"),
            effects=["kkt-os"],
        ), [])

    def test_a_program_whose_tests_alone_use_an_effect_crate_needs_an_entry(self):
        tests_only = package("kkt-product", targets=("lib", "bin"), dependencies=[dependency("kkt-os", kind="dev")])
        self.assertEqual(problems(package("kkt-os"), tests_only, effects=["kkt-os"]),
                         [effect_rule("kkt-product", "kkt-os", "[dev-dependencies]")])
        self.assertEqual(problems(package("kkt-os"), tests_only, effects=["kkt-os"],
                                  effect_exceptions=[exception("kkt-product", "kkt-os")]), [])

    def test_a_program_that_depends_on_an_effect_crate_may_name_it_in_its_tests_too(self):
        for section in [None, "build"]:
            with self.subTest(section=section):
                both = package("kashkeshet", targets=("lib", "bin"), dependencies=[
                    dependency("kkt-os", kind=section), dependency("kkt-os", kind="dev", features=["testing"])])
                self.assertEqual(problems(package("kkt-os"), both, effects=["kkt-os"]), [])

    def test_nothing_but_a_program_depends_on_an_effect_crate(self):
        # A logic, contract, composition, test-support or effect crate: the
        # rule reads only whether the package has a binary target, so it
        # covers an effect crate that depends on another effect crate too.
        cases = [
            (dependency("kkt-os"), "[dependencies]"),
            (dependency("kkt-os", kind="dev"), "[dev-dependencies]"),
            (dependency("kkt-os", kind="build"), "[build-dependencies]"),
            (dependency("kkt-os", target="cfg(unix)"), "[dependencies] for cfg(unix)"),
        ]
        for used, section in cases:
            for user in ["kkt-logic", "kkt-other-os"]:
                with self.subTest(user=user, section=section):
                    self.assertEqual(problems(
                        package("kkt-os"),
                        package(user, targets=("lib", "example"), dependencies=[used]),
                        effects=["kkt-os", "kkt-other-os"],
                    ), [effect_rule(user, "kkt-os", section)])

    def test_a_crate_that_is_not_an_effect_may_be_depended_on(self):
        self.assertEqual(problems(
            package("kkt-logic"),
            package("kkt-b", dependencies=[dependency("kkt-logic"), dependency("kkt-logic", kind="dev")]),
            effects=["kkt-os"],
        ), [])

    def test_a_listed_exception_lets_a_package_depend_on_an_effect_crate(self):
        tests_use = package("kkt-b", dependencies=[dependency("kkt-os", kind="dev")])
        self.assertEqual(problems(
            package("kkt-os"), tests_use, effects=["kkt-os"], effect_exceptions=[exception("kkt-b", "kkt-os")],
        ), [])

    def test_an_unlisted_dependency_still_fails(self):
        uses = package("kkt-b", dependencies=[dependency("kkt-os", kind="dev"), dependency("kkt-other-os", kind="dev")])
        self.assertEqual(problems(
            package("kkt-os"), package("kkt-other-os"), uses,
            effects=["kkt-os", "kkt-other-os"], effect_exceptions=[exception("kkt-b", "kkt-os")],
        ), [effect_rule("kkt-b", "kkt-other-os", "[dev-dependencies]")])

    def test_a_listed_exception_that_no_longer_applies_fails(self):
        gone = "effect-exceptions.json lists kkt-b depending on kkt-os, which it no longer does; remove the entry"
        for name, packages in [
            ("no such dependency", [package("kkt-os"), package("kkt-b")]),
            ("no such package", [package("kkt-os")]),
        ]:
            with self.subTest(name):
                self.assertEqual(problems(
                    *packages, effects=["kkt-os"], effect_exceptions=[exception("kkt-b", "kkt-os")],
                ), [gone])

    def test_testing_only_in_dev_dependencies_even_for_programs(self):
        for kind, section in [(None, "[dependencies]"), ("build", "[build-dependencies]")]:
            with self.subTest(section=section):
                self.assertEqual(problems(
                    package("kkt-a"),
                    package("kkt-b", targets=("bin", "example"), dependencies=[dependency("kkt-a", kind=kind, features=["testing"])]),
                ), [f"kkt-b: turns on kkt-a's testing in {section}; only [dev-dependencies] may, or a dev-only program's [dependencies]"])

    def test_a_dev_only_program_may_turn_on_testing_in_its_dependencies(self):
        uses = [dependency("kkt-a", features=["testing"])]
        self.assertEqual(problems(
            package("kkt-a"),
            package("kkt-b", targets=("bin",), dependencies=uses, dev_only=True),
        ), [])
        # Not a library marked dev-only, nor in [build-dependencies].
        self.assertEqual(problems(
            package("kkt-a"),
            package("kkt-b", dependencies=uses, dev_only=True),
        ), ["kkt-b: turns on kkt-a's testing in [dependencies]; only [dev-dependencies] may, or a dev-only program's [dependencies]"])
        self.assertEqual(problems(
            package("kkt-a"),
            package("kkt-b", targets=("bin",), dependencies=[dependency("kkt-a", kind="build", features=["testing"])], dev_only=True),
        ), ["kkt-b: turns on kkt-a's testing in [build-dependencies]; only [dev-dependencies] may, or a dev-only program's [dependencies]"])

    def test_scratch_only_in_dev_dependencies(self):
        self.assertEqual(problems(
            package("kkt-scratch"),
            package("kkt-b", targets=("bin",), dependencies=[dependency("kkt-scratch")]),
        ), ["kkt-b: lists kkt-scratch in [dependencies]; only [dev-dependencies] may"])

    def test_own_features_that_turn_on_testing(self):
        self.assertEqual(problems(
            package("kkt-a"),
            package("kkt-b", dependencies=[dependency("kkt-a", rename="a")], features={
                "stubs": ["a/testing"],
                "weak": ["a?/testing"],
                "fine": ["a/other", "dep:a"],
            }),
            package("kkt-c", targets=("bin",), dependencies=[dependency("kkt-a")], features={
                "stubs": ["kkt-a/testing"],
            }),
        ), [
            "kkt-b: its feature stubs turns on a's testing; only [dev-dependencies] may",
            "kkt-b: its feature weak turns on a's testing; only [dev-dependencies] may",
            "kkt-c: its feature stubs turns on kkt-a's testing; only [dev-dependencies] may",
        ])

    def test_a_packages_own_testing_may_turn_on_a_dependencys_testing(self):
        self.assertEqual(problems(
            package("kkt-a", features={"testing": []}),
            package("kkt-b", dependencies=[dependency("kkt-a", rename="a")], features={
                "testing": ["a/testing", "a?/testing"],
            }),
        ), [])

    def test_no_other_feature_may_turn_on_a_dependencys_testing(self):
        self.assertEqual(problems(
            package("kkt-a", features={"testing": []}),
            package("kkt-b", dependencies=[dependency("kkt-a")], features={
                "testing": [],
                "helpers": ["kkt-a/testing"],
            }),
        ), ["kkt-b: its feature helpers turns on kkt-a's testing; only [dev-dependencies] may"])

    def test_a_librarys_default_may_not_list_its_own_testing(self):
        self.assertEqual(problems(
            package("kkt-b", features={"default": ["testing"], "testing": []}),
        ), ["kkt-b: its default feature turns on its own testing; a library's default feature may not"])

    def test_a_librarys_default_may_not_reach_its_own_testing_through_its_features(self):
        self.assertEqual(problems(
            package("kkt-b", features={
                "default": ["extra"],
                "extra": ["more"],
                "more": ["testing"],
                "testing": [],
            }),
        ), ["kkt-b: its default feature turns on its own testing (through its feature more); a library's default feature may not"])

    def test_a_librarys_default_may_not_forward_to_another_packages_testing(self):
        for forward in ["a/testing", "a?/testing"]:
            with self.subTest(forward=forward, through="default"):
                self.assertEqual(problems(
                    package("kkt-a"),
                    package("kkt-b", dependencies=[dependency("kkt-a", rename="a")], features={"default": [forward]}),
                ), [
                    "kkt-b: its feature default turns on a's testing; only [dev-dependencies] may",
                    "kkt-b: its default feature turns on a's testing; a library's default feature may not",
                ])
            with self.subTest(forward=forward, through="extra"):
                self.assertEqual(problems(
                    package("kkt-a"),
                    package("kkt-b", dependencies=[dependency("kkt-a", rename="a")], features={"default": ["extra"], "extra": [forward]}),
                ), [
                    "kkt-b: its feature extra turns on a's testing; only [dev-dependencies] may",
                    "kkt-b: its default feature turns on a's testing (through its feature extra); a library's default feature may not",
                ])

    def test_a_default_that_reaches_no_testing_passes(self):
        self.assertEqual(problems(
            package("kkt-a"),
            package("kkt-b", dependencies=[dependency("kkt-a", rename="a")], features={
                "default": ["extra", "dep:a", "a/other"],
                "extra": ["a?/other"],
                "testing": [],
            }),
            package("kkt-c", dependencies=[dependency("outside")], features={"default": ["outside?/testing"]}),
            package("kkt-d", targets=("bin",), dependencies=[dependency("kkt-a")], features={"default": ["testing"], "testing": []}),
        ), [])

    def test_third_party_packages_are_not_checked(self):
        self.assertEqual(problems(
            package("kkt-b", dependencies=[dependency("outside", features=["testing"])]),
            effects=["outside"],
        ), [])

    def test_every_package_without_a_binary_target_is_named_kkt_something(self):
        self.assertEqual(problems(
            package("any-program", targets=("lib", "bin", "test")),
            package("kkt-a"),
            package("helpers"),
            package("kktools"),
            package("preview", targets=("lib", "example")),
        ), [
            "helpers: its name does not start with kkt-; every package without a binary target must",
            "kktools: its name does not start with kkt-; every package without a binary target must",
            "preview: its name does not start with kkt-; every package without a binary target must",
        ])

    def test_no_package_has_a_real_feature(self):
        for targets in [("lib",), ("bin",)]:
            with self.subTest(targets=targets):
                self.assertEqual(problems(
                    package("kkt-a", targets=targets, features={"real": [], "testing": []}),
                    package("kkt-b", features={"testing": []}),
                ), ["kkt-a: it has a `real` feature; code that touches the machine belongs in an effect crate, not behind a feature"])


if __name__ == "__main__":
    unittest.main()
