-- ABOUTME: Group invite funnel — how often a link's invite card was opened, and a held invite a full group refused (PostgreSQL)
-- ABOUTME: Lets the coach tell "never opened" from "opened, signed up, not joined" and keeps a full group's hold for a retry (carnet#1649, carnet#1650)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The public invite card (GET /api/invites/<code>) counts each opening of a
-- known link, except by the coach who sent it or the group's owner or coach.
-- Coach Home reads it as the funnel's "opened" step. Only a count and two
-- times: no reader, no address, no device is kept.
ALTER TABLE group_invites ADD COLUMN IF NOT EXISTS open_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE group_invites ADD COLUMN IF NOT EXISTS first_opened_at TIMESTAMPTZ;
ALTER TABLE group_invites ADD COLUMN IF NOT EXISTS last_opened_at TIMESTAMPTZ;

-- A held invite whose join was refused because the group was full is kept,
-- stamped here, instead of dropped: the athlete retries from Home once a seat
-- opens, and the coach's funnel shows the full group rather than "account
-- created". Any other decided refusal still drops the hold.
ALTER TABLE group_invite_holds ADD COLUMN IF NOT EXISTS refused_full_at TIMESTAMPTZ;
