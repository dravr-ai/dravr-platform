-- ABOUTME: Fix notifications UUID columns from TEXT to UUID type
-- ABOUTME: PG migration used SQLite-style TEXT but Rust model expects UUID
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The notifications table was created with TEXT columns (SQLite pattern) but the
-- Rust Notification struct uses Uuid for id, user_id, and tenant_id.
ALTER TABLE notifications
    ALTER COLUMN id TYPE UUID USING id::uuid,
    ALTER COLUMN user_id TYPE UUID USING user_id::uuid,
    ALTER COLUMN tenant_id TYPE UUID USING tenant_id::uuid;
