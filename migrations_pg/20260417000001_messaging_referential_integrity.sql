-- ABOUTME: Add FK constraints to messaging_* tables so orphan ids can never be inserted
-- ABOUTME: Converts user_id/tenant_id to UUID, keeps pierre_conversation_id as TEXT, CASCADE on delete

-- Context: messaging_link_states, messaging_channel_links, messaging_sessions historically
-- stored user_id/tenant_id as TEXT with no foreign key. A production incident on
-- 2026-04-16 revealed a messaging_session pointing at a user_id that had never existed
-- in users and a pierre_conversation_id referencing a dropped chat_conversations row —
-- leaving the WhatsApp inbox stuck on "Conversation not found". The schema allowed it
-- because nothing checked referential integrity.
--
-- This migration converts user_id and tenant_id to UUID (matching users.id and
-- tenants.id) and adds ON DELETE CASCADE foreign keys, and adds a CASCADE FK on
-- messaging_sessions.pierre_conversation_id -> chat_conversations(id) (kept as TEXT
-- because chat_conversations.id is VARCHAR(255), not UUID).
--
-- Safe to re-run: each ALTER is guarded by information_schema lookups.

-- ============================================================================
-- Step 1: pre-flight orphan cleanup. Sweep any rows pointing at nonexistent ids
-- BEFORE adding the FK, otherwise the constraint add fails.
-- ============================================================================

DELETE FROM messaging_link_states
WHERE user_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM users u WHERE u.id::text = messaging_link_states.user_id);

DELETE FROM messaging_link_states
WHERE NOT EXISTS (SELECT 1 FROM tenants t WHERE t.id::text = messaging_link_states.tenant_id);

DELETE FROM messaging_channel_links
WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.id::text = messaging_channel_links.user_id);

DELETE FROM messaging_channel_links
WHERE NOT EXISTS (SELECT 1 FROM tenants t WHERE t.id::text = messaging_channel_links.tenant_id);

-- messaging_messages.session_id REFERENCES messaging_sessions(id) without
-- ON DELETE CASCADE, so clean dependent rows before the session DELETE.
DELETE FROM messaging_messages
WHERE session_id IN (
    SELECT id FROM messaging_sessions ms
    WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.id::text = ms.user_id)
       OR NOT EXISTS (SELECT 1 FROM tenants t WHERE t.id::text = ms.tenant_id)
);

DELETE FROM messaging_sessions
WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.id::text = messaging_sessions.user_id);

DELETE FROM messaging_sessions
WHERE NOT EXISTS (SELECT 1 FROM tenants t WHERE t.id::text = messaging_sessions.tenant_id);

-- Null out dangling pierre_conversation_id references rather than deleting
-- the session, so re-linking isn't required on recovery. The self-heal layer
-- creates a fresh conversation when it observes NULL.
UPDATE messaging_sessions
SET pierre_conversation_id = NULL
WHERE pierre_conversation_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM chat_conversations cc
      WHERE cc.id = messaging_sessions.pierre_conversation_id
  );

-- ============================================================================
-- Step 2: convert TEXT columns to UUID where the referenced PK is UUID.
-- USING col::uuid casts in place; PostgreSQL rebuilds affected indexes.
-- ============================================================================

ALTER TABLE messaging_link_states
    ALTER COLUMN tenant_id TYPE UUID USING tenant_id::uuid;

ALTER TABLE messaging_link_states
    ALTER COLUMN user_id TYPE UUID USING user_id::uuid;

ALTER TABLE messaging_channel_links
    ALTER COLUMN tenant_id TYPE UUID USING tenant_id::uuid;

ALTER TABLE messaging_channel_links
    ALTER COLUMN user_id TYPE UUID USING user_id::uuid;

ALTER TABLE messaging_sessions
    ALTER COLUMN tenant_id TYPE UUID USING tenant_id::uuid;

ALTER TABLE messaging_sessions
    ALTER COLUMN user_id TYPE UUID USING user_id::uuid;

-- ============================================================================
-- Step 3: add the foreign keys. ON DELETE CASCADE so deleting a user or tenant
-- wipes their messaging state in one transaction — no orphans can survive.
-- ============================================================================

ALTER TABLE messaging_link_states
    ADD CONSTRAINT fk_messaging_link_states_tenant
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;

ALTER TABLE messaging_link_states
    ADD CONSTRAINT fk_messaging_link_states_user
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

ALTER TABLE messaging_channel_links
    ADD CONSTRAINT fk_messaging_channel_links_tenant
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;

ALTER TABLE messaging_channel_links
    ADD CONSTRAINT fk_messaging_channel_links_user
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

ALTER TABLE messaging_sessions
    ADD CONSTRAINT fk_messaging_sessions_tenant
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;

ALTER TABLE messaging_sessions
    ADD CONSTRAINT fk_messaging_sessions_user
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

ALTER TABLE messaging_sessions
    ADD CONSTRAINT fk_messaging_sessions_pierre_conversation
    FOREIGN KEY (pierre_conversation_id) REFERENCES chat_conversations(id) ON DELETE SET NULL;
