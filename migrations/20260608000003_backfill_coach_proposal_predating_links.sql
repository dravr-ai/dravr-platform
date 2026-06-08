-- ABOUTME: Backfill coach_proposal_sent_at for channel links created before the proposal feature
-- ABOUTME: Stops already-onboarded users from receiving the one-time "Welcome!" proposal retroactively

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The 20260605000001 migration added coach_proposal_sent_at as NULL for every
-- pre-existing link, so users who had been chatting for days received the
-- one-time onboarding proposal on their next turn after deploy. Stamp every
-- link that predates the feature (linked before 2026-06-05) as already
-- proposed, so the proposal only reaches links created from the feature
-- onward. Links created on or after that date are left NULL and receive the
-- proposal normally.
UPDATE messaging_channel_links
SET coach_proposal_sent_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE coach_proposal_sent_at IS NULL
  AND linked_at < '2026-06-05';
