-- ABOUTME: Indexes on coach_reply_reviews by coach and by correction entry (SQLite)
-- ABOUTME: The coach's review reads no longer lead with tenant_id (carnet#1210); the room joins on the correction (carnet#1211)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A coach reviews a group in whichever tenant owns it, so the weekly count and
-- the review window filter on coach_user_id and created_at alone.
-- idx_coach_reply_reviews_coach_created leads with tenant_id, which those
-- reads do not have, so each one scanned the table.
CREATE INDEX IF NOT EXISTS idx_coach_reply_reviews_coach_user_created
    ON coach_reply_reviews(coach_user_id, created_at);

-- The room transcript labels each correction with the coach who wrote it by
-- joining its review on (tenant_id, correction_entry_id).
CREATE INDEX IF NOT EXISTS idx_coach_reply_reviews_tenant_correction
    ON coach_reply_reviews(tenant_id, correction_entry_id);
