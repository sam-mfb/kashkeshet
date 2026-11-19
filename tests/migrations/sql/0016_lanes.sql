-- Lanes: named instructions an agent can take on. A lane is never deleted,
-- only retired. Its description and instructions live in its versions.
CREATE TABLE lanes (
    name TEXT PRIMARY KEY NOT NULL,
    created_at TEXT NOT NULL,
    run_id INTEGER NOT NULL REFERENCES server_runs(id),
    -- When it was retired, by whom (the admin agent, or NULL for the human
    -- operator) and in which run. All three are NULL while it is current.
    retired_at TEXT,
    retired_by_admin TEXT,
    retired_run_id INTEGER REFERENCES server_runs(id)
);
CREATE TRIGGER lanes_no_delete BEFORE DELETE ON lanes
BEGIN SELECT RAISE(ABORT, 'lanes are retired, not deleted'); END;

-- Every save of a lane is a new immutable version, numbered from 1 per lane.
CREATE TABLE lane_versions (
    lane TEXT NOT NULL REFERENCES lanes(name),
    version INTEGER NOT NULL CHECK (version > 0),
    description TEXT NOT NULL,
    text TEXT NOT NULL,
    saved_at TEXT NOT NULL,
    run_id INTEGER NOT NULL REFERENCES server_runs(id),
    -- The admin agent that saved it, such as `admin-sentinel`, or NULL when
    -- the human operator did.
    saved_by_admin TEXT,
    PRIMARY KEY (lane, version)
);
CREATE TRIGGER lane_versions_no_update BEFORE UPDATE ON lane_versions
BEGIN SELECT RAISE(ABORT, 'lane versions are immutable'); END;
CREATE TRIGGER lane_versions_no_delete BEFORE DELETE ON lane_versions
BEGIN SELECT RAISE(ABORT, 'lane versions are immutable'); END;
-- REPLACE performs an implicit delete, which need not fire delete triggers.
CREATE TRIGGER lane_versions_no_replacement BEFORE INSERT ON lane_versions
WHEN EXISTS (SELECT 1 FROM lane_versions WHERE lane = NEW.lane AND version = NEW.version)
BEGIN SELECT RAISE(ABORT, 'lane versions are immutable'); END;
