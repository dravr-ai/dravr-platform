-- ABOUTME: coach_access_requests — a coach's one-tap request for coach access, decided by a super-admin
-- ABOUTME: Written by POST /api/me/coach-access-request; granted or declined from the admin console queue

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- carnet#738. A request grants nothing by itself (ADR-018): a super-admin
-- decides. On a grant the requester gets manages_roster and, when the request
-- names the group they made during onboarding and it still has no coach, is
-- attached as that group's coach.
--
-- Scoped by user_id: coach access (manages_roster) is a property of the
-- account, not of a tenant. group_tenant_id only locates the named group for
-- the grant; it scopes nothing.
CREATE TABLE IF NOT EXISTS coach_access_requests (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    group_id TEXT,
    group_tenant_id TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'granted', 'declined')),
    created_at TIMESTAMPTZ NOT NULL,
    decided_at TIMESTAMPTZ,
    decided_by UUID REFERENCES users(id) ON DELETE SET NULL
);

-- One open request per user: a second tap returns the one already waiting.
CREATE UNIQUE INDEX IF NOT EXISTS idx_coach_access_requests_one_pending
    ON coach_access_requests (user_id) WHERE status = 'pending';

-- The admin queue reads by status, oldest first.
CREATE INDEX IF NOT EXISTS idx_coach_access_requests_status
    ON coach_access_requests (status, created_at);
