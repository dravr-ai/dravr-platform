// ABOUTME: Repository trait, statements and shared body for an athlete's standing policies (oauth_standing_policies)
// ABOUTME: Store-or-replace, list and remove one athlete's policies; a deleted client's go with it; emitted per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An athlete's standing policies (carnet#1544, design row 35).
//!
//! "Always allow this app this preset": the authorization server reads the
//! athlete's live policies for a client where it would skip the consent
//! screen, and renews a preset grant when one allows the request. Every
//! statement the athlete's own requests reach filters `tenant_id` and
//! `user_id`. The name is unrelated to `StandingGrant` and
//! `ClientGrantStanding`, which say whether a grant still stands.
//!
//! A policy is immutable: storing one with the box ticked again revokes the
//! live policy of the same client and preset and inserts the new one, in one
//! transaction, so the active partial unique index never sees two.
//!
//! Deleting a client revokes its policies with its grants
//! ([`REVOKE_CLIENT_STANDING_POLICIES_SQL`]), and so does the orphan sweep
//! ([`REVOKE_ORPHANED_STANDING_POLICIES_SQL`]), so a re-registered client is
//! never trusted on an old policy. Both are system statements across tenants,
//! as the grant hard deletes beside them are.

use std::fmt::Display;

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuthStandingPolicy;
use sqlx::Row;

/// An athlete's standing policies.
#[async_trait]
pub trait OAuthStandingPolicyRepository: Send + Sync {
    /// Store `policy`, revoking the live policy of the same athlete, client
    /// and preset in the same transaction.
    async fn store_standing_policy(&self, policy: &OAuthStandingPolicy) -> AppResult<()>;

    /// The live policies of `user_id` in `tenant_id`, newest first: of
    /// `client_id` only when one is named.
    async fn list_standing_policies(
        &self,
        tenant_id: &str,
        user_id: &str,
        client_id: Option<&str>,
    ) -> AppResult<Vec<OAuthStandingPolicy>>;

    /// Remove live policy `id` of `user_id` in `tenant_id`. Returns whether
    /// one was removed.
    async fn revoke_standing_policy(
        &self,
        tenant_id: &str,
        user_id: &str,
        id: &str,
    ) -> AppResult<bool>;
}

/// Revoke the live policy of one athlete, client and preset.
pub(crate) const REVOKE_LIVE_STANDING_POLICY_SQL: &str = r"
            UPDATE oauth_standing_policies SET revoked_at = $5
            WHERE tenant_id = $1 AND user_id = $2 AND client_id = $3 AND preset = $4
              AND revoked_at IS NULL
            ";

/// Insert a policy.
pub(crate) const INSERT_STANDING_POLICY_SQL: &str = r"
            INSERT INTO oauth_standing_policies
                (id, tenant_id, user_id, client_id, preset, scope, authorization_details,
                 consent_version, consent_locale, created_at, revoked_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NULL)
            ";

/// One athlete's live policies, newest first.
pub(crate) const LIST_STANDING_POLICIES_SQL: &str = r"
            SELECT id, tenant_id, user_id, client_id, preset, scope, authorization_details,
                   consent_version, consent_locale, created_at, revoked_at
            FROM oauth_standing_policies
            WHERE tenant_id = $1 AND user_id = $2 AND revoked_at IS NULL
            ORDER BY created_at DESC, id DESC
            ";

/// One athlete's live policies for one client, newest first.
pub(crate) const LIST_CLIENT_STANDING_POLICIES_SQL: &str = r"
            SELECT id, tenant_id, user_id, client_id, preset, scope, authorization_details,
                   consent_version, consent_locale, created_at, revoked_at
            FROM oauth_standing_policies
            WHERE tenant_id = $1 AND user_id = $2 AND client_id = $3 AND revoked_at IS NULL
            ORDER BY created_at DESC, id DESC
            ";

/// Remove one live policy; `tenant_id` + `user_id` in the filter is the
/// ownership check.
pub(crate) const REVOKE_STANDING_POLICY_SQL: &str = r"
            UPDATE oauth_standing_policies SET revoked_at = $4
            WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND revoked_at IS NULL
            ";

/// Revoke every live policy naming client `$1`, in every tenant: the client
/// is being deleted. A system statement, as the grant delete beside it.
pub(crate) const REVOKE_CLIENT_STANDING_POLICIES_SQL: &str = r"
            UPDATE oauth_standing_policies SET revoked_at = CURRENT_TIMESTAMP
            WHERE client_id = $1 AND revoked_at IS NULL
            ";

/// Revoke every live policy naming a client that no longer exists, in every
/// tenant: the orphan sweep's, beside its grant delete.
pub(crate) const REVOKE_ORPHANED_STANDING_POLICIES_SQL: &str = r"
            UPDATE oauth_standing_policies SET revoked_at = CURRENT_TIMESTAMP
            WHERE revoked_at IS NULL
              AND NOT EXISTS (
                SELECT 1 FROM oauth2_clients c
                WHERE c.client_id = oauth_standing_policies.client_id
            )
            ";

/// The error for a column of this table that would not decode.
fn policy_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!(
        "Failed to read oauth_standing_policies.{name}: {e}"
    ))
}

/// Decode one `oauth_standing_policies` row.
pub(crate) fn standing_policy_from_row<R>(row: &R) -> AppResult<OAuthStandingPolicy>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Timestamp: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<Timestamp>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let text = |name: &str| -> AppResult<String> {
        row.try_get(name).map_err(|e| policy_column_error(name, e))
    };
    Ok(OAuthStandingPolicy {
        id: text("id")?,
        tenant_id: text("tenant_id")?,
        user_id: text("user_id")?,
        client_id: text("client_id")?,
        preset: text("preset")?,
        scope: text("scope")?,
        authorization_details: text("authorization_details")?,
        consent_version: text("consent_version")?,
        consent_locale: text("consent_locale")?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| policy_column_error("created_at", e))?,
        revoked_at: row
            .try_get("revoked_at")
            .map_err(|e| policy_column_error("revoked_at", e))?,
    })
}

/// Emit the whole [`OAuthStandingPolicyRepository`] implementation for one
/// backend type; sqlx resolves the driver from `self.pool()`.
macro_rules! impl_standing_policy_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl OAuthStandingPolicyRepository for $ty {
            async fn store_standing_policy(&self, policy: &OAuthStandingPolicy) -> AppResult<()> {
                let store_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to store a standing policy: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(store_error)?;
                sqlx::query(REVOKE_LIVE_STANDING_POLICY_SQL)
                    .bind(&policy.tenant_id)
                    .bind(&policy.user_id)
                    .bind(&policy.client_id)
                    .bind(&policy.preset)
                    .bind(policy.created_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;
                sqlx::query(INSERT_STANDING_POLICY_SQL)
                    .bind(&policy.id)
                    .bind(&policy.tenant_id)
                    .bind(&policy.user_id)
                    .bind(&policy.client_id)
                    .bind(&policy.preset)
                    .bind(&policy.scope)
                    .bind(&policy.authorization_details)
                    .bind(&policy.consent_version)
                    .bind(&policy.consent_locale)
                    .bind(policy.created_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?;
                tx.commit().await.map_err(store_error)
            }

            async fn list_standing_policies(
                &self,
                tenant_id: &str,
                user_id: &str,
                client_id: Option<&str>,
            ) -> AppResult<Vec<OAuthStandingPolicy>> {
                let query = match client_id {
                    Some(client_id) => sqlx::query(LIST_CLIENT_STANDING_POLICIES_SQL)
                        .bind(tenant_id)
                        .bind(user_id)
                        .bind(client_id),
                    None => sqlx::query(LIST_STANDING_POLICIES_SQL)
                        .bind(tenant_id)
                        .bind(user_id),
                };
                let rows = query.fetch_all(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to list standing policies: {e}"))
                })?;
                rows.iter().map(standing_policy_from_row).collect()
            }

            async fn revoke_standing_policy(
                &self,
                tenant_id: &str,
                user_id: &str,
                id: &str,
            ) -> AppResult<bool> {
                let revoked = sqlx::query(REVOKE_STANDING_POLICY_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(id)
                    .bind(Timestamp::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to remove a standing policy: {e}"))
                    })?;
                Ok(revoked.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_standing_policy_repository;
