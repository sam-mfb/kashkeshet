-- The admin agent that saved each version, such as `admin-sentinel`, or NULL
-- when the human operator saved it. Every version saved before this column
-- was the human's: only the web page could save them.
ALTER TABLE operator_instructions ADD COLUMN saved_by_admin TEXT;
