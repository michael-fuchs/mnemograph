-- What agents read, searched and explored, for the owner's statistics. Entries are deleted
-- after a retention period; nothing else depends on them.
CREATE TABLE access (
    id         INTEGER PRIMARY KEY,
    at         TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    author     TEXT    NOT NULL, -- as in revision.author
    tool       TEXT    NOT NULL CHECK (tool IN ('read_page', 'search', 'related')),
    target     TEXT    NOT NULL, -- the title as requested, or the search terms
    target_key TEXT    NOT NULL, -- target with case and spacing normalized, as title keys
    hits       INTEGER NOT NULL  -- 1 or 0 for read_page, the number of results otherwise
) STRICT;

CREATE INDEX access_at ON access (at);
