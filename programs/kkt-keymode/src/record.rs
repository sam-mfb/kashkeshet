//! The program's run-record values. Owners word their own settings;
//! kkt-server orders these plain lines with its own settings' lines.

use kkt_logs::{LogLevel, LogPolicy};
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
    pub log_file: Option<String>,
    pub log_policy: LogPolicy,
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
        server.record_lines(lines)
    }
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
