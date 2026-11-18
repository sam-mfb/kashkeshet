//! Vendor formats stay inside the client configuration part.

use super::error::{ClientConfigError, Encoded, FileAction, Shape, Text, TomlProblem};
use super::{Client, Host, Setup, helper};
use kkt_toml::{Document, EditError, IfMissing};
use serde_json::{Map, Value, json};
use std::path::Path;

pub(super) fn header_json(key: &str) -> Result<String, ClientConfigError> {
    serde_json::to_string(&json!({"Authorization": format!("Bearer {key}")}))
        .map_err(|_| ClientConfigError::Encode(Encoded::Header))
}

pub(super) fn object(value: &mut Value) -> Result<&mut Map<String, Value>, ClientConfigError> {
    value
        .as_object_mut()
        .ok_or(ClientConfigError::Malformed(Shape::NotObjects))
}

pub(super) fn parse_json(text: &str) -> Result<Value, ClientConfigError> {
    serde_json::from_str(text).map_err(|e| {
        ClientConfigError::Malformed(Shape::Json {
            line: e.line(),
            column: e.column(),
        })
    })
}

/// Where a copied key comes from: a placeholder in a preview, which never
/// holds the key, or the key file.
#[derive(Clone, Copy)]
enum KeyFrom<'a> {
    Preview,
    File(&'a dyn Host),
}

fn key(setup: &Setup<'_>, from: KeyFrom<'_>) -> Result<Option<String>, ClientConfigError> {
    let Some(path) = setup.key_file else {
        return Ok(None);
    };
    let host = match from {
        KeyFrom::Preview => {
            return Ok(Some(format!(
                "<key read from {} by --write>",
                path.display()
            )));
        }
        KeyFrom::File(host) => host,
    };
    let text = host
        .read(path)
        .map_err(|e| ClientConfigError::file(FileAction::Read, path, e))?;
    let key = text.trim();
    kkt_keys::check_access_key(key)?;
    Ok(Some(key.to_string()))
}

fn json_entry(
    client: Client,
    setup: &Setup<'_>,
    from: KeyFrom<'_>,
    original: Option<Value>,
) -> Result<Value, ClientConfigError> {
    let mut entry = original.unwrap_or_else(|| json!({}));
    let fields = object(&mut entry)?;
    // Replace transport/credential fields owned by connect; retain filters,
    // timeouts and all unrelated client settings, including other headers.
    for name in [
        "command",
        "args",
        "env",
        "url",
        "serverUrl",
        "headersHelper",
        "type",
        "transport",
    ] {
        fields.remove(name);
    }
    let mut headers = fields.remove("headers").unwrap_or_else(|| json!({}));
    object(&mut headers)?.retain(|name, _| !name.eq_ignore_ascii_case("authorization"));
    match client {
        Client::ClaudeCode => {
            fields.insert("type".into(), "http".into());
            fields.insert("url".into(), setup.url.into());
            if let Some(command) = helper(setup)? {
                fields.insert("headersHelper".into(), command.into());
            }
        }
        Client::Agy => {
            fields.insert("serverUrl".into(), setup.url.into());
            if let Some(key) = key(setup, from)? {
                object(&mut headers)?
                    .insert("Authorization".into(), format!("Bearer {key}").into());
            }
        }
        Client::MuseCode => {
            fields.insert("transport".into(), "streamable_http".into());
            fields.insert("url".into(), setup.url.into());
            // A required server that is down aborts every Muse run, so
            // kkt's entry must never hold the run hostage.
            fields.insert("mode".into(), "optional".into());
            if let Some(key) = key(setup, from)? {
                object(&mut headers)?
                    .insert("Authorization".into(), format!("Bearer {key}").into());
            }
        }
        Client::OpenCode => {
            fields.remove("environment");
            fields.remove("cwd");
            fields.insert("type".into(), "remote".into());
            fields.insert("url".into(), setup.url.into());
            fields.insert("oauth".into(), false.into());
            if let Some(path) = setup.key_file {
                let path = path
                    .to_str()
                    .ok_or(ClientConfigError::NotText(Text::KeyFilePath))?;
                if path.contains(['{', '}', '"', '\\']) || path.chars().any(char::is_control) {
                    return Err(ClientConfigError::Malformed(Shape::KeyPathCharacters));
                }
                object(&mut headers)?.insert(
                    "Authorization".into(),
                    format!("Bearer {{file:{path}}}").into(),
                );
            }
        }
        Client::Codex => return Err(ClientConfigError::Malformed(Shape::CodexIsToml)),
    }
    if !object(&mut headers)?.is_empty() {
        fields.insert("headers".into(), headers);
    }
    Ok(entry)
}

fn json_document(
    client: Client,
    setup: &Setup<'_>,
    text: Option<&str>,
    from: KeyFrom<'_>,
) -> Result<(Value, Value), ClientConfigError> {
    let mut document = match text {
        Some(text) => parse_json(text)?,
        None => json!({}),
    };
    let original = document.clone();
    if client == Client::MuseCode {
        // Muse Code refuses the whole file without this, so a file we
        // create or extend has to carry it. A version already there is
        // left alone: Muse reports it if it cannot use it.
        object(&mut document)?
            .entry("schema_version")
            .or_insert_with(|| json!(1));
    }
    let group = if client == Client::OpenCode {
        "mcp"
    } else {
        "mcpServers"
    };
    let servers = object(&mut document)?
        .entry(group)
        .or_insert_with(|| json!({}));
    let servers = object(servers)?;
    let entry = json_entry(client, setup, from, servers.remove("kkt"))?;
    servers.insert("kkt".into(), entry);
    Ok((original, document))
}

/// Parse Codex TOML. The error is the parser's reason and where it found the
/// problem, like `parse_json`'s. It never quotes the file, which may hold a key.
pub(super) fn parse_toml(text: &str) -> Result<Document, TomlProblem> {
    Document::parse(text).map_err(TomlProblem)
}

fn toml_document(setup: &Setup<'_>, text: Option<&str>) -> Result<String, ClientConfigError> {
    let mut document = parse_toml(text.unwrap_or(""))
        .map_err(|e| ClientConfigError::Malformed(Shape::TomlUnchanged(e)))?;
    let mut entry = document
        .table_mut(&["mcp_servers", "kkt"], IfMissing::Add)
        .map_err(|e| {
            ClientConfigError::Malformed(match e {
                EditError::NotATable { path } if path.len() == 1 => Shape::McpServersNotTable,
                _ => Shape::KktNotTable,
            })
        })?;
    for name in [
        "command",
        "args",
        "env",
        "bearer_token_env_var",
        "http_headers_helper",
    ] {
        entry.remove(name);
    }
    for name in ["http_headers", "env_http_headers"] {
        let mut headers = match entry.table_mut(&[name], IfMissing::Refuse) {
            Ok(headers) => headers,
            Err(EditError::Missing { .. }) => continue,
            Err(_) => return Err(ClientConfigError::Malformed(Shape::HeadersNotTable)),
        };
        for name in headers.keys() {
            if name.eq_ignore_ascii_case("authorization") {
                headers.remove(&name);
            }
        }
    }
    entry.set_text("url", setup.url);
    if let Some(command) = helper(setup)? {
        entry.set_text("http_headers_helper", &command);
    }
    Ok(document.text())
}

/// The change `--write` would make. For Claude Code, `claude_config_dir` is
/// the `CLAUDE_CONFIG_DIR` the printed command sets, or `None` to unset it.
pub(super) fn preview(
    client: Client,
    setup: &Setup<'_>,
    claude_config_dir: Option<&Path>,
) -> Result<String, ClientConfigError> {
    if client == Client::Codex {
        return toml_document(setup, None);
    }
    let (_, doc) = json_document(client, setup, None, KeyFrom::Preview)?;
    if client == Client::ClaudeCode {
        let entry = claude_entry(
            &serde_json::to_string(&doc).map_err(|_| ClientConfigError::Encode(Encoded::Config))?,
        )?;
        let environment = match claude_config_dir {
            Some(dir) => format!(
                "CLAUDE_CONFIG_DIR={}",
                super::shell_word(dir.to_str().ok_or_else(|| {
                    ClientConfigError::ClaudeDirNotText {
                        dir: dir.to_path_buf(),
                        setting: setup.source.to_string(),
                    }
                })?)
            ),
            None => "env -u CLAUDE_CONFIG_DIR".to_string(),
        };
        return Ok(format!(
            "{environment} claude mcp add-json --scope user kkt {}",
            super::shell_word(&entry)
        ));
    }
    serde_json::to_string_pretty(&doc).map_err(|_| ClientConfigError::Encode(Encoded::Config))
}

/// `text`, the client's config if it has one, with kkt's entry set; `None`
/// if it has it already. A key the client copies is read through `host`.
pub(super) fn merge(
    client: Client,
    setup: &Setup<'_>,
    text: Option<&str>,
    host: &dyn Host,
) -> Result<Option<String>, ClientConfigError> {
    if client == Client::Codex {
        let updated = toml_document(setup, text)?;
        return Ok(if text == Some(updated.as_str()) {
            None
        } else {
            Some(updated)
        });
    }
    let (original, updated) = json_document(client, setup, text, KeyFrom::File(host))?;
    if original == updated {
        return Ok(None);
    }
    super::order::pretty(text, &updated).map(Some)
}

/// Whether the config `text` of `client` has a kkt entry.
pub(super) fn has_kkt_entry(client: Client, text: &str) -> Result<bool, ClientConfigError> {
    if client == Client::Codex {
        let document =
            parse_toml(text).map_err(|e| ClientConfigError::Malformed(Shape::Toml(e)))?;
        return Ok(document.find(&["mcp_servers", "kkt"]).is_some());
    }
    let group = if client == Client::OpenCode {
        "mcp"
    } else {
        "mcpServers"
    };
    Ok(parse_json(text)?
        .get(group)
        .and_then(|servers| servers.get("kkt"))
        .is_some())
}

pub(super) fn has_claude_entry(text: &str) -> Result<bool, ClientConfigError> {
    Ok(parse_json(text)?
        .get("mcpServers")
        .and_then(|s| s.get("kkt"))
        .is_some())
}

pub(super) fn claude_entry(text: &str) -> Result<String, ClientConfigError> {
    let document = parse_json(text)?;
    let entry = document
        .get("mcpServers")
        .and_then(|s| s.get("kkt"))
        .ok_or(ClientConfigError::Malformed(Shape::NoKktEntry))?;
    serde_json::to_string(entry).map_err(|_| ClientConfigError::Encode(Encoded::ClaudeConfig))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::Setup;
    use kkt_machine::Env;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    const SECRET: &str = "0123456789abcdef0123456789abcdef";
    /// The third line opens a string it never closes.
    const MALFORMED_TOML: &str =
        "[mcp_servers.kkt]\nurl = \"http://a/mcp\"\ntoken = '0123456789abcdef0123456789abcdef\n";

    /// The parser's own reason, as kkt passes it on.
    fn parser_reason() -> String {
        Document::parse(MALFORMED_TOML)
            .expect_err("malformed")
            .reason()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The cause after today's text: the parser's reason and where, never the file's text.
    fn assert_cause(error: &str, today: &str) {
        let cause = error
            .strip_prefix(today)
            .and_then(|rest| rest.strip_prefix(": "))
            .unwrap_or_else(|| panic!("{error}"));
        let reason = parser_reason();
        assert!(!reason.is_empty());
        assert!(cause.starts_with(&reason), "{error}");
        assert!(cause.contains(" at line 3, column "), "{error}");
        assert!(!error.contains(SECRET), "{error}");
    }

    #[test]
    fn a_toml_error_says_where_and_counts_columns_in_characters() {
        // The second `=` on line 2 is the 9th character but the 10th byte.
        let error = parse_toml("a = 1\n'\u{e9}' = 2 = 3\n")
            .expect_err("malformed")
            .to_string();
        assert!(error.ends_with(" at line 2, column 9"), "{error}");
    }

    #[test]
    fn merging_into_malformed_codex_toml_names_the_parsers_reason() {
        let env = Env {
            home: None,
            named: Default::default(),
            xdg_config_home: None,
            xdg_data_home: None,
        };
        let setup = Setup {
            env: &env,
            url: "http://localhost:7420/mcp",
            executable: Path::new("/opt/kkt"),
            key_file: None,
            client_config: Path::new("/home/sam/.codex/config.toml"),
            source: "--client-config",
        };
        let host = crate::fake::FakeHost::default();
        let error = merge(Client::Codex, &setup, Some(MALFORMED_TOML), &host)
            .expect_err("malformed")
            .to_string();
        assert_cause(&error, "invalid Codex TOML; the file was not changed");
    }

    #[test]
    fn checking_malformed_codex_toml_names_the_parsers_reason() {
        let error = has_kkt_entry(Client::Codex, MALFORMED_TOML)
            .expect_err("malformed")
            .to_string();
        assert_cause(&error, "invalid Codex TOML");
    }

    #[test]
    fn claude_preview_reports_a_non_utf8_profile_directory() {
        let env = Env {
            home: None,
            named: Default::default(),
            xdg_config_home: None,
            xdg_data_home: None,
        };
        let dir = PathBuf::from(OsString::from_vec(b"/profiles/\xff".to_vec()));
        let file = dir.join(".claude.json");
        let setup = Setup {
            env: &env,
            url: "http://localhost:7420/mcp",
            executable: Path::new("/opt/kkt"),
            key_file: None,
            client_config: &file,
            source: "--client-config",
        };
        let error = preview(Client::ClaudeCode, &setup, Some(&dir))
            .expect_err("non-UTF-8 directory")
            .to_string();
        assert!(
            error.contains("is not UTF-8 (from --client-config)"),
            "{error}"
        );
    }
}
