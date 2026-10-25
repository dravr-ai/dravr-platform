// ABOUTME: Whether a human coach writes or reads a coached athlete's coaching-platform calendar, decided before the provider is built
// ABOUTME: Through the athlete's link or their own connection: only the group's coach, a write needs calendar_write and Coach access
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach calendar access
//!
//! A group's human coach reaches a coached athlete's calendar on a coaching
//! platform through the coach's own account there, at the athlete's path,
//! never through the athlete's own provider path
//! ([`AuthService::create_authenticated_provider`]): that path serves the
//! athlete, and it would go through whichever coach's link the athlete
//! holds.
//!
//! ## Reaching the athlete
//!
//! The athlete's id on the platform and the coach's credential come from one
//! of two places ([`member_read`]):
//!
//! - **a link**, for an athlete with no connection of their own: the link the
//!   athlete confirmed names their platform id, and it must be the
//!   requester's own and still read (the group still backs it, its athlete is
//!   the member by email, the coach's own session is live);
//! - **the athlete's own connection**: their own stored session names their
//!   platform id (`provider_user_id`, captured on both the API-key and the
//!   OAuth path), and the coach's credential is the requester's own session,
//!   once they coach the athlete in a group where the athlete shares with
//!   them (`coach_sharing_consent`).
//!
//! Either way the reach is the requester's: only the group's own coach
//! reaches the athlete's calendar.
//!
//! ## Writes
//!
//! A coach writes a coached athlete's training calendar on a coaching
//! platform whose coach credential writes calendars (Intervals.icu:
//! [`ProviderCapabilities::supports_coach_calendar_write`]).
//! [`AuthService::coach_calendar_writer`] builds the writer only when every
//! one of these holds, in this order:
//!
//! 1. the coach reaches the athlete, as above;
//! 2. the athlete allowed their coach calendar writes, on their coach
//!    relationship (`calendar_write`, [`member_consents`]);
//! 3. the platform's roster lists the athlete with the coach's account as
//!    their coach ([`RosterAccess::Coach`]), read under the coach's tenant,
//!    never the request's or the athlete's. Access the platform did not
//!    report is read once more, live, before it is refused as unknown.
//!
//! The platform enforces its own permission on every write too, so a stale
//! reading here can only refuse: a write the platform turns down is
//! `coach_write_refused`, which ends nothing.
//!
//! ## Planned reads
//!
//! A coach reads a coached athlete's planned calendar on a coaching platform
//! that keeps one ([`ProviderCapabilities::supports_planned_workouts`]).
//! [`AuthService::coach_planned_reader`] builds the reader once the coach
//! reaches the athlete, with no consent of its own: the athlete shares their
//! training with the coach (`coach_sharing_consent`, which the plan scope
//! holds too), and the platform enforces its own read permission on the read.
//! The reader is the coach's, not the athlete's, so the platform shows it
//! what it hides from the athlete, labelled. The read's refusal ends a link
//! whose athlete left the coach's roster as every read through a link does.
//!
//! [`ProviderCapabilities::supports_coach_calendar_write`]: pierre_providers::spi::ProviderCapabilities::supports_coach_calendar_write
//! [`ProviderCapabilities::supports_planned_workouts`]: pierre_providers::spi::ProviderCapabilities::supports_planned_workouts

use std::cmp::Reverse;
use std::sync::Arc;

use pierre_core::errors::AppResult;
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{DelegatedConnection, RosterAccess, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope::AiGovernedProvider;
use pierre_providers::{CoreFitnessProvider, OAuth2Credentials};
use pierre_services::coach_consents::member_consents;
use pierre_services::coach_platform::{
    check_roster_transport, coach_platform, with_key_budget, CoachPlatform,
};
use pierre_services::delegated_connections::{
    coach_roster, coach_session_state, CoachSession, DelegationServices,
};
use pierre_services::delegated_read::{member_read, CoachReadRefusal, MemberRead};
use pierre_services::delegation_refusal::Refusal;
use tracing::info;
use uuid::Uuid;

use crate::protocol::auth::AuthService;
use crate::runtime::ToolRuntime;

/// Why a coach's write to or read of an athlete's calendar was refused, in
/// words the model relays to the coach.
pub(crate) struct CoachCalendarRefusal {
    /// The refusal's name, which a tool answers with beside the words.
    pub reason: &'static str,
    /// What was refused and what the coach or the athlete can do about it.
    pub message: String,
}

/// Whose account a coach's calendar access goes through: what a refusal from
/// the platform is reacted to by ([`AuthService::react_to_coach_refusal`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CoachSeat {
    /// The coach.
    pub coach_user_id: Uuid,
    /// The tenant the coach's credential was read under.
    pub coach_tenant: TenantId,
    /// The link the access goes through; `None` for an athlete who connects
    /// the platform themselves.
    pub link_id: Option<Uuid>,
}

/// The writer a coach's calendar write goes through.
pub(crate) struct CoachCalendarWriter {
    /// The athlete's calendar on the platform, written with the coach's
    /// credential.
    pub provider: Box<dyn CoreFitnessProvider>,
    /// Whose account it writes through.
    pub seat: CoachSeat,
}

/// The reader a coach's planned-calendar read goes through.
#[cfg(feature = "tools-data")]
pub(crate) struct CoachPlannedReader {
    /// The athlete's planned calendar on the platform, read with the coach's
    /// credential.
    pub provider: Box<dyn CoreFitnessProvider>,
    /// Whose account it reads through.
    pub seat: CoachSeat,
}

/// How the coach reaches the athlete on the platform.
struct Reach {
    /// The athlete's id on the platform.
    athlete_id: String,
    /// The coach's credential, counted against the coach's own key.
    credentials: OAuth2Credentials,
    /// The group the coach coaches the athlete in.
    group_id: Uuid,
    /// Whose account it goes through.
    seat: CoachSeat,
}

/// The linking services every roster read goes through, from `runtime`.
pub fn delegation_services(runtime: &dyn ToolRuntime) -> DelegationServices<'_> {
    #[cfg(feature = "client-notifications")]
    let notifications = runtime.notification_service();
    #[cfg(not(feature = "client-notifications"))]
    let notifications = None;
    DelegationServices {
        repos: runtime.repos(),
        registry: runtime.provider_registry(),
        cache: runtime.cache(),
        notifications,
    }
}

/// A refusal named `reason`, worded `message`.
const fn refused(reason: &'static str, message: String) -> CoachCalendarRefusal {
    CoachCalendarRefusal { reason, message }
}

/// What a coach asked to do with an athlete's calendar, as a refusal words
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Write it.
    Write,
    /// Read its plan: only the planned-workouts tool does (`tools-data`).
    #[cfg(feature = "tools-data")]
    Read,
}

impl Access {
    /// The verb, as "the link writes nothing".
    const fn verb(self) -> &'static str {
        match self {
            Self::Write => "writes",
            #[cfg(feature = "tools-data")]
            Self::Read => "reads",
        }
    }

    /// The participle, as "the calendar cannot be written".
    const fn participle(self) -> &'static str {
        match self {
            Self::Write => "written",
            #[cfg(feature = "tools-data")]
            Self::Read => "read",
        }
    }

    /// The bare verb, as "to write their calendar".
    const fn infinitive(self) -> &'static str {
        match self {
            Self::Write => "write",
            #[cfg(feature = "tools-data")]
            Self::Read => "read",
        }
    }
}

impl AuthService {
    /// The writer `requester`, a human coach, writes `athlete`'s training
    /// calendar on `backend` with, or why they may not.
    ///
    /// `athlete_tenant` is the athlete's home tenant, the one their
    /// calendar's ledger lives under. Nothing provider-side is read until the
    /// coach's reach and the athlete's consent are known; then the coach's
    /// roster is read for the athlete's access.
    ///
    /// # Errors
    ///
    /// Returns a repository error, the error of a roster that cannot be read
    /// (the coach's session refused, the platform failing), or the error of
    /// building the writer.
    pub(crate) async fn coach_calendar_writer(
        &self,
        requester: Uuid,
        athlete: Uuid,
        athlete_tenant: TenantId,
        backend: &str,
    ) -> AppResult<Result<CoachCalendarWriter, CoachCalendarRefusal>> {
        let registry = self.runtime().provider_registry();
        let Some(platform) = coach_platform(registry, backend) else {
            let brand = registry.get_display_name(backend).unwrap_or(backend);
            return Ok(Err(refused(
                "coach_write_unsupported",
                format!(
                    "{brand} is not a coaching platform, so a coach writes no athlete's calendar \
                     there."
                ),
            )));
        };
        let platform = platform.as_ref();
        let brand = platform.brand();
        let writes = registry
            .get_capabilities(platform.backend())
            .is_some_and(|caps| caps.supports_coach_calendar_write());
        if !writes {
            return Ok(Err(refused(
                "coach_write_unsupported",
                format!("{brand} does not let a coach write an athlete's calendar through Dravr."),
            )));
        }
        let reach = match self
            .reach(requester, athlete, athlete_tenant, platform, Access::Write)
            .await?
        {
            Ok(reach) => reach,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let consents = member_consents(
            self.runtime().repos().groups.as_ref(),
            reach.group_id,
            athlete,
        )
        .await?;
        if !consents.calendar_write {
            return Ok(Err(refused(
                "no_calendar_write_consent",
                "This athlete has not allowed you to put workouts on their calendar. They turn \
                 it on in the group, under what their coach may do; until then their calendar \
                 is read, never written."
                    .to_owned(),
            )));
        }
        if let Err(refusal) = self
            .coach_access(
                platform,
                requester,
                &reach.athlete_id,
                reach.seat.coach_tenant,
            )
            .await?
        {
            return Ok(Err(refusal));
        }
        let provider = registry
            .create_delegated_calendar_writer(
                platform.backend(),
                &reach.athlete_id,
                reach.credentials,
            )
            .await?;
        info!(
            link_id = ?reach.seat.link_id,
            coach_user_id = %requester,
            provider = platform.backend(),
            "Coach calendar writer built"
        );
        let terms = Arc::clone(registry);
        Ok(Ok(CoachCalendarWriter {
            provider: AiGovernedProvider::wrap(provider, terms),
            seat: reach.seat,
        }))
    }

    /// How `requester` reaches `athlete` on `platform`: through the athlete's
    /// link, or, for an athlete who connects the platform themselves, through
    /// the requester's own account at the athlete's own platform id. `None`
    /// of either is a refusal worded for `access`.
    async fn reach(
        &self,
        requester: Uuid,
        athlete: Uuid,
        athlete_tenant: TenantId,
        platform: &dyn CoachPlatform,
        access: Access,
    ) -> AppResult<Result<Reach, CoachCalendarRefusal>> {
        let brand = platform.brand();
        let read = match member_read(
            self.runtime().repos(),
            self.runtime().provider_registry(),
            platform,
            athlete,
            Some(athlete_tenant),
        )
        .await?
        {
            MemberRead::ThroughCoach(read) => *read,
            MemberRead::Refused { link, refusal } => {
                return Ok(Err(unreadable_link(
                    &link, refusal, requester, brand, access,
                )));
            }
            MemberRead::OwnCredential => {
                return self
                    .reach_own_connection(requester, athlete, athlete_tenant, platform, access)
                    .await;
            }
            MemberRead::NotLinked => {
                return Ok(Err(refused(
                    "not_linked",
                    format!(
                        "This athlete reaches {brand} neither through a connection of their own \
                         nor through a link with you. Either they connect {brand} in Dravr, or \
                         you propose a link (propose_athlete_link) and they confirm it."
                    ),
                )));
            }
        };
        let link = read.link;
        if link.coach_user_id != requester {
            return Ok(Err(another_coach(brand, access)));
        }
        Ok(Ok(Reach {
            athlete_id: link.provider_athlete_id,
            credentials: read.credentials,
            group_id: link.group_id,
            seat: CoachSeat {
                coach_user_id: requester,
                coach_tenant: read.coach_tenant,
                link_id: Some(link.id),
            },
        }))
    }

    /// How `requester` reaches `athlete`, who connects `platform` with a
    /// session of their own in `athlete_tenant`: the athlete's platform id
    /// from that session, the requester's own credential from their own
    /// session ([`coach_session_tenant`]), once they coach the athlete in a
    /// group the athlete shares with them.
    async fn reach_own_connection(
        &self,
        requester: Uuid,
        athlete: Uuid,
        athlete_tenant: TenantId,
        platform: &dyn CoachPlatform,
        access: Access,
    ) -> AppResult<Result<Reach, CoachCalendarRefusal>> {
        let repos = self.runtime().repos();
        let brand = platform.brand();
        let backend = platform.backend();
        let Some(group) = coached_group(repos, requester, athlete).await? else {
            return Ok(Err(refused(
                "not_coached",
                format!(
                    "This athlete is not in a group you coach, or has stopped sharing with you, \
                     so their {brand} calendar is not yours to {}.",
                    access.infinitive()
                ),
            )));
        };
        let athlete_id = repos
            .oauth_tokens
            .get_token(athlete, athlete_tenant, backend)
            .await?
            .and_then(|token| token.provider_user_id)
            .filter(|id| !id.is_empty());
        let Some(athlete_id) = athlete_id else {
            return Ok(Err(refused(
                "athlete_platform_id_unknown",
                format!(
                    "Dravr does not know this athlete's {brand} id yet: they reconnect {brand} \
                     in Dravr, and their calendar can be {} after that.",
                    access.participle()
                ),
            )));
        };
        let Some(coach_tenant) = coach_session_tenant(repos, requester, &group, backend).await?
        else {
            return Ok(Err(coach_session_refusal(
                CoachSessionGap::Missing,
                brand,
                access,
            )));
        };
        let token = match coach_session_state(repos, requester, coach_tenant, backend).await? {
            CoachSession::Live { token, .. } => token,
            CoachSession::NeedsReconnect => {
                return Ok(Err(coach_session_refusal(
                    CoachSessionGap::NeedsReconnect,
                    brand,
                    access,
                )));
            }
            CoachSession::Missing => {
                return Ok(Err(coach_session_refusal(
                    CoachSessionGap::Missing,
                    brand,
                    access,
                )));
            }
        };
        let Some(credentials) = platform.coach_credentials(&token) else {
            return Ok(Err(refused(
                Refusal::ApiKeyRequired.as_str(),
                format!(
                    "{brand} lets a coach reach their athletes through a personal API key only: \
                     reconnect {brand} in Dravr with your API key, and this athlete's calendar \
                     can be {} after that.",
                    access.participle()
                ),
            )));
        };
        Ok(Ok(Reach {
            athlete_id,
            credentials: with_key_budget(self.runtime().provider_registry(), &token, credentials),
            group_id: group.id,
            seat: CoachSeat {
                coach_user_id: requester,
                coach_tenant,
                link_id: None,
            },
        }))
    }

    /// Whether `coach` coaches the athlete `athlete_id` on `platform`, by
    /// the coach's roster read under `coach_tenant`, the tenant the coach's
    /// credential was read under.
    ///
    /// A roster that reports no access for the athlete is read once more,
    /// live, before the access is refused as unknown: it is never taken for
    /// a follower's.
    async fn coach_access(
        &self,
        platform: &dyn CoachPlatform,
        coach: Uuid,
        athlete_id: &str,
        coach_tenant: TenantId,
    ) -> AppResult<Result<(), CoachCalendarRefusal>> {
        let brand = platform.brand();
        let step = platform
            .coach_access_step()
            .map_or_else(String::new, |step| format!(" {step}"));
        check_roster_transport(self.runtime().provider_registry(), platform)?;
        let services = delegation_services(self.runtime().as_ref());
        for refresh in [false, true] {
            let roster = coach_roster(services, platform, coach, coach_tenant, refresh).await?;
            let Some(entry) = roster.iter().find(|athlete| athlete.id == athlete_id) else {
                return Ok(Err(refused(
                    Refusal::AthleteNotOnRoster.as_str(),
                    format!(
                        "This athlete is not on your {brand} roster, so their calendar is not \
                         yours to write.{step}"
                    ),
                )));
            };
            match entry.access {
                Some(RosterAccess::Coach) => return Ok(Ok(())),
                Some(RosterAccess::Follower) => {
                    return Ok(Err(refused(
                        "coach_follower",
                        format!(
                            "On {brand} you follow this athlete rather than coach them, so \
                             {brand} does not let you write their calendar.{step}"
                        ),
                    )));
                }
                Some(RosterAccess::NoAccess) => {
                    return Ok(Err(refused(
                        Refusal::AthleteNoAccess.as_str(),
                        format!(
                            "{brand} gives your account no access to this athlete, so their \
                             calendar is not written.{step}"
                        ),
                    )));
                }
                None => {}
            }
        }
        Ok(Err(refused(
            "coach_access_unknown",
            format!(
                "{brand} did not say what access your account has to this athlete, so their \
                 calendar is not written."
            ),
        )))
    }

    /// The reader `requester`, a human coach, reads `athlete`'s planned
    /// calendar on `backend` with, or why they may not.
    ///
    /// `athlete_tenant` is the athlete's home tenant. Nothing provider-side
    /// is read here: the platform enforces its own read permission on the
    /// read itself, and the caller reacts to its refusal
    /// ([`Self::react_to_coach_refusal`]). The reader is wrapped in the
    /// transport gate the athlete's own read goes through.
    ///
    /// # Errors
    ///
    /// Returns a repository error, or the error of building the reader.
    #[cfg(feature = "tools-data")]
    pub(crate) async fn coach_planned_reader(
        &self,
        requester: Uuid,
        athlete: Uuid,
        athlete_tenant: TenantId,
        backend: &str,
    ) -> AppResult<Result<CoachPlannedReader, CoachCalendarRefusal>> {
        let registry = self.runtime().provider_registry();
        let Some(platform) = coach_platform(registry, backend) else {
            let brand = registry.get_display_name(backend).unwrap_or(backend);
            return Ok(Err(refused(
                "coach_read_unsupported",
                format!(
                    "{brand} is not a coaching platform, so a coach reads no athlete's calendar \
                     there."
                ),
            )));
        };
        let platform = platform.as_ref();
        let brand = platform.brand();
        let plans = registry
            .get_capabilities(platform.backend())
            .is_some_and(|caps| caps.supports_planned_workouts());
        if !plans {
            return Ok(Err(refused(
                "coach_read_unsupported",
                format!("{brand} keeps no planned calendar that Dravr reads."),
            )));
        }
        let reach = match self
            .reach(requester, athlete, athlete_tenant, platform, Access::Read)
            .await?
        {
            Ok(reach) => reach,
            Err(refusal) => return Ok(Err(refusal)),
        };
        // No consent of its own: the athlete shares their training with this
        // coach (`coach_sharing_consent`), which the reach and the plan scope
        // both held. `calendar_write` (a write) and `wellness_read` (health
        // data) are the coach consents that ask for more.
        let provider = registry
            .create_delegated_coach_reader(platform.backend(), &reach.athlete_id, reach.credentials)
            .await?;
        info!(
            link_id = ?reach.seat.link_id,
            coach_user_id = %requester,
            provider = platform.backend(),
            "Coach planned reader built"
        );
        let terms = Arc::clone(registry);
        Ok(Ok(CoachPlannedReader {
            provider: AiGovernedProvider::wrap(provider, terms),
            seat: reach.seat,
        }))
    }
}

/// The active group `coach` coaches `athlete` in, where the athlete is a
/// live member who shares with the coach; `None` when there is none. Among
/// several, one where the athlete allowed the coach calendar writes comes
/// first: the consent is to the coach, whichever of their groups holds it.
async fn coached_group(
    repos: &RepositoryRegistry,
    coach: Uuid,
    athlete: Uuid,
) -> AppResult<Option<CoachingGroup>> {
    if coach == athlete {
        return Ok(None);
    }
    let mut first = None;
    for group in repos.groups.list_groups_coached_by(coach).await? {
        if !group.is_active {
            continue;
        }
        let Some(member) = repos
            .groups
            .get_member(&group.id.to_string(), athlete)
            .await?
            .filter(|member| member.coach_sharing_consent)
        else {
            continue;
        };
        if member.coach_consents.calendar_write {
            return Ok(Some(group));
        }
        first.get_or_insert(group);
    }
    Ok(first)
}

/// Why the coach's own session serves no access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoachSessionGap {
    /// Only the coach can renew it by signing in again.
    NeedsReconnect,
    /// The coach holds no session of their own for the platform.
    Missing,
}

/// The tenant `coach`'s own `backend` session lives in, the one their
/// credential is read under: the tenant of `group` when they hold one there,
/// else the one of their sessions they renewed last. A coach attached to a
/// group by invite keeps their own key in their own tenant, never the group
/// owner's. `None` when they hold no session for `backend`.
async fn coach_session_tenant(
    repos: &RepositoryRegistry,
    coach: Uuid,
    group: &CoachingGroup,
    backend: &str,
) -> AppResult<Option<TenantId>> {
    let mut sessions: Vec<_> = repos
        .oauth_tokens
        .get_tokens(coach, None)
        .await?
        .into_iter()
        .filter(|token| token.provider == backend)
        .collect();
    sessions.sort_by_key(|token| Reverse(token.updated_at));
    let chosen = sessions
        .iter()
        .find(|token| token.tenant_id == group.tenant_id)
        .or_else(|| sessions.first());
    Ok(chosen.and_then(|token| TenantId::parse_str(&token.tenant_id).ok()))
}

/// The refusal of `access` when the coach's own session cannot serve it.
fn coach_session_refusal(
    gap: CoachSessionGap,
    brand: &str,
    access: Access,
) -> CoachCalendarRefusal {
    match gap {
        CoachSessionGap::NeedsReconnect => refused(
            Refusal::ReconnectNeeded.as_str(),
            format!(
                "Your {brand} connection needs reconnecting before it {} this athlete's \
                 calendar.",
                access.verb()
            ),
        ),
        CoachSessionGap::Missing => refused(
            Refusal::NotConnected.as_str(),
            format!(
                "You hold no {brand} connection of your own, so this athlete's calendar cannot \
                 be {} through it: connect your {brand} account first.",
                access.participle()
            ),
        ),
    }
}

/// The refusal of `access` through a link another coach holds.
fn another_coach(brand: &str, access: Access) -> CoachCalendarRefusal {
    refused(
        "linked_through_another_coach",
        format!(
            "This athlete's {brand} link goes through another coach's account, so only that \
             coach {} their calendar.",
            access.verb()
        ),
    )
}

/// The refusal of `access` through `link`, which the delegated read decision
/// refused for `refusal`.
fn unreadable_link(
    link: &DelegatedConnection,
    refusal: CoachReadRefusal,
    requester: Uuid,
    brand: &str,
    access: Access,
) -> CoachCalendarRefusal {
    if link.coach_user_id != requester {
        return another_coach(brand, access);
    }
    match refusal {
        CoachReadRefusal::Unbound(_) => refused(
            "link_unbound",
            format!(
                "The link names a {brand} athlete whose email is not this athlete's verified \
                 Dravr email, so it {} nothing: link them again from the group, and they \
                 confirm.",
                access.verb()
            ),
        ),
        CoachReadRefusal::CoachNeedsReconnect => {
            coach_session_refusal(CoachSessionGap::NeedsReconnect, brand, access)
        }
        CoachReadRefusal::CoachDisconnected => {
            coach_session_refusal(CoachSessionGap::Missing, brand, access)
        }
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::models::RosterAthlete;
    use pierre_services::trainingpeaks_accounts::EmailBinding;

    use super::*;

    fn link(coach: Uuid) -> DelegatedConnection {
        DelegatedConnection::propose(
            "intervals_icu".to_owned(),
            Uuid::new_v4(),
            coach,
            TenantId::generate(),
            Uuid::new_v4(),
            RosterAthlete::new("i201", None, Some("alex@links.test".to_owned())),
        )
    }

    #[test]
    fn a_link_another_coach_holds_is_theirs_whatever_kept_it_from_reading() {
        let (requester, other) = (Uuid::new_v4(), Uuid::new_v4());
        for refusal in [
            CoachReadRefusal::Unbound(EmailBinding::Mismatch),
            CoachReadRefusal::CoachNeedsReconnect,
            CoachReadRefusal::CoachDisconnected,
        ] {
            let refused = unreadable_link(
                &link(other),
                refusal,
                requester,
                "Intervals.icu",
                Access::Write,
            );
            assert_eq!(refused.reason, "linked_through_another_coach");
        }
    }

    #[test]
    fn the_requesters_own_unreadable_link_says_what_to_fix() {
        let coach = Uuid::new_v4();
        let own = link(coach);
        let reason =
            |refusal| unreadable_link(&own, refusal, coach, "Intervals.icu", Access::Write).reason;
        assert_eq!(
            reason(CoachReadRefusal::Unbound(
                EmailBinding::ProviderEmailMissing
            )),
            "link_unbound"
        );
        // The roster's own names for the coach's session, which the roster
        // and link tools answer with too.
        assert_eq!(
            reason(CoachReadRefusal::CoachNeedsReconnect),
            "coach_platform_reconnect_needed"
        );
        assert_eq!(
            reason(CoachReadRefusal::CoachDisconnected),
            "coach_platform_not_connected"
        );
        let words = unreadable_link(
            &own,
            CoachReadRefusal::CoachDisconnected,
            coach,
            "Intervals.icu",
            Access::Write,
        );
        assert!(words.message.contains("Intervals.icu"), "{}", words.message);
    }

    #[cfg(feature = "tools-data")]
    #[test]
    fn a_refused_read_keeps_the_writes_names_in_a_readers_words() {
        let (coach, other) = (Uuid::new_v4(), Uuid::new_v4());
        let own = link(coach);
        for refusal in [
            CoachReadRefusal::Unbound(EmailBinding::Mismatch),
            CoachReadRefusal::CoachNeedsReconnect,
            CoachReadRefusal::CoachDisconnected,
        ] {
            let write = unreadable_link(&own, refusal, coach, "Intervals.icu", Access::Write);
            let read = unreadable_link(&own, refusal, coach, "Intervals.icu", Access::Read);
            assert_eq!(read.reason, write.reason);
            assert!(!read.message.contains("writ"), "{}", read.message);
            assert!(read.message.contains("read"), "{}", read.message);
            let elsewhere =
                unreadable_link(&link(other), refusal, coach, "Intervals.icu", Access::Read);
            assert_eq!(elsewhere.reason, "linked_through_another_coach");
        }
        let other = another_coach("Intervals.icu", Access::Read);
        assert!(
            other.message.contains("reads their calendar"),
            "{}",
            other.message
        );
    }
}
