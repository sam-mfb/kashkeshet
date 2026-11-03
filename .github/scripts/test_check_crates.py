"""check-crates.jq: the rules for turning on another workspace package's real and testing features."""

import json
from pathlib import Path
import subprocess
import unittest

RULES = Path(__file__).resolve().parent / "check-crates.jq"


def package(name, targets=("lib",), dependencies=(), features=None):
    """A package as `cargo metadata --no-deps` describes it, with only the fields the rules read."""
    return {
        "name": name,
        "targets": [{"name": name, "kind": [kind]} for kind in targets],
        "dependencies": list(dependencies),
        "features": features or {},
    }


def dependency(name, kind=None, features=(), rename=None, target=None):
    return {"name": name, "kind": kind, "features": list(features), "rename": rename, "target": target}


def problems(*packages):
    result = subprocess.run(
        ["jq", "-r", "-f", str(RULES)],
        input=json.dumps({"packages": list(packages)}),
        check=True, capture_output=True, text=True,
    )
    return result.stdout.splitlines()


class CheckCrates(unittest.TestCase):
    def test_programs_examples_and_dev_dependencies_may_turn_features_on(self):
        self.assertEqual(problems(
            package("kashkeshet", targets=("lib", "bin", "test"), dependencies=[
                dependency("kkt-a", features=["real"]),
                dependency("kkt-b", kind="dev", features=["testing"]),
                dependency("kkt-scratch", kind="dev"),
            ], features={"extra": ["kkt-a/real"]}),
            package("kkt-a"),
            package("kkt-b", targets=("lib", "example"), dependencies=[
                dependency("kkt-a", kind="dev", features=["real", "testing"]),
                dependency("kkt-scratch", kind="dev"),
            ]),
            package("kkt-scratch"),
        ), [])

    def test_a_library_may_not_turn_on_another_packages_real(self):
        self.assertEqual(problems(
            package("kkt-a"),
            package("kkt-b", dependencies=[dependency("kkt-a", features=["real"])]),
        ), ["kkt-b: turns on kkt-a's real in [dependencies]; only a package with a binary target may (in [dependencies]), or one with an example target (in [dev-dependencies])"])

    def test_real_is_allowed_only_in_the_section_the_target_allows(self):
        cases = [
            (("lib", "example"), dependency("kkt-a", features=["real"]), "[dependencies]"),
            (("lib",), dependency("kkt-a", kind="dev", features=["real"]), "[dev-dependencies]"),
            (("bin",), dependency("kkt-a", kind="dev", features=["real"]), "[dev-dependencies]"),
            (("bin", "example"), dependency("kkt-a", kind="build", features=["real"]), "[build-dependencies]"),
            (("bin",), dependency("kkt-a", kind="build", features=["real"], target="cfg(unix)"), "[build-dependencies] for cfg(unix)"),
        ]
        for targets, used, section in cases:
            with self.subTest(targets=targets, section=section):
                found = problems(package("kkt-a"), package("kkt-b", targets=targets, dependencies=[used]))
                self.assertEqual(len(found), 1)
                self.assertTrue(found[0].startswith(f"kkt-b: turns on kkt-a's real in {section};"), found[0])

    def test_testing_only_in_dev_dependencies_even_for_programs(self):
        for kind, section in [(None, "[dependencies]"), ("build", "[build-dependencies]")]:
            with self.subTest(section=section):
                self.assertEqual(problems(
                    package("kkt-a"),
                    package("kkt-b", targets=("bin", "example"), dependencies=[dependency("kkt-a", kind=kind, features=["testing"])]),
                ), [f"kkt-b: turns on kkt-a's testing in {section}; only [dev-dependencies] may"])

    def test_scratch_only_in_dev_dependencies(self):
        self.assertEqual(problems(
            package("kkt-scratch"),
            package("kkt-b", targets=("bin",), dependencies=[dependency("kkt-scratch")]),
        ), ["kkt-b: lists kkt-scratch in [dependencies]; only [dev-dependencies] may"])

    def test_own_features_that_turn_on_real_or_testing(self):
        self.assertEqual(problems(
            package("kkt-a"),
            package("kkt-b", dependencies=[dependency("kkt-a", rename="a")], features={
                "real": ["a/real"],
                "weak": ["a?/real"],
                "fine": ["a/other", "dep:a"],
            }),
            package("kkt-c", targets=("bin",), dependencies=[dependency("kkt-a")], features={
                "real": ["kkt-a/real"],
                "testing": ["kkt-a/testing"],
            }),
        ), [
            "kkt-b: its feature real turns on a's real; only a package with a binary target may, in [dependencies]",
            "kkt-b: its feature weak turns on a's real; only a package with a binary target may, in [dependencies]",
            "kkt-c: its feature testing turns on kkt-a's testing; only [dev-dependencies] may",
        ])

    def test_third_party_packages_are_not_checked(self):
        self.assertEqual(problems(
            package("kkt-b", dependencies=[dependency("outside", features=["real", "testing"])], features={"real": ["outside/real"]}),
        ), [])


if __name__ == "__main__":
    unittest.main()
