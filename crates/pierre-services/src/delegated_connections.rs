// ABOUTME: A group's coach links a coaching-platform roster athlete to a member, the member confirms, either side ends it
// ABOUTME: Reads the coach's roster through their own TrainingPeaks session or Intervals.icu key, enforces each step and tells the other side
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Delegated connections
//!
//! A coaching platform ([`CoachPlatform`]: `TrainingPeaks`, Intervals.icu)
//! lets a coach's own account read the athletes who share with it. A group's
//! human coach links one of those roster athletes to a live member of the
//! group ([`propose`]); the member confirms ([`confirm`]), and that
//! confirmation is their consent to having their workouts on that platform
//! read through the coach's account. Either side can end the link ([`end`]);
//! the group lifecycle and a disconnect end it through [`DelegationStore`].
//!
//! The roster comes from the coach's stored credential ([`coach_roster`]),
//! and only once every precondition holds: a connection of the coach's own
//! to the platform, live, whose account email is the coach's verified Dravr
//! email, whose owner accepted the platform's current notice where it asks
//! for one, plus what the platform itself requires of the account
//! ([`CoachPlatform::read_roster`]). Each refusal carries its reason in
//! `details.reason`, which is what a client branches on, and the platform in
//! `details.provider`.
//!
//! A link binds a roster athlete to a member by email: the coach proposes
//! only an athlete whose platform email the roster lists and which is the
//! member's verified Dravr email, and the member's confirm checks it again,
//! so the member consents to reading their own workouts and no one else's.
//! Every read through a confirmed link checks it once more
//! ([`link_binding`]), and a link that no longer binds reads nothing.
//!
//! Each step reaches the other side through the notification feed, the linked
//! chat channels and push: the member is asked, the coach hears the answer,
//! and both hear when the platform drops the athlete from the coach's roster
//! ([`end_off_roster`]). Notices are best-effort: a step stands even when its
//! notice cannot be addressed.
//!
//! The roster's athlete names are the provider's text about third parties.
//! They are stored and shown to the two people a link concerns, and never
//! placed in a notice or anything a model reads.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{
    ConnectionType, DelegatedConnection, DelegationEndReason, DelegationStatus,
    ProviderAccountRole, RosterAthlete, TenantId, User, UserOAuthToken,
};
use pierre_database::RepositoryRegistry;
use pierre_groups::delegation::DelegationStore;
use pierre_notifications::triggers::{self, LinkPlatform};
use pierre_notifications::{NotificationService, TenantId as NoticeTenantId};
use pierre_providers::backend_resolver::user_facing_name;
use pierre_providers::registry::ProviderRegistry;
use serde_json::json;
use tracing::{info, warn};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::coach_platform::{coach_platform, CoachPlatform, RosterRead};
use crate::delegation_refusal::Refusal;
use crate::provider_notice::notice_in_force;
use crate::trainingpeaks_accounts::{email_binding, link_binding, EmailBinding};

/// One athlete on the coach's roster, as the linking picker shows it.
#[derive(Debug, Clone)]
pub struct RosterEntry {
    /// The athlete as the coach's platform roster lists them.
    pub athlete: RosterAthlete,
    /// The live link that holds this athlete in the group, when there is one.
    pub link: Option<DelegatedConnection>,
    /// The live, unlinked member whose name reads as the athlete's roster
    /// name (ignoring case and accents), when exactly one does. A hint that
    /// preselects the picker; it never proposes on its own.
    pub suggested_member_user_id: Option<Uuid>,
}

/// What a coach asks to link: which roster athlete is which member.
#[derive(Debug, Clone, Copy)]
pub struct Proposal<'a> {
    /// The coaching platform, as the user knows it (`trainingpeaks`,
    /// `intervals_icu`).
    pub provider: &'a str,
    /// The athlete's id on the coach's platform roster.
    pub provider_athlete_id: &'a str,
    /// The live group member the athlete is.
    pub member_user_id: Uuid,
}

/// What the linking steps that read a coach's roster work with.
#[derive(Clone, Copy)]
pub struct DelegationServices<'a> {
    /// Repositories every step reads and writes.
    pub repos: &'a RepositoryRegistry,
    /// Builds the provider an API-backed platform reads the roster through.
    pub registry: &'a ProviderRegistry,
    /// Holds each coach's roster between live reads.
    pub cache: &'a Cache,
    /// Tells the other side of a step; `None` tells no one.
    pub notifications: Option<&'a Arc<NotificationService>>,
}

/// The name a person reads for another user: their display name, else their
/// email.
#[must_use]
pub fn person_name(user: &User) -> String {
    user.display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&user.email)
        .to_owned()
}

/// The athletes on the coach's `platform` roster, read through the coach's
/// own stored credential in `coach_tenant`.
///
/// Refused, with its reason, until the coach has a live connection of their
/// own whose account the platform accepts for a roster
/// ([`CoachPlatform::read_roster`]: a coach account whose email is the
/// coach's verified Dravr email) and has accepted the platform's current
/// notice where one is in force. The roster is cached for ten minutes per
/// coach, tenant and platform, and served from the cache only once the
/// platform says nothing is left to learn by reading it live
/// ([`CoachPlatform::serves_cached_roster`]). `refresh` reads it live too.
/// Only a roster read through a bound account is cached; the cache is
/// dropped whenever the coach's credential is stored anew or cleared
/// ([`forget_coach_roster`]).
///
/// A live read also ends every live link, proposed or confirmed, the coach
/// holds on the platform for an athlete the roster no longer lists, telling
/// both sides as [`end_off_roster`] does, before the roster is returned: what
/// the coach reads and what the member is asked agree. A roster served from
/// the cache ends nothing.
///
/// # Errors
///
/// Returns an invalid-input error carrying the refusal reason, the provider
/// error when the roster cannot be read, or a repository error.
pub async fn coach_roster(
    services: DelegationServices<'_>,
    platform: &dyn CoachPlatform,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    refresh: bool,
) -> AppResult<Vec<RosterAthlete>> {
    let repos = services.repos;
    let (token, recorded_role) =
        coach_session(repos, platform, coach_user_id, coach_tenant).await?;
    let key = roster_cache_key(coach_user_id, coach_tenant, platform.backend());
    if !refresh && platform.serves_cached_roster(recorded_role) {
        match services.cache.get::<Vec<RosterAthlete>>(&key).await {
            Ok(Some(athletes)) => return Ok(athletes),
            Ok(None) => {}
            Err(e) => warn!(
                user_id = %coach_user_id,
                provider = platform.backend(),
                error = %e,
                "Coach roster cache read failed; reading the roster live"
            ),
        }
    }

    let athletes = platform
        .read_roster(RosterRead {
            repos,
            registry: services.registry,
            coach_user_id,
            coach_tenant,
            token: &token,
            recorded_role,
        })
        .await?;
    end_off_coach_roster(
        repos,
        services.notifications,
        platform,
        coach_user_id,
        coach_tenant,
        &athletes,
    )
    .await?;
    if let Err(e) = services
        .cache
        .set(
            &key,
            &athletes,
            CacheResource::ProviderRoster.recommended_ttl(),
        )
        .await
    {
        warn!(
            user_id = %coach_user_id,
            provider = platform.backend(),
            error = %e,
            "Coach roster cache write failed; the next read goes live"
        );
    }
    Ok(athletes)
}

/// The key the coach's `backend` roster is cached under in `coach_tenant`.
#[must_use]
pub fn roster_cache_key(coach_user_id: Uuid, coach_tenant: TenantId, backend: &str) -> CacheKey {
    CacheKey::new(
        coach_tenant,
        coach_user_id,
        backend.to_owned(),
        CacheResource::ProviderRoster,
    )
}

/// Drop the roster cached for `coach_user_id`'s `backend` credential in
/// `coach_tenant`.
///
/// Called when the credential it was read through was stored anew (another
/// login or key, possibly to another account) or cleared.
///
/// Best-effort: a failure is logged, and the stale roster ages out with its
/// ten-minute lifetime.
pub async fn forget_coach_roster(
    cache: &Cache,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    backend: &str,
) {
    if let Err(e) = cache
        .invalidate(&roster_cache_key(coach_user_id, coach_tenant, backend))
        .await
    {
        warn!(
            user_id = %coach_user_id,
            provider = backend,
            error = %e,
            "Coach roster cache could not be dropped; it ages out on its own"
        );
    }
}

/// The coach's stored `platform` credential, once every precondition the
/// link flow shares holds: a live connection of their own, not recorded as
/// an athlete's account, under the platform's current notice.
///
/// Returned with the account role recorded on their connection (`None`
/// while it was never read).
async fn coach_session(
    repos: &RepositoryRegistry,
    platform: &dyn CoachPlatform,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
) -> AppResult<(Box<UserOAuthToken>, Option<ProviderAccountRole>)> {
    let backend = platform.backend();
    let (token, account_role) =
        match coach_session_state(repos, coach_user_id, coach_tenant, backend).await? {
            CoachSession::Missing => return Err(Refusal::NotConnected.error(Some(platform))),
            CoachSession::NeedsReconnect => {
                return Err(Refusal::ReconnectNeeded.error(Some(platform)));
            }
            CoachSession::Live {
                token,
                account_role,
            } => (token, account_role),
        };
    if account_role == Some(ProviderAccountRole::Athlete) {
        return Err(Refusal::NotCoachAccount.error(Some(platform)));
    }
    // A coach the notice is in force for reads their roster only under its
    // current version; one the flag leaves off was never asked for it.
    if let Some(current) =
        notice_in_force(repos, coach_tenant.as_uuid(), coach_user_id, backend).await
    {
        if repos
            .users
            .provider_terms_version(coach_user_id, backend)
            .await?
            .as_deref()
            != Some(current)
        {
            return Err(Refusal::TermsOutdated.error(Some(platform)));
        }
    }
    Ok((token, account_role))
}

/// Where a coach's own provider session stands: the one their roster is read
/// through, and every delegated read of a member they link.
#[derive(Debug, Clone)]
pub enum CoachSession {
    /// The coach holds no session of their own for the provider.
    Missing,
    /// The session is stored but expired, or its connection is flagged: only
    /// the coach can renew it by signing in again.
    NeedsReconnect,
    /// A session that still serves.
    Live {
        /// The coach's stored session row; its access token is the session.
        token: Box<UserOAuthToken>,
        /// The kind of account the session signed in to, when known.
        account_role: Option<ProviderAccountRole>,
    },
}

/// Where `coach_user_id`'s own `provider` session in `coach_tenant` stands.
///
/// Read from the coach's own rows, never a delegated connection: a session
/// is missing without a stored token, needs reconnecting when that token has
/// expired or the coach's connection is flagged, and is live otherwise.
/// Scraper sessions never refresh, so an expired one stays expired until the
/// coach signs in again.
///
/// # Errors
///
/// Returns the repository error when the token or the connection cannot be
/// read.
pub async fn coach_session_state(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    provider: &str,
) -> AppResult<CoachSession> {
    let Some(token) = repos
        .oauth_tokens
        .get_token(coach_user_id, coach_tenant, provider)
        .await?
    else {
        return Ok(CoachSession::Missing);
    };
    let connection = repos
        .provider_connections
        .get_for_user(coach_user_id, Some(coach_tenant))
        .await?
        .into_iter()
        .find(|c| c.provider == provider && c.connection_type != ConnectionType::Delegated);
    let expired = token.expires_at.is_some_and(|at| at <= Utc::now());
    if expired
        || connection
            .as_ref()
            .is_some_and(|c| c.status.requires_reauth())
    {
        return Ok(CoachSession::NeedsReconnect);
    }
    Ok(CoachSession::Live {
        token: Box::new(token),
        account_role: connection.and_then(|c| c.account_role),
    })
}

/// A member's live link as a status surface reports it: the provider card on
/// `/api/providers` and the connection-status tool.
#[derive(Debug, Clone)]
pub struct MemberDelegation {
    /// The link.
    pub link: DelegatedConnection,
    /// The group the link was made in.
    pub group_name: String,
    /// The coach's name: their display name, else their email.
    pub coach_name: String,
    /// Whether the coach's own session needs the coach to reconnect before
    /// the member's workouts can be read again.
    pub coach_needs_reconnect: bool,
    /// Why a confirmed link reads nothing ([`unbound_link_reason`]); `None`
    /// for a proposal and for a link that reads.
    pub read_refused: Option<&'static str>,
}

/// Why a confirmed link whose athlete binds as `binding` reads nothing.
///
/// `athlete_email_missing`, `athlete_email_mismatch` or
/// `member_email_unverified`, as the clients name it; `None` once it binds.
#[must_use]
pub fn unbound_link_reason(binding: EmailBinding) -> Option<&'static str> {
    Refusal::proposal(binding).map(Refusal::as_str)
}

/// The link `member_user_id`'s `provider` card shows: their confirmed link,
/// else their newest proposal; `None` when they have neither.
///
/// Only a link the group relation still backs is shown — the relation the
/// read path reads through — so the card never names a link a read would
/// refuse.
///
/// # Errors
///
/// Returns the repository error when the links, the group, the coach or the
/// coach's session cannot be read.
pub async fn member_delegation(
    repos: &RepositoryRegistry,
    member_user_id: Uuid,
    provider: &str,
) -> AppResult<Option<MemberDelegation>> {
    let shown = repos
        .delegated_connections
        .list_backed_for_member(member_user_id, provider)
        .await?
        .into_iter()
        .next();
    match shown {
        Some(link) => describe_link(repos, link).await,
        None => Ok(None),
    }
}

/// `link` as a status surface reports it, or `None` when its group or its
/// coach is gone. A confirmed link carries why it reads nothing, when it
/// does not ([`link_binding`]).
///
/// # Errors
///
/// Returns the repository error when the group, the coach, the coach's
/// session or the member's verification cannot be read.
pub async fn describe_link(
    repos: &RepositoryRegistry,
    link: DelegatedConnection,
) -> AppResult<Option<MemberDelegation>> {
    let Some(group) = repos
        .groups
        .get_group(&link.group_id.to_string(), link.coach_tenant_id)
        .await?
    else {
        return Ok(None);
    };
    let Some(coach) = repos.users.get_global(link.coach_user_id).await? else {
        return Ok(None);
    };
    let coach_needs_reconnect = !matches!(
        coach_session_state(
            repos,
            link.coach_user_id,
            link.coach_tenant_id,
            &link.provider
        )
        .await?,
        CoachSession::Live { .. }
    );
    let read_refused = if link.status == DelegationStatus::Confirmed {
        unbound_link_reason(link_binding(repos, &link).await?)
    } else {
        None
    };
    Ok(Some(MemberDelegation {
        group_name: group.name,
        coach_name: person_name(&coach),
        coach_needs_reconnect,
        read_refused,
        link,
    }))
}

/// The coach's `platform` roster as the linking picker for `group` shows it:
/// each athlete with the live link that holds them in the group, and the
/// member their roster name suggests.
///
/// # Errors
///
/// Returns the errors of [`coach_roster`], or a repository error.
pub async fn roster_for_group(
    services: DelegationServices<'_>,
    platform: &dyn CoachPlatform,
    group: &CoachingGroup,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    refresh: bool,
) -> AppResult<Vec<RosterEntry>> {
    let repos = services.repos;
    let athletes = coach_roster(services, platform, coach_user_id, coach_tenant, refresh).await?;
    let links: Vec<DelegatedConnection> = repos
        .delegated_connections
        .list_live_for_coach_in_group(group.id, coach_user_id)
        .await?
        .into_iter()
        .filter(|link| link.provider == platform.backend())
        .collect();
    let candidates: Vec<Uuid> = repos
        .groups
        .list_members(&group.id.to_string())
        .await?
        .into_iter()
        .map(|member| member.user_id)
        .filter(|id| *id != coach_user_id && !links.iter().any(|l| l.member_user_id == *id))
        .collect();
    let users = repos.users.get_global_many(&candidates).await?;
    let names: Vec<(Uuid, String)> = candidates
        .iter()
        .filter_map(|id| {
            let folded = fold_name(users.get(id)?.display_name.as_deref()?);
            (!folded.is_empty()).then_some((*id, folded))
        })
        .collect();

    Ok(athletes
        .into_iter()
        .map(|athlete| {
            let link = links
                .iter()
                .find(|l| l.provider_athlete_id == athlete.id)
                .cloned();
            let suggested_member_user_id = if link.is_none() {
                athlete
                    .name
                    .as_deref()
                    .and_then(|name| sole_match(&names, &fold_name(name)))
            } else {
                None
            };
            RosterEntry {
                athlete,
                link,
                suggested_member_user_id,
            }
        })
        .collect())
}

/// A name folded for comparison: decomposed, accents dropped, lowercased,
/// whitespace collapsed.
fn fold_name(name: &str) -> String {
    let folded: String = name
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The one candidate whose folded name is `folded`; `None` when none or
/// several are.
fn sole_match(names: &[(Uuid, String)], folded: &str) -> Option<Uuid> {
    if folded.is_empty() {
        return None;
    }
    let mut matches = names.iter().filter(|(_, name)| name == folded);
    let first = matches.next()?;
    matches.next().is_none().then_some(first.0)
}

/// The coach of `group` proposes linking a roster athlete to a live member.
/// The member is asked through a notice; nothing is read until they confirm.
///
/// # Errors
///
/// Returns an invalid-input error for an unsupported provider, a malformed
/// athlete id, an athlete off the roster, an athlete whose roster email is
/// missing or is not the member's verified email, a member whose email is
/// unverified, or the coach naming themselves; a
/// not-found error when the member is not a live member of the group; an
/// already-exists error (`already_proposed`, `athlete_already_linked`) when a
/// live link holds the member or the athlete; the roster's own refusals; or a
/// repository error.
pub async fn propose(
    services: DelegationServices<'_>,
    group: &CoachingGroup,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    proposal: Proposal<'_>,
) -> AppResult<DelegatedConnection> {
    let repos = services.repos;
    let platform = coach_platform(proposal.provider)
        .ok_or_else(|| Refusal::UnsupportedProvider.error(None))?;
    let backend = platform.backend();
    let athlete = proposal.provider_athlete_id;
    services
        .registry
        .check_delegated_athlete(backend, athlete)
        .map_err(|_| Refusal::InvalidAthlete.error(Some(platform)))?;
    let member = proposal.member_user_id;
    if member == coach_user_id {
        return Err(Refusal::MemberIsCoach.error(Some(platform)));
    }
    if repos
        .groups
        .get_member(&group.id.to_string(), member)
        .await?
        .is_none()
    {
        return Err(AppError::not_found("Group member"));
    }
    let roster = coach_roster(services, platform, coach_user_id, coach_tenant, false).await?;
    let Some(entry) = roster.into_iter().find(|a| a.id == athlete) else {
        return Err(Refusal::AthleteNotOnRoster.error(Some(platform)));
    };
    let binding = email_binding(repos, member, entry.email.as_deref()).await?;
    if let Some(refusal) = Refusal::proposal(binding) {
        return Err(refusal.error(Some(platform)));
    }

    let link = DelegatedConnection::propose(
        backend.to_owned(),
        group.id,
        coach_user_id,
        coach_tenant,
        member,
        entry,
    );
    let Some(stored) = repos.delegated_connections.propose(&link).await? else {
        let member_held = repos
            .delegated_connections
            .list_live_for_member_in_group(group.id, member)
            .await?
            .iter()
            .any(|live| live.provider == backend);
        return Err(if member_held {
            Refusal::AlreadyProposed
        } else {
            Refusal::AthleteAlreadyLinked
        }
        .error(Some(platform)));
    };
    info!(
        link_id = %stored.id,
        group_id = %group.id,
        coach_user_id = %coach_user_id,
        member_user_id = %member,
        provider = backend,
        "Delegated link proposed"
    );
    Notices::new(repos, services.notifications)
        .proposed(&stored, &group.name)
        .await;
    Ok(stored)
}

/// The member confirms `link`, a proposal naming them, in `member_tenant`.
///
/// Their delegated connection is registered there, their other live
/// proposals for the provider are superseded, and the coach is told.
///
/// # Errors
///
/// Returns an already-exists error when the member connects the platform
/// with a login of their own here (`own_connection`) or already has a
/// confirmed link (`already_linked`); an invalid-input error when the athlete
/// the link names is not the member by email: the link carries no roster
/// email, it is not the member's, or the member's email is unverified; a
/// not-found error when `link` is no longer a proposal; or a repository
/// error.
pub async fn confirm(
    repos: &RepositoryRegistry,
    notifications: Option<&Arc<NotificationService>>,
    group: &CoachingGroup,
    link: &DelegatedConnection,
    member_tenant: TenantId,
) -> AppResult<DelegatedConnection> {
    let member = link.member_user_id;
    let platform = coach_platform(&link.provider);
    if holds_own_connection(repos, member, member_tenant, &link.provider).await? {
        return Err(Refusal::OwnConnection.error(platform));
    }
    let binding = email_binding(repos, member, link.provider_athlete_email.as_deref()).await?;
    if let Some(refusal) = Refusal::confirmation(binding) {
        return Err(refusal.error(platform));
    }
    let confirmed = repos
        .delegated_connections
        .confirm(link.id, member, member_tenant, Utc::now())
        .await?
        .ok_or_else(|| AppError::not_found("Pending link"))?;
    supersede_other_proposals(repos, &confirmed).await?;

    repos
        .provider_connections
        .register_connection(
            member,
            member_tenant,
            &confirmed.provider,
            &ConnectionType::Delegated,
            Some(&json!({ "delegated_connection_id": confirmed.id }).to_string()),
        )
        .await?;
    info!(
        target: "notify",
        event = "provider.connected",
        provider = %user_facing_name(&confirmed.provider),
        backend = %confirmed.provider,
        delegated = true,
        user_id = %member,
        tenant_id = %member_tenant,
        "member confirmed a delegated connection"
    );
    Notices::new(repos, notifications)
        .confirmed(&confirmed, &group.name)
        .await;
    Ok(confirmed)
}

/// Whether the member reads the provider through a login of their own in
/// `tenant`: their own session, or a connection that is not a delegated one.
async fn holds_own_connection(
    repos: &RepositoryRegistry,
    member: Uuid,
    tenant: TenantId,
    provider: &str,
) -> AppResult<bool> {
    if repos
        .oauth_tokens
        .get_token(member, tenant, provider)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    Ok(repos
        .provider_connections
        .get_for_user(member, Some(tenant))
        .await?
        .iter()
        .any(|c| c.provider == provider && c.connection_type != ConnectionType::Delegated))
}

/// End the member's other live proposals for the confirmed link's provider:
/// they read it through one link at a time.
async fn supersede_other_proposals(
    repos: &RepositoryRegistry,
    confirmed: &DelegatedConnection,
) -> AppResult<()> {
    let member = confirmed.member_user_id;
    let store = DelegationStore::new(repos);
    for other in repos
        .delegated_connections
        .list_live_for_member(member)
        .await?
        .into_iter()
        .filter(|l| {
            l.id != confirmed.id
                && l.provider == confirmed.provider
                && l.status == DelegationStatus::Proposed
        })
    {
        store
            .end_one(
                other.id,
                member,
                Some(member),
                DelegationEndReason::Superseded,
            )
            .await?;
    }
    Ok(())
}

/// End the confirmed `backend` link a member's reads went through, now that
/// they connect the platform with a credential of their own: their own
/// connection takes its place.
///
/// Runs before the member's own connection is registered. That connection
/// lands on the same row, and would otherwise silently turn the delegated
/// connection into their own while the link still read as confirmed; ending
/// it first releases the delegated connection, so the own connection lands
/// on a clean slate.
///
/// # Errors
///
/// Returns the repository error when the link cannot be ended or its
/// delegated connection cannot be removed.
pub async fn supersede_delegated_link(
    repos: &RepositoryRegistry,
    member_user_id: Uuid,
    tenant: TenantId,
    backend: &str,
) -> AppResult<()> {
    let ended = DelegationStore::new(repos)
        .end_confirmed_for_member(
            member_user_id,
            tenant,
            backend,
            Some(member_user_id),
            DelegationEndReason::Superseded,
        )
        .await?;
    if !ended.is_empty() {
        info!(
            user_id = %member_user_id,
            tenant_id = %tenant,
            provider = backend,
            "A connection of the member's own superseded their coach link"
        );
    }
    Ok(())
}

/// End `link` for `caller`, its coach or its member.
///
/// The reason follows from who ends it and where the link stands: a member
/// declines a proposal and revokes a confirmed link; a coach withdraws a
/// proposal and revokes a confirmed link. A decline is told to the coach.
///
/// # Errors
///
/// Returns a not-found error when `link` has already ended, or a repository
/// error.
pub async fn end(
    repos: &RepositoryRegistry,
    notifications: Option<&Arc<NotificationService>>,
    group: &CoachingGroup,
    link: &DelegatedConnection,
    caller: Uuid,
) -> AppResult<DelegatedConnection> {
    let by_member = caller == link.member_user_id;
    let reason = match (link.status, by_member) {
        (DelegationStatus::Proposed, true) => DelegationEndReason::Declined,
        (DelegationStatus::Proposed, false) => DelegationEndReason::Withdrawn,
        (DelegationStatus::Confirmed, true) => DelegationEndReason::RevokedByMember,
        (DelegationStatus::Confirmed, false) => DelegationEndReason::RevokedByCoach,
        (DelegationStatus::Revoked, _) => {
            return Err(AppError::not_found("Live link"));
        }
    };
    let ended = DelegationStore::new(repos)
        .end_one(link.id, caller, Some(caller), reason)
        .await?
        .ok_or_else(|| AppError::not_found("Live link"))?;
    info!(
        link_id = %ended.id,
        group_id = %group.id,
        provider = %ended.provider,
        reason = reason.as_str(),
        "Delegated link ended"
    );
    if reason == DelegationEndReason::Declined {
        Notices::new(repos, notifications)
            .declined(&ended, &group.name)
            .await;
    }
    Ok(ended)
}

/// End the member's confirmed link because the coach's roster dropped them.
///
/// The provider no longer lists the athlete on the coach's roster, so the
/// member's `provider` link in `member_tenant` ends, and both sides are told:
/// the coach that the athlete left their roster, the member that their
/// workouts are no longer read through the coach's account.
///
/// # Errors
///
/// Returns the repository error when the link cannot be ended or its
/// delegated connection cannot be removed.
pub async fn end_off_roster(
    repos: &RepositoryRegistry,
    notifications: Option<&Arc<NotificationService>>,
    member_user_id: Uuid,
    member_tenant: TenantId,
    provider: &str,
) -> AppResult<Vec<DelegatedConnection>> {
    let ended = DelegationStore::new(repos)
        .end_confirmed_for_member(
            member_user_id,
            member_tenant,
            provider,
            None,
            DelegationEndReason::NotOnRoster,
        )
        .await?;
    tell_off_roster(repos, notifications, &ended).await;
    Ok(ended)
}

/// End every live link, proposed or confirmed, that `coach_user_id`'s
/// `platform` credential in `coach_tenant` holds for an athlete `roster`,
/// just read live, no longer lists, telling both sides as [`end_off_roster`]
/// does.
///
/// A proposal whose athlete left the roster shows nowhere on the coach's
/// roster, so without this the coach could not withdraw it while it kept the
/// member from being linked again.
async fn end_off_coach_roster(
    repos: &RepositoryRegistry,
    notifications: Option<&Arc<NotificationService>>,
    platform: &dyn CoachPlatform,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    roster: &[RosterAthlete],
) -> AppResult<()> {
    let listed: Vec<&str> = roster.iter().map(|athlete| athlete.id.as_str()).collect();
    let ended = DelegationStore::new(repos)
        .end_off_coach_roster(coach_user_id, coach_tenant, platform.backend(), &listed)
        .await?;
    for link in &ended {
        info!(
            link_id = %link.id,
            group_id = %link.group_id,
            coach_user_id = %coach_user_id,
            member_user_id = %link.member_user_id,
            provider = %link.provider,
            "Delegated link ended: the coach's roster no longer lists its athlete"
        );
    }
    tell_off_roster(repos, notifications, &ended).await;
    Ok(())
}

/// Tell both sides of each link in `ended` that the provider dropped its
/// athlete from the coach's roster.
async fn tell_off_roster(
    repos: &RepositoryRegistry,
    notifications: Option<&Arc<NotificationService>>,
    ended: &[DelegatedConnection],
) {
    let notices = Notices::new(repos, notifications);
    for link in ended {
        notices.off_roster(link).await;
    }
}

/// Tells each side of a link what the other did, or what the provider did to
/// it. Best-effort: a notice whose recipient or names cannot be read is
/// logged, and the step it reports stands.
struct Notices<'a> {
    repos: &'a RepositoryRegistry,
    service: Option<&'a Arc<NotificationService>>,
}

/// The two people a link concerns, by the names each reads for the other.
struct Parties {
    coach_name: String,
    member_name: String,
}

impl<'a> Notices<'a> {
    const fn new(
        repos: &'a RepositoryRegistry,
        service: Option<&'a Arc<NotificationService>>,
    ) -> Self {
        Self { repos, service }
    }

    /// Ask the member to confirm, in the tenant they sign in to.
    async fn proposed(&self, link: &DelegatedConnection, group_name: &str) {
        let (Some(service), Some(parties)) = (self.service, self.parties(link).await) else {
            return;
        };
        let Some(member_tenant) = self.home_tenant(link.member_user_id).await else {
            return;
        };
        triggers::trigger_delegation_proposed(
            service,
            link.member_user_id,
            member_tenant,
            link_platform(link),
            &parties.coach_name,
            group_name,
        );
    }

    /// Tell the coach the member confirmed.
    async fn confirmed(&self, link: &DelegatedConnection, group_name: &str) {
        let (Some(service), Some(parties)) = (self.service, self.parties(link).await) else {
            return;
        };
        triggers::trigger_delegation_confirmed(
            service,
            link.coach_user_id,
            notice_tenant(link.coach_tenant_id),
            link_platform(link).name,
            link.member_user_id,
            &parties.member_name,
            group_name,
        );
    }

    /// Tell the coach the member declined.
    async fn declined(&self, link: &DelegatedConnection, group_name: &str) {
        let (Some(service), Some(parties)) = (self.service, self.parties(link).await) else {
            return;
        };
        triggers::trigger_delegation_declined(
            service,
            link.coach_user_id,
            notice_tenant(link.coach_tenant_id),
            link_platform(link).name,
            link.member_user_id,
            &parties.member_name,
            group_name,
        );
    }

    /// Tell both sides the provider dropped the athlete from the coach's
    /// roster, which ended the link.
    async fn off_roster(&self, link: &DelegatedConnection) {
        let (Some(service), Some(parties)) = (self.service, self.parties(link).await) else {
            return;
        };
        let group_name = match self
            .repos
            .groups
            .get_group(&link.group_id.to_string(), link.coach_tenant_id)
            .await
        {
            Ok(Some(group)) => group.name,
            Ok(None) => {
                warn!(link_id = %link.id, "Off-roster notice skipped: the link's group is gone");
                return;
            }
            Err(e) => {
                warn!(link_id = %link.id, error = %e, "Off-roster notice skipped: group read failed");
                return;
            }
        };
        triggers::trigger_delegation_off_roster(
            service,
            link.coach_user_id,
            notice_tenant(link.coach_tenant_id),
            link_platform(link).name,
            link.member_user_id,
            &parties.member_name,
            &group_name,
        );
        let member_tenant = match link.member_tenant_id {
            Some(tenant) => Some(notice_tenant(tenant)),
            None => self.home_tenant(link.member_user_id).await,
        };
        if let Some(member_tenant) = member_tenant {
            triggers::trigger_delegation_off_coach_roster(
                service,
                link.member_user_id,
                member_tenant,
                link_platform(link),
                &parties.coach_name,
                &group_name,
            );
        }
    }

    /// The coach's and the member's names, or `None` (logged) when either
    /// user cannot be read.
    async fn parties(&self, link: &DelegatedConnection) -> Option<Parties> {
        let users: HashMap<Uuid, User> = match self
            .repos
            .users
            .get_global_many(&[link.coach_user_id, link.member_user_id])
            .await
        {
            Ok(users) => users,
            Err(e) => {
                warn!(link_id = %link.id, error = %e, "Link notice skipped: user read failed");
                return None;
            }
        };
        let (Some(coach), Some(member)) = (
            users.get(&link.coach_user_id),
            users.get(&link.member_user_id),
        ) else {
            warn!(link_id = %link.id, "Link notice skipped: a party's account is gone");
            return None;
        };
        Some(Parties {
            coach_name: person_name(coach),
            member_name: person_name(member),
        })
    }

    /// The tenant a user signs in to by default, where their feed is read:
    /// the first of their tenants, as login picks it.
    async fn home_tenant(&self, user_id: Uuid) -> Option<NoticeTenantId> {
        match self.repos.tenants.list_for_user(user_id).await {
            Ok(tenants) => tenants.first().map(|tenant| notice_tenant(tenant.id)),
            Err(e) => {
                warn!(user_id = %user_id, error = %e, "Link notice skipped: tenant read failed");
                None
            }
        }
    }
}

/// The coaching platform `link` reads through, as a notice names it: the
/// card it opens and the brand its words carry.
fn link_platform(link: &DelegatedConnection) -> LinkPlatform<'_> {
    let provider = user_facing_name(&link.provider);
    LinkPlatform {
        provider,
        name: coach_platform(&link.provider).map_or(provider, |platform| platform.brand()),
    }
}

/// A platform tenant as the notification pipeline names it.
const fn notice_tenant(tenant: TenantId) -> NoticeTenantId {
    NoticeTenantId(tenant.as_uuid())
}
