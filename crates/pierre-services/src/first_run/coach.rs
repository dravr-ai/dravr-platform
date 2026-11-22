// ABOUTME: A coach's first run — « Préparer ton coaching »: the coach seat, the athlete invited, a coaching platform linked
// ABOUTME: Each step is done from server state alone (the access, a member who joined, a connection), never from a client flag
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach's first steps (carnet#1529).
//!
//! A user who coaches others and holds no coach access yet lands on a
//! coach-shaped Home, never the athlete's. Its checklist has three steps, in
//! the board's order (P1·D):
//!
//! 1. **The coach seat** ([`COACH_SEAT_STEP`]) — done once the user holds
//!    coach access (`manages_roster`). An admin grants it (ADR-018); the
//!    request and its state are the coach-access request's to say.
//! 2. **The athlete invited** ([`INVITE_ATHLETE_STEP`]) — done once someone
//!    other than the coach joined one of the coach's groups: a coachless group
//!    they own in this tenant (the ones a grant attaches them to), or a group
//!    they already coach.
//! 3. **A coaching platform linked** ([`COACH_PLATFORM_STEP`]) — done once the
//!    coach holds a TrainingPeaks or Intervals.icu connection of their own.
//!    What that connection did for the seat (a TrainingPeaks coach account
//!    grants, an Intervals.icu roster files a request) is the server's, read
//!    back from the seat; this step only says the link exists.
//!
//! [`CoachFacts::read`] gathers what the steps are decided on; [`coach_setup`]
//! decides them, so the rule is a pure function its tests drive directly.

use dravr_tronc::dates::Timestamp;
use pierre_core::constants::oauth_providers::{INTERVALS_ICU, SCIOTTE_TRAININGPEAKS};
use pierre_core::errors::AppResult;
use pierre_core::models::{CoachingGroup, ConnectionType, TenantId};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// The coach seat: done once the coach holds coach access.
pub const COACH_SEAT_STEP: &str = "seat";
/// The athlete invited: done once someone joined one of the coach's groups.
pub const INVITE_ATHLETE_STEP: &str = "invite";
/// A coaching platform linked: done once the coach holds a connection to one.
pub const COACH_PLATFORM_STEP: &str = "platform";

/// The providers a coach coaches on, in the order the checklist offers them:
/// the same two the onboarding seat step and Group info's seat block list.
pub const COACH_PLATFORMS: [&str; 2] = [SCIOTTE_TRAININGPEAKS, INTERVALS_ICU];

/// One of the coach's groups, as the checklist names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoachGroup {
    /// The group's id.
    pub id: Uuid,
    /// The group's name.
    pub name: String,
}

/// The first person who joined one of the coach's groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedAthlete {
    /// Their display name, else their email, as the group's member list shows it.
    pub name: Option<String>,
    /// The group they joined.
    pub group_id: Uuid,
    /// When they joined it.
    pub joined_at: Timestamp,
}

/// What the coach's steps are decided on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoachFacts {
    /// The coach holds coach access (`manages_roster`).
    pub manages_roster: bool,
    /// The coach's groups, oldest first: the coachless ones they own in this
    /// tenant, then the ones they coach.
    pub groups: Vec<CoachGroup>,
    /// The first person who joined one of those groups, other than the coach.
    pub first_athlete: Option<JoinedAthlete>,
    /// The first coaching platform (in [`COACH_PLATFORMS`] order) the coach
    /// holds a connection of their own to.
    pub platform: Option<&'static str>,
}

/// One step of the coach's checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoachStep {
    /// [`COACH_SEAT_STEP`], [`INVITE_ATHLETE_STEP`] or [`COACH_PLATFORM_STEP`].
    pub id: &'static str,
    /// The step's place in the list, from 1.
    pub order: usize,
    /// Done.
    pub done: bool,
    /// The first step not done.
    pub current: bool,
}

/// The coach's checklist, « Préparer ton coaching ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoachSetup {
    /// The seat, the athlete invited, the coaching platform, in that order.
    pub steps: Vec<CoachStep>,
    /// Steps done.
    pub done_count: usize,
    /// Steps in all.
    pub total: usize,
}

/// The coach's checklist for `facts`: each step done from the state it names,
/// and the first step not done is the current one.
#[must_use]
pub fn coach_setup(facts: &CoachFacts) -> CoachSetup {
    let decided = [
        (COACH_SEAT_STEP, facts.manages_roster),
        (INVITE_ATHLETE_STEP, facts.first_athlete.is_some()),
        (COACH_PLATFORM_STEP, facts.platform.is_some()),
    ];
    let current = decided.iter().position(|(_, done)| !done);
    let steps: Vec<CoachStep> = decided
        .iter()
        .enumerate()
        .map(|(slot, (id, done))| CoachStep {
            id,
            order: slot + 1,
            done: *done,
            current: current == Some(slot),
        })
        .collect();
    let done_count = steps.iter().filter(|step| step.done).count();
    let total = steps.len();
    CoachSetup {
        steps,
        done_count,
        total,
    }
}

impl CoachFacts {
    /// Read `user`'s coach facts under `tenant`.
    ///
    /// The groups are the coachless ones the user owns in `tenant` — those a
    /// grant attaches them to (carnet#1521) — and the ones they already coach,
    /// each once. A group the user owns that has another coach is not theirs
    /// to coach and is left out. Members are read per group: a group's
    /// membership spans tenants, as its invite does.
    ///
    /// # Errors
    ///
    /// Propagates the repository error: a checklist built on a partial read
    /// could tick a step that is not done.
    pub async fn read(repos: &RepositoryRegistry, tenant: TenantId, user: Uuid) -> AppResult<Self> {
        let manages_roster = repos
            .users
            .get_global(user)
            .await?
            .is_some_and(|account| account.manages_roster);

        let mut owned: Vec<CoachingGroup> = repos
            .groups
            .list_coachless_groups_owned_by(user, tenant)
            .await?;
        for coached in repos.groups.list_groups_coached_by(user).await? {
            if !owned.iter().any(|group| group.id == coached.id) {
                owned.push(coached);
            }
        }
        owned.sort_by_key(|group| group.created_at);

        let mut first_athlete: Option<JoinedAthlete> = None;
        for group in &owned {
            let members = repos.groups.list_members(&group.id.to_string()).await?;
            let joined = members
                .into_iter()
                .filter(|member| member.user_id != user && member.user_id != group.owner_id)
                .min_by_key(|member| member.joined_at);
            if let Some(member) = joined {
                if first_athlete
                    .as_ref()
                    .is_none_or(|first| member.joined_at < first.joined_at)
                {
                    first_athlete = Some(JoinedAthlete {
                        name: member.display_name,
                        group_id: group.id,
                        joined_at: member.joined_at,
                    });
                }
            }
        }

        // A connection of the coach's own: a synthetic seed or a read through
        // someone else's credential is no account the coach linked.
        let connections = repos
            .provider_connections
            .get_for_user(user, Some(tenant))
            .await?;
        let platform = COACH_PLATFORMS.into_iter().find(|platform| {
            connections.iter().any(|connection| {
                connection.provider == *platform
                    && !matches!(
                        connection.connection_type,
                        ConnectionType::Synthetic | ConnectionType::Delegated
                    )
            })
        });

        Ok(Self {
            manages_roster,
            groups: owned
                .into_iter()
                .map(|group| CoachGroup {
                    id: group.id,
                    name: group.name,
                })
                .collect(),
            first_athlete,
            platform,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined() -> JoinedAthlete {
        JoinedAthlete {
            name: Some("Léa Gagnon".to_owned()),
            group_id: Uuid::nil(),
            joined_at: Timestamp::parse_from_rfc3339("2026-11-10T09:00:00Z").expect("instant"),
        }
    }

    fn ids(setup: &CoachSetup) -> Vec<&str> {
        setup.steps.iter().map(|step| step.id).collect()
    }

    fn step<'a>(setup: &'a CoachSetup, id: &str) -> &'a CoachStep {
        setup
            .steps
            .iter()
            .find(|step| step.id == id)
            .unwrap_or_else(|| panic!("a {id} step"))
    }

    #[test]
    fn the_steps_follow_the_board_seat_then_invite_then_platform() {
        let setup = coach_setup(&CoachFacts::default());
        assert_eq!(
            ids(&setup),
            [COACH_SEAT_STEP, INVITE_ATHLETE_STEP, COACH_PLATFORM_STEP]
        );
        assert_eq!(
            setup
                .steps
                .iter()
                .map(|step| step.order)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(setup.total, 3);
    }

    #[test]
    fn nothing_done_makes_the_seat_current() {
        let setup = coach_setup(&CoachFacts::default());
        assert_eq!(setup.done_count, 0);
        assert!(step(&setup, COACH_SEAT_STEP).current);
        assert!(!step(&setup, INVITE_ATHLETE_STEP).current);
        assert!(!step(&setup, COACH_PLATFORM_STEP).current);
    }

    #[test]
    fn a_joined_athlete_ticks_the_invite_and_the_seat_stays_current() {
        let setup = coach_setup(&CoachFacts {
            first_athlete: Some(joined()),
            ..CoachFacts::default()
        });
        assert_eq!(setup.done_count, 1);
        assert!(step(&setup, INVITE_ATHLETE_STEP).done);
        assert!(step(&setup, COACH_SEAT_STEP).current);
    }

    #[test]
    fn a_group_with_nobody_in_it_is_no_invited_athlete() {
        let setup = coach_setup(&CoachFacts {
            groups: vec![CoachGroup {
                id: Uuid::nil(),
                name: "Léa · Marathon".to_owned(),
            }],
            ..CoachFacts::default()
        });
        assert!(!step(&setup, INVITE_ATHLETE_STEP).done);
    }

    #[test]
    fn the_seat_done_moves_current_to_the_first_step_still_open() {
        let setup = coach_setup(&CoachFacts {
            manages_roster: true,
            platform: Some(INTERVALS_ICU),
            ..CoachFacts::default()
        });
        assert_eq!(setup.done_count, 2);
        assert!(step(&setup, COACH_SEAT_STEP).done);
        assert!(step(&setup, COACH_PLATFORM_STEP).done);
        assert!(step(&setup, INVITE_ATHLETE_STEP).current);
    }

    #[test]
    fn every_step_done_leaves_no_current_step() {
        let setup = coach_setup(&CoachFacts {
            manages_roster: true,
            groups: Vec::new(),
            first_athlete: Some(joined()),
            platform: Some(SCIOTTE_TRAININGPEAKS),
        });
        assert_eq!(setup.done_count, setup.total);
        assert!(setup.steps.iter().all(|step| !step.current));
    }
}
