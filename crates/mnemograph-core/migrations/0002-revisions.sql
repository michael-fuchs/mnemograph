-- Every change to a page creates a revision; the page itself only keeps its identity.
CREATE TABLE revision (
    id         INTEGER PRIMARY KEY,
    page_id    INTEGER NOT NULL REFERENCES page (id) ON DELETE CASCADE,
    number     INTEGER NOT NULL CHECK (number >= 1),
    body       TEXT    NOT NULL,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (page_id, number)
) STRICT;

INSERT INTO revision (page_id, number, body, created_at)
SELECT id, 1, body, updated_at FROM page;

ALTER TABLE page DROP COLUMN body;
ALTER TABLE page DROP COLUMN updated_at;
