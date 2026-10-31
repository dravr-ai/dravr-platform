-- ABOUTME: coach_attention_dismissals — a coach's "Traité" on one Coach Home attention reason, per athlete (PostgreSQL)
-- ABOUTME: Holds while the reason's signal fingerprint is unchanged; a changed signal brings the reason back (carnet#1252)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per (tenant, coach, athlete, reason): the reason id is stable for
-- the situation (`nothing_received`, `provider_reconnect:garmin`,
-- `unverified_figure:<reply>`), the fingerprint names the signal it was
-- dismissed on. tenant_id is the athlete's roster tenant (the group through
-- which they share), and every read and write filters on it with the coach.
-- The UNIQUE constraint's index leads with (tenant_id, coach_user_id), which
-- is what every read filters on, so the table needs no second index.
CREATE TABLE IF NOT EXISTS coach_attention_dismissals (
    id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    coach_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    athlete_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    dismissed_at TIMESTAMPTZ NOT NULL,
    UNIQUE (tenant_id, coach_user_id, athlete_user_id, reason_id)
);
