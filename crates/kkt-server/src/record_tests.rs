//! The existing run-record golden and ordering tests, with each owner's
//! settings supplied separately and program values passed as plain lines.

use crate::ServerSettings;
use kkt_logs::{LogLevel, LogPolicy};
mod golden;
mod program;

#[derive(Clone, Debug)]
struct FixtureLimits {
    rules: kkt_rules::Settings,
    server: ServerSettings,
    backup: kkt_backup::Settings,
}

impl FixtureLimits {
    const DEFAULT: Self = Self {
        rules: kkt_rules::Settings::DEFAULT,
        server: ServerSettings::DEFAULT,
        backup: kkt_backup::Settings::DEFAULT,
    };
}

#[derive(Clone, Debug)]
struct RecordedSettings {
    /// Whether the server may listen beyond loopback.
    pub allow_any_bind: bool,
    pub limits: FixtureLimits,
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
    fn lines(&self) -> Vec<(String, String)> {
        let mut server = self.limits.server.clone();
        server.allowed_origins = self.allowed_origins.clone();
        server.allowed_hosts = self.allowed_hosts.clone();
        program::RecordedSettings {
            server: &server,
            rules: self.limits.rules.clone(),
            backup: self.limits.backup,
            allow_any_bind: self.allow_any_bind,
            log_level: self.log_level,
            dependency_log_level: self.dependency_log_level,
            config_file: self.config_file.clone(),
            access_key_file: self.access_key_file.clone(),
            web_key_file: self.web_key_file.clone(),
            admin_key_file: self.admin_key_file.clone(),
            admin_name: self.admin_name.clone(),
            log_file: self.log_file.clone(),
            log_policy: self.log_policy,
        }
        .lines()
    }
}

// The original tests of just the limit portion leave out allow_any_bind.
fn limit_lines(
    limits: &FixtureLimits,
    log_level: LogLevel,
    dependency_log_level: LogLevel,
    allowed_origins: &[String],
    allowed_hosts: &[String],
    config_file: Option<&str>,
    access_key_file: Option<&str>,
) -> Vec<(String, String)> {
    RecordedSettings {
        allow_any_bind: false,
        limits: limits.clone(),
        log_level,
        dependency_log_level,
        allowed_origins: allowed_origins.to_vec(),
        allowed_hosts: allowed_hosts.to_vec(),
        config_file: config_file.map(str::to_owned),
        access_key_file: access_key_file.map(str::to_owned),
        web_key_file: None,
        admin_key_file: None,
        admin_name: None,
        log_file: None,
        log_policy: LogPolicy::DEFAULT,
    }
    .lines()
    .into_iter()
    .filter(|(name, _)| name != "allow_any_bind")
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_provenance::{Build, RunSettings, ServerRun};

    const TEST_BUILD: Build = Build {
        version: "9.9.9",
        commit: "abc123",
    };

    fn config(limits: FixtureLimits, origins: &[&str]) -> RecordedSettings {
        RecordedSettings {
            allow_any_bind: false,
            limits,
            log_level: LogLevel::Info,
            dependency_log_level: LogLevel::Warn,
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
        limits: &FixtureLimits,
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
            run(&config(FixtureLimits::DEFAULT, &[])),
            format!(
                "allow_any_bind=false\n{}",
                settings_text(
                    &FixtureLimits::DEFAULT,
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
                &FixtureLimits::DEFAULT,
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
            &FixtureLimits::DEFAULT,
            LogLevel::Info,
            LogLevel::Warn,
            &["http://b".into(), "http://a".into()],
            &[],
            None,
            None,
        );
        let b = settings_text(
            &FixtureLimits::DEFAULT,
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
        let mut from_file = config(FixtureLimits::DEFAULT, &[]);
        from_file.config_file = Some("/home/sam/.config/kkt/config.toml".into());
        from_file.access_key_file = Some("/keys/kkt".into());
        assert!(run(&from_file).ends_with(
            "\nconfig_file=/home/sam/.config/kkt/config.toml\naccess_key_file=/keys/kkt"
        ));
        let plain = run(&config(FixtureLimits::DEFAULT, &[]));
        assert!(!plain.contains("config_file"));
    }

    #[test]
    fn a_run_records_whether_any_address_may_be_bound() {
        let mut open = config(FixtureLimits::DEFAULT, &[]);
        let closed = run(&open);
        assert!(closed.starts_with("allow_any_bind=false\n"));
        open.allow_any_bind = true;
        let allowed = run(&open);
        assert!(allowed.starts_with("allow_any_bind=true\n"));
    }

    #[test]
    fn host_order_does_not_change_the_text() {
        let a = settings_text(
            &FixtureLimits::DEFAULT,
            LogLevel::Info,
            LogLevel::Warn,
            &[],
            &["b.example".into(), "a.example".into()],
            None,
            None,
        );
        let b = settings_text(
            &FixtureLimits::DEFAULT,
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
        let base = run(&config(FixtureLimits::DEFAULT, &[]));
        let mut with_host = config(FixtureLimits::DEFAULT, &[]);
        with_host.allowed_hosts = vec!["host.docker.internal".into()];
        let changed = run(&with_host);
        assert_ne!(base, changed);
        assert!(changed.ends_with("\nallowed_host=host.docker.internal"));
    }

    #[test]
    fn a_run_records_the_key_files() {
        let mut keyed = config(FixtureLimits::DEFAULT, &[]);
        keyed.access_key_file = Some("/keys/kkt".into());
        keyed.web_key_file = Some("/keys/web-key".into());
        let text = run(&keyed);
        assert!(text.contains("\naccess_key_file=/keys/kkt\n"));
        assert!(text.ends_with("\nweb_key_file=/keys/web-key"));
        let open = run(&config(FixtureLimits::DEFAULT, &[]));
        assert!(!open.contains("access_key"));
        assert!(!text.contains("admin_key"));
    }

    #[test]
    fn a_run_records_the_admin_key_file_and_name() {
        let mut keyed = config(FixtureLimits::DEFAULT, &[]);
        keyed.admin_key_file = Some("/keys/admin-key".into());
        let text = run(&keyed);
        assert!(text.ends_with("\nadmin_key_file=/keys/admin-key"));
        let without = run(&config(FixtureLimits::DEFAULT, &[]));
        assert_ne!(text, without);
        // A name tied to the key shapes the run too: its changes carry it.
        keyed.admin_name = Some("admin-sentinel".into());
        let named = run(&keyed);
        assert!(named.ends_with("\nadmin_key_file=/keys/admin-key\nadmin_name=admin-sentinel"));
    }

    #[test]
    fn different_limits_give_a_different_run() {
        let base = run(&config(FixtureLimits::DEFAULT, &[]));
        let other_limits = FixtureLimits {
            rules: kkt_rules::Settings {
                max_limit: 99,
                ..kkt_rules::Settings::DEFAULT
            },
            ..FixtureLimits::DEFAULT
        };
        let changed = run(&config(other_limits, &[]));
        assert_ne!(base, changed);
    }

    #[test]
    fn a_run_records_its_log_file_and_how_it_is_bounded() {
        let mut with_log = config(FixtureLimits::DEFAULT, &[]);
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
        let without = run(&config(FixtureLimits::DEFAULT, &[]));
        assert!(!without.contains("log_file"));
        assert!(
            without.contains("\nlog_level=info\ndependency_log_level=warn"),
            "{without}"
        );
    }
}
