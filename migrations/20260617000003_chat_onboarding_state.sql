-- ABOUTME: Adds onboarding_state to chat_conversations for the guided multi-turn pillar onboarding
-- ABOUTME: Nullable JSON; when set, prompt assembly runs the conversation in onboarding mode

-- When non-null, the conversation is mid-onboarding: prompt assembly injects an
-- onboarding directive and the extraction worker stamps facts source=onboarding.
ALTER TABLE chat_conversations
    ADD COLUMN onboarding_state TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; one-time additive column
