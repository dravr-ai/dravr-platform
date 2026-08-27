-- ABOUTME: Per-participant read marker on conversation_participants — the source of every unread count
-- ABOUTME: NULL means the participant has never opened the thread; every user/assistant row is unread
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- last_read_at is the created_at of the newest row the participant has seen,
-- advanced monotonically by the read routes and by the pipeline after it
-- persists the participant's own turn. It lives on the membership row rather
-- than the conversation because "unread" is a question about one participant:
-- two members of the same thread each keep their own marker. Cleared to NULL
-- by "mark unread".
ALTER TABLE conversation_participants ADD COLUMN last_read_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
