ALTER TABLE agents ADD COLUMN client_name TEXT;
ALTER TABLE agents ADD COLUMN client_version TEXT;
ALTER TABLE agents ADD COLUMN client_recorded_at TEXT;
ALTER TABLE agents ADD COLUMN client_run_id INTEGER REFERENCES server_runs(id);
