//! Pure, bounded projection of an optional convention in ordinary messages.
//! Reports are claims by their stored authors, never human authorization.
mod parse;
mod reduce;
#[cfg(test)]
mod tests;

use crate::config::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MessageInput {
    pub id: i64,
    pub channel_id: String,
    pub author: String,
    pub body: String,
    pub created_at: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionLimits {
    pub max_messages: usize,
    pub max_bytes: usize,
    pub max_items: usize,
    pub max_diagnostics: usize,
}
impl ProjectionLimits {
    pub fn configured() -> Self {
        Self {
            max_messages: WORKBOARD_MAX_MESSAGES,
            max_bytes: WORKBOARD_MAX_BYTES,
            max_items: WORKBOARD_MAX_ITEMS,
            max_diagnostics: WORKBOARD_MAX_DIAGNOSTICS,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionError(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Diagnostic {
    pub message_id: i64,
    pub explanation: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Task,
    Decision,
    Approval,
    PullRequest,
    Issue,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Task => "Task",
            Self::Decision => "Decision",
            Self::Approval => "Approval",
            Self::PullRequest => "PR",
            Self::Issue => "Issue",
        }
    }
    pub fn is_request(self) -> bool {
        matches!(self, Self::Decision | Self::Approval)
    }
    pub fn is_external(self) -> bool {
        matches!(self, Self::PullRequest | Self::Issue)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Task {
    pub title: String,
    pub state: String,
    pub owner: Option<String>,
    pub next_step: Option<String>,
    #[serde(default)]
    pub dependency_ids: Vec<String>,
    pub blocker_reason: Option<String>,
    #[serde(default)]
    pub evidence_message_ids: Vec<i64>,
    pub responsibility: Option<String>,
    pub check_interval_seconds: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Choice {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Decision {
    pub title: String,
    pub question: String,
    pub response_mode: String,
    #[serde(default)]
    pub options: Vec<Choice>,
    pub affected_task_ids: Vec<String>,
    pub recommendation: Option<String>,
    pub priority: Option<String>,
    pub response_url: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Approval {
    pub title: String,
    pub action: String,
    pub scope: String,
    pub subject: String,
    pub affected_task_ids: Vec<String>,
    pub expires_at: Option<String>,
    pub priority: Option<String>,
    pub response_url: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reference {
    pub title: String,
    pub provider: String,
    pub external_key: String,
    pub url: String,
    pub task_ids: Vec<String>,
    pub status: String,
    pub checks: Option<String>,
    pub reviews: Option<String>,
    pub observed_at: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(super) enum Payload {
    Task(Task),
    Decision(Decision),
    Approval(Approval),
    Reference(Reference),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resolution {
    pub author: String,
    pub note: String,
    pub evidence: String,
    pub evidence_url: Option<String>,
    pub evidence_message_ids: Vec<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Verification {
    pub message_id: i64,
    pub author: String,
    pub report_time: String,
    pub success: bool,
    pub evidence_message_ids: Vec<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub status: String,
    pub creator: String,
    pub editor: String,
    pub revision: i64,
    pub source_message_id: i64,
    pub updated_at: i64,
    pub verified_at: Option<i64>,
    pub verification_error: Option<String>,
    pub resolution: Option<Resolution>,
    pub(super) verification: Option<Verification>,
    pub(super) payload: Payload,
}
impl Item {
    pub(super) fn invalidate_verification(&mut self) {
        self.verified_at = None;
        self.verification_error = None;
        self.verification = None;
    }

    pub fn owner(&self) -> &str {
        match &self.payload {
            Payload::Task(t) => t.owner.as_deref().unwrap_or("Unassigned"),
            _ => &self.creator,
        }
    }
    pub fn priority(&self) -> u8 {
        let p = match &self.payload {
            Payload::Decision(d) => d.priority.as_deref(),
            Payload::Approval(a) => a.priority.as_deref(),
            _ => None,
        };
        match p {
            Some("high") => 0,
            Some("low") => 2,
            _ => 1,
        }
    }
    pub fn affected_tasks(&self) -> &[String] {
        match &self.payload {
            Payload::Decision(d) => &d.affected_task_ids,
            Payload::Approval(a) => &a.affected_task_ids,
            _ => &[],
        }
    }
    pub fn expires_at(&self) -> Option<i64> {
        match &self.payload {
            Payload::Approval(a) => a.expires_at.as_deref().and_then(parse::timestamp),
            _ => None,
        }
    }
    pub fn status_at(&self, now: i64) -> &str {
        if self.status == "pending" && self.expires_at().is_some_and(|at| at <= now) {
            "expired"
        } else {
            &self.status
        }
    }
    pub fn details(&self) -> Vec<(String, String)> {
        let mut d = match &self.payload {
            Payload::Task(t) => vec![
                ("Next step".into(), t.next_step.clone().unwrap_or_default()),
                (
                    "Blocker".into(),
                    t.blocker_reason.clone().unwrap_or_default(),
                ),
                ("Dependencies".into(), t.dependency_ids.join(", ")),
            ],
            Payload::Decision(t) => vec![
                ("Question".into(), t.question.clone()),
                (
                    "Options".into(),
                    t.options
                        .iter()
                        .map(|o| format!("{}: {}", o.id, o.label))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                (
                    "Recommendation".into(),
                    t.recommendation.clone().unwrap_or_default(),
                ),
            ],
            Payload::Approval(t) => vec![
                ("Action".into(), t.action.clone()),
                ("Scope".into(), t.scope.clone()),
                ("Subject".into(), t.subject.clone()),
            ],
            Payload::Reference(t) => vec![
                ("Provider".into(), t.provider.clone()),
                ("External ID".into(), t.external_key.clone()),
                ("Checks".into(), t.checks.clone().unwrap_or_default()),
                ("Reviews".into(), t.reviews.clone().unwrap_or_default()),
                (
                    "Observed (agent reported)".into(),
                    t.observed_at.clone().unwrap_or_default(),
                ),
            ],
        };
        let relationships = match &self.payload {
            Payload::Task(t) => &t.dependency_ids,
            Payload::Decision(d) => &d.affected_task_ids,
            Payload::Approval(a) => &a.affected_task_ids,
            Payload::Reference(r) => &r.task_ids,
        };
        if !relationships.is_empty() {
            d.push(("Related tasks".into(), relationships.join(", ")));
        }
        if let Some(v) = &self.verification {
            d.push((
                "Last verification".into(),
                format!(
                    "{} reported {} at {}; message {}",
                    v.author,
                    if v.success { "success" } else { "failure" },
                    v.report_time,
                    v.message_id
                ),
            ));
        }
        if let Some(r) = &self.resolution {
            d.push((format!("Reported resolved by {}", r.author), r.note.clone()));
            d.push(("Resolution evidence".into(), r.evidence.clone()));
        }
        if let Some(e) = &self.verification_error {
            d.push(("Last verification failed".into(), e.clone()));
        }
        d.retain(|(_, v)| !v.is_empty());
        d
    }
    pub fn external_url(&self) -> Option<&str> {
        match &self.payload {
            Payload::Reference(r) => Some(&r.url),
            _ => None,
        }
    }
    pub fn external_identity(&self) -> Option<(&str, &str)> {
        match &self.payload {
            Payload::Reference(reference) => Some((&reference.provider, &reference.external_key)),
            _ => None,
        }
    }
    pub fn response_url(&self) -> Option<&str> {
        match &self.payload {
            Payload::Decision(r) => r.response_url.as_deref(),
            Payload::Approval(r) => r.response_url.as_deref(),
            _ => None,
        }
    }
    pub fn evidence_ids(&self) -> Vec<i64> {
        let mut ids = BTreeSet::new();
        if let Payload::Task(t) = &self.payload {
            ids.extend(t.evidence_message_ids.iter().copied());
        }
        if let Some(v) = &self.verification {
            ids.insert(v.message_id);
            ids.extend(v.evidence_message_ids.iter().copied());
        }
        if let Some(r) = &self.resolution {
            ids.extend(r.evidence_message_ids.iter().copied());
        }
        ids.into_iter().collect()
    }
    pub fn verifier(&self) -> Option<(&str, i64)> {
        match &self.payload {
            Payload::Task(t) if t.responsibility.as_deref() == Some("workboard_verifier") => {
                t.owner.as_deref().zip(t.check_interval_seconds)
            }
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Board {
    pub items: BTreeMap<String, Item>,
    pub diagnostics: Vec<Diagnostic>,
    pub omitted_diagnostics: usize,
    pub last_message_id: i64,
    channel_id: Option<String>,
    limits: ProjectionLimits,
    count: usize,
    bytes: usize,
    requests: BTreeMap<(String, String), String>,
    seen_messages: BTreeSet<i64>,
}
pub(crate) fn project(
    messages: &[MessageInput],
    _now_unix_seconds: i64,
    limits: ProjectionLimits,
) -> Result<Board, ProjectionError> {
    let mut b = Board {
        items: BTreeMap::new(),
        diagnostics: Vec::new(),
        omitted_diagnostics: 0,
        last_message_id: 0,
        channel_id: None,
        limits,
        count: 0,
        bytes: 0,
        requests: BTreeMap::new(),
        seen_messages: BTreeSet::new(),
    };
    for m in messages {
        b.apply(m)?;
    }
    Ok(b)
}
