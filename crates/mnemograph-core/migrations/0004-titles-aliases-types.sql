-- Titles and aliases are compared by a normalized key (case-insensitive, whitespace collapsed);
-- the keys are filled in by the migration hook, which also creates the unique indexes.
ALTER TABLE page ADD COLUMN title_key TEXT;
ALTER TABLE page ADD COLUMN page_type TEXT;

CREATE TABLE alias (
    key     TEXT    PRIMARY KEY,
    alias   TEXT    NOT NULL,
    page_id INTEGER NOT NULL REFERENCES page (id) ON DELETE CASCADE
) STRICT;

CREATE INDEX alias_page ON alias (page_id);

-- A revision is a snapshot of everything about a page: title, type, aliases and body.
-- Earlier revisions get the page's current title.
ALTER TABLE revision ADD COLUMN title TEXT NOT NULL DEFAULT '';
ALTER TABLE revision ADD COLUMN page_type TEXT;
ALTER TABLE revision ADD COLUMN aliases TEXT NOT NULL DEFAULT ''; -- newline-separated
UPDATE revision SET title = (SELECT title FROM page WHERE page.id = revision.page_id);

-- Links are derived from the current revisions; the hook extracts them again.
DROP TABLE link;
CREATE TABLE link (
    source_page_id INTEGER NOT NULL REFERENCES page (id) ON DELETE CASCADE,
    target_title   TEXT    NOT NULL, -- as written
    target_key     TEXT    NOT NULL,
    position       INTEGER NOT NULL,
    PRIMARY KEY (source_page_id, target_key)
) STRICT;

CREATE INDEX link_target ON link (target_key);
