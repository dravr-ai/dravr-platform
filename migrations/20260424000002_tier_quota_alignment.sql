-- ABOUTME: Phase 3 — users.billing_test flag + usage_counters dimension columns for per-conv/coach caps
-- ABOUTME: Makes per-coach, per-conversation, per-channel, and per-tool quotas enforceable
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- billing_test users bypass quotas (QUOTA_BYPASS_USER_IDS env + this flag);
-- replaces the unconditional admin bypass removed from routes/chat/quotas.rs.
ALTER TABLE users ADD COLUMN billing_test INTEGER NOT NULL DEFAULT 0;

-- Nullable dimensions on usage_counters so the same counter_key can be
-- sharded by coach_id / group_id / conversation_id / channel_type / tool_name.
-- Existing rows retain NULL, which matches the "tenant + user" keying that
-- predates this migration.
ALTER TABLE usage_counters ADD COLUMN coach_id TEXT;
ALTER TABLE usage_counters ADD COLUMN group_id TEXT;
ALTER TABLE usage_counters ADD COLUMN conversation_id TEXT;
ALTER TABLE usage_counters ADD COLUMN channel_type TEXT;
ALTER TABLE usage_counters ADD COLUMN tool_name TEXT;

CREATE INDEX IF NOT EXISTS idx_usage_counters_conversation
    ON usage_counters(tenant_id, user_id, conversation_id, counter_key, period);
CREATE INDEX IF NOT EXISTS idx_usage_counters_coach
    ON usage_counters(tenant_id, user_id, coach_id, counter_key, period);
CREATE INDEX IF NOT EXISTS idx_usage_counters_tool
    ON usage_counters(tenant_id, user_id, tool_name, counter_key, period);
