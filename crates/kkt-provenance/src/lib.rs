//! Provenance: what produced a server run. Each run records the program
//! version, the git commit it was built from, the settings in effect, and a
//! hash of the instructions agents were given, so anything the run creates
//! can be traced back to them.

#[cfg(test)]
mod commit;
#[cfg(test)]
mod golden;

use kkt_logs::{LogLevel, LogPolicy};
use kkt_settings::Limits;
use sha2::{Digest, Sha256};

/// The build the server is running.
#[derive(Clone, Copy, Debug)]
pub struct Build {
    pub version: &'static str,
    /// The git commit, from `build.rs`. See `commit::commit_text`.
    pub commit: &'static str,
}

impl Build {
    /// This binary.
    pub const CURRENT: Build = Build {
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("KASHKESHET_GIT_COMMIT"),
    };
}

/// One run of the server, from start to stop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRun {
    pub version: String,
    pub commit: String,
    /// The settings in effect: the `allow_any_bind` line, then [`settings_text`].
    pub settings: String,
    /// SHA-256 of the instructions text, in lowercase hex.
    pub instructions_sha256: String,
}

/// The settings a server runs with, as the record of the run needs them.
/// Files and names are the text the record shows. Keys are never here: only
/// the files they came from.
#[derive(Clone, Debug)]
pub struct RunSettings {
    /// Whether the server may listen beyond loopback.
    pub allow_any_bind: bool,
    pub limits: Limits,
    /// How much kkt's own code logs.
    pub log_level: LogLevel,
    /// How much the libraries kkt uses log.
    pub dependency_log_level: LogLevel,
    /// Browser origins allowed to call `/mcp`.
    pub allowed_origins: Vec<String>,
    /// Host names allowed besides the loopback ones.
    pub allowed_hosts: Vec<String>,
    /// The config file the settings were read from, if any.
    pub config_file: Option<String>,
    /// The file the access key was read from, if there is one.
    pub access_key_file: Option<String>,
    /// The file the web key was read from, if there is one.
    pub web_key_file: Option<String>,
    /// The file the admin key was read from, if there is one.
    pub admin_key_file: Option<String>,
    /// The name tied to the admin key, such as `admin-sentinel`. Recorded
    /// only with `admin_key_file`.
    pub admin_name: Option<String>,
    /// The file the log goes to, if not stdout.
    pub log_file: Option<String>,
    /// The policy `log_file` was opened with. Recorded only with `log_file`.
    pub log_policy: LogPolicy,
    /// The instructions sent to every agent that connects.
    pub instructions: String,
}

impl ServerRun {
    pub fn new(build: Build, settings: &RunSettings) -> ServerRun {
        // Listing every field, with no `..`, makes a new setting fail to
        // compile here until it is added to the record.
        let RunSettings {
            allow_any_bind,
            limits,
            log_level,
            dependency_log_level,
            allowed_origins,
            allowed_hosts,
            config_file,
            access_key_file,
            web_key_file,
            admin_key_file,
            admin_name,
            log_file,
            log_policy,
            instructions,
        } = settings;
        let mut text = format!("allow_any_bind={allow_any_bind}\n");
        text.push_str(&settings_text(
            limits,
            *log_level,
            *dependency_log_level,
            allowed_origins,
            allowed_hosts,
            config_file.as_deref(),
            access_key_file.as_deref(),
        ));
        if let Some(file) = web_key_file {
            text.push_str(&format!("\nweb_key_file={file}"));
        }
        if let Some(file) = admin_key_file {
            text.push_str(&format!("\nadmin_key_file={file}"));
            if let Some(name) = admin_name {
                text.push_str(&format!("\nadmin_name={name}"));
            }
        }
        if let Some(file) = log_file {
            text.push('\n');
            text.push_str(&log_text(file, log_policy));
        }
        ServerRun {
            version: build.version.to_string(),
            commit: build.commit.to_string(),
            settings: text,
            instructions_sha256: sha256_hex(instructions),
        }
    }
}

/// The settings as text that stays the same for the same settings: one
/// `name=value` line each, in a fixed order, with times in milliseconds and
/// origins and added host names sorted. Then come the config file the
/// settings were read from, if any, and, when an access key is set, the file
/// it came from as the last line; the key itself is never recorded.
fn settings_text(
    limits: &Limits,
    log_level: LogLevel,
    dependency_log_level: LogLevel,
    allowed_origins: &[String],
    allowed_hosts: &[String],
    config_file: Option<&str>,
    access_key_file: Option<&str>,
) -> String {
    // Listing every field, with no `..`, makes a new limit fail to compile
    // here until it is added to the text.
    let Limits {
        name_pack,
        max_response_bytes,
        default_limit,
        max_limit,
        max_body_bytes,
        max_channel_name_chars,
        max_purpose_chars,
        max_description_chars,
        max_note_chars,
        max_server_announcement_chars,
        max_memory_entry_chars,
        max_memory_chars,
        max_lane_chars,
        sends_per_minute,
        registrations_per_minute,
        max_agents,
        default_wait,
        max_wait,
        waiting_grace,
        member_preview,
        purpose_preview_chars,
        web_session_lifetime,
        web_page_messages,
        web_member_excerpt_chars,
        upgrade_backups_kept,
    } = limits;
    let mut lines = vec![
        format!("name_pack={}", name_pack.name()),
        format!("max_response_bytes={max_response_bytes}"),
        format!("default_limit={default_limit}"),
        format!("max_limit={max_limit}"),
        format!("max_body_bytes={max_body_bytes}"),
        format!("max_channel_name_chars={max_channel_name_chars}"),
        format!("max_purpose_chars={max_purpose_chars}"),
        format!("max_description_chars={max_description_chars}"),
        format!("max_note_chars={max_note_chars}"),
        format!("max_server_announcement_chars={max_server_announcement_chars}"),
        format!("max_memory_entry_chars={max_memory_entry_chars}"),
        format!("max_memory_chars={max_memory_chars}"),
        format!("max_lane_chars={max_lane_chars}"),
        format!("sends_per_minute={sends_per_minute}"),
        format!("registrations_per_minute={registrations_per_minute}"),
        format!("max_agents={max_agents}"),
        format!("default_wait_ms={}", default_wait.as_millis()),
        format!("max_wait_ms={}", max_wait.as_millis()),
        format!("waiting_grace_ms={}", waiting_grace.as_millis()),
        format!("member_preview={member_preview}"),
        format!("purpose_preview_chars={purpose_preview_chars}"),
        format!(
            "web_session_lifetime_ms={}",
            web_session_lifetime.as_millis()
        ),
        format!("web_page_messages={web_page_messages}"),
        format!("web_member_excerpt_chars={web_member_excerpt_chars}"),
        format!("upgrade_backups_kept={upgrade_backups_kept}"),
    ];
    lines.push(format!("log_level={}", log_level.name()));
    lines.push(format!(
        "dependency_log_level={}",
        dependency_log_level.name()
    ));
    let mut origins = allowed_origins.to_vec();
    origins.sort();
    lines.extend(origins.iter().map(|o| format!("allowed_origin={o}")));
    let mut hosts = allowed_hosts.to_vec();
    hosts.sort();
    lines.extend(hosts.iter().map(|h| format!("allowed_host={h}")));
    lines.extend(config_file.map(|f| format!("config_file={f}")));
    lines.extend(access_key_file.map(|f| format!("access_key_file={f}")));
    lines.join("\n")
}

/// The log file and the policy that bounds it, as `name=value` lines.
fn log_text(file: &str, policy: &LogPolicy) -> String {
    // Listing every field, with no `..`, makes a new one fail to compile
    // here until it is added to the text.
    let LogPolicy {
        max_bytes,
        rotated_files,
        max_age,
    } = policy;
    [
        format!("log_file={file}"),
        format!("log_max_bytes={max_bytes}"),
        format!("log_rotated_files={rotated_files}"),
        format!("log_max_age_ms={}", max_age.as_millis()),
    ]
    .join("\n")
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(limits: Limits, origins: &[&str], instructions: &str) -> RunSettings {
        RunSettings {
            allow_any_bind: kkt_settings::testing::DEFAULT_ALLOW_ANY_BIND,
            limits,
            log_level: kkt_settings::DEFAULT_LOG_LEVEL,
            dependency_log_level: kkt_settings::DEFAULT_DEPENDENCY_LOG_LEVEL,
            allowed_origins: origins.iter().map(|o| o.to_string()).collect(),
            allowed_hosts: Vec::new(),
            config_file: None,
            access_key_file: None,
            web_key_file: None,
            admin_key_file: None,
            admin_name: None,
            log_file: None,
            log_policy: LogPolicy::DEFAULT,
            instructions: instructions.into(),
        }
    }

    const TEST_BUILD: Build = Build {
        version: "9.9.9",
        commit: "abc123",
    };

    #[test]
    fn a_run_records_the_build_settings_and_instructions() {
        let run = ServerRun::new(TEST_BUILD, &config(Limits::DEFAULT, &[], ""));
        assert_eq!(run.version, "9.9.9");
        assert_eq!(run.commit, "abc123");
        assert_eq!(
            run.settings,
            format!(
                "allow_any_bind=false\n{}",
                settings_text(
                    &Limits::DEFAULT,
                    LogLevel::Info,
                    LogLevel::Warn,
                    &[],
                    &[],
                    None,
                    None
                )
            )
        );
        // SHA-256 of the empty string.
        assert_eq!(
            run.instructions_sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn default_settings_text_is_fixed() {
        assert_eq!(
            settings_text(
                &Limits::DEFAULT,
                LogLevel::Info,
                LogLevel::Warn,
                &["http://localhost:6274".into()],
                &["host.docker.internal".into()],
                None,
                None
            ),
            "name_pack=animals\n\
             max_response_bytes=8192\n\
             default_limit=20\n\
             max_limit=100\n\
             max_body_bytes=8192\n\
             max_channel_name_chars=64\n\
             max_purpose_chars=500\n\
             max_description_chars=300\n\
             max_note_chars=10000\n\
             max_server_announcement_chars=4000\n\
             max_memory_entry_chars=20000\n\
             max_memory_chars=100000\n\
             max_lane_chars=8000\n\
             sends_per_minute=30\n\
             registrations_per_minute=20\n\
             max_agents=1000\n\
             default_wait_ms=30000\n\
             max_wait_ms=50000\n\
             waiting_grace_ms=3000\n\
             member_preview=3\n\
             purpose_preview_chars=120\n\
             web_session_lifetime_ms=43200000\n\
             web_page_messages=50\n\
             web_member_excerpt_chars=80\n\
             upgrade_backups_kept=5\n\
             log_level=info\n\
             dependency_log_level=warn\n\
             allowed_origin=http://localhost:6274\n\
             allowed_host=host.docker.internal"
        );
    }

    #[test]
    fn origin_order_does_not_change_the_text() {
        let a = settings_text(
            &Limits::DEFAULT,
            LogLevel::Info,
            LogLevel::Warn,
            &["http://b".into(), "http://a".into()],
            &[],
            None,
            None,
        );
        let b = settings_text(
            &Limits::DEFAULT,
            LogLevel::Info,
            LogLevel::Warn,
            &["http://a".into(), "http://b".into()],
            &[],
            None,
            None,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn a_run_records_the_config_file_before_the_key_file() {
        let mut from_file = config(Limits::DEFAULT, &[], "");
        from_file.config_file = Some("/home/sam/.config/kkt/config.toml".into());
        from_file.access_key_file = Some("/keys/kkt".into());
        let run = ServerRun::new(TEST_BUILD, &from_file);
        assert!(run.settings.ends_with(
            "\nconfig_file=/home/sam/.config/kkt/config.toml\naccess_key_file=/keys/kkt"
        ));
        let plain = ServerRun::new(TEST_BUILD, &config(Limits::DEFAULT, &[], ""));
        assert!(!plain.settings.contains("config_file"));
    }

    #[test]
    fn a_run_records_whether_any_address_may_be_bound() {
        let mut open = config(Limits::DEFAULT, &[], "");
        let closed = ServerRun::new(TEST_BUILD, &open);
        assert!(closed.settings.starts_with("allow_any_bind=false\n"));
        open.allow_any_bind = true;
        let allowed = ServerRun::new(TEST_BUILD, &open);
        assert!(allowed.settings.starts_with("allow_any_bind=true\n"));
    }

    #[test]
    fn host_order_does_not_change_the_text() {
        let a = settings_text(
            &Limits::DEFAULT,
            LogLevel::Info,
            LogLevel::Warn,
            &[],
            &["b.example".into(), "a.example".into()],
            None,
            None,
        );
        let b = settings_text(
            &Limits::DEFAULT,
            LogLevel::Info,
            LogLevel::Warn,
            &[],
            &["a.example".into(), "b.example".into()],
            None,
            None,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn added_hosts_give_a_different_run() {
        let base = ServerRun::new(TEST_BUILD, &config(Limits::DEFAULT, &[], ""));
        let mut with_host = config(Limits::DEFAULT, &[], "");
        with_host.allowed_hosts = vec!["host.docker.internal".into()];
        let changed = ServerRun::new(TEST_BUILD, &with_host);
        assert_ne!(base.settings, changed.settings);
        assert!(
            changed
                .settings
                .ends_with("\nallowed_host=host.docker.internal")
        );
    }

    #[test]
    fn a_run_records_the_key_files() {
        let mut keyed = config(Limits::DEFAULT, &[], "");
        keyed.access_key_file = Some("/keys/kkt".into());
        keyed.web_key_file = Some("/keys/web-key".into());
        let run = ServerRun::new(TEST_BUILD, &keyed);
        assert!(run.settings.contains("\naccess_key_file=/keys/kkt\n"));
        assert!(run.settings.ends_with("\nweb_key_file=/keys/web-key"));
        let open = ServerRun::new(TEST_BUILD, &config(Limits::DEFAULT, &[], ""));
        assert!(!open.settings.contains("access_key"));
        assert!(!run.settings.contains("admin_key"));
    }

    #[test]
    fn a_run_records_the_admin_key_file_and_name() {
        let mut keyed = config(Limits::DEFAULT, &[], "");
        keyed.admin_key_file = Some("/keys/admin-key".into());
        let run = ServerRun::new(TEST_BUILD, &keyed);
        assert!(run.settings.ends_with("\nadmin_key_file=/keys/admin-key"));
        let without = ServerRun::new(TEST_BUILD, &config(Limits::DEFAULT, &[], ""));
        assert_ne!(run.settings, without.settings);
        // A name tied to the key shapes the run too: its changes carry it.
        keyed.admin_name = Some("admin-sentinel".into());
        let named = ServerRun::new(TEST_BUILD, &keyed);
        assert!(
            named
                .settings
                .ends_with("\nadmin_key_file=/keys/admin-key\nadmin_name=admin-sentinel")
        );
    }

    #[test]
    fn different_settings_or_instructions_give_a_different_run() {
        let base = ServerRun::new(TEST_BUILD, &config(Limits::DEFAULT, &[], "hello"));
        let other_limits = Limits {
            max_limit: 99,
            ..Limits::DEFAULT
        };
        let changed = ServerRun::new(TEST_BUILD, &config(other_limits, &[], "hello"));
        assert_ne!(base.settings, changed.settings);
        let changed = ServerRun::new(TEST_BUILD, &config(Limits::DEFAULT, &[], "hello!"));
        assert_eq!(base.settings, changed.settings);
        assert_ne!(base.instructions_sha256, changed.instructions_sha256);
    }

    #[test]
    fn this_build_names_a_commit_or_says_unknown() {
        let commit = Build::CURRENT.commit;
        let hash = commit.strip_suffix("-dirty").unwrap_or(commit);
        assert!(
            commit == "unknown"
                || (hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit())),
            "unexpected commit text {commit:?}"
        );
    }

    #[test]
    fn a_run_records_its_log_file_and_how_it_is_bounded() {
        let mut with_log = config(Limits::DEFAULT, &[], "");
        with_log.log_file = Some("/logs/kkt.log".into());
        let run = ServerRun::new(TEST_BUILD, &with_log);
        assert!(
            run.settings.ends_with(
                "\nlog_file=/logs/kkt.log\n\
                 log_max_bytes=10485760\n\
                 log_rotated_files=5\n\
                 log_max_age_ms=2592000000"
            ),
            "{}",
            run.settings
        );
        let mut other_policy = with_log.clone();
        other_policy.log_policy = LogPolicy {
            max_bytes: 1024 * 1024,
            rotated_files: 2,
            max_age: std::time::Duration::from_secs(60),
        };
        let run = ServerRun::new(TEST_BUILD, &other_policy);
        assert!(
            run.settings.ends_with(
                "\nlog_file=/logs/kkt.log\n\
                 log_max_bytes=1048576\n\
                 log_rotated_files=2\n\
                 log_max_age_ms=60000"
            ),
            "{}",
            run.settings
        );
        let without = ServerRun::new(TEST_BUILD, &config(Limits::DEFAULT, &[], ""));
        assert!(!without.settings.contains("log_file"));
        assert!(
            without
                .settings
                .contains("\nlog_level=info\ndependency_log_level=warn"),
            "{}",
            without.settings
        );
    }
}
