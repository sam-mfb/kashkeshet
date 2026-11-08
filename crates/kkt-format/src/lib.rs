//! Turns rule results into small responses: plain-text lines by default, JSON
//! on request. Lists and message previews fit the response size cap. Single-item
//! get_message, registration and whoami replies keep their full contents.
//! When something is left out, the response says how to get it.
//!
//! `fit` holds the size-cap machinery every response shares; the other files
//! hold the output for one area each.

mod admin;
mod agents;
mod channels;
mod fit;
mod lanes;
mod memory;
mod messages;
mod operator;
pub use operator::NoticeWording;

#[cfg(test)]
mod tests;

use kkt_clock::Clock;
use kkt_rules::RuleError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Compact lines of text. The default.
    #[default]
    Text,
    /// A JSON object.
    Json,
}

#[derive(Clone)]
pub struct Formatter {
    max_bytes: usize,
    clock: Arc<dyn Clock>,
    server_version: &'static str,
}

impl Formatter {
    /// `server_version` is the kkt version agents are told at `register`
    /// and `whoami`.
    pub fn new(
        max_response_bytes: usize,
        clock: Arc<dyn Clock>,
        server_version: &'static str,
    ) -> Formatter {
        Formatter {
            max_bytes: max_response_bytes,
            clock,
            server_version,
        }
    }

    pub fn error(&self, f: Format, e: &RuleError) -> String {
        match f {
            Format::Text => format!("error: {}: {}", e.code.as_str(), e.message),
            Format::Json => {
                json!({ "error": { "code": e.code, "message": e.message } }).to_string()
            }
        }
    }

    pub fn ok(&self, f: Format) -> String {
        match f {
            Format::Text => "ok".to_string(),
            Format::Json => json!({ "ok": true }).to_string(),
        }
    }
}
