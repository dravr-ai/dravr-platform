-- ABOUTME: Stamps content derived from an athlete's data with whether a first-party-only provider fed it (carnet#769)
-- ABOUTME: Additive only: one first_party_only flag per derived table; rows written before it read as unstamped

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Some provider terms keep their data, raw or derived, on Dravr's own
-- surfaces (Nolio API terms §6.9). Each row below records whether anything it
-- was built from was such data, so an external caller (MCP, A2A, an API key)
-- is withheld only the stamped rows instead of every derived row of an
-- athlete who holds such a connection. Forward-only: no first-party-only
-- provider has written data before this migration, so nothing is backfilled.

-- Set on a reply built while the turn served such data, or replayed such a reply.
ALTER TABLE chat_messages ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a summary of stamped messages.
ALTER TABLE compaction_blocks ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a fact extracted from a stamped reply, or written in a turn that served such data.
ALTER TABLE user_facts ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a note the agent wrote in such a turn.
ALTER TABLE agent_notes ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a followup the agent scheduled in such a turn.
ALTER TABLE agent_followups ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a playbook labelled from stamped advice or such an outcome.
ALTER TABLE coaching_playbooks ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on advice captured from a stamped reply.
ALTER TABLE pending_advice ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a plan saved in such a turn.
ALTER TABLE training_plans ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a verdict on a stamped reply.
ALTER TABLE claim_verdicts ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a room entry copied from a stamped message.
ALTER TABLE group_transcript_entries ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a profile written in such a turn.
ALTER TABLE user_physiological_profiles ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a calendar entry pushed from a stamped plan, or prescribed in such a turn.
ALTER TABLE prescribed_workouts ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a thread opened from a first-party-only activity: the whole thread is about it.
ALTER TABLE activity_conversations ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Set on a commitment recorded in a turn that served such data, or whose verdict counted such sessions.
ALTER TABLE athlete_commitments ADD COLUMN IF NOT EXISTS first_party_only BOOLEAN NOT NULL DEFAULT FALSE;
