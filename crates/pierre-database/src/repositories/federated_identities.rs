// ABOUTME: The external sign-in identities (a Google account id) each account is reached by — trait, statements and body
// ABOUTME: One SQL text per operation; each backend shell supplies how it binds a uuid and reads one back as text

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Federated identities, written once.
//!
//! A row links `(provider, subject)` — `("google.com", <Google sub>)` — to
//! the account it signs in to. The primary key makes one identity reach one
//! account; a repeat link is a no-op. `user_id` is a `uuid` on Postgres and
//! `TEXT` on `SQLite`, so it binds through the macro's `$bind_id` and reads
//! back through `$text` (`::text` on Postgres, empty on `SQLite`).

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use uuid::Uuid;

/// The external sign-in identities an account is reached by.
///
/// Distinct from `users.firebase_uid`, which holds the Firebase UID Firebase
/// mints: the subject here is the identity provider's own account id, the
/// same whichever path (Firebase in the web app, or the authorization
/// server's direct Google sign-in) proved it.
#[async_trait]
pub trait FederatedIdentityRepository: Send + Sync {
    /// The account `(provider, subject)` is linked to, if any.
    async fn user_for_subject(&self, provider: &str, subject: &str) -> AppResult<Option<Uuid>>;
    /// The subject `user_id` is linked to at `provider`, if any.
    async fn subject_for_user(&self, user_id: Uuid, provider: &str) -> AppResult<Option<String>>;
    /// Link `(provider, subject)` to `user_id`. Returns `false`, changing
    /// nothing, when that identity is already linked (to any account).
    async fn link_subject(&self, user_id: Uuid, provider: &str, subject: &str) -> AppResult<bool>;
}

/// Link an identity; a repeat affects zero rows.
pub(crate) const LINK_SUBJECT_SQL: &str = r"
            INSERT INTO user_federated_identities (provider, subject, user_id, created_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (provider, subject) DO NOTHING
            ";

/// The account an identity is linked to.
macro_rules! user_for_subject_sql {
    ($text:literal) => {
        concat!(
            "SELECT user_id",
            $text,
            " AS user_id FROM user_federated_identities WHERE provider = $1 AND subject = $2"
        )
    };
}
pub(crate) use user_for_subject_sql;

/// The identity an account holds at one provider; the oldest when there are several.
pub(crate) const SUBJECT_FOR_USER_SQL: &str = r"
            SELECT subject FROM user_federated_identities
            WHERE user_id = $1 AND provider = $2
            ORDER BY created_at ASC
            LIMIT 1
            ";

/// Emit the whole [`FederatedIdentityRepository`] implementation for one
/// backend type. `$bind_id` turns a `Uuid` into what that backend's
/// `user_id` column accepts; `$text` reads it back as text. The invoking
/// shell must `use` every const, macro and type the body names.
macro_rules! impl_federated_identity_repository {
    ($ty:ty, $bind_id:path, $text:literal) => {
        #[async_trait::async_trait]
        impl FederatedIdentityRepository for $ty {
            async fn user_for_subject(
                &self,
                provider: &str,
                subject: &str,
            ) -> AppResult<Option<Uuid>> {
                let row = sqlx::query(user_for_subject_sql!($text))
                    .bind(provider)
                    .bind(subject)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to look up federated identity: {e}"))
                    })?;
                row.map(|row| {
                    let text: String = row.try_get("user_id").map_err(|e| {
                        AppError::database(format!("user_federated_identities user_id: {e}"))
                    })?;
                    Uuid::parse_str(&text).map_err(|e| {
                        AppError::database(format!("Invalid federated identity user_id: {e}"))
                    })
                })
                .transpose()
            }

            async fn subject_for_user(
                &self,
                user_id: Uuid,
                provider: &str,
            ) -> AppResult<Option<String>> {
                let row = sqlx::query(SUBJECT_FOR_USER_SQL)
                    .bind($bind_id(user_id))
                    .bind(provider)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read federated identity: {e}"))
                    })?;
                row.map(|row| {
                    row.try_get("subject").map_err(|e| {
                        AppError::database(format!("user_federated_identities subject: {e}"))
                    })
                })
                .transpose()
            }

            async fn link_subject(
                &self,
                user_id: Uuid,
                provider: &str,
                subject: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(LINK_SUBJECT_SQL)
                    .bind(provider)
                    .bind(subject)
                    .bind($bind_id(user_id))
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to link federated identity: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_federated_identity_repository;
