// ABOUTME: Ends the support sessions bound to a credential, inside the transaction of the write that revokes it
// ABOUTME: One statement per revocation, the audit row each ended session leaves, and the macro each backend emits

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A support session ends with its credential (carnet#1511, JF Q21)
//!
//! A support session an admin's assistant starts acts as the user only for
//! the credential that started it: an OAuth grant (the refresh-token family
//! its access token was minted in) or one API key. "Revoking the grant ends
//! the session": every write that revokes such a credential ends the
//! sessions bound to it in its own transaction, and leaves one
//! `impersonation.ended` row on the admin audit trail per session.
//!
//! Ended where the revocation is written, not when the next call looks:
//! a revoked API key, or a grant whose consent is withdrawn, no longer
//! authenticates, so no later lookup would ever reach the session. And a
//! lookup could not tell a revoked family from one mid-rotation, whose
//! presented token is consumed before its successor is stored.
//!
//! The revocations, by the repository method that writes them:
//!
//! | Revocation | Method | Sessions ended |
//! |---|---|---|
//! | A refresh family (replay, a grant past its life) | `revoke_refresh_token_family` | that family's |
//! | A consent, at Connected apps or by the client (RFC 7009) | `revoke_client_grant`, `withdraw_client_access`, `revoke_refresh_tokens_of_withdrawn_grant` | every family of that client for the user in the tenant |
//! | An account's sign-ins retired | `revoke_user_refresh_tokens` | every family of the user in the tenant |
//! | An OAuth application deleted | `delete_client`, `delete_stale_clients` | every family left with no token |
//! | An API key deactivated | `deactivate` | that key's |

use pierre_core::permissions::impersonation::{ImpersonationSession, ENDED_ACTION};
use serde_json::json;
use uuid::Uuid;

use super::admin_audit::{AuditOutcome, AuditSource, NewAuditEvent};
use super::impersonation::session_columns;

/// A credential a write revokes, as the sessions bound to it are found.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RevokedCredential<'a> {
    /// The refresh family the stored token (by its HMAC) belongs to.
    FamilyOfToken(&'a str),
    /// Every refresh family a user holds through one client in one tenant.
    ClientFamilies {
        /// The tenant, as `oauth2_refresh_tokens.tenant_id` stores it.
        tenant_id: &'a str,
        /// The user, as `oauth2_refresh_tokens.user_id` stores it.
        user_id: &'a str,
        /// The OAuth application.
        client_id: &'a str,
    },
    /// Every refresh family a user holds in one tenant, through any client.
    UserFamilies {
        /// The tenant, as `oauth2_refresh_tokens.tenant_id` stores it.
        tenant_id: &'a str,
        /// The user, as `oauth2_refresh_tokens.user_id` stores it.
        user_id: &'a str,
    },
    /// Every refresh family no token is left in: its application was deleted.
    DeletedFamilies,
    /// One API key of its owner.
    ApiKey {
        /// The key's id.
        key_id: &'a str,
        /// Its owner, the admin the sessions name.
        user_id: Uuid,
    },
}

/// The `UPDATE` that ends the live sessions bound to a family set: `$1` is
/// now, then the binds of the family subquery.
macro_rules! end_grant_sessions_sql {
    ($families:literal) => {
        concat!(
            "UPDATE impersonation_sessions SET is_active = FALSE, ended_at = $1 \
             WHERE is_active = TRUE AND grant_id IN (",
            $families,
            ") RETURNING ",
            session_columns!()
        )
    };
}

/// The sessions bound to the family of one stored token (`$2`, its HMAC).
pub(crate) const END_FAMILY_OF_TOKEN_SESSIONS_SQL: &str =
    end_grant_sessions_sql!("SELECT family_id FROM oauth2_refresh_tokens WHERE token = $2");

/// The sessions bound to any family of one user (`$3`) through one client
/// (`$4`) in one tenant (`$2`).
pub(crate) const END_CLIENT_FAMILY_SESSIONS_SQL: &str = end_grant_sessions_sql!(
    "SELECT family_id FROM oauth2_refresh_tokens \
     WHERE tenant_id = $2 AND user_id = $3 AND client_id = $4"
);

/// The sessions bound to any family of one user (`$3`) in one tenant (`$2`).
pub(crate) const END_USER_FAMILY_SESSIONS_SQL: &str = end_grant_sessions_sql!(
    "SELECT family_id FROM oauth2_refresh_tokens WHERE tenant_id = $2 AND user_id = $3"
);

/// The sessions bound to a family no token is left in.
///
/// A rotation never leaves a family empty (it revokes the presented token, never deletes
/// it), so only a deletion matches.
pub(crate) const END_DELETED_FAMILY_SESSIONS_SQL: &str = concat!(
    "UPDATE impersonation_sessions SET is_active = FALSE, ended_at = $1 \
     WHERE is_active = TRUE AND grant_id IS NOT NULL AND NOT EXISTS (\
     SELECT 1 FROM oauth2_refresh_tokens t WHERE t.family_id = impersonation_sessions.grant_id\
     ) RETURNING ",
    session_columns!()
);

/// The sessions bound to one API key (`$2`) of its owner (`$3`).
pub(crate) const END_API_KEY_SESSIONS_SQL: &str = concat!(
    "UPDATE impersonation_sessions SET is_active = FALSE, ended_at = $1 \
     WHERE is_active = TRUE AND api_key_id = $2 AND impersonator_id = $3 \
     RETURNING ",
    session_columns!()
);

/// The audit row of a support session through an admin's assistant.
///
/// Its start, end or expiry: it names the admin, the user, the session, the
/// credential it was bound to and the reason given; `transport` is the
/// surface the call came over, `None` when no call ended it (a revocation).
#[must_use]
pub fn support_session_event(
    session: &ImpersonationSession,
    action: &str,
    summary: String,
    transport: Option<&str>,
) -> NewAuditEvent {
    NewAuditEvent {
        action: action.to_owned(),
        summary,
        outcome: AuditOutcome::Success,
        source: AuditSource::ToolCall,
        actor_user_id: Some(session.impersonator_id),
        // The user's tenant, not the admin's: only a view of the whole trail
        // reads it, as every row reaching past the tenant its admin acts in.
        tenant_id: None,
        ip_address: None,
        user_agent: None,
        metadata: json!({
            "target": { "user_id": session.target_user_id },
            "impersonation_session_id": session.id,
            "grant_id": session.grant_id,
            "api_key_id": session.api_key_id,
            "reason": session.reason,
            "transport": transport,
        }),
    }
}

/// The audit row of a session ended by its credential's revocation.
pub(crate) fn revoked_end_event(session: &ImpersonationSession) -> NewAuditEvent {
    support_session_event(
        session,
        ENDED_ACTION,
        "Support session through an assistant ended: the credential it was bound to was revoked"
            .to_owned(),
        None,
    )
}

/// Emit, for one backend, the transaction-scoped step every revoking write
/// runs: `$row` is the driver's row type and `$ids` its
/// [`uuid_columns`](super::uuid_columns) codec, as in
/// `impl_impersonation_repository!`, which emits the `session_from_row` this
/// reads with (so both are invoked in the same backend module).
macro_rules! impl_support_binding_ends {
    ($ty:ty, $row:ty, $ids:ident) => {
        impl $ty {
            /// End, in `conn`'s transaction, the live support sessions bound
            /// to the credential `revoked` names, and write one
            /// `impersonation.ended` audit row for each. Returns the ended
            /// sessions.
            ///
            /// # Errors
            /// Returns a database error when a statement fails; the caller's
            /// transaction is then not committed, so the credential is not
            /// revoked with a session still acting through it.
            pub(crate) async fn end_sessions_of_revoked(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                revoked: $crate::repositories::support_binding::RevokedCredential<'_>,
            ) -> pierre_core::errors::AppResult<
                Vec<pierre_core::permissions::impersonation::ImpersonationSession>,
            > {
                use $crate::repositories::support_binding::{
                    RevokedCredential, END_API_KEY_SESSIONS_SQL, END_CLIENT_FAMILY_SESSIONS_SQL,
                    END_DELETED_FAMILY_SESSIONS_SQL, END_FAMILY_OF_TOKEN_SESSIONS_SQL,
                    END_USER_FAMILY_SESSIONS_SQL,
                };
                let failed = |e: sqlx::Error| {
                    pierre_core::errors::AppError::database(format!(
                        "Failed to end the support sessions of a revoked credential: {e}"
                    ))
                };
                let now = chrono::Utc::now();
                let query = match revoked {
                    RevokedCredential::FamilyOfToken(token_hash) => {
                        sqlx::query(END_FAMILY_OF_TOKEN_SESSIONS_SQL)
                            .bind(now)
                            .bind(token_hash)
                    }
                    RevokedCredential::ClientFamilies {
                        tenant_id,
                        user_id,
                        client_id,
                    } => sqlx::query(END_CLIENT_FAMILY_SESSIONS_SQL)
                        .bind(now)
                        .bind(tenant_id)
                        .bind(user_id)
                        .bind(client_id),
                    RevokedCredential::UserFamilies { tenant_id, user_id } => {
                        sqlx::query(END_USER_FAMILY_SESSIONS_SQL)
                            .bind(now)
                            .bind(tenant_id)
                            .bind(user_id)
                    }
                    RevokedCredential::DeletedFamilies => {
                        sqlx::query(END_DELETED_FAMILY_SESSIONS_SQL).bind(now)
                    }
                    RevokedCredential::ApiKey { key_id, user_id } => {
                        sqlx::query(END_API_KEY_SESSIONS_SQL)
                            .bind(now)
                            .bind(key_id)
                            .bind($ids::bind(user_id))
                    }
                };
                let ended = query
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(failed)?
                    .iter()
                    .map(session_from_row)
                    .collect::<pierre_core::errors::AppResult<Vec<_>>>()?;
                for session in &ended {
                    let event = $crate::repositories::support_binding::revoked_end_event(session);
                    sqlx::query($crate::repositories::admin_audit::INSERT_AUDIT_EVENT_SQL)
                        .bind(uuid::Uuid::new_v4().to_string())
                        .bind(&event.action)
                        .bind($crate::repositories::admin_audit::severity_for(
                            event.outcome,
                        ))
                        .bind(&event.summary)
                        .bind(event.source.as_str())
                        .bind(event.outcome.as_str())
                        .bind(&event.tenant_id)
                        .bind($ids::bind_opt(event.actor_user_id))
                        .bind(&event.ip_address)
                        .bind(&event.user_agent)
                        .bind(sqlx::types::Json(&event.metadata))
                        .bind(now)
                        .execute(&mut *conn)
                        .await
                        .map_err(failed)?;
                }
                Ok(ended)
            }
        }
    };
}
pub(crate) use impl_support_binding_ends;
