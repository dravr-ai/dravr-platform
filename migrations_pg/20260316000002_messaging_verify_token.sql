-- ABOUTME: Add verify_token column to messaging_channel_configs for Meta webhook verification
-- ABOUTME: Separates the Meta verify token from the HMAC webhook_secret to avoid leaking signing keys
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

ALTER TABLE messaging_channel_configs ADD COLUMN IF NOT EXISTS verify_token TEXT;
