-- ABOUTME: user_invitations — single-use invitation links an operator mails to an address with no account (SQLite)
-- ABOUTME: Redeeming one creates the account active, with the invitee's own password and approved_by the inviter

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An invitation is the single-use companion of a standing pre-approval
-- (pre_approved_emails): the operator allows an address and mails it a link,
-- and the link is where the invitee chooses their password. The token is the
-- `<selector>.<verifier>` mechanism every mailed link uses: only the selector
-- and the SHA-256 of the verifier are stored, and a wrong verifier costs one
-- attempt until the cap revokes the row.
--
-- There is no account yet, so there is no user_id or tenant to scope a row
-- by: the account and its personal tenant are created at redemption, as a
-- registration creates them. A row is reached either by the unguessable token
-- the invitee holds, or by the operator surface that manages
-- pre_approved_emails, which is global for the same reason.
--
-- No FK on invited_by: an invitation must survive the inviting operator's own
-- account deletion, as an allow does.
CREATE TABLE IF NOT EXISTS user_invitations (
    id            TEXT PRIMARY KEY,
    email         TEXT NOT NULL,     -- stored lowercase
    selector      TEXT NOT NULL,
    token_hash    TEXT NOT NULL,     -- SHA-256 hex of the verifier half
    invited_by    TEXT,              -- users.id of the inviting operator; NULL for a service token
    expires_at    TEXT NOT NULL,     -- RFC3339 UTC
    redeemed_at   TEXT,              -- set once, when the account is created from it
    revoked_at    TEXT,              -- superseded, withdrawn by the operator, or locked out
    attempt_count INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL      -- RFC3339 UTC
);

-- Redemption is a selector lookup.
CREATE UNIQUE INDEX IF NOT EXISTS idx_user_invitations_selector
    ON user_invitations (selector);

-- At most one live invitation per address: issuing a new one revokes the
-- previous one first, so an older link stops working the moment a newer one
-- is sent.
CREATE UNIQUE INDEX IF NOT EXISTS idx_user_invitations_live_email
    ON user_invitations (email)
    WHERE redeemed_at IS NULL AND revoked_at IS NULL;

-- The send budget counts recent rows per address, live or not.
CREATE INDEX IF NOT EXISTS idx_user_invitations_email_created_at
    ON user_invitations (email, created_at);
