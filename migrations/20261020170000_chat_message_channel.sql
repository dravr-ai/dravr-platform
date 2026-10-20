-- ABOUTME: Names the channel each chat turn was written from, so a thread two channels share can tag a Telegram turn (carnet#840)
-- ABOUTME: Additive only: one nullable column; rows written before it, and rows no turn wrote, stay NULL

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The athlete's messaging DM now writes into their personal thread, the one
-- Home shows. The conversation's own channel_type names where the thread was
-- opened, so it cannot say which turns came over Telegram. Each turn row now
-- carries the wire word of its channel: web, mobile, telegram, whatsapp,
-- discord, slack or messenger.
ALTER TABLE chat_messages ADD COLUMN channel_type TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
