//! Settings for the parts this assembly builds: their problems, the checked
//! snapshot `Parts::new` takes, and their run-record lines.

use kkt_hosts::AllowedHosts;

/// The settings of what kkt-server builds or passes on. The program owns
/// its database, rules, keys, logging, bind address and backup settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSettings {
    pub formatter: kkt_mcp::result_format::Settings,
    pub web: kkt_web::Settings,
    pub mcp: kkt_mcp::Settings,
    pub allowed_origins: Vec<String>,
    pub allowed_hosts: Vec<String>,
}

/// An immutable snapshot whose settings and host policy passed the startup
/// checks together. Only `ServerSettings::check` constructs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedSettings {
    settings: ServerSettings,
    hosts: AllowedHosts,
}

impl CheckedSettings {
    /// The checked values, for the program's run record.
    pub fn settings(&self) -> &ServerSettings {
        &self.settings
    }

    pub(crate) fn into_parts(self) -> (ServerSettings, AllowedHosts) {
        (self.settings, self.hosts)
    }
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
        formatter: kkt_mcp::result_format::Settings::DEFAULT,
        web: kkt_web::Settings::DEFAULT,
        mcp: kkt_mcp::Settings::DEFAULT,
        allowed_origins: Vec::new(),
        allowed_hosts: Vec::new(),
    };

    /// The problems of these settings, each as (setting, owner's check,
    /// rule): the result formatter's, the web page's and the host list's.
    /// The program orders them with its own (see [`ServerSettings::check`]).
    pub fn problems(&self) -> Vec<(&'static str, &'static str, String)> {
        let mut problems: Vec<_> = self
            .formatter
            .problems()
            .into_iter()
            .map(|p| (p.name, p.check, p.rule))
            .collect();
        problems.extend(
            self.web
                .problems()
                .into_iter()
                .map(|p| (p.name, p.check, p.rule)),
        );
        if let Err(error) = AllowedHosts::new(&self.allowed_hosts) {
            problems.push((error.name, "host", error.rule));
        }
        problems
    }

    /// One checked snapshot for `Parts::new`, or the first problem: of
    /// `program`'s, each (setting, owner's check, rule), in the order given,
    /// and then of [`ServerSettings::problems`]. The order is the
    /// program's: a program that orders its checks with these settings'
    /// problems puts them in order first, or passes none.
    pub fn check(
        self,
        program: impl IntoIterator<Item = (&'static str, &'static str, String)>,
    ) -> Result<CheckedSettings, SettingError> {
        let problems = program.into_iter().chain(self.problems());
        if let Some((name, _, rule)) = problems.into_iter().next() {
            return Err(SettingError { name, rule });
        }
        let hosts = AllowedHosts::new(&self.allowed_hosts).map_err(|error| SettingError {
            name: error.name,
            rule: error.rule,
        })?;
        Ok(CheckedSettings {
            settings: self,
            hosts,
        })
    }

    /// `program`'s lines as given, then these settings' recorded lines: the
    /// result formatter's, the web page's, then the allowed origins and
    /// hosts, each sorted. The run record's order is the program's: it puts
    /// these lines in it.
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
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_problems_are_each_owners_and_the_host_lists() {
        let settings = ServerSettings {
            allowed_hosts: vec!["bad host/".into()],
            ..ServerSettings::DEFAULT
        };
        let problems = settings.problems();
        assert!(
            problems.iter().any(|(_, check, _)| *check == "host"),
            "{problems:?}"
        );
        assert!(ServerSettings::DEFAULT.problems().is_empty());
        let error = settings.check([]).expect_err("a bad host");
        assert_eq!(error.name, problems[0].0);
    }

    #[test]
    fn check_keeps_the_values_and_host_policy_together() {
        let settings = ServerSettings {
            allowed_hosts: vec!["HOST.example:7420".into()],
            ..ServerSettings::DEFAULT
        };
        let expected = settings.allowed_hosts.clone();
        let checked = settings.check([]).expect("checked snapshot");
        assert_eq!(checked.settings().allowed_hosts, expected);
        let (settings, hosts) = checked.into_parts();
        assert!(hosts.allows("host.example:7420"));
        assert!(!hosts.allows("host.example:7421"));
        assert_eq!(hosts.extra(), settings.allowed_hosts);
    }

    #[test]
    fn origins_and_hosts_are_recorded_sorted_after_the_owners_lines() {
        let settings = ServerSettings {
            allowed_origins: vec!["http://b.example".into(), "http://a.example".into()],
            allowed_hosts: vec!["b.example".into(), "a.example".into()],
            ..ServerSettings::DEFAULT
        };
        let lines = settings.record_lines([]);
        let tail: Vec<_> = lines[lines.len() - 4..]
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        assert_eq!(
            tail,
            [
                "allowed_origin=http://a.example",
                "allowed_origin=http://b.example",
                "allowed_host=a.example",
                "allowed_host=b.example"
            ]
        );
    }
}
