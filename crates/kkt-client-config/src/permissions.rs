//! Letting a client always use kkt: every tool of the MCP server named
//! `kkt` (the agent server; never another, such as an admin server added by
//! hand) and shell commands that start with `kkt watch`, without asking
//! each time. Each client keeps these rules in its own file and format.
//! Only kkt's rules are added; every other entry is left as it was, a rule
//! the person already has for the same thing (such as a deny) is kept, a
//! file is backed up before it changes, and allowing again adds nothing
//! twice.

use super::formats::{object, parse_json, parse_toml};
use super::{Client, Host, files};
use kkt_config as config;
use kkt_settings::Env;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use toml_edit::value;

/// The tools of kkt's agent server (`/mcp`). OpenCode needs them one by
/// one, since a pattern such as `kkt_*` also matches a server named
/// `kkt_admin`. `tests/contract.rs` checks this against the tools the
/// server offers.
pub const KKT_AGENT_TOOLS: [&str; 30] = [
    "activity",
    "add_to_channel",
    "create_channel",
    "forget",
    "get_message",
    "get_note",
    "hide_channel",
    "join_channel",
    "leave_channel",
    "list_agents",
    "list_channels",
    "list_lanes",
    "list_members",
    "list_memories",
    "read",
    "recall",
    "reclaim",
    "register",
    "remember",
    "remove_from_channel",
    "search_messages",
    "send",
    "set_description",
    "set_note",
    "set_purpose",
    "take_lane",
    "transfer_owner",
    "unregister",
    "wait_for_messages",
    "whoami",
];

/// The name setup gives kkt's agent server in every client.
const SERVER: &str = "kkt";

/// Claude Code's rules: `mcp__kkt` matches every tool of the server named
/// exactly `kkt`; `Bash(kkt watch *)` matches `kkt watch` with or without
/// more words.
const CLAUDE_RULES: [&str; 2] = ["mcp__kkt", "Bash(kkt watch *)"];

/// The Antigravity CLI's rules: every tool of the server `kkt`, and
/// commands that start with `kkt watch`.
const AGY_RULES: [&str; 2] = ["mcp(kkt/*)", "command(kkt watch)"];

/// Codex's rule for commands that start with the words `kkt watch`.
const CODEX_RULE: &str = "prefix_rule(pattern = [\"kkt\", \"watch\"], decision = \"allow\")";

/// The Codex setting, in the server's table, that approves all its tools.
const CODEX_APPROVAL: (&str, &str) = ("default_tools_approval_mode", "approve");

/// OpenCode's shell pattern for `kkt watch` with or without more words.
const OPENCODE_SHELL: &str = "kkt watch *";

impl Client {
    /// Whether this client has a setting that lets kkt's tools and
    /// `kkt watch` run without asking. Muse Code documents none.
    pub fn can_allow_kkt(self) -> bool {
        !matches!(self, Client::MuseCode)
    }
}

/// Lets `client` always use kkt's tools and `kkt watch`, in its default
/// files (following its own variables, as [`Client::file`] does). Returns
/// what it did, for people. For Codex, kkt must be connected first, since
/// the setting goes in its server entry. A client with no such setting
/// ([`Client::can_allow_kkt`]) is an error saying so.
pub fn allow_kkt(client: Client, env: &Env, host: &dyn Host) -> Result<String, String> {
    let source = client.source(env);
    let changes = match client {
        Client::ClaudeCode => {
            let dir = match &env.claude_config_dir {
                Some(dir) => dir.clone(),
                None => home(env)?.join(config::CLAUDE_HOME_DEFAULT),
            };
            vec![apply(
                host,
                &dir.join(config::CLAUDE_SETTINGS),
                source,
                |text| allow_list(text, &CLAUDE_RULES),
            )?]
        }
        Client::Codex => {
            let path = client.file(env, host)?;
            let rules = path
                .parent()
                .ok_or("the Codex config has no folder")?
                .join(config::CODEX_RULES_DIR)
                .join(config::CODEX_KKT_RULES);
            vec![
                apply(host, &path, source, codex_config)?,
                apply(host, &rules, source, |text| Ok(codex_rules(text)))?,
            ]
        }
        Client::OpenCode => {
            let path = client.file(env, host)?;
            vec![apply(host, &path, source, opencode)?]
        }
        Client::Agy => {
            let path = home(env)?.join(config::AGY_CLI_SETTINGS);
            vec![apply(host, &path, "HOME", |text| {
                allow_list(text, &AGY_RULES)
            })?]
        }
        Client::MuseCode => {
            return Err(format!(
                "{} has no setting that allows one MCP server's tools or one command, so it \
                 still asks before each",
                client.display_name()
            ));
        }
    };
    let changed: Vec<_> = changes.iter().filter(|(_, changed)| *changed).collect();
    let paths = |list: &[&(PathBuf, bool)]| {
        list.iter()
            .map(|(path, _)| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut out = if changed.is_empty() {
        let all: Vec<_> = changes.iter().collect();
        format!("Already allowed: {}", paths(&all))
    } else {
        format!("Allowed kkt's tools and `kkt watch`: {}", paths(&changed))
    };
    if client == Client::Agy {
        out.push_str(
            "\nThis is for the Antigravity CLI. The Antigravity app sets permissions in its own \
             settings screen.",
        );
    }
    Ok(out)
}

fn home(env: &Env) -> Result<&Path, String> {
    env.home
        .as_deref()
        .ok_or_else(|| "set HOME to find the client's config".to_string())
}

/// Reads `path`, passes its text (`None` when it does not exist) to
/// `change`, and writes what it returns, backing the old file up first.
/// `None` from `change` means nothing to change. Returns the path and
/// whether it changed. Errors name the path and `source`, the setting
/// that chose it.
fn apply(
    host: &dyn Host,
    path: &Path,
    source: &str,
    change: impl FnOnce(Option<&str>) -> Result<Option<String>, String>,
) -> Result<(PathBuf, bool), String> {
    let from = |error: String| format!("{}: {error} (from {source})", path.display());
    let original = files::read(host, path).map_err(|e| format!("{e} (from {source})"))?;
    let Some(updated) = change(original.as_deref()).map_err(from)? else {
        return Ok((path.to_path_buf(), false));
    };
    files::replace(host, path, original.as_deref(), &updated)
        .map_err(|e| format!("{e} (from {source})"))?;
    Ok((path.to_path_buf(), true))
}

/// `document` as the text kkt writes for a JSON file, or `None` if it is
/// `original` unchanged.
fn json_text(original: &Value, document: &Value) -> Result<Option<String>, String> {
    if original == document {
        return Ok(None);
    }
    serde_json::to_string_pretty(document)
        .map(|text| Some(format!("{text}\n")))
        .map_err(|_| "could not encode the settings".to_string())
}

fn json_or_empty(text: Option<&str>) -> Result<Value, String> {
    match text {
        Some(text) => parse_json(text),
        None => Ok(json!({})),
    }
}

/// Adds each of `rules` missing from `permissions.allow`, the list both
/// Claude Code and the Antigravity CLI keep.
fn allow_list(text: Option<&str>, rules: &[&str]) -> Result<Option<String>, String> {
    let original = json_or_empty(text)?;
    let mut document = original.clone();
    let permissions = object(&mut document)?
        .entry("permissions")
        .or_insert_with(|| json!({}));
    let allow = object(permissions)?
        .entry("allow")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("permissions.allow must be a list")?;
    for rule in rules {
        if !allow.iter().any(|r| r.as_str() == Some(rule)) {
            allow.push(json!(rule));
        }
    }
    json_text(&original, &document)
}

/// Sets Codex to approve every tool of the kkt server, in its entry, which
/// must exist: a server entry with only this setting would stop Codex
/// from starting. An approval mode the entry has already is kept.
fn codex_config(text: Option<&str>) -> Result<Option<String>, String> {
    let missing = || "Codex has no kkt server here; connect it first".to_string();
    let text = text.ok_or_else(missing)?;
    let mut document = parse_toml(text)
        .map_err(|e| format!("invalid Codex TOML; the file was not changed: {e}"))?;
    // Looked up through tables: indexing an item adds the key if missing.
    let entry = document
        .as_table_mut()
        .get_mut("mcp_servers")
        .and_then(|servers| servers.as_table_like_mut())
        .and_then(|servers| servers.get_mut(SERVER))
        .ok_or_else(missing)?
        .as_table_like_mut()
        .ok_or("mcp_servers.kkt must be a TOML table")?;
    let (name, approve) = CODEX_APPROVAL;
    if entry.contains_key(name) {
        return Ok(None);
    }
    entry.insert(name, value(approve));
    Ok(Some(document.to_string()))
}

/// Codex's rules file for kkt, with kkt's rule added unless a line holds
/// it already. Other lines stay as they are.
fn codex_rules(text: Option<&str>) -> Option<String> {
    let text = text.unwrap_or("");
    if text.lines().any(|line| line.trim() == CODEX_RULE) {
        return None;
    }
    let mut updated = text.to_string();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(CODEX_RULE);
    updated.push('\n');
    Some(updated)
}

/// OpenCode's rules. OpenCode 2 keeps them in `permissions`, a list of
/// `{action, resource, effect}`, and still reads the older `permission`
/// object, which it turns into the same list, placed before the
/// `permissions` entries. The last rule that matches wins. So kkt's rules go
/// at the end of `permissions` when the file has that list, and otherwise
/// into `permission`, which every version reads. A tool's action is
/// `<server>_<tool>`; the shell's is `shell`, called `bash` in
/// `permission`. A rule already there for the same action and resource,
/// whatever its effect, is kept, and none is added beside it.
fn opencode(text: Option<&str>) -> Result<Option<String>, String> {
    let original = json_or_empty(text)?;
    let mut document = original.clone();
    let root = object(&mut document)?;
    if let Some(list) = root.get_mut("permissions") {
        let list = list
            .as_array_mut()
            .ok_or("permissions must be a list of rules")?;
        let shell = ("shell".to_string(), OPENCODE_SHELL.to_string());
        let tools = KKT_AGENT_TOOLS
            .iter()
            .map(|tool| (format!("{SERVER}_{tool}"), "*".to_string()));
        for (action, resource) in std::iter::once(shell).chain(tools) {
            let present = list.iter().any(|rule| {
                rule.get("action").and_then(Value::as_str) == Some(action.as_str())
                    && rule.get("resource").and_then(Value::as_str) == Some(resource.as_str())
            });
            if !present {
                list.push(json!({"action": action, "resource": resource, "effect": "allow"}));
            }
        }
        return json_text(&original, &document);
    }
    let permission = root.entry("permission").or_insert_with(|| json!({}));
    // A single word applies to everything; as an object it is `*`.
    if let Some(everything) = permission.as_str() {
        *permission = json!({"*": everything});
    }
    let permission = object(permission)?;
    let bash = permission.entry("bash").or_insert_with(|| json!({}));
    if let Some(every_command) = bash.as_str() {
        *bash = json!({"*": every_command});
    }
    object(bash)
        .map_err(|_| "permission.bash must be a word or an object".to_string())?
        .entry(OPENCODE_SHELL)
        .or_insert_with(|| json!("allow"));
    for tool in KKT_AGENT_TOOLS {
        permission
            .entry(format!("{SERVER}_{tool}"))
            .or_insert_with(|| json!("allow"));
    }
    json_text(&original, &document)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Value {
        serde_json::from_str(text).expect("JSON")
    }

    #[test]
    fn claude_code_gains_both_rules_beside_its_own_and_only_once() {
        let before = r#"{"model":"opus","permissions":{"allow":["Bash(git status)"],"deny":["Read(.env)"]}}"#;
        let after = allow_list(Some(before), &CLAUDE_RULES)
            .expect("merge")
            .expect("changed");
        assert_eq!(
            parsed(&after),
            json!({
                "model": "opus",
                "permissions": {
                    "allow": ["Bash(git status)", "mcp__kkt", "Bash(kkt watch *)"],
                    "deny": ["Read(.env)"]
                }
            })
        );
        assert_eq!(allow_list(Some(&after), &CLAUDE_RULES), Ok(None));
    }

    #[test]
    fn a_missing_settings_file_is_made_with_only_the_rules() {
        let made = allow_list(None, &AGY_RULES).expect("merge").expect("made");
        assert_eq!(
            parsed(&made),
            json!({"permissions": {"allow": ["mcp(kkt/*)", "command(kkt watch)"]}})
        );
    }

    #[test]
    fn a_settings_file_of_the_wrong_shape_is_refused() {
        for text in [
            r#"{"permissions":{"allow":"mcp__kkt"}}"#,
            r#"{"permissions":[]}"#,
            "[]",
            "{",
        ] {
            assert!(allow_list(Some(text), &CLAUDE_RULES).is_err(), "{text}");
        }
    }

    #[test]
    fn codex_approves_the_kkt_server_only_and_keeps_the_rest() {
        let before = "model = \"o3\"\n\n[mcp_servers.kkt]\nurl = \"http://a/mcp\"\n\n\
                      [mcp_servers.kkt-admin]\nurl = \"http://a/mcp-admin\"\n";
        let after = codex_config(Some(before)).expect("merge").expect("changed");
        assert_eq!(
            after,
            "model = \"o3\"\n\n[mcp_servers.kkt]\nurl = \"http://a/mcp\"\n\
             default_tools_approval_mode = \"approve\"\n\n\
             [mcp_servers.kkt-admin]\nurl = \"http://a/mcp-admin\"\n"
        );
        assert_eq!(codex_config(Some(&after)), Ok(None));
    }

    #[test]
    fn codex_without_a_kkt_server_is_refused_rather_than_given_half_an_entry() {
        for text in [
            None,
            Some("model = \"o3\"\n"),
            Some("[mcp_servers.other]\nurl = \"x\"\n"),
        ] {
            let error = codex_config(text).expect_err("no kkt server");
            assert!(error.contains("connect it first"), "{error}");
        }
    }

    #[test]
    fn the_codex_rule_is_added_once_after_any_other_rules() {
        let made = codex_rules(None).expect("made");
        assert_eq!(made, format!("{CODEX_RULE}\n"));
        assert_eq!(codex_rules(Some(&made)), None);
        let other = "prefix_rule(pattern = [\"git\", \"status\"], decision = \"allow\")";
        assert_eq!(
            codex_rules(Some(other)),
            Some(format!("{other}\n{CODEX_RULE}\n"))
        );
    }

    fn opencode_tools() -> Vec<String> {
        KKT_AGENT_TOOLS.iter().map(|t| format!("kkt_{t}")).collect()
    }

    #[test]
    fn opencode_rules_go_in_the_older_object_every_version_reads() {
        let before =
            r#"{"mcp":{"kkt":{"type":"remote"}},"permission":{"edit":"ask","bash":"ask"}}"#;
        let after = opencode(Some(before)).expect("merge").expect("changed");
        let document = parsed(&after);
        let permission = document["permission"].as_object().expect("object");
        assert_eq!(permission["edit"], "ask");
        assert_eq!(
            permission["bash"],
            json!({"*": "ask", "kkt watch *": "allow"})
        );
        for tool in opencode_tools() {
            assert_eq!(permission[&tool], "allow", "{tool}");
        }
        // Only kkt's own tools, never a pattern another server could match.
        assert!(permission.keys().all(|k| !k.contains('*') || k == "*"));
        assert_eq!(document["mcp"], json!({"kkt": {"type": "remote"}}));
        assert!(document.get("permissions").is_none());
        assert_eq!(opencode(Some(&after)), Ok(None));
    }

    #[test]
    fn a_single_opencode_permission_word_keeps_its_meaning() {
        let after = opencode(Some(r#"{"permission":"ask"}"#))
            .expect("merge")
            .expect("changed");
        let document = parsed(&after);
        assert_eq!(document["permission"]["*"], "ask");
        assert_eq!(
            document["permission"]["bash"],
            json!({"kkt watch *": "allow"})
        );
    }

    #[test]
    fn opencode_rules_go_at_the_end_of_a_permissions_list_so_they_win() {
        let before = r#"{"permissions":[{"action":"*","resource":"*","effect":"ask"}]}"#;
        let after = opencode(Some(before)).expect("merge").expect("changed");
        let document = parsed(&after);
        let list = document["permissions"].as_array().expect("list");
        assert_eq!(
            list[0],
            json!({"action": "*", "resource": "*", "effect": "ask"})
        );
        assert_eq!(
            list[1],
            json!({"action": "shell", "resource": "kkt watch *", "effect": "allow"})
        );
        let actions: Vec<_> = list[2..]
            .iter()
            .map(|rule| rule["action"].as_str().expect("action").to_string())
            .collect();
        assert_eq!(actions, opencode_tools());
        assert!(document.get("permission").is_none());
        assert_eq!(opencode(Some(&after)), Ok(None));
    }

    #[test]
    fn opencode_permissions_of_the_wrong_shape_are_refused() {
        for text in [
            r#"{"permissions":{"bash":"ask"}}"#,
            r#"{"permission":{"bash":3}}"#,
        ] {
            assert!(opencode(Some(text)).is_err(), "{text}");
        }
    }

    #[test]
    fn a_rule_the_person_already_has_for_kkt_is_kept() {
        let object = r#"{"permission":{"kkt_send":"deny","bash":{"kkt watch *":"ask"}}}"#;
        let after = parsed(&opencode(Some(object)).expect("merge").expect("changed"));
        assert_eq!(after["permission"]["kkt_send"], "deny");
        assert_eq!(after["permission"]["bash"]["kkt watch *"], "ask");
        assert_eq!(after["permission"]["kkt_read"], "allow");

        let list = r#"{"permissions":[{"action":"kkt_send","resource":"*","effect":"deny"}]}"#;
        let after = parsed(&opencode(Some(list)).expect("merge").expect("changed"));
        let send: Vec<_> = after["permissions"]
            .as_array()
            .expect("list")
            .iter()
            .filter(|rule| rule["action"] == "kkt_send")
            .collect();
        assert_eq!(
            send,
            [&json!({"action": "kkt_send", "resource": "*", "effect": "deny"})]
        );

        let codex = "[mcp_servers.kkt]\nurl = \"http://a/mcp\"\n\
                     default_tools_approval_mode = \"prompt\"\n";
        assert_eq!(codex_config(Some(codex)), Ok(None));
    }

    #[test]
    fn only_muse_code_has_no_setting() {
        let unable: Vec<_> = Client::ALL
            .into_iter()
            .filter(|c| !c.can_allow_kkt())
            .collect();
        assert_eq!(unable, [Client::MuseCode]);
    }
}
