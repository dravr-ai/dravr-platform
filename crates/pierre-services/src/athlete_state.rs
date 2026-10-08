// ABOUTME: The athlete's state as use-case starters read it: providers, activities, dossier, season, plan and stage
// ABOUTME: One reader for the starter ranker and GET /api/me/onboarding-status, built under ai_scope so it carries its policy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Athlete state
//!
//! What a welcome needs to know to offer a starter the athlete can use
//! (carnet#828): whether a provider is connected, whether activities exist and
//! over how many weeks, what the dossier holds, whether a season and a current
//! plan week exist, whether a connection reports recovery or takes a pushed
//! plan, and how old the account is. [`AthleteState::holds`] answers each
//! catalogue [`Predicate`] from it.
//!
//! Every read is one the product already makes elsewhere, called the same way,
//! so a starter is offered exactly when the surface it leads to has something
//! to show: the plan through [`select_active_weeks`] as `/plan` reads it, the
//! calendar through [`resolve_calendar_target`] as the push tool resolves it,
//! the date through [`athlete_today`]. The dossier read is
//! [`dossier_coverage`], which `GET /api/me/onboarding-status` reads too.
//!
//! [`AthleteState::read`] runs under [`ai_scope::derived`], so the
//! [`TransportPolicy`] it returns is the strictest of what it read: a welcome
//! whose starters were chosen from Strava activities is stamped first-party
//! only, like any other content derived from them.

use std::collections::BTreeSet;

use chrono::{DateTime, Datelike, Duration, Utc};
use pierre_contremaitre::use_case_catalogue::{Predicate, Stage};
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, ConnectionType, CoverageMap, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::serving_backends;
use pierre_providers::registry::global_registry;
use uuid::Uuid;

use crate::athlete_clock::athlete_today;
use crate::plan_calendar_push::resolve_calendar_target;
use crate::training_plan_render::select_active_weeks;

/// The window `weeks_of_data` counts distinct ISO weeks of activity over.
const WEEKS_OF_DATA_WINDOW_DAYS: i64 = 28;

/// The most activities the weeks count reads. Newest first, so a cut drops the
/// oldest weeks of the window; it takes seven a day for four weeks to reach it.
const RECENT_ACTIVITY_LIMIT: i64 = 200;

/// An account younger than this is in its first session.
const FIRST_SESSION_HOURS: i64 = 24;

/// An account younger than this is in its first week.
const FIRST_WEEK_DAYS: i64 = 7;

/// Whether the athlete has activities we can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityHistory {
    /// At least one cached activity the athlete's AI may read.
    Present,
    /// A real provider is connected and nothing has been fetched from it yet:
    /// a fresh Strava athlete nobody has read. Counts as present for
    /// eligibility, since the first read will find them.
    NotYetRead,
    /// No activity, and either no provider or one that was read and had none.
    Absent,
}

/// What the dossier says the athlete has told us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DossierCoverage {
    /// Pillars with at least one fact.
    pub covered: usize,
    /// Every pillar is covered.
    pub complete: bool,
    /// Nothing at all: no pillar, no physiology, no goal.
    pub empty: bool,
}

/// Read the dossier's coverage for `(tenant, user_id)`.
///
/// Composed with the reader's own transport gates, so a fact this caller may
/// not read is not counted, and one first-party-only fact that is counted marks
/// the running derivation.
///
/// # Errors
///
/// Propagates the repository error when the dossier cannot be composed.
pub async fn dossier_coverage(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
) -> AppResult<DossierCoverage> {
    let dossier = repos
        .dossier
        .compose_dossier(
            tenant,
            user_id,
            ai_scope::readable_policy(),
            &ai_scope::admit_derived,
        )
        .await?;
    let coverage = CoverageMap::from_dossier(&dossier);
    let covered = coverage.covered_count();
    Ok(DossierCoverage {
        covered,
        complete: coverage.is_complete(),
        empty: covered == 0 && dossier.physiology.is_none() && dossier.goals.is_empty(),
    })
}

/// The athlete's state, as the use-case starters read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AthleteState {
    /// How old the account is, as a catalogue stage.
    pub stage: Stage,
    /// A real (non-synthetic) provider is connected.
    pub has_provider: bool,
    /// Whether activities exist.
    pub activities: ActivityHistory,
    /// Distinct ISO weeks with an activity in the last 28 days.
    pub weeks_of_data: u8,
    /// What the dossier holds.
    pub dossier: DossierCoverage,
    /// A season plan is active.
    pub season_set: bool,
    /// The active plan has a week covering today, in the athlete's zone.
    pub plan_active: bool,
    /// A connected provider reports sleep or recovery.
    pub recovery_source: bool,
    /// A connected provider takes a pushed training plan.
    pub calendar_writable: bool,
}

impl AthleteState {
    /// Read the state of `user_id` under `tenant` at `now`, with the policy
    /// anything derived from it must be stamped with.
    ///
    /// # Errors
    ///
    /// Propagates the first repository error. Every read is needed to say
    /// which starters fit, so a caller falls back to its static ones rather
    /// than ranking on a partial state.
    pub async fn read(
        repos: &RepositoryRegistry,
        tenant: TenantId,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<(Self, TransportPolicy)> {
        let (state, policy) = ai_scope::derived(read_untracked(repos, tenant, user_id, now)).await;
        Ok((state?, policy))
    }

    /// Whether `predicate` holds for this athlete.
    #[must_use]
    pub const fn holds(&self, predicate: Predicate) -> bool {
        match predicate {
            Predicate::CalendarWritable => self.calendar_writable,
            Predicate::DossierEmpty => self.dossier.empty,
            Predicate::HasActivities => !matches!(self.activities, ActivityHistory::Absent),
            Predicate::HasRecoverySource => self.recovery_source,
            Predicate::NoProvider => !self.has_provider,
            Predicate::NoSeason => !self.season_set,
            Predicate::PillarsDone => self.dossier.complete,
            Predicate::PlanActive => self.plan_active,
            Predicate::SeasonSet => self.season_set,
            Predicate::WeeksOfData(weeks) => self.weeks_of_data >= weeks,
        }
    }
}

/// The catalogue stage of an account created at `created_at`; an unknown
/// creation time reads as an established account.
fn stage_at(created_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Stage {
    match created_at.map(|created| now.signed_duration_since(created)) {
        Some(age) if age < Duration::hours(FIRST_SESSION_HOURS) => Stage::FirstSession,
        Some(age) if age < Duration::days(FIRST_WEEK_DAYS) => Stage::FirstWeek,
        _ => Stage::Any,
    }
}

/// Distinct ISO weeks among `activities`, saturating at `u8::MAX`.
fn distinct_weeks(activities: &[Activity]) -> u8 {
    let weeks: BTreeSet<(i32, u32)> = activities
        .iter()
        .map(|activity| {
            let week = activity.start_date().iso_week();
            (week.year(), week.week())
        })
        .collect();
    u8::try_from(weeks.len()).unwrap_or(u8::MAX)
}

async fn read_untracked(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> AppResult<AthleteState> {
    let created_at = repos
        .users
        .get_global(user_id)
        .await?
        .map(|user| user.created_at);

    let connections = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await?;
    let connected: Vec<&str> = connections.iter().map(|c| c.provider.as_str()).collect();
    let real: Vec<&str> = connections
        .iter()
        .filter(|c| c.connection_type != ConnectionType::Synthetic)
        .map(|c| c.provider.as_str())
        .collect();
    let registry = global_registry();
    let recovery_source = real
        .iter()
        .flat_map(|provider| serving_backends(provider))
        .any(|backend| registry.supports_sleep(&backend) || registry.supports_recovery(&backend));
    let calendar_writable = resolve_calendar_target(&registry, &connected, None).is_ok();

    // Through the terms filter, as every model-facing read: a session a
    // provider keeps from AI is no ground for "my last workout", and one kept
    // first-party marks this derivation.
    let recent = repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant,
            None,
            now - Duration::days(WEEKS_OF_DATA_WINDOW_DAYS),
            now,
            RECENT_ACTIVITY_LIMIT,
        )
        .await?;
    let recent = ai_scope::filter_activities(registry.as_ref(), recent);
    let activities = if recent.is_empty() {
        let older = repos
            .activity_cache
            .get_cached_activities(user_id, &tenant, None, DateTime::UNIX_EPOCH, now, 1)
            .await?;
        if !ai_scope::filter_activities(registry.as_ref(), older).is_empty() {
            ActivityHistory::Present
        } else if !real.is_empty()
            && repos
                .activity_cache
                .latest_activity_sync_any(user_id, &tenant)
                .await?
                .is_none()
        {
            ActivityHistory::NotYetRead
        } else {
            ActivityHistory::Absent
        }
    } else {
        ActivityHistory::Present
    };

    let tenant_key = tenant.to_string();
    let user_key = user_id.to_string();
    let plan = repos
        .training_plans
        .get_active_plan(&tenant_key, &user_key)
        .await?
        .filter(|plan| ai_scope::admit_derived(plan.transport_policy));
    let plan_active = match &plan {
        Some(plan) => {
            let weeks = repos
                .training_plans
                .list_plan_weeks(&tenant_key, &user_key, &plan.id, false)
                .await?;
            let today = athlete_today(repos, user_id).await;
            select_active_weeks(&weeks, today, 1)
                .weeks
                .first()
                .is_some_and(|week| week.is_current)
        }
        None => false,
    };

    Ok(AthleteState {
        stage: stage_at(created_at, now),
        has_provider: !real.is_empty(),
        activities,
        weeks_of_data: distinct_weeks(&recent),
        dossier: dossier_coverage(repos, tenant, user_id).await?,
        season_set: plan.is_some(),
        plan_active,
        recovery_source,
        calendar_writable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn state() -> AthleteState {
        AthleteState {
            stage: Stage::Any,
            has_provider: false,
            activities: ActivityHistory::Absent,
            weeks_of_data: 0,
            dossier: DossierCoverage {
                covered: 0,
                complete: false,
                empty: true,
            },
            season_set: false,
            plan_active: false,
            recovery_source: false,
            calendar_writable: false,
        }
    }

    #[test]
    fn an_account_ages_through_the_three_stages() {
        let now = Utc::now();
        let at = |age: Duration| stage_at(Some(now - age), now);
        assert_eq!(at(Duration::hours(1)), Stage::FirstSession);
        assert_eq!(at(Duration::hours(23)), Stage::FirstSession);
        assert_eq!(at(Duration::hours(24)), Stage::FirstWeek);
        assert_eq!(at(Duration::days(6)), Stage::FirstWeek);
        assert_eq!(at(Duration::days(7)), Stage::Any);
        assert_eq!(stage_at(None, now), Stage::Any);
    }

    #[test]
    fn a_new_athlete_with_nothing_connected_holds_only_the_setup_predicates() {
        let fresh = state();
        let holding: Vec<&str> = Predicate::VOCABULARY
            .iter()
            .map(|(name, threshold)| {
                let text = if *threshold {
                    format!("{name}>=1")
                } else {
                    (*name).to_owned()
                };
                Predicate::parse(&text).expect(name)
            })
            .filter(|p| fresh.holds(*p))
            .map(Predicate::name)
            .collect();
        assert_eq!(holding, ["dossier_empty", "no_provider", "no_season"]);
    }

    #[test]
    fn a_provider_nobody_has_read_yet_counts_as_having_activities() {
        let connected = AthleteState {
            has_provider: true,
            activities: ActivityHistory::NotYetRead,
            ..state()
        };
        assert!(connected.holds(Predicate::HasActivities));
        assert!(!connected.holds(Predicate::NoProvider));
        assert!(!connected.holds(Predicate::WeeksOfData(1)));
        assert!(!state().holds(Predicate::HasActivities));
    }

    #[test]
    fn weeks_of_data_is_a_floor() {
        let three = AthleteState {
            weeks_of_data: 3,
            ..state()
        };
        assert!(three.holds(Predicate::WeeksOfData(2)));
        assert!(three.holds(Predicate::WeeksOfData(3)));
        assert!(!three.holds(Predicate::WeeksOfData(4)));
    }
}
