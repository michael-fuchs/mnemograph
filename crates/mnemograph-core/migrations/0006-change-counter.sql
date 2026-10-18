-- A counter that every write increases. The search index records the value it reflects, so
-- an index that missed a write (crash, failure) is detected and rebuilt.
CREATE TABLE meta (
    key   TEXT    PRIMARY KEY,
    value INTEGER NOT NULL
) STRICT;

INSERT INTO meta (key, value) VALUES ('change_counter', 0);
