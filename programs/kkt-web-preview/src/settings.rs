//! The preview's flags, checked: what it listens on, its seed, and the
//! settings it hands the simulated application and the page.

use kkt_api_sim::PreviewSettings;
use kkt_web::{MAX_WEB_PAGE_MESSAGES, PageLimits};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

/// Address the preview listens on: next to the server's default port.
pub const PREVIEW_BIND: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 7421));
/// Seed for the simulated data when none is given.
pub const PREVIEW_SEED: u64 = 1;

/// Why the preview's flags are refused: the flag's name and the rule it
/// breaks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingError {
    pub name: &'static str,
    pub rule: String,
}

impl std::fmt::Display for SettingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.name, self.rule)
    }
}

impl From<kkt_api_sim::SettingError> for SettingError {
    fn from(e: kkt_api_sim::SettingError) -> SettingError {
        SettingError {
            name: e.name,
            rule: e.rule,
        }
    }
}

/// The simulated feed's settings, `rate` steps a minute, checked.
pub fn simulation(rate: u32) -> Result<PreviewSettings, SettingError> {
    let settings = PreviewSettings {
        steps_per_minute: rate,
    };
    settings.check()?;
    Ok(settings)
}

/// The page's limits, showing at most `page_messages` messages a page,
/// checked as the server checks `web_page_messages`.
pub fn page(page_messages: usize) -> Result<PageLimits, SettingError> {
    if page_messages == 0 || page_messages > MAX_WEB_PAGE_MESSAGES {
        return Err(SettingError {
            name: "page_messages",
            rule: format!("must be from 1 to {MAX_WEB_PAGE_MESSAGES}"),
        });
    }
    Ok(PageLimits {
        page_messages,
        ..PageLimits::DEFAULT
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_must_be_sensible() {
        assert_eq!(
            simulation(PreviewSettings::DEFAULT.steps_per_minute),
            Ok(PreviewSettings::DEFAULT)
        );
        for bad in [0, 60_001] {
            assert_eq!(simulation(bad).map_err(|e| e.name), Err("rate"));
        }
    }

    #[test]
    fn page_sizes_must_be_sensible() {
        assert_eq!(
            page(PageLimits::DEFAULT.page_messages),
            Ok(PageLimits::DEFAULT)
        );
        assert_eq!(
            page(MAX_WEB_PAGE_MESSAGES).map(|p| p.page_messages),
            Ok(MAX_WEB_PAGE_MESSAGES)
        );
        for bad in [0, MAX_WEB_PAGE_MESSAGES + 1] {
            assert_eq!(page(bad).map_err(|e| e.name), Err("page_messages"));
        }
    }
}
