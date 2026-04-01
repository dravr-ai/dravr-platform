-- ABOUTME: Fix notifications.id column type from TEXT to UUID
-- ABOUTME: PG migration used SQLite-style TEXT but Rust model expects UUID
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The notifications table was created with TEXT id (SQLite pattern) but the
-- Rust Notification struct uses Uuid for id. The user_id and tenant_id columns
-- stay TEXT because dravr-commere binds them as strings in queries.
ALTER TABLE notifications ALTER COLUMN id TYPE UUID USING id::uuid;
