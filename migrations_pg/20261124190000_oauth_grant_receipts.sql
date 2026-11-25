-- ABOUTME: oauth_grant_events and oauth_standing_policies — a consent grant's signed receipts and the athlete's standing policies (PostgreSQL)
-- ABOUTME: Each grant write records its event in its own transaction; the record_log worker logs and signs it (carnet#1544)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Owl phase 5 slice 3b (carnet#1544, design rows 34 and 35, owl PDPP §6.4
-- and §8.1). Additive: two new tables and one nullable column. Grants
-- given before this migration get their receipt from the record_log
-- worker's baseline step: each live grant with no event is given one
-- `issue` receipt (approval.baseline true, at its granted_at).
--
-- oauth_grant_events: one row per event in a consent grant's life (an
-- oauth_client_grants row): issue, revoke, expire. Written by the
-- repository statement that starts or ends the grant, in its transaction.
-- The row is also the outbox: the hourly record_log worker appends the
-- receipt's SHA-256 to the athlete's barrage record_log (logged_at) and
-- signs the receipt as a compact JWS with the checkpoint key (jws).
--   id               the receipt's id (receipt_id in the payload)
--   tenant_id, user_id  the grant's; every athlete-facing read filters both
--   client_grant_id  oauth_client_grants.id; no foreign key: a grant row
--                    hard-deleted with its client keeps its receipts
--   event            issue | revoke | expire
--   actor            user (the athlete) | client (RFC 7009) | auto (the AS:
--                    a standing policy, a sweep, a deleted client)
--   at               when the event happened
--   payload          the receipt payload, fixed field order, as signed
--   payload_sha256   hex SHA-256 of payload: the record_log entry's content
--   jws              the compact JWS; NULL until a checkpoint signer signs it
--   logged_at        when its record_log entry was appended; NULL until then
--   attempts         how many times the outbox failed to log or sign it; a
--                    failing row sorts behind the ones not yet tried
--   poisoned_at      when it failed the outbox's last allowed attempt: the
--                    outbox skips it from then on (logged at error level)
-- One terminal event (revoke or expire) per grant: the partial unique
-- index below, inserted ON CONFLICT DO NOTHING, so a revoke of an expired
-- grant or two sweeps racing write one.
--
-- oauth_client_grants.standing_policy_id: the standing policy that renewed
-- the grant, NULL for one the athlete approved on the consent screen. A
-- column, not a member of the details document, so the review digest and a
-- rollback binary's reading of the details are untouched.
--
-- oauth_standing_policies: the athlete's "always allow this app this
-- preset". Read only where the consent screen would be skipped; a request
-- with the same terms from the same client is approved without the
-- screen, its grant citing the policy and its issue receipt actor auto.
--   scope            the OAuth scope approved with the box: a renewal asks
--                    for exactly this scope, never another
--   authorization_details  the resolved details approved with the box,
--                    without the frozen window: what a renewal must equal
--   consent_version, consent_locale  the review digest and language the
--                    box was ticked under; a changed consent text shows the
--                    screen again

CREATE TABLE IF NOT EXISTS oauth_grant_events (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    client_grant_id TEXT NOT NULL,
    event TEXT NOT NULL CHECK (event IN ('issue', 'revoke', 'expire')),
    actor TEXT NOT NULL CHECK (actor IN ('user', 'client', 'auto')),
    at TIMESTAMPTZ NOT NULL,
    payload TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL,
    jws TEXT,
    logged_at TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0,
    poisoned_at TIMESTAMPTZ
);

-- The athlete's receipts, newest last.
CREATE INDEX IF NOT EXISTS idx_oauth_grant_events_owner
    ON oauth_grant_events (tenant_id, user_id, at);

-- A grant's events: its latest issue, and whether it already ended.
CREATE INDEX IF NOT EXISTS idx_oauth_grant_events_grant
    ON oauth_grant_events (client_grant_id, event);

-- One terminal event per grant.
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_grant_events_terminal
    ON oauth_grant_events (client_grant_id)
    WHERE event IN ('revoke', 'expire');

-- The outbox: receipts not yet logged, not yet signed.
CREATE INDEX IF NOT EXISTS idx_oauth_grant_events_unlogged
    ON oauth_grant_events (attempts, at) WHERE logged_at IS NULL AND poisoned_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_oauth_grant_events_unsigned
    ON oauth_grant_events (attempts, at) WHERE jws IS NULL AND poisoned_at IS NULL;

CREATE TABLE IF NOT EXISTS oauth_standing_policies (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    preset TEXT NOT NULL,
    scope TEXT NOT NULL,
    authorization_details TEXT NOT NULL,
    consent_version TEXT NOT NULL,
    consent_locale TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);

-- One live policy per athlete, client and preset.
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_standing_policies_active
    ON oauth_standing_policies (tenant_id, user_id, client_id, preset)
    WHERE revoked_at IS NULL;

-- The standing policy that renewed a grant; NULL when the athlete approved it.
ALTER TABLE oauth_client_grants ADD COLUMN IF NOT EXISTS standing_policy_id TEXT;
