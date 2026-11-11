use super::*;

/// Something an operator may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    /// See every channel, hidden ones included, every avatar and their
    /// memory, and the server's records.
    Read,
    /// Change channels, avatars, lanes and the server announcement as the
    /// operator, each change announced or recorded as the operator's.
    Change,
    /// Issue a new recovery code for an avatar.
    IssueRecoveryCodes,
}

/// The permissions an operator holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permissions(BTreeSet<Permission>);

impl Permissions {
    /// Exactly the permissions listed.
    pub fn of(list: &[Permission]) -> Permissions {
        Permissions(list.iter().copied().collect())
    }

    pub fn has(&self, permission: Permission) -> bool {
        self.0.contains(&permission)
    }
}

/// Where an operator's request came from: recorded with what it changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Audit {
    /// The human, on the web page.
    WebPage,
    /// An admin agent at the admin endpoint, under the name tied to its
    /// admin key, if the key has one.
    AdminEndpoint { key_name: Option<String> },
}

impl Audit {
    /// What is stored as the author of a change the operator makes, where
    /// the store keeps one (`saved_by_admin`, `retired_by_admin`): the
    /// admin key's name, or `None` for the web page, as kkt has always
    /// stored them.
    pub fn author(&self) -> Option<String> {
        match self {
            Audit::WebPage => None,
            Audit::AdminEndpoint { key_name } => key_name.clone(),
        }
    }
}

/// The operator, as one caller: what it may do and where it asks from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operator {
    pub permissions: Permissions,
    pub audit: Audit,
}

impl Operator {
    pub fn new(permissions: Permissions, audit: Audit) -> Operator {
        Operator { permissions, audit }
    }

    /// Refuses, as `unauthenticated`, an operator without `permission`.
    /// The refusal is marked [`Refusal::LacksPermission`], so the edge that
    /// admitted the operator can say how to get it.
    pub fn may(&self, permission: Permission) -> Result<()> {
        if self.permissions.has(permission) {
            return Ok(());
        }
        let what = match permission {
            Permission::Read => "read",
            Permission::Change => "make changes",
            Permission::IssueRecoveryCodes => "issue recovery codes",
        };
        Err(RuleError::new(
            Code::Unauthenticated,
            format!("this operator may not {what}"),
        )
        .because(Refusal::LacksPermission(permission)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_hold_exactly_what_was_listed() {
        let read_only = Permissions::of(&[Permission::Read]);
        assert!(read_only.has(Permission::Read));
        assert!(!read_only.has(Permission::Change));
        assert!(!read_only.has(Permission::IssueRecoveryCodes));
        let none = Permissions::of(&[]);
        assert!(!none.has(Permission::Read));
    }

    #[test]
    fn the_author_is_the_admin_keys_name_or_none() {
        assert_eq!(Audit::WebPage.author(), None);
        assert_eq!(
            Audit::AdminEndpoint {
                key_name: Some("admin-sentinel".into())
            }
            .author(),
            Some("admin-sentinel".to_string())
        );
        assert_eq!(Audit::AdminEndpoint { key_name: None }.author(), None);
    }
}
