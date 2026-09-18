// ABOUTME: Repository trait + shorten helper for the channel-agnostic URL shortener
// ABOUTME: Maps a short dot-free code to a full URL so WhatsApp can linkify chat reconnect/connect links
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use tracing::warn;
use uuid::Uuid;

/// How long a freshly-minted short link stays resolvable.
///
/// Deliberately generous: the signed link-token JWT *inside* `target_url` carries
/// the real, much shorter security window, so the short link only needs to outlive
/// it. A user who taps after the JWT expires simply lands on the hosted error page
/// (same as the long link today), never a dead `/r/<code>`.
const SHORT_LINK_TTL_HOURS: i64 = 24;

/// Persistent code → URL mapping backing the URL shortener.
///
/// Not resolved by tenant: the redirect is public (the recipient taps it in a chat
/// client before any auth round-trip), and the link-token JWT embedded in
/// `target_url` is the real authorization gate. `tenant_id` / `user_id` are audit
/// columns; lookup is by `code` + expiry only.
#[async_trait]
pub trait ShortLinkRepository: Send + Sync {
    /// Persist a `code` → `target_url` mapping that resolves until `expires_at`.
    ///
    /// `code` is a caller-supplied high-entropy url-safe token; `tenant_id` /
    /// `user_id` are stringified ids kept for audit + cleanup.
    async fn create_short_link(
        &self,
        code: &str,
        target_url: &str,
        tenant_id: &str,
        user_id: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Resolve `code` to its target URL when present and not yet expired.
    /// Returns `None` on miss or expiry.
    async fn resolve_short_link(&self, code: &str) -> AppResult<Option<String>>;

    /// Delete every short link whose TTL has elapsed, returning how many rows
    /// were removed.
    ///
    /// `resolve_short_link` already filters expired rows at read time, so this
    /// is purely storage hygiene: without it the table grows unbounded (one row
    /// per minted reconnect/connect link, and the chat reconnect path mints on
    /// every expired-session turn by design). A periodic background sweep calls
    /// this so only live (<= TTL) links are ever retained.
    async fn delete_expired_short_links(&self) -> AppResult<u64>;
}

/// Persist `target_url` behind a short, dot-free `<base_url>/r/<code>` link and
/// return that link, or fall back to `target_url` verbatim if persistence fails.
///
/// The code is a uuid-simple token (32 hex chars, 122-bit entropy) — url-safe and
/// free of the dots that make `WhatsApp` truncate linkification of the raw JWT URL.
/// On a DB error the caller still gets a working (if long) link, so a shortener
/// outage degrades clickability rather than dropping the message entirely.
pub async fn shorten_url(
    repo: &dyn ShortLinkRepository,
    base_url: &str,
    target_url: &str,
    tenant_id: &str,
    user_id: &str,
) -> String {
    let code = Uuid::new_v4().simple().to_string();
    let expires_at = Utc::now() + Duration::hours(SHORT_LINK_TTL_HOURS);
    match repo
        .create_short_link(&code, target_url, tenant_id, user_id, expires_at)
        .await
    {
        Ok(()) => format!("{}/r/{code}", base_url.trim_end_matches('/')),
        Err(e) => {
            warn!(error = %e, "shorten_url: persist failed, returning full URL");
            target_url.to_owned()
        }
    }
}

/// Mint a mapping that resolves until its epoch-second `expires_at`.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, and every bind on this table is a plain `&str`/`i64`, so one
/// statement serves both backends and cannot drift between them.
pub(crate) const INSERT_SHORT_LINK_SQL: &str = r"
            INSERT INTO short_links (code, target_url, tenant_id, user_id, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            ";

/// Resolve a code, filtering rows whose TTL has elapsed.
pub(crate) const RESOLVE_SHORT_LINK_SQL: &str = r"
            SELECT target_url
            FROM short_links
            WHERE code = $1 AND expires_at > $2
            ";

/// Storage hygiene: drop every elapsed row.
pub(crate) const SWEEP_SHORT_LINKS_SQL: &str = r"DELETE FROM short_links WHERE expires_at <= $1";

/// Extract the `target_url` column via `try_get` (never `Row::get`, which is
/// `try_get().unwrap()` and panics on a type/NULL surprise) so a corrupt row
/// surfaces as a recoverable error rather than a crash.
///
/// # Errors
/// Returns a database error when the column cannot be decoded.
pub(crate) fn target_url_from_row<R>(row: &R) -> AppResult<String>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    row.try_get::<String, _>("target_url")
        .map_err(|e| AppError::database(format!("short_link target_url: {e}")))
}

/// Emit the whole [`ShortLinkRepository`] implementation for one backend type.
/// The body is written once here; each backend's shell invokes it with its own
/// type, and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_short_link_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl ShortLinkRepository for $ty {
            async fn create_short_link(
                &self,
                code: &str,
                target_url: &str,
                tenant_id: &str,
                user_id: &str,
                expires_at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(INSERT_SHORT_LINK_SQL)
                    .bind(code)
                    .bind(target_url)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(expires_at.timestamp())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to insert short_link: {e}")))?;
                Ok(())
            }

            async fn resolve_short_link(&self, code: &str) -> AppResult<Option<String>> {
                let row = sqlx::query(RESOLVE_SHORT_LINK_SQL)
                    .bind(code)
                    .bind(Utc::now().timestamp())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read short_link: {e}")))?;
                row.map(|r| target_url_from_row(&r)).transpose()
            }

            async fn delete_expired_short_links(&self) -> AppResult<u64> {
                let result = sqlx::query(SWEEP_SHORT_LINKS_SQL)
                    .bind(Utc::now().timestamp())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to sweep short_links: {e}")))?;
                Ok(result.rows_affected())
            }
        }
    };
}
pub(crate) use impl_short_link_repository;
