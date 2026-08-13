-- ABOUTME: messaging_channel_links.proposed_coach_ids — what the coach proposal actually offered (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration; JSONB would buy nothing since the column is read whole

-- See the matching SQLite migration for the rationale. Summary: "Reply with a
-- number to start" was unimplemented, and resolving the number needs the ordered
-- list the user is looking at — it cannot be re-derived because the proposal is
-- LLM-re-ranked and could come back in a different order, binding the wrong coach.
--
-- TEXT rather than JSONB deliberately: the value is only ever read whole and
-- parsed in Rust, so JSONB's indexing and operators would be unused ceremony.

ALTER TABLE messaging_channel_links ADD COLUMN IF NOT EXISTS proposed_coach_ids TEXT;
