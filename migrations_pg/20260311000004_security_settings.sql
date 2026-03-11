-- ABOUTME: Consolidated PostgreSQL migration for security and system settings tables.
-- ABOUTME: Creates system_secrets, rsa_keypairs, and system_settings with default seed data.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- system_secrets
-- ============================================================================
CREATE TABLE IF NOT EXISTS system_secrets (
    secret_type TEXT PRIMARY KEY,
    secret_value TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_system_secrets_type ON system_secrets(secret_type);

-- ============================================================================
-- rsa_keypairs
-- ============================================================================
CREATE TABLE IF NOT EXISTS rsa_keypairs (
    kid TEXT PRIMARY KEY,
    private_key_pem TEXT NOT NULL,
    public_key_pem TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT FALSE,
    key_size_bits INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rsa_keypairs_active ON rsa_keypairs(is_active) WHERE is_active = TRUE;

-- ============================================================================
-- system_settings
-- ============================================================================
CREATE TABLE IF NOT EXISTS system_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Seed default settings
INSERT INTO system_settings (key, value, description, created_at, updated_at)
VALUES ('auto_approval_enabled', 'false', 'When enabled, new user registrations are automatically approved without admin intervention', NOW(), NOW())
ON CONFLICT (key) DO NOTHING;
