-- Links from a page's current revision to page titles. The target is a title, not a page ID:
-- a title no page has yet is a stub, and it resolves as soon as a page with that title exists.
CREATE TABLE link (
    source_page_id INTEGER NOT NULL REFERENCES page (id) ON DELETE CASCADE,
    target_title   TEXT    NOT NULL,
    position       INTEGER NOT NULL,
    PRIMARY KEY (source_page_id, target_title)
) STRICT;

CREATE INDEX link_target ON link (target_title);
