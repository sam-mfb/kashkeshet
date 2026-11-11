//! The rules as kkt-api-admin's contract, and as the combined
//! [`OperatorApi`] and [`AdminApi`]: each operation is the rules' own method
//! of the same name.

use super::*;
use kkt_api_admin::{Admin, AdminApi, Swarms};
use kkt_api_agent::Answer;
use std::sync::Arc;

impl Rules {
    /// Makes the swarm `swarm`, recorded as made by this server run. It
    /// starts with no avatars and no channels.
    pub async fn create_swarm(&self, swarm: SwarmId) -> Result<()> {
        let at = self.now_text();
        let run_id = self.inner.deps.run_id;
        self.db()
            .write(move |w, _| {
                w.insert_swarm(&swarm, &at, run_id)?;
                Ok(())
            })
            .await
    }
}

impl Admin for Rules {
    fn admin_activity<'a>(
        &'a self,
        operator: &'a Operator,
        after_id: i64,
    ) -> Answer<'a, Vec<ActivityRow>> {
        Box::pin(Rules::admin_activity(self, operator, after_id))
    }

    fn admin_agent<'a>(&'a self, operator: &'a Operator, handle: String) -> Answer<'a, AgentView> {
        Box::pin(Rules::admin_agent(self, operator, handle))
    }

    fn admin_channel<'a>(
        &'a self,
        operator: &'a Operator,
        channel_id: String,
    ) -> Answer<'a, AdminChannel> {
        Box::pin(Rules::admin_channel(self, operator, channel_id))
    }

    fn admin_list_agents<'a>(
        &'a self,
        operator: &'a Operator,
        query: Option<String>,
        lane: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Page<AgentInfo>> {
        Box::pin(Rules::admin_list_agents(
            self, operator, query, lane, limit, cursor,
        ))
    }

    fn admin_list_channels<'a>(
        &'a self,
        operator: &'a Operator,
        query: Option<String>,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Page<AdminChannel>> {
        Box::pin(Rules::admin_list_channels(
            self, operator, query, limit, cursor,
        ))
    }

    fn admin_list_members<'a>(
        &'a self,
        operator: &'a Operator,
        channel_id: String,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'a, Members> {
        Box::pin(Rules::admin_list_members(
            self, operator, channel_id, limit, cursor,
        ))
    }

    fn admin_list_memories(
        &self,
        handle: String,
        limit: usize,
        cursor: Option<String>,
    ) -> Answer<'_, MemoryList> {
        Box::pin(Rules::admin_list_memories(self, handle, limit, cursor))
    }

    fn admin_message<'a>(&'a self, operator: &'a Operator, id: i64) -> Answer<'a, Message> {
        Box::pin(Rules::admin_message(self, operator, id))
    }

    fn admin_new_messages<'a>(
        &'a self,
        operator: &'a Operator,
        after_id: i64,
        channel_ids: Option<Vec<String>>,
        limit: usize,
    ) -> Answer<'a, WaitResult> {
        Box::pin(Rules::admin_new_messages(
            self,
            operator,
            after_id,
            channel_ids,
            limit,
        ))
    }

    fn admin_note<'a>(
        &'a self,
        operator: &'a Operator,
        channel_id: String,
    ) -> Answer<'a, ChannelNote> {
        Box::pin(Rules::admin_note(self, operator, channel_id))
    }

    fn admin_read<'a>(&'a self, operator: &'a Operator, q: ReadQuery) -> Answer<'a, MessagePage> {
        Box::pin(Rules::admin_read(self, operator, q))
    }

    fn admin_recall(&self, handle: String, name: String) -> Answer<'_, MemoryEntry> {
        Box::pin(Rules::admin_recall(self, handle, name))
    }
}

impl Swarms for Rules {
    fn create_swarm(&self, swarm: SwarmId) -> Answer<'_, ()> {
        Box::pin(Rules::create_swarm(self, swarm))
    }
}

impl OperatorApi for Rules {
    fn in_swarm_operator(&self, swarm: SwarmId) -> Arc<dyn OperatorApi> {
        Arc::new(Rules::in_swarm(self, swarm))
    }
}

impl AdminApi for Rules {
    fn in_swarm_admin(&self, swarm: SwarmId) -> Arc<dyn AdminApi> {
        Arc::new(Rules::in_swarm(self, swarm))
    }
}
