-- ABOUTME: oauth_host_profile_claims — the per-request Host Profile claim a step-up stands on for a client with no metadata document (PostgreSQL)
-- ABOUTME: Additive: one new table; a two-way door (a rollback binary never reads it)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Owl phase 5 slice 4b (carnet#1544, owl PDPP §6.2 basis declared_request,
-- JF 2026-11-26). A client registered by DCR has no metadata
-- document, so it has no Host Profile on carrier A. It may still reach T2
-- on its own per-request claim (carrier B: the MCP client extension
-- app.dravr.vault/host-profile), which owl requires be "seen on the request
-- that triggered consent". The authorize request that follows is a browser
-- redirect carrying no MCP _meta, so the claim is recorded here when /mcp
-- sees it on a call a step-up would cure, and the step-up approval reads it.
-- Every later call is still checked against its own claim (the ceiling).
--   client_grant_id  the live parent grant the call was made under
--                    (oauth_client_grants.id); one row per parent, replaced
--                    by the next claim seen under it
--   tenant_id, user_id, client_id  the parent's; every read filters the
--                    tenant and the user
--   tier_max         the tier the call declared: T2 (the only one recorded)
--   seen_at          when /mcp saw it; a step-up approval stands on a claim
--                    at most HOST_PROFILE_CLAIM_MAX_AGE_SECS old

CREATE TABLE IF NOT EXISTS oauth_host_profile_claims (
    client_grant_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    tier_max TEXT NOT NULL,
    seen_at TIMESTAMPTZ NOT NULL
);

-- An athlete's claims, as the account's export and delete read them.
CREATE INDEX IF NOT EXISTS idx_oauth_host_profile_claims_owner
    ON oauth_host_profile_claims (tenant_id, user_id);
