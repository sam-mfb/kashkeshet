//! What the agent endpoint (`mcp.rs`) and the admin endpoint
//! (`mcp_admin.rs`) share: dressing their tools with text from `tool_text`
//! and with the server's limits, turning a rule's result into a tool result,
//! and reading the parameters both take the same way (the read position, a
//! body cut, an offset into text).
//!
//! Nothing here logs. Logging stays with each endpoint, which passes in what
//! to do with an internal error.

use crate::config::Limits;
use crate::format::{Format, Formatter};
use crate::rules::{self, Position, RuleError};
use crate::tool_text;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;
use std::sync::Arc;

/// `router`'s tools with their descriptions and their parameters' taken from
/// `texts`. A tool or parameter with no text is left without; the tests
/// check there are none.
pub(crate) fn with_text<S>(mut router: ToolRouter<S>, texts: &[tool_text::Tool]) -> ToolRouter<S> {
    for route in router.map.values_mut() {
        let tool = &mut route.attr;
        let Some(text) = tool_text::find(texts, &tool.name) else {
            continue;
        };
        tool.description = Some(text.description.into());
        let schema = Arc::make_mut(&mut tool.input_schema);
        if let Some(Value::Object(properties)) = schema.get_mut("properties") {
            for (name, property) in properties.iter_mut() {
                if let (Some(text), Value::Object(property)) = (text.parameter(name), property) {
                    property.insert("description".into(), text.into());
                }
            }
        }
    }
    router
}

/// The tools in `router` with their parameters' names, to check against
/// their text.
#[cfg(test)]
pub(crate) fn offered<S>(router: &ToolRouter<S>) -> Vec<tool_text::Offered> {
    router
        .map
        .values()
        .map(|route| &route.attr)
        .map(|tool| tool_text::Offered {
            name: tool.name.to_string(),
            parameters: tool
                .input_schema
                .get("properties")
                .and_then(Value::as_object)
                .map(|properties| properties.keys().cloned().collect())
                .unwrap_or_default(),
        })
        .collect()
}

/// `router`'s tools with every limit in their descriptions filled in from
/// `limits`.
pub(crate) fn with_limits<S>(mut router: ToolRouter<S>, limits: &Limits) -> ToolRouter<S> {
    for route in router.map.values_mut() {
        let tool = &mut route.attr;
        if let Some(text) = &tool.description {
            tool.description = Some(describe(text, limits).into());
        }
        let mut schema = Value::Object(tool.input_schema.as_ref().clone());
        describe_schema(&mut schema, limits);
        if let Value::Object(object) = schema {
            tool.input_schema = Arc::new(object);
        }
    }
    router
}

/// Fills in the limit placeholders in a tool or parameter description.
fn describe(text: &str, limits: &Limits) -> String {
    let values = [
        ("{default_limit}", limits.default_limit.to_string()),
        ("{max_limit}", limits.max_limit.to_string()),
        ("{max_body_size}", size(limits.max_body_bytes)),
        (
            "{max_channel_name_chars}",
            limits.max_channel_name_chars.to_string(),
        ),
        ("{max_purpose_chars}", limits.max_purpose_chars.to_string()),
        ("{max_note_chars}", limits.max_note_chars.to_string()),
        (
            "{max_server_announcement_chars}",
            limits.max_server_announcement_chars.to_string(),
        ),
        (
            "{max_memory_entry_chars}",
            limits.max_memory_entry_chars.to_string(),
        ),
        ("{max_memory_chars}", limits.max_memory_chars.to_string()),
        (
            "{max_memory_name_chars}",
            crate::config::MAX_MEMORY_NAME_CHARS.to_string(),
        ),
        (
            "{max_description_chars}",
            limits.max_description_chars.to_string(),
        ),
        (
            "{default_wait_s}",
            limits.default_wait.as_secs().to_string(),
        ),
        ("{max_wait_s}", limits.max_wait.as_secs().to_string()),
        (
            "{max_handle_chars}",
            crate::config::MAX_HANDLE_CHARS.to_string(),
        ),
        ("{max_lane_chars}", limits.max_lane_chars.to_string()),
        (
            "{max_lane_name_chars}",
            crate::config::MAX_LANE_NAME_CHARS.to_string(),
        ),
        (
            "{max_lane_description_chars}",
            crate::config::MAX_LANE_DESCRIPTION_CHARS.to_string(),
        ),
    ];
    values.iter().fold(text.to_string(), |text, (name, value)| {
        text.replace(name, value)
    })
}

/// Fills in every `description` string found anywhere in a JSON schema.
fn describe_schema(schema: &mut Value, limits: &Limits) {
    match schema {
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                match value {
                    Value::String(text) if key == "description" => {
                        *text = describe(text, limits);
                    }
                    _ => describe_schema(value, limits),
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| describe_schema(item, limits)),
        _ => {}
    }
}

/// A byte count as people say it: `8 KB` for whole kilobytes, else `1500 bytes`.
fn size(bytes: usize) -> String {
    if bytes.is_multiple_of(1024) {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

/// A rule's result as a tool result: the text on success, the error
/// formatted in `format` on failure. `log_internal` is called with an
/// internal error before it is formatted; the endpoint logs it there.
pub(crate) fn respond(
    fmt: &Formatter,
    format: Format,
    out: rules::Result<String>,
    log_internal: impl FnOnce(&RuleError),
) -> CallToolResult {
    match out {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(e) => {
            if e.code == rules::Code::Internal {
                log_internal(&e);
            }
            CallToolResult::error(vec![ContentBlock::text(fmt.error(format, &e))])
        }
    }
}

/// The body cut a caller asked for; with none, bodies are cut only to fit
/// the response cap.
pub(crate) fn body_chars(requested: Option<u32>) -> Option<usize> {
    requested.map(|n| (n as usize).max(1))
}

/// Where to read from, given `after_id` and `before_id`. Both at once is
/// refused.
pub(crate) fn position(after_id: Option<i64>, before_id: Option<i64>) -> rules::Result<Position> {
    match (after_id, before_id) {
        (Some(_), Some(_)) => Err(RuleError::invalid(
            "give `after_id` or `before_id`, not both",
        )),
        (Some(a), None) => Ok(Position::After(a)),
        (None, Some(b)) => Ok(Position::Before(b)),
        (None, None) => Ok(Position::Latest),
    }
}

/// The offset a caller asked for into something `len` long, zero if none.
/// An offset past `len` is refused with `past_end` as the message.
pub(crate) fn offset(requested: Option<u32>, len: usize, past_end: &str) -> rules::Result<usize> {
    let offset = requested.unwrap_or(0) as usize;
    if offset > len {
        return Err(RuleError::invalid(past_end));
    }
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::testing::FixedClock;
    use crate::rules::Code;
    use chrono::{TimeZone, Utc};
    use std::time::Duration;

    fn odd_limits() -> Limits {
        Limits {
            default_limit: 7,
            max_limit: 42,
            max_body_bytes: 3000,
            max_channel_name_chars: 33,
            max_purpose_chars: 77,
            max_description_chars: 55,
            max_lane_chars: 666,
            default_wait: Duration::from_secs(5),
            max_wait: Duration::from_secs(9),
            ..Limits::DEFAULT
        }
    }

    fn formatter() -> Formatter {
        let now = Utc.with_ymd_and_hms(2026, 10, 23, 15, 0, 0).unwrap();
        Formatter::new(
            Limits::DEFAULT.max_response_bytes,
            Arc::new(FixedClock::new(now)),
            "1.2.3-test",
        )
    }

    /// The text of a tool result, and whether it is an error.
    fn shown(result: &CallToolResult) -> (String, bool) {
        let json = serde_json::to_value(result).expect("result as JSON");
        let text = json["content"][0]["text"]
            .as_str()
            .expect("one text block")
            .to_string();
        (text, json["isError"] == Value::Bool(true))
    }

    #[test]
    fn describe_fills_every_placeholder() {
        let text = "{default_limit} {max_limit} {max_body_size} \
            {max_channel_name_chars} {max_purpose_chars} {max_description_chars} \
            {default_wait_s} {max_wait_s} {max_lane_chars}";
        assert_eq!(
            describe(text, &odd_limits()),
            "7 42 3000 bytes 33 77 55 5 9 666"
        );
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size(8 * 1024), "8 KB");
        assert_eq!(size(1500), "1500 bytes");
    }

    #[test]
    fn respond_shows_text_or_the_formatted_error() {
        let fmt = formatter();
        let ok = respond(&fmt, Format::Text, Ok("fine".into()), |_| {
            panic!("nothing to log")
        });
        assert_eq!(shown(&ok), ("fine".to_string(), false));

        let refused = RuleError::invalid("no");
        let err = respond(&fmt, Format::Json, Err(refused.clone()), |_| {
            panic!("only internal errors are logged")
        });
        assert_eq!(shown(&err), (fmt.error(Format::Json, &refused), true));
    }

    #[test]
    fn respond_hands_only_internal_errors_to_the_log() {
        let fmt = formatter();
        let broken = RuleError::new(Code::Internal, "disk on fire");
        let mut logged = Vec::new();
        let result = respond(&fmt, Format::Text, Err(broken.clone()), |e| {
            logged.push(e.clone())
        });
        assert_eq!(logged, vec![broken.clone()]);
        assert_eq!(shown(&result), (fmt.error(Format::Text, &broken), true));
    }

    #[test]
    fn body_chars_is_at_least_one_and_none_when_not_asked() {
        assert_eq!(body_chars(None), None);
        assert_eq!(body_chars(Some(0)), Some(1));
        assert_eq!(body_chars(Some(1)), Some(1));
        assert_eq!(body_chars(Some(250)), Some(250));
    }

    #[test]
    fn position_takes_one_of_after_and_before() {
        assert_eq!(position(None, None), Ok(Position::Latest));
        assert_eq!(position(Some(5), None), Ok(Position::After(5)));
        assert_eq!(position(None, Some(9)), Ok(Position::Before(9)));
        assert_eq!(
            position(Some(5), Some(9)),
            Err(RuleError::invalid(
                "give `after_id` or `before_id`, not both"
            ))
        );
    }

    #[test]
    fn offset_may_reach_the_end_but_not_pass_it() {
        assert_eq!(offset(None, 0, "past"), Ok(0));
        assert_eq!(offset(Some(3), 3, "past"), Ok(3));
        assert_eq!(
            offset(Some(4), 3, "offset is past the end of the note"),
            Err(RuleError::invalid("offset is past the end of the note"))
        );
    }
}
