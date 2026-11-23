// ABOUTME: Revokes and replaces OAuth consent grants for both backends, the one revoke body both writes share
// ABOUTME: A replacement revokes the live row and inserts the new one in a single transaction (carnet#1544)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Writes that end a consent grant.
//!
//! [`revoke_client_grant`](crate::repositories::OAuth2ServerRepository::revoke_client_grant)
//! and
//! [`replace_client_grant`](crate::repositories::OAuth2ServerRepository::replace_client_grant)
//! end a grant the same way: the row is revoked, and with it the client's
//! refresh tokens and unredeemed codes for the user in the tenant, and the
//! support sessions bound to those families (carnet#1511). The body is written
//! once here, in the caller's transaction, so the two cannot drift.
//!
//! An approval leaves the client one live grant: storing or replacing a grant
//! revokes, in its own transaction, every other live grant of the same user,
//! tenant and client, whatever its scope, so narrowing to a preset never
//! leaves an older plain grant standing beside it.

/// Emit the grant-ending helpers for one backend type, `$row` being its row
/// type (the transaction's connection is named through it).
macro_rules! impl_client_grant_writes {
    ($ty:ty, $row:ty) => {
        impl $ty {
            /// Revoke grant `id`, owned by `user_id` in `tenant_id`, in
            /// `conn`'s transaction, with the client's refresh tokens, codes
            /// and bound support sessions. Returns the grant's client, `None`
            /// when no live grant matched (nothing else is touched then).
            async fn revoke_grant_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                id: &str,
                user_id: &str,
                tenant_id: &str,
            ) -> AppResult<Option<String>> {
                use $crate::repositories::tokens::{
                    REVOKE_CLIENT_GRANT_SQL, REVOKE_CLIENT_OAUTH2_AUTH_CODES_SQL,
                    REVOKE_CLIENT_OAUTH2_REFRESH_TOKENS_SQL,
                };
                let revoke_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to revoke OAuth client grant: {e}"))
                };
                let revoked = sqlx::query_scalar::<_, String>(REVOKE_CLIENT_GRANT_SQL)
                    .bind(id)
                    .bind(user_id)
                    .bind(tenant_id)
                    .fetch_optional(&mut *conn)
                    .await
                    .map_err(revoke_error)?;
                let Some(client_id) = revoked else {
                    return Ok(None);
                };

                // The grant is the consent the client's refresh tokens ride
                // on: revoking it without them would leave the client
                // refreshing indefinitely.
                sqlx::query(REVOKE_CLIENT_OAUTH2_REFRESH_TOKENS_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(&client_id)
                    .execute(&mut *conn)
                    .await
                    .map_err(revoke_error)?;
                sqlx::query(REVOKE_CLIENT_OAUTH2_AUTH_CODES_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(&client_id)
                    .execute(&mut *conn)
                    .await
                    .map_err(revoke_error)?;
                let families =
                    $crate::repositories::support_binding::RevokedCredential::ClientFamilies {
                        tenant_id,
                        user_id,
                        client_id: &client_id,
                    };
                Self::end_sessions_of_revoked(conn, families).await?;
                Ok(Some(client_id))
            }

            /// Revoke the refresh tokens the client's refresh may have written
            /// after the transaction's snapshot. A refresh checks the grant
            /// after writing its successor, and this runs once the revocation
            /// is visible, so one of the two always sees the other.
            async fn revoke_late_refresh_tokens(
                &self,
                user_id: &str,
                tenant_id: &str,
                client_id: &str,
            ) -> AppResult<()> {
                sqlx::query($crate::repositories::tokens::REVOKE_CLIENT_OAUTH2_REFRESH_TOKENS_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(client_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to revoke OAuth client grant: {e}"))
                    })?;
                Ok(())
            }

            /// Revoke, in `conn`'s transaction, every live grant the user
            /// gave `client_id` in `tenant_id` other than `keep_id`, each as
            /// [`Self::revoke_grant_in`] revokes one. Returns whether any was.
            async fn revoke_other_grants_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                keep_id: &str,
                grant: &OAuthClientGrant,
            ) -> AppResult<bool> {
                let others = sqlx::query_scalar::<_, String>(
                    $crate::repositories::tokens::LIST_OTHER_LIVE_CLIENT_GRANTS_SQL,
                )
                .bind(&grant.user_id)
                .bind(&grant.tenant_id)
                .bind(&grant.client_id)
                .bind(keep_id)
                .fetch_all(&mut *conn)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to list OAuth client grants: {e}"))
                })?;
                let mut ended = false;
                for other in &others {
                    ended |= Self::revoke_grant_in(conn, other, &grant.user_id, &grant.tenant_id)
                        .await?
                        .is_some();
                }
                Ok(ended)
            }

            /// `store_client_grant`: insert `grant`, or record a re-consent on
            /// the live row of its scope, and revoke the client's other live
            /// grants, in one transaction.
            async fn store_grant_rows(&self, grant: &OAuthClientGrant) -> AppResult<String> {
                let store_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to store OAuth client grant: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(store_error)?;
                let stored = Self::insert_grant_in(&mut tx, grant).await?;
                let ended = Self::revoke_other_grants_in(&mut tx, &stored, grant).await?;
                tx.commit().await.map_err(store_error)?;
                if ended {
                    self.revoke_late_refresh_tokens(
                        &grant.user_id,
                        &grant.tenant_id,
                        &grant.client_id,
                    )
                    .await?;
                }
                Ok(stored)
            }

            /// Insert `grant`, or on its scope's live row record the re-consent,
            /// in `conn`'s transaction: the id of the row holding it.
            async fn insert_grant_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                grant: &OAuthClientGrant,
            ) -> AppResult<String> {
                sqlx::query_scalar::<_, String>(
                    $crate::repositories::tokens::STORE_CLIENT_GRANT_SQL,
                )
                .bind(&grant.id)
                .bind(&grant.user_id)
                .bind(&grant.tenant_id)
                .bind(&grant.client_id)
                .bind(&grant.scope)
                .bind(&grant.consent_version)
                .bind(&grant.consent_locale)
                .bind(&grant.authorization_details)
                .bind(grant.expires_at)
                .fetch_one(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("Failed to store OAuth client grant: {e}")))
            }

            /// `revoke_client_grant`: one grant and what rides on it.
            async fn revoke_grant_rows(
                &self,
                id: &str,
                user_id: &str,
                tenant_id: &str,
            ) -> AppResult<bool> {
                let mut tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to revoke OAuth client grant: {e}"))
                })?;
                let Some(client_id) =
                    Self::revoke_grant_in(&mut tx, id, user_id, tenant_id).await?
                else {
                    return Ok(false);
                };
                tx.commit().await.map_err(|e| {
                    AppError::database(format!("Failed to revoke OAuth client grant: {e}"))
                })?;
                self.revoke_late_refresh_tokens(user_id, tenant_id, &client_id)
                    .await?;
                Ok(true)
            }

            /// `replace_client_grant`: revoke `live_id` and insert `grant` in
            /// one transaction, with the client's other live grants. The
            /// revoke comes first, so the active partial unique index admits
            /// the new row. A live row already revoked
            /// meanwhile leaves nothing to end, and the new grant is still
            /// stored: the athlete just approved it.
            async fn replace_grant_rows(
                &self,
                live_id: &str,
                grant: &OAuthClientGrant,
            ) -> AppResult<String> {
                let replace_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to replace OAuth client grant: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(replace_error)?;
                let ended =
                    Self::revoke_grant_in(&mut tx, live_id, &grant.user_id, &grant.tenant_id)
                        .await?
                        .is_some();
                let stored = Self::insert_grant_in(&mut tx, grant).await?;
                let others = Self::revoke_other_grants_in(&mut tx, &stored, grant).await?;
                tx.commit().await.map_err(replace_error)?;
                if ended || others {
                    self.revoke_late_refresh_tokens(
                        &grant.user_id,
                        &grant.tenant_id,
                        &grant.client_id,
                    )
                    .await?;
                }
                Ok(stored)
            }
        }
    };
}

pub(crate) use impl_client_grant_writes;
