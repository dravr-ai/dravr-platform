-- ABOUTME: Records whose data an outside client's tool call read, and through which client (SQLite)
-- ABOUTME: Adds nullable llm_usage subject_user_id, client_id, credential_kind, transport, impersonator_user_id: the owner feed reads them (carnet#1543)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A direct tool call (call_type 'mcp_tool') made by a client outside
-- Dravr's own surfaces, over MCP or A2A, writes one llm_usage row. These
-- columns say what the athlete's "who read my data" feed shows of it:
--   subject_user_id  whose data the call read: the coached athlete for a
--                    coach's read (the athlete's own record of it is
--                    coach_data_reads), the caller otherwise; NULL when
--                    the call read another member it resolved no subject
--                    for (a peer's read under peer consent): not guessed
--   client_id        the client the caller's credential names: the OAuth
--                    client a token was issued to, or the A2A client of a
--                    client-credentials token; NULL for an API key, a
--                    personal MCP token or a first-party session
--   credential_kind  the kind of credential the caller presented, by
--                    CredentialKind::as_str ('oauth_grant', 'api_key',
--                    'mcp_token', 'first_party_session',
--                    'a2a_client_credentials')
--   transport        pierre_core::transport::Transport::as_str of the
--                    call ('mcp_http', 'a2a')
--   impersonator_user_id  the admin whose support session made the call
--                    while acting as the user, as coach_data_reads
--                    records it; NULL for the user's own call. No foreign
--                    key, as there: the record keeps saying "support"
--                    after that admin's account is gone
-- Additive and nullable; no backfill: rows written before these columns,
-- and every row that is not a direct tool call, keep NULL (not guessed).
ALTER TABLE llm_usage ADD COLUMN subject_user_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE llm_usage ADD COLUMN client_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE llm_usage ADD COLUMN credential_kind TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE llm_usage ADD COLUMN transport TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE llm_usage ADD COLUMN impersonator_user_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- The athlete's feed: their own reads by outside clients, newest first.
CREATE INDEX IF NOT EXISTS idx_llm_usage_feed
    ON llm_usage (tenant_id, subject_user_id, created_at);
