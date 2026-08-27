-- ABOUTME: Turns prescribed_workouts into the ledger of every calendar event Dravr owns on a provider
-- ABOUTME: Adds external_id / source / plan_week_id / replaces_id / payload_hash / updated_at; template_slug becomes nullable
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A plan day has no template behind it, so template_slug must admit NULL; SQLite
-- cannot drop a NOT NULL constraint in place, hence the rebuild. Existing rows
-- are single prescriptions (source 'prescription') and keep their created_at as
-- updated_at — nothing has touched them since they were written.
CREATE TABLE prescribed_workouts_new ( -- idempotency-ok: table rebuild — SQLite cannot drop NOT NULL in place; sqlx applies each migration exactly once
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    coach_id TEXT,
    template_slug TEXT,
    sport TEXT NOT NULL,
    prescribed_for_date TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_event_id TEXT,
    external_id TEXT,
    source TEXT NOT NULL DEFAULT 'prescription',
    plan_week_id TEXT,
    replaces_id TEXT,
    payload_json TEXT NOT NULL,
    payload_hash TEXT,
    status TEXT NOT NULL DEFAULT 'pushed',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT INTO prescribed_workouts_new (
    id, tenant_id, user_id, coach_id, template_slug, sport, prescribed_for_date,
    provider, provider_event_id, payload_json, status, created_at, updated_at
)
  SELECT id, tenant_id, user_id, coach_id, template_slug, sport, prescribed_for_date,
         provider, provider_event_id, payload_json, status, created_at, created_at
  FROM prescribed_workouts;
DROP TABLE prescribed_workouts; -- idempotency-ok: table rebuild — the rebuilt table is renamed over it on the next line
ALTER TABLE prescribed_workouts_new RENAME TO prescribed_workouts;

CREATE INDEX IF NOT EXISTS idx_prescribed_workouts_tenant_user
    ON prescribed_workouts(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_prescribed_workouts_date
    ON prescribed_workouts(tenant_id, user_id, prescribed_for_date DESC);
-- One live calendar entry per Dravr key: a re-push supersedes the previous row
-- (status 'replaced') before the new one lands, so this can never be violated
-- by a correct write and always is by a duplicate one.
CREATE UNIQUE INDEX IF NOT EXISTS idx_prescribed_workouts_one_pushed_per_key
    ON prescribed_workouts(tenant_id, user_id, provider, external_id)
    WHERE status = 'pushed' AND external_id IS NOT NULL;
