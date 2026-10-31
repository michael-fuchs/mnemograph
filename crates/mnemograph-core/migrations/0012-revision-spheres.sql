-- A revision records the page's spheres too, so the history shows when they changed. Earlier
-- revisions get the page's current spheres: they could not change before.
ALTER TABLE revision ADD COLUMN spheres TEXT NOT NULL DEFAULT ''; -- newline-separated, sorted
UPDATE revision SET spheres = coalesce(
    (SELECT group_concat(sphere, char(10))
     FROM (SELECT sphere FROM page_sphere WHERE page_id = revision.page_id ORDER BY sphere)),
    '');
