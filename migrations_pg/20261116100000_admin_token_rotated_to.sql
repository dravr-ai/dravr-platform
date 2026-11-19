-- ABOUTME: rotated_to_token_id on admin_tokens: the replacement a rotation minted, written by the rotation's claim (PostgreSQL)
-- ABOUTME: Tells a rotated token from a revoked one, so revoking a rotated token can name the credential that is still live
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- See the matching SQLite migration for the full rationale (carnet#1589).
-- NULL for a live token, for a revoked one, and for every token rotated
-- before this column. TEXT with no foreign key, like admin_tokens.tenant_id
-- and admin_tokens.operator_user_id, so one statement serves both backends.

ALTER TABLE admin_tokens ADD COLUMN IF NOT EXISTS rotated_to_token_id TEXT;
