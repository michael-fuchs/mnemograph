-- The spheres of the registration through which an agent read, searched or explored, so the
-- statistics can be shown per sphere. Entries from before spheres have none.
CREATE TABLE access_sphere (
    access_id INTEGER NOT NULL REFERENCES access (id) ON DELETE CASCADE,
    sphere    TEXT    NOT NULL,
    PRIMARY KEY (access_id, sphere)
) STRICT;

CREATE INDEX access_sphere_sphere ON access_sphere (sphere);
