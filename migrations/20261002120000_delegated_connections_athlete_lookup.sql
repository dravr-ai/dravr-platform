-- ABOUTME: Index for resolving a provider push event's athlete id to its confirmed delegated connection (SQLite)
-- ABOUTME: Covers confirmed rows only: a proposed or revoked link is never a webhook's owner

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A coach platform's webhook names the athlete while the token belongs to the
-- coach, so the event is resolved from (provider, provider_athlete_id) to the
-- confirmed link and its member. The existing indexes all lead with a user or
-- a group; none serves a lookup that starts from the athlete.
CREATE INDEX IF NOT EXISTS idx_delegated_connections_athlete_confirmed
    ON delegated_connections(provider, provider_athlete_id) WHERE status = 'confirmed';
