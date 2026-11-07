-- ABOUTME: Indexes leading with user_id on llm_usage, user_facts and usage_counters, for the athlete's data export (PostgreSQL)
-- ABOUTME: The export filters on the athlete alone, so an index that leads with tenant_id costs a walk over every athlete's entries

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The export's tenant test is `tenant_id IS NULL OR CAST(tenant_id AS TEXT) IN
-- (...)` (repositories/user_export.rs), which no index on tenant_id can serve,
-- so the existing (tenant_id, user_id, ...) indexes are read end to end on every
-- page. Measured on PostgreSQL 15 with 5,000 athletes and 1,003,000 llm_usage,
-- 100,000 user_facts and 500,000 usage_counters rows: one page of one athlete
-- read 1,398, 222 and 711 buffers before these indexes and 316, 29 and 109 after.

CREATE INDEX IF NOT EXISTS idx_llm_usage_user ON llm_usage(user_id);
CREATE INDEX IF NOT EXISTS idx_user_facts_user ON user_facts(user_id);
CREATE INDEX IF NOT EXISTS idx_usage_counters_user ON usage_counters(user_id);
