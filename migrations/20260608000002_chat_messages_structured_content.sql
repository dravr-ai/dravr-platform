-- ABOUTME: Adds structured_content column to chat_messages for validated plan payloads.
-- ABOUTME: Stores the schema-validated JSON extracted from a structured-output coach reply.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- Add structured_content column for rich message rendering
-- ============================================================================

-- JSON payload extracted and schema-validated from an assistant reply (e.g. a
-- structured-workout plan). When present, clients render it as a rich card
-- instead of the raw message text. NULL for ordinary free-text replies.
ALTER TABLE chat_messages ADD COLUMN structured_content TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
