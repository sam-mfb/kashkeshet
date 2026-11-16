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

    def test_comments_doc_comments_and_crate_paths_are_not_read(self):
        text = ('//! kkt\n/// "kkt"\n// "kkt"\n/* "kkt" */\n'
                'use kkt_clock::Time;\nfn f() -> kkt_names::Handle { x() } // "kkt"\n')
        self.assertEqual(self.found(text), [])

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


class Problems(unittest.TestCase):
    def test_only_platform_crates_and_their_code_outside_tests_are_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "docs/architecture").mkdir(parents=True)
            (root / "docs/architecture/concerns.json").write_text(json.dumps({"crates": {
                "kkt-plain": {"layer": "platform"}, "kkt-core": {"layer": "core"}}}))
            for crate, path, text in [
                ("kkt-plain", "src/lib.rs", 'pub const A: &str = "kkt";\n'),
                ("kkt-plain", "src/tests.rs", 'const B: &str = "kkt";\n'),
                ("kkt-plain", "src/x/tests/more.rs", 'const C: &str = "kkt";\n'),
                ("kkt-core", "src/lib.rs", 'const D: &str = "kkt";\n'),
            ]:
                file = root / "crates" / crate / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(text)
            self.assertEqual(check.problems(root),
                             ['crates/kkt-plain/src/lib.rs:1: a string names kkt: "kkt"'])


if __name__ == "__main__":
    unittest.main()
