-- Memory: named entries an avatar keeps for whoever next holds it, in place
-- of the single handoff note. An entry records when it was saved and in
-- which server run; entries made from handoff notes have no run.
CREATE TABLE memory_entries (
    agent TEXT NOT NULL REFERENCES agents(name),
    name TEXT NOT NULL,
    text TEXT NOT NULL,
    saved_at TEXT NOT NULL,
    run_id INTEGER REFERENCES server_runs(id),
    PRIMARY KEY (agent, name)
);

-- Each handoff note with text becomes an entry named `handoff`, keeping its
-- text and the time it was set. The old columns stay, unused.
INSERT INTO memory_entries (agent, name, text, saved_at, run_id)
    SELECT name, 'handoff', handoff_note, COALESCE(handoff_note_set_at, created_at), NULL
    FROM agents
    WHERE handoff_note <> '';
