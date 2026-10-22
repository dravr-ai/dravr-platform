-- ABOUTME: signup_codes — shareable codes an admin mints so whoever enters one is approved; plus one redemption per account (PostgreSQL)
-- ABOUTME: A coach's code is a group_invites row with approves_signup set, so it also joins the coach's group (carnet#1128)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An admin's signup code is a bearer secret handed out by any channel (a
-- talk, Slack, a landing page). Only its SHA-256 is stored, with the last
-- four characters as a hint so the admin can tell codes apart; the code
-- itself is shown once, when minted. Like user_invitations there is no
-- tenant: a sign-up precedes any tenant. No FK on created_by, so a code
-- outlives its admin's account, as an allow does.
CREATE TABLE IF NOT EXISTS signup_codes (
    id          TEXT PRIMARY KEY,
    code_hash   TEXT NOT NULL,     -- SHA-256 hex of the normalized code
    code_hint   TEXT NOT NULL,     -- last four characters, for display
    note        TEXT,              -- the cohort or reason the admin recorded
    created_by  UUID,              -- users.id of the minting admin; NULL for a service token
    expires_at  TIMESTAMPTZ,       -- NULL never expires
    max_uses    INTEGER,           -- NULL is unlimited
    use_count   INTEGER NOT NULL DEFAULT 0,
    revoked_at  TIMESTAMPTZ,       -- set when the admin withdraws it
    created_at  TIMESTAMPTZ NOT NULL
);

-- Redemption is a hash lookup.
CREATE UNIQUE INDEX IF NOT EXISTS idx_signup_codes_code_hash
    ON signup_codes (code_hash);

-- A coach's code: a member invite of the coach's group that also approves
-- the account that redeems it. Plain invites keep needing an approved account.
ALTER TABLE group_invites ADD COLUMN IF NOT EXISTS approves_signup BOOLEAN NOT NULL DEFAULT FALSE;

-- One redemption per account: the code's use is claimed when it is entered,
-- and the account is activated once its address is proven (activated_at).
-- Exactly one of signup_code_id / group_invite_id names the source.
CREATE TABLE IF NOT EXISTS signup_code_redemptions (
    user_id          UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    signup_code_id   TEXT REFERENCES signup_codes(id) ON DELETE SET NULL,
    group_invite_id  UUID REFERENCES group_invites(id) ON DELETE SET NULL,
    granted_by       UUID,         -- the admin or coach who minted the code
    redeemed_at      TIMESTAMPTZ NOT NULL,
    activated_at     TIMESTAMPTZ
);

-- The admin listing reads who came in on which code.
CREATE INDEX IF NOT EXISTS idx_signup_code_redemptions_code
    ON signup_code_redemptions (signup_code_id);
CREATE INDEX IF NOT EXISTS idx_signup_code_redemptions_invite
    ON signup_code_redemptions (group_invite_id);
