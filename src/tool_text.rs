//! What agents are told about each tool and each of its parameters. The text
//! lives in `agent-tools.toml` (for `/mcp`) and `admin-tools.toml` (for
//! `/mcp-admin`), beside `agent-instructions.md`, so it can be edited without
//! reading code. `build.rs` turns each file into a constant here; nothing is
//! read at run time.

/// One tool's text: its description and each parameter's, with limits still
/// written as `{placeholders}`.
pub(crate) struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// Each parameter's name and text.
    pub parameters: &'static [(&'static str, &'static str)],
}

// Defines `AGENT` (from agent-tools.toml) and `ADMIN` (from admin-tools.toml).
include!(concat!(env!("OUT_DIR"), "/tool_text.rs"));

impl Tool {
    /// The text for the parameter `name`, if there is one.
    pub fn parameter(&self, name: &str) -> Option<&'static str> {
        self.parameters
            .iter()
            .find(|(parameter, _)| *parameter == name)
            .map(|(_, text)| *text)
    }
}

/// The text for the tool `name` in `texts`, if there is one.
pub(crate) fn find<'a>(texts: &'a [Tool], name: &str) -> Option<&'a Tool> {
    texts.iter().find(|tool| tool.name == name)
}

/// A tool a server offers: its name and its parameters' names.
#[cfg(test)]
pub(crate) struct Offered {
    pub name: String,
    pub parameters: Vec<String>,
}

/// Everything that does not match between `texts` and the tools a server
/// offers, one line each: a tool or parameter with no text, and text for a
/// tool or parameter that does not exist. Empty when they match.
#[cfg(test)]
pub(crate) fn mismatches(texts: &[Tool], offered: &[Offered]) -> Vec<String> {
    let mut out = Vec::new();
    for tool in offered {
        let Some(text) = find(texts, &tool.name) else {
            out.push(format!("tool `{}` has no text", tool.name));
            continue;
        };
        for parameter in &tool.parameters {
            if text.parameter(parameter).is_none() {
                out.push(format!(
                    "parameter `{parameter}` of tool `{}` has no text",
                    tool.name
                ));
            }
        }
        for (parameter, _) in text.parameters {
            if !tool.parameters.iter().any(|p| p == parameter) {
                out.push(format!(
                    "text for parameter `{parameter}` of tool `{}`, which has no such parameter",
                    tool.name
                ));
            }
        }
    }
    for text in texts {
        if !offered.iter().any(|tool| tool.name == text.name) {
            out.push(format!(
                "text for tool `{}`, which does not exist",
                text.name
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXTS: &[Tool] = &[
        Tool {
            name: "send",
            description: "Send a message.",
            parameters: &[("body", "The text."), ("colour", "Unused.")],
        },
        Tool {
            name: "gone",
            description: "A tool that was removed.",
            parameters: &[],
        },
    ];

    fn offered(name: &str, parameters: &[&str]) -> Offered {
        Offered {
            name: name.to_string(),
            parameters: parameters.iter().map(|p| p.to_string()).collect(),
        }
    }

    #[test]
    fn matching_text_has_no_mismatches() {
        let texts = &TEXTS[..1];
        let tools = [offered("send", &["body", "colour"])];
        assert!(mismatches(texts, &tools).is_empty());
    }

    #[test]
    fn mismatches_name_each_missing_or_extra_item() {
        let tools = [
            offered("send", &["body", "format"]),
            offered("read", &["channel_id"]),
        ];
        assert_eq!(
            mismatches(TEXTS, &tools),
            [
                "parameter `format` of tool `send` has no text",
                "text for parameter `colour` of tool `send`, which has no such parameter",
                "tool `read` has no text",
                "text for tool `gone`, which does not exist",
            ]
        );
    }

    #[test]
    fn parameters_are_found_by_name() {
        assert_eq!(TEXTS[0].parameter("body"), Some("The text."));
        assert_eq!(TEXTS[0].parameter("format"), None);
        assert!(find(TEXTS, "gone").is_some());
        assert!(find(TEXTS, "read").is_none());
    }

    /// The files' line breaks become single spaces, as `build.rs` promises.
    #[test]
    fn line_breaks_become_single_spaces() {
        let register = find(AGENT, "register").expect("register");
        assert!(register.description.starts_with("Get a new identity:"));
        assert!(!register.description.contains('\n'));
        assert!(!register.description.contains("  "));
        assert!(register.description.contains("shown once. Takes no"));
    }
}
