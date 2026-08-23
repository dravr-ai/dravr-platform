-- ABOUTME: Standing per-email pre-approval allow-list — an operator "allow" recorded before the person registers
-- ABOUTME: Registration consults it so an allowed address lands Active instead of the pending queue
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per allowed address, keyed lowercase. Registration gating is global
-- (pre-registration there is no tenant yet — the personal tenant is created at
-- signup), matching the scope of AUTO_APPROVE_DOMAINS. Rows are standing, not
-- single-use: users.email uniqueness already caps one account per address, and
-- `pierre-cli user list-allowed` reports which entries have registered.
-- No FK on allowed_by: an allow must survive the approving operator's own
-- account deletion.
CREATE TABLE IF NOT EXISTS pre_approved_emails (
    email       TEXT PRIMARY KEY,  -- stored lowercase
    allowed_by  TEXT,              -- users.id of the approving operator; NULL pre-bootstrap
    note        TEXT,              -- operator note (cohort, reason)
    created_at  TEXT NOT NULL      -- RFC3339 UTC
);
