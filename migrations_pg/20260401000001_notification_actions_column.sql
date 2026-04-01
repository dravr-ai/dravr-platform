-- ABOUTME: Adds missing actions column to notifications table for rich action buttons
-- ABOUTME: Was present in SQLite migration but omitted from PG schema creation
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Action buttons for rich notifications (Accept/Decline, Reply, Open Screen)
-- Stored as JSON text: [{"id": "accept", "title": "Accept", "action_type": "accept_decline"}, ...]
ALTER TABLE notifications ADD COLUMN IF NOT EXISTS actions TEXT;
