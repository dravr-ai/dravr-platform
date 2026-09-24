// ABOUTME: Resolves user-facing provider names to the actual backend implementation
// ABOUTME: Routes Strava to OAuth when a token exists, else to the sciotte* mirror

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Backend resolution for provider aliasing.
//!
//! Some fitness services have more than one auth backend inside Pierre:
//!
//! - `strava` (OAuth against Strava's public API) and `sciotte` (web-scraping
//!   of the user's logged-in Strava session via dravr-sciotte)
//! - `garmin` (OAuth) and `sciotte_garmin` (web-scraping of Garmin Connect)
//! - `trainingpeaks` (a name only — TrainingPeaks' partner API is closed, so
//!   no OAuth backend exists) and `sciotte_trainingpeaks` (web-scraping of the
//!   TrainingPeaks calendar)
//!
//! From the user's point of view there is only one provider — "Strava",
//! "Garmin" or "TrainingPeaks". `sciotte*` is an internal implementation
//! detail that must never be surfaced to an end user via chat, messaging, or
//! tool output.
//!
//! When a user has a sciotte* token row in the database (regardless of
//! whether the session cookies are still valid — a stale row still encodes
//! their explicit choice to use the mirror backend), a request targeting the
//! user-facing provider name is routed to the mirror backend. If the mirror
//! session is stale the user is prompted to re-authenticate through the mirror
//! flow.
//!
//! Strava is migrating to its OAuth API: when a user holds an OAuth token for
//! Strava it takes precedence over the sciotte mirror. Garmin and TrainingPeaks
//! stay on the mirror: Garmin's official API is partner-gated, and
//! TrainingPeaks has none Pierre can call.
//!
//! This module centralises that decision so handlers do not each re-invent
//! the alias rules.

use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_database::AuthRepos;
use uuid::Uuid;

use crate::registry::ProviderRegistry;
use pierre_core::models::{DelegatedConnection, TenantId};

/// Every mirror pair: the user-facing provider and the sciotte backend that
/// scrapes it. The single table the name mappings below read, so a provider
/// added here is known to routing, status, the hosted login and the reconnect
/// copy at once.
const MIRROR_PAIRS: [(&str, &str); 4] = [
    (oauth_providers::STRAVA, oauth_providers::SCIOTTE),
    (oauth_providers::GARMIN, oauth_providers::SCIOTTE_GARMIN),
    (
        oauth_providers::TRAININGPEAKS,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    ),
    (oauth_providers::COROS, oauth_providers::SCIOTTE_COROS),
];

/// Return the mirror-backend provider name for a user-facing provider, if any.
///
/// Returns `Some(backend)` when the given user-facing provider has a mirror
/// backend, and `None` otherwise (most providers have no mirror).
#[must_use]
pub fn mirror_backend_for(user_facing: &str) -> Option<&'static str> {
    MIRROR_PAIRS
        .iter()
        .find(|(provider, _)| *provider == user_facing)
        .map(|(_, mirror)| *mirror)
}

/// The hosted login page's `target` for a mirror backend (`strava`, `garmin`,
/// `trainingpeaks`, `coros`), or `None` for a slug with no hosted login.
///
/// The scraper keys a login on the name the athlete knows the provider by, so
/// the target is the mirror's user-facing name — the same string
/// `SciotteTarget::from_target_param` reads back when the page posts. A `None`
/// is an OAuth provider, which reconnects through its authorization URL.
#[must_use]
pub fn hosted_login_target(backend: &str) -> Option<&'static str> {
    MIRROR_PAIRS
        .iter()
        .find(|(_, mirror)| *mirror == backend)
        .map(|(provider, _)| *provider)
}

/// The hosted-login targets, in table order — for a refusal that names them.
#[must_use]
pub fn hosted_login_targets() -> Vec<&'static str> {
    MIRROR_PAIRS.iter().map(|(provider, _)| *provider).collect()
}

/// The brand name an athlete reads for a provider slug, or `None` when no
/// registered descriptor names it.
///
/// Either half of a mirror pair reads as the mirror's descriptor ("Garmin"
/// for both `garmin` and `sciotte_garmin`): the athlete connected through the
/// scrape, and the OAuth descriptor's "Garmin Connect" names an API they never
/// touched. The registry is the one source, so the reconnect copy, the hosted
/// pages and the connect cards cannot disagree on a label.
#[must_use]
pub fn brand_name(registry: &ProviderRegistry, slug: &str) -> Option<&'static str> {
    registry.get_display_name(mirror_backend_for(slug).unwrap_or(slug))
}

/// Map a backend provider name to the user-facing provider it serves.
///
/// `sciotte`, `sciotte_garmin`, `sciotte_trainingpeaks` and `sciotte_coros`
/// are mirror backends — they appear to users and LLMs as `strava`, `garmin`,
/// `trainingpeaks` and `coros`. Any other name is returned unchanged.
#[must_use]
pub fn user_facing_name(backend: &str) -> &str {
    hosted_login_target(backend).unwrap_or(backend)
}

/// The backends whose token row actually makes a user-facing provider usable.
///
/// Connectedness is a ROUTING question, not a row-existence one, and the two
/// answers differ for Garmin. `resolve_backend` sends every Garmin request to
/// `sciotte_garmin` unconditionally — the raw `garmin` OAuth API is
/// partner-gated and uncredentialed, so it is never routed to and a `garmin`
/// row on its own serves nothing. A status surface that counted that row
/// reported `connected: true, needs_reauth: false` while every coach data call
/// failed with "Provider `sciotte_garmin` requires authentication" (carnet#352).
/// TrainingPeaks is the same shape with no OAuth backend at all: only the
/// `sciotte_trainingpeaks` row serves it. COROS is Garmin's shape: its partner
/// API is not approved (carnet#509), so only `sciotte_coros` serves it.
///
/// Strava is the opposite case and genuinely accepts either: its OAuth backend
/// is real and takes precedence when a token exists, falling back to the
/// mirror. Providers with no mirror serve themselves.
///
/// This lives here, beside `resolve_backend`, so the two cannot drift: a
/// handler that re-invents the rule is how they drifted in the first place.
#[must_use]
pub fn serving_backends(provider: &str) -> Vec<String> {
    // Accept either half of a coalesced card, as `backend_pair_for` does: a
    // caller naming the mirror must get the same answer as one naming the card.
    let user_facing = user_facing_name(provider);
    match user_facing {
        oauth_providers::STRAVA => vec![
            oauth_providers::STRAVA.to_owned(),
            oauth_providers::SCIOTTE.to_owned(),
        ],
        // Mirror only — see above.
        oauth_providers::GARMIN => vec![oauth_providers::SCIOTTE_GARMIN.to_owned()],
        oauth_providers::TRAININGPEAKS => vec![oauth_providers::SCIOTTE_TRAININGPEAKS.to_owned()],
        oauth_providers::COROS => vec![oauth_providers::SCIOTTE_COROS.to_owned()],
        // No mirror: the provider is its own only backend.
        other => vec![other.to_owned()],
    }
}

/// Every backend that can serve a user-facing provider, mirror included.
///
/// `get_connection_status` coalesces a provider and its mirror into ONE card,
/// so a caller acting on that card must act on the whole pair. Resolution is
/// one-directional — `mirror_backend_for` maps `strava` → `sciotte` and never
/// the reverse — so a caller naming the card's own id would otherwise touch
/// only half of it. Accepts either name and returns the same pair for both.
#[must_use]
pub fn backend_pair_for(provider: &str) -> Vec<String> {
    let user_facing = user_facing_name(provider).to_owned();
    let mut pair = vec![user_facing.clone()];
    if let Some(mirror) = mirror_backend_for(&user_facing) {
        pair.push(mirror.to_owned());
    }
    pair
}

/// Is this provider name an internal mirror-backend that must never
/// surface to end users?
#[must_use]
pub fn is_mirror_backend(provider: &str) -> bool {
    hosted_login_target(provider).is_some()
}

/// Does the user have a token row (valid or stale) for this backend?
///
/// The `oauth_tokens` table stores sciotte sessions the same way it stores
/// OAuth tokens. Row presence — independent of expiry — encodes the user's
/// intent to use that backend; callers use this to decide whether to
/// prefer the mirror over OAuth.
async fn has_token_row(
    repos: &AuthRepos,
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
/// - `garmin` → `sciotte_garmin` always (the mirror is Garmin's only backend)
/// - `trainingpeaks` → `sciotte_trainingpeaks` always (same reason)
/// - `coros` → `sciotte_coros` always (its partner API is not approved)
/// - any other name (including a mirror backend name passed through
///   directly) is returned unchanged
///
/// The resolver honours the user's explicit preference even when the
/// mirror session is stale. Downstream code surfaces an auth failure in
/// that case, which the caller is expected to translate into a
/// mirror-re-login prompt — never a fallback to OAuth.
///
/// Exceptions:
/// - A Strava request whose user holds an OAuth token resolves to the OAuth
///   backend instead of the sciotte mirror (Strava → OAuth-API migration).
/// - A request for a provider whose mirror is its ONLY backend (Garmin,
///   TrainingPeaks) always resolves to the mirror, even with no mirror token:
///   Garmin's OAuth API is partner-gated/uncredentialed and TrainingPeaks has
///   no API at all, so returning the user-facing name would hit "No Garmin
///   OAuth credentials" (or an unknown provider) and fail the turn. Staying on
///   the mirror makes a missing session surface as a clean reconnect auth
///   error instead.
pub async fn resolve_backend(
    repos: &AuthRepos,
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
    // Strava → OAuth-API migration (ADR: retire sciotte scraping for Strava).
    // Prefer the OAuth backend over the sciotte mirror whenever the user holds
    // an OAuth token for Strava. The OAuth-token check is itself the guard:
    // sciotte-only users have no OAuth token, so they keep the mirror until they
    // authorize OAuth, at which point their real OAuth connection wins. Garmin
    // and TrainingPeaks have no such branch and stay on the scraper.
    if requested == oauth_providers::STRAVA && has_token_row(repos, user_id, tid, requested).await {
        return requested.to_owned();
    }
    if has_token_row(repos, user_id, tid, mirror).await {
        return mirror.to_owned();
    }
    // No mirror token. When the mirror is the provider's ONLY backend (Garmin,
    // TrainingPeaks), stay on it: the raw `garmin` OAuth API is uncredentialed
    // and `trainingpeaks` has no backend at all, so a fallthrough fails the turn
    // with a credentials error instead of the scrape's clean reconnect auth
    // error. `serving_backends` is the one table that says which providers
    // those are, so a new mirror-only provider needs no branch here. Strava
    // keeps returning the OAuth name (a real, connectable backend) so a
    // not-yet-connected user is routed into the OAuth connect flow rather than
    // a dead scraper.
    if serving_backends(requested) == [mirror] {
        return mirror.to_owned();
    }
    requested.to_owned()
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
    /// Whether any backend (OAuth, mirror or a delegated link) serves it
    pub connected: bool,
    /// Which backend is active when connected
    pub backend_kind: BackendKind,
    /// The confirmed link a [`BackendKind::Delegated`] provider is read
    /// through — the coach whose session serves it; `None` for every other
    /// kind.
    pub delegation: Option<DelegatedConnection>,
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
    /// The mirror, read through the group coach's own session under a link
    /// the member confirmed: the member holds no session of their own
    Delegated,
}

impl BackendKind {
    /// Short string form for JSON output (`"oauth"`, `"mirror"`,
    /// `"delegated"`, `"none"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Oauth => "oauth",
            Self::Mirror => "mirror",
            Self::Delegated => "delegated",
        }
    }
}

/// Probe connection status for a single user-facing provider.
///
/// For Strava an OAuth token wins over the sciotte mirror (Strava → OAuth-API
/// migration), matching `resolve_backend`. For every other mirror-backed
/// provider (Garmin, TrainingPeaks) the mirror still wins when present — even
/// if stale — because the user's stated preference is to keep using it. With
/// no session of the user's own, a confirmed link their group coach reads the
/// mirror through ([`BackendKind::Delegated`]) connects it, found the way the
/// read path finds it, so the status and the read cannot disagree. A link that
/// cannot be read counts as none, as an unreadable token row does.
pub async fn coalesced_status(
    repos: &AuthRepos,
    user_id: Uuid,
    tenant_id: TenantId,
    user_facing: &'static str,
) -> CoalescedStatus {
    // Strava prefers its OAuth backend: report it as the active backend when an
    // OAuth token exists, even if a sciotte mirror row also lingers.
    if user_facing == oauth_providers::STRAVA
        && has_token_row(repos, user_id, tenant_id, user_facing).await
    {
        return CoalescedStatus {
            user_facing,
            connected: true,
            backend_kind: BackendKind::Oauth,
            delegation: None,
        };
    }

    if let Some(mirror) = mirror_backend_for(user_facing) {
        if has_token_row(repos, user_id, tenant_id, mirror).await {
            return CoalescedStatus {
                user_facing,
                connected: true,
                backend_kind: BackendKind::Mirror,
                delegation: None,
            };
        }
        if let Ok(Some(link)) = repos
            .delegated_connections
            .find_active_for_member(user_id, tenant_id, mirror)
            .await
        {
            return CoalescedStatus {
                user_facing,
                connected: true,
                backend_kind: BackendKind::Delegated,
                delegation: Some(link),
            };
        }
    }

    // The OAuth row counts only when it is one of the backends that can serve
    // this provider. Garmin's is not: `resolve_backend` routes every Garmin
    // request to the mirror, so a bare `garmin` row reported `connected: true`
    // here while every coach call failed (carnet#352). Same predicate the card
    // uses, so this surface and `/api/providers` cannot disagree.
    if serving_backends(user_facing)
        .iter()
        .any(|b| b == user_facing)
        && has_token_row(repos, user_id, tenant_id, user_facing).await
    {
        return CoalescedStatus {
            user_facing,
            connected: true,
            backend_kind: BackendKind::Oauth,
            delegation: None,
        };
    }

    CoalescedStatus {
        user_facing,
        connected: false,
        backend_kind: BackendKind::None,
        delegation: None,
    }
}
