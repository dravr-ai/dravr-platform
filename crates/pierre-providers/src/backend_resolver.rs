// ABOUTME: Resolves user-facing provider names to the actual backend implementation
// ABOUTME: Routes Strava and Garmin to OAuth when a token exists, else to the sciotte* mirror

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
//! Strava it takes precedence over the sciotte mirror. Garmin works the same
//! way since Dravr joined the Garmin Connect Developer Program (carnet#1155):
//! a user holding a `garmin` token is served from what Garmin pushed, and every
//! other Garmin athlete stays on the connection using usr/pwd from Dravr.
//! Garmin pushes only what the athlete records after connecting, so a live
//! usr/pwd session, which reads the whole history, serves Garmin whenever the
//! athlete holds one, whatever the grant's store holds; the grant serves only
//! an athlete with no live session (carnet#1302, [`oauth_grant`]).
//! TrainingPeaks and COROS stay on the mirror: neither has an API Pierre can
//! call.
//!
//! This module centralises that decision so handlers do not each re-invent
//! the alias rules.

use chrono::{DateTime, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_database::RepositoryRegistry;
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

/// The mirror-backed providers whose own OAuth backend serves them too: a
/// token row for that backend wins over the mirror. Strava's API is the
/// migration target; Garmin's is the Developer Program API, fed by push
/// (carnet#1155).
const OAUTH_SERVED: [&str; 2] = [oauth_providers::STRAVA, oauth_providers::GARMIN];

/// Whether `provider`'s own OAuth backend serves it beside its mirror, and so
/// wins over the mirror when the user holds a token row for it.
fn oauth_serves(provider: &str) -> bool {
    OAUTH_SERVED.contains(&provider)
}

/// Whether a request for `provider` goes to its mirror when the user holds no
/// row for either backend.
///
/// True for every mirror-backed provider but Strava. TrainingPeaks and COROS
/// have no API Pierre can call. Garmin's API is push-only and holds nothing
/// until the athlete connects it, so an unconnected Garmin request stays on
/// the connection using usr/pwd from Dravr, whose missing session surfaces as
/// a clean reconnect. Strava's API is the one a not-yet-connected user is
/// sent to connect. Accepts either half of a pair.
#[must_use]
pub fn mirror_is_default(provider: &str) -> bool {
    let user_facing = user_facing_name(provider);
    mirror_backend_for(user_facing).is_some() && user_facing != oauth_providers::STRAVA
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

/// The backends whose token row actually makes a user-facing provider usable,
/// in the order [`resolve_backend`] prefers them.
///
/// Connectedness is a ROUTING question, not a row-existence one. A status
/// surface that counted a row routing never reads reported
/// `connected: true, needs_reauth: false` while every coach data call failed
/// with "Provider `sciotte_garmin` requires authentication" (carnet#352): at
/// the time the raw `garmin` OAuth backend was never routed to.
///
/// Strava and Garmin accept either backend: the OAuth one takes precedence
/// when a token exists, falling back to the mirror. Garmin's OAuth backend
/// serves since carnet#1155, reading what Garmin pushed. TrainingPeaks and
/// COROS are served by their mirror alone (COROS's partner API is not
/// approved, carnet#509). Providers with no mirror serve themselves.
///
/// This lives here, beside `resolve_backend`, so the two cannot drift: a
/// handler that re-invents the rule is how they drifted in the first place.
#[must_use]
pub fn serving_backends(provider: &str) -> Vec<String> {
    // Accept either half of a coalesced card, as `backend_pair_for` does: a
    // caller naming the mirror must get the same answer as one naming the card.
    let user_facing = user_facing_name(provider);
    match mirror_backend_for(user_facing) {
        Some(mirror) if oauth_serves(user_facing) => {
            vec![user_facing.to_owned(), mirror.to_owned()]
        }
        Some(mirror) => vec![mirror.to_owned()],
        // No mirror: the provider is its own only backend.
        None => vec![user_facing.to_owned()],
    }
}

/// The backend whose token row the background health sync reads for a
/// provider, by the name the sync knows it under (`garmin`, `coros`, `whoop`).
///
/// A provider routed to its mirror by default ([`mirror_is_default`]: Garmin,
/// COROS) is pulled through its mirror's browser session, and every other
/// provider through its own backend. Garmin's OAuth backend is never pulled:
/// Garmin pushes its data ([`pushed_backend_for`]), so a user who holds it is
/// skipped by the sync rather than read through it. The sync stamps its rows
/// with the provider name, so a stored night is `coros` whichever backend fed
/// it, while the session lives on `sciotte_coros`.
#[must_use]
pub fn sync_backend(provider: &str) -> String {
    let user_facing = user_facing_name(provider);
    match mirror_backend_for(user_facing) {
        Some(mirror) if mirror_is_default(user_facing) => mirror.to_owned(),
        _ => serving_backends(provider)
            .into_iter()
            .next()
            .unwrap_or_else(|| provider.to_owned()),
    }
}

/// The OAuth backend that feeds `provider` by push for a user who holds a
/// token row for it, or `None` when the provider has none.
///
/// Garmin's Developer Program API pushes every summary to Dravr (carnet#1155),
/// so the background sync, which pulls through [`sync_backend`], must leave a
/// user who holds this backend alone: reading them through the usr/pwd
/// session as well would write the same days twice.
#[must_use]
pub fn pushed_backend_for(provider: &str) -> Option<&'static str> {
    (user_facing_name(provider) == oauth_providers::GARMIN).then_some(oauth_providers::GARMIN)
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

/// The provider an activity reads as to an athlete or a model.
///
/// A mirror backend stamps every row it reads with its own backend name —
/// the bare `sciotte`, whichever service it signed in to — and records that
/// service (`garmin`, `strava`, `trainingpeaks`, `coros`) as the row's
/// `source`. So a mirror row reads as its source, falling back to the
/// mirror's user-facing name. Any other row reads as its own provider:
/// there `source` names the device or app that recorded it, not the
/// connection it came through.
///
/// [`user_facing_name`] of the row's provider alone is wrong here: the bare
/// stamp maps to `strava`, which would name every scraped Garmin, COROS and
/// TrainingPeaks row Strava.
#[must_use]
pub fn activity_origin<'a>(provider: &'a str, source: Option<&'a str>) -> &'a str {
    if is_mirror_backend(provider) {
        source.unwrap_or_else(|| user_facing_name(provider))
    } else {
        provider
    }
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
    repos: &RepositoryRegistry,
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

/// Whether the push-fed store behind `oauth_backend` holds no activity for
/// the user in this tenant: the provider pushes only what the athlete records
/// after connecting, so a fresh grant holds nothing, whatever the window.
///
/// `false` for a backend that is not push-fed (its API reads history on
/// request) and when the store cannot be read: a failed read keeps the grant
/// the user chose rather than moving them on a guess. Existence is read from
/// the rows themselves, never from `latest_activity_sync`, which a push of
/// wellness data alone also advances.
async fn pushed_store_is_empty(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    oauth_backend: &str,
) -> bool {
    if pushed_backend_for(oauth_backend) != Some(oauth_backend) {
        return false;
    }
    matches!(
        repos
            .activity_cache
            .get_cached_activities(
                user_id,
                &tenant_id,
                Some(oauth_backend),
                DateTime::<Utc>::UNIX_EPOCH,
                Utc::now(),
                1,
            )
            .await,
        Ok(rows) if rows.is_empty()
    )
}

/// Whether the user holds a usable session for `mirror` in this tenant: a
/// token row, and a connection that has not been flagged for a reconnect or
/// revoked. A dead session serves nothing, so it never takes over from an
/// OAuth grant.
async fn mirror_session_is_live(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    mirror: &str,
) -> bool {
    if !has_token_row(repos, user_id, tenant_id, mirror).await {
        return false;
    }
    repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .is_ok_and(|connections| {
            connections
                .iter()
                .any(|conn| conn.provider == mirror && !conn.status.requires_reauth())
        })
}

/// What the user's OAuth grant for a mirror-backed provider amounts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthGrant {
    /// The user holds no OAuth token row for the provider.
    Absent,
    /// The grant serves the provider's reads.
    Serves,
    /// A live mirror session serves the provider instead, whatever the
    /// grant's push-fed store holds (carnet#1302).
    DefersToMirror,
    /// The grant's push-fed store holds no activity yet and no live mirror
    /// session stands beside it: the grant serves, with nothing to read, so
    /// the clients offer the mirror login for the athlete's history.
    AwaitsActivities,
}

impl OAuthGrant {
    /// Whether the OAuth backend serves the provider's reads.
    #[must_use]
    pub const fn serves(self) -> bool {
        matches!(self, Self::Serves | Self::AwaitsActivities)
    }
}

/// Classify the user's OAuth grant for `user_facing` in this tenant — the one
/// rule [`resolve_backend`], [`coalesced_status`] and the connect cards read,
/// so routing and every status surface agree (carnet#1302).
///
/// Strava's grant serves whenever its row exists: its API reads history on
/// request. Garmin's API pushes only what the athlete records after
/// connecting, so a live usr/pwd session beside the grant serves Garmin
/// whatever the grant's store holds: the session reads the whole history,
/// the pushes only what came after the grant. A session flagged for a
/// reconnect or revoked serves nothing, and then the grant serves: reading
/// [`OAuthGrant::AwaitsActivities`] while its store holds no activity at all
/// (the clients then offer the usr/pwd login), [`OAuthGrant::Serves`] once it
/// holds one. "No activity" is the whole store, never a requested window, so
/// one user resolves to one backend — and one cache key — whatever they ask
/// for.
/// Providers whose OAuth backend never serves beside its mirror, and those
/// with no mirror, read [`OAuthGrant::Absent`].
pub async fn oauth_grant(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    user_facing: &str,
) -> OAuthGrant {
    let Some(mirror) = mirror_backend_for(user_facing) else {
        return OAuthGrant::Absent;
    };
    if !oauth_serves(user_facing) || !has_token_row(repos, user_id, tenant_id, user_facing).await {
        return OAuthGrant::Absent;
    }
    if pushed_backend_for(user_facing) != Some(user_facing) {
        return OAuthGrant::Serves;
    }
    if mirror_session_is_live(repos, user_id, tenant_id, mirror).await {
        return OAuthGrant::DefersToMirror;
    }
    if pushed_store_is_empty(repos, user_id, tenant_id, user_facing).await {
        OAuthGrant::AwaitsActivities
    } else {
        OAuthGrant::Serves
    }
}

/// Resolve a user-facing provider name to the backend that should actually
/// serve the request.
///
/// Rules:
/// - `strava` → `strava` when the user holds a Strava OAuth token row, else
///   `sciotte` when a sciotte token row exists, else `strava`
/// - `garmin` → `sciotte_garmin` while the user holds a live `sciotte_garmin`
///   session, whatever the Garmin API store holds; else `garmin` when the
///   user holds a Garmin API token row; else `sciotte_garmin`, with or
///   without a session row (carnet#1155, carnet#1302 — [`oauth_grant`])
/// - `trainingpeaks` → `sciotte_trainingpeaks` always (no API to call)
/// - `coros` → `sciotte_coros` always (its partner API is not approved)
/// - any other name (including a mirror backend name passed through
///   directly) is returned unchanged
///
/// The resolver honours the user's explicit preference even when the
/// mirror session is stale. Downstream code surfaces an auth failure in
/// that case, which the caller is expected to translate into a
/// mirror-re-login prompt — never a fallback to OAuth.
///
/// A provider routed to its mirror by default ([`mirror_is_default`])
/// resolves to the mirror even with no mirror token: returning the
/// user-facing name would send an unconnected Garmin athlete to an API that
/// holds nothing for them (and TrainingPeaks to an unknown provider), where
/// staying on the mirror makes a missing session surface as a clean reconnect
/// auth error instead.
pub async fn resolve_backend(
    repos: &RepositoryRegistry,
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
    // Strava → OAuth-API migration, and the Garmin Developer Program API
    // (carnet#1155): the OAuth backend wins whenever the user holds a token
    // row for it, except that a live Garmin usr/pwd session serves Garmin
    // whatever the grant holds (carnet#1302). Mirror-only users have no such
    // row, so they keep the mirror until they authorize OAuth.
    match oauth_grant(repos, user_id, tid, requested).await {
        grant if grant.serves() => return requested.to_owned(),
        OAuthGrant::DefersToMirror => return mirror.to_owned(),
        _ => {}
    }
    if has_token_row(repos, user_id, tid, mirror).await {
        return mirror.to_owned();
    }
    // No row of either backend. Strava keeps returning the OAuth name (a real,
    // connectable backend) so a not-yet-connected user is routed into the OAuth
    // connect flow rather than a dead scraper; every other pair stays on its
    // mirror (see `mirror_is_default`).
    if mirror_is_default(requested) {
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
    /// A coaching platform read through the group coach's own credential
    /// under a link the member confirmed: the member holds no credential of
    /// their own
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
/// For Strava and Garmin a serving OAuth grant ([`oauth_grant`]) wins over the
/// sciotte mirror, matching `resolve_backend`. For every other mirror-backed
/// provider (TrainingPeaks, COROS) the mirror wins when present — even if
/// stale — because the user's stated preference is to keep using it. With
/// no credential of the user's own, a confirmed link their group coach reads
/// one of the provider's backends through ([`BackendKind::Delegated`])
/// connects it, found the way the read path finds it, so the status and the
/// read cannot disagree. A link that
/// cannot be read counts as none, as an unreadable token row does.
pub async fn coalesced_status(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    user_facing: &'static str,
) -> CoalescedStatus {
    // Strava and Garmin prefer their OAuth backend: report it as the active
    // backend when its grant serves, even if a mirror row also lingers — by
    // the same rule `resolve_backend` routes by ([`oauth_grant`]).
    if oauth_grant(repos, user_id, tenant_id, user_facing)
        .await
        .serves()
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
    }

    // The provider's own row counts only when it is one of the backends that
    // can serve it: a row routing never reads reported `connected: true` here
    // while every coach call failed (carnet#352). Same predicate the card
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

    // With no credential of the user's own, a confirmed link their group
    // coach reads one of the provider's backends through connects it.
    for backend in serving_backends(user_facing) {
        if let Ok(Some(link)) = repos
            .delegated_connections
            .find_active_for_member(user_id, tenant_id, &backend)
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

    CoalescedStatus {
        user_facing,
        connected: false,
        backend_kind: BackendKind::None,
        delegation: None,
    }
}
