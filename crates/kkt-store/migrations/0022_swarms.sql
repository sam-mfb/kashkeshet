-- Swarms: the groups that avatars and channels belong to. Every lookup of
-- an avatar or a channel is bound to one swarm. A server in key mode has
-- exactly one, `sw_main`, made here; every existing avatar and channel
-- joins it. A channel's memberships, messages and note, and an avatar's
-- memory, belong to the swarm of their channel or avatar.
--
-- The swarm made here has no real creation time or server run: a migration
-- reads no clock and runs before the run is recorded. It carries the
-- sentinels 1970-01-01T00:00:00Z and run 0, which no server run has (their
-- IDs start at 1). Swarms made later record their time and run.
CREATE TABLE swarms (
    id TEXT PRIMARY KEY NOT NULL,
    created_at TEXT NOT NULL,
    run_id INTEGER NOT NULL
);
CREATE TRIGGER swarms_no_delete BEFORE DELETE ON swarms
BEGIN SELECT RAISE(ABORT, 'swarms are not deleted'); END;

INSERT INTO swarms (id, created_at, run_id) VALUES ('sw_main', '1970-01-01T00:00:00Z', 0);

ALTER TABLE agents ADD COLUMN swarm TEXT NOT NULL DEFAULT 'sw_main' REFERENCES swarms(id);
ALTER TABLE channels ADD COLUMN swarm TEXT NOT NULL DEFAULT 'sw_main' REFERENCES swarms(id);

-- An avatar or a channel never moves to another swarm.
CREATE TRIGGER agents_swarm_fixed BEFORE UPDATE OF swarm ON agents
WHEN NEW.swarm IS NOT OLD.swarm
BEGIN SELECT RAISE(ABORT, 'an avatar never moves to another swarm'); END;
CREATE TRIGGER channels_swarm_fixed BEFORE UPDATE OF swarm ON channels
WHEN NEW.swarm IS NOT OLD.swarm
BEGIN SELECT RAISE(ABORT, 'a channel never moves to another swarm'); END;
