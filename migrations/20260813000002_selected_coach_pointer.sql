-- ABOUTME: tenant_users.selected_coach_id — the one pointer to a user's current coach (SQLite)
-- ABOUTME: Replaces users.default_coach_id and coach_assignments.is_active, which were two answers to one question

-- Three bindings answered "which coach is this user's?" and disagreed:
--
--   users.default_coach_id        1 writer  (/coach select in a DM), 3 readers,
--                                 and the only one that actually decided which
--                                 coach answered a message.
--   coach_assignments.is_active   6 writers (admin assign, bulk, the
--                                 activate/deactivate tools), 2 readers, and
--                                 nothing that selected the responding coach.
--   chat_conversations.coach_id   per-conversation, chosen in the web chat UI.
--
-- Consequence: a user who finished the whole web wizard still read as "no active
-- coach", so their first messaging turn re-ran the coach proposal. The same
-- person was onboarded twice.
--
-- The shape here is the standard one for "which of these is current": a foreign
-- key on the entity that owns the choice, not a boolean on each row of the
-- collection. `coach_assignments.is_active` needed two non-transactional
-- UPDATEs to maintain "at most one" (clear all, then set one), which can leave a
-- user with zero or two. A single column cannot.
--
-- It lives on tenant_users rather than users because coaches are tenant-scoped:
-- get_active_coach filters on c.tenant_id, while users.default_coach_id carried
-- no tenant at all, so a user in two tenants had one global coach that silently
-- failed to resolve in the other. UNIQUE(tenant_id, user_id) already exists here,
-- so "at most one selection per membership" comes from the schema for free.
--
-- coach_assignments survives as what it always was underneath: the roster —
-- who assigned it, favourites, use counts, last used.

ALTER TABLE tenant_users ADD COLUMN selected_coach_id TEXT REFERENCES coaches(id) ON DELETE SET NULL; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS

-- Carry existing selections over. users.default_coach_id first (it is the one
-- that actually drove replies), then any active assignment for users who never
-- ran /coach select — so nobody loses the coach they were already talking to.
UPDATE tenant_users
   SET selected_coach_id = (SELECT u.default_coach_id FROM users u WHERE u.id = tenant_users.user_id)
 WHERE selected_coach_id IS NULL
   AND (SELECT u.default_coach_id FROM users u WHERE u.id = tenant_users.user_id) IS NOT NULL;

UPDATE tenant_users
   SET selected_coach_id = (
         SELECT ca.coach_id FROM coach_assignments ca
          WHERE ca.user_id = tenant_users.user_id AND ca.is_active = 1
          LIMIT 1)
 WHERE selected_coach_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_tenant_users_selected_coach ON tenant_users(selected_coach_id);
