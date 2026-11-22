-- ABOUTME: Marks a coach read made in an admin's support session, by the admin acting as the coach (PostgreSQL)
-- ABOUTME: Adds coach_data_reads.impersonator_user_id: NULL for the coach's own read, else the admin (carnet#1511)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- impersonator_user_id: the admin whose support session made the read while
-- acting as the coach (impersonation_sessions.impersonator_id), so the
-- athlete's record says the read was support's, not their coach's. NULL for
-- a read the coach made themselves. No foreign key on purpose: the athlete's
-- record keeps saying "support" after that admin's account is gone, as
-- coach_user_id keeps the read itself. Written for a read in any support
-- session: the admin's assistant over MCP or A2A, or a chat turn the admin
-- typed on the web. Additive and nullable; no backfill: the reads recorded
-- before this column are not guessed.
ALTER TABLE coach_data_reads ADD COLUMN IF NOT EXISTS impersonator_user_id UUID;
