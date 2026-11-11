//! The settings a server run records, worded as lines of a setting's name
//! and value. kkt-provenance stores the lines as the run's settings text,
//! which is hashed and kept, so the wording here changes only on purpose.

use crate::Limits;
use kkt_logs::{LogLevel, LogPolicy};

/// The settings a server runs with, as the record of the run needs them.
/// Files and names are the text the record shows. Keys are never here: only
/// the files they came from.
#[derive(Clone, Debug)]
pub struct RecordedSettings {
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
}

impl RecordedSettings {
    /// The settings as lines of a name and a value that stay the same for
    /// the same settings, in a fixed order: `allow_any_bind`, then
    /// [`limit_lines`]'s, then the web key file, the admin key file and its
    /// name, and the log file and its policy, each only when there is one.
    pub fn lines(&self) -> Vec<(String, String)> {
        // Listing every field, with no `..`, makes a new setting fail to
        // compile here until it is added to the record.
        let RecordedSettings {
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
        } = self;
        let mut lines = vec![line("allow_any_bind", allow_any_bind)];
        lines.extend(limit_lines(
            limits,
            *log_level,
            *dependency_log_level,
            allowed_origins,
            allowed_hosts,
            config_file.as_deref(),
            access_key_file.as_deref(),
        ));
        lines.extend(web_key_file.iter().map(|file| line("web_key_file", file)));
        if let Some(file) = admin_key_file {
            lines.push(line("admin_key_file", file));
            lines.extend(admin_name.iter().map(|name| line("admin_name", name)));
        }
        if let Some(file) = log_file {
            lines.extend(log_lines(file, log_policy));
        }
        lines
    }
}

fn line(name: &str, value: impl std::fmt::Display) -> (String, String) {
    (name.to_string(), value.to_string())
}

/// The limits and the other settings every run has, one line each, in a
/// fixed order, with times in milliseconds and origins and added host names
/// sorted. Then come the config file the settings were read from, if any,
/// and, when an access key is set, the file it came from; the key itself is
/// never recorded.
fn limit_lines(
    limits: &Limits,
    log_level: LogLevel,
    dependency_log_level: LogLevel,
    allowed_origins: &[String],
    allowed_hosts: &[String],
    config_file: Option<&str>,
    access_key_file: Option<&str>,
) -> Vec<(String, String)> {
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
        line("name_pack", name_pack.name()),
        line("max_response_bytes", max_response_bytes),
        line("default_limit", default_limit),
        line("max_limit", max_limit),
        line("max_body_bytes", max_body_bytes),
        line("max_channel_name_chars", max_channel_name_chars),
        line("max_purpose_chars", max_purpose_chars),
        line("max_description_chars", max_description_chars),
        line("max_note_chars", max_note_chars),
        line(
            "max_server_announcement_chars",
            max_server_announcement_chars,
        ),
        line("max_memory_entry_chars", max_memory_entry_chars),
        line("max_memory_chars", max_memory_chars),
        line("max_lane_chars", max_lane_chars),
        line("sends_per_minute", sends_per_minute),
        line("registrations_per_minute", registrations_per_minute),
        line("max_agents", max_agents),
        line("default_wait_ms", default_wait.as_millis()),
        line("max_wait_ms", max_wait.as_millis()),
        line("waiting_grace_ms", waiting_grace.as_millis()),
        line("member_preview", member_preview),
        line("purpose_preview_chars", purpose_preview_chars),
        line("web_session_lifetime_ms", web_session_lifetime.as_millis()),
        line("web_page_messages", web_page_messages),
        line("web_member_excerpt_chars", web_member_excerpt_chars),
        line("upgrade_backups_kept", upgrade_backups_kept),
    ];
    lines.push(line("log_level", log_level.name()));
    lines.push(line("dependency_log_level", dependency_log_level.name()));
    let mut origins = allowed_origins.to_vec();
    origins.sort();
    lines.extend(origins.iter().map(|o| line("allowed_origin", o)));
    let mut hosts = allowed_hosts.to_vec();
    hosts.sort();
    lines.extend(hosts.iter().map(|h| line("allowed_host", h)));
    lines.extend(config_file.map(|f| line("config_file", f)));
    lines.extend(access_key_file.map(|f| line("access_key_file", f)));
    lines
}

/// The log file and the policy that bounds it.
fn log_lines(file: &str, policy: &LogPolicy) -> Vec<(String, String)> {
    // Listing every field, with no `..`, makes a new one fail to compile
    // here until it is added to the text.
    let LogPolicy {
        max_bytes,
        rotated_files,
        max_age,
    } = policy;
    vec![
        line("log_file", file),
        line("log_max_bytes", max_bytes),
        line("log_rotated_files", rotated_files),
        line("log_max_age_ms", max_age.as_millis()),
    ]
}

#[cfg(test)]
mod golden;

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_provenance::{Build, RunSettings, ServerRun};

    const TEST_BUILD: Build = Build {
        version: "9.9.9",
        commit: "abc123",
    };

    fn config(limits: Limits, origins: &[&str]) -> RecordedSettings {
        RecordedSettings {
            allow_any_bind: crate::DEFAULT_ALLOW_ANY_BIND,
            limits,
            log_level: crate::DEFAULT_LOG_LEVEL,
            dependency_log_level: crate::DEFAULT_DEPENDENCY_LOG_LEVEL,
            allowed_origins: origins.iter().map(|o| o.to_string()).collect(),
            allowed_hosts: Vec::new(),
            config_file: None,
            access_key_file: None,
            web_key_file: None,
            admin_key_file: None,
            admin_name: None,
            log_file: None,
            log_policy: LogPolicy::DEFAULT,
        }
    }

    /// The text a run records for `lines`, as kkt-provenance writes it.
    fn recorded(lines: Vec<(String, String)>) -> String {
        ServerRun::new(
            TEST_BUILD,
            &RunSettings {
                settings: lines,
                instructions: String::new(),
            },
        )
        .settings
    }

    fn run(settings: &RecordedSettings) -> String {
        recorded(settings.lines())
    }

    /// The text of [`limit_lines`] alone.
    fn settings_text(
        limits: &Limits,
        log_level: LogLevel,
        dependency_log_level: LogLevel,
        allowed_origins: &[String],
        allowed_hosts: &[String],
        config_file: Option<&str>,
        access_key_file: Option<&str>,
    ) -> String {
        recorded(limit_lines(
            limits,
            log_level,
            dependency_log_level,
            allowed_origins,
            allowed_hosts,
            config_file,
            access_key_file,
        ))
    }

    #[test]
    fn a_run_records_the_settings() {
        assert_eq!(
            run(&config(Limits::DEFAULT, &[])),
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
        let mut from_file = config(Limits::DEFAULT, &[]);
        from_file.config_file = Some("/home/sam/.config/kkt/config.toml".into());
        from_file.access_key_file = Some("/keys/kkt".into());
        assert!(run(&from_file).ends_with(
            "\nconfig_file=/home/sam/.config/kkt/config.toml\naccess_key_file=/keys/kkt"
        ));
        let plain = run(&config(Limits::DEFAULT, &[]));
        assert!(!plain.contains("config_file"));
    }

    #[test]
    fn a_run_records_whether_any_address_may_be_bound() {
        let mut open = config(Limits::DEFAULT, &[]);
        let closed = run(&open);
        assert!(closed.starts_with("allow_any_bind=false\n"));
        open.allow_any_bind = true;
        let allowed = run(&open);
        assert!(allowed.starts_with("allow_any_bind=true\n"));
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
        let base = run(&config(Limits::DEFAULT, &[]));
        let mut with_host = config(Limits::DEFAULT, &[]);
        with_host.allowed_hosts = vec!["host.docker.internal".into()];
        let changed = run(&with_host);
        assert_ne!(base, changed);
        assert!(changed.ends_with("\nallowed_host=host.docker.internal"));
    }

    #[test]
    fn a_run_records_the_key_files() {
        let mut keyed = config(Limits::DEFAULT, &[]);
        keyed.access_key_file = Some("/keys/kkt".into());
        keyed.web_key_file = Some("/keys/web-key".into());
        let text = run(&keyed);
        assert!(text.contains("\naccess_key_file=/keys/kkt\n"));
        assert!(text.ends_with("\nweb_key_file=/keys/web-key"));
        let open = run(&config(Limits::DEFAULT, &[]));
        assert!(!open.contains("access_key"));
        assert!(!text.contains("admin_key"));
    }

    #[test]
    fn a_run_records_the_admin_key_file_and_name() {
        let mut keyed = config(Limits::DEFAULT, &[]);
        keyed.admin_key_file = Some("/keys/admin-key".into());
        let text = run(&keyed);
        assert!(text.ends_with("\nadmin_key_file=/keys/admin-key"));
        let without = run(&config(Limits::DEFAULT, &[]));
        assert_ne!(text, without);
        // A name tied to the key shapes the run too: its changes carry it.
        keyed.admin_name = Some("admin-sentinel".into());
        let named = run(&keyed);
        assert!(named.ends_with("\nadmin_key_file=/keys/admin-key\nadmin_name=admin-sentinel"));
    }

    #[test]
    fn different_limits_give_a_different_run() {
        let base = run(&config(Limits::DEFAULT, &[]));
        let other_limits = Limits {
            max_limit: 99,
            ..Limits::DEFAULT
        };
        let changed = run(&config(other_limits, &[]));
        assert_ne!(base, changed);
    }

    #[test]
    fn a_run_records_its_log_file_and_how_it_is_bounded() {
        let mut with_log = config(Limits::DEFAULT, &[]);
        with_log.log_file = Some("/logs/kkt.log".into());
        let text = run(&with_log);
        assert!(
            text.ends_with(
                "\nlog_file=/logs/kkt.log\n\
                 log_max_bytes=10485760\n\
                 log_rotated_files=5\n\
                 log_max_age_ms=2592000000"
            ),
            "{text}"
        );
        let mut other_policy = with_log.clone();
        other_policy.log_policy = LogPolicy {
            max_bytes: 1024 * 1024,
            rotated_files: 2,
            max_age: std::time::Duration::from_secs(60),
        };
        let text = run(&other_policy);
        assert!(
            text.ends_with(
                "\nlog_file=/logs/kkt.log\n\
                 log_max_bytes=1048576\n\
                 log_rotated_files=2\n\
                 log_max_age_ms=60000"
            ),
            "{text}"
        );
        let without = run(&config(Limits::DEFAULT, &[]));
        assert!(!without.contains("log_file"));
        assert!(
            without.contains("\nlog_level=info\ndependency_log_level=warn"),
            "{without}"
        );
    }
}
