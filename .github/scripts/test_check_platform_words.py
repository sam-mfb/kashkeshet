"""check-platform-words.py: no platform crate names kkt in a string or a public item's name."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "check_platform_words", Path(__file__).resolve().parent / "check-platform-words.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


class ProblemsIn(unittest.TestCase):
    def found(self, text):
        return check.problems_in(text, "f.rs")

    def test_a_string_that_names_kkt_or_kashkeshet_is_found(self):
        self.assertEqual(len(self.found('const NAME: &str = "kkt";\n')), 1)
        self.assertEqual(len(self.found('fn f() { g("then rerun `kkt setup`"); }\n')), 1)
        self.assertEqual(len(self.found('let t = ["kashkeshet", "x"];\n')), 1)
        self.assertEqual(len(self.found('let r = r#"a "kkt" b"#;\n')), 1)

    def test_the_line_is_named(self):
        self.assertEqual(self.found('\n\nlet a = "kkt";\n'), ['f.rs:3: a string names kkt: "kkt"'])

    def test_a_word_inside_another_is_not_kkt(self):
        self.assertEqual(self.found('let a = "kktools"; let b = "x_kkt_y";\n'), [])

    def test_ordinary_comments_and_crate_paths_are_not_read(self):
        text = ('// "kkt"\n/* "kkt" */\n'
                'use kkt_clock::Time;\nfn f() -> kkt_names::Handle { x() } // "kkt"\n')
        self.assertEqual(self.found(text), [])

    def test_a_doc_comment_that_names_kkt_is_found_but_a_crate_name_is_not(self):
        self.assertEqual(self.found("//! For kkt's server.\n/// Like Kashkeshet does.\nfn f() {}\n"),
                         ["f.rs:1: a doc comment names kkt: //! For kkt's server.",
                          "f.rs:2: a doc comment names kkt: /// Like Kashkeshet does."])
        self.assertEqual(self.found("//! Implements [`kkt_log::Log`] over kkt-log's levels.\n"), [])

    def test_doc_comments_in_test_modules_are_left_out(self):
        self.assertEqual(self.found("#[cfg(test)]\nmod tests {\n    /// kkt\n    fn t() {}\n}\n"), [])

    def test_a_quote_in_a_character_literal_opens_no_string(self):
        self.assertEqual(self.found("let q = '\"'; let e = '\\\"'; // kkt\nlet s = \"plain\";\n"), [])

    def test_test_modules_are_left_out(self):
        text = '#[cfg(test)]\nmod tests {\n    fn t() { let a = "kkt"; { } }\n}\nlet b = "plain";\n'
        self.assertEqual(self.found(text), [])

    def test_a_public_item_named_for_kkt_is_found(self):
        self.assertEqual(
            self.found("pub fn kkt_path() {}\npub(crate) struct KktThing;\nfn kkt_private() {}\n"),
            ["f.rs:1: a public item's name names kkt: kkt_path",
             "f.rs:2: a public item's name names kkt: KktThing"])


class ToolProblemsIn(unittest.TestCase):
    def found(self, text, own=frozenset()):
        return check.tool_problems_in(text, "f.rs", set(own))

    def test_a_string_that_names_a_tool_the_crate_is_not_for_is_found(self):
        self.assertEqual(self.found('let s = "Log in to Tailscale by opening: {url}";\n'),
                         ['f.rs:1: a string names tailscale, which the crate\'s description does not: '
                          '"Log in to Tailscale by opening: {url}"'])
        self.assertEqual(len(self.found('let s = "run `launchctl print` and `systemctl status`";\n')), 2)

    def test_a_tool_the_description_names_is_the_crates_own(self):
        self.assertEqual(self.found('let c = "docker compose up";\n', {"docker", "compose"}), [])
        self.assertEqual(self.found('let c = "xdg-open";\n', {"xdg-open"}), [])

    def test_comments_doc_comments_and_tests_are_not_read(self):
        text = ('/// Started by systemd.\n// "tailscale"\n'
                '#[cfg(test)]\nmod tests {\n    fn t() { let a = "docker"; }\n}\n')
        self.assertEqual(self.found(text), [])

    def test_a_word_inside_another_is_not_a_tool(self):
        self.assertEqual(self.found('let a = "digital"; let b = "composer";\n'), [])


class DescriptionProblems(unittest.TestCase):
    def found(self, crates):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs/architecture").mkdir(parents=True)
            (root / "docs/architecture/concerns.json").write_text(json.dumps({"crates": crates}))
            return check.description_problems(root)

    def test_a_platform_description_that_names_kkt_or_another_kkt_crate_is_found(self):
        self.assertEqual(self.found({"kkt-plain": {"layer": "platform", "concern": "Implements kkt-log's contract."}}),
                         ["concerns.json: kkt-plain's description names kkt: kkt"])
        self.assertEqual(len(self.found({"kkt-plain": {"layer": "platform", "concern": "Files for kashkeshet."}})), 1)

    def test_kkt_in_another_layers_description_is_fine(self):
        self.assertEqual(self.found({"kkt-core": {"layer": "core", "concern": "Implements kkt-db's Backend."}}), [])

    def test_history_in_any_description_is_found(self):
        for text in ["A file (was kkt-logs-os).", "Formerly two crates.", "As ARCHITECTURE.md 4.2.2 says.",
                     "Per RUST.md 6.", "Added in #563.", "Split in phase 16."]:
            self.assertEqual(len(self.found({"kkt-core": {"layer": "core", "concern": text}})), 1, text)

    def test_the_layer_texts_keep_to_the_rules(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs/architecture").mkdir(parents=True)
            (root / "docs/architecture/concerns.json").write_text(json.dumps({"crates": {}, "layers": [
                {"id": "platform", "about": "General crates that know nothing of kkt."},
                {"id": "core", "about": "The kkt core, split in phase 16."},
                {"id": "data", "about": "The kkt database."}]}))
            self.assertEqual(check.description_problems(root), [
                "concerns.json: the platform layer's text names kkt: kkt",
                "concerns.json: the platform layer's text says what it is not: nothing",
                "concerns.json: the core layer's text gives history: phase 16"])

    def test_a_negative_in_any_description_is_found(self):
        for text, negative in [("It knows nothing of what runs in it.", "nothing"), ("It never names a tool.", "never"),
                               ("It does not log.", "not"), ("It does  not log.", "not"), ("It Is Not a contract.", "Not"),
                               ("Do not log here.", "not"), ("It cannot log.", "cannot"),
                               ("It has nothing to do with logs.", "nothing"),
                               ("It doesn't log.", "n't"), ("It doesn’t log.", "n’t"), ("It isn't a contract.", "n't")]:
            self.assertEqual(self.found({"kkt-core": {"layer": "core", "concern": text}}),
                             [f"concerns.json: kkt-core's description says what it is not: {negative}"], text)
        # A word that only contains one is fine.
        self.assertEqual(self.found({"kkt-core": {"layer": "core", "concern": "Nevertheless, it notes each change."}}), [])

    def test_a_description_longer_than_the_most_three_short_sentences_hold_is_found(self):
        longest = "x" * check.MAX_DESCRIPTION
        self.assertEqual(self.found({"kkt-core": {"layer": "core", "concern": longest}}), [])
        self.assertEqual(self.found({"kkt-core": {"layer": "core", "concern": longest + "x"}}), [
            f"concerns.json: kkt-core's description is {check.MAX_DESCRIPTION + 1} characters, "
            f"more than {check.MAX_DESCRIPTION}: say it in one to three short sentences"])

    def test_a_plain_description_passes(self):
        self.assertEqual(self.found({"kkt-plain": {"layer": "platform", "concern": "A file that rolls over at a size."},
                                     "kkt-core": {"layer": "core", "concern": "Every change, in one swarm."}}), [])


class HeaderProblems(unittest.TestCase):
    def found(self, files):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs/architecture").mkdir(parents=True)
            (root / "docs/architecture/concerns.json").write_text(json.dumps(
                {"crates": {name: {"layer": "core"} for name in files}}))
            for name, text in files.items():
                file = root / "crates" / name / "src/lib.rs"
                file.parent.mkdir(parents=True)
                file.write_text(text)
            return check.header_problems(root)

    def test_history_anywhere_in_a_header_is_found(self):
        self.assertEqual(self.found({"kkt-a": "//! Files.\n//!\n//! Split out in #563.\nmod x;\n"}),
                         ["crates/kkt-a/src/lib.rs:3: the crate's header gives history: #563"])

    def test_a_negative_in_the_first_sentence_is_found_and_later_is_fine(self):
        self.assertEqual(self.found({"kkt-a": "//! Files, never the network.\nmod x;\n"}),
                         ["crates/kkt-a/src/lib.rs: the crate's header begins with what it is not: never"])
        self.assertEqual(self.found({"kkt-a": "//! Files. A missing one is not an error.\n//!\n"
                                              "//! It does not log.\nmod x;\n"}), [])

    def test_the_plainer_marks_of_history_are_found_too(self):
        for text in ["Kept as before.", "It used to be part of kkt-web.", "It no longer logs.",
                     "In the established order.", "Previously in kkt-web.", "Originally one crate."]:
            self.assertEqual(len(self.found({"kkt-a": f"//! Files.\n//! {text}\nmod x;\n"})), 1, text)

    def test_the_root_packages_library_and_binaries_are_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs/architecture").mkdir(parents=True)
            (root / "docs/architecture/concerns.json").write_text(json.dumps({"crates": {"app": {"layer": "program"}}}))
            (root / "Cargo.toml").write_text('[package]\nname = "app"\n\n[workspace]\n')
            (root / "src/bin/app").mkdir(parents=True)
            (root / "src/lib.rs").write_text("//! The app, never a library.\n")
            (root / "src/bin/app/main.rs").write_text("//! The command.\n//! Split out in #12.\n")
            self.assertEqual(check.header_problems(root), [
                "src/lib.rs: the crate's header begins with what it is not: never",
                "src/bin/app/main.rs:2: the crate's header gives history: #12"])

    def test_only_the_opening_lines_are_the_header(self):
        self.assertEqual(self.found({"kkt-a": "//! Files.\n\nmod x;\n//! Was in #1.\n"}), [])

    def test_the_first_sentence_reads_across_lines(self):
        self.assertEqual(self.first("Files on\ndisk. More."), "Files on disk.")

    def first(self, text):
        return check.first_sentence(text.split("\n"))


class Problems(unittest.TestCase):
    def test_only_platform_crates_and_their_code_outside_tests_are_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs/architecture").mkdir(parents=True)
            (root / "docs/architecture/concerns.json").write_text(json.dumps({"crates": {
                "kkt-plain": {"layer": "platform"}, "kkt-core": {"layer": "core"},
                "kkt-git": {"layer": "platform", "concern": "The commit a folder is at in git."}}}))
            for crate, path, text in [
                ("kkt-plain", "src/lib.rs", 'pub const A: &str = "kkt";\n'),
                ("kkt-plain", "src/tests.rs", 'const B: &str = "kkt";\n'),
                ("kkt-plain", "src/x/tests/more.rs", 'const C: &str = "kkt";\n'),
                ("kkt-plain", "src/open.rs", 'const E: &str = "Log in to Tailscale";\n'),
                ("kkt-plain", "src/testing.rs", 'const F: &str = "docker";\n'),
                ("kkt-plain", "src/testing/more.rs", 'const G: &str = "docker";\n'),
                ("kkt-git", "src/lib.rs", 'const H: &str = "git";\n'),
                ("kkt-core", "src/lib.rs", 'const D: &str = "kkt"; const I: &str = "docker";\n'),
            ]:
                file = root / "crates" / crate / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(text)
            self.assertEqual(check.problems(root),
                             ['crates/kkt-plain/src/lib.rs:1: a string names kkt: "kkt"',
                              "crates/kkt-plain/src/open.rs:1: a string names tailscale, which the "
                              "crate's description does not: \"Log in to Tailscale\""])


if __name__ == "__main__":
    unittest.main()
