// ABOUTME: The readiness rail — an athlete's alerts and load read into a ladder level, and the week measured against it
// ABOUTME: Gathers the series the nine alerts are derived from, reports the days the level moves, and what the bank offers instead

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
//! series raise, and which ladder level those alerts clear — and names why
//! each day moves. This gathers the series, hands the kernel each day's
//! purpose and template floor, and answers the one thing the kernel cannot
//! see: which templates the athlete's bank holds. It decides nothing about
//! readiness itself, so the thresholds cannot drift from the ones the agent
//! bodies publish.
//!
//! Advisory, like the compliance rail: it reports and never refuses. A
//! substitution the athlete disagrees with is a conversation, not a rejected
//! save.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use pierre_core::models::periodization::alerts::BASELINE_DAYS;
use pierre_core::models::periodization::{
    alerts, ladder, readiness_level, substitute, AlertInput, DayReadinessInput, DaySubstitution,
    Flavour, LoadDay, PhaseKind, ReadinessInput, ReadinessLevel, RecoveryDay, StrainDay,
    SubstitutionReason, TrainingAlert, WorkoutFilter, WorkoutPurpose, WorkoutTemplate,
};
use pierre_core::models::{merge_recovery_metrics, merge_sleep_sessions, SportType, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek, TrainingPlan};
use pierre_services::agent_package::{load_agent_package, PackagedCatalogue};
use tracing::{info, warn};
use uuid::Uuid;

use crate::implementations::training_plans_output::{
    DayReplacement, ReadinessSubstitution, ReplacementBasis, WeekReadiness,
};
use crate::runtime::ToolRuntime;

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
    /// One reading per week read, in the order they were given.
    pub weeks: Vec<WeekReadiness>,
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

    let package =
        match load_agent_package(repos, tenant, user_id, plan.author_agent_id.as_deref()).await {
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
    let bank = Bank {
        catalogue: &catalogue,
        level,
        allowed: ladder(&flavour)
            .get(&level)
            .map_or(&[], |rule| rule.purposes.as_slice()),
    };

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
                let phase = week
                    .phase_index
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| plan.phases.get(i))
                    .map(|phase| phase.kind);
                let preferred = phase
                    .map(|kind| phase_purposes(&flavour, kind))
                    .unwrap_or_default();
                let (days, templates): (Vec<_>, Vec<_>) =
                    week_days(&catalogue, week).into_iter().unzip();
                let verdict = substitute(&flavour, level, &days, &preferred);
                WeekReadiness {
                    week_start: week.week_start.clone(),
                    level: verdict.level,
                    substitutions: bank.answer(
                        verdict.substitutions,
                        &days,
                        &templates,
                        phase,
                        &preferred,
                    ),
                    hard_sessions: verdict.hard_sessions,
                    hard_sessions_allowed: verdict.hard_sessions_allowed,
                    unclassified_days: verdict.unclassified_days,
                }
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
    for week in &reading.weeks {
        report(week, &reading.alerts);
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

/// One week's days as the ladder reads them, each beside the template it
/// resolved to in the athlete's bank.
///
/// The kernel reads a day's purpose and its template's floor; the template
/// itself stays here, because what an easier session looks like — which
/// sport it is written for — is the bank's question, not the ladder's.
fn week_days(
    catalogue: &PackagedCatalogue<'_>,
    week: &PlanWeek,
) -> Vec<(DayReadinessInput, Option<WorkoutTemplate>)> {
    week.days
        .iter()
        .filter(|day| !day.is_rest())
        .map(|day| {
            let template = day
                .template_slug
                .as_ref()
                .and_then(|slug| catalogue.workout(slug))
                .map(|(template, _)| template);
            let input = DayReadinessInput {
                date: day.date.clone(),
                purpose: template.as_ref().map(|t| t.purpose),
                readiness_min: template.as_ref().map(|t| t.fit.readiness_min),
            };
            (input, template)
        })
        .collect()
}

/// The athlete's template bank — the agent's package over the catalogue —
/// read against the level the athlete cleared today.
struct Bank<'a> {
    catalogue: &'a PackagedCatalogue<'a>,
    level: ReadinessLevel,
    /// The purposes the flavour's ladder opens at `level`.
    allowed: &'a [WorkoutPurpose],
}

impl Bank<'_> {
    /// Each substitution the kernel made, with what the bank offers in its
    /// place.
    ///
    /// The kernel keeps the week's order and skips only days no template
    /// resolved for, so one walk over the days pairs each substitution with
    /// the template its day resolved to — by position, which still holds on a
    /// date that carries two sessions.
    fn answer(
        &self,
        substitutions: Vec<DaySubstitution>,
        days: &[DayReadinessInput],
        templates: &[Option<WorkoutTemplate>],
        phase: Option<PhaseKind>,
        preferred: &[WorkoutPurpose],
    ) -> Vec<ReadinessSubstitution> {
        let mut walk = days.iter().zip(templates);
        substitutions
            .into_iter()
            .map(|day| {
                let template = walk
                    .find(|(input, _)| input.date == day.date && input.purpose == Some(day.from))
                    .and_then(|(_, template)| template.as_ref());
                let replacement = match (day.reason, template) {
                    (SubstitutionReason::TemplateAboveLevel, Some(template)) => {
                        Some(self.replacement(template, phase, preferred))
                    }
                    _ => None,
                };
                ReadinessSubstitution { day, replacement }
            })
            .collect()
    }

    /// What to run on a day whose template sits above today's level.
    ///
    /// An easier template of the same purpose when the bank holds one; when
    /// it holds none, the substitution the level allows for a closed purpose
    /// — the phase's heaviest purpose the level opens, other than the day's
    /// own — with a template of it when the bank holds one.
    fn replacement(
        &self,
        template: &WorkoutTemplate,
        phase: Option<PhaseKind>,
        preferred: &[WorkoutPurpose],
    ) -> DayReplacement {
        if let Some(easier) = self.fitting(template.purpose, &template.sport, phase, &template.slug)
        {
            return DayReplacement {
                basis: ReplacementBasis::EasierTemplate,
                purpose: Some(template.purpose),
                template: Some(easier),
            };
        }
        let purpose = preferred
            .iter()
            .copied()
            .find(|candidate| *candidate != template.purpose && self.allowed.contains(candidate));
        DayReplacement {
            basis: ReplacementBasis::PurposeFallback,
            purpose,
            template: purpose.and_then(|p| self.fitting(p, &template.sport, phase, &template.slug)),
        }
    }

    /// The bank's closest template of `purpose` that today's level clears.
    ///
    /// Written for `sport` as its primary sport or a variant, and fitting
    /// `phase`. A template whose primary sport is `sport` comes before one
    /// that only lists it as a variant; then the highest floor at or below
    /// the level — the smallest step down from what the week asked for; then
    /// the bank's own order, package first.
    fn fitting(
        &self,
        purpose: WorkoutPurpose,
        sport: &SportType,
        phase: Option<PhaseKind>,
        other_than: &str,
    ) -> Option<String> {
        self.catalogue
            .workouts_matching(&WorkoutFilter {
                purpose: Some(purpose),
                phase,
                sport: Some(sport.clone()),
            })
            .into_iter()
            .filter(|t| t.slug != other_than && t.fit.readiness_min <= self.level)
            .min_by_key(|t| (t.sport != *sport, Reverse(t.fit.readiness_min)))
            .map(|t| t.slug)
    }
}

/// What the alerts and the ladder are read from.
struct Gathered {
    alert_input: AlertInput,
    form_pct: Option<f64>,
}

/// Read every series the nine alerts are derived from.
///
/// One span of history per source — the kernel's baseline, the longest
/// window any alert reads. The kernel windows each alert by `days_ago`
/// itself, so the week-long recovery alerts read their own seven days out of
/// the same span the 28-day baselines read.
async fn gather(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    today: NaiveDate,
) -> Gathered {
    let from = today - Duration::days(i64::from(BASELINE_DAYS));
    let history = repos
        .training_history
        .get_training_history(tenant, user_id, from, today)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "week readiness: training history unreadable");
            Vec::new()
        });

    let latest = history.iter().max_by_key(|day| day.date);
    let form_pct = latest.and_then(|day| day.form_reading().form_pct);
    let load = latest.map_or_else(LoadDay::default, |day| LoadDay {
        acwr: day.acwr,
        monotony: day.monotony,
        strain: day.strain,
        ramp_rate: day.ramp_rate,
    });
    // Dated from the day judged — the newest row, whose strain is `load`'s.
    // The kernel reads the baseline from the days before it and leaves that
    // day's own entry out, so today never drags the mean toward itself and
    // blunts the very breach the alert exists to catch.
    let strain_baseline: Vec<StrainDay> = latest.map_or_else(Vec::new, |judged| {
        history
            .iter()
            .filter_map(|day| {
                Some(StrainDay {
                    days_ago: days_ago(judged.date, day.date)?,
                    strain: day.strain?,
                })
            })
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

/// Whole days from `date` to `judged`; `None` for a date after it.
fn days_ago(judged: NaiveDate, date: NaiveDate) -> Option<u32> {
    u32::try_from(judged.signed_duration_since(date).num_days()).ok()
}

/// The recovery readings behind the three corroborating signals, one per
/// date.
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
    // One day per date: two providers reporting the same morning are one
    // reading, not two days of evidence.
    let mut days: BTreeMap<NaiveDate, RecoveryDay> = BTreeMap::new();
    for merged in merge_recovery_metrics(metrics) {
        let m = merged.record;
        if let Some(days_ago) = days_ago(today, m.date) {
            days.insert(
                m.date,
                RecoveryDay {
                    days_ago,
                    hrv_rmssd: m.hrv_rmssd,
                    resting_heart_rate: m.resting_heart_rate.map(f64::from),
                    sleep_hours: None,
                },
            );
        }
    }
    // A night with no morning metrics beside it is still a night: a wearable
    // that records sleep alone feeds the sleep alert on its own.
    for (night, hours) in sleep {
        if let Some(days_ago) = days_ago(today, night) {
            days.entry(night)
                .or_insert_with(|| RecoveryDay {
                    days_ago,
                    ..RecoveryDay::default()
                })
                .sleep_hours = Some(hours);
        }
    }
    days.into_values().collect()
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
    // Merged first, so one night two wearables both recorded counts once
    // instead of being summed into a double night.
    let mut by_night: HashMap<NaiveDate, f64> = HashMap::new();
    for merged in merge_sleep_sessions(sessions) {
        let session = merged.record;
        if session.is_nap {
            continue;
        }
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
fn report(week: &WeekReadiness, raised: &BTreeSet<TrainingAlert>) {
    let labels: Vec<&str> = raised.iter().map(|a| a.as_str()).collect();
    let swapped: Vec<&str> = week
        .substitutions
        .iter()
        .map(|s| s.day.date.as_str())
        .collect();
    let reasons: Vec<&str> = week
        .substitutions
        .iter()
        .map(|s| s.day.reason.as_str())
        .collect();
    info!(
        week_start = %week.week_start,
        readiness = week.level.as_str(),
        alerts = ?labels,
        substitutions = ?swapped,
        reasons = ?reasons,
        hard_sessions = week.hard_sessions,
        hard_sessions_allowed = week.hard_sessions_allowed,
        unclassified_days = week.unclassified_days,
        clear = week.is_clear(),
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
