//! Whether the installed binary should replace the version the server runs.

use super::ServerVersion;
use std::cmp::Ordering;

/// Why `kkt upgrade` will not replace the server.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UpgradeError {
    /// This machine is a client: its server runs elsewhere.
    #[error("this machine is a client; upgrade the server on the machine where it runs")]
    OnAClient,
    /// This program's own version is not a version.
    #[error("this kkt has an invalid version {target:?}")]
    InvalidVersion { target: String },
    /// The server's version cannot be compared with this program's.
    #[error(
        "cannot compare server version {current:?} with {target}; pass --allow-downgrade to replace it explicitly"
    )]
    Incomparable { current: String, target: String },
    /// This program is older than the server.
    #[error(
        "refusing to downgrade the server from {current} to {target}; pass --allow-downgrade to override"
    )]
    Downgrade { current: String, target: String },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Upgrade {
    Already,
    Proceed,
}

/// Compare semantic versions without exposing the parser's types. Build
/// metadata does not affect precedence. Unknown image tags require the same
/// explicit override as a downgrade, because their ordering is unknowable.
/// A server whose version is not known ([`ServerVersion::NotKnown`]) is
/// always replaced.
pub fn upgrade(
    current: &ServerVersion,
    target: &str,
    allow_downgrade: bool,
) -> Result<Upgrade, UpgradeError> {
    let target_version =
        semver::Version::parse(target).map_err(|_| UpgradeError::InvalidVersion {
            target: target.to_string(),
        })?;
    let current = match current {
        ServerVersion::NotKnown => return Ok(Upgrade::Proceed),
        ServerVersion::Known(current) => current,
    };
    let current_version = match semver::Version::parse(current) {
        Ok(version) => version,
        Err(_) if allow_downgrade => return Ok(Upgrade::Proceed),
        Err(_) => {
            return Err(UpgradeError::Incomparable {
                current: current.clone(),
                target: target.to_string(),
            });
        }
    };
    match target_version.cmp_precedence(&current_version) {
        Ordering::Equal => Ok(Upgrade::Already),
        Ordering::Greater => Ok(Upgrade::Proceed),
        Ordering::Less if allow_downgrade => Ok(Upgrade::Proceed),
        Ordering::Less => Err(UpgradeError::Downgrade {
            current: current.clone(),
            target: target.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(version: &str) -> ServerVersion {
        ServerVersion::Known(version.into())
    }

    #[test]
    fn same_version_including_build_metadata_does_nothing() {
        for (current, target) in [("0.4.0", "0.4.0"), ("0.4.0+one", "0.4.0+two")] {
            assert_eq!(
                upgrade(&known(current), target, false).unwrap(),
                Upgrade::Already
            );
            assert_eq!(
                upgrade(&known(current), target, true).unwrap(),
                Upgrade::Already
            );
        }
    }

    #[test]
    fn upgrades_use_numeric_and_prerelease_precedence() {
        for (current, target) in [
            ("0.9.0", "0.10.0"),
            ("0.4.0-rc.9", "0.4.0-rc.10"),
            ("0.4.0-rc.1", "0.4.0"),
        ] {
            assert_eq!(
                upgrade(&known(current), target, false).unwrap(),
                Upgrade::Proceed
            );
            assert!(
                upgrade(&known(target), current, false)
                    .unwrap_err()
                    .to_string()
                    .contains("--allow-downgrade")
            );
            assert_eq!(
                upgrade(&known(target), current, true).unwrap(),
                Upgrade::Proceed
            );
        }
    }

    #[test]
    fn unknown_tags_require_override_and_invalid_target_is_always_refused() {
        assert!(
            upgrade(&known("local-test"), "0.4.0", false)
                .unwrap_err()
                .to_string()
                .contains("cannot compare")
        );
        assert_eq!(
            upgrade(&known("local-test"), "0.4.0", true).unwrap(),
            Upgrade::Proceed
        );
        assert!(upgrade(&known("0.4.0"), "invalid", true).is_err());
    }

    #[test]
    fn a_service_of_unknown_version_is_always_replaced() {
        assert_eq!(
            upgrade(&ServerVersion::NotKnown, "0.5.1", false).unwrap(),
            Upgrade::Proceed
        );
        assert!(upgrade(&ServerVersion::NotKnown, "invalid", false).is_err());
    }
}
