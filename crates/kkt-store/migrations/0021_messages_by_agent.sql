-- Messages by sender, newest last, so an agent's latest message (the
-- `last_sent` in agent and member lists) is found without reading every
-- message.
CREATE INDEX messages_by_agent ON messages(agent, id);
