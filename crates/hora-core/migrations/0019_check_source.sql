-- Where a check row came from: 'probe' (the scheduler probed or ran it),
-- 'push' (an explicit heartbeat on /api/push/{id}) or 'miss' (the scheduler
-- recorded an overdue heartbeat as down). Push alerting needs to tell an
-- explicit `status=down` push - the job saying it failed, with its message -
-- from a recorded miss without matching the error text.
--
-- Rows written before this migration keep the default 'probe'. For push
-- monitors and peers those were pushes and misses mixed; the heartbeat queries
-- read such legacy rows as before (status != 0 is a heartbeat, status 0 a
-- miss), so no backfill (a full scan of `checks`) is needed.
ALTER TABLE checks ADD COLUMN source TEXT NOT NULL DEFAULT 'probe'
    CHECK (source IN ('probe', 'push', 'miss'));
