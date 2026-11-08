//! Unit tests, against [`FakeHost`]: no real files and no `claude`.

use super::fake::{FakeHost, File};
use super::*;

const HOME: &str = "/home/sam";
const KEY: &str = "0123456789abcdef0123456789abcdef";

fn env() -> Env {
    Env {
        home: Some(PathBuf::from(HOME)),
        xdg_config_home: None,
        xdg_data_home: None,
        claude_config_dir: None,
        codex_home: None,
    }
}

fn setup<'a>(env: &'a Env, client_config: &'a Path, key_file: Option<&'a Path>) -> Setup<'a> {
    Setup {
        env,
        url: "http://localhost:7420/mcp",
        executable: Path::new("/opt/kkt"),
        key_file,
        client_config,
        source: "HOME",
    }
}

fn private(text: &str) -> File {
    File {
        text: text.into(),
        private: true,
    }
}

/// `path` with `suffix` added to its last part.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

#[test]
fn shell_word_quotes_text_for_helper_commands() {
    assert_eq!(shell_word(""), "''");
    assert_eq!(shell_word("/usr/local/bin/kkt"), "'/usr/local/bin/kkt'");
    assert_eq!(shell_word("/My Keys/$HOME"), "'/My Keys/$HOME'");
    assert_eq!(shell_word("/sam's/key"), r"'/sam'\''s/key'");
    assert_eq!(shell_word("''"), r"''\'''\'''");
}

#[test]
fn names_read_back() {
    for client in Client::ALL {
        assert_eq!(Client::from_name(client.name()), Some(client));
    }
    assert_eq!(Client::from_name("cursor"), None);
}

#[test]
fn a_client_is_connected_when_its_config_has_a_kkt_entry() {
    let host = FakeHost::default();
    let env = env();
    let write = |client: Client, text: &str| {
        host.put(client.file(&env, &host).expect("path"), text);
    };
    // No config at all: not connected.
    for client in Client::ALL {
        assert_eq!(
            client.connected(&env, &host),
            Ok(false),
            "{}",
            client.name()
        );
    }
    write(Client::Codex, "[mcp_servers.kkt]\nurl = \"http://a/mcp\"\n");
    write(Client::ClaudeCode, r#"{"mcpServers": {"other": {}}}"#);
    write(Client::OpenCode, r#"{"mcp": {"kkt": {"type": "remote"}}}"#);
    write(Client::MuseCode, r#"{"mcpServers": {"kkt": {}}}"#);
    write(Client::Agy, "not json");
    assert_eq!(Client::Codex.connected(&env, &host), Ok(true));
    assert_eq!(Client::ClaudeCode.connected(&env, &host), Ok(false));
    assert_eq!(Client::OpenCode.connected(&env, &host), Ok(true));
    assert_eq!(Client::MuseCode.connected(&env, &host), Ok(true));
    let error = Client::Agy.connected(&env, &host).expect_err("bad JSON");
    assert!(error.starts_with("Antigravity: "), "{error}");
    assert!(error.contains("invalid client JSON"), "{error}");
}

#[test]
fn a_malformed_codex_config_names_the_parsers_reason() {
    let host = FakeHost::default();
    let env = env();
    let path = Client::Codex.file(&env, &host).expect("path");
    let text = format!("[mcp_servers.kkt]\ntoken = '{KEY}\n");
    host.put(&path, &text);
    let error = codex_password_store_hint(&env, &host).expect_err("malformed");
    let cause = error
        .strip_prefix(&format!("invalid Codex TOML in {}: ", path.display()))
        .unwrap_or_else(|| panic!("{error}"));
    let reason = text
        .parse::<toml_edit::DocumentMut>()
        .expect_err("malformed");
    let first = reason.message().lines().next().expect("reason").trim();
    assert!(!first.is_empty());
    assert!(cause.starts_with(first), "{error}");
    assert!(cause.contains(" at line 2, column "), "{error}");
    assert!(!error.contains(KEY), "{error}");
}

#[test]
fn a_client_is_found_by_its_program_or_its_config_file() {
    let host = FakeHost::default();
    let env = env();
    host.put(
        Path::new(HOME)
            .join(config::CODEX_HOME_DEFAULT)
            .join(config::CODEX_CONFIG),
        "",
    );
    let on_path = |program: &str| program == config::CLAUDE_PROGRAM;
    let found: Vec<Client> = Client::ALL
        .into_iter()
        .filter(|c| c.installed(&env, &on_path, &host))
        .collect();
    assert_eq!(found, [Client::ClaudeCode, Client::Codex]);
}

#[test]
fn antigravity_uses_its_legacy_config_only_when_the_new_one_is_missing() {
    let host = FakeHost::default();
    let env = env();
    let new = Path::new(HOME).join(config::AGY_CONFIG);
    let legacy = Path::new(HOME).join(config::AGY_LEGACY_CONFIG);
    assert_eq!(Client::Agy.file(&env, &host), Ok(new.clone()));
    host.put(&legacy, "{}");
    assert_eq!(Client::Agy.file(&env, &host), Ok(legacy));
    host.put(&new, "{}");
    assert_eq!(Client::Agy.file(&env, &host), Ok(new));
}

#[test]
fn writing_backs_up_the_old_config_under_a_free_name_and_replaces_it_whole() {
    let host = FakeHost::default();
    let env = env();
    let path = Client::Codex.file(&env, &host).expect("path");
    let old = "model = \"o3\"\n";
    host.put(&path, old);
    host.put(beside(&path, ".bak"), "an older backup");

    let out = configure(Client::Codex, &setup(&env, &path, None), true, &host).expect("write");
    assert_eq!(out, format!("Configured: {}", path.display()));
    let written = host.file(&path).expect("config").text;
    assert!(written.starts_with(old), "{written}");
    assert!(written.contains("[mcp_servers.kkt]"), "{written}");
    // The taken name is kept; the old config goes to the next one, readable
    // only by its owner.
    assert_eq!(
        host.file(beside(&path, ".bak")).expect("kept").text,
        "an older backup"
    );
    assert_eq!(host.file(beside(&path, ".bak.1")), Some(private(old)));
    // No temporary file is left.
    assert_eq!(
        host.paths(),
        [path.clone(), beside(&path, ".bak"), beside(&path, ".bak.1")]
    );

    let again = configure(Client::Codex, &setup(&env, &path, None), true, &host).expect("again");
    assert_eq!(again, format!("Already configured: {}", path.display()));
    assert!(host.file(beside(&path, ".bak.2")).is_none());
}

#[test]
fn a_failed_replacement_removes_its_temporary_file_and_keeps_the_config() {
    let host = FakeHost::default();
    let env = env();
    let path = Client::Codex.file(&env, &host).expect("path");
    let old = "model = \"o3\"\n";
    host.put(&path, old);
    host.rename_fails.set(true);
    let error =
        configure(Client::Codex, &setup(&env, &path, None), true, &host).expect_err("refused");
    assert_eq!(
        error,
        format!(
            "could not write {}: rename refused (from HOME)",
            path.display()
        )
    );
    assert_eq!(host.file(&path).expect("kept").text, old);
    assert!(host.file(beside(&path, ".kkt-7")).is_none());
}

#[test]
fn a_client_that_copies_the_key_reads_it_and_keeps_its_config_private() {
    let host = FakeHost::default();
    let env = env();
    let path = Client::MuseCode.file(&env, &host).expect("path");
    let key_file = Path::new("/keys/kkt.key");
    host.put(key_file, &format!("{KEY}\n"));
    let muse = setup(&env, &path, Some(key_file));

    let preview = configure(Client::MuseCode, &muse, false, &host).expect("preview");
    assert!(!preview.contains(KEY), "{preview}");
    assert!(host.file(&path).is_none(), "a preview writes nothing");

    configure(Client::MuseCode, &muse, true, &host).expect("write");
    let written = host.file(&path).expect("config");
    assert!(written.private);
    assert!(written.text.contains(KEY), "{}", written.text);

    // Already configured, but made readable by others: made private again.
    host.files
        .borrow_mut()
        .get_mut(&path)
        .expect("config")
        .private = false;
    let again = configure(Client::MuseCode, &muse, true, &host).expect("again");
    assert!(again.starts_with("Already configured"), "{again}");
    assert!(host.file(&path).expect("config").private);
}

#[test]
fn claude_code_is_configured_through_its_command_and_restored_if_it_fails() {
    let host = FakeHost::default();
    let env = env();
    let path = Client::ClaudeCode.file(&env, &host).expect("path");
    let old = r#"{"mcpServers": {"kkt": {"type": "http", "url": "http://old/mcp"}}}"#;
    host.put(&path, old);

    host.claude_fails.set(true);
    let error =
        configure(Client::ClaudeCode, &setup(&env, &path, None), true, &host).expect_err("fails");
    assert_eq!(error, "claude mcp failed");
    assert_eq!(host.file(&path), Some(private(old)), "restored whole");

    host.claude_fails.set(false);
    host.claude_runs.borrow_mut().clear();
    configure(Client::ClaudeCode, &setup(&env, &path, None), true, &host).expect("write");
    let runs = host.claude_runs.borrow();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].0, ["mcp", "remove", "--scope", "user", "kkt"]);
    assert_eq!(
        runs[1].0[..5],
        ["mcp", "add-json", "--scope", "user", "kkt"]
    );
    assert!(
        runs[1].0[5].contains("http://localhost:7420/mcp"),
        "{}",
        runs[1].0[5]
    );
    // The default config under HOME: CLAUDE_CONFIG_DIR is unset.
    assert!(runs.iter().all(|(_, dir)| dir.is_none()));
}

#[test]
fn a_new_claude_code_config_is_removed_if_its_command_fails() {
    let host = FakeHost::default();
    let env = env();
    let dir = Path::new("/profiles/work");
    let path = dir.join(config::CLAUDE_CONFIG);
    host.create_dir_all(dir).expect("dir");
    // The command writes the file, then fails.
    *host.claude_writes.borrow_mut() = Some(path.clone());
    host.claude_fails.set(true);
    let error =
        configure(Client::ClaudeCode, &setup(&env, &path, None), true, &host).expect_err("fails");
    assert_eq!(error, "claude mcp failed");
    assert!(host.file(&path).is_none());
    // Nothing to remove first, and a profile outside HOME is passed as
    // CLAUDE_CONFIG_DIR.
    let runs = host.claude_runs.borrow();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].1.as_deref(), Some(dir));
}

#[test]
fn allowing_kkt_writes_each_clients_own_permission_file() {
    let host = FakeHost::default();
    let env = env();
    host.put(
        Client::Codex.file(&env, &host).expect("path"),
        "[mcp_servers.kkt]\nurl = \"http://a/mcp\"\n",
    );
    let written = |client: Client| allow_kkt(client, &env, &host).expect("allowed");
    let claude = written(Client::ClaudeCode);
    assert_eq!(
        claude,
        "Allowed kkt's tools and `kkt watch`: /home/sam/.claude/settings.json"
    );
    let codex = written(Client::Codex);
    assert_eq!(
        codex,
        "Allowed kkt's tools and `kkt watch`: /home/sam/.codex/config.toml, \
         /home/sam/.codex/rules/kkt.rules"
    );
    let opencode = written(Client::OpenCode);
    assert!(
        opencode.ends_with(": /home/sam/.config/opencode/opencode.json"),
        "{opencode}"
    );
    let agy = written(Client::Agy);
    assert!(
        agy.starts_with(
            "Allowed kkt's tools and `kkt watch`: /home/sam/.gemini/antigravity-cli/settings.json\n"
        ),
        "{agy}"
    );
    assert!(agy.contains("Antigravity app"), "{agy}");
    assert!(
        host.file("/home/sam/.codex/rules/kkt.rules")
            .expect("rules")
            .text
            .starts_with("prefix_rule(pattern = [\"kkt\", \"watch\"]")
    );

    // Allowing again changes nothing and backs nothing up.
    let before = host.paths();
    for client in [Client::ClaudeCode, Client::Codex, Client::OpenCode] {
        let again = written(client);
        assert!(again.starts_with("Already allowed: "), "{again}");
    }
    assert_eq!(host.paths(), before);
}

#[test]
fn allowing_kkt_backs_up_a_file_it_changes() {
    let host = FakeHost::default();
    let env = env();
    let settings = PathBuf::from("/home/sam/.claude/settings.json");
    let old = r#"{"permissions": {"allow": ["Bash(ls)"]}}"#;
    host.put(&settings, old);
    allow_kkt(Client::ClaudeCode, &env, &host).expect("allowed");
    assert_eq!(host.file(beside(&settings, ".bak")), Some(private(old)));
    let new = host.file(&settings).expect("settings").text;
    assert!(new.contains("\"Bash(ls)\""), "{new}");
    assert!(new.contains("\"mcp__kkt\""), "{new}");
}

#[test]
fn allowing_kkt_follows_the_clients_own_variables() {
    let host = FakeHost::default();
    let env = Env {
        claude_config_dir: Some("/profiles/work".into()),
        codex_home: Some("/codex".into()),
        ..env()
    };
    host.put(
        "/codex/config.toml",
        "[mcp_servers.kkt]\nurl = \"http://a/mcp\"\n",
    );
    allow_kkt(Client::ClaudeCode, &env, &host).expect("allowed");
    allow_kkt(Client::Codex, &env, &host).expect("allowed");
    assert!(host.file("/profiles/work/settings.json").is_some());
    assert!(host.file("/codex/rules/kkt.rules").is_some());
}

#[test]
fn allowing_kkt_in_a_bad_file_names_it_and_changes_nothing() {
    let host = FakeHost::default();
    let env = env();
    host.put("/home/sam/.claude/settings.json", "{ not json");
    let error = allow_kkt(Client::ClaudeCode, &env, &host).expect_err("bad JSON");
    assert!(
        error.starts_with("/home/sam/.claude/settings.json: invalid client JSON"),
        "{error}"
    );
    assert!(error.ends_with("(from HOME)"), "{error}");
    assert_eq!(
        host.paths(),
        [PathBuf::from("/home/sam/.claude/settings.json")]
    );

    let error = allow_kkt(Client::Codex, &env, &host).expect_err("not connected");
    assert!(error.contains("connect it first"), "{error}");
    assert!(host.file("/home/sam/.codex/rules/kkt.rules").is_none());

    let error = allow_kkt(Client::MuseCode, &env, &host).expect_err("no setting");
    assert!(error.starts_with("Muse Code has no setting"), "{error}");
}

#[test]
fn a_kept_rule_is_named_and_kkt_is_not_called_allowed() {
    let host = FakeHost::default();
    let env = env();
    host.put(
        "/home/sam/.codex/config.toml",
        "[mcp_servers.kkt]\nurl = \"http://a/mcp\"\ndefault_tools_approval_mode = \"prompt\"\n",
    );
    host.put(
        "/home/sam/.codex/rules/kkt.rules",
        "prefix_rule(pattern = [\"kkt\", \"watch\"], decision = \"allow\")\n",
    );
    let said = allow_kkt(Client::Codex, &env, &host).expect("allowed");
    assert_eq!(
        said,
        "Left as they are: /home/sam/.codex/config.toml, /home/sam/.codex/rules/kkt.rules\n\
         Kept your own rules for kkt, which still apply instead: \
         default_tools_approval_mode = \"prompt\" (/home/sam/.codex/config.toml)"
    );
}

#[test]
fn rewriting_a_json_config_keeps_the_order_of_its_keys() {
    let host = FakeHost::default();
    let env = env();
    let path = Client::OpenCode.file(&env, &host).expect("path");
    let before = r#"{"theme": "dark", "permission": {"bash": {"rm -rf *": "deny", "rm *": "ask"}, "edit": "ask"}, "mcp": {"other": {"type": "remote"}}}"#;
    host.put(&path, before);
    allow_kkt(Client::OpenCode, &env, &host).expect("allowed");
    let file = Path::new("/home/sam/.config/opencode/opencode.json");
    let configured = setup(&env, file, None);
    configure(Client::OpenCode, &configured, true, &host).expect("connected");
    let text = host.file(&path).expect("config").text;
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle}: {text}"))
    };
    // The file's keys keep their places; new ones come after them.
    assert!(at("\"theme\"") < at("\"permission\""), "{text}");
    assert!(at("\"permission\"") < at("\"mcp\""), "{text}");
    assert!(at("\"rm -rf *\"") < at("\"rm *\""), "{text}");
    assert!(at("\"rm *\"") < at("\"kkt watch *\""), "{text}");
    assert!(at("\"edit\"") < at("\"kkt_activity\""), "{text}");
    assert!(at("\"other\"") < at("\"kkt\": {"), "{text}");
}
