// ABOUTME: Whether a human coach writes or reads a coached athlete's coaching-platform calendar, decided before the provider is built
// ABOUTME: Only the link's own coach; a write also needs the athlete's calendar_write consent and Coach access on the platform roster
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
//! holds. Both providers below are built only for the link's own coach.
//!
//! ## Writes
//!
//! A coach writes a coached athlete's training calendar on a coaching
//! platform whose coach credential writes calendars (Intervals.icu:
//! [`ProviderCapabilities::supports_coach_calendar_write`]).
//! [`AuthService::coach_calendar_writer`] builds the writer only when every one of
//! these holds, in this order:
//!
//! 1. the athlete reads the platform through a confirmed link, not an
//!    account of their own, and the link reads ([`member_read`]: the group
//!    still backs it, its athlete is the member by email, the coach's own
//!    session is live);
//! 2. the link is the requester's own: only the link's coach writes;
//! 3. the athlete allowed calendar writes on the link (`calendar_write`);
//! 4. the platform's roster lists the athlete with the coach's account as
//!    their coach ([`RosterAccess::Coach`]), read under the link's coach
//!    tenant, never the request's or the athlete's. Access the platform did
//!    not report is read once more, live, before it is refused as unknown.
//!
//! The platform enforces its own permission on every write too, so a stale
//! reading here can only refuse: a write the platform turns down is
//! `coach_write_refused`, which ends nothing.
//!
//! ## Planned reads
//!
//! A coach reads a coached athlete's planned calendar on a coaching platform
//! that keeps one ([`ProviderCapabilities::supports_planned_workouts`]).
//! [`AuthService::coach_planned_reader`] builds the reader under the first two
//! conditions above and no consent of its own: the link the athlete confirmed
//! is their consent to workout reads through the coach's account, and the
//! plan scope has already held their `coach_sharing_consent`. The reader is
//! the coach's, not the athlete's, so the platform shows it what it hides from
//! the athlete, labelled. The platform enforces its own read permission on
//! the read, and the read's refusal ends a link whose athlete left the coach's
//! roster as every read through a link does.
//!
//! [`ProviderCapabilities::supports_coach_calendar_write`]: pierre_providers::spi::ProviderCapabilities::supports_coach_calendar_write
//! [`ProviderCapabilities::supports_planned_workouts`]: pierre_providers::spi::ProviderCapabilities::supports_planned_workouts

use std::sync::Arc;

use pierre_core::errors::AppResult;
use pierre_core::models::{DelegatedConnection, RosterAccess, TenantId};
use pierre_providers::ai_scope::AiGovernedProvider;
use pierre_providers::CoreFitnessProvider;
use pierre_services::coach_platform::{check_roster_transport, coach_platform, CoachPlatform};
use pierre_services::delegated_connections::{coach_roster, DelegationServices};
use pierre_services::delegated_read::{member_read, CoachReadRefusal, MemberRead};
use pierre_services::delegation_refusal::Refusal;
use tracing::info;
use uuid::Uuid;

use crate::protocol::auth::AuthService;
use crate::runtime::ToolRuntime;

/// Why a coach's write to or read of an athlete's calendar was refused, in
/// words the model relays to the coach.
pub struct CoachCalendarRefusal {
    /// The refusal's name, which a tool answers with beside the words.
    pub reason: &'static str,
    /// What was refused and what the coach or the athlete can do about it.
    pub message: String,
}

/// The writer a coach's calendar write goes through.
pub struct CoachCalendarWriter {
    /// The athlete's calendar on the platform, written with the coach's
    /// credential.
    pub provider: Box<dyn CoreFitnessProvider>,
    /// The link the writer goes through.
    pub link_id: Uuid,
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
}

impl AuthService {
    /// The writer `requester`, a human coach, writes `athlete`'s training
    /// calendar on `backend` with, or why they may not.
    ///
    /// `athlete_tenant` is the tenant the athlete's link is confirmed in, the
    /// one their calendar's ledger lives under. Nothing provider-side is read
    /// until the link, its coach and the athlete's consent are known; then
    /// the coach's roster is read for the athlete's access.
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
        let read = match member_read(
            self.runtime().repos(),
            registry,
            platform,
            athlete,
            Some(athlete_tenant),
        )
        .await?
        {
            MemberRead::ThroughCoach(read) => *read,
            MemberRead::Refused { link, refusal } => {
                return Ok(Err(unreadable_link(
                    &link,
                    refusal,
                    requester,
                    brand,
                    Access::Write,
                )));
            }
            // LIMITATION(registre#1055): `coach_calendar_writer` refuses an athlete who connects
            // the platform themselves: the athlete's write consent lives on a delegated link,
            // and an athlete with their own connection holds none, so the coach's key never
            // writes their calendar.
            MemberRead::OwnCredential => {
                return Ok(Err(refused(
                    Refusal::OwnConnection.as_str(),
                    format!(
                        "This athlete connects {brand} themselves, so there is no link through \
                         your {brand} account to write their calendar with: a human coach writes \
                         an athlete's {brand} calendar only through a link the athlete confirmed."
                    ),
                )));
            }
            MemberRead::NotLinked => {
                return Ok(Err(refused(
                    "not_linked",
                    format!(
                        "This athlete has no confirmed {brand} link with you. Propose one \
                         (propose_athlete_link); once they confirm it and allow calendar writes \
                         on it, their calendar takes your writes."
                    ),
                )));
            }
        };
        let link = read.link;
        if link.coach_user_id != requester {
            return Ok(Err(another_coach(brand, Access::Write)));
        }
        if !link.calendar_write {
            return Ok(Err(refused(
                "no_calendar_write_consent",
                format!(
                    "This athlete has not allowed calendar writes on their {brand} link with you. \
                     They turn it on on the link in the group; until then their calendar is \
                     read, never written."
                ),
            )));
        }
        if let Err(refusal) = self
            .coach_access(platform, &link, read.coach_tenant)
            .await?
        {
            return Ok(Err(refusal));
        }
        let provider = registry
            .create_delegated_calendar_writer(
                &link.provider,
                &link.provider_athlete_id,
                read.credentials,
            )
            .await?;
        info!(
            link_id = %link.id,
            coach_user_id = %link.coach_user_id,
            provider = %link.provider,
            "Coach calendar writer built through the athlete's link"
        );
        let terms = Arc::clone(registry);
        Ok(Ok(CoachCalendarWriter {
            provider: AiGovernedProvider::wrap(provider, terms),
            link_id: link.id,
        }))
    }

    /// Whether `link`'s coach coaches its athlete on `platform`, by the
    /// coach's roster read under `coach_tenant`, the tenant the coach's
    /// credential was read under.
    ///
    /// A roster that reports no access for the athlete is read once more,
    /// live, before the access is refused as unknown: it is never taken for
    /// a follower's.
    async fn coach_access(
        &self,
        platform: &dyn CoachPlatform,
        link: &DelegatedConnection,
        coach_tenant: TenantId,
    ) -> AppResult<Result<(), CoachCalendarRefusal>> {
        let brand = platform.brand();
        check_roster_transport(self.runtime().provider_registry(), platform)?;
        let services = delegation_services(self.runtime().as_ref());
        for refresh in [false, true] {
            let roster = coach_roster(
                services,
                platform,
                link.coach_user_id,
                coach_tenant,
                refresh,
            )
            .await?;
            let Some(entry) = roster
                .iter()
                .find(|athlete| athlete.id == link.provider_athlete_id)
            else {
                return Ok(Err(refused(
                    Refusal::AthleteNotOnRoster.as_str(),
                    format!(
                        "This athlete is no longer on your {brand} roster, so their calendar is \
                         not yours to write."
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
                             {brand} does not let you write their calendar. The athlete makes you \
                             their coach on {brand} first."
                        ),
                    )));
                }
                Some(RosterAccess::NoAccess) => {
                    return Ok(Err(refused(
                        Refusal::AthleteNoAccess.as_str(),
                        format!(
                            "{brand} gives your account no access to this athlete, so their \
                             calendar is not written."
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
    /// `athlete_tenant` is the tenant the athlete's link is confirmed in.
    /// Nothing provider-side is read here: the platform enforces its own read
    /// permission on the read itself, and the caller reacts to its refusal
    /// ([`Self::react_to_delegated_refusal`]) as every read through a link
    /// does. The reader is wrapped in the transport gate the athlete's own
    /// read goes through.
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
    ) -> AppResult<Result<Box<dyn CoreFitnessProvider>, CoachCalendarRefusal>> {
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
        let read = match member_read(
            self.runtime().repos(),
            registry,
            platform,
            athlete,
            Some(athlete_tenant),
        )
        .await?
        {
            MemberRead::ThroughCoach(read) => *read,
            MemberRead::Refused { link, refusal } => {
                return Ok(Err(unreadable_link(
                    &link,
                    refusal,
                    requester,
                    brand,
                    Access::Read,
                )));
            }
            // LIMITATION(registre#1055): `coach_planned_reader` refuses an athlete who connects
            // the platform themselves: a coach reads an athlete's calendar only through a link
            // the athlete confirmed, and an athlete with their own connection holds none.
            MemberRead::OwnCredential => {
                return Ok(Err(refused(
                    Refusal::OwnConnection.as_str(),
                    format!(
                        "This athlete connects {brand} themselves, so there is no link through \
                         your {brand} account to read their calendar with: a human coach reads \
                         an athlete's {brand} calendar only through a link the athlete confirmed."
                    ),
                )));
            }
            MemberRead::NotLinked => {
                return Ok(Err(refused(
                    "not_linked",
                    format!(
                        "This athlete has no confirmed {brand} link with you. Propose one \
                         (propose_athlete_link); once they confirm it, you read their {brand} \
                         calendar through it."
                    ),
                )));
            }
        };
        let link = read.link;
        if link.coach_user_id != requester {
            return Ok(Err(another_coach(brand, Access::Read)));
        }
        // No consent of its own: the confirmed link is the athlete's consent
        // to workout reads through this coach's account, and the plan scope
        // held their `coach_sharing_consent`. `calendar_write` (a write) and
        // `wellness_read` (health data) are the per-link consents.
        let provider = registry
            .create_delegated_coach_reader(
                &link.provider,
                &link.provider_athlete_id,
                read.credentials,
            )
            .await?;
        info!(
            link_id = %link.id,
            coach_user_id = %link.coach_user_id,
            provider = %link.provider,
            "Coach planned reader built through the athlete's link"
        );
        let terms = Arc::clone(registry);
        Ok(Ok(AiGovernedProvider::wrap(provider, terms)))
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
    let verb = access.verb();
    match refusal {
        CoachReadRefusal::Unbound(_) => refused(
            "link_unbound",
            format!(
                "The link names a {brand} athlete whose email is not this athlete's verified \
                 Dravr email, so it {verb} nothing: link them again from the group, and they \
                 confirm."
            ),
        ),
        CoachReadRefusal::CoachNeedsReconnect => refused(
            Refusal::ReconnectNeeded.as_str(),
            format!(
                "Your {brand} connection needs reconnecting before it {verb} this athlete's \
                 calendar."
            ),
        ),
        CoachReadRefusal::CoachDisconnected => refused(
            Refusal::NotConnected.as_str(),
            format!(
                "You hold no {brand} connection of your own, so this athlete's calendar cannot \
                 be {} through it: connect your {brand} account first.",
                access.participle()
            ),
        ),
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
