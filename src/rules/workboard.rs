//! The human's read-only projection of channel messages.
use super::*;
use crate::workboard::{self, Board, MessageInput, ProjectionLimits};

#[derive(Debug)]
pub(crate) struct WorkboardView {
    pub channel: ChannelInfo,
    pub hidden: bool,
    pub now: i64,
    pub board: Option<Board>,
    pub unavailable: Option<String>,
}
impl Rules {
    pub(crate) async fn workboard_view(&self, channel_id: String) -> Result<WorkboardView> {
        let now = self.inner.deps.clock.now().timestamp();
        let preview = self.limits().member_preview;
        self.db()
            .read(move |store| {
                let (channel, hidden) = human_readable_channel(store, &channel_id, preview)?;
                let mut board = workboard::project(&[], now, ProjectionLimits::configured())
                    .map_err(|e| RuleError::invalid(e.0))?;
                let mut cursor = 0;
                loop {
                    let messages = store.channel_messages(
                        &channel_id,
                        None,
                        MessageRange::After(cursor),
                        crate::config::WORKBOARD_PAGE_MESSAGES,
                    )?;
                    if messages.is_empty() {
                        break;
                    }
                    for m in messages {
                        cursor = m.id;
                        if let Err(e) = board.apply(&MessageInput {
                            id: m.id,
                            channel_id: m.channel_id,
                            author: m.agent,
                            body: m.body,
                            created_at: m.created_at,
                        }) {
                            return Ok(WorkboardView {
                                channel,
                                hidden,
                                now,
                                board: None,
                                unavailable: Some(e.0),
                            });
                        }
                    }
                }
                Ok(WorkboardView {
                    channel,
                    hidden,
                    now,
                    board: Some(board),
                    unavailable: None,
                })
            })
            .await
    }
}
