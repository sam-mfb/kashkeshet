//! The program's run-record values. Owners word their own settings;
//! this program puts them, with kkt-server's, in the run record's order.
//! kkt-keymode writes no log file, so it records none.

use kkt_log_tracing::LogLevel;
use kkt_provenance::ServerRun;
use kkt_server::ServerSettings;

pub(crate) struct RecordedSettings<'a> {
    pub server: &'a ServerSettings,
    pub rules: kkt_rules::Settings,
    pub backup: kkt_backup::Settings,
    pub allow_any_bind: bool,
    pub log_level: LogLevel,
    pub dependency_log_level: LogLevel,
    pub config_file: Option<String>,
    pub access_key_file: Option<String>,
    pub web_key_file: Option<String>,
    pub admin_key_file: Option<String>,
    pub admin_name: Option<String>,
}

impl RecordedSettings<'_> {
    pub(crate) fn lines(&self) -> Vec<(String, String)> {
        let Self {
            server,
            rules,
            backup,
            allow_any_bind,
            log_level,
            dependency_log_level,
            config_file,
            access_key_file,
            web_key_file,
            admin_key_file,
            admin_name,
        } = self;
        let mut lines = rules.record_lines();
        lines.extend(backup.record_lines());
        lines.extend([
            line("allow_any_bind", allow_any_bind),
            line("log_level", log_level.name()),
            line("dependency_log_level", dependency_log_level.name()),
        ]);
        lines.extend(config_file.iter().map(|file| line("config_file", file)));
        lines.extend(
            access_key_file
                .iter()
                .map(|file| line("access_key_file", file)),
        );
        lines.extend(web_key_file.iter().map(|file| line("web_key_file", file)));
        if let Some(file) = admin_key_file {
            lines.push(line("admin_key_file", file));
            lines.extend(admin_name.iter().map(|name| line("admin_name", name)));
        }
        lines.extend(server.record_lines());
        in_record_order(lines)
    }
}

/// The run record's order: each setting's lines come where its name is in
/// this list, and a name the list does not know comes after every one it
/// does, in the order given.
const RECORD_ORDER: &[&str] = &[
    "allow_any_bind",
    "name_pack",
    "max_response_bytes",
    "default_limit",
    "max_limit",
    "max_body_bytes",
    "max_channel_name_chars",
    "max_purpose_chars",
    "max_description_chars",
    "max_note_chars",
    "max_server_announcement_chars",
    "max_memory_entry_chars",
    "max_memory_chars",
    "max_lane_chars",
    "sends_per_minute",
    "registrations_per_minute",
    "max_agents",
    "default_wait_ms",
    "max_wait_ms",
    "waiting_grace_ms",
    "member_preview",
    "purpose_preview_chars",
    "web_session_lifetime_ms",
    "web_page_messages",
    "web_member_excerpt_chars",
    "upgrade_backups_kept",
    "log_level",
    "dependency_log_level",
    "allowed_origin",
    "allowed_host",
    "config_file",
    "access_key_file",
    "web_key_file",
    "admin_key_file",
    "admin_name",
    "log_file",
    "log_max_bytes",
    "log_rotated_files",
    "log_max_age_ms",
];

/// `lines` in [`RECORD_ORDER`]; lines of one name keep their order.
fn in_record_order(mut lines: Vec<(String, String)>) -> Vec<(String, String)> {
    lines.sort_by_key(|(name, _)| {
        RECORD_ORDER
            .iter()
            .position(|known| *known == name)
            .unwrap_or(RECORD_ORDER.len())
    });
    lines
}

fn line(name: &str, value: impl std::fmt::Display) -> (String, String) {
    (name.to_owned(), value.to_string())
}

/// What produced this run, as the rules record it at startup.
pub(crate) fn new_server_run(run: &ServerRun) -> kkt_rules::NewServerRun {
    kkt_rules::NewServerRun {
        version: run.version.clone(),
        git_commit: run.commit.clone(),
        settings: run.settings.clone(),
        instructions_sha256: run.instructions_sha256.clone(),
    }
}

#[cfg(test)]
mod tests {
    //! The run record's settings text, pinned byte for byte through kkt-keymode's
    //! own lines, which are kkt's without a log file: a plain server, and one with every setting
    //! given, whose text fixes the place of every name. A changed text
    //! changes what every existing install records.

    use super::*;
    use std::time::Duration;

    fn plain(server: &ServerSettings) -> RecordedSettings<'_> {
        RecordedSettings {
            server,
            rules: kkt_rules::Settings::DEFAULT,
            backup: kkt_backup::Settings::DEFAULT,
            allow_any_bind: false,
            log_level: LogLevel::Info,
            dependency_log_level: LogLevel::Warn,
            config_file: None,
            access_key_file: None,
            web_key_file: None,
            admin_key_file: None,
            admin_name: None,
        }
    }

    fn text(settings: &RecordedSettings<'_>) -> String {
        settings
            .lines()
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    const DEFAULT_LIMITS: &str = "name_pack=animals\n\
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
         upgrade_backups_kept=5";

    #[test]
    fn a_plain_server_records_the_defaults() {
        let server = ServerSettings::DEFAULT;
        assert_eq!(
            text(&plain(&server)),
            format!(
                "allow_any_bind=false\n{DEFAULT_LIMITS}\nlog_level=info\ndependency_log_level=warn"
            )
        );
    }

    #[test]
    fn a_server_with_every_setting_given_records_them_all_in_order() {
        let server = ServerSettings {
            allowed_origins: vec!["http://b.example".into(), "http://a.example".into()],
            allowed_hosts: vec!["b.example".into(), "a.example".into()],
            ..ServerSettings::DEFAULT
        };
        let settings = RecordedSettings {
            rules: kkt_rules::Settings {
                max_limit: 99,
                default_wait: Duration::from_secs(12),
                ..kkt_rules::Settings::DEFAULT
            },
            allow_any_bind: true,
            log_level: LogLevel::Debug,
            dependency_log_level: LogLevel::Error,
            config_file: Some("/home/sam/.config/kkt/config.toml".into()),
            access_key_file: Some("/keys/access-key".into()),
            web_key_file: Some("/keys/web-key".into()),
            admin_key_file: Some("/keys/admin-key".into()),
            admin_name: Some("admin-sentinel".into()),
            ..plain(&server)
        };
        assert_eq!(
            text(&settings),
            "allow_any_bind=true\n\
             name_pack=animals\n\
             max_response_bytes=8192\n\
             default_limit=20\n\
             max_limit=99\n\
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
             default_wait_ms=12000\n\
             max_wait_ms=50000\n\
             waiting_grace_ms=3000\n\
             member_preview=3\n\
             purpose_preview_chars=120\n\
             web_session_lifetime_ms=43200000\n\
             web_page_messages=50\n\
             web_member_excerpt_chars=80\n\
             upgrade_backups_kept=5\n\
             log_level=debug\n\
             dependency_log_level=error\n\
             allowed_origin=http://a.example\n\
             allowed_origin=http://b.example\n\
             allowed_host=a.example\n\
             allowed_host=b.example\n\
             config_file=/home/sam/.config/kkt/config.toml\n\
             access_key_file=/keys/access-key\n\
             web_key_file=/keys/web-key\n\
             admin_key_file=/keys/admin-key\n\
             admin_name=admin-sentinel"
        );
    }
}
