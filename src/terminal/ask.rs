//! Asking setup's questions on the terminal.

use kkt_setup as setup;
use kkt_terminal_os::Terminal;

/// Asks on the terminal, or, with none, refuses each question naming the
/// flag that answers it.
pub struct TerminalAsk {
    pub interactive: bool,
}

impl TerminalAsk {
    fn choose_with(
        question: setup::Question,
        options: &[&str],
        default: usize,
        mut line: impl FnMut(&str) -> Result<String, String>,
    ) -> Result<usize, String> {
        let mut prompt = format!("{}\n", question.text);
        for (i, option) in options.iter().enumerate() {
            prompt.push_str(&format!("  {}. {option}\n", i + 1));
        }
        prompt.push_str(&format!("Choose a number [{}]: ", default + 1));
        loop {
            let answer = line(&prompt)?;
            if answer.trim().is_empty() {
                return Ok(default);
            }
            match answer.trim().parse::<usize>() {
                Ok(n) if (1..=options.len()).contains(&n) => return Ok(n - 1),
                _ => Terminal.output(format_args!("Type a number from 1 to {}.\n", options.len())),
            }
        }
    }

    fn confirm_with(
        question: setup::Question,
        mut line: impl FnMut(&str) -> Result<String, String>,
    ) -> Result<bool, String> {
        let choices = match question.default {
            Some(true) => "[Y/n]",
            Some(false) => "[y/N]",
            None => "[y/n]",
        };
        let prompt = format!("{} {choices} ", question.text);
        loop {
            let answer = line(&prompt)?;
            match answer.trim().to_lowercase().as_str() {
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                "" => {
                    if let Some(default) = question.default {
                        return Ok(default);
                    }
                    Terminal.output(format_args!("Type y or n.\n"));
                }
                _ => Terminal.output(format_args!("Type y or n.\n")),
            }
        }
    }

    fn text_with(
        question: setup::Question,
        default: &str,
        mut line: impl FnMut(&str) -> Result<String, String>,
    ) -> Result<String, String> {
        let shown = if default.is_empty() { "none" } else { default };
        let answer = line(&format!("{} [{shown}] ", question.text))?;
        Ok(if answer.trim().is_empty() {
            default.to_string()
        } else {
            answer
        })
    }

    fn line(&self, question: setup::Question, prompt: &str) -> Result<String, String> {
        if !self.interactive {
            return Err(setup::unanswered(question));
        }
        Terminal.output(format_args!("{prompt}"));
        Terminal
            .flush_output()
            .map_err(|e| format!("could not write to the terminal: {e}"))?;
        let mut line = String::new();
        let read = std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| format!("could not read the answer: {e}"))?;
        if read == 0 {
            return Err(format!("no answer: {}", setup::unanswered(question)));
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }
}

impl setup::Ask for TerminalAsk {
    fn interactive(&self) -> bool {
        self.interactive
    }

    fn choose(
        &self,
        question: setup::Question,
        options: &[&str],
        default: usize,
    ) -> Result<usize, String> {
        Self::choose_with(question, options, default, |prompt| {
            self.line(question, prompt)
        })
    }

    fn confirm(&self, question: setup::Question) -> Result<bool, String> {
        Self::confirm_with(question, |prompt| self.line(question, prompt))
    }

    fn text(&self, question: setup::Question, default: &str) -> Result<String, String> {
        Self::text_with(question, default, |prompt| self.line(question, prompt))
    }

    fn tell(&self, text: &str) {
        Terminal.output(format_args!("{text}\n"));
    }
}

#[cfg(test)]
mod terminal_ask_tests {
    use super::*;
    use kkt_setup::Ask;
    use kkt_setup::testing as defaults;

    fn question(default: Option<bool>) -> setup::Question<'static> {
        setup::Question {
            text: "Connect Codex to kkt?",
            flag: "--clients",
            default,
        }
    }

    fn mode_question() -> setup::Question<'static> {
        setup::Question {
            text: "What is this machine for?",
            flag: "--mode",
            default: None,
        }
    }

    const MODES: [&str; 3] = ["Docker (Recommended)", "Service", "Client"];

    #[test]
    fn text_defaults_are_shown_and_enter_accepts_them() {
        for default in [defaults::DEFAULT_SETUP_URL, defaults::DEFAULT_SETUP_KEY] {
            for answer in ["", "  ", "explicit answer"] {
                let result = TerminalAsk::text_with(question(None), default, |prompt| {
                    let shown = if default.is_empty() { "none" } else { default };
                    assert_eq!(prompt, format!("Connect Codex to kkt? [{shown}] "));
                    Ok(answer.into())
                });
                assert_eq!(
                    result,
                    Ok(if answer.trim().is_empty() {
                        default
                    } else {
                        answer
                    }
                    .into())
                );
            }
            let ask = TerminalAsk { interactive: false };
            assert_eq!(
                ask.text(question(None), default),
                Err(setup::unanswered(question(None)))
            );
            assert_eq!(
                TerminalAsk::text_with(question(None), default, |_| Err("no answer".into())),
                Err("no answer".into())
            );
        }
    }

    #[test]
    fn enter_selects_the_displayed_mode_default() {
        for answer in ["", "  "] {
            assert_eq!(
                TerminalAsk::choose_with(mode_question(), &MODES, 0, |prompt| {
                    assert_eq!(
                        prompt,
                        "What is this machine for?\n  1. Docker (Recommended)\n  2. Service\n  3. Client\nChoose a number [1]: "
                    );
                    Ok(answer.into())
                }),
                Ok(0)
            );
        }
    }

    #[test]
    fn explicit_mode_choices_override_the_default() {
        for (answer, expected) in [("1", 0), (" 2 ", 1), ("3", 2)] {
            assert_eq!(
                TerminalAsk::choose_with(mode_question(), &MODES, 0, |_| Ok(answer.into())),
                Ok(expected)
            );
        }
    }

    #[test]
    fn invalid_mode_choices_ask_again() {
        let mut answers = ["0", "4", "-1", "docker", ""].into_iter();
        assert_eq!(
            TerminalAsk::choose_with(mode_question(), &MODES, 0, |_| {
                Ok(answers.next().expect("another answer").into())
            }),
            Ok(0)
        );
        assert_eq!(answers.next(), None);
    }

    #[test]
    fn the_mode_default_never_replaces_a_missing_answer() {
        let ask = TerminalAsk { interactive: false };
        assert_eq!(
            ask.choose(mode_question(), &MODES, 0),
            Err(setup::unanswered(mode_question()))
        );
        assert_eq!(
            TerminalAsk::choose_with(mode_question(), &MODES, 0, |_| Err("no answer".into())),
            Err("no answer".into())
        );
    }

    #[test]
    fn enter_accepts_the_displayed_default() {
        for (default, prompt, expected) in [
            (true, "Connect Codex to kkt? [Y/n] ", true),
            (false, "Connect Codex to kkt? [y/N] ", false),
        ] {
            for answer in ["", "  "] {
                let got = TerminalAsk::confirm_with(question(Some(default)), |shown| {
                    assert_eq!(shown, prompt);
                    Ok(answer.to_string())
                });
                assert_eq!(got, Ok(expected));
            }
        }
    }

    #[test]
    fn explicit_answers_override_any_default() {
        for default in [None, Some(true), Some(false)] {
            for (answer, expected) in [("y", true), (" YES ", true), ("n", false), (" No ", false)]
            {
                assert_eq!(
                    TerminalAsk::confirm_with(question(default), |_| Ok(answer.into())),
                    Ok(expected)
                );
            }
        }
    }

    #[test]
    fn without_a_default_enter_and_invalid_answers_ask_again() {
        let mut answers = ["", "maybe", "n"].into_iter();
        let got = TerminalAsk::confirm_with(question(None), |prompt| {
            assert_eq!(prompt, "Connect Codex to kkt? [y/n] ");
            Ok(answers.next().expect("another answer").to_string())
        });
        assert_eq!(got, Ok(false));
        assert_eq!(answers.next(), None);
    }

    #[test]
    fn an_invalid_answer_does_not_accept_a_default() {
        let mut answers = ["maybe", "n"].into_iter();
        assert_eq!(
            TerminalAsk::confirm_with(question(Some(true)), |_| {
                Ok(answers.next().expect("another answer").to_string())
            }),
            Ok(false)
        );
        assert_eq!(answers.next(), None);
    }

    #[test]
    fn a_default_never_answers_without_a_terminal() {
        let ask = TerminalAsk { interactive: false };
        for default in [None, Some(true), Some(false)] {
            assert_eq!(
                ask.confirm(question(default)),
                Err(setup::unanswered(question(default)))
            );
        }
    }

    #[test]
    fn a_read_failure_is_returned_even_with_a_default() {
        assert_eq!(
            TerminalAsk::confirm_with(question(Some(true)), |_| Err("no answer".into())),
            Err("no answer".into())
        );
    }
}
