-- ABOUTME: Adds content_blocks to chat_messages for inline visual blocks lifted from prose.
-- ABOUTME: Ordered JSON array; distinct from structured_content, which replaces a whole reply.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- Add content_blocks for inline visuals
-- ============================================================================

-- Ordered JSON array of schema-validated visual blocks (chart/table) lifted out
-- of an assistant reply's prose. The reply text keeps a positional marker where
-- each block sat, so clients interleave the two.
--
-- Deliberately separate from structured_content rather than reusing it: that
-- column holds ONE payload that replaces the entire reply (a workout plan),
-- while this holds N payloads embedded in prose. Same column, two content
-- models, would make every reader guess which it was looking at.
--
-- NULL for ordinary replies, which is every reply until a coach is granted
-- `visuals:` in contremaitre.
ALTER TABLE chat_messages ADD COLUMN content_blocks TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
