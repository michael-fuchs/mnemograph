-- Every page and note belongs to one or more spheres (ADR 0008). The hook gives existing pages
-- and notes the default sphere chosen at the upgrade.
CREATE TABLE page_sphere (
    page_id INTEGER NOT NULL REFERENCES page (id) ON DELETE CASCADE,
    sphere  TEXT    NOT NULL,
    PRIMARY KEY (page_id, sphere)
) STRICT;

CREATE INDEX page_sphere_sphere ON page_sphere (sphere);

CREATE TABLE note_sphere (
    note_id INTEGER NOT NULL REFERENCES note (id) ON DELETE CASCADE,
    sphere  TEXT    NOT NULL,
    PRIMARY KEY (note_id, sphere)
) STRICT;

CREATE INDEX note_sphere_sphere ON note_sphere (sphere);
