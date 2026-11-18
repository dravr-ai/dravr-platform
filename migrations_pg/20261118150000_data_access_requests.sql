-- ABOUTME: data_access_requests — the athlete's statutory access requests to a company, and the deadline each one runs to (PostgreSQL)
-- ABOUTME: Written by /api/user/data-access-requests; the daily sweep stamps each reminder it claims (carnet#1646)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Owl row 60. Dravr writes the letter; the athlete sends it, to an address
-- they entered, and Dravr tracks the legal deadline (Quebec P-39.1 art. 32:
-- 30 days; GDPR Art. 12(3): one month, extensible by two).
--   id                  the request's id
--   tenant_id           the session's tenant; every read filters on it
--   user_id             the athlete, the row's owner
--   reference           DRV-<year>-<6 base32>, quoted in the letter
--   controller          the company the request is addressed to
--   controller_contact  its privacy address, as the athlete entered it
--   athlete_identifier  the account or email the athlete uses there
--   law                 quebec_p39_1 or gdpr
--   scope               access, or access_portability
--   letter_locale       the language the letter is written in
--   status              drafted, sent, answered or closed; "due soon" and
--                       "overdue" are derived at read time, never stored
--   sent_on             the day the athlete sent it, which the count runs from
--   due_on              recomputed from sent_on, law and extended_until on
--                       every write, so the sweep has a column to index
--   extended_until      the later date a GDPR controller notified
--   answered_on         the day the company answered
--   due_soon_notified_at, overdue_notified_at
--                       the reminder each stage sent, claimed by a conditional
--                       update so no two instances send one twice
-- Additive; no backfill.
CREATE TABLE IF NOT EXISTS data_access_requests (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reference TEXT NOT NULL UNIQUE,
    controller TEXT NOT NULL,
    controller_contact TEXT NOT NULL,
    athlete_identifier TEXT NOT NULL,
    law TEXT NOT NULL CHECK (law IN ('quebec_p39_1', 'gdpr')),
    scope TEXT NOT NULL CHECK (scope IN ('access', 'access_portability')),
    letter_locale TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('drafted', 'sent', 'answered', 'closed')),
    sent_on DATE,
    due_on DATE,
    extended_until DATE,
    answered_on DATE,
    due_soon_notified_at TIMESTAMPTZ,
    overdue_notified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

-- The sweep: sent requests by due date.
CREATE INDEX IF NOT EXISTS idx_data_access_requests_due
    ON data_access_requests (status, due_on);

-- The athlete's list, newest first.
CREATE INDEX IF NOT EXISTS idx_data_access_requests_user
    ON data_access_requests (tenant_id, user_id, created_at);
