use super::*;
use kkt_names::{MAX_HANDLE_CHARS, MAX_LANE_NAME_CHARS};

/// How many numbered handles to try before giving up on a drawn handle.
/// kkt-names' tests show every drawn handle, numbered up to attempt
/// 10,000, fits its handle limit, so this must stay at most that.
const MAX_HANDLE_ATTEMPTS: u32 = 10_000;
const _: () = assert!(MAX_HANDLE_ATTEMPTS <= 10_000);

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
            names::check_proposed_handle(handle, MAX_HANDLE_CHARS).map_err(RuleError::invalid)?;
        }
        if let Some(name) = &lane {
            names::check_lane_name(name, MAX_LANE_NAME_CHARS)
                .map_err(|e| RuleError::invalid(format!("{e}. Nothing was registered.")))?;
        }
        let now = self.inner.deps.clock.now();
        if !self.inner.registrations.allow(now) {
            return Err(RuleError::new(
                Code::RateLimited,
                format!(
                    "this server accepts at most {} registrations a minute; wait and try again",
                    self.settings().registrations_per_minute
                ),
            ));
        }
        let registered = self.insert_new_agent(client, proposed, lane).await;
        if registered.is_err() {
            self.inner.registrations.refund();
        }
        registered
    }

    async fn insert_new_agent(
        &self,
        client: Option<ClientInfo>,
        proposed: Option<String>,
        lane: Option<String>,
    ) -> Result<Registration> {
        let max_agents = self.settings().max_agents;
        let entropy = self.inner.deps.entropy.clone();
        let wanted = match proposed {
            Some(handle) => Wanted::Proposed(handle),
            None => Wanted::Drawn(self.settings().name_pack.agent_handle(entropy.as_ref())),
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
            .write(move |store, swarm| {
                let lane = lane
                    .map(|name| {
                        super::lanes::takeable_lane(store, &name, "Nothing was registered.")
                    })
                    .transpose()?;
                // A cap too large for the database's count type is no cap.
                let cap = i64::try_from(max_agents).unwrap_or(i64::MAX);
                if store.active_agent_count(swarm, )? >= cap {
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
                store.insert_agent(swarm, &NewAgentRecord {
                    name: handle.clone(),
                    token_hash: hash,
                    recovery_hash: Some(recovery_hash),
                    created_at: now.clone(),
                    last_seen: Some(now.clone()),
                    run_id,
                })?;
                store.set_agent_client(swarm,
                    &handle,
                    client
                        .map(|c| crate::AgentClient {
                            name: c.name,
                            version: c.version,
                            recorded_at: now.clone(),
                            run_id,
                        })
                        .as_ref(),
                )?;
                if let Some(lane) = lane {
                    // A new avatar is in no channel, so there is no one to tell.
                    store.set_agent_lane(swarm,
                        &handle,
                        Some(&crate::TakenLaneRecord {
                            lane: lane.name,
                            version: lane.current.version,
                            taken_at: now,
                            run_id,
                        }),
                    )?;
                }
                let announcement = store
                    .current_server_announcement()?;
                let held = super::lanes::held_lane(store, swarm, &handle)?;
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
            names::check_lane_name(name, MAX_LANE_NAME_CHARS)
                .map_err(|e| RuleError::invalid(format!("{e}. Nothing changed.")))?;
        }
        let recovery_hash = auth::hash_token(&recovery_code);
        let token = auth::token(self.inner.deps.entropy.as_ref());
        let token_hash = auth::hash_token(&token);
        let now = self.now_text();
        let self_run_id = self.inner.deps.run_id;
        let rules = self.clone();
        let (handle, server_announcement, held, notices, memory) = self
            .db()
            .write(move |store, swarm| {
                let found = store.agent_by_recovery_hash(swarm, &handle, &recovery_hash)?;
                let Some(AgentRecord {
                    revoked_at: None, ..
                }) = found
                else {
                    return Err(RuleError::new(
                        Code::Unauthenticated,
                        "unknown handle or recovery code, or avatar unavailable",
                    ));
                };
                store.set_token_hash(swarm, &handle, &token_hash)?;
                store.set_last_seen(swarm, &handle, &now)?;
                store.set_agent_client(
                    swarm,
                    &handle,
                    client
                        .map(|c| crate::AgentClient {
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
                            swarm,
                            &handle,
                            Some(&lane),
                            &now,
                            "Nothing changed.",
                        )?;
                        (taking.held, notices)
                    }
                    None => (super::lanes::held_lane(store, swarm, &handle)?, Vec::new()),
                };
                let announcement = store.current_server_announcement()?;
                let memory = super::memory::memory_summaries(store, swarm, &handle)?;
                Ok::<_, RuleError>((handle, announcement, held, notices, memory))
            })
            .await?;
        for notice in &notices {
            self.publish_message(notice);
        }
        Ok(Reclaimed {
            handle,
            token,
            server_announcement,
            lane: held,
            memory,
        })
    }

    /// Replaces `me`'s description, which others see when they list agents
    /// or members. Empty text clears it.
    pub async fn set_description(&self, me: &Agent, text: String) -> Result<AgentInfo> {
        let description = clean_description(&text, self.settings().max_description_chars)?;
        let me = me.handle.clone();
        self.db()
            .write(move |store, swarm| {
                store.set_description(swarm, &me, description.as_deref())?;
                store
                    .agent(swarm, &me)?
                    .map(AgentInfo::from)
                    .ok_or_else(|| RuleError::new(Code::Internal, format!("agent {me} vanished")))
            })
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
            .read(move |store, swarm| {
                let mut items: Vec<AgentInfo> = store
                    .agents(
                        swarm,
                        query.as_deref(),
                        lane.as_deref(),
                        cursor.as_deref(),
                        limit + 1,
                    )?
                    .into_iter()
                    .map(|a| rules.observed_agent(store, swarm, a, Some(&viewer), None))
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

/// The handle a new agent asks for.
enum Wanted {
    /// Drawn from the name pack: numbered if taken.
    Drawn(String),
    /// Proposed by the agent: refused if taken.
    Proposed(String),
}

/// `base` if no agent has it, else the first free numbered form of it.
fn free_handle(store: &Reader<'_>, base: &str) -> Result<String> {
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

/// Refuses a change from an operator without [`Permission::Change`].
/// Each rule that makes a change calls this before anything else.
pub(super) fn may_change(actor: &Actor) -> Result<()> {
    match actor {
        Actor::Avatar(_) => Ok(()),
        Actor::Operator(operator) => operator.may(Permission::Change),
    }
}

// Agents see revoked avatars like any other: their handles and messages stay.
impl From<AgentRecord> for AgentInfo {
    fn from(a: AgentRecord) -> AgentInfo {
        AgentInfo {
            handle: a.name,
            created_at: a.created_at,
            last_seen: a.last_seen,
            last_sent: None,
            run_id: a.run_id,
            description: a.description,
            client: a.client.map(RecordedClient::from),
            listening: false,
            waiting_channels: Vec::new(),
            lane: a.lane.map(AgentLane::from),
        }
    }
}

impl From<crate::AgentClient> for RecordedClient {
    fn from(c: crate::AgentClient) -> RecordedClient {
        RecordedClient {
            name: c.name,
            version: c.version,
            recorded_at: c.recorded_at,
            run_id: c.run_id,
        }
    }
}
