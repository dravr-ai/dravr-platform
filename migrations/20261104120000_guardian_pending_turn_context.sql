-- ABOUTME: guardian_pending_actions learns who a parked call acted as and what its turn served: agent, seat, provenance (SQLite)
-- ABOUTME: All additive and nullable, so a binary that predates them keeps reading and writing the table

-- A parked call re-dispatches from `/confirm`, in a later request with no turn
-- around it. Without these the re-dispatch ran as the athlete's own seat under
-- the conversation's agent, whatever turn had parked it: a coach's call to a
-- tool their seat withholds would run once confirmed, and a `@handle` turn's
-- write would be authored by the wrong agent (carnet#1398). The row names both
-- so the re-dispatch answers the same questions the turn did.
--
-- `seat` is NULL only on a row parked before this migration or on a parked agent
-- draft, which never re-dispatches. `/confirm` refuses a tool call whose seat it
-- cannot read rather than assume the athlete's own. Rows live five minutes.

-- The third column answers for what the parked turn had already served: a
-- first-party-only flag, attributions, grounding and group members, as JSON. Without
-- it a confirmed write derived from first-party-only data was stamped AnyTransport.
-- NULL is read as first-party-only served, never as clear.

ALTER TABLE guardian_pending_actions ADD COLUMN turn_agent_id TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS
ALTER TABLE guardian_pending_actions ADD COLUMN seat TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS
ALTER TABLE guardian_pending_actions ADD COLUMN turn_provenance TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS
