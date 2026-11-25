// ABOUTME: Every statement that starts or ends an OAuth consent grant, and the receipt each one writes in its transaction
// ABOUTME: Store, replace, revoke, withdraw, hard-delete, expire and baseline grants for both backends (carnet#1544)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Writes that start or end a consent grant, and its receipts.
//!
//! Every SQL text that inserts into, updates or deletes from
//! `oauth_client_grants` is defined here (the completeness test in
//! `tokens.rs` holds that), and each primitive that runs one also writes
//! the grant's event to `oauth_grant_events` in the same transaction,
//! through [`record_grant_event_in`](#method.record_grant_event_in): an
//! `issue` for a grant stored or re-consented to, one terminal event for a
//! grant ended. A grant ended after its expiry ends as `expire`, at its
//! `expires_at`, by the AS, whoever ends it
//! ([`GrantEvent::ending`](pierre_core::models::GrantEvent::ending)).
//!
//! An event's `at` is the row's own time where the statement stamps one
//! (`granted_at` on insert, `revoked_at` on revoke, `expires_at` on expire),
//! read back by `RETURNING`; a write that stamps none (an in-place
//! re-consent, a hard delete) takes the database's `CURRENT_TIMESTAMP` in
//! the same statement. One clock.
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
//!
//! The hard deletes, the expire pass and the baseline select across
//! tenants: they are system sweeps. Every event they write is keyed by the
//! `(tenant_id, user_id, id)` of the row they read.

/// Record a consent. The active partial-unique index keeps one live grant:
/// the conflict target names that index, which both engines resolve. A
/// re-consent to the same grant keeps its row and records the consent text
/// it was given under (and the standing policy that renewed it, if any), so
/// a screen asked again because its text changed is not asked again on the
/// next authorization. The details and expiry are written with the row and
/// never by the re-consent: a grant's details do not change under it.
/// Returns the stored row, the existing one on a re-consent, and the
/// statement's time (`event_at`), the re-consent's.
pub(crate) const STORE_CLIENT_GRANT_SQL: &str = r"
            INSERT INTO oauth_client_grants
                (id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                 consent_version, consent_locale, authorization_details, expires_at,
                 standing_policy_id)
            VALUES ($1, $2, $3, $4, $5, CURRENT_TIMESTAMP, NULL, $6, $7, $8, $9, $10)
            ON CONFLICT (user_id, tenant_id, client_id, scope)
                WHERE revoked_at IS NULL AND parent_grant_id IS NULL
            DO UPDATE SET consent_version = excluded.consent_version,
                          consent_locale = excluded.consent_locale,
                          standing_policy_id = excluded.standing_policy_id
            RETURNING id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                      consent_version, consent_locale, authorization_details, expires_at,
                      CURRENT_TIMESTAMP AS event_at
            ";

/// Soft-delete a grant; `user_id` + `tenant_id` in the filter is the
/// ownership check. Returns the revoked row, so the same transaction can
/// retire its client's refresh tokens and write its receipt.
pub(crate) const REVOKE_CLIENT_GRANT_SQL: &str = r"
            UPDATE oauth_client_grants
            SET revoked_at = CURRENT_TIMESTAMP
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3 AND revoked_at IS NULL
            RETURNING id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                      consent_version, consent_locale, authorization_details, expires_at
            ";

/// Soft-delete every active grant a user gave one client in one tenant;
/// returns the rows revoked.
pub(crate) const WITHDRAW_CLIENT_GRANTS_SQL: &str = r"
            UPDATE oauth_client_grants
            SET revoked_at = CURRENT_TIMESTAMP
            WHERE user_id = $1 AND tenant_id = $2 AND client_id = $3 AND revoked_at IS NULL
            RETURNING id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                      consent_version, consent_locale, authorization_details, expires_at
            ";

/// Record a client's access as withdrawn when it has no grant row to
/// revoke: a row born revoked, returned.
pub(crate) const STORE_WITHDRAWN_CLIENT_GRANT_SQL: &str = r"
            INSERT INTO oauth_client_grants
                (id, user_id, tenant_id, client_id, scope, granted_at, revoked_at)
            VALUES ($1, $2, $3, $4, $5, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            RETURNING id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                      consent_version, consent_locale, authorization_details, expires_at
            ";

/// The live grants naming client `$1`, in every tenant, with the
/// statement's time (`event_at`): what deleting the client ends.
pub(crate) const LIST_LIVE_CLIENT_GRANTS_FOR_CLIENT_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at,
                   CURRENT_TIMESTAMP AS event_at
            FROM oauth_client_grants
            WHERE client_id = $1 AND revoked_at IS NULL
            ";

/// The consent grants naming client `$1`, which has no foreign key to cascade
/// them.
pub(crate) const DELETE_CLIENT_GRANTS_FOR_CLIENT_SQL: &str = r"
            DELETE FROM oauth_client_grants WHERE client_id = $1
            ";

/// The live grants naming a client that no longer exists, with the
/// statement's time (`event_at`): what the orphan delete ends.
pub(crate) const LIST_LIVE_ORPHANED_CLIENT_GRANTS_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at,
                   CURRENT_TIMESTAMP AS event_at
            FROM oauth_client_grants
            WHERE revoked_at IS NULL
              AND NOT EXISTS (
                SELECT 1 FROM oauth2_clients c WHERE c.client_id = oauth_client_grants.client_id
            )
            ";

/// Consent grants naming a client that no longer exists. `oauth_client_grants`
/// has no foreign key to `oauth2_clients`, so deleting a client cascades to its
/// codes, refresh tokens and states but not to these.
pub(crate) const DELETE_ORPHANED_CLIENT_GRANTS_SQL: &str = r"
            DELETE FROM oauth_client_grants
            WHERE NOT EXISTS (
                SELECT 1 FROM oauth2_clients c WHERE c.client_id = oauth_client_grants.client_id
            )
            ";

/// Grants still un-revoked whose expiry is at or before `$1` and that have
/// no terminal event yet, in every tenant: what the expire pass ends.
pub(crate) const LIST_EXPIRED_UNENDED_CLIENT_GRANTS_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at
            FROM oauth_client_grants g
            WHERE g.revoked_at IS NULL
              AND g.expires_at IS NOT NULL AND g.expires_at <= $1
              AND NOT EXISTS (
                SELECT 1 FROM oauth_grant_events e
                WHERE e.tenant_id = g.tenant_id AND e.user_id = g.user_id
                  AND e.client_grant_id = g.id AND e.event IN ('revoke', 'expire')
            )
            ";

/// Un-revoked grants, live or past their expiry, with no `issue` event,
/// oldest first, at most `$1`: the grants given before receipts were, which
/// the baseline gives an `issue`. A grant the expire pass already ended
/// before the baseline reached it holds an `expire` and no `issue`, and is
/// still given its `issue`; once it has one it is never listed again.
pub(crate) const LIST_UNRECEIPTED_LIVE_CLIENT_GRANTS_SQL: &str = r"
            SELECT id, user_id, tenant_id, client_id, scope, granted_at, revoked_at,
                   consent_version, consent_locale, authorization_details, expires_at
            FROM oauth_client_grants g
            WHERE g.revoked_at IS NULL
              AND NOT EXISTS (
                SELECT 1 FROM oauth_grant_events e
                WHERE e.tenant_id = g.tenant_id AND e.user_id = g.user_id
                  AND e.client_grant_id = g.id AND e.event = 'issue'
            )
            ORDER BY g.granted_at, g.id
            LIMIT $1
            ";

/// Write one grant event. A second terminal event for the same grant
/// conflicts with the partial unique index and writes nothing.
pub(crate) const INSERT_GRANT_EVENT_SQL: &str = r"
            INSERT INTO oauth_grant_events
                (id, tenant_id, user_id, client_grant_id, event, actor, at,
                 payload, payload_sha256, jws, logged_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, NULL)
            ON CONFLICT DO NOTHING
            ";

/// The latest `issue` event of grant `$3`, held by `$2` in `$1`.
pub(crate) const LATEST_ISSUE_RECEIPT_SQL: &str = r"
            SELECT id FROM oauth_grant_events
            WHERE tenant_id = $1 AND user_id = $2 AND client_grant_id = $3 AND event = 'issue'
            ORDER BY at DESC, id DESC
            LIMIT 1
            ";

/// Emit the grant-writing helpers for one backend type, `$row` being its row
/// type (the transaction's connection is named through it).
macro_rules! impl_client_grant_writes {
    ($ty:ty, $row:ty) => {
        impl $ty {
            /// Write `event` of `grant` at `at`, caused as `approval` says,
            /// in `conn`'s transaction: its payload is built and hashed now.
            /// A terminal event names the grant's latest `issue` receipt.
            /// Returns whether a row was written: a grant already ended
            /// writes no second terminal event.
            pub(crate) async fn record_grant_event_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                grant: &OAuthClientGrant,
                event: GrantEvent,
                approval: ReceiptApproval<'_>,
                at: Timestamp,
            ) -> AppResult<bool> {
                use $crate::repositories::tokens::grant_writes::{
                    INSERT_GRANT_EVENT_SQL, LATEST_ISSUE_RECEIPT_SQL,
                };
                let event_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to record an OAuth grant event: {e}"))
                };
                let issue_receipt_id = if event.is_terminal() {
                    sqlx::query_scalar::<_, String>(LATEST_ISSUE_RECEIPT_SQL)
                        .bind(&grant.tenant_id)
                        .bind(&grant.user_id)
                        .bind(&grant.id)
                        .fetch_optional(&mut *conn)
                        .await
                        .map_err(event_error)?
                } else {
                    None
                };
                let receipt_id = Uuid::new_v4().to_string();
                let (payload, payload_sha256) = GrantReceipt {
                    receipt_id: &receipt_id,
                    event,
                    at,
                    grant,
                    approval,
                    issue_receipt_id: issue_receipt_id.as_deref(),
                }
                .payload()?;
                let written = sqlx::query(INSERT_GRANT_EVENT_SQL)
                    .bind(&receipt_id)
                    .bind(&grant.tenant_id)
                    .bind(&grant.user_id)
                    .bind(&grant.id)
                    .bind(event.as_str())
                    .bind(approval.actor.as_str())
                    .bind(at)
                    .bind(&payload)
                    .bind(&payload_sha256)
                    .execute(&mut *conn)
                    .await
                    .map_err(event_error)?;
                Ok(written.rows_affected() > 0)
            }

            /// Write the event that ends `grant`, ended by `actor` at
            /// `ended_at`: a `revoke`, or an `expire` at its expiry for a
            /// grant already past it.
            async fn end_grant_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                grant: &OAuthClientGrant,
                actor: GrantActor,
                ended_at: Timestamp,
            ) -> AppResult<bool> {
                let (event, actor, at) =
                    GrantEvent::ending(grant, actor, ended_at);
                Self::record_grant_event_in(
                    conn,
                    grant,
                    event,
                    ReceiptApproval::by(actor),
                    at,
                )
                .await
            }

            /// Revoke grant `id`, owned by `user_id` in `tenant_id`, in
            /// `conn`'s transaction, with the client's refresh tokens, codes
            /// and bound support sessions, and write its ending caused by
            /// `actor`. Returns the grant's client, `None` when no live grant
            /// matched (nothing else is touched then).
            async fn revoke_grant_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                id: &str,
                user_id: &str,
                tenant_id: &str,
                actor: GrantActor,
            ) -> AppResult<Option<String>> {
                use $crate::repositories::tokens::grant_writes::REVOKE_CLIENT_GRANT_SQL;
                use $crate::repositories::tokens::{
                    client_grant_from_row, REVOKE_CLIENT_OAUTH2_AUTH_CODES_SQL,
                    REVOKE_CLIENT_OAUTH2_REFRESH_TOKENS_SQL,
                };
                let revoke_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to revoke OAuth client grant: {e}"))
                };
                let revoked = sqlx::query(REVOKE_CLIENT_GRANT_SQL)
                    .bind(id)
                    .bind(user_id)
                    .bind(tenant_id)
                    .fetch_optional(&mut *conn)
                    .await
                    .map_err(revoke_error)?;
                let Some(row) = revoked else {
                    return Ok(None);
                };
                let grant = client_grant_from_row(&row)?;
                let ended_at = grant.revoked_at.unwrap_or(grant.granted_at);
                Self::end_grant_in(conn, &grant, actor, ended_at).await?;
                let client_id = grant.client_id;

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
            /// [`Self::revoke_grant_in`] revokes one, caused by `actor`.
            /// Returns whether any was.
            async fn revoke_other_grants_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                keep_id: &str,
                grant: &OAuthClientGrant,
                actor: GrantActor,
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
                    ended |= Self::revoke_grant_in(
                        conn,
                        other,
                        &grant.user_id,
                        &grant.tenant_id,
                        actor,
                    )
                    .await?
                    .is_some();
                }
                Ok(ended)
            }

            /// `store_client_grant`: insert `grant`, or record a re-consent on
            /// the live row of its scope, and revoke the client's other live
            /// grants, in one transaction.
            async fn store_grant_rows(
                &self,
                grant: &OAuthClientGrant,
                approval: &GrantApproval,
            ) -> AppResult<String> {
                let store_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to store OAuth client grant: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(store_error)?;
                let stored = Self::insert_grant_in(&mut tx, grant, approval).await?;
                let ended =
                    Self::revoke_other_grants_in(&mut tx, &stored, grant, approval.actor).await?;
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
            /// in `conn`'s transaction, and write its `issue` receipt: the id
            /// of the row holding it. The receipt states the stored row, so a
            /// re-consent's states the grant as it stands, not the request.
            async fn insert_grant_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                grant: &OAuthClientGrant,
                approval: &GrantApproval,
            ) -> AppResult<String> {
                use sqlx::Row as _;
                let store_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to store OAuth client grant: {e}"))
                };
                let row = sqlx::query($crate::repositories::tokens::grant_writes::STORE_CLIENT_GRANT_SQL)
                    .bind(&grant.id)
                    .bind(&grant.user_id)
                    .bind(&grant.tenant_id)
                    .bind(&grant.client_id)
                    .bind(&grant.scope)
                    .bind(&grant.consent_version)
                    .bind(&grant.consent_locale)
                    .bind(&grant.authorization_details)
                    .bind(grant.expires_at)
                    .bind(approval.standing_policy.as_ref().map(|policy| policy.id.as_str()))
                    .fetch_one(&mut *conn)
                    .await
                    .map_err(store_error)?;
                let stored = $crate::repositories::tokens::client_grant_from_row(&row)?;
                let at = if stored.id == grant.id {
                    stored.granted_at
                } else {
                    row.try_get::<Timestamp, _>("event_at").map_err(store_error)?
                };
                Self::record_grant_event_in(
                    conn,
                    &stored,
                    GrantEvent::Issue,
                    ReceiptApproval::from_approval(approval),
                    at,
                )
                .await?;
                Ok(stored.id)
            }

            /// `revoke_client_grant`: one grant and what rides on it.
            async fn revoke_grant_rows(
                &self,
                id: &str,
                user_id: &str,
                tenant_id: &str,
                actor: GrantActor,
            ) -> AppResult<bool> {
                let mut tx = self.pool().begin().await.map_err(|e| {
                    AppError::database(format!("Failed to revoke OAuth client grant: {e}"))
                })?;
                let Some(client_id) =
                    Self::revoke_grant_in(&mut tx, id, user_id, tenant_id, actor).await?
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
                approval: &GrantApproval,
            ) -> AppResult<String> {
                let replace_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to replace OAuth client grant: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(replace_error)?;
                let ended = Self::revoke_grant_in(
                    &mut tx,
                    live_id,
                    &grant.user_id,
                    &grant.tenant_id,
                    approval.actor,
                )
                .await?
                .is_some();
                let stored = Self::insert_grant_in(&mut tx, grant, approval).await?;
                let others =
                    Self::revoke_other_grants_in(&mut tx, &stored, grant, approval.actor).await?;
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

            /// `withdraw_client_access`'s grant writes, in `conn`'s
            /// transaction: every live grant the user gave the client in the
            /// tenant is revoked, by the client; when the user never had a
            /// grant row for it, a row born revoked records the withdrawal.
            /// Each row ended gets its receipt.
            async fn withdraw_grants_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                user_id: &str,
                tenant_id: &str,
                client_id: &str,
                scope: &str,
            ) -> AppResult<()> {
                use $crate::repositories::tokens::grant_writes::{
                    STORE_WITHDRAWN_CLIENT_GRANT_SQL, WITHDRAW_CLIENT_GRANTS_SQL,
                };
                use $crate::repositories::tokens::{
                    client_grant_from_row, LIST_CLIENT_GRANT_HISTORY_SQL,
                };
                let withdraw_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to withdraw OAuth client access: {e}"))
                };
                let client = GrantActor::Client;
                let withdrawn = sqlx::query(WITHDRAW_CLIENT_GRANTS_SQL)
                    .bind(user_id)
                    .bind(tenant_id)
                    .bind(client_id)
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(withdraw_error)?;
                for row in &withdrawn {
                    let grant = client_grant_from_row(row)?;
                    let ended_at = grant.revoked_at.unwrap_or(grant.granted_at);
                    Self::end_grant_in(conn, &grant, client, ended_at).await?;
                }
                if !withdrawn.is_empty() {
                    return Ok(());
                }
                let history = sqlx::query(LIST_CLIENT_GRANT_HISTORY_SQL)
                    .bind(user_id)
                    .bind(tenant_id)
                    .bind(client_id)
                    .fetch_optional(&mut *conn)
                    .await
                    .map_err(withdraw_error)?;
                if history.is_some() {
                    return Ok(());
                }
                let row = sqlx::query(STORE_WITHDRAWN_CLIENT_GRANT_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(user_id)
                    .bind(tenant_id)
                    .bind(client_id)
                    .bind(scope)
                    .fetch_one(&mut *conn)
                    .await
                    .map_err(withdraw_error)?;
                let grant = client_grant_from_row(&row)?;
                let ended_at = grant.revoked_at.unwrap_or(grant.granted_at);
                Self::end_grant_in(conn, &grant, client, ended_at).await?;
                Ok(())
            }

            /// Hard-delete every grant naming `client_id`, in every tenant,
            /// in `conn`'s transaction, first writing the ending of each live
            /// one, caused by `actor`: its receipts outlive the row. The
            /// client's standing policies are revoked with them, so a client
            /// registered again under the same id is never trusted on one.
            async fn delete_client_grants_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                client_id: &str,
                actor: GrantActor,
            ) -> AppResult<()> {
                use $crate::repositories::tokens::grant_writes::{
                    DELETE_CLIENT_GRANTS_FOR_CLIENT_SQL, LIST_LIVE_CLIENT_GRANTS_FOR_CLIENT_SQL,
                };
                let delete_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to delete OAuth client grants: {e}"))
                };
                let live = sqlx::query(LIST_LIVE_CLIENT_GRANTS_FOR_CLIENT_SQL)
                    .bind(client_id)
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(delete_error)?;
                Self::end_listed_grants_in(conn, &live, actor).await?;
                sqlx::query(DELETE_CLIENT_GRANTS_FOR_CLIENT_SQL)
                    .bind(client_id)
                    .execute(&mut *conn)
                    .await
                    .map_err(delete_error)?;
                sqlx::query(
                    $crate::repositories::standing_policies::REVOKE_CLIENT_STANDING_POLICIES_SQL,
                )
                .bind(client_id)
                .execute(&mut *conn)
                .await
                .map_err(delete_error)?;
                Ok(())
            }

            /// Hard-delete every grant whose client no longer exists, in
            /// every tenant, in `conn`'s transaction, first writing the
            /// ending of each live one (by the AS), and revoke the standing
            /// policies naming such a client. Returns the grant rows deleted.
            async fn delete_orphaned_grants_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
            ) -> AppResult<u64> {
                use $crate::repositories::tokens::grant_writes::{
                    DELETE_ORPHANED_CLIENT_GRANTS_SQL, LIST_LIVE_ORPHANED_CLIENT_GRANTS_SQL,
                };
                let sweep_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to sweep OAuth2 clients: {e}"))
                };
                let live = sqlx::query(LIST_LIVE_ORPHANED_CLIENT_GRANTS_SQL)
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(sweep_error)?;
                Self::end_listed_grants_in(conn, &live, GrantActor::Auto)
                    .await?;
                let deleted = sqlx::query(DELETE_ORPHANED_CLIENT_GRANTS_SQL)
                    .execute(&mut *conn)
                    .await
                    .map_err(sweep_error)?
                    .rows_affected();
                sqlx::query(
                    $crate::repositories::standing_policies::REVOKE_ORPHANED_STANDING_POLICIES_SQL,
                )
                .execute(&mut *conn)
                .await
                .map_err(sweep_error)?;
                Ok(deleted)
            }

            /// Write the ending of each grant row `rows` holds, each read with
            /// the statement's time as `event_at`, caused by `actor`.
            async fn end_listed_grants_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                rows: &[$row],
                actor: GrantActor,
            ) -> AppResult<()> {
                use sqlx::Row as _;
                for row in rows {
                    let grant = $crate::repositories::tokens::client_grant_from_row(row)?;
                    let ended_at = row.try_get::<Timestamp, _>("event_at").map_err(|e| {
                        AppError::database(format!(
                            "Failed to read oauth_client_grants.event_at: {e}"
                        ))
                    })?;
                    Self::end_grant_in(conn, &grant, actor, ended_at).await?;
                }
                Ok(())
            }

            /// The expire pass, in `conn`'s transaction: every un-revoked
            /// grant whose expiry is at or before `now` and that has not
            /// ended gets its `expire` event (by the AS, at its expiry). The
            /// row itself stays as it is: Connected apps lists it as
            /// expired. Returns the events written.
            async fn expire_grants_in(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                now: Timestamp,
            ) -> AppResult<u64> {
                let expired = sqlx::query(
                    $crate::repositories::tokens::grant_writes::LIST_EXPIRED_UNENDED_CLIENT_GRANTS_SQL,
                )
                .bind(now)
                .fetch_all(&mut *conn)
                .await
                .map_err(|e| AppError::database(format!("Failed to list expired OAuth grants: {e}")))?;
                let mut written = 0_u64;
                for row in &expired {
                    let grant = $crate::repositories::tokens::client_grant_from_row(row)?;
                    if Self::end_grant_in(
                        conn,
                        &grant,
                        GrantActor::Auto,
                        now,
                    )
                    .await?
                    {
                        written += 1;
                    }
                }
                Ok(written)
            }

            /// The baseline: up to `limit` un-revoked grants with no `issue`
            /// event (live, or past their expiry) get one (by the AS, flagged
            /// a baseline, at their `granted_at`), in one transaction. Returns
            /// the receipts written; 0 once every such grant has one.
            pub(crate) async fn baseline_grant_rows(&self, limit: u32) -> AppResult<u64> {
                let baseline_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to baseline OAuth grant receipts: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(baseline_error)?;
                let rows = sqlx::query(
                    $crate::repositories::tokens::grant_writes::LIST_UNRECEIPTED_LIVE_CLIENT_GRANTS_SQL,
                )
                .bind(i64::from(limit))
                .fetch_all(&mut *tx)
                .await
                .map_err(baseline_error)?;
                let mut written = 0_u64;
                for row in &rows {
                    let grant = $crate::repositories::tokens::client_grant_from_row(row)?;
                    let approval = ReceiptApproval {
                        actor: GrantActor::Auto,
                        standing_policy: None,
                        baseline: true,
                    };
                    if Self::record_grant_event_in(
                        &mut tx,
                        &grant,
                        GrantEvent::Issue,
                        approval,
                        grant.granted_at,
                    )
                    .await?
                    {
                        written += 1;
                    }
                }
                tx.commit().await.map_err(baseline_error)?;
                Ok(written)
            }
        }
    };
}

pub(crate) use impl_client_grant_writes;
