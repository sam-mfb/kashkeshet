//! The output format a tool call may ask for, as the tools' inputs take it.
//! The MCP library describes each input with a JSON schema, so the choice
//! derives one here, in the crate that wraps the library, and the call
//! turns it into kkt-mcp-formatter's [`Format`]. The schema keeps the name
//! `Format` and the variants' descriptions the tools have always shown.

use kkt_mcp_formatter::Format;
use schemars::JsonSchema;
use serde::Deserialize;

// No doc comment on the type itself: schemars would add it to the
// published schema, which has never had one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(rename = "Format")]
pub(crate) enum FormatChoice {
    /// Compact lines of text. The default.
    Text,
    /// A JSON object.
    Json,
}

/// The format a call asked for, or text when it asked for none.
pub(crate) fn chosen(choice: Option<FormatChoice>) -> Format {
    match choice {
        None | Some(FormatChoice::Text) => Format::Text,
        Some(FormatChoice::Json) => Format::Json,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_choice_is_text_and_each_choice_is_its_format() {
        assert_eq!(chosen(None), Format::Text);
        assert_eq!(chosen(Some(FormatChoice::Text)), Format::Text);
        assert_eq!(chosen(Some(FormatChoice::Json)), Format::Json);
    }

    #[test]
    fn the_choice_is_read_from_its_lowercase_name() {
        let read = |text: &str| serde_json::from_str::<FormatChoice>(text);
        assert_eq!(read("\"text\"").expect("text"), FormatChoice::Text);
        assert_eq!(read("\"json\"").expect("json"), FormatChoice::Json);
        assert!(read("\"Text\"").is_err());
    }
}
