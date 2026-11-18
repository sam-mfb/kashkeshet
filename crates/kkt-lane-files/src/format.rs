//! One lane as a Markdown file, `<lane>.md`: a short header with its name
//! and description, then its instructions.
//!
//! ```text
//! ---
//! name: reviewer
//! description: Reviews pull requests before they merge.
//! ---
//!
//! The instructions, as Markdown or plain text.
//! ```
//!
//! [`render`] and [`parse`] undo each other exactly, so exporting and then
//! importing a lane changes nothing.

/// The file name extension of a lane file.
pub(crate) const EXTENSION: &str = "md";

/// The line that opens and closes the header.
const FENCE: &str = "---";

/// What is wrong with a lane file, worded for the person who wrote it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("it must start with a {FENCE} line, then name: and description: lines, then {FENCE}")]
    NoHeader,
    #[error("header line {line:?} is not `name: ...` or `description: ...`")]
    NotAHeaderLine { line: String },
    #[error("the header has {key:?}; it takes only name and description")]
    UnknownKey { key: String },
    #[error("the header gives {0} twice")]
    Twice(HeaderKey),
    #[error("the header has no closing {FENCE} line")]
    NoClosingLine,
    #[error("the header has no name: line")]
    NoName,
    #[error(
        "the header names the lane {name:?}, but the file is {stem}.{EXTENSION}; they must match"
    )]
    NameMismatch { name: String, stem: String },
    #[error("the header has no description: line")]
    NoDescription,
}

/// A line a lane file's header takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderKey {
    Name,
    Description,
}

impl std::fmt::Display for HeaderKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HeaderKey::Name => "name",
            HeaderKey::Description => "description",
        })
    }
}

/// A lane as a file holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneFile {
    pub name: String,
    pub description: String,
    pub text: String,
}

/// The file's text: the header, a blank line, the instructions and a line
/// end. The line end is `\n`, or `\r\n` when the instructions end with
/// `\r`: [`parse`] takes a final `\r\n` as one line end, so the `\r` in
/// the instructions is kept.
pub fn render(lane: &LaneFile) -> String {
    let end = if lane.text.ends_with('\r') {
        "\r\n"
    } else {
        "\n"
    };
    format!(
        "{FENCE}\nname: {}\ndescription: {}\n{FENCE}\n\n{}{end}",
        lane.name, lane.description, lane.text
    )
}

/// Whether `contents` starts with a lane header's opening line, so it is
/// meant as a lane file: [`parse`] then reads it or says what is wrong.
pub fn has_header(contents: &str) -> bool {
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    contents
        .split_inclusive('\n')
        .next()
        .is_some_and(|line| line_text(line) == FENCE)
}

/// Reads a lane file whose name, without `.md`, is `stem`. The header must
/// name the same lane as the file, and give a description. One blank line
/// after the header and one line end at the end of the file are not part of
/// the instructions; everything else is, as written. Lines may end with
/// `\r\n`. Whether the name, description and instructions are acceptable is
/// for the lane rules to say.
pub fn parse(stem: &str, contents: &str) -> Result<LaneFile, ParseError> {
    if !has_header(contents) {
        return Err(ParseError::NoHeader);
    }
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    let mut lines = contents.split_inclusive('\n');
    let opening = lines.next().unwrap_or_default();
    // How many bytes of `contents` the header has taken so far.
    let mut used = opening.len();
    let mut name = None;
    let mut description = None;
    let mut closed = false;
    for line in lines.by_ref() {
        used += line.len();
        let text = line_text(line);
        if text == FENCE {
            closed = true;
            break;
        }
        if text.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = text.split_once(':') else {
            return Err(ParseError::NotAHeaderLine {
                line: text.to_string(),
            });
        };
        let (slot, key) = match key.trim() {
            "name" => (&mut name, HeaderKey::Name),
            "description" => (&mut description, HeaderKey::Description),
            other => {
                return Err(ParseError::UnknownKey {
                    key: other.to_string(),
                });
            }
        };
        if slot.is_some() {
            return Err(ParseError::Twice(key));
        }
        *slot = Some(value.trim().to_string());
    }
    if !closed {
        return Err(ParseError::NoClosingLine);
    }
    let name = name.ok_or(ParseError::NoName)?;
    if name != stem {
        return Err(ParseError::NameMismatch {
            name,
            stem: stem.to_string(),
        });
    }
    let description = description.ok_or(ParseError::NoDescription)?;
    let rest = &contents[used..];
    let rest = strip_line_end_prefix(rest);
    let text = strip_line_end_suffix(rest);
    Ok(LaneFile {
        name,
        description,
        text: text.to_string(),
    })
}

/// A line without its line end.
fn line_text(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

fn strip_line_end_prefix(text: &str) -> &str {
    text.strip_prefix("\r\n")
        .or_else(|| text.strip_prefix('\n'))
        .unwrap_or(text)
}

fn strip_line_end_suffix(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(text: &str) -> LaneFile {
        LaneFile {
            name: "reviewer".into(),
            description: "Reviews pull requests: carefully.".into(),
            text: text.into(),
        }
    }

    #[test]
    fn a_rendered_lane_reads_back_the_same() {
        for text in [
            "Review it.",
            "Review it.\n",
            "\nStarts blank and ends blank.\n\n",
            "# Heading\n\n---\n\nA rule above, not a header.",
            "  indented",
            "Read.\r",
            "Read.\r\r",
            "Read.\r\n",
            "Read.\n\r",
            "\r",
            "\r\nStarts with a carriage return.",
            "\rLines\rwith\rcarriage\rreturns\r",
        ] {
            let lane = lane(text);
            assert_eq!(parse("reviewer", &render(&lane)), Ok(lane), "{text:?}");
        }
    }

    #[test]
    fn the_file_format_is_fixed() {
        assert_eq!(
            render(&lane("Review it.")),
            "---\nname: reviewer\ndescription: Reviews pull requests: carefully.\n---\n\nReview it.\n"
        );
        assert_eq!(
            render(&lane("Review it.\r")),
            "---\nname: reviewer\ndescription: Reviews pull requests: carefully.\n---\n\nReview it.\r\r\n"
        );
    }

    #[test]
    fn a_hand_written_file_reads_with_crlf_a_bom_and_no_blank_line() {
        let parsed = parse(
            "reviewer",
            "\u{feff}---\r\ndescription:   Reviews.  \r\n\r\nname: reviewer\r\n---\r\nLine one\r\nLine two\r\n",
        );
        assert_eq!(
            parsed,
            Ok(LaneFile {
                name: "reviewer".into(),
                description: "Reviews.".into(),
                text: "Line one\r\nLine two".into(),
            })
        );
        let no_end = parse("reviewer", "---\nname: reviewer\ndescription: d\n---\nText");
        assert_eq!(no_end.map(|l| l.text), Ok("Text".to_string()));
    }

    #[test]
    fn a_file_has_a_header_when_its_first_line_is_the_fence() {
        for contents in ["---\n", "---", "---\r\nname: x\n", "\u{feff}---\n"] {
            assert!(has_header(contents), "{contents:?}");
        }
        for contents in ["", "# Notes\n---\n", " ---\n", "----\n", "\n---\n"] {
            assert!(!has_header(contents), "{contents:?}");
        }
    }

    #[test]
    fn a_malformed_file_is_refused_saying_why() {
        let cases = [
            ("name: reviewer\n", "must start with a ---"),
            ("", "must start with a ---"),
            ("---\nname: reviewer\ndescription: d\n", "no closing ---"),
            ("---\ndescription: d\n---\nx\n", "no name:"),
            ("---\nname: reviewer\n---\nx\n", "no description:"),
            (
                "---\nname: other\ndescription: d\n---\nx\n",
                "file is reviewer.md",
            ),
            (
                "---\nname: reviewer\nname: reviewer\ndescription: d\n---\nx\n",
                "name twice",
            ),
            (
                "---\nname: reviewer\nowner: me\ndescription: d\n---\nx\n",
                "\"owner\"",
            ),
            (
                "---\nname reviewer\ndescription: d\n---\nx\n",
                "is not `name",
            ),
        ];
        for (contents, says) in cases {
            let error = parse("reviewer", contents).expect_err(contents).to_string();
            assert!(error.contains(says), "{contents:?}: {error}");
        }
    }
}
