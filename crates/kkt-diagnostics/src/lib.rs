//! Read-only connection diagnostics. HTTP and JSON types stay in this part.
//! The checks and their names are here; reading key files and probing a
//! server, which touch the machine, are in `probe.rs`.

#[cfg(feature = "real")]
mod probe;

#[cfg(feature = "real")]
pub use probe::{inspect, inspect_address, inspect_admin_key, inspect_web_key, inspect_with_key};
use std::net::SocketAddr;
use url::Url;

#[derive(Debug)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

impl Check {
    pub fn ok(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            ok: true,
            detail: detail.into(),
            fix: None,
        }
    }

    pub fn problem(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
        Check {
            name,
            ok: false,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}

/// `text` as one shell word, for fix text the operator copies into a shell
/// (not for running a process here). Single-quoted, with each `'` written as
/// `'\''`.
pub fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Validate an HTTP MCP endpoint without allowing secrets in its URL. A bare
/// origin means /mcp; a supplied path is left intact for forwarding setups.
pub fn endpoint(bind: SocketAddr, given: Option<&str>) -> Result<String, String> {
    let default = format!("http://{bind}/mcp");
    let text = given.unwrap_or(&default);
    if text.chars().any(char::is_control) {
        return Err("URL contains a control character".into());
    }
    let mut url =
        Url::parse(text).map_err(|_| "URL must be an absolute http:// or https:// address")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("URL must be an absolute http:// or https:// address".into());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("put credentials in the key file, not in the URL; query strings and fragments are not supported".into());
    }
    if url.path() == "/" {
        url.set_path("/mcp");
    }
    Ok(url.to_string())
}

/// The name of the check that the server answered at all.
pub const SERVER_CHECK: &str = "server";

/// Whether `checks` found the server not answering at all, as opposed to
/// answering with a problem.
pub fn unreachable(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.name == SERVER_CHECK && !c.ok)
}

/// The name of the check that the server accepted the key.
pub const KEY_CHECK: &str = "key accepted";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_word_quotes_text_for_fix_text() {
        assert_eq!(shell_word(""), "''");
        assert_eq!(shell_word("/home/sam/.kkt/key"), "'/home/sam/.kkt/key'");
        assert_eq!(shell_word("/My Keys/$HOME"), "'/My Keys/$HOME'");
        assert_eq!(shell_word("/sam's/key"), "'/sam'\\''s/key'");
        assert_eq!(shell_word("''"), "''\\'''\\'''");
    }
}
