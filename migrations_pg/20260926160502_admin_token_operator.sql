-- ABOUTME: operator_user_id on admin_tokens — the super-admin a device-login token acts as, stored at the grant (PostgreSQL)
-- ABOUTME: Replaces reading the operator out of the free-text service_name, and takes operator emails out of token names

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale (carnet#561).
-- TEXT like admin_tokens.tenant_id, so one statement serves both backends.
ALTER TABLE admin_tokens ADD COLUMN IF NOT EXISTS operator_user_id TEXT;

UPDATE admin_tokens
SET operator_user_id = (
    SELECT u.id::text FROM users u WHERE u.email = substring(admin_tokens.service_name FROM 12)
)
WHERE service_name LIKE 'device-cli:%' AND operator_user_id IS NULL;

UPDATE admin_tokens
SET service_name = 'device-cli:' || COALESCE(operator_user_id, 'unresolved')
WHERE service_name LIKE 'device-cli:%';
