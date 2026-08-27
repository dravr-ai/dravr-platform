-- ABOUTME: Turns prescribed_workouts into the ledger of every calendar event Dravr owns on a provider
-- ABOUTME: Adds external_id / source / plan_week_id / replaces_id / payload_hash / updated_at; template_slug becomes nullable
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A plan day has no template behind it, so template_slug admits NULL. Existing
-- rows are single prescriptions (source 'prescription') and keep their
-- created_at as updated_at — nothing has touched them since they were written.
ALTER TABLE prescribed_workouts
    ALTER COLUMN template_slug DROP NOT NULL,
    ADD COLUMN IF NOT EXISTS external_id TEXT,
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'prescription',
    ADD COLUMN IF NOT EXISTS plan_week_id TEXT,
    ADD COLUMN IF NOT EXISTS replaces_id UUID,
    ADD COLUMN IF NOT EXISTS payload_hash TEXT,
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
UPDATE prescribed_workouts SET updated_at = created_at;

-- One live calendar entry per Dravr key: a re-push supersedes the previous row
-- (status 'replaced') before the new one lands, so this can never be violated
-- by a correct write and always is by a duplicate one.
CREATE UNIQUE INDEX IF NOT EXISTS idx_prescribed_workouts_one_pushed_per_key
    ON prescribed_workouts(tenant_id, user_id, provider, external_id)
    WHERE status = 'pushed' AND external_id IS NOT NULL;
