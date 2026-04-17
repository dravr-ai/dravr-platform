// ABOUTME: Resolves user-facing provider names to the actual backend implementation
// ABOUTME: Prefers web-scraping (sciotte*) backends when their token row exists

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Backend resolution for provider aliasing.
//!
//! Some fitness services have more than one auth backend inside Pierre:
//!
//! - `strava` (OAuth against Strava's public API) and `sciotte` (web-scraping
//!   of the user's logged-in Strava session via dravr-sciotte)
//! - `garmin` (OAuth) and `sciotte_garmin` (web-scraping of Garmin Connect)
//!
//! From the user's point of view there is only one provider — "Strava" or
//! "Garmin". `sciotte*` is an internal implementation detail that must never
//! be surfaced to an end user via chat, messaging, or tool output.
//!
//! When a user has a sciotte* token row in the database (regardless of
//! whether the session cookies are still valid — a stale row still encodes
//! their explicit choice to use the mirror backend), any request that
//! targets the user-facing provider name must be routed to the mirror
//! backend. If the mirror session is stale the user is prompted to
//! re-authenticate through the mirror flow, never through OAuth.
//!
//! This module centralises that decision so handlers do not each re-invent
//! the alias rules.

use std::sync::Arc;

use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

use crate::models::TenantId;

/// Return the mirror-backend provider name for a user-facing provider, if any.
///
/// Returns `Some(backend)` when the given user-facing provider has a mirror
/// backend, and `None` otherwise (most providers have no mirror).
#[must_use]
pub fn mirror_backend_for(user_facing: &str) -> Option<&'static str> {
    match user_facing {
        oauth_providers::STRAVA => Some(oauth_providers::SCIOTTE),
        oauth_providers::GARMIN => Some(oauth_providers::SCIOTTE_GARMIN),
        _ => None,
    }
}

/// Map a backend provider name to the user-facing provider it serves.
///
/// `sciotte` and `sciotte_garmin` are mirror backends — they appear to
/// users and LLMs as `strava` and `garmin`. Any other name is returned
/// unchanged.
#[must_use]
pub fn user_facing_name(backend: &str) -> &str {
    match backend {
        oauth_providers::SCIOTTE => oauth_providers::STRAVA,
        oauth_providers::SCIOTTE_GARMIN => oauth_providers::GARMIN,
        other => other,
    }
}

/// Is this provider name an internal mirror-backend that must never
/// surface to end users?
#[must_use]
pub fn is_mirror_backend(provider: &str) -> bool {
    matches!(
        provider,
        oauth_providers::SCIOTTE | oauth_providers::SCIOTTE_GARMIN
    )
}

/// Does the user have a token row (valid or stale) for this backend?
///
/// The `oauth_tokens` table stores sciotte sessions the same way it stores
/// OAuth tokens. Row presence — independent of expiry — encodes the user's
/// intent to use that backend; callers use this to decide whether to
/// prefer the mirror over OAuth.
async fn has_token_row(
    repos: &Arc<RepositoryRegistry>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> bool {
    matches!(
        repos
            .oauth_tokens
            .get_token(user_id, tenant_id, provider)
            .await,
        Ok(Some(_))
    )
}

/// Resolve a user-facing provider name to the backend that should actually
/// serve the request.
///
/// Rules:
/// - `strava` → `sciotte` when a sciotte token row exists for the user
/// - `garmin` → `sciotte_garmin` when a `sciotte_garmin` token row exists
/// - any other name (including a mirror backend name passed through
///   directly) is returned unchanged
///
/// The resolver honours the user's explicit preference even when the
/// mirror session is stale. Downstream code surfaces an auth failure in
/// that case, which the caller is expected to translate into a
/// mirror-re-login prompt — never a fallback to OAuth.
pub async fn resolve_backend(
    repos: &Arc<RepositoryRegistry>,
    user_id: Uuid,
    tenant_id: Option<TenantId>,
    requested: &str,
) -> String {
    let Some(mirror) = mirror_backend_for(requested) else {
        return requested.to_owned();
    };
    let Some(tid) = tenant_id else {
        return requested.to_owned();
    };
    if has_token_row(repos, user_id, tid, mirror).await {
        mirror.to_owned()
    } else {
        requested.to_owned()
    }
}

/// Connection status for a user-facing provider after mirror coalescing.
///
/// Used by `get_connection_status` to report a single row per user-facing
/// provider while recording which backend is active so routing decisions
/// downstream remain deterministic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoalescedStatus {
    /// User-facing provider name (e.g. "strava")
    pub user_facing: &'static str,
    /// Whether any backend (OAuth or mirror) has a token row
    pub connected: bool,
    /// Which backend is active when connected: "oauth" or "mirror"
    pub backend_kind: BackendKind,
}

/// Kind of backend serving a user-facing provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// No backend is connected for this provider
    None,
    /// OAuth backend is the active one
    Oauth,
    /// Mirror (sciotte*) backend is the active one
    Mirror,
}

impl BackendKind {
    /// Short string form for JSON output (`"oauth"`, `"mirror"`, `"none"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Oauth => "oauth",
            Self::Mirror => "mirror",
        }
    }
}

/// Probe connection status for a single user-facing provider.
///
/// Mirror presence wins over OAuth. The function reports `Mirror` when
/// the mirror row exists (even if stale) because the user's stated
/// preference is to keep using it.
pub async fn coalesced_status(
    repos: &Arc<RepositoryRegistry>,
    user_id: Uuid,
    tenant_id: TenantId,
    user_facing: &'static str,
) -> CoalescedStatus {
    if let Some(mirror) = mirror_backend_for(user_facing) {
        if has_token_row(repos, user_id, tenant_id, mirror).await {
            return CoalescedStatus {
                user_facing,
                connected: true,
                backend_kind: BackendKind::Mirror,
            };
        }
    }

    if has_token_row(repos, user_id, tenant_id, user_facing).await {
        return CoalescedStatus {
            user_facing,
            connected: true,
            backend_kind: BackendKind::Oauth,
        };
    }

    CoalescedStatus {
        user_facing,
        connected: false,
        backend_kind: BackendKind::None,
    }
}
