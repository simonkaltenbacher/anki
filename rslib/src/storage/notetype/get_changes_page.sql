-- anki-api extension: cursor-paged notetype change feed query.
-- TODO(api-v1-perf): Add a composite index on notetypes(usn, id) to optimize
-- this cursor query pattern and avoid extra work when many rows share usn.
SELECT id,
  usn,
  mtime_secs
FROM notetypes
WHERE (usn > ?1)
  OR (
    usn = ?1
    AND id > ?2
  )
ORDER BY usn,
  id
LIMIT ?3