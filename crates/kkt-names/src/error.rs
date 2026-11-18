//! Which rule a name breaks, worded for the agent or the person who gave it.

use crate::{ADMIN_PREFIX, OPERATOR};

/// The kind of name a check was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    /// A handle an agent proposes for its avatar.
    Handle,
    /// A lane's name.
    Lane,
    /// The name of a memory entry.
    MemoryEntry,
    /// An admin agent's name.
    AdminName,
}

/// A rule a name can break.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The name is not the characters its kind allows, in words joined by
    /// single hyphens: `got` is the name as checked.
    Words { got: String },
    /// The name is longer than `max` characters.
    TooLong { max: usize },
    /// The handle is the operator's, or starts as an admin's does.
    Reserved,
}

/// A name that breaks one of its kind's rules.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", sentence(*of, broke))]
pub struct NameError {
    pub of: Named,
    pub broke: Rule,
}

impl NameError {
    pub(crate) fn new(of: Named, broke: Rule) -> Self {
        NameError { of, broke }
    }
}

fn sentence(of: Named, broke: &Rule) -> String {
    match (of, broke) {
        (Named::Handle, Rule::Words { got }) => format!(
            "the handle must be letters, digits and underscores, in words joined by single hyphens, such as `api-reviewer-2` or `Api_Reviewer-2`; got {got:?}"
        ),
        (Named::Lane, Rule::Words { got }) => format!(
            "the lane name must be lowercase letters and digits, in words joined by single hyphens, such as `orchestrator` or `code-reviewer`; got {got:?}"
        ),
        (Named::MemoryEntry, Rule::Words { got }) => format!(
            "the entry name must be lowercase letters and digits, in words joined by single hyphens, such as `handoff` or `open-questions`; got {got:?}"
        ),
        (Named::AdminName, Rule::Words { got }) => format!(
            "the admin name must be lowercase letters and digits, in words joined by single hyphens, such as `sentinel` (which becomes `{ADMIN_PREFIX}sentinel`); got {got:?}"
        ),
        (Named::AdminName, Rule::TooLong { max }) => {
            format!("the admin name must be at most {max} characters, `{ADMIN_PREFIX}` included")
        }
        (of, Rule::TooLong { max }) => format!("{} must be at most {max} characters", the(of)),
        (_, Rule::Reserved) => {
            format!("`{OPERATOR}` and handles starting `{ADMIN_PREFIX}` are reserved, in any case")
        }
    }
}

/// The name's kind as a sentence begins with it.
fn the(of: Named) -> &'static str {
    match of {
        Named::Handle => "the handle",
        Named::Lane => "the lane name",
        Named::MemoryEntry => "the entry name",
        Named::AdminName => "the admin name",
    }
}
