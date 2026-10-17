-- ABOUTME: A human coach's training-plan draft lives beside the athlete's live season, unseen until published (carnet#1069)
-- ABOUTME: Widens the status CHECK by three statuses, adds three draft columns and one open draft per coach and athlete
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale. A group's human
-- coach writes a coached athlete's plan into a draft row in status 'draft',
-- under the athlete's tenant and user, which no athlete read selects; publish
-- replays it through the live save path and marks it 'published', discard
-- marks it 'discarded'.
--
--   drafted_by_user_id  the coach who owns the draft: a users(id) as text,
--                       like user_id, and required on a draft row.
--   draft_base_plan_id  the live season the draft began from, NULL when the
--                       athlete had none.
--   outline_drafted     whether publishing re-lays the season, or only puts
--                       the drafted weeks on the live one.
--
-- The status CHECK was declared inline in 20260712000001, which names it
-- training_plans_status_check; it is dropped and re-added with the three new
-- statuses. Every statement is guarded, so a second pass is a no-op.
ALTER TABLE training_plans DROP CONSTRAINT IF EXISTS training_plans_status_check;
ALTER TABLE training_plans
    ADD CONSTRAINT training_plans_status_check
    CHECK (status IN ('active','superseded','completed','abandoned',
                      'draft','published','discarded'));

ALTER TABLE training_plans ADD COLUMN IF NOT EXISTS drafted_by_user_id TEXT;
ALTER TABLE training_plans ADD COLUMN IF NOT EXISTS draft_base_plan_id TEXT;
ALTER TABLE training_plans ADD COLUMN IF NOT EXISTS outline_drafted BOOLEAN NOT NULL DEFAULT FALSE;

-- A draft always names the coach who owns it.
ALTER TABLE training_plans DROP CONSTRAINT IF EXISTS training_plans_draft_owner_check;
ALTER TABLE training_plans
    ADD CONSTRAINT training_plans_draft_owner_check
    CHECK (status <> 'draft' OR drafted_by_user_id IS NOT NULL);

-- One OPEN draft per coach per athlete: every save the coach makes lands on it.
CREATE UNIQUE INDEX IF NOT EXISTS idx_training_plans_one_draft
    ON training_plans(tenant_id, user_id, drafted_by_user_id) WHERE status = 'draft';
