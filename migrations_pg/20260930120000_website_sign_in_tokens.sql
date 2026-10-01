-- ABOUTME: website_sign_in_tokens — the magic links that open the members part of the dravr.ai docs (PostgreSQL)
-- ABOUTME: The website keeps no user store of its own; its Worker reaches these through /admin/website/*

-- The docs sign-in link. Same `<selector>.<verifier>` mechanism and lockout as
-- email_verification_tokens, in its own token space: a link that signs someone
-- in to the website docs must never verify an address, and one that verifies
-- an address must never open a docs session.
--
-- Rows are scoped by user_id, a narrower key than a tenant: a link belongs to
-- the one account it signs in.
CREATE TABLE IF NOT EXISTS website_sign_in_tokens (
    id            TEXT PRIMARY KEY,
    user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    selector      TEXT NOT NULL,
    token_hash    TEXT NOT NULL,
    expires_at    TIMESTAMPTZ NOT NULL,
    used_at       TIMESTAMPTZ,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Consumption is a selector lookup; the send budget counts recent rows per user.
CREATE UNIQUE INDEX IF NOT EXISTS idx_website_sign_in_tokens_selector
    ON website_sign_in_tokens (selector);
CREATE INDEX IF NOT EXISTS idx_website_sign_in_tokens_user_id
    ON website_sign_in_tokens (user_id);
CREATE INDEX IF NOT EXISTS idx_website_sign_in_tokens_expires_at
    ON website_sign_in_tokens (expires_at);
