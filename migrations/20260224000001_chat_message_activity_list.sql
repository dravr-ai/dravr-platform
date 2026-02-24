-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
-- ABOUTME: Add activity_list column to chat_messages for persisting tool-fetched activity data
-- ABOUTME: Ensures activity lists survive message reloads from history

ALTER TABLE chat_messages ADD COLUMN activity_list TEXT;
