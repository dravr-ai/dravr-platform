-- ABOUTME: Drops the fact embedding column and the embedding usage table, both unwritten since 9cecc68a4
-- ABOUTME: The SQLite half; the Postgres file carries the reasoning

-- Nothing computes an embedding any more. `9cecc68a4` replaced a cosine
-- threshold with the extractor's own judgement — measured on two vendors'
-- models, two different race goals score closer to each other than one goal
-- restated in another language scores to itself, so no threshold separated
-- them — and deleted the provider that produced the vectors along with it.
--
-- What is left is an empty column and an empty table that every future reader
-- of this schema would have to ask about. Both go: the column was written by
-- the fact writers, which no longer pass one, and the table by an instrumented
-- provider that no longer exists.
--
-- Safe to drop rather than keep: no row anywhere carries a value. The column
-- has been NULL on every backend since it was added — the model the provider
-- called (`text-embedding-004`) has answered 404 for longer than the code has
-- been wired, so not one vector was ever stored.
--
-- SQLite has supported ALTER TABLE ... DROP COLUMN since 3.35; this repo
-- bundles 3.51. The column carries no index, and coach_notes keeps its own
-- embedding, which the compaction path still writes.
ALTER TABLE user_facts DROP COLUMN embedding;  -- idempotency-ok: SQLite has no IF EXISTS for DROP COLUMN (syntax error, tested on 3.51); the column exists on every database the 2026-06-17 migration created, and this is the first statement anywhere to remove it

DROP INDEX IF EXISTS idx_embedding_usage_tenant_user_created;
DROP TABLE IF EXISTS embedding_usage;
