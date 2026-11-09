-- ABOUTME: Records on an assistant row whether its turn tried to change anything, so a client can tell if re-sending it is safe
-- ABOUTME: Additive and nullable: a row from before this column, or one no model turn wrote, reads as unknown and offers no regenerate

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Regenerate re-sends the question as a new turn on a fresh Guardian budget, so
-- in production it completed a delete the Guardian had just blocked. TRUE: the
-- turn was blocked, or called a tool that writes or is irreversible, or one the
-- registry cannot classify, whether or not the call succeeded. FALSE: it did
-- none of that. NULL: nobody knows, which clients treat as TRUE.
ALTER TABLE chat_messages ADD COLUMN turn_attempted_write BOOLEAN; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
