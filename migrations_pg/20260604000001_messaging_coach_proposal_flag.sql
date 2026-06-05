-- ABOUTME: Per-(user,channel) flag marking that the onboarding coach proposal was auto-sent
-- ABOUTME: NULL = not yet proposed; set to the send timestamp once the messaging auto-send fires (one-time)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The messaging ingress auto-sends a one-time coach proposal on the user's
-- first provider-connected turn. This column makes that idempotent: once the
-- proposal is delivered for a (tenant, channel_type, channel_user_id) link, the
-- timestamp is set and the ingress never re-sends. NULL = not yet proposed.
ALTER TABLE messaging_channel_links ADD COLUMN IF NOT EXISTS coach_proposal_sent_at TIMESTAMPTZ;
