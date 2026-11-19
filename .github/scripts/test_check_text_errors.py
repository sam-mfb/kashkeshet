"""check-text-errors.py: no function outside tests returns a text error, but for the listed exceptions."""

import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "check_text_errors", Path(__file__).resolve().parent / "check-text-errors.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


class TextErrors(unittest.TestCase):
    def found(self, text):
        return check.text_errors(text)[0]

    def test_a_text_error_is_found_on_one_line_or_several(self):
        self.assertEqual(self.found("fn a() -> Result<(), String> {}\n"), ["a"])
        self.assertEqual(self.found("pub fn b(\n    x: u8,\n) -> Result<\n    u8,\n    String,\n> {\n}\n"), ["b"])

    def test_a_pair_ending_in_text_is_found(self):
        self.assertEqual(self.found("fn a() -> Result<u8, (Purpose, String)> {}\n"), ["a"])

    def test_a_text_error_in_a_closure_parameter_is_found(self):
        text = "pub fn parse<L>(level: impl Fn(&str) -> Result<L, String>) -> Result<L, Error> {}\n"
        self.assertEqual(self.found(text), ["parse"])

    def test_a_text_error_in_a_stream_is_found(self):
        self.assertEqual(self.found("fn s(self) -> BoxStream<'static, Result<Vec<u8>, String>> {}\n"), ["s"])

    def test_typed_errors_and_text_values_pass(self):
        for text in ["fn a() -> Result<String, Error> {}\n",
                     "fn b() -> io::Result<String> {}\n",
                     "fn c() -> Result<HashMap<String, String>, Error> {}\n",
                     "fn d(f: impl Fn() -> String) -> Result<(), Error> {}\n",
                     "fn e() -> Option<String> {}\n"]:
            self.assertEqual(self.found(text), [], text)

    def test_a_body_is_not_read_as_the_signature(self):
        text = "fn a() -> u8 {\n    let x: Result<(), String> = Ok(());\n    0\n}\n"
        self.assertEqual(self.found(text), [])

    def test_comments_and_strings_are_not_read(self):
        text = ('/// fn doc() -> Result<(), String>\n'
                'fn a() -> u8 { let s = "fn b() -> Result<(), String>"; 0 }\n'
                '/* fn c() -> Result<(), String> */\n')
        self.assertEqual(self.found(text), [])

    def test_cfg_test_items_are_left_out(self):
        text = ("#[cfg(test)]\nfn a() -> Result<(), String> {}\n"
                "#[cfg(test)]\n#[derive(Debug)]\nmod tests {\n    fn b() -> Result<(), String> {}\n}\n"
                "fn c() -> Result<(), String> {}\n")
        self.assertEqual(self.found(text), ["c"])

    def test_cfg_not_test_items_are_read(self):
        self.assertEqual(self.found("#[cfg(not(test))]\nfn a() -> Result<(), String> {}\n"), ["a"])

    def test_a_cfg_test_module_declared_here_is_named(self):
        self.assertEqual(check.text_errors("#[cfg(test)]\nmod helpers;\n")[1], ["helpers"])


class Scan(unittest.TestCase):
    TEXT = "fn a() -> Result<(), String> {}\n"

    def scan(self, files):
        return check.scan(sorted(files), lambda relative: files[relative])

    def test_test_files_and_folders_are_left_out(self):
        files = {f: self.TEXT for f in ["c/src/tests.rs", "c/src/testing.rs", "c/src/x_tests.rs",
                                         "c/tests/t.rs", "c/src/testing/m.rs", "c/src/lib.rs"]}
        self.assertEqual(self.scan(files), [("c/src/lib.rs", "a")])

    def test_a_module_declared_cfg_test_is_left_out_with_its_folder(self):
        files = {"c/src/lib.rs": "#[cfg(test)]\nmod helpers;\n",
                 "c/src/helpers.rs": self.TEXT,
                 "c/src/helpers/more.rs": self.TEXT,
                 "c/src/x.rs": "#[cfg(test)]\nmod inner;\n",
                 "c/src/x/inner.rs": self.TEXT,
                 "c/src/y.rs": self.TEXT}
        self.assertEqual(self.scan(files), [("c/src/y.rs", "a")])


class Problems(unittest.TestCase):
    def test_a_text_error_not_excepted_is_named(self):
        self.assertEqual(check.problems([("c/src/lib.rs", "a")], {}),
                         ["c/src/lib.rs: fn a returns a text error; give it an error type (RUST.md 8)"])

    def test_an_excepted_text_error_passes(self):
        self.assertEqual(check.problems([("c/src/lib.rs", "a")], {"c/src/lib.rs": {"a"}}), [])

    def test_an_exception_no_longer_found_is_named(self):
        self.assertEqual(check.problems([], {"c/src/lib.rs": {"a"}}),
                         ["c/src/lib.rs: fn a is listed as an exception but returns no text error; "
                          "take it off the list"])


if __name__ == "__main__":
    unittest.main()
