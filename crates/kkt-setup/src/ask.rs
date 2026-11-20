//! Talking with the person running `kkt setup`. Every question has a flag
//! that answers it instead, so setup also runs with no one to ask.

/// A question, and the flag that answers it without asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Question<'a> {
    pub text: &'a str,
    /// Plain lowercase phrase completing `"no terminal to ask {topic}"` and
    /// `"no answer when asked {topic}"`, such as `"for the server's MCP address"`
    /// or `"what this machine is for"`. Falls back to `text` when empty.
    pub topic: &'a str,
    /// Such as `--mode`. Empty for a question only asked at a terminal,
    /// such as whether to try a failed step again.
    pub flag: &'static str,
    /// The answer Enter accepts for a yes/no question at a terminal.
    /// This never answers a question when there is no terminal.
    pub default: Option<bool>,
}

/// Why the terminal could not be read: the caller's own error, passed on
/// unread.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct LineError(pub Box<dyn std::error::Error + Send + Sync>);

/// Why a question got no answer.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    /// There is no terminal to ask on; the flag answers it instead.
    #[error("{}", no_terminal(text, flag))]
    NoTerminal { text: String, flag: &'static str },
    /// The input ended before an answer.
    #[error("{}", no_answer(text, flag))]
    NoAnswer { text: String, flag: &'static str },
    /// The terminal could not be read.
    #[error(transparent)]
    Read(LineError),
}

fn no_terminal(text: &str, flag: &str) -> String {
    if flag.is_empty() {
        format!("no terminal to ask {text}")
    } else {
        format!("no terminal to ask {text}; pass {flag}")
    }
}

fn no_answer(text: &str, flag: &str) -> String {
    if flag.is_empty() {
        format!("no answer when asked {text}")
    } else {
        format!("no answer when asked {text}; pass {flag}")
    }
}

fn error_topic(question: Question<'_>) -> String {
    if question.topic.is_empty() {
        question.text.to_string()
    } else {
        question.topic.to_string()
    }
}

/// Asks questions and says what happened. [`Prompter`] asks over the
/// caller's [`Lines`]; tests can use scripted answers.
pub trait Ask {
    /// Whether there is someone to ask. Without, every question needs its
    /// flag, and a failed step ends setup instead of being offered again.
    fn interactive(&self) -> bool;
    /// The index of the option chosen from `options`. Enter accepts `default`
    /// at a terminal; without a terminal the question still needs its flag.
    fn choose(
        &self,
        question: Question,
        options: &[&str],
        default: usize,
    ) -> Result<usize, AskError>;
    /// Yes or no.
    fn confirm(&self, question: Question) -> Result<bool, AskError>;
    /// A line of text. Enter accepts `default` at a terminal; without a
    /// terminal the question still needs its flag.
    fn text(&self, question: Question, default: &str) -> Result<String, AskError>;
    /// Says something, such as what was just done.
    fn tell(&self, text: &str);
}

/// The error for a question there is no one to ask: it names the flag that
/// answers it.
pub fn unanswered(question: Question) -> AskError {
    AskError::NoTerminal {
        text: error_topic(question),
        flag: question.flag,
    }
}

/// The caller's line input and output for setup's questions.
pub trait Lines {
    /// Whether someone can answer a question on this input.
    fn interactive(&self) -> bool;
    /// Says one complete line.
    fn say(&self, text: &str);
    /// Shows and flushes the prompt, then reads a line; None means EOF.
    fn read_line(&self, prompt: &str) -> Result<Option<String>, LineError>;
}

use crate as setup;

/// Asks on the terminal, or, with none, refuses each question naming the
/// flag that answers it.
pub struct Prompter<'a> {
    lines: &'a dyn Lines,
    interactive: bool,
}

impl<'a> Prompter<'a> {
    /// Asks through the caller's line input and output.
    pub fn new(lines: &'a dyn Lines) -> Self {
        Self {
            lines,
            interactive: lines.interactive(),
        }
    }

    fn choose_with(
        question: setup::Question,
        options: &[&str],
        default: usize,
        say: impl Fn(&str),
        mut line: impl FnMut(&str) -> Result<String, AskError>,
    ) -> Result<usize, AskError> {
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
                _ => say(&format!("Type a number from 1 to {}.", options.len())),
            }
        }
    }

    fn confirm_with(
        question: setup::Question,
        say: impl Fn(&str),
        mut line: impl FnMut(&str) -> Result<String, AskError>,
    ) -> Result<bool, AskError> {
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
                    say("Type y or n.");
                }
                _ => say("Type y or n."),
            }
        }
    }

    fn text_with(
        question: setup::Question,
        default: &str,
        mut line: impl FnMut(&str) -> Result<String, AskError>,
    ) -> Result<String, AskError> {
        let shown = if default.is_empty() { "none" } else { default };
        let answer = line(&format!("{} [{shown}] ", question.text))?;
        Ok(if answer.trim().is_empty() {
            default.to_string()
        } else {
            answer
        })
    }

    fn line(&self, question: setup::Question, prompt: &str) -> Result<String, AskError> {
        if !self.interactive {
            return Err(setup::unanswered(question));
        }
        let Some(line) = self.lines.read_line(prompt).map_err(AskError::Read)? else {
            return Err(AskError::NoAnswer {
                text: error_topic(question),
                flag: question.flag,
            });
        };
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }
}

impl setup::Ask for Prompter<'_> {
    fn interactive(&self) -> bool {
        self.interactive
    }

    fn choose(
        &self,
        question: setup::Question,
        options: &[&str],
        default: usize,
    ) -> Result<usize, AskError> {
        Self::choose_with(
            question,
            options,
            default,
            |text| self.lines.say(text),
            |prompt| self.line(question, prompt),
        )
    }

    fn confirm(&self, question: setup::Question) -> Result<bool, AskError> {
        Self::confirm_with(
            question,
            |text| self.lines.say(text),
            |prompt| self.line(question, prompt),
        )
    }

    fn text(&self, question: setup::Question, default: &str) -> Result<String, AskError> {
        Self::text_with(question, default, |prompt| self.line(question, prompt))
    }

    fn tell(&self, text: &str) {
        self.lines.say(text);
    }
}

#[cfg(test)]
mod terminal_ask_tests {
    use super::*;
    use crate::Ask;
    use crate::flow as defaults;

    fn question(default: Option<bool>) -> setup::Question<'static> {
        setup::Question {
            text: "Connect Codex to kkt?",
            topic: "whether to connect Codex to kkt",
            flag: "--clients",
            default,
        }
    }

    fn mode_question() -> setup::Question<'static> {
        setup::Question {
            text: "What is this machine for?",
            topic: "what this machine is for",
            flag: "--mode",
            default: None,
        }
    }

    struct ScriptedLines {
        answers: std::cell::RefCell<std::collections::VecDeque<Option<String>>>,
        said: std::cell::RefCell<Vec<String>>,
        prompts: std::cell::RefCell<Vec<String>>,
    }

    impl ScriptedLines {
        fn new(answers: impl IntoIterator<Item = Option<&'static str>>) -> Self {
            Self {
                answers: std::cell::RefCell::new(
                    answers.into_iter().map(|s| s.map(str::to_owned)).collect(),
                ),
                said: std::cell::RefCell::default(),
                prompts: std::cell::RefCell::default(),
            }
        }
    }

    impl Lines for ScriptedLines {
        fn interactive(&self) -> bool {
            true
        }
        fn say(&self, text: &str) {
            self.said.borrow_mut().push(text.to_owned());
        }
        fn read_line(&self, prompt: &str) -> Result<Option<String>, LineError> {
            self.prompts.borrow_mut().push(prompt.to_owned());
            Ok(self
                .answers
                .borrow_mut()
                .pop_front()
                .expect("scripted answer"))
        }
    }

    #[test]
    fn the_prompter_asks_again_through_its_lines_and_keeps_text_whitespace() {
        let lines = ScriptedLines::new([Some("0\n"), Some("2\r\n"), Some("  chosen  \r\n")]);
        let ask = Prompter::new(&lines);
        assert_eq!(
            ask.choose(mode_question(), &MODES, 0)
                .map_err(|e| e.to_string()),
            Ok(1)
        );
        assert_eq!(*lines.said.borrow(), ["Type a number from 1 to 3."]);
        assert_eq!(lines.prompts.borrow()[0], lines.prompts.borrow()[1]);
        assert_eq!(
            ask.text(question(None), "default")
                .map_err(|e| e.to_string()),
            Ok("  chosen  ".into())
        );
        ask.tell("finished");
        assert_eq!(
            lines.said.borrow().last().map(String::as_str),
            Some("finished")
        );
    }

    #[test]
    fn end_of_input_never_supplies_a_default() {
        let lines = ScriptedLines::new([None, None, None]);
        let ask = Prompter::new(&lines);
        assert_eq!(
            ask.confirm(question(Some(true))).map_err(|e| e.to_string()),
            Err("no answer when asked whether to connect Codex to kkt; pass --clients".to_string())
        );
        assert_eq!(
            ask.text(question(Some(true)), "default")
                .map_err(|e| e.to_string()),
            Err("no answer when asked whether to connect Codex to kkt; pass --clients".to_string())
        );
        assert_eq!(
            ask.choose(mode_question(), &MODES, 0)
                .map_err(|e| e.to_string()),
            Err("no answer when asked what this machine is for; pass --mode".to_string())
        );
    }

    /// What a question says when it gets no answer, pinned exactly.
    #[test]
    fn every_unanswered_question_says_what_it_always_said() {
        let flagless = setup::Question {
            text: "Continue?",
            topic: "whether to continue",
            flag: "",
            default: None,
        };
        assert_eq!(
            setup::unanswered(flagless).to_string(),
            "no terminal to ask whether to continue"
        );
        let not_there = NotInteractive;
        assert_eq!(
            Prompter::new(&not_there)
                .confirm(question(None))
                .map_err(|e| e.to_string()),
            Err("no terminal to ask whether to connect Codex to kkt; pass --clients".to_string())
        );
        let ended = ScriptedLines::new([None]);
        assert_eq!(
            Prompter::new(&ended)
                .confirm(question(None))
                .map_err(|e| e.to_string()),
            Err("no answer when asked whether to connect Codex to kkt; pass --clients".to_string())
        );
        let broken = Broken;
        assert_eq!(
            Prompter::new(&broken)
                .text(question(None), "")
                .map_err(|e| e.to_string()),
            Err("could not read the terminal: closed".to_string())
        );
    }

    /// A terminal that cannot be read.
    struct Broken;
    impl Lines for Broken {
        fn interactive(&self) -> bool {
            true
        }
        fn say(&self, _text: &str) {}
        fn read_line(&self, _prompt: &str) -> Result<Option<String>, LineError> {
            Err(LineError("could not read the terminal: closed".into()))
        }
    }

    struct NotInteractive;
    impl Lines for NotInteractive {
        fn interactive(&self) -> bool {
            false
        }
        fn say(&self, _text: &str) {
            panic!("no output before asking");
        }
        fn read_line(&self, _prompt: &str) -> Result<Option<String>, LineError> {
            panic!("no input without a terminal");
        }
    }

    const MODES: [&str; 3] = ["Docker (Recommended)", "Service", "Client"];

    #[test]
    fn text_defaults_are_shown_and_enter_accepts_them() {
        for default in [defaults::DEFAULT_SETUP_URL, defaults::DEFAULT_SETUP_KEY] {
            for answer in ["", "  ", "explicit answer"] {
                let result = Prompter::text_with(question(None), default, |prompt| {
                    let shown = if default.is_empty() { "none" } else { default };
                    assert_eq!(prompt, format!("Connect Codex to kkt? [{shown}] "));
                    Ok(answer.into())
                })
                .map_err(|e| e.to_string());
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
            let ask = Prompter::new(&NotInteractive);
            assert_eq!(
                ask.text(question(None), default).map_err(|e| e.to_string()),
                Err(setup::unanswered(question(None)).to_string())
            );
            assert_eq!(
                Prompter::text_with(question(None), default, |_| Err(setup::unanswered(
                    question(None)
                )))
                .map_err(|e| e.to_string()),
                Err(setup::unanswered(question(None)).to_string())
            );
        }
    }

    #[test]
    fn enter_selects_the_displayed_mode_default() {
        for answer in ["", "  "] {
            assert_eq!(
                Prompter::choose_with(
                    mode_question(),
                    &MODES,
                    0,
                    |_| {},
                    |prompt| {
                        assert_eq!(
                            prompt,
                            "What is this machine for?\n  1. Docker (Recommended)\n  2. Service\n  3. Client\nChoose a number [1]: "
                        );
                        Ok(answer.into())
                    }
                ).map_err(|e| e.to_string()),
                Ok(0)
            );
        }
    }

    #[test]
    fn explicit_mode_choices_override_the_default() {
        for (answer, expected) in [("1", 0), (" 2 ", 1), ("3", 2)] {
            assert_eq!(
                Prompter::choose_with(mode_question(), &MODES, 0, |_| {}, |_| Ok(answer.into()))
                    .map_err(|e| e.to_string()),
                Ok(expected)
            );
        }
    }

    #[test]
    fn invalid_mode_choices_ask_again() {
        let mut answers = ["0", "4", "-1", "docker", ""].into_iter();
        assert_eq!(
            Prompter::choose_with(
                mode_question(),
                &MODES,
                0,
                |_| {},
                |_| { Ok(answers.next().expect("another answer").into()) }
            )
            .map_err(|e| e.to_string()),
            Ok(0)
        );
        assert_eq!(answers.next(), None);
    }

    #[test]
    fn the_mode_default_never_replaces_a_missing_answer() {
        let ask = Prompter::new(&NotInteractive);
        assert_eq!(
            ask.choose(mode_question(), &MODES, 0)
                .map_err(|e| e.to_string()),
            Err(setup::unanswered(mode_question()).to_string())
        );
        assert_eq!(
            Prompter::choose_with(
                mode_question(),
                &MODES,
                0,
                |_| {},
                |_| Err(setup::unanswered(question(None)))
            )
            .map_err(|e| e.to_string()),
            Err(setup::unanswered(question(None)).to_string())
        );
    }

    #[test]
    fn enter_accepts_the_displayed_default() {
        for (default, prompt, expected) in [
            (true, "Connect Codex to kkt? [Y/n] ", true),
            (false, "Connect Codex to kkt? [y/N] ", false),
        ] {
            for answer in ["", "  "] {
                let got = Prompter::confirm_with(
                    question(Some(default)),
                    |_| {},
                    |shown| {
                        assert_eq!(shown, prompt);
                        Ok(answer.to_string())
                    },
                )
                .map_err(|e| e.to_string());
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
                    Prompter::confirm_with(question(default), |_| {}, |_| Ok(answer.into()))
                        .map_err(|e| e.to_string()),
                    Ok(expected)
                );
            }
        }
    }

    #[test]
    fn without_a_default_enter_and_invalid_answers_ask_again() {
        let mut answers = ["", "maybe", "n"].into_iter();
        let got = Prompter::confirm_with(
            question(None),
            |_| {},
            |prompt| {
                assert_eq!(prompt, "Connect Codex to kkt? [y/n] ");
                Ok(answers.next().expect("another answer").to_string())
            },
        )
        .map_err(|e| e.to_string());
        assert_eq!(got, Ok(false));
        assert_eq!(answers.next(), None);
    }

    #[test]
    fn an_invalid_answer_does_not_accept_a_default() {
        let mut answers = ["maybe", "n"].into_iter();
        assert_eq!(
            Prompter::confirm_with(
                question(Some(true)),
                |_| {},
                |_| { Ok(answers.next().expect("another answer").to_string()) }
            )
            .map_err(|e| e.to_string()),
            Ok(false)
        );
        assert_eq!(answers.next(), None);
    }

    #[test]
    fn a_default_never_answers_without_a_terminal() {
        let ask = Prompter::new(&NotInteractive);
        for default in [None, Some(true), Some(false)] {
            assert_eq!(
                ask.confirm(question(default)).map_err(|e| e.to_string()),
                Err(setup::unanswered(question(default)).to_string())
            );
        }
    }

    #[test]
    fn a_read_failure_is_returned_even_with_a_default() {
        assert_eq!(
            Prompter::confirm_with(
                question(Some(true)),
                |_| {},
                |_| Err(setup::unanswered(question(None)))
            )
            .map_err(|e| e.to_string()),
            Err(setup::unanswered(question(None)).to_string())
        );
    }
}
