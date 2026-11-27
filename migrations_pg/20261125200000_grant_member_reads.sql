-- ABOUTME: When an outside client's grant last read another member, one row per grant (PostgreSQL)
-- ABOUTME: A free-text write over /mcp or A2A by that grant is refused for a while after such a read

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- carnet#1511 call 30: an /mcp or A2A tool call carries no topic, so what
-- another member's data may reach through a free-text write is decided per
-- grant: the OAuth grant, the API key, or the caller's credential of one
-- kind and client. Each row is the newest read of another member that grant
-- made; a free-text write by it waits until the window after that read ends.
--
-- grant_key   the grant, as the executor names it (never a token or a secret)
-- user_id     the account the grant acts for; every statement filters on it
-- read_at_ms  epoch milliseconds of the newest read, identical on both engines
CREATE TABLE IF NOT EXISTS grant_member_reads (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    grant_key TEXT NOT NULL,
    read_at_ms BIGINT NOT NULL,
    PRIMARY KEY (user_id, grant_key)
);
