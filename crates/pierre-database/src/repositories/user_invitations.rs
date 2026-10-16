// ABOUTME: Repository trait, shared statements and body for single-use user invitations — issue, redeem once, revoke, list, rate-limit
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its invited_by column needs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! User invitations, written once.
//!
//! The link an operator mails to an address that has no account, where the
//! invitee chooses their own password. It uses the `<selector>.<verifier>`
//! mechanism of every other mailed link, in a token space of its own: only
//! the SHA-256 of the verifier is stored, a wrong guess costs one attempt
//! without spending the invitation, and past the attempt cap the invitation
//! is revoked.
//!
//! An address holds at most one live invitation (`idx_user_invitations_live_email`):
//! [`UserInvitationRepository::issue`] revokes the previous one and inserts
//! the new one in one transaction, so an older link stops working the moment
//! a newer one is sent.
//!
//! `invited_by` is a `uuid` column on Postgres and `TEXT` on `SQLite`, so the
//! shell hands the body its [`uuid_columns`](super::uuid_columns) codec;
//! timestamps bind as [`DateTime<Utc>`] on both. The consume runs through the one body every link token shares
//! ([`link_token_consume`](super::link_token_consume)), which binds the
//! attempt cap against the `INTEGER` `attempt_count` column on both drivers.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserInvitation;
use uuid::Uuid;

use super::link_token_consume::{link_token_guess_sql, LinkTokenTable, LINK_TOKEN_MAX_ATTEMPTS};

/// Single-use invitations mailed to an address with no account.
#[async_trait]
pub trait UserInvitationRepository: Send + Sync {
    /// Issue an invitation to `email`, revoking any live one it already holds.
    ///
    /// `selector` is the plaintext lookup half and `verifier_hash` the SHA-256
    /// of the secret half; the delivered token is never stored whole.
    async fn issue(
        &self,
        email: &str,
        invited_by: Option<Uuid>,
        selector: &str,
        verifier_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<UserInvitation>;
    /// Redeem an invitation once, returning it.
    ///
    /// Every failure (unknown, expired, redeemed, revoked, wrong verifier,
    /// locked out) answers the same error, so the caller cannot tell which it
    /// hit.
    async fn redeem(&self, selector: &str, verifier_hash: &str) -> AppResult<UserInvitation>;
    /// Revoke the live invitation an address holds. Returns whether there was one.
    async fn revoke_for_email(&self, email: &str) -> AppResult<bool>;
    /// Every invitation still redeemable at `now`, oldest first.
    async fn list_live(&self, now: DateTime<Utc>) -> AppResult<Vec<UserInvitation>>;
    /// Invitations issued to an address since `since`, live or not, for the send budget.
    async fn count_recent(&self, email: &str, since: DateTime<Utc>) -> AppResult<i64>;
}

/// Supersede whatever the address still holds: unredeemed and unrevoked rows,
/// expired or not, since the live-email index counts an expired row as live.
pub(crate) const REVOKE_LIVE_INVITATIONS_SQL: &str = r"
            UPDATE user_invitations SET revoked_at = $1
            WHERE email = $2
              AND redeemed_at IS NULL
              AND revoked_at IS NULL
            ";

/// Issue one invitation: the selector half in clear, the verifier half hashed.
pub(crate) const INSERT_INVITATION_SQL: &str = r"
            INSERT INTO user_invitations
                (id, email, selector, token_hash, invited_by, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ";

/// How an invitation is redeemed ([`consume_link_token`]): live means
/// unredeemed and unrevoked, and an invitation past the cap is revoked.
pub(crate) const INVITATION_LINK_TOKEN: LinkTokenTable = LinkTokenTable {
    guess_sql: link_token_guess_sql!(
        "user_invitations",
        "redeemed_at IS NULL AND revoked_at IS NULL",
        "email, invited_by, expires_at, created_at"
    ),
    lock_sql: "UPDATE user_invitations SET revoked_at = $1 \
               WHERE selector = $2 AND redeemed_at IS NULL AND revoked_at IS NULL \
               AND attempt_count >= $3",
    claim_sql: "UPDATE user_invitations SET redeemed_at = $1 \
                WHERE selector = $2 AND redeemed_at IS NULL AND revoked_at IS NULL",
    invalid: invalid_invitation,
    what: "invitation",
    max_attempts: LINK_TOKEN_MAX_ATTEMPTS,
};

/// Every invitation still redeemable at `$1`, oldest first.
pub(crate) const LIST_LIVE_INVITATIONS_SQL: &str = r"
            SELECT email, invited_by, expires_at, created_at
            FROM user_invitations
            WHERE redeemed_at IS NULL
              AND revoked_at IS NULL
              AND expires_at > $1
            ORDER BY created_at ASC, email ASC
            ";

/// Invitations issued to an address since a moment, live or not.
pub(crate) const COUNT_RECENT_INVITATIONS_SQL: &str = r"
            SELECT COUNT(*) AS cnt
            FROM user_invitations
            WHERE email = $1
              AND created_at >= $2
            ";

/// One uniform error for every redemption failure, so the endpoint never
/// reveals which one hit.
#[must_use]
pub fn invalid_invitation() -> AppError {
    AppError::not_found("Invitation link is invalid, expired, or already used")
}

/// Decode the columns every invitation read shares. `invited_by` is decoded
/// by the caller through its backend's uuid codec and handed in; `try_get`
/// throughout, so a corrupt row surfaces as a recoverable error.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn invitation_from_row<R>(row: &R, invited_by: Option<Uuid>) -> AppResult<UserInvitation>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(UserInvitation {
        email: row
            .try_get("email")
            .map_err(|e| AppError::database(format!("user_invitations email: {e}")))?,
        invited_by,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| AppError::database(format!("user_invitations expires_at: {e}")))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::database(format!("user_invitations created_at: {e}")))?,
    })
}

/// Emit the whole [`UserInvitationRepository`] implementation for one backend
/// type. `$ids` is that backend's [`uuid_columns`](super::uuid_columns)
/// codec. The body names its consts, helpers and types unqualified, so the
/// invoking shell must `use` every one of them.
macro_rules! impl_user_invitation_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl UserInvitationRepository for $ty {
            async fn issue(
                &self,
                email: &str,
                invited_by: Option<Uuid>,
                selector: &str,
                verifier_hash: &str,
                expires_at: DateTime<Utc>,
            ) -> AppResult<UserInvitation> {
                let email = normalize_email(email);
                let now = Utc::now();
                let issue_error =
                    |e: sqlx::Error| AppError::database(format!("Failed to issue invitation: {e}"));

                let mut tx = self.pool().begin().await.map_err(issue_error)?;
                sqlx::query(REVOKE_LIVE_INVITATIONS_SQL)
                    .bind(now)
                    .bind(&email)
                    .execute(&mut *tx)
                    .await
                    .map_err(issue_error)?;
                sqlx::query(INSERT_INVITATION_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(&email)
                    .bind(selector)
                    .bind(verifier_hash)
                    .bind($ids::bind_opt(invited_by))
                    .bind(expires_at)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(issue_error)?;
                tx.commit().await.map_err(issue_error)?;

                Ok(UserInvitation {
                    email,
                    invited_by,
                    expires_at,
                    created_at: now,
                })
            }

            async fn redeem(
                &self,
                selector: &str,
                verifier_hash: &str,
            ) -> AppResult<UserInvitation> {
                let row = consume_link_token!(
                    self.pool(),
                    &INVITATION_LINK_TOKEN,
                    selector,
                    verifier_hash
                )?;
                invitation_from_row(&row, $ids::read_opt(&row, "invited_by")?)
            }

            async fn revoke_for_email(&self, email: &str) -> AppResult<bool> {
                let result = sqlx::query(REVOKE_LIVE_INVITATIONS_SQL)
                    .bind(Utc::now())
                    .bind(normalize_email(email))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to revoke invitation: {e}")))?;

                Ok(result.rows_affected() > 0)
            }

            async fn list_live(&self, now: DateTime<Utc>) -> AppResult<Vec<UserInvitation>> {
                let rows = sqlx::query(LIST_LIVE_INVITATIONS_SQL)
                    .bind(now)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list invitations: {e}")))?;

                rows.iter()
                    .map(|row| invitation_from_row(row, $ids::read_opt(row, "invited_by")?))
                    .collect()
            }

            async fn count_recent(&self, email: &str, since: DateTime<Utc>) -> AppResult<i64> {
                let row = sqlx::query(COUNT_RECENT_INVITATIONS_SQL)
                    .bind(normalize_email(email))
                    .bind(since)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count recent invitations: {e}"))
                    })?;

                row.try_get::<i64, _>("cnt")
                    .map_err(|e| AppError::database(format!("Failed to get cnt: {e}")))
            }
        }
    };
}
pub(crate) use impl_user_invitation_repository;
