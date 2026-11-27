// ABOUTME: The reads of an OAuth consent grant row: a client's live grant, a user's grants, a client's history
// ABOUTME: The lists read parents only, a step-up child through its parent; the read by id returns either
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// The ids of a user's live grants to one client in one tenant, but one:
/// what an approval of that one revokes. Parents only: a child goes with its
/// parent.
pub(crate) const LIST_OTHER_LIVE_CLIENT_GRANTS_SQL: &str = r"
            SELECT id FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND client_id = $3 AND id <> $4
              AND revoked_at IS NULL AND parent_grant_id IS NULL
              AND (scope = 'pdpp:agent_write') = $5
            ";

/// The live grant for one `(user, tenant, client, scope)` tuple: a parent,
/// never a step-up child, which shares its parent's scope.
pub(crate) const FIND_ACTIVE_CLIENT_GRANT_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND client_id = $3 AND scope = $4
              AND revoked_at IS NULL AND parent_grant_id IS NULL
            LIMIT 1
            ";

/// A user's live grants within a tenant, newest first; parents only (each
/// one's child is listed under it).
pub(crate) const LIST_CLIENT_GRANTS_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND revoked_at IS NULL AND parent_grant_id IS NULL
            ORDER BY granted_at DESC
            ";

/// One grant by its id, live or not, a parent or a step-up child (the
/// single-use check reads a child ceiling's grant by the child's id);
/// `user_id` + `tenant_id` in the filter is the ownership check.
pub(crate) const GET_CLIENT_GRANT_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at
            FROM oauth_client_grants
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ";

/// Every grant, live or revoked, a user gave one client in one tenant;
/// parents only: a child consumed or revoked never reads as the client's
/// access withdrawn.
pub(crate) const LIST_CLIENT_GRANT_HISTORY_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND client_id = $3 AND parent_grant_id IS NULL
            ";

/// The live step-up children of a user's grants to client `$3` in tenant `$2`, with their
/// step-up columns.
///
/// An agent-write child (scope `pdpp:agent_write`) is not a step-up child: it reads nothing,
/// and is never a read ceiling.
pub(crate) const LIST_CLIENT_LIVE_CHILD_GRANTS_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at,
                   parent_grant_id, step_up_auth_time, step_up_acr, host_profile_basis,
                   host_profile_tier
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND client_id = $3 AND revoked_at IS NULL
              AND parent_grant_id IS NOT NULL AND scope <> 'pdpp:agent_write'
            ";

/// The live step-up children of a user's grants in tenant `$2`, newest
/// first, with their step-up columns: what Connected apps lists under each
/// parent. Agent-write children are not among them.
pub(crate) const LIST_LIVE_CHILD_GRANTS_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at,
                   parent_grant_id, step_up_auth_time, step_up_acr, host_profile_basis,
                   host_profile_tier
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND revoked_at IS NULL
              AND parent_grant_id IS NOT NULL AND scope <> 'pdpp:agent_write'
            ORDER BY granted_at DESC
            ";
