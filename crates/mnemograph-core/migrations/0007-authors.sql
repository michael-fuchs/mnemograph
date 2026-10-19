-- Who created each revision and note, as established by authentication: 'owner:<user>',
-- 'agent:<client id>' or 'anonymous' (authentication disabled). NULL for rows written before
-- authors were recorded.
ALTER TABLE revision ADD COLUMN author TEXT;
ALTER TABLE note ADD COLUMN author TEXT;
