-- ABOUTME: user_invitations.kind — what redeeming an invitation gives besides the account: nothing (NULL) or a coach seat (PostgreSQL)
-- ABOUTME: Additive and nullable: every invitation sent before this column reads as a plain account invitation (carnet#1524)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A super-admin can invite a coach the team recruited: the same single-use
-- mailed link, locked to the same address, whose redemption also grants
-- coach access (manages_roster, as an operator grant by the inviter) and
-- records the coach role, so the coach meets neither the role question nor
-- the coach-access request (ADR-018: a super-admin decides coach access,
-- here before sign-up rather than after a request).
--   kind  NULL          an account and nothing else (every row before this)
--         'coach_seat'  an account with coach access and the coach role
-- The vocabulary is enforced where the row is written, so a new kind needs
-- no table rebuild. The live-email index still allows one live invitation
-- per address, whatever its kind.
ALTER TABLE user_invitations ADD COLUMN IF NOT EXISTS kind TEXT;
