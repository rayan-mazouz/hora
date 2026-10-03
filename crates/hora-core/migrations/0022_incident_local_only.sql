-- Where an incident's down alert went, so a restart mid-incident routes the
-- recovery the same way: 1 = the down was local-only (every peer that answered
-- saw the target up) and went to `alerts.notify_unconfirmed` only (or nowhere);
-- 0 = it was local-only, then the peers confirmed it and the usual down went
-- out too. NULL = an ordinary down (and every incident written before).
--
-- Nullable, no default, no CHECK: ADD COLUMN only rewrites the schema, O(1)
-- however large `incidents` is. No backfill.
ALTER TABLE incidents ADD COLUMN local_only INTEGER;
