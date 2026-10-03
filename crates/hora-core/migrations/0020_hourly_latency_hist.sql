-- A latency histogram per hourly bucket (see `hora_core::histogram`), so the
-- 24h p50/p95/p99 merge two dozen small histograms instead of sorting a day
-- of raw checks per monitor (~20 s on a 575M-row database).
--
-- Nullable, no default, no CHECK: ADD COLUMN then only rewrites the schema,
-- O(1) however large the table. NULL means "not computed": buckets rolled up
-- before this migration keep it, readers fall back to the raw checks for
-- those hours, and the roll-up task fills in the last day's on its next tick
-- (an hour of raw rows each - never a scan of the whole history). An hour
-- without any latency sample stores the empty histogram, not NULL.
ALTER TABLE checks_hourly ADD COLUMN latency_hist BLOB;

-- The roll-up frontier (`MAX(hour)`, read by every status-page refresh and
-- every roll-up) and the hour-range reads seek on `hour` alone; the
-- monitor-led keys made both a full index scan. A few seconds to build on
-- millions of buckets, once.
CREATE INDEX IF NOT EXISTS idx_checks_hourly_hour ON checks_hourly (hour);

-- The history, timeline, feed and report read the newest incidents
-- (`ORDER BY started_at DESC, id DESC LIMIT n`, `started_at < ?`): without
-- an index on `started_at` every one of them scanned and sorted the whole
-- table. Incidents are few (thousands), so this builds in milliseconds.
CREATE INDEX IF NOT EXISTS idx_incidents_started ON incidents (started_at DESC, id DESC);
