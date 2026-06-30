// ABOUTME: Repository trait + shorten helper for the channel-agnostic URL shortener
// ABOUTME: Maps a short dot-free code to a full URL so WhatsApp can linkify chat reconnect/connect links
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::AppResult;
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
