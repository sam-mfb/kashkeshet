use super::*;
use kkt_events::{NewMessage, Notice};
use std::sync::atomic::{AtomicBool, Ordering};

impl Rules {
    /// Posts a message to a channel `me` is in. Returns the stored message.
    pub async fn send(&self, me: &Agent, channel_id: String, body: String) -> Result<Message> {
        let max = self.limits().max_body_bytes;
        if body.trim().is_empty() {
            return Err(RuleError::invalid("message body must not be empty"));
        }
        if body.len() > max {
            return Err(RuleError::new(
                Code::BodyTooLong,
                format!("message body is {} bytes; the limit is {max}", body.len()),
            ));
        }
        if auth::contains_secret(&body) {
            return Err(RuleError::new(
                Code::ContainsSecret,
                "message contains something shaped like a token or recovery code. \
                 Tokens and recovery codes must never be shared; remove it and send again.",
            ));
        }
        let me = me.handle.clone();
        let now = self.inner.deps.clock.now();
        let rules = self.clone();
        // Set once the rate limiter has counted this send, so a failed write
        // can give it back.
        let charged = Arc::new(AtomicBool::new(false));
        let charged_in_tx = charged.clone();
        let sender = me.clone();
        let written = self
            .db()
            .write(move |store, swarm| {
                let me = sender;
                let channel = member_channel(store, swarm, &me, &channel_id)?;
                if !rules.inner.sends.allow(&me, now) {
                    return Err(RuleError::new(
                        Code::RateLimited,
                        format!(
                            "you can send at most {} messages a minute; wait and try again",
                            rules.limits().sends_per_minute
                        ),
                    ));
                }
                charged_in_tx.store(true, Ordering::SeqCst);
                let created_at = clock::to_text(now);
                let run_id = rules.inner.deps.run_id;
                let id = store.insert_message(
                    swarm,
                    &NewMessageRecord {
                        channel_id: channel.id.clone(),
                        agent: me.clone(),
                        body: body.clone(),
                        created_at: created_at.clone(),
                        run_id,
                    },
                )?;
                Ok(Message {
                    id,
                    channel_id: channel.id,
                    channel_name: channel.name,
                    agent: me,
                    body,
                    created_at,
                    run_id: Some(run_id),
                })
            })
            .await;
        let message = match written {
            Ok(message) => message,
            Err(e) => {
                if charged.load(Ordering::SeqCst) {
                    self.inner.sends.refund(&me);
                }
                return Err(e);
            }
        };
        self.publish_message(&message);
        Ok(message)
    }

    /// Tells waiting agents and the web page about a stored message.
    pub(super) fn publish_message(&self, message: &Message) {
        self.events().publish(NewMessage {
            id: message.id,
            channel_id: message.channel_id.clone(),
            channel_name: message.channel_name.clone(),
            agent: message.agent.clone(),
            body: message.body.clone(),
            created_at: message.created_at.clone(),
            run_id: message.run_id,
        });
    }

    /// Stores a message from the operator in `channel`, saying what the
    /// human changed. Runs inside the change's own write, so the change and
    /// its message are stored together or not at all. The caller publishes
    /// the message once the write is done.
    pub(super) fn post_as_operator(
        &self,
        store: &dyn Writes,
        swarm: &SwarmId,
        channel: &ChannelFacts,
        body: String,
    ) -> Result<Message> {
        self.post_notice(store, swarm, channel, names::OPERATOR, body)
    }

    /// Stores a message from `author` in `channel`, saying what changed,
    /// inside the change's own write, as [`Rules::post_as_operator`] does.
    pub(super) fn post_notice(
        &self,
        store: &dyn Writes,
        swarm: &SwarmId,
        channel: &ChannelFacts,
        author: &str,
        body: String,
    ) -> Result<Message> {
        let created_at = self.now_text();
        let run_id = self.inner.deps.run_id;
        let id = store.insert_message(
            swarm,
            &NewMessageRecord {
                channel_id: channel.id.clone(),
                agent: author.to_string(),
                body: body.clone(),
                created_at: created_at.clone(),
                run_id,
            },
        )?;
        Ok(Message {
            id,
            channel_id: channel.id.clone(),
            channel_name: channel.name.clone(),
            agent: author.to_string(),
            body,
            created_at,
            run_id: Some(run_id),
        })
    }

    /// Messages in a channel `me` is in.
    pub async fn read(&self, me: &Agent, q: ReadQuery) -> Result<MessagePage> {
        let me = me.handle.clone();
        self.db()
            .read(move |store, swarm| {
                member_channel(store, swarm, &me, &q.channel_id)?;
                read_channel(store, swarm, &q)
            })
            .await
    }

    /// One message in full.
    pub async fn get_message(&self, me: &Agent, id: i64) -> Result<Message> {
        let me = me.handle.clone();
        self.db()
            .read(move |store, swarm| {
                let message = store
                    .message(swarm, id)?
                    .ok_or_else(|| RuleError::message_not_found(id))?;
                let channel = store.channel(swarm, &message.channel_id)?;
                let member = match &channel {
                    Some(c) => store.is_member(swarm, &c.id, &me)?,
                    None => false,
                };
                match access::decide(
                    agent_view_of(channel.as_ref()),
                    // Reading needs membership only, so ownership does not matter.
                    Who::Agent {
                        member,
                        owner: false,
                    },
                    Need::Membership,
                ) {
                    Decision::Allowed => Ok(Message::from(message)),
                    Decision::NotMember => Err(RuleError::not_member(&message.channel_id)),
                    Decision::NotOwner => Err(RuleError::not_owner(&message.channel_id)),
                    // A message in a hidden channel does not exist, as far as `me` can tell.
                    Decision::NotFound => Err(RuleError::message_not_found(id)),
                }
            })
            .await
    }

    /// Full-text search over the channels `me` is in, newest first.
    pub async fn search(&self, me: &Agent, q: SearchQuery) -> Result<Vec<SearchHit>> {
        let words: Vec<String> = q
            .query
            .as_deref()
            .unwrap_or("")
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let phrase = q
            .phrase
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string);
        if words.is_empty() && phrase.is_none() {
            // A call with only `query` gets the answer it always has.
            return Err(RuleError::invalid(match (&q.query, &q.phrase) {
                (Some(_), None) => "search `query` must not be empty",
                _ => "search needs words in `query` or `phrase`",
            }));
        }
        let since = search_time("since", q.since.as_deref())?;
        let until = search_time("until", q.until.as_deref())?;
        let me = me.handle.clone();
        self.db()
            .read(move |store, swarm| {
                if let Some(ch) = &q.channel_id {
                    member_channel(store, swarm, &me, ch)?;
                }
                let hits = store.search(&SearchFilter {
                    scope: access::scope(swarm, Viewer::Agent(&me), Need::Membership),
                    words,
                    phrase,
                    channel_id: q.channel_id,
                    from_agent: q.from_agent,
                    after_id: q.after_id,
                    before_id: q.before_id,
                    since,
                    until,
                    limit: q.limit,
                })?;
                Ok(hits.into_iter().map(SearchHit::from).collect())
            })
            .await
    }

    /// The channels `me` is in that have messages newer than `after_id`,
    /// busiest-most-recently first.
    pub async fn activity(&self, me: &Agent, after_id: i64) -> Result<Vec<ActivityRow>> {
        let me = me.handle.clone();
        self.db()
            .read(move |store, swarm| {
                let scope = access::scope(swarm, Viewer::Agent(&me), Need::Membership);
                let rows = store.activity(&scope, after_id)?;
                Ok(rows.into_iter().map(ActivityRow::from).collect())
            })
            .await
    }

    /// Returns messages newer than `after_id` in `me`'s channels (or the listed
    /// ones). If there are none, waits until one arrives or the time runs out:
    /// `q.timeout` cut to `limits.max_wait`, or `limits.default_wait` if none.
    pub async fn wait_for_messages(&self, me: &Agent, q: WaitQuery) -> Result<WaitResult> {
        let timeout = wait_timeout(self.limits(), q.timeout);
        let filter = match &q.channel_ids {
            None => None,
            Some(ids) if ids.is_empty() => {
                return Err(RuleError::invalid(
                    "`channel_ids` must list at least one channel, or be left out",
                ));
            }
            Some(ids) => {
                let who = me.handle.clone();
                let to_check = ids.clone();
                self.db()
                    .read(move |store, swarm| {
                        for id in &to_check {
                            member_channel(store, swarm, &who, id)?;
                        }
                        Ok::<_, RuleError>(())
                    })
                    .await?;
                Some(ids.clone())
            }
        };

        let grace = Span::from_std(self.limits().waiting_grace)
            .map_err(|e| RuleError::invalid(e.to_string()))?;
        let guard =
            self.inner
                .waiting
                .start(&me.handle, filter.clone(), self.inner.deps.clock.now());
        let result: Result<WaitResult> = async {
            // Subscribe before the first look, so a message sent in between is not missed.
            let mut notices = self.events().subscribe();
            let clock = &self.inner.deps.clock;
            let timeout = Span::from_std(timeout)
                .map_err(|e| RuleError::invalid(format!("`timeout_s` is out of range: {e}")))?;
            let mut deadline = clock.sleep_until(clock.now() + timeout);
            loop {
                let found = self
                    .newer_messages(me, q.after_id, filter.clone(), q.limit)
                    .await?;
                if !found.messages.is_empty() {
                    return Ok(found);
                }
                loop {
                    tokio::select! {
                        () = &mut deadline => return Ok(found),
                        notice = notices.next() => match notice {
                            Some(Notice::Message(m)) if m.id > q.after_id => break,
                            Some(Notice::Message(_older)) => continue,
                            // Missed some notices; look in the database instead.
                            Some(Notice::Missed(_)) => break,
                            // New and hidden agents, channels and members are for
                            // the web page; a waiting agent wants messages.
                            Some(
                                Notice::Agent(_)
                                | Notice::Channel(_)
                                | Notice::Joined(_)
                                | Notice::Left(_)
                                | Notice::ChannelHidden(_)
                                | Notice::AgentHidden(_),
                            ) => continue,
                            // The server is shutting down.
                            None => return Ok(found),
                        },
                    }
                }
            }
        }
        .await;
        if result.is_ok() {
            guard.finish(self.inner.deps.clock.now() + grace);
        }
        let mut result = result?;
        // Looked up as the wait returns, so a version saved while it waited
        // is seen. A new version does not end the wait: only messages do.
        let handle = me.handle.clone();
        result.newer_lane = self
            .db()
            .read(move |store, swarm| lanes::newer_lane(store, swarm, &handle))
            .await?;
        Ok(result)
    }

    async fn newer_messages(
        &self,
        me: &Agent,
        after_id: i64,
        filter: Option<Vec<String>>,
        limit: usize,
    ) -> Result<WaitResult> {
        let me = me.handle.clone();
        self.db()
            .read(move |store, swarm| {
                let filter = NewerFilter {
                    scope: access::scope(swarm, Viewer::Agent(&me), Need::Membership),
                    after_id,
                    channel_ids: filter,
                };
                let messages = store.newer_messages(&filter, limit)?;
                let total = store.count_newer_messages(&filter)?;
                Ok(WaitResult {
                    more_waiting: total - messages.len() as i64,
                    messages: messages.into_iter().map(Message::from).collect(),
                    newer_lane: None,
                })
            })
            .await
    }
}

/// Reads one page of a channel. The caller has already checked access.
pub(super) fn read_channel(
    store: &dyn Reads,
    swarm: &SwarmId,
    q: &ReadQuery,
) -> Result<MessagePage> {
    let (range, ascending) = match q.position {
        Position::After(id) => (MessageRange::After(id), true),
        Position::Before(id) => (MessageRange::Before(id), false),
        Position::Latest => (MessageRange::Latest, false),
    };
    let mut messages: Vec<Message> = store
        .channel_messages(
            swarm,
            &q.channel_id,
            q.from_agent.as_deref(),
            range,
            q.limit + 1,
        )?
        .into_iter()
        .map(Message::from)
        .collect();
    let has_more = messages.len() > q.limit;
    messages.truncate(q.limit);
    if !ascending {
        messages.reverse();
    }
    let more = match (has_more, ascending) {
        (false, _) => None,
        (true, true) => messages.last().map(|m| Position::After(m.id)),
        (true, false) => messages.first().map(|m| Position::Before(m.id)),
    };
    Ok(MessagePage { messages, more })
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub channel_id: String,
    pub channel_name: String,
    pub agent: String,
    pub body: String,
    pub created_at: String,
    /// The server run that stored the message, if it was recorded.
    pub run_id: Option<i64>,
}

/// Where to read from in a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    /// The newest messages.
    Latest,
    /// Messages with IDs above this one, oldest first.
    After(i64),
    /// Messages with IDs below this one, the nearest first.
    Before(i64),
}

#[derive(Clone, Debug)]
pub struct ReadQuery {
    pub channel_id: String,
    pub position: Position,
    pub from_agent: Option<String>,
    pub limit: usize,
}

/// Messages in ID order, plus where to continue if more exist in the
/// direction being read.
#[derive(Clone, Debug, Serialize)]
pub struct MessagePage {
    pub messages: Vec<Message>,
    #[serde(skip)]
    pub more: Option<Position>,
}

/// Converts a search's `since` or `until` time, as given, to a stored time
/// that compares with message times as text. Stored times are whole seconds,
/// so a time with a fraction rounds up to the next whole second: a message is
/// at or after `14:00:00.5` exactly when it is at or after `14:00:01`.
fn search_time(parameter: &str, given: Option<&str>) -> Result<Option<String>> {
    let Some(text) = given else {
        return Ok(None);
    };
    let Some(time) = clock::from_text(text.trim()) else {
        return Err(RuleError::invalid(format!(
            "search `{parameter}` is not an RFC 3339 time, such as 2026-10-30T14:00:00Z"
        )));
    };
    let whole = time.whole_seconds();
    let rounded = if whole < time {
        whole + Span::seconds(1)
    } else {
        whole
    };
    Ok(Some(clock::to_text(rounded)))
}

#[derive(Clone, Debug)]
pub struct SearchQuery {
    /// Words that must all appear.
    pub query: Option<String>,
    /// Words that must appear together and in this order.
    pub phrase: Option<String>,
    pub channel_id: Option<String>,
    pub from_agent: Option<String>,
    pub after_id: Option<i64>,
    pub before_id: Option<i64>,
    /// An RFC 3339 time, as the agent gave it.
    pub since: Option<String>,
    /// An RFC 3339 time, as the agent gave it.
    pub until: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchHit {
    pub message_id: i64,
    pub channel_id: String,
    pub channel_name: String,
    pub agent: String,
    pub created_at: String,
    pub excerpt: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActivityRow {
    pub channel_id: String,
    pub channel_name: String,
    pub new_messages: i64,
    pub latest_id: i64,
}

/// How long a wait lasts: `asked` cut to `limits.max_wait`, or
/// `limits.default_wait` when nothing was asked.
fn wait_timeout(limits: &Limits, asked: Option<std::time::Duration>) -> std::time::Duration {
    match asked {
        Some(t) => t.min(limits.max_wait),
        None => limits.default_wait,
    }
}

#[derive(Clone, Debug)]
pub struct WaitQuery {
    pub after_id: i64,
    pub channel_ids: Option<Vec<String>>,
    /// How long to wait, cut to `limits.max_wait`. `None` waits
    /// `limits.default_wait`.
    pub timeout: Option<std::time::Duration>,
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct WaitResult {
    pub messages: Vec<Message>,
    /// Newer messages beyond the ones returned.
    pub more_waiting: i64,
    /// The lane the waiting agent holds, when it has a newer version than
    /// the one the agent took. Always `None` for the admin endpoint.
    pub newer_lane: Option<AgentLane>,
}

impl From<MessageRecord> for Message {
    fn from(m: MessageRecord) -> Message {
        Message {
            id: m.id,
            channel_id: m.channel_id,
            channel_name: m.channel_name,
            agent: m.agent,
            body: m.body,
            created_at: m.created_at,
            run_id: m.run_id,
        }
    }
}

impl From<SearchHitRecord> for SearchHit {
    fn from(h: SearchHitRecord) -> SearchHit {
        SearchHit {
            message_id: h.message_id,
            channel_id: h.channel_id,
            channel_name: h.channel_name,
            agent: h.agent,
            created_at: h.created_at,
            excerpt: h.excerpt,
        }
    }
}

impl From<ActivityRecord> for ActivityRow {
    fn from(r: ActivityRecord) -> ActivityRow {
        ActivityRow {
            channel_id: r.channel_id,
            channel_name: r.channel_name,
            new_messages: r.new_messages,
            latest_id: r.latest_id,
        }
    }
}
