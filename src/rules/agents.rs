use super::*;
use crate::config::MAX_HANDLE_ATTEMPTS;
use lanes::{AgentLane, HeldLane};

impl Rules {
    /// Creates a new avatar with a server-chosen handle and a fresh token.
    /// Only hashes of the token and recovery code are stored.
    /// Refused when the server is over `registrations_per_minute` or already
    /// has `max_agents` agents that are not revoked.
    pub async fn register(&self) -> Result<Registration> {
        self.register_with_client(None, None, None).await
    }

    /// As [`Rules::register`], recording the MCP client, and with the handle
    /// `proposed` if one is given. A proposed handle must pass
    /// [`names::check_proposed_handle`] (`invalid_argument`) and be no other
    /// avatar's, ignoring case, hidden and unregistered avatars included
    /// (`handle_taken`). It is never changed to fit; a refused one registers
    /// nothing. The new avatar takes the lane named `lane`, if given, at its
    /// newest version, announcing nothing; an unknown or retired lane
    /// registers nothing.
    pub async fn register_with_client(
        &self,
        client: Option<ClientInfo>,
        proposed: Option<String>,
        lane: Option<String>,
    ) -> Result<Registration> {
        if let Some(handle) = &proposed {
            names::check_proposed_handle(handle).map_err(RuleError::invalid)?;
        }
        if let Some(name) = &lane {
            names::check_lane_name(name)
                .map_err(|e| RuleError::invalid(format!("{e}. Nothing was registered.")))?;
        }
        let now = self.inner.deps.clock.now();
        if !self.inner.deps.registrations.allow(now) {
            return Err(RuleError::new(
                Code::RateLimited,
                format!(
                    "this server accepts at most {} registrations a minute; wait and try again",
                    self.limits().registrations_per_minute
                ),
            ));
        }
        let registered = self.insert_new_agent(client, proposed, lane).await;
        if registered.is_err() {
            self.inner.deps.registrations.refund();
        }
        registered
    }

    async fn insert_new_agent(
        &self,
        client: Option<ClientInfo>,
        proposed: Option<String>,
        lane: Option<String>,
    ) -> Result<Registration> {
        let max_agents = self.limits().max_agents;
        let entropy = self.inner.deps.entropy.clone();
        let wanted = match proposed {
            Some(handle) => Wanted::Proposed(handle),
            None => Wanted::Drawn(self.limits().name_pack.agent_handle(entropy.as_ref())),
        };
        let token = auth::token(entropy.as_ref());
        let hash = auth::hash_token(&token);
        let recovery_code = auth::recovery_code(entropy.as_ref());
        let recovery_hash = auth::hash_token(&recovery_code);
        let now = self.now_text();
        let created_at = now.clone();
        let run_id = self.inner.deps.run_id;
        let (handle, server_announcement, held) = self
            .db()
            .write(move |store| {
                let lane = lane
                    .map(|name| {
                        super::lanes::takeable_lane(store, &name, "Nothing was registered.")
                    })
                    .transpose()?;
                // A cap too large for the database's count type is no cap.
                let cap = i64::try_from(max_agents).unwrap_or(i64::MAX);
                if store.active_agent_count()? >= cap {
                    return Err(RuleError::new(
                        Code::LimitReached,
                        format!(
                            "this server already has its limit of {max_agents} agents; \
                             ask its operator to revoke unused ones"
                        ),
                    ));
                }
                let handle = match wanted {
                    Wanted::Drawn(base) => free_handle(store, &base)?,
                    Wanted::Proposed(handle) => {
                        if store.agent_name_taken(&handle)? {
                            return Err(RuleError::new(
                                Code::HandleTaken,
                                format!(
                                    "the handle {handle} is taken: handles are unique ignoring case, \
                                     and those of hidden and unregistered avatars stay taken"
                                ),
                            ));
                        }
                        handle
                    }
                };
                store.insert_agent(&NewAgentRecord {
                    name: handle.clone(),
                    token_hash: hash,
                    recovery_hash: Some(recovery_hash),
                    created_at: now.clone(),
                    last_seen: Some(now.clone()),
                    run_id,
                })?;
                store.set_agent_client(
                    &handle,
                    client
                        .map(|c| crate::store::AgentClient {
                            name: c.name,
                            version: c.version,
                            recorded_at: now.clone(),
                            run_id,
                        })
                        .as_ref(),
                )?;
                if let Some(lane) = lane {
                    // A new avatar is in no channel, so there is no one to tell.
                    store.set_agent_lane(
                        &handle,
                        Some(&crate::store::TakenLaneRecord {
                            lane: lane.name,
                            version: lane.current.version,
                            taken_at: now,
                            run_id,
                        }),
                    )?;
                }
                let announcement = store
                    .current_server_announcement()?
                    .map(ServerAnnouncement::from);
                let held = super::lanes::held_lane(store, &handle)?;
                Ok::<_, RuleError>((handle, announcement, held))
            })
            .await?;
        self.events().publish_agent(NewAgent {
            handle: handle.clone(),
            created_at,
        });
        Ok(Registration {
            handle,
            token,
            recovery_code,
            server_announcement,
            lane: held,
        })
    }

    /// Exchanges a recovery code for a fresh token for the same avatar.
    /// A revoked avatar cannot be reclaimed. The recovery code remains valid.
    pub async fn reclaim(&self, handle: String, recovery_code: String) -> Result<Reclaimed> {
        self.reclaim_with_client(handle, recovery_code, None, None)
            .await
    }

    /// As [`Rules::reclaim`], recording the MCP client. With `lane`, the
    /// avatar also takes that lane, as [`Rules::take_lane`] does, announced
    /// the same way; an unknown or retired lane is refused and changes
    /// nothing, the token included. Either way it returns the lane the
    /// avatar holds.
    pub async fn reclaim_with_client(
        &self,
        handle: String,
        recovery_code: String,
        client: Option<ClientInfo>,
        lane: Option<String>,
    ) -> Result<Reclaimed> {
        if let Some(name) = &lane {
            names::check_lane_name(name)
                .map_err(|e| RuleError::invalid(format!("{e}. Nothing changed.")))?;
        }
        let recovery_hash = auth::hash_token(&recovery_code);
        let token = auth::token(self.inner.deps.entropy.as_ref());
        let token_hash = auth::hash_token(&token);
        let now = self.now_text();
        let self_run_id = self.inner.deps.run_id;
        let rules = self.clone();
        let (handle, held, notices) = self
            .db()
            .write(move |store| {
                let found = store.agent_by_recovery_hash(&handle, &recovery_hash)?;
                let Some(AgentRecord {
                    revoked_at: None, ..
                }) = found
                else {
                    return Err(RuleError::new(
                        Code::Unauthenticated,
                        "unknown handle or recovery code, or avatar unavailable",
                    ));
                };
                store.set_token_hash(&handle, &token_hash)?;
                store.set_last_seen(&handle, &now)?;
                store.set_agent_client(
                    &handle,
                    client
                        .map(|c| crate::store::AgentClient {
                            name: c.name,
                            version: c.version,
                            recorded_at: now.clone(),
                            run_id: self_run_id,
                        })
                        .as_ref(),
                )?;
                let (held, notices) = match lane {
                    Some(lane) => {
                        let (taking, notices) = rules.take_lane_in(
                            store,
                            &handle,
                            Some(&lane),
                            &now,
                            "Nothing changed.",
                        )?;
                        (taking.held, notices)
                    }
                    None => (super::lanes::held_lane(store, &handle)?, Vec::new()),
                };
                Ok::<_, RuleError>((handle, held, notices))
            })
            .await?;
        for notice in &notices {
            self.publish_message(notice);
        }
        Ok(Reclaimed {
            handle,
            token,
            lane: held,
        })
    }

    /// Replaces `me`'s description, which others see when they list agents
    /// or members. Empty text clears it.
    pub async fn set_description(&self, me: &Agent, text: String) -> Result<AgentInfo> {
        let description = clean_description(&text, self.limits().max_description_chars)?;
        let me = me.handle.clone();
        self.db()
            .write(move |store| {
                store.set_description(&me, description.as_deref())?;
                store
                    .agent(&me)?
                    .map(AgentInfo::from)
                    .ok_or_else(|| RuleError::new(Code::Internal, format!("agent {me} vanished")))
            })
            .await
    }

    /// Replaces `me`'s handoff note: a note for whoever next holds this
    /// avatar, such as a session that takes it over with `reclaim`. Only
    /// this avatar reads it through MCP; the human sees it on the web
    /// page. Empty text clears it. The time is recorded either way.
    pub async fn set_handoff_note(&self, me: &Agent, text: String) -> Result<HandoffNote> {
        let max = self.limits().max_handoff_note_chars;
        if text.chars().count() > max {
            return Err(RuleError::invalid(format!(
                "handoff note must be at most {max} characters"
            )));
        }
        let handle = me.handle.clone();
        let now = self.now_text();
        self.db()
            .write(move |store| {
                store.set_handoff_note(&handle, &text, &now)?;
                handoff_note(store, &handle)
            })
            .await
    }

    /// `me`'s own handoff note.
    pub async fn handoff_note(&self, me: &Agent) -> Result<HandoffNote> {
        let handle = me.handle.clone();
        self.db()
            .read(move |store| handoff_note(store, &handle))
            .await
    }

    /// Agents in handle order, optionally matching handles or descriptions,
    /// and optionally only those holding the lane named `lane`.
    pub async fn list_agents(
        &self,
        me: &Agent,
        query: Option<String>,
        lane: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<Page<AgentInfo>> {
        let viewer = me.handle.clone();
        let rules = self.clone();
        self.db()
            .read(move |store| {
                let mut items: Vec<AgentInfo> = store
                    .agents(
                        query.as_deref(),
                        lane.as_deref(),
                        cursor.as_deref(),
                        limit + 1,
                    )?
                    .into_iter()
                    .map(|a| rules.observed_agent(store, a, Some(&viewer), None))
                    .collect::<Result<Vec<_>>>()?;
                let next_cursor = if items.len() > limit {
                    items.truncate(limit);
                    items.last().map(|a| a.handle.clone())
                } else {
                    None
                };
                Ok(Page { items, next_cursor })
            })
            .await
    }
}

pub(super) fn handoff_note(store: &dyn Reads, handle: &str) -> Result<HandoffNote> {
    let note = store
        .handoff_note(handle)?
        .ok_or_else(|| RuleError::agent_not_found(handle))?;
    Ok(HandoffNote {
        handle: handle.to_string(),
        text: note.text,
        set_at: note.set_at,
    })
}

/// The handle a new agent asks for.
enum Wanted {
    /// Drawn from the name pack: numbered if taken.
    Drawn(String),
    /// Proposed by the agent: refused if taken.
    Proposed(String),
}

/// `base` if no agent has it, else the first free numbered form of it.
fn free_handle(store: &dyn Reads, base: &str) -> Result<String> {
    if !store.agent_name_taken(base)? {
        return Ok(base.to_string());
    }
    for attempt in 1..=MAX_HANDLE_ATTEMPTS {
        let candidate = names::numbered_handle(base, attempt);
        if !store.agent_name_taken(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(RuleError::new(
        Code::Internal,
        format!("could not find a free handle based on {base}"),
    ))
}

/// Trimmed description text, or `None` if it is empty.
fn clean_description(text: &str, max_chars: usize) -> Result<Option<String>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() > max_chars {
        return Err(RuleError::invalid(format!(
            "description must be at most {max_chars} characters"
        )));
    }
    Ok(Some(text.to_string()))
}

/// The note an avatar keeps for whoever next holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HandoffNote {
    pub handle: String,
    pub text: String,
    /// When it was last set, also when cleared. `None` if never.
    pub set_at: Option<String>,
}

/// Who makes a change: an agent, the human operator through the web page,
/// or an admin agent acting for the operator at `/mcp-admin`. All go
/// through the same rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Actor {
    Agent(Agent),
    Human,
    /// An admin agent, under the name tied to its admin key. It may change
    /// what the human may, except issuing recovery codes; its changes are
    /// announced as the operator's, via its name.
    Admin(crate::names::AdminName),
}

impl From<&Agent> for Actor {
    fn from(agent: &Agent) -> Actor {
        Actor::Agent(agent.clone())
    }
}

/// An agent that has proven which avatar it holds on this call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub handle: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Registration {
    pub server_announcement: Option<ServerAnnouncement>,
    pub handle: String,
    pub token: String,
    pub recovery_code: String,
    /// The lane the new avatar took, if it asked for one.
    pub lane: Option<HeldLane>,
}

/// A new authentication token for an existing avatar.
#[derive(Clone, Debug, Serialize)]
pub struct Reclaimed {
    pub handle: String,
    pub token: String,
    /// The lane the avatar holds, if any.
    pub lane: Option<HeldLane>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentInfo {
    pub handle: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    /// The server run that created the agent, if it was recorded.
    pub run_id: Option<i64>,
    /// What the agent says about itself, if anything.
    pub description: Option<String>,
    pub client: Option<crate::store::AgentClient>,
    pub listening: bool,
    pub waiting_channels: Vec<String>,
    /// The lane the agent holds, if any. It grants no permissions.
    pub lane: Option<AgentLane>,
}

// Agents see revoked avatars like any other: their handles and messages stay.
impl From<AgentRecord> for AgentInfo {
    fn from(a: AgentRecord) -> AgentInfo {
        AgentInfo {
            handle: a.name,
            created_at: a.created_at,
            last_seen: a.last_seen,
            run_id: a.run_id,
            description: a.description,
            client: a.client,
            listening: false,
            waiting_channels: Vec::new(),
            lane: a.lane.map(AgentLane::from),
        }
    }
}

/// Client-reported MCP initialization metadata, without library types.
#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}
