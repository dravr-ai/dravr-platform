-- ABOUTME: Add prompt_tokens and model columns to chat_messages for full token usage tracking
-- ABOUTME: Enables per-message LLM cost analysis by recording both prompt and completion tokens
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Add prompt_tokens column to track input token cost per message
ALTER TABLE chat_messages ADD COLUMN prompt_tokens INTEGER;

-- Add model column to track which LLM model generated each message
ALTER TABLE chat_messages ADD COLUMN model TEXT;
