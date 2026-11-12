// ABOUTME: Resolves user-facing provider names to the backend that serves an athlete's reads and writes
// ABOUTME: Sciotte first: a session that is not revoked serves, its live OAuth grant stands behind it as the fallback

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
//! Sciotte first (carnet#1494): every provider with a mirror is read through
//! its usr/pwd session, and the provider's own OAuth grant (Strava's API,
//! Garmin's Developer Program push, carnet#1155) stands behind it as the
//! fallback while it is live. One rule decides, [`read_plan`]:
//!
//! | The athlete holds | `primary` | `fallback` |
//! |---|---|---|
//! | a session that is not revoked, with or without a grant | mirror | the grant, while its connection is active |
//! | a live grant only | grant | none |
//! | a dead grant only | mirror, whose read raises the hosted sign-in | none |
//! | a revoked session and a live grant | grant | none |
//! | nothing | mirror ([`mirror_is_default`]) | none |
//!
//! [`resolve_backend`] is the plan's primary. A read that must reach one
//! backend exactly (a Strava webhook, the first read of a connection just
//! made) never goes through it. A write goes to the backend of the plan that
//! carries the write's capability ([`write_backend`]). TrainingPeaks and
//! COROS have no API Pierre can call, so their mirror is their only backend.
//!
//! This module centralises that decision so handlers do not each re-invent
//! the alias rules.

use std::iter;

use chrono::{DateTime, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

use crate::registry::ProviderRegistry;
use crate::spi::ProviderCapabilities;
use pierre_core::models::{ConnectionStatus, DelegatedConnection, ProviderConnection, TenantId};
use tracing::warn;

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
/// live grant stands behind the mirror as the fallback, and serves alone for
/// an athlete with no session. Strava's is its public API; Garmin's is the
/// Developer Program API, fed by push (carnet#1155).
const OAUTH_SERVED: [&str; 2] = [oauth_providers::STRAVA, oauth_providers::GARMIN];

/// Whether `provider`'s own OAuth backend serves it beside its mirror.
fn oauth_serves(provider: &str) -> bool {
    OAUTH_SERVED.contains(&provider)
}

/// Whether `provider`'s own OAuth backend reads the provider's data on
/// request, an API a caller can page through: Strava's. Garmin's is fed by
/// push, and TrainingPeaks and COROS have no API Pierre can call.
///
/// Such a provider keeps its own name where a mirror-only provider takes its
/// mirror's: the health sync pulls nothing through Strava's session
/// ([`sync_backend`]), and a build without Strava's mirror still shows
/// Strava's own card. Accepts either half of a pair.
#[must_use]
pub fn api_reads_on_request(provider: &str) -> bool {
    let user_facing = user_facing_name(provider);
    oauth_serves(user_facing) && pushed_backend_for(user_facing).is_none()
}

/// Whether a request for `provider` goes to its mirror when the user holds no
/// row for either backend.
///
/// True for every mirror-backed provider (carnet#1494): the athlete connects
/// through the hosted sign-in, and a missing session surfaces as that
/// sign-in's reconnect. Accepts either half of a pair.
#[must_use]
pub fn mirror_is_default(provider: &str) -> bool {
    mirror_backend_for(user_facing_name(provider)).is_some()
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

/// The hosted sign-in `target` an athlete reconnects `provider` through, or
/// `None` for a provider with no mirror, which reconnects through its own
/// authorization (WHOOP, Intervals.icu).
///
/// The one link rule of every reconnect (carnet#1494): a dead Strava or
/// Garmin grant reconnects through its mirror's hosted sign-in, never its
/// OAuth, as a dead session does. Accepts either half of a pair: both name
/// one target.
#[must_use]
pub fn reconnect_target(provider: &str) -> Option<&'static str> {
    mirror_backend_for(user_facing_name(provider)).and_then(hosted_login_target)
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
/// Connectedness is a ROUTING question, not a row-existence one. A status
/// surface that counted a row routing never reads reported
/// `connected: true, needs_reauth: false` while every coach data call failed
/// with "Provider `sciotte_garmin` requires authentication" (carnet#352): at
/// the time the raw `garmin` OAuth backend was never routed to.
///
/// Strava and Garmin accept either backend: which one serves an athlete is
/// [`read_plan`]'s to say, never this list's order. Garmin's OAuth backend
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
/// A mirror-backed provider whose own API does not read on request
/// ([`api_reads_on_request`]: Garmin, COROS, TrainingPeaks) is pulled through
/// its mirror's browser session, and every other provider through its own
/// backend: Strava has no wellness to pull. Garmin's OAuth backend is never
/// pulled: Garmin pushes its data ([`pushed_backend_for`]), so a user whose
/// live grant is in their read plan ([`read_plan`]) is left to the pushes
/// rather than pulled. The sync stamps its rows with the provider name, so a
/// stored night is `coros` whichever backend fed it, while the session lives
/// on `sciotte_coros`.
#[must_use]
pub fn sync_backend(provider: &str) -> String {
    let user_facing = user_facing_name(provider);
    match mirror_backend_for(user_facing) {
        Some(mirror) if !api_reads_on_request(user_facing) => mirror.to_owned(),
        _ => user_facing.to_owned(),
    }
}

/// The OAuth backend that feeds `provider` by push for a user who holds a
/// token row for it, or `None` when the provider has none.
///
/// Garmin's Developer Program API pushes every summary to Dravr (carnet#1155),
/// so the background sync, which pulls through [`sync_backend`], leaves alone
/// a user whose live grant for this backend is in their read plan
/// ([`read_plan`]): the pushes feed the wellness, and reading it through the
/// usr/pwd session as well would write the same days twice.
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

/// Whether the user holds a token row for `backend` in this tenant, for the
/// read plan. A row that cannot be read (a store failure, a ciphertext that
/// does not decrypt) counts as held: routing the athlete away from it would
/// read a store failure as a missing credential and send them to reconnect
/// an intact connection.
async fn token_row_held(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    match repos
        .oauth_tokens
        .get_token(user_id, tenant_id, backend)
        .await
    {
        Ok(row) => row.is_some(),
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = backend,
                error = %e,
                "could not read the token row; routing as though it is held"
            );
            true
        }
    }
}

/// Whether the user holds any activity under the cache `key` in this tenant.
///
/// Read from the rows themselves. A store that cannot be read counts as
/// holding some: callers keep what the user has rather than act on a guess.
pub async fn holds_activities(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    key: &str,
) -> bool {
    !matches!(
        repos
            .activity_cache
            .get_cached_activities(
                user_id,
                &tenant_id,
                Some(key),
                DateTime::<Utc>::UNIX_EPOCH,
                Utc::now(),
                1,
            )
            .await,
        Ok(rows) if rows.is_empty()
    )
}

/// The activity-cache keys a read of `backend`'s own window serves, its own
/// key first.
///
/// - A mirror whose provider's OAuth backend serves it beside the mirror
///   (`sciotte`, `sciotte_garmin`) serves its grant's key too: the rows the
///   API stored before the session became the primary (carnet#1494) stay
///   served, and the pushes a live Garmin grant keeps storing merge into the
///   session's window.
/// - A push-fed OAuth backend ([`pushed_backend_for`]: `garmin`) serves its
///   mirror's key too, the history a usr/pwd session imported before the
///   grant retired it (carnet#1386).
/// - Every other backend has its one key: Strava's API reads history on
///   request, so a read naming it exactly serves what it stored.
///
/// Rows of both keys are one window: the readers merge every recording of a
/// workout into one session. The athlete's own uploads
/// ([`companion_keys`](oauth_providers::companion_keys)) are not among them:
/// those are served by their own path.
#[must_use]
pub fn history_keys(backend: &str) -> Vec<&str> {
    let other = if is_mirror_backend(backend) {
        Some(user_facing_name(backend)).filter(|grant| oauth_serves(grant))
    } else if pushed_backend_for(backend) == Some(backend) {
        mirror_backend_for(backend)
    } else {
        None
    };
    iter::once(backend).chain(other).collect()
}

/// What one half of a mirror pair (the session or the grant) amounts to for
/// an athlete in a tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Half {
    /// No token row.
    Absent,
    /// A token row whose connection is active, or that has no connection
    /// row: nothing ever flagged it.
    Live,
    /// A token row whose connection is `needs_reauth`: a refused refresh, a
    /// dead scrape session.
    Flagged,
    /// A token row whose connection was revoked.
    Revoked,
}

impl Half {
    /// Whether a session in this state is the athlete's way in: it is held
    /// and not revoked. A flagged session still is, so its read raises the
    /// hosted sign-in rather than passing to the grant for good.
    const fn signed_in(self) -> bool {
        matches!(self, Self::Live | Self::Flagged)
    }
}

/// The state of `backend`'s half among the athlete's `connections`, given
/// whether they hold a token row for it.
fn half(held: bool, connections: &[ProviderConnection], backend: &str) -> Half {
    if !held {
        return Half::Absent;
    }
    match connections
        .iter()
        .find(|connection| connection.provider == backend)
        .map(|connection| connection.status)
    {
        Some(ConnectionStatus::NeedsReauth) => Half::Flagged,
        Some(ConnectionStatus::Revoked) => Half::Revoked,
        Some(ConnectionStatus::Active) | None => Half::Live,
    }
}

/// Both halves of a pair whose grant serves beside its mirror, for one
/// athlete in one tenant.
#[derive(Debug, Clone, Copy)]
struct PairHalves {
    session: Half,
    grant: Half,
}

impl PairHalves {
    /// Read the session `mirror` and the grant `grant`. A token row or a
    /// connection list that cannot be read counts as held and live: the
    /// athlete keeps the credential they hold rather than lose it to a guess,
    /// and the read that follows reports what it finds.
    async fn read(
        repos: &RepositoryRegistry,
        user_id: Uuid,
        tenant_id: TenantId,
        grant: &str,
        mirror: &str,
    ) -> Self {
        let session_held = token_row_held(repos, user_id, tenant_id, mirror).await;
        let grant_held = token_row_held(repos, user_id, tenant_id, grant).await;
        let connections = if session_held || grant_held {
            repos
                .provider_connections
                .get_for_user(user_id, Some(tenant_id))
                .await
                .inspect_err(|e| {
                    warn!(
                        user_id = %user_id,
                        provider = grant,
                        error = %e,
                        "could not read the connection statuses; reading every held row as live"
                    );
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Self {
            session: half(session_held, &connections, mirror),
            grant: half(grant_held, &connections, grant),
        }
    }
}

/// Where an athlete's reads of a provider go: the backend that serves them,
/// and the one that answers in its place when it has an unexpected error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPlan {
    /// The backend every read of the provider goes to first.
    pub primary: String,
    /// The provider's live OAuth grant standing behind a session that serves
    /// as the primary; `None` when no live grant stands behind it.
    pub fallback: Option<String>,
}

impl ReadPlan {
    /// A plan served by `backend` alone.
    fn alone(backend: &str) -> Self {
        Self {
            primary: backend.to_owned(),
            fallback: None,
        }
    }

    /// The plan's backends, the primary first.
    pub fn backends(&self) -> impl Iterator<Item = &str> {
        iter::once(self.primary.as_str()).chain(self.fallback.as_deref())
    }

    /// Whether `backend` serves in this plan, as its primary or its fallback.
    #[must_use]
    pub fn serves(&self, backend: &str) -> bool {
        self.backends().any(|served| served == backend)
    }

    /// The first of the plan's backends whose descriptor in `registry`
    /// carries `capability`, or `None` when neither does.
    #[must_use]
    pub fn carrying(
        &self,
        registry: &ProviderRegistry,
        capability: ProviderCapabilities,
    ) -> Option<&str> {
        self.backends().find(|backend| {
            registry
                .get_capabilities(backend)
                .is_some_and(|capabilities| capabilities.contains(capability))
        })
    }
}

/// The athlete's read plan for `provider` in this tenant.
///
/// The one rule [`resolve_backend`], [`coalesced_status`], the connect cards
/// and the wellness sync read, so routing and every status surface agree
/// (carnet#1302, carnet#1494).
///
/// For a pair whose own OAuth backend serves beside its mirror (Strava,
/// Garmin), in order:
/// 1. a session that is not revoked (active or `needs_reauth`) is the
///    primary, and the grant its fallback while the grant's connection is
///    active;
/// 2. else a live grant serves alone;
/// 3. else the mirror, so a dead grant's read raises the hosted sign-in, and
///    an athlete with nothing connects through it ([`mirror_is_default`]).
///
/// TrainingPeaks and COROS are their mirror alone, and a provider with no
/// mirror is itself alone. Accepts either half of a pair: both name one plan.
pub async fn read_plan(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> ReadPlan {
    let user_facing = user_facing_name(provider);
    let Some(mirror) = mirror_backend_for(user_facing) else {
        return ReadPlan::alone(provider);
    };
    if !oauth_serves(user_facing) {
        return ReadPlan::alone(mirror);
    }
    let halves = PairHalves::read(repos, user_id, tenant_id, user_facing, mirror).await;
    if halves.session.signed_in() {
        return ReadPlan {
            primary: mirror.to_owned(),
            fallback: (halves.grant == Half::Live).then(|| user_facing.to_owned()),
        };
    }
    if halves.grant == Half::Live || !mirror_is_default(user_facing) {
        return ReadPlan::alone(user_facing);
    }
    ReadPlan::alone(mirror)
}

/// Whether `backend` is an OAuth grant standing beside a usr/pwd session of
/// its provider that is not revoked, in this tenant.
///
/// The session is then the provider's primary ([`read_plan`]), so a walk of
/// the athlete's connections reads the session and passes over the grant:
/// the API is never read beside the scrape. `false` for a mirror, for a
/// provider whose OAuth backend never serves beside its mirror, and for a
/// grant with no session beside it, a dead grant alone included, whose flag
/// is the athlete's reconnect.
pub async fn grant_beside_session(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    let Some(mirror) = mirror_backend_for(backend) else {
        return false;
    };
    oauth_serves(backend)
        && PairHalves::read(repos, user_id, tenant_id, backend, mirror)
            .await
            .session
            .signed_in()
}

/// The live OAuth grant that keeps `backend` syncing when `backend` is the
/// athlete's session for its provider, else `None`.
///
/// It is the session's [`read_plan`] fallback: a session that is not revoked
/// serves first, and a live grant answers when it cannot. A session that
/// dies with such a grant beside it is still told, once, that the provider
/// keeps syncing for now and that a fresh sign-in keeps it that way
/// (carnet#1494). `None` for a grant, a provider with no mirror, and a
/// session with no live grant beside it.
pub async fn fallback_grant(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> Option<String> {
    if !is_mirror_backend(backend) {
        return None;
    }
    let plan = read_plan(repos, user_id, tenant_id, backend).await;
    if plan.primary == backend {
        plan.fallback
    } else {
        None
    }
}

/// The OAuth grant of `provider`'s pair when the athlete holds it dead in
/// this tenant (its connection `needs_reauth`: a refused refresh), else
/// `None`.
///
/// A dead grant reconnects only through its mirror's hosted sign-in
/// ([`reconnect_target`]), and a sign-in over it replaces it (carnet#1494).
/// `None` for a provider whose OAuth backend never serves beside its mirror,
/// for a live or absent grant, and when the store cannot be read: nothing is
/// rerouted or deleted on a guess. Accepts either half of a pair.
pub async fn flagged_grant(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> Option<&'static str> {
    let user_facing = user_facing_name(provider);
    mirror_backend_for(user_facing)?;
    let grant = OAUTH_SERVED
        .iter()
        .copied()
        .find(|served| *served == user_facing)?;
    if !has_token_row(repos, user_id, tenant_id, grant).await {
        return None;
    }
    let connections = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .inspect_err(|e| {
            warn!(
                user_id = %user_id,
                provider = grant,
                error = %e,
                "could not read the connection statuses; the grant reads as live"
            );
        })
        .ok()?;
    (half(true, &connections, grant) == Half::Flagged).then_some(grant)
}

/// Resolve a user-facing provider name to the backend that should actually
/// serve the request: its [`read_plan`]'s primary.
///
/// - `strava`, `garmin` → the session (`sciotte`, `sciotte_garmin`) when one
///   that is not revoked is held, else a live grant, else the session
/// - `trainingpeaks` → `sciotte_trainingpeaks` always (no API to call)
/// - `coros` → `sciotte_coros` always (its partner API is not approved)
/// - a mirror named directly stays that mirror, and any other name is
///   returned unchanged
///
/// A read that must reach one backend exactly whatever the plan (a Strava
/// webhook, the first read of a connection just made) never comes here.
pub async fn resolve_backend(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: Option<TenantId>,
    requested: &str,
) -> String {
    match tenant_id {
        Some(tid) if mirror_backend_for(requested).is_some() => {
            read_plan(repos, user_id, tid, requested).await.primary
        }
        _ => requested.to_owned(),
    }
}

/// The backend a write carrying `capability` (a calendar write, a planned
/// workout read) goes to when the athlete names `requested`.
///
/// Chosen within the pair by capability, never by routing alone: the first
/// backend of the athlete's [`read_plan`] that carries it, so a workout sent
/// to Garmin keeps going to the Garmin grant while the session serves the
/// reads. When neither does, the plan's primary, whose refusal names the
/// capability. A mirror named directly and a provider with no mirror are
/// returned unchanged, as [`resolve_backend`] returns them.
///
/// # Errors
///
/// The [`grant_owed_refusal`] when only the pair's grant carries
/// `capability` and the grant is [`grant_owed`] beside the athlete's session
/// (JF's directive 8): the write is never sent to a session that cannot make
/// it, and the refusal names the authorization to connect again. An athlete
/// holding neither half gets the plan's primary, whose refusal says the
/// provider is not connected.
pub async fn write_backend(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    requested: &str,
    capability: ProviderCapabilities,
) -> AppResult<String> {
    if mirror_backend_for(requested).is_none() {
        return Ok(requested.to_owned());
    }
    let plan = read_plan(repos, user_id, tenant_id, requested).await;
    if let Some(backend) = plan.carrying(registry, capability) {
        return Ok(backend.to_owned());
    }
    match grant_owed(repos, registry, user_id, tenant_id, requested).await {
        Some(grant)
            if registry
                .get_capabilities(grant)
                .is_some_and(|carried| carried.contains(capability)) =>
        {
            Err(grant_owed_refusal(registry, grant))
        }
        _ => Ok(plan.primary),
    }
}

/// What only a pair's own OAuth grant writes and its mirror cannot: the
/// training calendar (Garmin's workout and calendar push).
const GRANT_ONLY_WRITES: ProviderCapabilities = ProviderCapabilities::CALENDAR_WRITE;

/// `details.reason` of [`grant_owed_refusal`].
pub const GRANT_OWED_REASON: &str = "grant_owed";

/// The OAuth grant of `provider`'s pair when it writes what the pair's
/// mirror cannot, else `None`. Accepts either half of a pair.
///
/// Garmin's: its session reads everything the athlete records, and only its
/// grant pushes workouts and writes the calendar. So Garmin connects and
/// reconnects by its hosted sign-in first and its OAuth grant after it, and
/// the grant is owed while it is not live (JF's directive 8). Strava's grant
/// writes nothing its session cannot, so it is never owed.
#[must_use]
pub fn grant_writes_beside_mirror(
    registry: &ProviderRegistry,
    provider: &str,
) -> Option<&'static str> {
    let user_facing = user_facing_name(provider);
    let mirror = mirror_backend_for(user_facing)?;
    let grant = OAUTH_SERVED
        .iter()
        .copied()
        .find(|served| *served == user_facing)?;
    let writes = |backend: &str| {
        registry
            .get_capabilities(backend)
            .map_or_else(ProviderCapabilities::empty, |carried| {
                carried.intersection(GRANT_ONLY_WRITES)
            })
    };
    (!writes(grant).difference(writes(mirror)).is_empty()).then_some(grant)
}

/// The grant `provider`'s athlete still owes for its writes in this tenant.
///
/// A [`grant_writes_beside_mirror`] grant whose half is not live (absent,
/// flagged or revoked) while the athlete is signed in to the pair's session
/// (held and not revoked). `None` when the provider has no such grant, the
/// athlete's grant is live, or no session is held: an athlete with no
/// session connects by the hosted sign-in first, which then goes on to the
/// grant (JF's directive 8), so nothing points them at the grant alone. A
/// store that cannot be read reads the grant as live: nothing is asked on a
/// guess.
pub async fn grant_owed(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> Option<&'static str> {
    let grant = grant_writes_beside_mirror(registry, provider)?;
    let mirror = mirror_backend_for(grant)?;
    let halves = PairHalves::read(repos, user_id, tenant_id, grant, mirror).await;
    (halves.session.signed_in() && halves.grant != Half::Live).then_some(grant)
}

/// Whether `backend` is a [`grant_writes_beside_mirror`] grant standing
/// beside a session that serves its provider: held, and its connection
/// active, in this tenant.
///
/// Such a grant's death, or its withdrawal, leaves the reads going and the
/// writes only it makes owed, so its athlete is told that much and given the
/// grant's own authorization (JF's directive 8). A flagged session does not
/// serve, though it still routes the reads to its sign-in: "your activities
/// are still syncing" would then be false. `false` for a mirror, for a grant
/// that writes nothing its mirror cannot (Strava's), and for a grant with no
/// serving session beside it, whose notice is the ordinary reconnect. A store
/// that cannot be read reads every held row as live, as [`read_plan`] reads
/// it.
pub async fn grant_writes_beside_serving_session(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    let Some(grant) = grant_writes_beside_mirror(registry, backend) else {
        return false;
    };
    let Some(mirror) = mirror_backend_for(grant) else {
        return false;
    };
    grant == backend
        && PairHalves::read(repos, user_id, tenant_id, grant, mirror)
            .await
            .session
            == Half::Live
}

/// The refusal of a write only `grant`, the pair's own authorization, makes
/// while the athlete's grant is not live.
///
/// `NoProviderConnected`, which reaches the model verbatim, with
/// `details.action` `connect_provider`, `details.reason`
/// [`GRANT_OWED_REASON`] and `details.provider` the grant: the caller that
/// can mint the grant's authorization link adds it.
#[must_use]
pub fn grant_owed_refusal(registry: &ProviderRegistry, grant: &str) -> AppError {
    let display = registry.get_display_name(grant).unwrap_or(grant);
    let mut refusal = AppError::new(
        ErrorCode::NoProviderConnected,
        format!(
            "{display} takes workouts and calendar writes only through its own authorization \
             ({grant}), which the athlete has not connected or must connect again. Their \
             {display} sign-in keeps the reads coming; this write needs {display} connected \
             again through its authorization."
        ),
    );
    refusal.details = Some(Box::new(serde_json::json!({
        "action": "connect_provider",
        "reason": GRANT_OWED_REASON,
        "provider": grant,
    })));
    refusal
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
/// For Strava and Garmin the backend [`read_plan`] serves first is the one
/// reported: a session that is not revoked is `mirror`, whatever grant
/// stands behind it, so its flag is the card's reconnect. Without one, a
/// grant is `oauth` — live it serves, flagged its reconnect is the card's
/// to show — and a revoked session alone is still `mirror`. For every other
/// mirror-backed provider (TrainingPeaks, COROS) the mirror wins when
/// present — even if stale — because the user's stated preference is to keep
/// using it. With no credential of the user's own, a confirmed link their
/// group coach reads one of the provider's backends through
/// ([`BackendKind::Delegated`]) connects it, found the way the read path
/// finds it, so the status and the read cannot disagree. A link that cannot
/// be read counts as none, as an unreadable token row does.
pub async fn coalesced_status(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    user_facing: &'static str,
) -> CoalescedStatus {
    let held = |backend_kind| CoalescedStatus {
        user_facing,
        connected: true,
        backend_kind,
        delegation: None,
    };
    if let Some(mirror) = mirror_backend_for(user_facing) {
        if oauth_serves(user_facing) {
            let halves = PairHalves::read(repos, user_id, tenant_id, user_facing, mirror).await;
            if halves.session.signed_in() {
                return held(BackendKind::Mirror);
            }
            if halves.grant != Half::Absent {
                return held(BackendKind::Oauth);
            }
            if halves.session != Half::Absent {
                return held(BackendKind::Mirror);
            }
        } else if has_token_row(repos, user_id, tenant_id, mirror).await {
            return held(BackendKind::Mirror);
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

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_auth::connect_order::CONNECT_ORDER_PROVIDERS;
    use pierre_core::models::ConnectionType;

    /// A session's window serves its grant's key too, a Garmin grant's
    /// window serves its session's, and every other backend has one key
    /// (carnet#1494, carnet#1386).
    #[test]
    fn a_session_serves_its_grants_rows_and_a_push_fed_grant_its_sessions() {
        assert_eq!(
            history_keys(oauth_providers::SCIOTTE),
            vec![oauth_providers::SCIOTTE, oauth_providers::STRAVA],
            "bug: the Strava session's window drops the rows the API stored"
        );
        assert_eq!(
            history_keys(oauth_providers::SCIOTTE_GARMIN),
            vec![oauth_providers::SCIOTTE_GARMIN, oauth_providers::GARMIN],
            "bug: the Garmin session's window drops the grant's pushes"
        );
        assert_eq!(
            history_keys(oauth_providers::GARMIN),
            vec![oauth_providers::GARMIN, oauth_providers::SCIOTTE_GARMIN],
            "bug: a Garmin grant's window drops the history its session imported"
        );
        assert_eq!(
            history_keys(oauth_providers::STRAVA),
            vec![oauth_providers::STRAVA],
            "bug: a Strava grant's window serves only its own key"
        );
        for single in [
            oauth_providers::COROS,
            oauth_providers::SCIOTTE_COROS,
            oauth_providers::SCIOTTE_TRAININGPEAKS,
            oauth_providers::WHOOP,
        ] {
            assert_eq!(
                history_keys(single),
                vec![single],
                "bug: {single} has no grant beside it and one key"
            );
        }
    }

    /// A held row's half reads its connection's status; a row with no
    /// connection was never flagged, and no row is absent whatever a stray
    /// connection says.
    #[test]
    fn a_half_reads_its_connections_status() {
        let connection = |provider: &str, status| ProviderConnection {
            id: "id".to_owned(),
            user_id: Uuid::nil(),
            tenant_id: Uuid::nil().to_string(),
            provider: provider.to_owned(),
            connection_type: ConnectionType::Manual,
            connected_at: Utc::now(),
            last_used_at: None,
            status,
            metadata: None,
            account_role: None,
            status_changed_at: None,
        };
        let connections = [
            connection(oauth_providers::SCIOTTE, ConnectionStatus::NeedsReauth),
            connection(oauth_providers::STRAVA, ConnectionStatus::Revoked),
            connection(oauth_providers::GARMIN, ConnectionStatus::Active),
        ];
        assert_eq!(
            half(true, &connections, oauth_providers::SCIOTTE),
            Half::Flagged
        );
        assert_eq!(
            half(true, &connections, oauth_providers::STRAVA),
            Half::Revoked
        );
        assert_eq!(
            half(true, &connections, oauth_providers::GARMIN),
            Half::Live
        );
        assert_eq!(
            half(true, &connections, oauth_providers::SCIOTTE_GARMIN),
            Half::Live,
            "bug: a token row no connection flagged reads as dead"
        );
        assert_eq!(
            half(false, &connections, oauth_providers::GARMIN),
            Half::Absent,
            "bug: a connection with no token row reads as a credential"
        );
        assert!(Half::Flagged.signed_in() && Half::Live.signed_in());
        assert!(!Half::Revoked.signed_in() && !Half::Absent.signed_in());
    }

    /// Only Strava's own API reads on request; the health sync keeps pulling
    /// every other mirror-backed provider through its session, and Strava
    /// through nothing.
    #[test]
    fn only_stravas_api_reads_on_request() {
        assert!(api_reads_on_request(oauth_providers::STRAVA));
        assert!(api_reads_on_request(oauth_providers::SCIOTTE));
        for provider in [
            oauth_providers::GARMIN,
            oauth_providers::COROS,
            oauth_providers::TRAININGPEAKS,
            oauth_providers::WHOOP,
        ] {
            assert!(
                !api_reads_on_request(provider),
                "bug: {provider}'s API reads nothing on request"
            );
        }
        assert_eq!(
            sync_backend(oauth_providers::STRAVA),
            oauth_providers::STRAVA
        );
        assert_eq!(
            sync_backend(oauth_providers::SCIOTTE),
            oauth_providers::STRAVA
        );
        assert_eq!(
            sync_backend(oauth_providers::GARMIN),
            oauth_providers::SCIOTTE_GARMIN
        );
    }

    /// `pushed_backend_for` names the push-fed grant beside a session:
    /// Garmin's session has one, Strava's has none, and only the Garmin
    /// grant's window serves its session's rows.
    #[test]
    fn only_the_garmin_session_has_a_pushed_grant() {
        assert_eq!(
            pushed_backend_for(oauth_providers::SCIOTTE_GARMIN),
            Some(oauth_providers::GARMIN),
            "bug: the Garmin session's grant is the Garmin push"
        );
        assert_eq!(
            pushed_backend_for(oauth_providers::SCIOTTE),
            None,
            "bug: Strava's session has no push-fed grant"
        );
        assert_eq!(
            pushed_backend_for(oauth_providers::SCIOTTE_COROS),
            None,
            "bug: COROS has no push-fed grant"
        );
        assert!(
            history_keys(oauth_providers::GARMIN).contains(&oauth_providers::SCIOTTE_GARMIN),
            "bug: a Garmin grant's window serves its session's copy"
        );
        assert!(
            !history_keys(oauth_providers::STRAVA).contains(&oauth_providers::SCIOTTE),
            "bug: a Strava grant never serves its session's rows"
        );
    }

    /// The mirror of a user-facing provider and back.
    #[test]
    fn a_mirror_maps_to_its_user_facing_provider_and_back() {
        assert_eq!(
            mirror_backend_for(oauth_providers::GARMIN),
            Some(oauth_providers::SCIOTTE_GARMIN),
            "bug: Garmin's mirror is sciotte_garmin"
        );
        assert_eq!(
            user_facing_name(oauth_providers::SCIOTTE_GARMIN),
            oauth_providers::GARMIN,
            "bug: sciotte_garmin is the athlete's Garmin"
        );
        assert!(
            is_mirror_backend(oauth_providers::SCIOTTE_GARMIN),
            "bug: sciotte_garmin is the Garmin mirror"
        );
        assert!(
            !is_mirror_backend(oauth_providers::GARMIN),
            "bug: the Garmin grant is no mirror"
        );
    }

    /// A dead grant reconnects through its mirror's hosted sign-in, as a dead
    /// session does; a provider with no mirror through its own authorization.
    #[test]
    fn every_half_of_a_pair_reconnects_through_its_mirrors_sign_in() {
        for (provider, target) in [
            (oauth_providers::STRAVA, "strava"),
            (oauth_providers::SCIOTTE, "strava"),
            (oauth_providers::GARMIN, "garmin"),
            (oauth_providers::SCIOTTE_GARMIN, "garmin"),
            (oauth_providers::TRAININGPEAKS, "trainingpeaks"),
            (oauth_providers::SCIOTTE_COROS, "coros"),
        ] {
            assert_eq!(
                reconnect_target(provider),
                Some(target),
                "bug: {provider} reconnects through another sign-in"
            );
        }
        assert_eq!(
            reconnect_target(oauth_providers::WHOOP),
            None,
            "bug: a provider with no mirror is sent to a sign-in it has none of"
        );
    }

    /// The connect order is set for every provider with an account sign-in
    /// (directive 5), and for nothing else: the two lists are one table.
    #[test]
    fn every_hosted_sign_in_carries_a_connect_order() {
        assert_eq!(
            hosted_login_targets(),
            CONNECT_ORDER_PROVIDERS,
            "bug: a provider with a sign-in carries no connect order, or one without carries one"
        );
    }
}
