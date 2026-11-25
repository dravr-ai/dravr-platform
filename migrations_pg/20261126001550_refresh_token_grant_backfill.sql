-- ABOUTME: Links each unrevoked refresh token minted before grants were named to its grant, when exactly one live grant matches (PostgreSQL)
-- ABOUTME: carnet#1550, JF 2026-11-25 "backfill grant ids": UPDATE only; the rest stay NULL and the server logs them at boot

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- oauth2_refresh_tokens.client_grant_id has been NULL on every token minted
-- before migration 20261122100000 named grants, and a refresh copies it
-- forward, so those families never gain one by rotating. This names the grant
-- of each unrevoked one whose tenant, client and user match exactly one live
-- grant: not revoked, and unexpired (expires_at NULL never expires). A token
-- naming a live grant is admitted while that grant lives, as the NULL one was
-- while the client held one, so no athlete reconnects. A family with no live
-- grant, or with several (one per scope set), is left NULL and admitted as
-- before; the boot log names those by tenant, client and user.
-- Re-running changes nothing: only NULL rows are touched.
UPDATE oauth2_refresh_tokens t
SET client_grant_id = live.id
FROM (
    SELECT tenant_id, client_id, user_id, MIN(id) AS id
    FROM oauth_client_grants
    WHERE revoked_at IS NULL
      AND (expires_at IS NULL OR expires_at > NOW())
    GROUP BY tenant_id, client_id, user_id
    HAVING COUNT(*) = 1
) live
WHERE t.client_grant_id IS NULL
  AND t.revoked = FALSE
  AND t.tenant_id = live.tenant_id
  AND t.client_id = live.client_id
  AND t.user_id = live.user_id;
