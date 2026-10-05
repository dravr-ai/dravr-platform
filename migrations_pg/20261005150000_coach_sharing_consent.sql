-- ABOUTME: Per-member coach_sharing_consent, split from peer_sharing_consent: the group's coach reads a member by this flag (PostgreSQL)
-- ABOUTME: Joining grants it (ADR-002: membership is consent to the coach); a member who had explicitly revoked sharing keeps it off (carnet#786)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- peer_sharing_consent used to gate both the coach's read and every other
-- member's read. ADR-002 separates them: joining a group is consent to share
-- with its coach, while peers stay opt-in. Additive only: peer_sharing_consent
-- keeps its meaning (peers) and its values.
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_sharing_consent BOOLEAN NOT NULL DEFAULT TRUE;

-- The default backfills every existing member as sharing with the coach,
-- which is what ADR-002 says membership means. The one exception is a member
-- who explicitly turned sharing off: the insert writes consent_given_at and
-- joined_at from one instant, and only a /group consent decision moves
-- consent_given_at, so a row whose consent is off and whose consent instant
-- differs from its join instant is a member who said no while that no still
-- covered the coach. Their refusal stands.
UPDATE coaching_group_members
   SET coach_sharing_consent = FALSE
 WHERE peer_sharing_consent = FALSE
   AND consent_given_at <> joined_at;
