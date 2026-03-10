-- ABOUTME: Adds actions column to notifications table for rich notification action buttons
-- ABOUTME: Stores action buttons as JSON (e.g., Accept/Decline, Reply, Open Screen)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Action buttons for rich notifications (Phase 5)
-- Stored as JSON array: [{"id": "accept", "title": "Accept", "action_type": "accept_decline"}, ...]
ALTER TABLE notifications ADD COLUMN actions TEXT;
