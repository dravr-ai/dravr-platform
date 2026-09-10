// ABOUTME: The readiness rail — an athlete's alerts and load read into a ladder level, and the week measured against it
// ABOUTME: Gathers the series the nine alerts are derived from, then reports which saved days the level no longer allows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Readiness-driven substitution, on the daily-adjust path.
//!
//! A week-only `save_training_plan` is how an adjustment reaches the
//! platform. This runs after the commit, beside the compliance rail, and
//! answers the question the compliance rail deliberately does not: not *was
//! this week built to its phase*, but *can this athlete run it today*.
//!
//! The kernel owns both halves of the decision — which alerts the athlete's
//! series raise, and which ladder level those alerts clear. This gathers the
//! series and reports the verdict; it decides nothing itself, so the
//! thresholds cannot drift from the ones the agent bodies publish.
//!
//! Advisory, like the compliance rail: it reports and never refuses. A
//! substitution the athlete disagrees with is a conversation, not a rejected
//! save.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use pierre_core::models::periodization::{
    alerts, readiness_level, substitute, AlertInput, DayReadinessInput, Flavour, LoadDay,
    PhaseKind, ReadinessInput, ReadinessLevel, RecoveryDay, SubstitutionVerdict, TrainingAlert,
    WorkoutPurpose,
};
use pierre_core::models::{FormReading, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek, TrainingPlan};
use pierre_services::coach_package::{load_coach_package, PackagedCatalogue};
use tracing::{info, warn};
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// Days of history the alert baselines are drawn from.
///
/// The taxonomy names a 28-day baseline for the load signals; the recovery
/// signals read a shorter window, and taking one span for both keeps this to
/// a single query per source.
const BASELINE_DAYS: i64 = 28;

/// Seconds in an hour, for the nightly sleep total.
const SECONDS_PER_HOUR: f64 = 3600.0;

/// What the ladder said about an athlete and the weeks it was read against.
///
/// Returned rather than only logged: the rails compute a verdict on every
/// save and, until this existed, threw it away into a log line no agent
/// reads — which is what both ledger entries mean by "nothing warns the
/// agent". A caller that wants the verdict asks for it; the rail's own
/// reporting is unchanged.
pub struct ReadinessReading {
    /// The level the athlete's signals cleared.
    pub level: ReadinessLevel,
    /// The alert labels the athlete's series raised.
    pub alerts: BTreeSet<TrainingAlert>,
    /// One verdict per week read, in the order they were given.
    pub weeks: Vec<(String, SubstitutionVerdict)>,
}

/// Read the ladder for one athlete and measure some weeks against it.
///
/// Best-effort throughout: a source that cannot be read narrows what the
/// ladder can see, and a level read from less is still worth reporting.
/// `None` means the ladder could not be read at all — no flavour, or a
/// flavour this athlete's catalogue does not carry — which is not the same
/// as a clear week.
pub(super) async fn read_ladder(
    state: &Arc<dyn ToolRuntime>,
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    plan: &TrainingPlan,
    weeks: &[PlanWeek],
) -> Option<ReadinessReading> {
    let selection = plan.flavour.as_ref()?;

    let package = match load_coach_package(repos, tenant, user_id, plan.coach_slug.as_deref()).await
    {
        Ok(package) => package,
        Err(e) => {
            warn!(error = %e, "week readiness: coach package unreadable; catalogue alone");
            None
        }
    };
    let catalogue = PackagedCatalogue::new(state.training_catalogue(), package);
    let Some((flavour, _)) = catalogue.flavour(&selection.id) else {
        warn!(
            flavour = %selection.id,
            "week readiness: the plan's flavour is not in this athlete's catalogue"
        );
        return None;
    };

    let today = Utc::now().date_naive();
    let input = gather(repos, tenant, user_id, today).await;
    let raised = alerts(&input.alert_input);
    let level = readiness_level(&ReadinessInput {
        form_pct: input.form_pct,
        acwr: input.alert_input.today.acwr,
        monotony: input.alert_input.today.monotony,
        ramp_rate: input.alert_input.today.ramp_rate,
        alerts: raised.clone(),
        // No feed publishes an acute injury: it reaches a plan through the
        // athlete telling the agent, not through a series.
        injury: false,
    });

    Some(ReadinessReading {
        level,
        alerts: raised,
        weeks: weeks
            .iter()
            .map(|week| {
                // Per week, not once for the set: the phase *this* week sits
                // in decides what its substitution may reach for. Reading the
                // first week's phase for all of them substituted a taper week
                // against the build's session mix on any plan whose weeks
                // span a boundary — the same mistake the prompt's phase
                // header made until it was fixed to render every phase the
                // fortnight touches.
                let preferred = week
                    .phase_index
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| plan.phases.get(i))
                    .map(|phase| phase_purposes(&flavour, phase.kind))
                    .unwrap_or_default();
                let days = week_days(&catalogue, week);
                (
                    week.week_start.clone(),
                    substitute(&flavour, level, &days, &preferred),
                )
            })
            .collect(),
    })
}

/// Read the ladder and log what it said, on the daily-adjust path.
pub(super) async fn emit_week_readiness(
    state: &Arc<dyn ToolRuntime>,
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    plan: &TrainingPlan,
    saved: &[PlanWeek],
) {
    let Some(reading) = read_ladder(state, repos, tenant, user_id, plan, saved).await else {
        return;
    };
    for (week_start, verdict) in &reading.weeks {
        report(week_start, verdict, &reading.alerts);
    }
}

/// The purposes a phase asks for, heaviest first.
fn phase_purposes(flavour: &Flavour, kind: PhaseKind) -> Vec<WorkoutPurpose> {
    let Some(mix) = flavour.session_mix.get(&kind) else {
        return Vec::new();
    };
    let mut weighted: Vec<(&WorkoutPurpose, &u8)> = mix.iter().collect();
    weighted.sort_by(|a, b| b.1.cmp(a.1));
    weighted.into_iter().map(|(purpose, _)| *purpose).collect()
}

/// One week's days as the ladder reads them.
fn week_days(catalogue: &PackagedCatalogue<'_>, week: &PlanWeek) -> Vec<DayReadinessInput> {
    week.days
        .iter()
        .filter(|day| !day.is_rest())
        .map(|day| DayReadinessInput {
            date: day.date.clone(),
            purpose: day
                .template_slug
                .as_ref()
                .and_then(|slug| catalogue.workout(slug))
                .map(|(template, _)| template.purpose),
        })
        .collect()
}

/// What the alerts and the ladder are read from.
struct Gathered {
    alert_input: AlertInput,
    form_pct: Option<f64>,
}

/// Read every series the nine alerts are derived from.
async fn gather(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    today: NaiveDate,
) -> Gathered {
    let from = today - Duration::days(BASELINE_DAYS);
    let history = repos
        .training_history
        .get_training_history(tenant, user_id, from, today)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "week readiness: training history unreadable");
            Vec::new()
        });

    let latest = history.iter().max_by_key(|day| day.date);
    let form_pct = latest.and_then(|day| FormReading::new(day.ctl, day.atl, day.tsb).form_pct);
    let load = latest.map_or_else(LoadDay::default, |day| LoadDay {
        acwr: day.acwr,
        monotony: day.monotony,
        strain: day.strain,
        ramp_rate: day.ramp_rate,
    });
    // The baseline excludes the day it judges, or today drags the mean toward
    // itself and blunts the very breach the alert exists to catch.
    let strain_baseline: Vec<f64> = latest.map_or_else(Vec::new, |newest| {
        history
            .iter()
            .filter(|day| day.date != newest.date)
            .filter_map(|day| day.strain)
            .collect()
    });

    let recovery = recovery_series(repos, tenant, user_id, today, from).await;

    Gathered {
        alert_input: AlertInput {
            today: load,
            strain_baseline,
            recovery,
            // The week's distribution and the athlete's threshold age are
            // read by the compliance rail and the calibration walk
            // respectively; leaving them unset here raises neither label
            // rather than raising a wrong one.
            above_lt2_share: None,
            threshold_age_days: None,
        },
        form_pct,
    }
}

/// The recovery readings behind the three corroborating signals.
async fn recovery_series(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    today: NaiveDate,
    from: NaiveDate,
) -> Vec<RecoveryDay> {
    let (Some(start), Some(end)) = (
        from.and_hms_opt(0, 0, 0).map(|t| t.and_utc()),
        today.and_hms_opt(23, 59, 59).map(|t| t.and_utc()),
    ) else {
        return Vec::new();
    };
    let sleep = sleep_by_night(repos, tenant, user_id, start, end).await;
    let metrics = repos
        .recovery
        .get_recovery_metrics(user_id, &tenant, start, end)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "week readiness: recovery metrics unreadable");
            Vec::new()
        });
    metrics
        .iter()
        .filter_map(|m| {
            let days_ago = today.signed_duration_since(m.date).num_days();
            u32::try_from(days_ago).ok().map(|days_ago| RecoveryDay {
                days_ago,
                hrv_rmssd: m.hrv_rmssd,
                resting_heart_rate: m.resting_heart_rate.map(f64::from),
                sleep_hours: sleep.get(&m.date).copied(),
            })
        })
        .collect()
}

/// Hours slept per night, keyed by the night's own date.
///
/// Naps are left out: the taxonomy's deficit is "under seven hours a night",
/// which is a nightly figure, and folding a twenty-minute afternoon sleep in
/// as its own night would drag the mean below the threshold on a week the
/// athlete slept perfectly well.
async fn sleep_by_night(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> HashMap<NaiveDate, f64> {
    let sessions = repos
        .sleep
        .get_sleep_sessions(user_id, &tenant, start, end)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "week readiness: sleep sessions unreadable");
            Vec::new()
        });
    let mut by_night: HashMap<NaiveDate, f64> = HashMap::new();
    for session in sessions.iter().filter(|s| !s.is_nap) {
        let Some(seconds) = session.total_sleep_seconds else {
            continue;
        };
        // A night is dated by when the athlete woke, so a sleep starting
        // before midnight belongs to the morning it ended on.
        let night = session.end_datetime.date_naive();
        *by_night.entry(night).or_insert(0.0) += f64::from(seconds) / SECONDS_PER_HOUR;
    }
    by_night
}

/// Log what the ladder said about one week.
///
/// Log-only for the same reason the compliance rail is: the rule has never
/// run against real traffic, and a rail that refuses before anyone has read
/// what it would have refused is a rail nobody can argue with.
fn report(week_start: &str, verdict: &SubstitutionVerdict, raised: &BTreeSet<TrainingAlert>) {
    let labels: Vec<&str> = raised.iter().map(|a| a.as_str()).collect();
    let swapped: Vec<&str> = verdict
        .substitutions
        .iter()
        .map(|s| s.date.as_str())
        .collect();
    info!(
        week_start = %week_start,
        readiness = verdict.level.as_str(),
        alerts = ?labels,
        substitutions = ?swapped,
        hard_sessions = verdict.hard_sessions,
        hard_sessions_allowed = verdict.hard_sessions_allowed,
        unclassified_days = verdict.unclassified_days,
        clear = verdict.is_clear(),
        "week readiness assessed"
    );
}

/// Whether a week is worth reading the ladder for at all.
///
/// A week already run is history: substituting a session the athlete
/// completed on Tuesday helps nobody. Only days from today forward can be
/// changed, so a week that ended before today is skipped.
pub(super) fn week_is_actionable(week: &PlanWeek, today: NaiveDate) -> bool {
    parse_plan_date(&week.week_start).is_some_and(|start| start + Duration::days(6) >= today)
}
