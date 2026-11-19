//! Unit tests, against [`FakeHost`]: no real files and no `claude`.

use super::fake::{FakeHost, File};
use super::*;
use crate::testing::Call;

const HOME: &str = "/home/sam";
const KEY: &str = "0123456789abcdef0123456789abcdef";

fn env() -> Env {
    Env {
        home: Some(PathBuf::from(HOME)),
        xdg_config_home: None,
        xdg_data_home: None,
        named: Default::default(),
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
        mode: 0o600,
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
            client.connected(&env, &host).map_err(|e| e.to_string()),
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
    assert_eq!(
        Client::Codex
            .connected(&env, &host)
            .map_err(|e| e.to_string()),
        Ok(true)
    );
    assert_eq!(
        Client::ClaudeCode
            .connected(&env, &host)
            .map_err(|e| e.to_string()),
        Ok(false)
    );
    assert_eq!(
        Client::OpenCode
            .connected(&env, &host)
            .map_err(|e| e.to_string()),
        Ok(true)
    );
    assert_eq!(
        Client::MuseCode
            .connected(&env, &host)
            .map_err(|e| e.to_string()),
        Ok(true)
    );
    let error = Client::Agy
        .connected(&env, &host)
        .expect_err("bad JSON")
        .to_string();
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
    let error = codex_password_store_hint(&env, &host)
        .expect_err("malformed")
        .to_string();
    let cause = error
        .strip_prefix(&format!("invalid Codex TOML in {}: ", path.display()))
        .unwrap_or_else(|| panic!("{error}"));
    let reason = kkt_toml::Document::parse(&text).expect_err("malformed");
    let first = reason.reason().lines().next().expect("reason").trim();
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
        Path::new(HOME).join(CODEX_HOME_DEFAULT).join(CODEX_CONFIG),
        "",
    );
    let on_path = |program: &str| program == CLAUDE_PROGRAM;
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
    let new = Path::new(HOME).join(AGY_CONFIG);
    let legacy = Path::new(HOME).join(AGY_LEGACY_CONFIG);
    assert_eq!(
        Client::Agy.file(&env, &host).map_err(|e| e.to_string()),
        Ok(new.clone())
    );
    host.put(&legacy, "{}");
    assert_eq!(
        Client::Agy.file(&env, &host).map_err(|e| e.to_string()),
        Ok(legacy)
    );
    host.put(&new, "{}");
    assert_eq!(
        Client::Agy.file(&env, &host).map_err(|e| e.to_string()),
        Ok(new)
    );
}

#[test]
fn writing_backs_up_the_old_config_under_a_free_name_and_replaces_it_whole() {
    let host = FakeHost::default();
    let env = env();
    let path = Client::Codex.file(&env, &host).expect("path");
    let old = "model = \"o3\"\n";
    host.put(&path, old);
    host.put(beside(&path, ".bak"), "an older backup");

    let out =
        configure(Client::Codex, &setup(&env, &path, None), true, &host, &host).expect("write");
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

    let again =
        configure(Client::Codex, &setup(&env, &path, None), true, &host, &host).expect("again");
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
    let error = configure(Client::Codex, &setup(&env, &path, None), true, &host, &host)
        .expect_err("refused");
    assert_eq!(
        error.to_string(),
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

    let preview = configure(Client::MuseCode, &muse, false, &host, &host).expect("preview");
    assert!(!preview.contains(KEY), "{preview}");
    assert!(host.file(&path).is_none(), "a preview writes nothing");

    configure(Client::MuseCode, &muse, true, &host, &host).expect("write");
    let written = host.file(&path).expect("config");
    assert!(written.private);
    assert!(written.text.contains(KEY), "{}", written.text);

    // Already configured, but made readable by others: made private again.
    host.files
        .borrow_mut()
        .get_mut(&path)
        .expect("config")
        .private = false;
    let again = configure(Client::MuseCode, &muse, true, &host, &host).expect("again");
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
    let error = configure(
        Client::ClaudeCode,
        &setup(&env, &path, None),
        true,
        &host,
        &host,
    )
    .expect_err("fails");
    assert_eq!(error.to_string(), "claude mcp failed");
    assert_eq!(host.file(&path), Some(private(old)), "restored whole");

    host.claude_fails.set(false);
    host.claude_runs.borrow_mut().clear();
    configure(
        Client::ClaudeCode,
        &setup(&env, &path, None),
        true,
        &host,
        &host,
    )
    .expect("write");
    let runs = host.claude_runs.borrow();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0], Call::Remove { config_dir: None });
    let Call::Add { entry, config_dir } = &runs[1] else {
        panic!("{:?}", runs[1]);
    };
    assert!(entry.contains("http://localhost:7420/mcp"), "{entry}");
    // The default config under HOME: no profile folder is given.
    assert_eq!(config_dir, &None);
}

#[test]
fn a_new_claude_code_config_is_removed_if_its_command_fails() {
    let host = FakeHost::default();
    let env = env();
    let dir = Path::new("/profiles/work");
    let path = dir.join(CLAUDE_CONFIG);
    host.create_dir_all(dir).expect("dir");
    // The command writes the file, then fails.
    *host.claude_writes.borrow_mut() = Some(path.clone());
    host.claude_fails.set(true);
    let error = configure(
        Client::ClaudeCode,
        &setup(&env, &path, None),
        true,
        &host,
        &host,
    )
    .expect_err("fails");
    assert_eq!(error.to_string(), "claude mcp failed");
    assert!(host.file(&path).is_none());
    // Nothing to remove first, and a profile outside HOME is passed as
    // CLAUDE_CONFIG_DIR.
    let runs = host.claude_runs.borrow();
    assert_eq!(runs.len(), 1);
    assert!(
        matches!(&runs[0], Call::Add { config_dir: Some(d), .. } if d == dir),
        "{:?}",
        runs[0]
    );
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
fn allowing_kkt_keeps_an_existing_settings_files_mode() {
    let host = FakeHost::default();
    let env = env();
    let settings = PathBuf::from("/home/sam/.claude/settings.json");
    host.put(&settings, r#"{"permissions": {"allow": ["Bash(ls)"]}}"#);
    host.files
        .borrow_mut()
        .get_mut(&settings)
        .expect("settings")
        .mode = 0o640;
    allow_kkt(Client::ClaudeCode, &env, &host).expect("allowed");
    let written = host.file(&settings).expect("settings");
    assert_eq!(written.mode, 0o640);
    assert!(!written.private);
}

#[test]
fn a_key_holding_config_replaced_whole_ends_private() {
    let host = FakeHost::default();
    let env = env();
    let path = Client::MuseCode.file(&env, &host).expect("path");
    let key_file = Path::new("/keys/kkt.key");
    host.put(key_file, &format!("{KEY}\n"));
    host.put(&path, "{}");
    configure(
        Client::MuseCode,
        &setup(&env, &path, Some(key_file)),
        true,
        &host,
        &host,
    )
    .expect("write");
    let written = host.file(&path).expect("config");
    assert_eq!(written.mode, 0o600);
    assert!(written.private);
}

#[test]
fn allowing_kkt_follows_the_clients_own_variables() {
    let host = FakeHost::default();
    let env = Env {
        named: [
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                std::path::PathBuf::from("/profiles/work"),
            ),
            ("CODEX_HOME".to_string(), std::path::PathBuf::from("/codex")),
        ]
        .into(),
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
    let error = allow_kkt(Client::ClaudeCode, &env, &host)
        .expect_err("bad JSON")
        .to_string();
    assert!(
        error.starts_with("/home/sam/.claude/settings.json: invalid client JSON"),
        "{error}"
    );
    assert!(error.ends_with("(from HOME)"), "{error}");
    assert_eq!(
        host.paths(),
        [PathBuf::from("/home/sam/.claude/settings.json")]
    );

    let error = allow_kkt(Client::Codex, &env, &host)
        .expect_err("not connected")
        .to_string();
    assert!(error.contains("connect it first"), "{error}");
    assert!(host.file("/home/sam/.codex/rules/kkt.rules").is_none());

    let error = allow_kkt(Client::MuseCode, &env, &host)
        .expect_err("no setting")
        .to_string();
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
    configure(Client::OpenCode, &configured, true, &host, &host).expect("connected");
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

/// Every message the crate can give, in the order of the cases below, as
/// the code gives it today.
fn every_message() -> Vec<String> {
    use std::os::unix::ffi::OsStrExt;
    let e = env();
    let no_home = Env {
        home: None,
        ..env()
    };
    let config = PathBuf::from("/c/config.json");
    let mut said = Vec::new();
    let mut push = |result: Result<String, ClientConfigError>| {
        said.push(match result {
            Err(e) => e.to_string(),
            Ok(v) => format!("OK<{v}>"),
        })
    };
    // The files' steps.
    let refusing = |op: &'static str| {
        let host = FakeHost::default();
        host.refuse(op);
        host
    };
    push(files::read(&refusing("read"), &config).map(|_| String::new()));
    let host = refusing("create_private");
    host.put(&config, "{}");
    push(files::replace(&host, &config, Some("{}"), "{}").map(|_| String::new()));
    push(files::restrict(&refusing("restrict"), &config).map(|_| String::new()));
    push(files::replace(&refusing("create_dir_all"), &config, None, "{}").map(|_| String::new()));
    push(files::replace(&refusing("create_private"), &config, None, "{}").map(|_| String::new()));
    let host = FakeHost::default();
    host.rename_fails.set(true);
    push(files::replace(&host, &config, None, "{}").map(|_| String::new()));
    let host = FakeHost::default();
    host.rename_fails.set(true);
    host.refuse("remove_file");
    push(files::replace(&host, &config, None, "{}").map(|_| String::new()));
    push(files::replace(&FakeHost::default(), Path::new("/"), None, "{}").map(|_| String::new()));
    // Naming and finding the client's config.
    push(
        Client::Codex
            .check_name(Path::new("/c/wrong.toml"), "CODEX_HOME")
            .map(|_| String::new()),
    );
    push(
        Client::ClaudeCode
            .file(&no_home, &FakeHost::default())
            .map(|p| p.display().to_string()),
    );
    push(
        Client::OpenCode
            .file(&no_home, &FakeHost::default())
            .map(|p| p.display().to_string()),
    );
    let host = FakeHost::default();
    host.put(Client::Codex.file(&e, &host).unwrap(), "a = = b");
    push(Client::Codex.connected(&e, &host).map(|_| String::new()));
    // Merging an entry.
    let merge = |client: Client, text: &str, key: Option<&Path>, host: &FakeHost| {
        formats::merge(client, &setup(&e, &config, key), Some(text), host).map(|_| String::new())
    };
    push(merge(
        Client::OpenCode,
        "{not json",
        None,
        &FakeHost::default(),
    ));
    push(merge(Client::OpenCode, "[]", None, &FakeHost::default()));
    let key = Path::new("/k/key");
    push(merge(Client::Agy, "{}", Some(key), &refusing("read")));
    let host = FakeHost::default();
    host.put(key, "short");
    push(merge(Client::Agy, "{}", Some(key), &host));
    let host = FakeHost::default();
    let odd = Path::new(std::ffi::OsStr::from_bytes(b"/k/\xff"));
    host.put(odd, KEY);
    push(merge(Client::OpenCode, "{}", Some(odd), &host));
    let host = FakeHost::default();
    let braces = Path::new("/k/{key}");
    host.put(braces, KEY);
    push(merge(Client::OpenCode, "{}", Some(braces), &host));
    push(merge(
        Client::Codex,
        "not = toml = x",
        None,
        &FakeHost::default(),
    ));
    push(merge(
        Client::Codex,
        "mcp_servers = 1\n",
        None,
        &FakeHost::default(),
    ));
    push(merge(
        Client::Codex,
        "[mcp_servers]\nkkt = 1\n",
        None,
        &FakeHost::default(),
    ));
    let host = FakeHost::default();
    host.put(key, KEY);
    push(merge(
        Client::Codex,
        "[mcp_servers.kkt]\nhttp_headers = 1\n",
        Some(key),
        &host,
    ));
    push(formats::claude_entry("{}"));
    let host = FakeHost::default();
    host.put(odd, KEY);
    push(merge(Client::ClaudeCode, "{}", Some(odd), &host));
    let odd_kkt = Setup {
        executable: odd,
        ..setup(&e, &config, Some(key))
    };
    push(formats::merge(Client::ClaudeCode, &odd_kkt, Some("{}"), &host).map(|_| String::new()));
    let odd_dir = Path::new(std::ffi::OsStr::from_bytes(b"/c/\xff"));
    let in_odd_dir = Env {
        named: [(CLAUDE_CONFIG_DIR.to_string(), odd_dir.to_path_buf())].into(),
        ..env()
    };
    let odd_claude = odd_dir.join(".claude.json");
    let host = FakeHost::default();
    push(configure(
        Client::ClaudeCode,
        &setup(&in_odd_dir, &odd_claude, None),
        false,
        &host,
        &host,
    ));
    push(formats::has_kkt_entry(Client::Codex, "a = = b").map(|_| String::new()));
    // Configuring, with the context the errors carry.
    let claude = PathBuf::from("/c/.claude.json");
    let claude_setup = setup(&e, &claude, None);
    let host = refusing("absolute");
    push(configure(
        Client::ClaudeCode,
        &claude_setup,
        true,
        &host,
        &host,
    ));
    let host = FakeHost::default();
    host.put(&claude, "{not json");
    push(configure(
        Client::ClaudeCode,
        &claude_setup,
        true,
        &host,
        &host,
    ));
    let host = FakeHost::default();
    host.claude_fails.set(true);
    push(configure(
        Client::ClaudeCode,
        &claude_setup,
        true,
        &host,
        &host,
    ));
    let host = FakeHost::default();
    host.put(&claude, "{}");
    host.claude_fails.set(true);
    host.claude_writes.replace(Some(claude.clone()));
    host.refuse("create_dir_all");
    push(configure(
        Client::ClaudeCode,
        &claude_setup,
        true,
        &host,
        &host,
    ));
    let host = FakeHost::default();
    host.claude_fails.set(true);
    host.claude_writes.replace(Some(claude.clone()));
    host.refuse("remove_file");
    push(configure(
        Client::ClaudeCode,
        &claude_setup,
        true,
        &host,
        &host,
    ));
    let host = refusing("read");
    push(configure(
        Client::OpenCode,
        &setup(&e, &config, None),
        true,
        &host,
        &host,
    ));
    push(header("short", false));
    let host = FakeHost::default();
    host.put(Client::Codex.file(&e, &host).unwrap(), "a = = b");
    push(codex_password_store_hint(&e, &host).map(|_| String::new()));
    // Allowing kkt's tools.
    push(permissions::allow_kkt(
        Client::MuseCode,
        &e,
        &FakeHost::default(),
    ));
    push(permissions::allow_kkt(
        Client::Codex,
        &e,
        &FakeHost::default(),
    ));
    push(permissions::allow_kkt(
        Client::ClaudeCode,
        &no_home,
        &FakeHost::default(),
    ));
    let host = FakeHost::default();
    host.put(
        Client::Codex.file(&e, &host).unwrap(),
        "[mcp_servers]\nkkt = 1\n",
    );
    push(permissions::allow_kkt(Client::Codex, &e, &host));
    let host = FakeHost::default();
    host.put(Client::Codex.file(&e, &host).unwrap(), "a = = b");
    push(permissions::allow_kkt(Client::Codex, &e, &host));
    let settings = PathBuf::from(HOME).join(".claude/settings.json");
    for text in ["[]", r#"{"permissions":{"allow":1}}"#] {
        let host = FakeHost::default();
        host.put(&settings, text);
        push(permissions::allow_kkt(Client::ClaudeCode, &e, &host));
    }
    for text in [r#"{"permission":1}"#, r#"{"permission":{"bash":1}}"#] {
        let host = FakeHost::default();
        host.put(Client::OpenCode.file(&e, &host).unwrap(), text);
        push(permissions::allow_kkt(Client::OpenCode, &e, &host));
    }
    push(permissions::allow_kkt(
        Client::ClaudeCode,
        &e,
        &refusing("read"),
    ));
    let host = refusing("create_dir_all");
    push(permissions::allow_kkt(Client::ClaudeCode, &e, &host));
    said
}

/// Every message a person can be shown about a client's config, pinned
/// byte for byte: what `kkt connect`, `kkt setup` and `kkt switch` print
/// when configuring a client fails.
#[test]
fn every_client_config_message_says_what_it_always_said() {
    let expected = [
        "could not read /c/config.json: refused",
        "could not back up /c/config.json: refused",
        "could not restrict /c/config.json: refused",
        "could not create /c: refused",
        "could not prepare /c/config.json: refused",
        "could not write /c/config.json: rename refused",
        "could not write /c/config.json: rename refused; could not remove temporary file: refused",
        "config path has no parent",
        "Codex config must be named config.toml: /c/wrong.toml (from CODEX_HOME)",
        "set HOME to find the client's config",
        "set HOME to find the client's config",
        "Codex: /home/sam/.codex/config.toml (from HOME): invalid Codex TOML: extra `=`, expected nothing at line 1, column 5",
        "invalid client JSON at line 1, column 2; the file was not changed",
        "client configuration must contain objects, not scalars or lists",
        "could not read /k/key: refused",
        "access_key_file must hold a key of at least 32 characters",
        "key path is not UTF-8",
        "OpenCode file references cannot represent braces, quotes, backslashes or control characters in a key path",
        "invalid Codex TOML; the file was not changed: unexpected key or value, expected newline, `#` at line 1, column 12",
        "mcp_servers must be a TOML table",
        "mcp_servers.kkt must be a TOML table",
        "Codex HTTP headers must be a table",
        "missing kkt entry",
        "the key path is not UTF-8",
        "kkt's path is not UTF-8",
        "the Claude Code config directory /c/\u{FFFD} is not UTF-8 (from HOME)",
        "invalid Codex TOML: extra `=`, expected nothing at line 1, column 5",
        "could not resolve the directory of the Claude Code config /c/.claude.json (from HOME): refused",
        "/c/.claude.json (from HOME): invalid client JSON at line 1, column 2; the file was not changed",
        "claude mcp failed",
        "claude mcp failed; could not restore: could not create /c: refused (from HOME)",
        "claude mcp failed; could not restore /c/.claude.json: refused (from HOME)",
        "could not read /c/config.json: refused (from HOME)",
        "access_key_file must hold a key of at least 32 characters",
        "invalid Codex TOML in /home/sam/.codex/config.toml: extra `=`, expected nothing at line 1, column 5",
        "Muse Code has no setting that allows one MCP server's tools or one command, so it still asks before each",
        "/home/sam/.codex/config.toml: Codex has no kkt server here; connect it first (from HOME)",
        "set HOME to find the client's config",
        "/home/sam/.codex/config.toml: mcp_servers.kkt must be a TOML table (from HOME)",
        "/home/sam/.codex/config.toml: invalid Codex TOML; the file was not changed: extra `=`, expected nothing at line 1, column 5 (from HOME)",
        "/home/sam/.claude/settings.json: client configuration must contain objects, not scalars or lists (from HOME)",
        "/home/sam/.claude/settings.json: permissions.allow must be a list (from HOME)",
        "/home/sam/.config/opencode/opencode.json: client configuration must contain objects, not scalars or lists (from HOME)",
        "/home/sam/.config/opencode/opencode.json: permission.bash must be a word or an object (from HOME)",
        "could not read /home/sam/.claude/settings.json: refused (from HOME)",
        "could not create /home/sam/.claude: refused (from HOME)",
    ];
    assert_eq!(every_message(), expected);
}
