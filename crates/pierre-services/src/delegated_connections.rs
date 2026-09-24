// ABOUTME: A group's coach links a TrainingPeaks roster athlete to a member, the member confirms, either side ends it
// ABOUTME: Reads the coach's roster through their own session, enforces each step's rules and tells the other side
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Delegated `TrainingPeaks` connections
//!
//! A `TrainingPeaks` coach account keeps no calendar of its own; its session
//! reads each athlete on the coach's roster. A group's human coach links one
//! of those roster athletes to a live member of the group ([`propose`]); the
//! member confirms ([`confirm`]), and that confirmation is their consent to
//! having their `TrainingPeaks` workouts read through the coach's account.
//! Either side can end the link ([`end`]); the group lifecycle and a
//! disconnect end it through [`DelegationStore`].
//!
//! The roster comes from the coach's stored session ([`coach_roster`]), and
//! only once every precondition holds: a `TrainingPeaks` connection of the
//! coach's own, live, signed in with a coach account, whose owner accepted the
//! current `TrainingPeaks` notice. Each refusal carries its reason in
//! `details.reason`, which is what a client branches on.
//!
//! Each step reaches the other side through the notification feed, the linked
//! chat channels and push: the member is asked, the coach hears the answer,
//! and both hear when `TrainingPeaks` drops the athlete from the coach's
//! roster ([`end_off_roster`]). Notices are best-effort: a step stands even
//! when its notice cannot be addressed.
//!
//! The roster's athlete names are the provider's text about third parties.
//! They are stored and shown to the two people a link concerns, and never
//! placed in a notice or anything a model reads.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::constants::oauth_providers::{
    SCIOTTE_TRAININGPEAKS, TRAININGPEAKS, TRAININGPEAKS_TERMS_VERSION,
};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{
    ConnectionType, DelegatedConnection, DelegationEndReason, DelegationStatus,
    ProviderAccountRole, TenantId, User, UserOAuthToken,
};
use pierre_database::RepositoryRegistry;
use pierre_groups::delegation::DelegationStore;
use pierre_notifications::{triggers, NotificationService, TenantId as NoticeTenantId};
use pierre_providers::backend_resolver::user_facing_name;
use pierre_providers::sciotte_remote::{AthleteId, AuthSession, CoachedAthlete};
use serde_json::json;
use tracing::{info, warn};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::trainingpeaks_accounts::{
    account_role, record_trainingpeaks_role, trainingpeaks_profile,
};

/// Why a linking step was refused. The wire form travels in the error's
/// `details.reason`; the message is what an API caller reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// The coach has no `TrainingPeaks` connection of their own here.
    NotConnected,
    /// The coach's `TrainingPeaks` account trains rather than coaches.
    NotCoachAccount,
    /// The coach's `TrainingPeaks` session is dead or flagged.
    ReconnectNeeded,
    /// The coach has not accepted the current `TrainingPeaks` notice.
    TermsOutdated,
    /// A provider other than `TrainingPeaks` was named.
    UnsupportedProvider,
    /// The athlete id is not one `TrainingPeaks` could have issued.
    InvalidAthlete,
    /// The coach's roster does not list the athlete.
    AthleteNotOnRoster,
    /// The coach named themselves as the member.
    MemberIsCoach,
    /// The member already has a live link in this group.
    AlreadyProposed,
    /// The athlete is already linked, in this group or another the coach coaches.
    AthleteAlreadyLinked,
    /// The member reads `TrainingPeaks` through a login of their own.
    OwnConnection,
}

impl Refusal {
    const fn as_str(self) -> &'static str {
        match self {
            Self::NotConnected => "trainingpeaks_not_connected",
            Self::NotCoachAccount => "trainingpeaks_not_coach_account",
            Self::ReconnectNeeded => "trainingpeaks_reconnect_needed",
            Self::TermsOutdated => "trainingpeaks_terms_outdated",
            Self::UnsupportedProvider => "unsupported_provider",
            Self::InvalidAthlete => "invalid_athlete",
            Self::AthleteNotOnRoster => "athlete_not_on_roster",
            Self::MemberIsCoach => "member_is_coach",
            Self::AlreadyProposed => "already_proposed",
            Self::AthleteAlreadyLinked => "athlete_already_linked",
            Self::OwnConnection => "own_connection",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::NotConnected => "Connect your TrainingPeaks coach account first",
            Self::NotCoachAccount => "This TrainingPeaks account is not a coach account",
            Self::ReconnectNeeded => "Reconnect TrainingPeaks to read your roster",
            Self::TermsOutdated => {
                "Reconnect TrainingPeaks and accept the updated notice to read your roster"
            }
            Self::UnsupportedProvider => "Only TrainingPeaks athletes can be linked",
            Self::InvalidAthlete => "That is not a TrainingPeaks athlete id",
            Self::AthleteNotOnRoster => "That athlete is not on your TrainingPeaks roster",
            Self::MemberIsCoach => "A coach cannot be linked as their own athlete",
            Self::AlreadyProposed => "This member already has a TrainingPeaks link in this group",
            Self::AthleteAlreadyLinked => "This TrainingPeaks athlete is already linked",
            Self::OwnConnection => "You already connect TrainingPeaks with your own account",
        }
    }

    const fn code(self) -> ErrorCode {
        match self {
            Self::AlreadyProposed | Self::AthleteAlreadyLinked | Self::OwnConnection => {
                ErrorCode::ResourceAlreadyExists
            }
            _ => ErrorCode::InvalidInput,
        }
    }

    fn error(self) -> AppError {
        let mut error = AppError::new(self.code(), self.message());
        error.details = Some(Box::new(json!({ "reason": self.as_str() })));
        error
    }
}

/// One athlete on the coach's roster, as the linking picker shows it.
#[derive(Debug, Clone)]
pub struct RosterEntry {
    /// The athlete as the coach's `TrainingPeaks` roster lists them.
    pub athlete: CoachedAthlete,
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
    /// The user-facing provider, `trainingpeaks`.
    pub provider: &'a str,
    /// The athlete's id on the coach's `TrainingPeaks` roster.
    pub provider_athlete_id: &'a str,
    /// The live group member the athlete is.
    pub member_user_id: Uuid,
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

/// The athletes on the coach's `TrainingPeaks` roster, read through the
/// coach's own stored session in `coach_tenant`.
///
/// Refused, with its reason, until the coach has a live connection of their
/// own, signed in with a coach account, and has accepted the current
/// `TrainingPeaks` notice. The roster is cached for ten minutes per coach and
/// tenant, and served from the cache only once the connection is recorded as
/// a coach account's: a connection whose role was never read is read live,
/// which records it. `refresh` reads it live too. A live read records the
/// account's role when it differs from the one recorded, as every
/// `TrainingPeaks` profile read does; the cache is dropped whenever the
/// coach's session is stored anew or cleared ([`forget_coach_roster`]).
///
/// A live read also ends every live link, proposed or confirmed, the coach
/// holds for an athlete the roster no longer lists, telling both sides as
/// [`end_off_roster`] does, before the roster is returned: what the coach
/// reads and what the member is asked agree. A roster served from the cache
/// ends nothing.
///
/// # Errors
///
/// Returns an invalid-input error carrying the refusal reason, the scraper
/// error when the profile cannot be read, or a repository error.
pub async fn coach_roster(
    repos: &RepositoryRegistry,
    cache: &Cache,
    notifications: Option<&Arc<NotificationService>>,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    refresh: bool,
) -> AppResult<Vec<CoachedAthlete>> {
    let (session, recorded_role) = coach_session(repos, coach_user_id, coach_tenant).await?;
    let key = roster_cache_key(coach_user_id, coach_tenant);
    if !refresh && recorded_role == Some(ProviderAccountRole::Coach) {
        match cache.get::<Vec<CoachedAthlete>>(&key).await {
            Ok(Some(athletes)) => return Ok(athletes),
            Ok(None) => {}
            Err(e) => warn!(
                user_id = %coach_user_id,
                error = %e,
                "TrainingPeaks roster cache read failed; reading the roster live"
            ),
        }
    }

    let profile = match trainingpeaks_profile(&session).await {
        Ok(profile) => profile,
        Err(e) if e.provider_auth_required_provider().is_some() => {
            return Err(Refusal::ReconnectNeeded.error());
        }
        Err(e) => return Err(e),
    };
    let role = account_role(&profile);
    if recorded_role != Some(role) {
        record_trainingpeaks_role(repos, coach_user_id, coach_tenant, role).await?;
    }
    if role != ProviderAccountRole::Coach {
        return Err(Refusal::NotCoachAccount.error());
    }

    let athletes = profile.coached_athletes;
    end_off_coach_roster(repos, notifications, coach_user_id, coach_tenant, &athletes).await?;
    if let Err(e) = cache
        .set(
            &key,
            &athletes,
            CacheResource::ProviderRoster.recommended_ttl(),
        )
        .await
    {
        warn!(
            user_id = %coach_user_id,
            error = %e,
            "TrainingPeaks roster cache write failed; the next read goes live"
        );
    }
    Ok(athletes)
}

/// The key the coach's roster is cached under in `coach_tenant`.
#[must_use]
pub fn roster_cache_key(coach_user_id: Uuid, coach_tenant: TenantId) -> CacheKey {
    CacheKey::new(
        coach_tenant,
        coach_user_id,
        SCIOTTE_TRAININGPEAKS.to_owned(),
        CacheResource::ProviderRoster,
    )
}

/// Drop the roster cached for `coach_user_id`'s `TrainingPeaks` session in
/// `coach_tenant`.
///
/// Called when the session it was read through was stored anew (another
/// login, possibly to another account) or cleared.
///
/// Best-effort: a failure is logged, and the stale roster ages out with its
/// ten-minute lifetime.
pub async fn forget_coach_roster(cache: &Cache, coach_user_id: Uuid, coach_tenant: TenantId) {
    if let Err(e) = cache
        .invalidate(&roster_cache_key(coach_user_id, coach_tenant))
        .await
    {
        warn!(
            user_id = %coach_user_id,
            error = %e,
            "TrainingPeaks roster cache could not be dropped; it ages out on its own"
        );
    }
}

/// The coach's stored `TrainingPeaks` session, once every precondition for
/// reading their roster holds.
///
/// Returned with the account role recorded on their connection (`None`
/// while it was never read).
async fn coach_session(
    repos: &RepositoryRegistry,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
) -> AppResult<(AuthSession, Option<ProviderAccountRole>)> {
    let (token, account_role) =
        match coach_session_state(repos, coach_user_id, coach_tenant, SCIOTTE_TRAININGPEAKS).await?
        {
            CoachSession::Missing => return Err(Refusal::NotConnected.error()),
            CoachSession::NeedsReconnect => return Err(Refusal::ReconnectNeeded.error()),
            CoachSession::Live {
                token,
                account_role,
            } => (token, account_role),
        };
    if account_role == Some(ProviderAccountRole::Athlete) {
        return Err(Refusal::NotCoachAccount.error());
    }
    if repos
        .users
        .trainingpeaks_terms_version(coach_user_id)
        .await?
        .as_deref()
        != Some(TRAININGPEAKS_TERMS_VERSION)
    {
        return Err(Refusal::TermsOutdated.error());
    }
    let session = serde_json::from_str::<AuthSession>(&token.access_token)
        .map_err(|_| Refusal::ReconnectNeeded.error())?;
    Ok((session, account_role))
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
/// coach is gone.
///
/// # Errors
///
/// Returns the repository error when the group, the coach or the coach's
/// session cannot be read.
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
    Ok(Some(MemberDelegation {
        group_name: group.name,
        coach_name: person_name(&coach),
        coach_needs_reconnect,
        link,
    }))
}

/// The coach's roster as the linking picker for `group` shows it: each
/// athlete with the live link that holds them in the group, and the member
/// their roster name suggests.
///
/// # Errors
///
/// Returns the errors of [`coach_roster`], or a repository error.
pub async fn roster_for_group(
    repos: &RepositoryRegistry,
    cache: &Cache,
    notifications: Option<&Arc<NotificationService>>,
    group: &CoachingGroup,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    refresh: bool,
) -> AppResult<Vec<RosterEntry>> {
    let athletes = coach_roster(
        repos,
        cache,
        notifications,
        coach_user_id,
        coach_tenant,
        refresh,
    )
    .await?;
    let links = repos
        .delegated_connections
        .list_live_for_coach_in_group(group.id, coach_user_id)
        .await?;
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
                    .display_name
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
/// athlete id, an athlete off the roster or the coach naming themselves; a
/// not-found error when the member is not a live member of the group; an
/// already-exists error (`already_proposed`, `athlete_already_linked`) when a
/// live link holds the member or the athlete; the roster's own refusals; or a
/// repository error.
pub async fn propose(
    repos: &RepositoryRegistry,
    cache: &Cache,
    notifications: Option<&Arc<NotificationService>>,
    group: &CoachingGroup,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    proposal: Proposal<'_>,
) -> AppResult<DelegatedConnection> {
    if proposal.provider != TRAININGPEAKS {
        return Err(Refusal::UnsupportedProvider.error());
    }
    let athlete = AthleteId::from_str(proposal.provider_athlete_id)
        .map_err(|_| Refusal::InvalidAthlete.error())?;
    let member = proposal.member_user_id;
    if member == coach_user_id {
        return Err(Refusal::MemberIsCoach.error());
    }
    if repos
        .groups
        .get_member(&group.id.to_string(), member)
        .await?
        .is_none()
    {
        return Err(AppError::not_found("Group member"));
    }
    let roster = coach_roster(
        repos,
        cache,
        notifications,
        coach_user_id,
        coach_tenant,
        false,
    )
    .await?;
    let Some(entry) = roster.iter().find(|a| a.id == athlete.as_str()) else {
        return Err(Refusal::AthleteNotOnRoster.error());
    };

    let link = DelegatedConnection::propose(
        SCIOTTE_TRAININGPEAKS.to_owned(),
        group.id,
        coach_user_id,
        coach_tenant,
        member,
        athlete.as_str().to_owned(),
        entry.display_name.clone(),
    );
    let Some(stored) = repos.delegated_connections.propose(&link).await? else {
        let member_held = repos
            .delegated_connections
            .list_live_for_member_in_group(group.id, member)
            .await?
            .iter()
            .any(|live| live.provider == SCIOTTE_TRAININGPEAKS);
        return Err(if member_held {
            Refusal::AlreadyProposed
        } else {
            Refusal::AthleteAlreadyLinked
        }
        .error());
    };
    info!(
        link_id = %stored.id,
        group_id = %group.id,
        coach_user_id = %coach_user_id,
        member_user_id = %member,
        "TrainingPeaks link proposed"
    );
    Notices::new(repos, notifications)
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
/// Returns an already-exists error when the member connects `TrainingPeaks`
/// with a login of their own here (`own_connection`) or already has a
/// confirmed link (`already_linked`); a not-found error when `link` is no
/// longer a proposal; or a repository error.
pub async fn confirm(
    repos: &RepositoryRegistry,
    notifications: Option<&Arc<NotificationService>>,
    group: &CoachingGroup,
    link: &DelegatedConnection,
    member_tenant: TenantId,
) -> AppResult<DelegatedConnection> {
    let member = link.member_user_id;
    if holds_own_connection(repos, member, member_tenant, &link.provider).await? {
        return Err(Refusal::OwnConnection.error());
    }
    let confirmed = repos
        .delegated_connections
        .confirm(link.id, member, member_tenant, Utc::now())
        .await?
        .ok_or_else(|| AppError::not_found("Pending TrainingPeaks link"))?;
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
        "member confirmed a delegated TrainingPeaks connection"
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
            return Err(AppError::not_found("Live TrainingPeaks link"));
        }
    };
    let ended = DelegationStore::new(repos)
        .end_one(link.id, caller, Some(caller), reason)
        .await?
        .ok_or_else(|| AppError::not_found("Live TrainingPeaks link"))?;
    info!(
        link_id = %ended.id,
        group_id = %group.id,
        reason = reason.as_str(),
        "TrainingPeaks link ended"
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
/// `TrainingPeaks` session in `coach_tenant` holds for an athlete `roster`,
/// just read live, no longer lists, telling both sides as [`end_off_roster`]
/// does.
///
/// A proposal whose athlete left the roster shows nowhere on the coach's
/// roster, so without this the coach could not withdraw it while it kept the
/// member from being linked again.
async fn end_off_coach_roster(
    repos: &RepositoryRegistry,
    notifications: Option<&Arc<NotificationService>>,
    coach_user_id: Uuid,
    coach_tenant: TenantId,
    roster: &[CoachedAthlete],
) -> AppResult<()> {
    let listed: Vec<&str> = roster.iter().map(|athlete| athlete.id.as_str()).collect();
    let ended = DelegationStore::new(repos)
        .end_off_coach_roster(coach_user_id, coach_tenant, SCIOTTE_TRAININGPEAKS, &listed)
        .await?;
    for link in &ended {
        info!(
            link_id = %link.id,
            group_id = %link.group_id,
            coach_user_id = %coach_user_id,
            member_user_id = %link.member_user_id,
            "TrainingPeaks link ended: the coach's roster no longer lists its athlete"
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

/// A platform tenant as the notification pipeline names it.
const fn notice_tenant(tenant: TenantId) -> NoticeTenantId {
    NoticeTenantId(tenant.as_uuid())
}
