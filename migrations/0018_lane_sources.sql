-- Where an imported lane version came from: the folder `kkt lanes import`
-- read, and the git commit checked out there, if it was a git checkout.
-- Both are NULL for a version saved on the web page or at /mcp-admin, as
-- every version from before imports is; the commit is NULL too for a folder
-- outside git.
ALTER TABLE lane_versions ADD COLUMN source_folder TEXT;
ALTER TABLE lane_versions ADD COLUMN source_commit TEXT;
