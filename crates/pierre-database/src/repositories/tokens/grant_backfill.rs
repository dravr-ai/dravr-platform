// ABOUTME: The live refresh-token families the one-time grant backfill left without a named grant, by tenant, client and user
// ABOUTME: One SQL text for both backends; read once at boot so the log names what migration 20261126001550 left NULL
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Refresh-token families still without a named grant (carnet#1550).
//!
//! Migration `20261126001550` links each live refresh token minted before
//! grants were named to its grant, when exactly one live grant matches its
//! tenant, client and user (JF 2026-11-25, "backfill grant ids"). The ones it
//! leaves `NULL` (no live grant, or several) keep being admitted as before
//! (`ClientGrantStanding::admits_token`), and this read names them for the
//! log by tenant, client and user, with how many families each holds: never
//! by token or family id.

use pierre_core::errors::{AppError, AppResult};

/// Live refresh tokens with no named grant, grouped by tenant, client and
/// user, unexpired at `$1`. Every tenant, by design: the one-time backfill's
/// boot report, which no request reaches.
pub(crate) const UNGRANTED_REFRESH_FAMILIES_SQL: &str = r"
            SELECT tenant_id, client_id, user_id, COUNT(DISTINCT family_id) AS families
            FROM oauth2_refresh_tokens
            WHERE client_grant_id IS NULL
              AND revoked = FALSE
              AND expires_at > $1
            GROUP BY tenant_id, client_id, user_id
            ORDER BY tenant_id, client_id, user_id
            ";

/// The live refresh-token families one athlete's client holds with no named
/// grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UngrantedRefreshFamilies {
    /// The tenant the tokens were minted in.
    pub tenant_id: String,
    /// The OAuth client holding them.
    pub client_id: String,
    /// The athlete they speak for.
    pub user_id: String,
    /// How many rotation families.
    pub families: i64,
}

/// Decode one row of [`UNGRANTED_REFRESH_FAMILIES_SQL`].
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn ungranted_families_from_row<R>(row: &R) -> AppResult<UngrantedRefreshFamilies>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column =
        |col: &str, e: sqlx::Error| AppError::database(format!("Failed to get {col}: {e}"));
    Ok(UngrantedRefreshFamilies {
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column("tenant_id", e))?,
        client_id: row
            .try_get("client_id")
            .map_err(|e| column("client_id", e))?,
        user_id: row.try_get("user_id").map_err(|e| column("user_id", e))?,
        families: row.try_get("families").map_err(|e| column("families", e))?,
    })
}

/// Emit the read for one backend type, which the `OAuth2ServerRepository`
/// implementation delegates to.
macro_rules! impl_ungranted_refresh_families {
    ($ty:ty) => {
        impl $ty {
            /// `refresh_families_without_grant`.
            async fn ungranted_family_rows(
                &self,
                now: Timestamp,
            ) -> AppResult<Vec<$crate::repositories::UngrantedRefreshFamilies>> {
                let grants =
                    $crate::repositories::tokens::grant_backfill::UNGRANTED_REFRESH_FAMILIES_SQL;
                sqlx::query(grants)
                    .bind(now)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to read refresh tokens without a grant: {e}"
                        ))
                    })?
                    .iter()
                    .map($crate::repositories::tokens::grant_backfill::ungranted_families_from_row)
                    .collect()
            }
        }
    };
}

pub(crate) use impl_ungranted_refresh_families;
