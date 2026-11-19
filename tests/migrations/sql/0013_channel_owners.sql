-- Each channel's owner: the agent who may remove members and pass the
-- ownership on. NULL for no owner. Existing channels get the agent that
-- created them, unless that agent has been hidden or unregistered, or the
-- operator created the channel.
ALTER TABLE channels ADD COLUMN owner TEXT REFERENCES agents(name);
UPDATE channels SET owner = created_by
WHERE created_by IN (SELECT name FROM agents WHERE hidden = 0);
