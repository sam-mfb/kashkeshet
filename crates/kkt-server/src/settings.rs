//! Settings for the parts this assembly builds, and the order in which a
//! run checks and records settings, including the program's plain values.

use kkt_hosts::AllowedHosts;

/// The settings of what kkt-server builds or passes on. The program owns
/// its database, rules, keys, logging, bind address and backup settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSettings {
    pub formatter: kkt_mcp_formatter::Settings,
    pub web: kkt_web::Settings,
    pub mcp: kkt_mcp::Settings,
    pub allowed_origins: Vec<String>,
    pub allowed_hosts: Vec<String>,
}

/// A setting's refusal, as worded by the owner of its check.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{name} {rule}")]
pub struct SettingError {
    pub name: &'static str,
    pub rule: String,
}

impl ServerSettings {
    pub const DEFAULT: Self = Self {
        formatter: kkt_mcp_formatter::Settings::DEFAULT,
        web: kkt_web::Settings::DEFAULT,
        mcp: kkt_mcp::Settings::DEFAULT,
        allowed_origins: Vec::new(),
        allowed_hosts: Vec::new(),
    };

    /// The first problem in the startup order, or the checked host policy
    /// for Parts::new. Program problems are (setting, owner's check, rule).
    /// All checks stay with their owners; this method only orders them.
    /// Problems outside the known order follow it, in the order supplied.
    pub fn check(
        &self,
        program: impl IntoIterator<Item = (&'static str, &'static str, String)>,
    ) -> Result<AllowedHosts, SettingError> {
        let mut problems: Vec<_> = program.into_iter().collect();
        problems.extend(
            self.formatter
                .problems()
                .into_iter()
                .map(|p| (p.name, p.check, p.rule)),
        );
        problems.extend(
            self.web
                .problems()
                .into_iter()
                .map(|p| (p.name, p.check, p.rule)),
        );
        let hosts = AllowedHosts::new(&self.allowed_hosts);
        if let Err(error) = &hosts {
            problems.push((error.name, "host", error.rule.clone()));
        }
        if let Some(index) = CHECK_ORDER
            .iter()
            .find_map(|key| {
                problems
                    .iter()
                    .position(|(name, check, _)| (*name, *check) == *key)
            })
            .or_else(|| (!problems.is_empty()).then_some(0))
        {
            let (name, _, rule) = problems.swap_remove(index);
            return Err(SettingError { name, rule });
        }
        hosts.map_err(|error| SettingError {
            name: error.name,
            rule: error.rule,
        })
    }

    /// Each setting's recorded line, in the established run-record order.
    /// The program supplies its own lines; origins and hosts are sorted as
    /// before. Unknown program names follow the known names in input order.
    pub fn record_lines(
        &self,
        program: impl IntoIterator<Item = (String, String)>,
    ) -> Vec<(String, String)> {
        let Self {
            formatter,
            web,
            mcp: _,
            allowed_origins,
            allowed_hosts,
        } = self;
        let mut lines: Vec<_> = program.into_iter().collect();
        lines.extend(formatter.record_lines());
        lines.extend(web.record_lines());
        for (name, values) in [
            ("allowed_origin", allowed_origins),
            ("allowed_host", allowed_hosts),
        ] {
            let mut values = values.clone();
            values.sort();
            lines.extend(values.into_iter().map(|value| (name.to_owned(), value)));
        }
        lines.sort_by_key(|(name, _)| {
            RECORD_ORDER
                .iter()
                .position(|known| *known == name)
                .unwrap_or(usize::MAX)
        });
        lines
    }
}

// Some names belong to the program. Naming them keeps today's single
// ordering without making the assembly depend on their owning crates.
const CHECK_ORDER: &[(&str, &str)] = &[
    ("bind", "check_bind"),
    ("allow_any_bind", "check_bind"),
    ("dependency_log_level", "check_dependency_log_level"),
    ("default_limit", "floor"),
    ("max_body_bytes", "floor"),
    ("max_channel_name_chars", "floor"),
    ("max_purpose_chars", "floor"),
    ("max_description_chars", "floor"),
    ("max_note_chars", "floor"),
    ("max_server_announcement_chars", "floor"),
    ("max_memory_entry_chars", "floor"),
    ("max_memory_chars", "floor"),
    ("max_lane_chars", "floor"),
    ("sends_per_minute", "floor"),
    ("registrations_per_minute", "floor"),
    ("max_agents", "floor"),
    ("web_page_messages", "floor"),
    ("web_member_excerpt_chars", "floor"),
    ("upgrade_backups_kept", "floor"),
    ("max_response_bytes", "floor"),
    ("max_limit", "default_limit"),
    ("max_memory_chars", "max_memory_entry_chars"),
    ("web_page_messages", "ceiling"),
    ("web_session_lifetime", "floor"),
    ("waiting_grace", "max_wait"),
    ("max_wait", "default_wait"),
    ("allow_host", "host"),
];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_problem_outside_the_known_order_is_not_lost() {
        let error = ServerSettings::DEFAULT
            .check([("product_setting", "floor", "must be positive".into())])
            .expect_err("program refusal");
        assert_eq!(
            error,
            SettingError {
                name: "product_setting",
                rule: "must be positive".into()
            }
        );
        assert_eq!(error.to_string(), "product_setting must be positive");
    }

    #[test]
    fn check_returns_the_host_policy_for_the_parts() {
        let settings = ServerSettings {
            allowed_hosts: vec!["HOST.example:7420".into()],
            ..ServerSettings::DEFAULT
        };
        let hosts = settings.check([]).expect("checked host policy");
        assert!(hosts.allows("host.example:7420"));
        assert!(!hosts.allows("host.example:7421"));
        assert_eq!(hosts.extra(), settings.allowed_hosts);
    }
}
