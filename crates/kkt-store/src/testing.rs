//! What every store must do with swarms, written over [`Reads`] and
//! [`Writes`] alone, so any implementation of the contract can be checked:
//! a swarm sees and changes nothing of another.
//!
//! [`swarms_stay_apart`] seeds one swarm with rows of every sort and asks
//! every read that takes a swarm, in that swarm, many ways, with the
//! handles, channel IDs and message IDs of both swarms. It then seeds a
//! second swarm the same way and asks again: every answer must be as it
//! was. Last, it makes every change in the first swarm aimed at the
//! second's rows: the second must read as it did, and each change that
//! would tie a new row to the second must fail.
//!
//! Two reads look across swarms by design and are left out:
//! [`Reads::agent_name_taken`] and [`Reads::channel_id_taken`], since
//! handles and channel IDs are unique on the server. Message IDs are one
//! sequence for the server, so the second swarm's messages leave gaps in
//! the first's; the check seeds the first swarm completely before the
//! second, so its IDs are the same in both rounds.

use super::*;
use std::fmt::Debug;

const AT: &str = "2026-11-08T12:00:00Z";

/// One swarm's rows, named after `tag`.
struct Seeded {
    swarm: SwarmId,
    agents: Vec<String>,
    channels: Vec<String>,
    messages: Vec<i64>,
}

/// Checks that two swarms stay apart in `store`, which must hold no
/// avatars or channels yet. `Err` says what leaked.
pub fn swarms_stay_apart(store: &dyn Writes) -> Result<(), String> {
    let fail = |e: StoreError| e.to_string();
    let run_id = store
        .insert_server_run(&NewServerRunRecord {
            started_at: AT.into(),
            version: "test".into(),
            git_commit: "test".into(),
            settings: "test".into(),
            instructions_sha256: "test".into(),
        })
        .map_err(fail)?;
    store.insert_lane("shared-lane", AT, run_id).map_err(fail)?;
    store
        .append_lane_version(&NewLaneVersionRecord {
            lane: "shared-lane".into(),
            description: "A lane".into(),
            text: "Do the work.".into(),
            saved_at: AT.into(),
            run_id,
            saved_by_admin: None,
            source: None,
        })
        .map_err(fail)?;

    let home = SwarmId::new("sw_home");
    let away = SwarmId::new("sw_away");
    for swarm in [&home, &away] {
        store.insert_swarm(swarm, AT, run_id).map_err(fail)?;
    }
    let home_rows = seed(store, &home, "home", run_id).map_err(fail)?;
    let mut ids = AllIds::default();
    ids.add(&home_rows);
    let away_names = Seeded {
        swarm: away.clone(),
        agents: agent_names("away"),
        channels: channel_ids("away"),
        messages: Vec::new(),
    };
    ids.add(&away_names);
    // Message IDs the second swarm will take.
    let next = home_rows.messages.iter().max().copied().unwrap_or(0);
    ids.messages.extend(next + 1..next + 40);

    let before = every_read(store, &home, &ids).map_err(fail)?;
    for marker in [
        "home-ann",
        "ch_home_open",
        "home secret",
        "home note",
        "home memory",
    ] {
        if !before.contains(marker) {
            return Err(format!("the home swarm's own reads never show {marker:?}"));
        }
    }
    let away_rows = seed(store, &away, "away", run_id).map_err(fail)?;
    let after = every_read(store, &home, &ids).map_err(fail)?;
    if let Some((was, now)) = first_difference(&before, &after) {
        return Err(format!(
            "a read in one swarm changed when another swarm was seeded:\n  before: {was}\n  after:  {now}"
        ));
    }
    let shows_away = |line: &&str| {
        line.split_once(" = ")
            .is_some_and(|(_, a)| a.contains("away"))
    };
    if let Some(line) = after.lines().find(shows_away) {
        return Err(format!("a read in one swarm shows another's rows: {line}"));
    }

    let away_before = every_read(store, &away, &ids).map_err(fail)?;
    changes_aimed_across(store, &home_rows, &away_rows, run_id)?;
    let away_after = every_read(store, &away, &ids).map_err(fail)?;
    if let Some((was, now)) = first_difference(&away_before, &away_after) {
        return Err(format!(
            "a change in one swarm reached another:\n  before: {was}\n  after:  {now}"
        ));
    }
    Ok(())
}

fn agent_names(tag: &str) -> Vec<String> {
    ["ann", "bob", "hid", "rev"]
        .iter()
        .map(|n| format!("{tag}-{n}"))
        .collect()
}

fn channel_ids(tag: &str) -> Vec<String> {
    ["open", "closed", "hidden", "ownerless"]
        .iter()
        .map(|n| format!("ch_{tag}_{n}"))
        .collect()
}

/// Avatars `<tag>-ann` (owner of two channels, with a lane, a client, a
/// description and memory), `<tag>-bob`, `<tag>-hid` (hidden) and
/// `<tag>-rev` (revoked); an open, a closed, a hidden and an ownerless
/// channel, with notes, members and messages from avatars and the operator.
fn seed(store: &dyn Writes, swarm: &SwarmId, tag: &str, run_id: i64) -> StoreResult<Seeded> {
    let agents = agent_names(tag);
    let channels = channel_ids(tag);
    let [ann, bob, hid, rev] = [&agents[0], &agents[1], &agents[2], &agents[3]];
    let [open, closed, hidden, ownerless] =
        [&channels[0], &channels[1], &channels[2], &channels[3]];
    for name in &agents {
        store.insert_agent(
            swarm,
            &NewAgentRecord {
                name: name.clone(),
                token_hash: format!("token of {name}").into_bytes(),
                recovery_hash: Some(format!("code of {name}").into_bytes()),
                created_at: AT.into(),
                last_seen: Some(AT.into()),
                run_id,
            },
        )?;
    }
    store.set_description(swarm, ann, Some(&format!("{tag} describes itself")))?;
    store.set_agent_client(
        swarm,
        ann,
        Some(&AgentClient {
            name: format!("{tag} client"),
            version: "1".into(),
            recorded_at: AT.into(),
            run_id,
        }),
    )?;
    store.set_agent_lane(
        swarm,
        ann,
        Some(&TakenLaneRecord {
            lane: "shared-lane".into(),
            version: 1,
            taken_at: AT.into(),
            run_id,
        }),
    )?;
    store.save_memory_entry(
        swarm,
        ann,
        &MemoryEntryRecord {
            name: "handoff".into(),
            text: format!("{tag} memory"),
            saved_at: AT.into(),
            run_id: Some(run_id),
        },
    )?;
    store.set_revoked(swarm, rev, AT)?;
    for (id, open_to_all, owner) in [
        (open, true, Some(ann.as_str())),
        (closed, false, Some(ann.as_str())),
        (hidden, false, Some(bob.as_str())),
        (ownerless, true, None),
    ] {
        store.insert_channel(
            swarm,
            &NewChannelRecord {
                id: id.clone(),
                name: format!("{tag} talk"),
                purpose: Some(format!("{tag} purpose")),
                open: open_to_all,
                created_by: owner.unwrap_or("operator").into(),
                created_at: AT.into(),
                owner: owner.map(str::to_string),
                run_id,
            },
        )?;
        store.set_note(
            swarm,
            id,
            &NoteEditRecord {
                text: format!("{tag} note"),
                editor: ann.clone(),
                at: AT.into(),
                run_id,
            },
        )?;
    }
    for (id, members) in [
        (open, vec![ann, bob, hid, rev]),
        (closed, vec![ann, bob]),
        (hidden, vec![bob]),
        (ownerless, vec![bob, rev]),
    ] {
        for member in members {
            store.add_member(swarm, id, member, AT)?;
        }
    }
    let mut messages = Vec::new();
    for (id, from) in [
        (open, ann.as_str()),
        (open, bob),
        (open, "operator"),
        (closed, ann),
        (hidden, bob),
        (ownerless, rev),
        (open, hid),
    ] {
        messages.push(store.insert_message(
            swarm,
            &NewMessageRecord {
                channel_id: id.clone(),
                agent: from.into(),
                body: format!("{tag} secret hello from {from}"),
                created_at: AT.into(),
                run_id,
            },
        )?);
    }
    store.hide_channel(swarm, hidden)?;
    store.hide_agent(swarm, hid, AT)?;
    Ok(Seeded {
        swarm: swarm.clone(),
        agents,
        channels,
        messages,
    })
}

/// Every handle, channel ID and message ID of both swarms, to ask about.
#[derive(Default)]
struct AllIds {
    agents: Vec<String>,
    channels: Vec<String>,
    messages: Vec<i64>,
}

impl AllIds {
    fn add(&mut self, rows: &Seeded) {
        self.agents.extend(rows.agents.iter().cloned());
        self.channels.extend(rows.channels.iter().cloned());
        self.messages.extend(rows.messages.iter().copied());
    }
}

/// Every read that takes a swarm, asked in `swarm` about every ID in
/// `ids`, one answer per line.
fn every_read(store: &dyn Reads, swarm: &SwarmId, ids: &AllIds) -> StoreResult<String> {
    let mut out = String::new();
    let mut line = |question: String, answer: &dyn Debug| {
        out.push_str(&format!("{question} = {answer:?}\n"));
    };
    let s = swarm;
    line("active_agent_count".into(), &store.active_agent_count(s)?);
    line("agents_by_last_seen".into(), &store.agents_by_last_seen(s)?);
    line("agents".into(), &store.agents(s, None, None, None, 100)?);
    line(
        "agents with lane".into(),
        &store.agents(s, None, Some("shared-lane"), None, 100)?,
    );
    line("all_channel_ids".into(), &store.all_channel_ids(s, None)?);
    line("hidden_channel_ids".into(), &store.hidden_channel_ids(s)?);
    line("recent_messages".into(), &store.recent_messages(s, 100)?);
    let mut uses = store.channel_use(s)?;
    uses.sort_by(|a, b| a.channel_id.cmp(&b.channel_id));
    line("channel_use".into(), &uses);
    line("memberships".into(), &store.memberships(s)?);
    line(
        "message_times_since".into(),
        &store.message_times_since(s, "")?,
    );
    line(
        "any_channel_ids".into(),
        &store.any_channel_ids(s, None, None, 100)?,
    );
    line("any_activity".into(), &store.any_activity(s, 0)?);
    line(
        "any_newer_messages".into(),
        &store.any_newer_messages(s, 0, None, 100)?,
    );
    line(
        "count_any_newer_messages".into(),
        &store.count_any_newer_messages(s, 0, None)?,
    );
    line(
        "any_newer_messages in every channel".into(),
        &store.any_newer_messages(s, 0, Some(&ids.channels), 100)?,
    );
    line(
        "count_any_newer_messages in every channel".into(),
        &store.count_any_newer_messages(s, 0, Some(&ids.channels))?,
    );
    for agent in &ids.agents {
        line(format!("agent {agent}"), &store.agent(s, agent)?);
        let token = format!("token of {agent}").into_bytes();
        line(
            format!("agent_by_token_hash {agent}"),
            &store.agent_by_token_hash(s, &token)?,
        );
        let code = format!("code of {agent}").into_bytes();
        line(
            format!("agent_by_recovery_hash {agent}"),
            &store.agent_by_recovery_hash(s, agent, &code)?,
        );
        line(
            format!("memory_entries {agent}"),
            &store.memory_entries(s, agent)?,
        );
        line(
            format!("memory_entry {agent}"),
            &store.memory_entry(s, agent, "handoff")?,
        );
        line(
            format!("message_count_from {agent}"),
            &store.message_count_from(s, agent)?,
        );
        line(
            format!("owned_channel_ids {agent}"),
            &store.owned_channel_ids(s, agent)?,
        );
        line(
            format!("all_channel_ids of {agent}"),
            &store.all_channel_ids(s, Some(agent))?,
        );
        line(
            format!("agents named {agent}"),
            &store.agents(s, Some(agent), None, None, 100)?,
        );
    }
    for id in &ids.channels {
        line(format!("channel {id}"), &store.channel(s, id)?);
        line(format!("any_channel {id}"), &store.any_channel(s, id)?);
        line(
            format!("hidden_channel_summary {id}"),
            &store.hidden_channel_summary(s, id)?,
        );
        line(
            format!("channel_summary {id}"),
            &store.channel_summary(s, id, None, Some(0))?,
        );
        line(format!("member_count {id}"), &store.member_count(s, id)?);
        line(
            format!("members_by_join {id}"),
            &store.members_by_join(s, id, 100)?,
        );
        line(
            format!("members_by_name {id}"),
            &store.members_by_name(s, id, None, 100)?,
        );
        line(
            format!("channel_messages {id}"),
            &store.channel_messages(s, id, None, MessageRange::After(0), 100)?,
        );
        line(
            format!("any_channel_ids {id}"),
            &store.any_channel_ids(s, Some(id), None, 100)?,
        );
        for agent in &ids.agents {
            line(
                format!("is_member {id} {agent}"),
                &store.is_member(s, id, agent)?,
            );
            line(
                format!("channel_summary {id} as {agent}"),
                &store.channel_summary(s, id, Some(agent), Some(0))?,
            );
        }
    }
    for id in &ids.messages {
        line(format!("message {id}"), &store.message(s, *id)?);
    }
    let viewers = std::iter::once(None).chain(ids.agents.iter().map(|a| Some(a.clone())));
    for viewer in viewers {
        for kind in ChannelKind::all() {
            let scope = ChannelScope::new(s.clone(), viewer.clone(), |k| k == kind);
            let q = |what: &str| format!("{what} as {viewer:?} for {kind:?}");
            line(q("activity"), &store.activity(&scope, 0)?);
            line(
                q("channel_ids"),
                &store.channel_ids(&ChannelFilter {
                    scope: scope.clone(),
                    shared_with: None,
                    text: None,
                    after: None,
                    limit: 100,
                })?,
            );
            let newer = NewerFilter {
                scope: scope.clone(),
                after_id: 0,
                channel_ids: None,
            };
            line(q("newer_messages"), &store.newer_messages(&newer, 100)?);
            line(
                q("count_newer_messages"),
                &store.count_newer_messages(&newer)?,
            );
            line(
                q("search"),
                &store.search(&SearchFilter {
                    scope: scope.clone(),
                    words: vec!["secret".into()],
                    phrase: None,
                    channel_id: None,
                    from_agent: None,
                    after_id: None,
                    before_id: None,
                    since: None,
                    until: None,
                    limit: 100,
                })?,
            );
            for agent in &ids.agents {
                line(
                    q(&format!("latest_message_from {agent}")),
                    &store.latest_message_from(&SentFilter {
                        agent: agent.clone(),
                        channel_id: None,
                        scope: scope.clone(),
                    })?,
                );
                line(
                    q(&format!("channel_ids shared with {agent}")),
                    &store.channel_ids(&ChannelFilter {
                        scope: scope.clone(),
                        shared_with: Some(agent.clone()),
                        text: None,
                        after: None,
                        limit: 100,
                    })?,
                );
            }
            for id in &ids.channels {
                let only = NewerFilter {
                    scope: scope.clone(),
                    after_id: 0,
                    channel_ids: Some(vec![id.clone()]),
                };
                line(
                    q(&format!("newer_messages in {id}")),
                    &store.newer_messages(&only, 100)?,
                );
                line(
                    q(&format!("search in {id}")),
                    &store.search(&SearchFilter {
                        scope: scope.clone(),
                        words: vec!["secret".into()],
                        phrase: None,
                        channel_id: Some(id.clone()),
                        from_agent: None,
                        after_id: None,
                        before_id: None,
                        since: None,
                        until: None,
                        limit: 100,
                    })?,
                );
            }
        }
    }
    Ok(out)
}

/// Every change, made in `home`'s swarm and aimed at `away`'s rows. Changes
/// to `away`'s rows must change nothing (the caller checks); changes that
/// would add a row tied to `away` must fail.
fn changes_aimed_across(
    store: &dyn Writes,
    home: &Seeded,
    away: &Seeded,
    run_id: i64,
) -> Result<(), String> {
    let s = &home.swarm;
    let mine_agent = &home.agents[0];
    let mine_channel = &home.channels[0];
    let fail = |e: StoreError| e.to_string();
    for agent in &away.agents {
        store.set_token_hash(s, agent, b"stolen").map_err(fail)?;
        store.set_recovery_hash(s, agent, b"stolen").map_err(fail)?;
        store
            .set_last_seen(s, agent, "2030-01-01T00:00:00Z")
            .map_err(fail)?;
        store
            .set_description(s, agent, Some("changed"))
            .map_err(fail)?;
        store.set_agent_client(s, agent, None).map_err(fail)?;
        store.set_agent_lane(s, agent, None).map_err(fail)?;
        store
            .set_revoked(s, agent, "2030-01-01T00:00:00Z")
            .map_err(fail)?;
        store
            .delete_memory_entry(s, agent, "handoff")
            .map_err(fail)?;
        store
            .hide_agent(s, agent, "2030-01-01T00:00:00Z")
            .map_err(fail)?;
        for channel in &away.channels {
            store.remove_member(s, channel, agent).map_err(fail)?;
        }
    }
    for channel in &away.channels {
        store
            .set_purpose(s, channel, Some("changed"))
            .map_err(fail)?;
        store.set_channel_open(s, channel, false).map_err(fail)?;
        store.set_owner(s, channel, None).map_err(fail)?;
        store
            .set_note(
                s,
                channel,
                &NoteEditRecord {
                    text: "changed".into(),
                    editor: mine_agent.clone(),
                    at: AT.into(),
                    run_id,
                },
            )
            .map_err(fail)?;
        store.hide_channel(s, channel).map_err(fail)?;
    }

    let refused = |what: &str, result: StoreResult<()>| match result {
        Err(StoreError::OutsideSwarm(_)) => Ok(()),
        Err(other) => Err(format!("{what} across swarms failed, but as {other:?}")),
        Ok(()) => Err(format!("{what} across swarms was accepted")),
    };
    let away_agent = &away.agents[0];
    let away_channel = &away.channels[0];
    refused(
        "adding a member to another swarm's channel",
        store.add_member(s, away_channel, mine_agent, AT),
    )?;
    refused(
        "adding another swarm's avatar to a channel",
        store.add_member(s, mine_channel, away_agent, AT),
    )?;
    let message = |channel: &str, agent: &str| NewMessageRecord {
        channel_id: channel.into(),
        agent: agent.into(),
        body: "across".into(),
        created_at: AT.into(),
        run_id,
    };
    refused(
        "a message in another swarm's channel",
        store
            .insert_message(s, &message(away_channel, mine_agent))
            .map(|_| ()),
    )?;
    refused(
        "an operator message in another swarm's channel",
        store
            .insert_message(s, &message(away_channel, "operator"))
            .map(|_| ()),
    )?;
    refused(
        "a message from another swarm's avatar",
        store
            .insert_message(s, &message(mine_channel, away_agent))
            .map(|_| ()),
    )?;
    refused(
        "memory for another swarm's avatar",
        store.save_memory_entry(
            s,
            away_agent,
            &MemoryEntryRecord {
                name: "handoff".into(),
                text: "changed".into(),
                saved_at: AT.into(),
                run_id: Some(run_id),
            },
        ),
    )?;
    refused(
        "another swarm's avatar as an owner",
        store.set_owner(s, mine_channel, Some(away_agent)),
    )?;
    refused(
        "a channel owned by another swarm's avatar",
        store.insert_channel(
            s,
            &NewChannelRecord {
                id: "ch_across".into(),
                name: "across".into(),
                purpose: None,
                open: true,
                created_by: mine_agent.clone(),
                created_at: AT.into(),
                owner: Some(away_agent.clone()),
                run_id,
            },
        ),
    )?;
    refused(
        "a channel created by another swarm's avatar",
        store.insert_channel(
            s,
            &NewChannelRecord {
                id: "ch_across".into(),
                name: "across".into(),
                purpose: None,
                open: true,
                created_by: away_agent.clone(),
                created_at: AT.into(),
                owner: None,
                run_id,
            },
        ),
    )?;
    Ok(())
}

fn first_difference<'a>(a: &'a str, b: &'a str) -> Option<(&'a str, &'a str)> {
    if a == b {
        return None;
    }
    Some(
        a.lines()
            .zip(b.lines())
            .find(|(x, y)| x != y)
            .unwrap_or(("(the answers differ in length)", "")),
    )
}
