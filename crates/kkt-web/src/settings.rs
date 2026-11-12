//! The page's own settings.

use std::time::Duration;

/// Largest `web_page_messages`. More would make a page slow to load and
/// heavy to keep live.
pub const MAX_WEB_PAGE_MESSAGES: usize = 5000;

/// How much the web page shows at once, how it keeps its event stream
/// open, and how long a sign-in lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// How much the page shows at once.
    pub page: PageLimits,
    /// How often the web page's event stream sends a keep-alive comment.
    pub keep_alive: Duration,
    /// How long a web page sign-in lasts, when sign-in is required. Read by
    /// whoever makes the page's [`Sessions`](crate::Sessions).
    pub session_lifetime: Duration,
}

impl Settings {
    pub const DEFAULT: Settings = Settings {
        page: PageLimits::DEFAULT,
        keep_alive: Duration::from_secs(15),
        session_lifetime: Duration::from_secs(12 * 60 * 60),
    };

    /// Every invalid setting, with the check that found it. The caller
    /// chooses which problem to report alongside other owners' problems.
    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for (name, value) in [
            ("web_page_messages", self.page.page_messages),
            ("web_member_excerpt_chars", self.page.member_excerpt_chars),
        ] {
            if value == 0 {
                problems.push(Problem {
                    name,
                    check: "floor",
                    rule: "must be at least 1".into(),
                });
            }
        }
        if self.page.page_messages > MAX_WEB_PAGE_MESSAGES {
            problems.push(Problem {
                name: "web_page_messages",
                check: "ceiling",
                rule: format!("must be at most {MAX_WEB_PAGE_MESSAGES}"),
            });
        }
        if self.session_lifetime < Duration::from_secs(1) {
            problems.push(Problem {
                name: "web_session_lifetime",
                check: "floor",
                rule: "must be at least 1 second".into(),
            });
        }
        problems
    }

    /// The page's operator settings as the run record words them, with
    /// the session lifetime in milliseconds. Keep-alive is not recorded.
    pub fn record_lines(&self) -> Vec<(String, String)> {
        let Self {
            page:
                PageLimits {
                    page_messages,
                    member_excerpt_chars,
                },
            keep_alive: _,
            session_lifetime,
        } = self;
        vec![
            (
                "web_session_lifetime_ms".into(),
                session_lifetime.as_millis().to_string(),
            ),
            ("web_page_messages".into(), page_messages.to_string()),
            (
                "web_member_excerpt_chars".into(),
                member_excerpt_chars.to_string(),
            ),
        ]
    }
}

/// An invalid page setting and the owner's check that found it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{name} {rule}")]
pub struct Problem {
    pub name: &'static str,
    pub check: &'static str,
    pub rule: String,
}

/// The limits only the page applies: how many messages a page shows and
/// how much of a member's latest message. The limits the application
/// enforces, such as the announcement's and lanes' sizes, are its own
/// (`Api::limits`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageLimits {
    /// The most messages one web page shows at first: a channel's page, and
    /// the live view, which drops the oldest as new ones arrive. A channel
    /// page's "Show previous" button loads this many earlier ones at a time.
    pub page_messages: usize,
    /// Characters of each member's latest message that a channel's web page
    /// shows in its member list, as an excerpt linking to the whole message.
    pub member_excerpt_chars: usize,
}

impl PageLimits {
    pub const DEFAULT: PageLimits = PageLimits {
        page_messages: 50,
        member_excerpt_chars: 80,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_settings_keep_the_existing_bounds_and_refusals() {
        type Change = fn(&mut Settings);
        let cases: [(Change, &str, &str, &str); 4] = [
            (
                |s| s.page.page_messages = 0,
                "web_page_messages",
                "floor",
                "must be at least 1",
            ),
            (
                |s| s.page.member_excerpt_chars = 0,
                "web_member_excerpt_chars",
                "floor",
                "must be at least 1",
            ),
            (
                |s| s.page.page_messages = MAX_WEB_PAGE_MESSAGES + 1,
                "web_page_messages",
                "ceiling",
                "must be at most 5000",
            ),
            (
                |s| s.session_lifetime = Duration::from_millis(999),
                "web_session_lifetime",
                "floor",
                "must be at least 1 second",
            ),
        ];
        for (change, name, check, rule) in cases {
            let mut settings = Settings::DEFAULT;
            change(&mut settings);
            let problems = settings.problems();
            assert_eq!(
                problems,
                vec![Problem {
                    name,
                    check,
                    rule: rule.into()
                }]
            );
            assert_eq!(problems[0].to_string(), format!("{name} {rule}"));
        }
        assert!(Settings::DEFAULT.problems().is_empty());
        let settings = Settings {
            page: PageLimits {
                page_messages: MAX_WEB_PAGE_MESSAGES,
                member_excerpt_chars: 1,
            },
            session_lifetime: Duration::from_secs(1),
            ..Settings::DEFAULT
        };
        assert!(settings.problems().is_empty());
        let settings = Settings {
            page: PageLimits {
                page_messages: 1,
                ..settings.page
            },
            session_lifetime: Duration::from_nanos(999_999_999),
            ..settings
        };
        assert_eq!(settings.problems()[0].name, "web_session_lifetime");
    }

    #[test]
    fn every_page_problem_is_collected() {
        let settings = Settings {
            page: PageLimits {
                page_messages: MAX_WEB_PAGE_MESSAGES + 1,
                member_excerpt_chars: 0,
            },
            session_lifetime: Duration::ZERO,
            ..Settings::DEFAULT
        };
        assert_eq!(
            settings.problems(),
            vec![
                Problem {
                    name: "web_member_excerpt_chars",
                    check: "floor",
                    rule: "must be at least 1".into()
                },
                Problem {
                    name: "web_page_messages",
                    check: "ceiling",
                    rule: "must be at most 5000".into()
                },
                Problem {
                    name: "web_session_lifetime",
                    check: "floor",
                    rule: "must be at least 1 second".into()
                },
            ]
        );
        let settings = Settings {
            page: PageLimits {
                page_messages: 0,
                member_excerpt_chars: 0,
            },
            ..Settings::DEFAULT
        };
        assert_eq!(
            settings
                .problems()
                .iter()
                .map(|p| (p.name, p.check))
                .collect::<Vec<_>>(),
            vec![
                ("web_page_messages", "floor"),
                ("web_member_excerpt_chars", "floor")
            ]
        );
    }

    #[test]
    fn page_settings_keep_the_run_records_names_units_and_values() {
        assert_eq!(
            Settings::DEFAULT.record_lines(),
            vec![
                ("web_session_lifetime_ms".into(), "43200000".into()),
                ("web_page_messages".into(), "50".into()),
                ("web_member_excerpt_chars".into(), "80".into()),
            ]
        );
        let settings = Settings {
            page: PageLimits {
                page_messages: 11,
                member_excerpt_chars: 23,
            },
            keep_alive: Duration::from_secs(999),
            session_lifetime: Duration::from_nanos(1_234_567_890),
        };
        assert_eq!(
            settings.record_lines(),
            vec![
                ("web_session_lifetime_ms".into(), "1234".into()),
                ("web_page_messages".into(), "11".into()),
                ("web_member_excerpt_chars".into(), "23".into()),
            ]
        );
    }
}
