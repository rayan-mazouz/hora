-- What kind of failure a check or an incident reports (`probe::FailureKind`
-- codes: 'timeout', 'http', 'content', 'pushed', ...), next to the detail in
-- `error`. The public page maps the kind to its safe words instead of
-- guessing them from the wording of the detail.
--
-- Nullable, no default, no CHECK: ADD COLUMN then only rewrites the schema,
-- O(1) however large `checks` is. No backfill: rows written before keep NULL
-- and go through the old wording classifier, which still fails safe.
ALTER TABLE checks ADD COLUMN reason TEXT;
ALTER TABLE incidents ADD COLUMN reason TEXT;
