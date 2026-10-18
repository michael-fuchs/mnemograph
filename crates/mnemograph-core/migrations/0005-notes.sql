-- Notes are append-only: there is no revision history, and nothing updates a note.
CREATE TABLE note (
    id         INTEGER PRIMARY KEY,
    body       TEXT    NOT NULL,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Links from a note to titles; like page links, they attach the note to pages and stubs.
CREATE TABLE note_link (
    note_id      INTEGER NOT NULL REFERENCES note (id) ON DELETE CASCADE,
    target_title TEXT    NOT NULL, -- as written
    target_key   TEXT    NOT NULL,
    position     INTEGER NOT NULL,
    PRIMARY KEY (note_id, target_key)
) STRICT;

CREATE INDEX note_link_target ON note_link (target_key);
