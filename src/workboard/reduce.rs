use super::parse::{self, Operation};
use super::*;

enum ReportError {
    Invalid(String),
    ItemLimit,
}
impl From<String> for ReportError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}
impl From<&str> for ReportError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.into())
    }
}

impl Board {
    pub fn apply(&mut self, m: &MessageInput) -> Result<(), ProjectionError> {
        if m.id <= self.last_message_id
            || self.channel_id.as_ref().is_some_and(|c| c != &m.channel_id)
        {
            return Err(ProjectionError(
                "Messages must be ascending in one channel".into(),
            ));
        }
        if self.count >= self.limits.max_messages
            || self.bytes.saturating_add(m.body.len()) > self.limits.max_bytes
        {
            return Err(ProjectionError("Board unavailable: history limit".into()));
        }
        self.channel_id = Some(m.channel_id.clone());
        self.count += 1;
        self.bytes += m.body.len();
        self.last_message_id = m.id;
        match parse::parse(&m.body) {
            Ok(Some(r)) => {
                let key = (m.author.clone(), r.request_id.clone());
                if let Some(old) = self.requests.get(&key) {
                    if old != &r.canonical {
                        self.diagnostic(m.id, "Request ID reused with different content".into());
                    }
                } else {
                    match self.report(m, &r) {
                        Ok(()) => {
                            self.requests.insert(key, r.canonical);
                        }
                        Err(ReportError::Invalid(e)) => self.diagnostic(m.id, e),
                        Err(ReportError::ItemLimit) => {
                            return Err(ProjectionError("Board unavailable: item limit".into()));
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => self.diagnostic(m.id, e),
        }
        self.seen_messages.insert(m.id);
        Ok(())
    }
    fn diagnostic(&mut self, id: i64, explanation: String) {
        if self.diagnostics.len() < self.limits.max_diagnostics {
            self.diagnostics.push(Diagnostic {
                message_id: id,
                explanation,
            });
        } else {
            self.omitted_diagnostics += 1;
        }
    }
    fn evidence(&self, ids: &[i64]) -> Result<(), String> {
        if ids.iter().any(|id| !self.seen_messages.contains(id)) {
            return Err("Evidence must reference earlier messages in this channel".into());
        }
        Ok(())
    }
    fn report(&mut self, m: &MessageInput, r: &parse::Report) -> Result<(), ReportError> {
        let at = parse::timestamp(&m.created_at).ok_or("Invalid server timestamp")?;
        match &r.operation {
            Operation::Create {
                kind,
                payload,
                supersedes,
            } => {
                if self.items.contains_key(&r.item_id) {
                    return Err("Item already exists".into());
                }
                let Some(suffix) = r.item_id.strip_prefix(&format!("{}/", m.author)) else {
                    return Err("Item ID must start with creator name/".into());
                };
                if suffix.is_empty() {
                    return Err("Missing creator-chosen item ID".into());
                }
                let p = parse::payload(*kind, payload.clone())?;
                self.validate_payload(&r.item_id, &p, at)?;
                if let Some(old) = supersedes {
                    let item = self.items.get(old).ok_or("Superseded request not found")?;
                    if !kind.is_request()
                        || item.kind != *kind
                        || item.creator != m.author
                        || item.status_at(at) != "pending"
                    {
                        return Err(
                            "Only requester may supersede a pending request of the same kind"
                                .into(),
                        );
                    }
                }
                // Only an otherwise valid create consumes the projected-item budget.
                // Check before mutating either the new item or a superseded request.
                if self.items.len() >= self.limits.max_items {
                    return Err(ReportError::ItemLimit);
                }
                let (title, status) = payload_summary(&p);
                let item = Item {
                    id: r.item_id.clone(),
                    kind: *kind,
                    title,
                    status,
                    creator: m.author.clone(),
                    editor: m.author.clone(),
                    revision: m.id,
                    source_message_id: m.id,
                    updated_at: at,
                    verified_at: None,
                    verification_error: None,
                    resolution: None,
                    verification: None,
                    payload: p,
                };
                if let Some(old) = supersedes
                    && let Some(i) = self.items.get_mut(old)
                {
                    i.status = "superseded".into();
                    i.revision = m.id;
                    i.updated_at = at;
                    i.editor = m.author.clone();
                    i.invalidate_verification();
                }
                self.items.insert(r.item_id.clone(), item);
            }
            Operation::Update {
                base_message_id,
                payload,
                reason,
            } => {
                let old = self.current(&r.item_id, *base_message_id)?;
                if old.kind.is_request() {
                    return Err("Request content is immutable; create a new request".into());
                }
                let p = parse::payload(old.kind, payload.clone())?;
                self.validate_payload(&r.item_id, &p, at)?;
                let (title, status) = payload_summary(&p);
                if matches!(old.status.as_str(), "done" | "cancelled")
                    && old.status != status
                    && !reason.as_deref().is_some_and(parse::text)
                {
                    return Err("Reopening requires a reason".into());
                }
                if let Some(i) = self.items.get_mut(&r.item_id) {
                    i.title = title;
                    i.status = status;
                    i.payload = p;
                    i.revision = m.id;
                    i.updated_at = at;
                    i.editor = m.author.clone();
                    i.invalidate_verification();
                    i.verified_at = None;
                    i.verification_error = None;
                }
            }
            Operation::Withdraw {
                base_message_id,
                reason,
            } => {
                let old = self.pending_request(&r.item_id, *base_message_id, &m.author, at)?;
                if !parse::text(reason) {
                    return Err("Withdrawal requires a reason".into());
                }
                let _ = old;
                if let Some(i) = self.items.get_mut(&r.item_id) {
                    i.status = "withdrawn".into();
                    i.revision = m.id;
                    i.updated_at = at;
                    i.editor = m.author.clone();
                    i.invalidate_verification();
                }
            }
            Operation::Verify {
                item_revision,
                evidence_message_ids,
                observed_at,
                success,
            } => {
                self.current(&r.item_id, *item_revision)?;
                self.evidence(evidence_message_ids)?;
                if evidence_message_ids.is_empty() {
                    return Err("Verification requires evidence".into());
                }
                let observed = match observed_at {
                    Some(s) => parse::timestamp(s).ok_or("Invalid observed_at")?,
                    None => at,
                };
                if observed > at {
                    return Err("Observation time is in the future".into());
                }
                if let Some(i) = self.items.get_mut(&r.item_id) {
                    i.verification = Some(Verification {
                        message_id: m.id,
                        author: m.author.clone(),
                        report_time: m.created_at.clone(),
                        success: *success,
                        evidence_message_ids: evidence_message_ids.clone(),
                    });
                    if *success {
                        i.verified_at = Some(observed);
                        i.verification_error = None;
                    } else {
                        i.verification_error = Some(format!(
                            "Failed check reported by {} at {}",
                            m.author, m.created_at
                        ));
                    }
                }
            }
            Operation::Resolve {
                base_message_id,
                resolution_note,
                evidence_description,
                evidence_message_ids,
                evidence_url,
            } => {
                self.pending_request(&r.item_id, *base_message_id, &m.author, at)?;
                if !parse::text(resolution_note) || !parse::text(evidence_description) {
                    return Err("Resolution requires a note and evidence description".into());
                }
                self.evidence(evidence_message_ids)?;
                if evidence_url.as_deref().is_some_and(|u| !parse::safe_url(u)) {
                    return Err("Unsafe evidence URL".into());
                }
                if let Some(i) = self.items.get_mut(&r.item_id) {
                    i.status = "reported_resolved".into();
                    i.revision = m.id;
                    i.editor = m.author.clone();
                    i.invalidate_verification();
                    i.updated_at = at;
                    i.resolution = Some(Resolution {
                        author: m.author.clone(),
                        note: resolution_note.clone(),
                        evidence: evidence_description.clone(),
                        evidence_url: evidence_url.clone(),
                        evidence_message_ids: evidence_message_ids.clone(),
                    });
                }
            }
        }
        Ok(())
    }
    fn current(&self, id: &str, base: i64) -> Result<&Item, String> {
        let i = self.items.get(id).ok_or("Item not found")?;
        if i.revision != base {
            return Err(format!(
                "Revision conflict: current message is {}",
                i.revision
            ));
        }
        Ok(i)
    }
    fn pending_request(&self, id: &str, base: i64, author: &str, at: i64) -> Result<&Item, String> {
        let i = self.current(id, base)?;
        if !i.kind.is_request() || i.creator != author || i.status_at(at) != "pending" {
            return Err("Only requester may close a pending request".into());
        }
        Ok(i)
    }
    fn tasks(&self, ids: &[String]) -> Result<(), String> {
        for id in ids {
            if self.items.get(id).is_none_or(|i| i.kind != Kind::Task) {
                return Err("Relationships must reference existing tasks in this channel".into());
            }
        }
        Ok(())
    }
    fn validate_payload(&self, id: &str, p: &Payload, at: i64) -> Result<(), String> {
        let (title, _) = payload_summary(p);
        if title.trim().is_empty() || title.chars().count() > WORKBOARD_MAX_TITLE_CHARS {
            return Err("Invalid title".into());
        }
        match p {
            Payload::Task(t) => {
                if !["queued", "in_progress", "blocked", "done", "cancelled"]
                    .contains(&t.state.as_str())
                {
                    return Err("Invalid task state".into());
                }
                if t.state == "blocked"
                    && !t.blocker_reason.as_deref().is_some_and(parse::text)
                    && t.dependency_ids.is_empty()
                {
                    return Err("Blocked work requires a blocker".into());
                }
                self.tasks(&t.dependency_ids)?;
                self.evidence(&t.evidence_message_ids)?;
                let mut stack = t.dependency_ids.clone();
                let mut visited = BTreeSet::new();
                while let Some(dep) = stack.pop() {
                    if dep == id {
                        return Err("Dependency cycle".into());
                    }
                    if visited.insert(dep.clone())
                        && let Some(Item {
                            payload: Payload::Task(other),
                            ..
                        }) = self.items.get(&dep)
                    {
                        stack.extend(other.dependency_ids.clone());
                    }
                }
                if let Some(role) = &t.responsibility {
                    if role != "workboard_verifier"
                        || t.owner.as_deref().is_none_or(|s| !parse::text(s))
                        || t.check_interval_seconds.is_none_or(|n| n <= 0)
                    {
                        return Err("Verifier requires owner and positive cadence".into());
                    }
                } else if t.check_interval_seconds.is_some() {
                    return Err("Cadence requires verifier responsibility".into());
                }
            }
            Payload::Decision(d) => {
                self.tasks(&d.affected_task_ids)?;
                priority(d.priority.as_deref())?;
                url(d.response_url.as_deref())?;
                if !parse::text(&d.question)
                    || !["choice", "text"].contains(&d.response_mode.as_str())
                {
                    return Err("Invalid decision".into());
                }
                if d.response_mode == "choice" {
                    let mut seen = BTreeSet::new();
                    if d.options.is_empty()
                        || d.options.len() > WORKBOARD_MAX_OPTIONS
                        || d.options.iter().any(|o| {
                            !parse::id(&o.id) || !parse::text(&o.label) || !seen.insert(&o.id)
                        })
                    {
                        return Err("Choice options must be nonempty and unique".into());
                    }
                } else if !d.options.is_empty() {
                    return Err("Text decisions cannot have choice options".into());
                }
            }
            Payload::Approval(a) => {
                self.tasks(&a.affected_task_ids)?;
                priority(a.priority.as_deref())?;
                url(a.response_url.as_deref())?;
                if !parse::text(&a.action) || !parse::text(&a.scope) || !parse::text(&a.subject) {
                    return Err("Approval requires exact action, scope, and subject".into());
                }
                if let Some(s) = &a.expires_at
                    && parse::timestamp(s).is_none_or(|v| v <= at)
                {
                    return Err("Expiry must be a future timestamp".into());
                }
            }
            Payload::Reference(r) => {
                self.tasks(&r.task_ids)?;
                url(Some(&r.url))?;
                if !parse::text(&r.provider)
                    || !parse::text(&r.external_key)
                    || !parse::text(&r.status)
                {
                    return Err("External identity and status required".into());
                }
                if let Some(t) = &r.observed_at
                    && parse::timestamp(t).is_none_or(|t| t > at)
                {
                    return Err("Invalid or future observation time".into());
                }
            }
        }
        Ok(())
    }
}
fn priority(p: Option<&str>) -> Result<(), String> {
    if p.is_some_and(|s| !["low", "normal", "high"].contains(&s)) {
        Err("Invalid priority".into())
    } else {
        Ok(())
    }
}
fn url(p: Option<&str>) -> Result<(), String> {
    if p.is_some_and(|s| !parse::safe_url(s)) {
        Err("Unsafe URL".into())
    } else {
        Ok(())
    }
}
fn payload_summary(p: &Payload) -> (String, String) {
    match p {
        Payload::Task(t) => (t.title.clone(), t.state.clone()),
        Payload::Decision(t) => (t.title.clone(), "pending".into()),
        Payload::Approval(t) => (t.title.clone(), "pending".into()),
        Payload::Reference(t) => (t.title.clone(), t.status.clone()),
    }
}
