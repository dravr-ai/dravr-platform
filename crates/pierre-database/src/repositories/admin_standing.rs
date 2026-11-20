// ABOUTME: What a user write does to the credentials an account holds as an admin, inside that write's transaction
// ABOUTME: The withdrawn-credentials types, and the two statements each backend's user repository runs under its own lock
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Two rules follow the admin role, and both are kept by the write that
//! changes the row, in its own transaction (carnet#1574):
//!
//! - an account left out of admin standing (demoted, suspended, set back to
//!   pending, deactivated) loses the device logins it approved and the
//!   impersonations it opened;
//! - an account left with the admin role is no longer impersonated by anyone.
//!
//! Done after the write, as a second step, the first rule could fail on its
//! own and leave the credentials live behind a role already changed, where a
//! retry answered "not an admin" and never reached them. Split from
//! `users.rs`, whose `impl_user_repository!` body calls the two functions
//! [`impl_admin_standing_writes!`] emits.

use pierre_core::models::User;
use serde::Serialize;

/// What an account held on its admin standing and lost with it: the
/// device-login tokens it approved (`admin_tokens.operator_user_id` names
/// it) and the impersonations it had open.
///
/// The write that takes the account out of standing deactivates both in its
/// own transaction, so there is no moment, and no failed second step, in
/// which the account is demoted or suspended and still holds them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WithdrawnAdminCredentials {
    /// The device-login tokens deactivated, by token id; for a deleted
    /// account, the live ones its deletion retired, whose rows stay and
    /// name no account.
    pub device_tokens: Vec<String>,
    /// The impersonation sessions ended. A deleted account's sessions go
    /// with its row and are not counted here.
    pub impersonations: u64,
}

impl WithdrawnAdminCredentials {
    /// Whether the account held nothing to withdraw.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.device_tokens.is_empty() && self.impersonations == 0
    }
}

/// What [`UserRepository::update_status`](super::UserRepository::update_status) and
/// [`UserRepository::set_admin_status`](super::UserRepository::set_admin_status) wrote: the row as stored, and what
/// the account lost with its admin standing in the same transaction.
#[derive(Debug, Clone)]
pub struct StandingWrite {
    /// The account after the write.
    pub user: User,
    /// Empty unless the write left the account out of admin standing while
    /// it held such credentials.
    pub withdrawn: WithdrawnAdminCredentials,
}

/// Emit the two transaction-scoped statements for one backend type: `$row`
/// is the driver's row type and `$ids` that backend's
/// [`uuid_columns`](super::uuid_columns) codec, as in `impl_user_repository!`.
macro_rules! impl_admin_standing_writes {
    ($ty:ty, $row:ty, $ids:ident) => {
        impl $ty {
            /// Deactivate, in `conn`'s transaction, the device-login tokens
            /// `user_id` approved and end the impersonations it opened: what
            /// an account holds on its admin standing and loses with it.
            /// Only what is still live is touched, so repeating it is safe.
            ///
            /// # Errors
            /// Returns a database error when a statement fails; the caller's
            /// transaction is then not committed.
            async fn withdraw_admin_credentials(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                user_id: Uuid,
            ) -> AppResult<WithdrawnAdminCredentials> {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to withdraw an admin's credentials: {e}"))
                };
                let device_tokens = sqlx::query(DEACTIVATE_ADMIN_TOKENS_SQL)
                    .bind(user_id.to_string())
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(failed)?
                    .iter()
                    .map(|row| sqlx::Row::try_get(row, "id").map_err(failed))
                    .collect::<AppResult<Vec<String>>>()?;
                let impersonations = sqlx::query(END_ALL_SESSIONS_SQL)
                    .bind(Utc::now())
                    .bind($ids::bind(user_id))
                    .execute(&mut *conn)
                    .await
                    .map_err(failed)?
                    .rows_affected();
                let withdrawn = WithdrawnAdminCredentials {
                    device_tokens,
                    impersonations,
                };
                if !withdrawn.is_empty() {
                    info!(
                        user_id = %user_id,
                        device_tokens = withdrawn.device_tokens.len(),
                        impersonations = withdrawn.impersonations,
                        "Withdrew the credentials an account held as an admin"
                    );
                }
                Ok(withdrawn)
            }

            /// End, in `conn`'s transaction, every impersonation open on
            /// `user_id`, which the write leaves with the admin role.
            ///
            /// # Errors
            /// Returns a database error when the statement fails.
            async fn end_impersonations_of(
                conn: &mut <<$row as sqlx::Row>::Database as sqlx::Database>::Connection,
                user_id: Uuid,
            ) -> AppResult<()> {
                let ended = sqlx::query(END_SESSIONS_OF_TARGET_SQL)
                    .bind(Utc::now())
                    .bind($ids::bind(user_id))
                    .execute(conn)
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to end the impersonations of a new admin: {e}"
                        ))
                    })?
                    .rows_affected();
                if ended > 0 {
                    info!(
                        user_id = %user_id,
                        ended,
                        "Ended the impersonations open on an account that is now an admin"
                    );
                }
                Ok(())
            }
        }
    };
}
pub(crate) use impl_admin_standing_writes;
