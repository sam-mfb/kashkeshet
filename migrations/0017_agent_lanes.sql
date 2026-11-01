-- The lane an avatar has taken: the lane's name, the version it took, when,
-- and in which server run. All four are NULL while it holds no lane, as
-- every avatar from before lanes does.
ALTER TABLE agents ADD COLUMN lane TEXT REFERENCES lanes(name);
ALTER TABLE agents ADD COLUMN lane_version INTEGER;
ALTER TABLE agents ADD COLUMN lane_taken_at TEXT;
ALTER TABLE agents ADD COLUMN lane_run_id INTEGER REFERENCES server_runs(id);
CREATE INDEX agents_by_lane ON agents(lane);
