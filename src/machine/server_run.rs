//! The program's run-record values. Owners word their own settings;
//! this program puts them, with kkt-server's, in the run record's order. The
//! integration-test harness uses this same program mapping.

use kkt_log_tracing::LogLevel;
use kkt_provenance::ServerRun;
use kkt_rolling_file::LogPolicy;
use kkt_server::ServerSettings;

pub struct RecordedSettings<'a> {
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
    pub log_file: Option<String>,
    pub log_policy: LogPolicy,
}

impl RecordedSettings<'_> {
    pub fn lines(&self) -> Vec<(String, String)> {
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
            log_file,
            log_policy,
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
        if let Some(file) = log_file {
            let LogPolicy {
                max_bytes,
                rotated_files,
                max_age,
            } = log_policy;
            lines.extend([
                line("log_file", file),
                line("log_max_bytes", max_bytes),
                line("log_rotated_files", rotated_files),
                line("log_max_age_ms", max_age.as_millis()),
            ]);
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
pub fn new_server_run(run: &ServerRun) -> kkt_rules::NewServerRun {
    kkt_rules::NewServerRun {
        version: run.version.clone(),
        git_commit: run.commit.clone(),
        settings: run.settings.clone(),
        instructions_sha256: run.instructions_sha256.clone(),
    }
}

#[cfg(test)]
mod golden;
