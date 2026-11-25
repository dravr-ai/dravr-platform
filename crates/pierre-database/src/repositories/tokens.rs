// ABOUTME: Shared statements, row decoders and body for the OAuth 2.0 server — clients, codes, refresh tokens,
// ABOUTME: CSRF states, client grants and RFC 8628 device codes; one SQL text per operation, emitted per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The OAuth 2.0 server's persistence, written once.
//!
//! Every table here declares its id columns as `TEXT` on both backends —
//! `oauth2_auth_codes.user_id`, `oauth2_refresh_tokens.user_id` and
//! `oauth2_states.user_id` included — so a uuid binds as its hyphenated text
//! and is parsed on the way back on both drivers, and no uuid codec is
//! needed. Timestamps bind as [`Timestamp`] on both (`TIMESTAMPTZ` on
//! Postgres, RFC 3339 text on `SQLite`, which orders correctly for the
//! `expires_at > now` filters because every value shares one offset and
//! width); `device_authorization` keeps epoch seconds in a `BIGINT`/`INTEGER`
//! column and reads as `i64` on both. Booleans are spelled `TRUE`/`FALSE`,
//! which both engines accept (`SQLite` stores them as 1/0).
//!
//! The refresh token value is never stored: `store_refresh_token` keeps its
//! HMAC-SHA256 under the blind-index key through
//! [`HasEncryption::hash_token_for_storage`], and every lookup hashes the
//! presented token the same way. The consume paths are single statements
//! (`UPDATE … WHERE … RETURNING`), so an authorization code is exchanged at
//! most once even under concurrent requests (RFC 6749 §4.1.2, §10.5), a
//! refresh token is rotated at most once, and a CSRF state is redeemed at
//! most once (RFC 6749 §10.12).
//!
//! Client registrations are reclaimed, not only gated by their expiry.
//! `store_refresh_token` stamps the client's `last_authorized_at` in the same
//! transaction as the token, which is what separates a client a user connected
//! from a registration nobody finished; `store_client_within_ceiling` holds the
//! unfinished ones to a ceiling, and `delete_stale_clients` deletes them once
//! abandoned, along with every registration past its expiry grace.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them.

/// The live refresh-token families the one-time grant backfill left unnamed
pub(crate) mod grant_backfill;
/// Every statement that starts or ends a consent grant, and its receipts
pub(crate) mod grant_writes;
/// The RFC 9126 pushed authorization request store
pub(crate) mod pushed;
mod rows;

pub(crate) use grant_backfill::impl_ungranted_refresh_families;
pub(crate) use grant_writes::impl_client_grant_writes;
pub(crate) use pushed::impl_pushed_request_store;
pub(crate) use rows::{
    client_grant_from_row, device_authorization_from_row, oauth2_auth_code_from_row,
    oauth2_client_from_row, oauth2_refresh_token_from_row, oauth2_state_from_row,
};

/// Register an RFC 7591 client unless `$12` registrations are already
/// pending — carrying an expiry and never issued a refresh token. The count
/// and the insert are one statement, so no second round trip separates the
/// check from the write. The list columns hold JSON arrays as text.
pub(crate) const STORE_OAUTH2_CLIENT_WITHIN_CEILING_SQL: &str = r"
            INSERT INTO oauth2_clients (id, client_id, client_secret_hash, redirect_uris, grant_types, response_types, client_name, client_uri, scope, created_at, expires_at)
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11
            WHERE (
                SELECT COUNT(*) FROM oauth2_clients
                WHERE expires_at IS NOT NULL AND last_authorized_at IS NULL
            ) < $12
            ";

/// Register a client provisioned by the server itself rather than through
/// RFC 7591 — no pending ceiling applies. The list columns hold JSON arrays
/// as text.
pub(crate) const STORE_OAUTH2_CLIENT_SQL: &str = r"
            INSERT INTO oauth2_clients (id, client_id, client_secret_hash, redirect_uris, grant_types, response_types, client_name, client_uri, scope, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ";

/// Rewrite a public client's metadata (`client_secret_hash` empty) from a
/// freshly fetched client metadata document.
pub(crate) const UPDATE_PUBLIC_OAUTH2_CLIENT_METADATA_SQL: &str = r"
            UPDATE oauth2_clients
            SET redirect_uris = $2, grant_types = $3, response_types = $4,
                client_name = $5, client_uri = $6, scope = $7, expires_at = $8
            WHERE client_id = $1 AND client_secret_hash = ''
            ";

/// Slide the expiry of a registration that has one forward, never back.
pub(crate) const EXTEND_OAUTH2_CLIENT_EXPIRY_SQL: &str = r"
            UPDATE oauth2_clients SET expires_at = $2
            WHERE client_id = $1 AND expires_at IS NOT NULL AND expires_at < $2
            ";

/// One client by its public `client_id`; codes, refresh tokens and states go
/// with it by cascade.
pub(crate) const DELETE_OAUTH2_CLIENT_SQL: &str = r"
            DELETE FROM oauth2_clients WHERE client_id = $1
            ";

/// Registrations whose expiry passed before `$1`, the grace cutoff. A row
/// without an expiry was not written by dynamic registration and never matches.
pub(crate) const DELETE_EXPIRED_OAUTH2_CLIENTS_SQL: &str = r"
            DELETE FROM oauth2_clients
            WHERE expires_at IS NOT NULL AND expires_at < $1
            ";

/// Pending registrations created before `$1`: registered, and no refresh
/// token ever issued through them, so no user finished authorizing them.
pub(crate) const DELETE_ABANDONED_OAUTH2_CLIENTS_SQL: &str = r"
            DELETE FROM oauth2_clients
            WHERE expires_at IS NOT NULL AND last_authorized_at IS NULL AND created_at < $1
            ";

/// Record that a refresh token was issued through client `$1` at `$2`. Only a
/// user-bound grant issues one, so this is what marks a registration authorized.
pub(crate) const STAMP_OAUTH2_CLIENT_AUTHORIZED_SQL: &str = r"
            UPDATE oauth2_clients SET last_authorized_at = $2 WHERE client_id = $1
            ";

/// One client by its public `client_id`.
pub(crate) const GET_OAUTH2_CLIENT_SQL: &str = r"
            SELECT id, client_id, client_secret_hash, redirect_uris, grant_types, response_types, client_name, client_uri, scope, created_at, expires_at
            FROM oauth2_clients
            WHERE client_id = $1
            ";

/// Mint an authorization code with its PKCE challenge and RFC 8707 resource.
pub(crate) const STORE_OAUTH2_AUTH_CODE_SQL: &str = r"
            INSERT INTO oauth2_auth_codes (code, client_id, user_id, tenant_id, redirect_uri, scope, code_challenge, code_challenge_method, expires_at, used, state, resource, client_grant_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ";

/// Exchange the code exactly once: it must belong to the client, match the
/// redirect it was issued for, be unused and unexpired at `$4`. A second
/// exchange, even a concurrent one, matches zero rows.
pub(crate) const CONSUME_OAUTH2_AUTH_CODE_SQL: &str = r"
            UPDATE oauth2_auth_codes
            SET used = TRUE
            WHERE code = $1
              AND client_id = $2
              AND redirect_uri = $3
              AND used = FALSE
              AND expires_at > $4
            RETURNING code, client_id, user_id, tenant_id, redirect_uri, scope, expires_at, used, state, code_challenge, code_challenge_method, resource, client_grant_id
            ";

/// The code [`CONSUME_OAUTH2_AUTH_CODE_SQL`] would exchange right now, read
/// without spending it: the same client, redirect, unused and unexpired at
/// `$4` conditions, so a row here is one the exchange could still match.
pub(crate) const PEEK_OAUTH2_AUTH_CODE_SQL: &str = r"
            SELECT code, client_id, user_id, tenant_id, redirect_uri, scope, expires_at, used, state, code_challenge, code_challenge_method, resource, client_grant_id
            FROM oauth2_auth_codes
            WHERE code = $1
              AND client_id = $2
              AND redirect_uri = $3
              AND used = FALSE
              AND expires_at > $4
            ";

/// Store a refresh token under its HMAC, with the RFC 8707 resource its grant
/// is bound to.
pub(crate) const STORE_OAUTH2_REFRESH_TOKEN_SQL: &str = r"
            INSERT INTO oauth2_refresh_tokens (token, client_id, user_id, tenant_id, scope, created_at, expires_at, revoked, family_id, resource, family_created_at, client_grant_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ";

/// A refresh token by its HMAC, whatever its state.
pub(crate) const GET_OAUTH2_REFRESH_TOKEN_SQL: &str = r"
            SELECT token, client_id, user_id, tenant_id, scope, created_at, expires_at, revoked, family_id, resource, family_created_at, client_grant_id
            FROM oauth2_refresh_tokens
            WHERE token = $1
            ";

/// Rotate a refresh token exactly once: live, unexpired at `$3` and owned by
/// the client presenting it.
pub(crate) const CONSUME_OAUTH2_REFRESH_TOKEN_SQL: &str = r"
            UPDATE oauth2_refresh_tokens
            SET revoked = TRUE
            WHERE token = $1
              AND client_id = $2
              AND revoked = FALSE
              AND expires_at > $3
            RETURNING token, client_id, user_id, tenant_id, scope, created_at, expires_at, revoked, family_id, resource, family_created_at, client_grant_id
            ";

/// Revoke every live member of the rotation chain the token (by HMAC)
/// belongs to.
pub(crate) const REVOKE_OAUTH2_REFRESH_TOKEN_FAMILY_SQL: &str = r"
            UPDATE oauth2_refresh_tokens
            SET revoked = TRUE
            WHERE revoked = FALSE
              AND family_id = (
                  SELECT family_id FROM oauth2_refresh_tokens WHERE token = $1
              )
            ";

/// Revoke every live refresh token one user holds in one tenant, through any client.
pub(crate) const REVOKE_USER_OAUTH2_REFRESH_TOKENS_SQL: &str = r"
            UPDATE oauth2_refresh_tokens
            SET revoked = TRUE
            WHERE tenant_id = $1
              AND user_id = $2
              AND revoked = FALSE
            ";

/// Mint a CSRF state for one authorization round trip.
pub(crate) const STORE_OAUTH2_STATE_SQL: &str = r"
            INSERT INTO oauth2_states (state, client_id, user_id, tenant_id, redirect_uri, scope, code_challenge, code_challenge_method, created_at, expires_at, used)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ";

/// Redeem a state exactly once, for the client it was minted for.
pub(crate) const CONSUME_OAUTH2_STATE_SQL: &str = r"
            UPDATE oauth2_states
            SET used = TRUE
            WHERE state = $1
              AND client_id = $2
              AND used = FALSE
              AND expires_at > $3
            RETURNING state, client_id, user_id, tenant_id, redirect_uri, scope, code_challenge, code_challenge_method, created_at, expires_at, used
            ";

/// The ids of a user's live grants to one client in one tenant, but one:
/// what an approval of that one revokes.
pub(crate) const LIST_OTHER_LIVE_CLIENT_GRANTS_SQL: &str = r"
            SELECT id FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND client_id = $3 AND id <> $4
              AND revoked_at IS NULL
            ";

/// The live grant for one `(user, tenant, client, scope)` tuple.
pub(crate) const FIND_ACTIVE_CLIENT_GRANT_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND client_id = $3 AND scope = $4
              AND revoked_at IS NULL
            LIMIT 1
            ";

/// A user's live grants within a tenant, newest first.
pub(crate) const LIST_CLIENT_GRANTS_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND revoked_at IS NULL
            ORDER BY granted_at DESC
            ";

/// One grant by its id, live or not; `user_id` + `tenant_id` in the filter is
/// the ownership check.
pub(crate) const GET_CLIENT_GRANT_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at
            FROM oauth_client_grants
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ";

/// Every grant, live or revoked, a user gave one client in one tenant.
pub(crate) const LIST_CLIENT_GRANT_HISTORY_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at
            FROM oauth_client_grants
            WHERE user_id = $1 AND tenant_id = $2 AND client_id = $3
            ";

/// Revoke every live refresh token one user holds in one tenant through one
/// client: every family the revoked grant started.
pub(crate) const REVOKE_CLIENT_OAUTH2_REFRESH_TOKENS_SQL: &str = r"
            UPDATE oauth2_refresh_tokens
            SET revoked = TRUE
            WHERE tenant_id = $1
              AND user_id = $2
              AND client_id = $3
              AND revoked = FALSE
            ";

/// Burn every unredeemed authorization code one user holds in one tenant
/// through one client, so a code issued just before a grant is revoked cannot
/// start a new refresh family after it.
pub(crate) const REVOKE_CLIENT_OAUTH2_AUTH_CODES_SQL: &str = r"
            UPDATE oauth2_auth_codes
            SET used = TRUE
            WHERE tenant_id = $1
              AND user_id = $2
              AND client_id = $3
              AND used = FALSE
            ";

/// Revoke the refresh tokens of a consent the user withdrew: the client has a
/// revoked grant and no active one. A client that never had a grant row is not
/// a withdrawn one.
pub(crate) const REVOKE_WITHDRAWN_OAUTH2_REFRESH_TOKENS_SQL: &str = r"
            UPDATE oauth2_refresh_tokens
            SET revoked = TRUE
            WHERE tenant_id = $1
              AND user_id = $2
              AND client_id = $3
              AND revoked = FALSE
              AND EXISTS (
                  SELECT 1 FROM oauth_client_grants g
                  WHERE g.tenant_id = $1 AND g.user_id = $2 AND g.client_id = $3
                    AND g.revoked_at IS NOT NULL
              )
              AND NOT EXISTS (
                  SELECT 1 FROM oauth_client_grants g
                  WHERE g.tenant_id = $1 AND g.user_id = $2 AND g.client_id = $3
                    AND g.revoked_at IS NULL
              )
            ";

/// Delete every device authorization past its expiry: `$1` is now, in epoch
/// seconds. `<=` is the comparison the token endpoint and the approval read
/// an expired row by, so what goes is exactly what neither would honour.
///
/// Nothing else removes a row whose device never polled again (a login
/// started and abandoned, or one denied and never collected), so the store
/// runs this each time it opens a new authorization: the table holds the
/// logins still inside their window, and each abandoned row is deleted
/// once, by the next login to start. `idx_device_authorization_expires_at`
/// serves the range.
pub(crate) const SWEEP_EXPIRED_DEVICE_AUTHORIZATIONS_SQL: &str =
    "DELETE FROM device_authorization WHERE expires_at <= $1";

/// Open an RFC 8628 device authorization, pending until an admin acts.
pub(crate) const CREATE_DEVICE_AUTHORIZATION_SQL: &str = r"
            INSERT INTO device_authorization
                (device_code_hash, user_code, status, approved_by, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ";

/// A device authorization by the hash of its device code (the polling side).
pub(crate) const GET_DEVICE_AUTHORIZATION_BY_CODE_HASH_SQL: &str = r"
            SELECT device_code_hash, user_code, status, approved_by, created_at, expires_at
            FROM device_authorization
            WHERE device_code_hash = $1
            ";

/// A device authorization by its user code (the admin side).
pub(crate) const GET_DEVICE_AUTHORIZATION_BY_USER_CODE_SQL: &str = r"
            SELECT device_code_hash, user_code, status, approved_by, created_at, expires_at
            FROM device_authorization
            WHERE user_code = $1
            ";

/// Approve a still-pending authorization; an already-decided one matches nothing.
pub(crate) const APPROVE_DEVICE_AUTHORIZATION_SQL: &str = r"
            UPDATE device_authorization
            SET status = 'approved', approved_by = $2
            WHERE user_code = $1 AND status = 'pending'
            ";

/// Deny a still-pending authorization.
pub(crate) const DENY_DEVICE_AUTHORIZATION_SQL: &str = r"
            UPDATE device_authorization
            SET status = 'denied'
            WHERE user_code = $1 AND status = 'pending'
            ";

/// Consume a device authorization; the token endpoint mints only when a row
/// went, so a duplicate poll can never mint twice.
pub(crate) const DELETE_DEVICE_AUTHORIZATION_SQL: &str =
    "DELETE FROM device_authorization WHERE device_code_hash = $1";

/// Emit the whole [`OAuth2ServerRepository`](super::OAuth2ServerRepository)
/// implementation for one backend type.
///
/// No per-engine argument: every column involved has the same shape on both
/// backends (see the module doc). The body is written once here; each
/// backend's shell invokes it with its own type, and sqlx resolves the driver
/// from `self.pool()` per expansion. The body names its consts, helpers and
/// types unqualified, so the invoking shell must `use` every one of them.
macro_rules! impl_oauth2_server_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl OAuth2ServerRepository for $ty {
            async fn store_client_within_ceiling(
                &self,
                client: &OAuth2Client,
                ceiling: u64,
            ) -> AppResult<bool> {
                // COUNT(*) is a signed 64-bit value on both engines; a ceiling
                // past its range is no ceiling at all.
                let ceiling = i64::try_from(ceiling).unwrap_or(i64::MAX);
                let result = sqlx::query(STORE_OAUTH2_CLIENT_WITHIN_CEILING_SQL)
                    .bind(&client.id)
                    .bind(&client.client_id)
                    .bind(&client.client_secret_hash)
                    .bind(serde_json::to_string(&client.redirect_uris)?)
                    .bind(serde_json::to_string(&client.grant_types)?)
                    .bind(serde_json::to_string(&client.response_types)?)
                    .bind(&client.client_name)
                    .bind(&client.client_uri)
                    .bind(&client.scope)
                    .bind(client.created_at)
                    .bind(client.expires_at)
                    .bind(ceiling)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store OAuth2 client: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn store_client(&self, client: &OAuth2Client) -> AppResult<()> {
                sqlx::query(STORE_OAUTH2_CLIENT_SQL)
                    .bind(&client.id)
                    .bind(&client.client_id)
                    .bind(&client.client_secret_hash)
                    .bind(serde_json::to_string(&client.redirect_uris)?)
                    .bind(serde_json::to_string(&client.grant_types)?)
                    .bind(serde_json::to_string(&client.response_types)?)
                    .bind(&client.client_name)
                    .bind(&client.client_uri)
                    .bind(&client.scope)
                    .bind(client.created_at)
                    .bind(client.expires_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store OAuth2 client: {e}"))
                    })?;
                Ok(())
            }

            async fn update_public_client_metadata(
                &self,
                client: &OAuth2Client,
            ) -> AppResult<bool> {
                let result = sqlx::query(UPDATE_PUBLIC_OAUTH2_CLIENT_METADATA_SQL)
                    .bind(&client.client_id)
                    .bind(serde_json::to_string(&client.redirect_uris)?)
                    .bind(serde_json::to_string(&client.grant_types)?)
                    .bind(serde_json::to_string(&client.response_types)?)
                    .bind(&client.client_name)
                    .bind(&client.client_uri)
                    .bind(&client.scope)
                    .bind(client.expires_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update OAuth2 client metadata: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn extend_client_expiry(
                &self,
                client_id: &str,
                expires_at: Timestamp,
            ) -> AppResult<()> {
                sqlx::query(EXTEND_OAUTH2_CLIENT_EXPIRY_SQL)
                    .bind(client_id)
                    .bind(expires_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to extend OAuth2 client expiry: {e}"))
                    })?;
                Ok(())
            }

            async fn delete_client(&self, client_id: &str) -> AppResult<bool> {
                let delete_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to delete OAuth2 client: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(delete_error)?;

                let deleted = sqlx::query(DELETE_OAUTH2_CLIENT_SQL)
                    .bind(client_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(delete_error)?
                    .rows_affected();
                // Its only callers act for the client's owner (an A2A
                // client deactivated, or its creation undone): the client
                // ended the grants.
                Self::delete_client_grants_in(&mut tx, client_id, GrantActor::Client).await?;
                // Its refresh families went with it: so do the support
                // sessions bound to one (carnet#1511).
                Self::end_sessions_of_revoked(
                    &mut tx,
                    $crate::repositories::support_binding::RevokedCredential::DeletedFamilies,
                )
                .await?;

                tx.commit().await.map_err(delete_error)?;
                Ok(deleted > 0)
            }

            async fn delete_stale_clients(
                &self,
                expired_before: Timestamp,
                unauthorized_before: Timestamp,
                now: Timestamp,
            ) -> AppResult<OAuth2ClientSweep> {
                let sweep_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to sweep OAuth2 clients: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(sweep_error)?;

                let expired = sqlx::query(DELETE_EXPIRED_OAUTH2_CLIENTS_SQL)
                    .bind(expired_before)
                    .execute(&mut *tx)
                    .await
                    .map_err(sweep_error)?
                    .rows_affected();
                let abandoned = sqlx::query(DELETE_ABANDONED_OAUTH2_CLIENTS_SQL)
                    .bind(unauthorized_before)
                    .execute(&mut *tx)
                    .await
                    .map_err(sweep_error)?
                    .rows_affected();
                // Before the orphan delete, so an expired grant of a deleted
                // client ends as expired.
                let expired_grants = Self::expire_grants_in(&mut tx, now).await?;
                let orphaned_grants = Self::delete_orphaned_grants_in(&mut tx).await?;
                let expired_pushed_requests = sqlx::query(
                    $crate::repositories::tokens::pushed::DELETE_EXPIRED_PUSHED_REQUESTS_SQL,
                )
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(sweep_error)?
                .rows_affected();
                Self::end_sessions_of_revoked(
                    &mut tx,
                    $crate::repositories::support_binding::RevokedCredential::DeletedFamilies,
                )
                .await?;

                tx.commit().await.map_err(sweep_error)?;

                Ok(OAuth2ClientSweep {
                    expired,
                    abandoned,
                    orphaned_grants,
                    expired_grants,
                    expired_pushed_requests,
                })
            }

            async fn get_client(&self, client_id: &str) -> AppResult<Option<OAuth2Client>> {
                let row = sqlx::query(GET_OAUTH2_CLIENT_SQL)
                    .bind(client_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query OAuth2 client: {e}"))
                    })?;

                row.map(|row| oauth2_client_from_row(&row)).transpose()
            }

            async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()> {
                sqlx::query(STORE_OAUTH2_AUTH_CODE_SQL)
                    .bind(&auth_code.code)
                    .bind(&auth_code.client_id)
                    .bind(auth_code.user_id.to_string())
                    .bind(&auth_code.tenant_id)
                    .bind(&auth_code.redirect_uri)
                    .bind(&auth_code.scope)
                    .bind(&auth_code.code_challenge)
                    .bind(&auth_code.code_challenge_method)
                    .bind(auth_code.expires_at)
                    .bind(auth_code.used)
                    .bind(&auth_code.state)
                    .bind(&auth_code.resource)
                    .bind(&auth_code.client_grant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store OAuth2 auth code: {e}"))
                    })?;

                Ok(())
            }

            async fn store_refresh_token(
                &self,
                refresh_token: &OAuth2RefreshToken,
            ) -> AppResult<()> {
                let token_hash = HasEncryption::hash_token_for_storage(
                    self,
                    refresh_token.token.expose_secret(),
                )?;
                let store_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to store OAuth2 refresh token: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(store_error)?;

                sqlx::query(STORE_OAUTH2_REFRESH_TOKEN_SQL)
                    .bind(&token_hash)
                    .bind(&refresh_token.client_id)
                    .bind(refresh_token.user_id.to_string())
                    .bind(&refresh_token.tenant_id)
                    .bind(&refresh_token.scope)
                    .bind(refresh_token.created_at)
                    .bind(refresh_token.expires_at)
                    .bind(refresh_token.revoked)
                    .bind(&refresh_token.family_id)
                    .bind(&refresh_token.resource)
                    .bind(refresh_token.family_created_at)
                    .bind(&refresh_token.client_grant_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;
                sqlx::query(STAMP_OAUTH2_CLIENT_AUTHORIZED_SQL)
                    .bind(&refresh_token.client_id)
                    .bind(refresh_token.created_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;

                tx.commit().await.map_err(store_error)
            }

            async fn consume_auth_code(
                &self,
                code: &str,
                client_id: &str,
                redirect_uri: &str,
                now: Timestamp,
            ) -> AppResult<Option<OAuth2AuthCode>> {
                let row = sqlx::query(CONSUME_OAUTH2_AUTH_CODE_SQL)
                    .bind(code)
                    .bind(client_id)
                    .bind(redirect_uri)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to consume OAuth2 auth code: {e}"))
                    })?;

                row.map(|row| oauth2_auth_code_from_row(&row)).transpose()
            }

            async fn peek_auth_code(
                &self,
                code: &str,
                client_id: &str,
                redirect_uri: &str,
                now: Timestamp,
            ) -> AppResult<Option<OAuth2AuthCode>> {
                let row = sqlx::query(PEEK_OAUTH2_AUTH_CODE_SQL)
                    .bind(code)
                    .bind(client_id)
                    .bind(redirect_uri)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read OAuth2 auth code: {e}"))
                    })?;

                row.map(|row| oauth2_auth_code_from_row(&row)).transpose()
            }

            async fn consume_refresh_token(
                &self,
                token: &str,
                client_id: &str,
                now: Timestamp,
            ) -> AppResult<Option<OAuth2RefreshToken>> {
                let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

                let row = sqlx::query(CONSUME_OAUTH2_REFRESH_TOKEN_SQL)
                    .bind(&token_hash)
                    .bind(client_id)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to consume OAuth2 refresh token: {e}"))
                    })?;

                row.map(|row| oauth2_refresh_token_from_row(&row))
                    .transpose()
            }

            async fn revoke_refresh_token_family(&self, token: &str) -> AppResult<u64> {
                let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

                let revoke_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to revoke OAuth2 refresh token family: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(revoke_error)?;
                let result = sqlx::query(REVOKE_OAUTH2_REFRESH_TOKEN_FAMILY_SQL)
                    .bind(&token_hash)
                    .execute(&mut *tx)
                    .await
                    .map_err(revoke_error)?;
                Self::end_sessions_of_revoked(
                    &mut tx,
                    $crate::repositories::support_binding::RevokedCredential::FamilyOfToken(
                        &token_hash,
                    ),
                )
                .await?;
                tx.commit().await.map_err(revoke_error)?;

                Ok(result.rows_affected())
            }

            async fn revoke_user_refresh_tokens(
                &self,
                user_id: &str,
                tenant_id: &str,
            ) -> AppResult<u64> {
                let revoke_error = |e: sqlx::Error| {
                    AppError::database(format!(
                        "Failed to revoke the user's OAuth2 refresh tokens: {e}"
                    ))
                };
                let mut tx = self.pool().begin().await.map_err(revoke_error)?;
                let result = sqlx::query(REVOKE_USER_OAUTH2_REFRESH_TOKENS_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(revoke_error)?;
                let families =
                    $crate::repositories::support_binding::RevokedCredential::UserFamilies {
                        tenant_id,
                        user_id,
                    };
                Self::end_sessions_of_revoked(&mut tx, families).await?;
                tx.commit().await.map_err(revoke_error)?;

                Ok(result.rows_affected())
            }

            async fn get_refresh_token_by_value(
                &self,
                token: &str,
            ) -> AppResult<Option<OAuth2RefreshToken>> {
                let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

                let row = sqlx::query(GET_OAUTH2_REFRESH_TOKEN_SQL)
                    .bind(&token_hash)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query OAuth2 refresh token: {e}"))
                    })?;

                row.map(|row| oauth2_refresh_token_from_row(&row))
                    .transpose()
            }

            async fn store_state(&self, state: &OAuth2State) -> AppResult<()> {
                sqlx::query(STORE_OAUTH2_STATE_SQL)
                    .bind(&state.state)
                    .bind(&state.client_id)
                    .bind(state.user_id.map(|id| id.to_string()))
                    .bind(&state.tenant_id)
                    .bind(&state.redirect_uri)
                    .bind(&state.scope)
                    .bind(&state.code_challenge)
                    .bind(&state.code_challenge_method)
                    .bind(state.created_at)
                    .bind(state.expires_at)
                    .bind(state.used)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        // `state` is the client's own value and the table's
                        // primary key: a repeat is the client reusing it.
                        if e.as_database_error()
                            .is_some_and(|db| db.is_unique_violation())
                        {
                            AppError::already_exists("OAuth2 state")
                        } else {
                            AppError::database(format!("Failed to store OAuth2 state: {e}"))
                        }
                    })?;

                Ok(())
            }

            async fn consume_state(
                &self,
                state_value: &str,
                client_id: &str,
                now: Timestamp,
            ) -> AppResult<Option<OAuth2State>> {
                let row = sqlx::query(CONSUME_OAUTH2_STATE_SQL)
                    .bind(state_value)
                    .bind(client_id)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to consume OAuth2 state: {e}"))
                    })?;

                row.map(|row| oauth2_state_from_row(&row)).transpose()
            }

            async fn store_client_grant(
                &self,
                grant: &OAuthClientGrant,
                approval: &GrantApproval,
            ) -> AppResult<String> {
                self.store_grant_rows(grant, approval).await
            }

            async fn get_client_grant(
                &self,
                id: &str,
                user_id: &str,
                tenant_id: &str,
            ) -> AppResult<Option<OAuthClientGrant>> {
                let row = sqlx::query(GET_CLIENT_GRANT_SQL)
                    .bind(id)
                    .bind(user_id)
                    .bind(tenant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read OAuth client grant: {e}"))
                    })?;

                row.map(|row| client_grant_from_row(&row)).transpose()
            }

            async fn find_active_client_grant(
                &self,
                user_id: &str,
                tenant_id: &str,
                client_id: &str,
                scope: &str,
            ) -> AppResult<Option<OAuthClientGrant>> {
                let row = sqlx::query(FIND_ACTIVE_CLIENT_GRANT_SQL)
                    .bind(user_id)
                    .bind(tenant_id)
                    .bind(client_id)
                    .bind(scope)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query OAuth client grant: {e}"))
                    })?;

                row.map(|row| client_grant_from_row(&row)).transpose()
            }

            async fn list_client_grants(
                &self,
                user_id: &str,
                tenant_id: &str,
            ) -> AppResult<Vec<OAuthClientGrant>> {
                let rows = sqlx::query(LIST_CLIENT_GRANTS_SQL)
                    .bind(user_id)
                    .bind(tenant_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list OAuth client grants: {e}"))
                    })?;

                rows.iter().map(client_grant_from_row).collect()
            }

            async fn revoke_client_grant(
                &self,
                id: &str,
                user_id: &str,
                tenant_id: &str,
                actor: GrantActor,
            ) -> AppResult<bool> {
                self.revoke_grant_rows(id, user_id, tenant_id, actor).await
            }

            async fn replace_client_grant(
                &self,
                live_id: &str,
                grant: &OAuthClientGrant,
                approval: &GrantApproval,
            ) -> AppResult<String> {
                self.replace_grant_rows(live_id, grant, approval).await
            }

            async fn store_pushed_request(&self, request: &OAuth2PushedRequest) -> AppResult<()> {
                self.store_pushed_row(request).await
            }

            async fn find_pushed_request(
                &self,
                request_uri_hash: &str,
                client_id: &str,
                now: Timestamp,
            ) -> AppResult<Option<OAuth2PushedRequest>> {
                self.find_pushed_row(request_uri_hash, client_id, now).await
            }

            async fn extend_pushed_request(
                &self,
                request_uri_hash: &str,
                client_id: &str,
                expires_at: Timestamp,
            ) -> AppResult<()> {
                self.extend_pushed_row(request_uri_hash, client_id, expires_at)
                    .await
            }

            async fn consume_pushed_request(
                &self,
                request_uri_hash: &str,
                client_id: &str,
                now: Timestamp,
            ) -> AppResult<bool> {
                self.consume_pushed_row(request_uri_hash, client_id, now)
                    .await
            }

            async fn refresh_families_without_grant(
                &self,
                now: Timestamp,
            ) -> AppResult<Vec<$crate::repositories::UngrantedRefreshFamilies>> {
                self.ungranted_family_rows(now).await
            }

            async fn client_grant_standing(
                &self,
                user_id: &str,
                tenant_id: &str,
                client_id: &str,
            ) -> AppResult<ClientGrantStanding> {
                let rows = sqlx::query(LIST_CLIENT_GRANT_HISTORY_SQL)
                    .bind(user_id)
                    .bind(tenant_id)
                    .bind(client_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read OAuth client grants: {e}"))
                    })?;
                let grants = rows
                    .iter()
                    .map(client_grant_from_row)
                    .collect::<AppResult<Vec<_>>>()?;
                Ok(ClientGrantStanding::of_grants(&grants, Timestamp::now()))
            }

            async fn withdraw_client_access(
                &self,
                user_id: &str,
                tenant_id: &str,
                client_id: &str,
                scope: &str,
            ) -> AppResult<()> {
                let withdraw_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to withdraw OAuth client access: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(withdraw_error)?;

                Self::withdraw_grants_in(&mut tx, user_id, tenant_id, client_id, scope).await?;

                sqlx::query(REVOKE_CLIENT_OAUTH2_REFRESH_TOKENS_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(client_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(withdraw_error)?;
                sqlx::query(REVOKE_CLIENT_OAUTH2_AUTH_CODES_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(client_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(withdraw_error)?;
                let families =
                    $crate::repositories::support_binding::RevokedCredential::ClientFamilies {
                        tenant_id,
                        user_id,
                        client_id,
                    };
                Self::end_sessions_of_revoked(&mut tx, families).await?;

                tx.commit().await.map_err(withdraw_error)
            }

            async fn revoke_refresh_tokens_of_withdrawn_grant(
                &self,
                user_id: &str,
                tenant_id: &str,
                client_id: &str,
            ) -> AppResult<bool> {
                let revoke_error = |e: sqlx::Error| {
                    AppError::database(format!(
                        "Failed to revoke the refresh tokens of a withdrawn grant: {e}"
                    ))
                };
                let mut tx = self.pool().begin().await.map_err(revoke_error)?;
                let result = sqlx::query(REVOKE_WITHDRAWN_OAUTH2_REFRESH_TOKENS_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(client_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(revoke_error)?;
                let revoked = result.rows_affected() > 0;
                if revoked {
                    let families =
                        $crate::repositories::support_binding::RevokedCredential::ClientFamilies {
                            tenant_id,
                            user_id,
                            client_id,
                        };
                    Self::end_sessions_of_revoked(&mut tx, families).await?;
                }
                tx.commit().await.map_err(revoke_error)?;
                Ok(revoked)
            }

            async fn create_device_authorization(&self, da: &DeviceAuthorization) -> AppResult<()> {
                // Housekeeping first, as of the moment this login starts:
                // the rows no poll will ever consume.
                sqlx::query(SWEEP_EXPIRED_DEVICE_AUTHORIZATIONS_SQL)
                    .bind(da.created_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to sweep expired device authorizations: {e}"
                        ))
                    })?;
                sqlx::query(CREATE_DEVICE_AUTHORIZATION_SQL)
                    .bind(&da.device_code_hash)
                    .bind(&da.user_code)
                    .bind(&da.status)
                    .bind(&da.approved_by)
                    .bind(da.created_at)
                    .bind(da.expires_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store device authorization: {e}"))
                    })?;
                Ok(())
            }

            async fn get_device_authorization_by_code_hash(
                &self,
                device_code_hash: &str,
            ) -> AppResult<Option<DeviceAuthorization>> {
                let row = sqlx::query(GET_DEVICE_AUTHORIZATION_BY_CODE_HASH_SQL)
                    .bind(device_code_hash)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to load device authorization: {e}"))
                    })?;
                row.map(|row| device_authorization_from_row(&row))
                    .transpose()
            }

            async fn get_device_authorization_by_user_code(
                &self,
                user_code: &str,
            ) -> AppResult<Option<DeviceAuthorization>> {
                let row = sqlx::query(GET_DEVICE_AUTHORIZATION_BY_USER_CODE_SQL)
                    .bind(user_code)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to load device authorization by user_code: {e}"
                        ))
                    })?;
                row.map(|row| device_authorization_from_row(&row))
                    .transpose()
            }

            async fn approve_device_authorization(
                &self,
                user_code: &str,
                approved_by: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(APPROVE_DEVICE_AUTHORIZATION_SQL)
                    .bind(user_code)
                    .bind(approved_by)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to approve device authorization: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn deny_device_authorization(&self, user_code: &str) -> AppResult<bool> {
                let result = sqlx::query(DENY_DEVICE_AUTHORIZATION_SQL)
                    .bind(user_code)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to deny device authorization: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn delete_device_authorization(&self, device_code_hash: &str) -> AppResult<bool> {
                let result = sqlx::query(DELETE_DEVICE_AUTHORIZATION_SQL)
                    .bind(device_code_hash)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete device authorization: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_oauth2_server_repository;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// The table every grant receipt is written beside.
    const GRANTS_TABLE: &str = "oauth_client_grants";

    /// The statements that start, change or end a grant row.
    const WRITE_VERBS: [&str; 3] = ["insert into", "update", "delete from"];

    /// Every `.rs` file under `dir`, recursively.
    fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("the crate's source directory reads") {
            let path = entry.expect("a directory entry reads").path();
            if path.is_dir() {
                rust_files(&path, into);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                into.push(path);
            }
        }
    }

    /// `text` lowercased with every run of whitespace one space, so a
    /// statement split over lines reads as one.
    fn collapsed(text: &str) -> String {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }

    /// The SQL write forms on `oauth_client_grants` that `text` holds.
    fn grant_writes_in(text: &str) -> Vec<String> {
        let text = collapsed(text);
        WRITE_VERBS
            .iter()
            .map(|verb| format!("{verb} {GRANTS_TABLE}"))
            .filter(|needle| text.contains(needle.as_str()))
            .collect()
    }

    /// Every SQL text that inserts into, updates or deletes from
    /// `oauth_client_grants` is defined in `tokens/grant_writes.rs`, where
    /// each primitive running one writes the grant's receipt in its
    /// transaction (carnet#1544). That each primitive does write its event
    /// is held by the integration suite `oauth_grant_receipts_test`, one test
    /// per caller path; this holds the location only.
    ///
    /// The account delete's table list (`user_references.rs`) builds its
    /// statements with `concat!` from the bare table name, so it holds no
    /// such text and needs no exemption: the receipts go with the account
    /// in that same purge.
    #[test]
    fn every_grant_write_is_defined_beside_its_receipt() {
        // A scan that enforces a rule across the crate's source, which no
        // lint can express: the files are the data under test.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let home = src.join("repositories/tokens/grant_writes.rs");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        assert!(
            files.len() > 100,
            "bug: the scan found {} source files, not the crate's",
            files.len()
        );
        let home_text = fs::read_to_string(&home).expect("grant_writes.rs reads");
        assert_eq!(
            grant_writes_in(&home_text).len(),
            WRITE_VERBS.len(),
            "bug: grant_writes.rs does not hold every grant write, so the scan checks nothing"
        );
        let strays: Vec<String> = files
            .iter()
            .filter(|path| **path != home)
            .filter_map(|path| {
                let text = fs::read_to_string(path).expect("a source file reads");
                let writes = grant_writes_in(&text);
                (!writes.is_empty()).then(|| format!("{}: {writes:?}", path.display()))
            })
            .collect();
        assert!(
            strays.is_empty(),
            "bug: a statement writes {GRANTS_TABLE} outside tokens/grant_writes.rs, so it writes \
             no receipt: {strays:?}"
        );
    }
}
