-- The server run that created each channel, and the one that last edited
-- its note. Channels and notes saved before this have neither.
ALTER TABLE channels ADD COLUMN run_id INTEGER REFERENCES server_runs(id);
ALTER TABLE channels ADD COLUMN note_run_id INTEGER REFERENCES server_runs(id);
