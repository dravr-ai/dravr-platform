-- ABOUTME: messaging_channel_links.proposed_coach_ids — what the coach proposal actually offered (SQLite)
-- ABOUTME: Lets a bare numeric reply resolve to a coach, which is what the proposal has always told users to send

-- The coach proposal ends with "Reply with a number to start", and nothing
-- parsed that number. The instruction was unimplemented: typing "1" went to the
-- model as ordinary conversation and bound nothing, teaching the user on their
-- first proactive message that the bot does not listen.
--
-- Resolving the number needs the ordered list the user is looking at. It cannot
-- be re-derived: the proposal is LLM-re-ranked, so rebuilding it later can return
-- a different order and bind the wrong coach — worse than binding none.
--
-- Stored on the same row that already stamps `coach_proposal_sent_at`, so what
-- was offered and the fact it was offered cannot drift apart. JSON array of coach
-- ids in display order; NULL for links that predate a proposal.

ALTER TABLE messaging_channel_links ADD COLUMN proposed_coach_ids TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS
