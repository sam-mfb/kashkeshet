"""check-lint-exceptions.py: no `allow` attribute, and every `expect` gives a reason."""

import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "check_lint_exceptions", Path(__file__).resolve().parent / "check-lint-exceptions.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


class ProblemsIn(unittest.TestCase):
    def found(self, text):
        return check.problems_in(text, "f.rs")

    def test_an_allow_is_found_in_each_form(self):
        for text in ["#[allow(dead_code)]\n", "#![allow(dead_code)]\n", "    #[ allow (clippy::x)]\n",
                     '#[cfg_attr(feature = "x", allow(dead_code))]\n']:
            self.assertEqual(len(self.found(text)), 1, text)

    def test_the_line_is_named(self):
        self.assertEqual(self.found("fn a() {}\n#[allow(unused)]\nfn b() {}\n"),
                         ['f.rs:2: use #[expect(.., reason = "..")], not allow: #[allow(unused)]'])

    def test_an_expect_with_a_reason_passes_on_one_line_or_several(self):
        self.assertEqual(self.found('#[expect(dead_code, reason = "shared")]\n'), [])
        self.assertEqual(self.found('#![expect(\n    dead_code,\n    reason = "shared"\n)]\n'), [])

    def test_an_expect_without_a_reason_is_found(self):
        self.assertEqual(self.found("fn a() {}\n#[expect(\n    dead_code\n)]\n"),
                         ["f.rs:2: an expect gives no reason: #[expect( dead_code )]"])

    def test_an_allow_split_over_lines_by_rustfmt_is_found(self):
        text = "#![cfg_attr(\n    not(test),\n    allow(dead_code)\n)]\n"
        self.assertEqual(self.found(text),
                         ['f.rs:1: use #[expect(.., reason = "..")], not allow: '
                          "#![cfg_attr( not(test), allow(dead_code) )]"])

    def test_an_expect_inside_cfg_attr_needs_a_reason_too(self):
        self.assertEqual(len(self.found("#[cfg_attr(not(test), expect(dead_code))]\n")), 1)
        text = ('#![cfg_attr(\n    not(test),\n    expect(\n        dead_code,\n'
                '        reason = "built for testing alone, some are not used"\n    )\n)]\n')
        self.assertEqual(self.found(text), [])

    def test_a_reason_belongs_to_its_own_expect(self):
        text = '#[expect(dead_code)]\n#[doc = "reason = x"]\nfn a() {}\n'
        self.assertEqual(len(self.found(text)), 1)

    def test_an_attribute_with_a_closing_bracket_in_a_string_is_read_whole(self):
        text = '#[expect(dead_code, reason = "a ] in the reason")]\n#[allow(unused)]\n'
        self.assertEqual(len(self.found(text)), 1)

    def test_a_comment_or_a_call_named_allow_is_not_an_attribute(self):
        self.assertEqual(self.found("// #[allow(dead_code)] is not used here\nif limiter.allow(now) {}\n"), [])


if __name__ == "__main__":
    unittest.main()
