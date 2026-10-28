// ABOUTME: Builds the athlete context a claim is judged against — the personalized snapshot and the athlete's own record
// ABOUTME: One builder for every surface: the chat pipeline's verification stage and the verify_claim tool read the same data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The athlete context of a claim
//!
//! Two of the claim checker's layers judge a claim against the athlete rather
//! than against the literature: the personalized-physiology layer (their
//! paces, zones and load) and the athlete-data layer (their own activities).
//! This module builds their inputs, and it is the only place that does: the
//! chat pipeline's verification stage checks Dravr's own replies with it, and
//! the `verify_claim` tool checks a claim an MCP client or an A2A agent is
//! about to state with it (carnet#852).
//!
//! Every read goes through the provider-terms filters of the call it runs in.
//! Inside a tool body served over an external transport, a first-party-only
//! provider's activities are dropped before the layers see them, so a verdict
//! can only ever rest on data that transport may carry.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use dravr_cageux::config::intelligence::{AlgorithmConfig, TrainingZonesConfig};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::civil_time::{clock_date, local_date, resolve_zone};
use pierre_core::constants::provider_capture::current_capture_version;
use pierre_core::models::{default_locale, Activity, TenantId};
use pierre_database::repositories::{historical_depth_covered, BackfillCoverage};
use pierre_database::RepositoryRegistry;
use pierre_evals::athlete_data::{
    AthleteRecord, BackfillDepth, CacheRead, HistoryInputs, PeriodHistory, RecordedActivity,
    SourceCoverage, SourceSessions, ATHLETE_RECORD_WINDOW,
};
use pierre_evals::{AthleteMetrics, PersonalizedConfig, ToleranceStrategy};
use pierre_memory::training_plans::PlanPhase;
use pierre_providers::ai_scope;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use tracing::warn;
use uuid::Uuid;

use crate::athlete_snapshot::build_athlete_metrics;
use crate::onboarding_gate::user_has_connected_provider;
use crate::training_history_read::resolve_compute_backends;

/// Cap on activities pulled for one verification pass.
///
/// The layer only needs the set of values a figure could match, and an athlete
/// with more than this in a quarter is not going to be adjudicated differently
/// by the tail. A read that fills it vouches for period totals only from the
/// day after its oldest row.
const ATHLETE_RECORD_LIMIT: i64 = 500;

/// Whose data a claim is judged against, and through which terms.
pub struct AthleteSources<'a> {
    /// Repositories the athlete's profile, connections and activities are read from.
    pub repos: &'a RepositoryRegistry,
    /// The provider terms every activity read is filtered by.
    pub policies: &'a dyn ProviderTerms,
    /// Tenant owning the athlete's data.
    pub tenant_id: TenantId,
    /// The athlete the claim is about.
    pub user_id: Uuid,
}

impl fmt::Debug for AthleteSources<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AthleteSources")
            .field("repos", &self.repos)
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}

/// Build the personalized-layer inputs: the athlete snapshot and the tolerance
/// strategy that decides a contradiction.
///
/// Returns `None` when `config` disables the layer, or when the snapshot is too
/// thin to trust ([`AthleteMetrics::is_usable`]), which keeps the layer silent.
pub async fn build_personalized_inputs(
    sources: &AthleteSources<'_>,
    algorithm_config: &AlgorithmConfig,
    training_zones: &TrainingZonesConfig,
    config: &PersonalizedConfig,
) -> Option<(AthleteMetrics, Box<dyn ToleranceStrategy>)> {
    if !config.enabled {
        return None;
    }
    let metrics = build_athlete_metrics(
        sources.repos,
        sources.policies,
        algorithm_config,
        training_zones,
        sources.tenant_id,
        sources.user_id,
    )
    .await;
    if !metrics.is_usable() {
        return None;
    }
    Some((metrics, config.tolerance_strategy()))
}

/// Gather what we hold about this athlete's own records.
///
/// Two facts, from two sources that disagree in an important way: whether any
/// provider is connected at all (`provider_connections`, the same table the
/// onboarding gate reads), and what activities we actually hold (the cache).
/// The first is what licenses a contradiction — no provider means a specific
/// figure had no source — while the second only ever supports or fails to
/// support one, because the cache is a window and can legitimately miss a run.
///
/// `locale` selects the relative-day vocabulary the layer reads ("hier" is
/// yesterday in French and here in German). `None` reads the athlete's own
/// account locale, for a caller that has no reply language to go by.
///
/// Returns `None` when the connection lookup *fails*. That means we cannot
/// establish the athlete's state, and this is the one layer where guessing is
/// dangerous in a specific direction: "providerless" is what licenses a
/// `Contradicted` verdict at 0.95 confidence, saying the agent invented a
/// figure. Collapsing an `Err` into `false` would let a pool timeout brand an
/// accurate reply a fabrication and persist that accusation. Skipping the
/// layer costs a verdict; guessing costs the athlete's trust, so absence of
/// knowledge must never read as knowledge of absence.
///
/// `has_provider` mirrors [`user_has_connected_provider`] plus the `oauth_tokens`
/// second source, exactly as the dispatch chokepoint does, because the two
/// tables are known to drift. An athlete holding a live token whose connection
/// row is missing is served by every tool, so calling them providerless here
/// would contradict every true figure they were just given.
///
/// For a connected athlete the record also carries the history a period total
/// is checked against (carnet#846): each workout once, each source's own
/// sessions, the days every live source holds in full, and the current plan
/// block. A failed cache or coverage read leaves it out, so no total is
/// contradicted on a guess.
pub async fn build_athlete_record(
    sources: &AthleteSources<'_>,
    locale: Option<&str>,
) -> Option<AthleteRecord> {
    let repos = sources.repos;
    let user_id = sources.user_id;

    let connected = match user_has_connected_provider(&repos.provider_connections, user_id).await {
        Ok(true) => true,
        Ok(false) => match repos.oauth_tokens.get_tokens(user_id, None).await {
            Ok(tokens) => !tokens.is_empty(),
            Err(e) => {
                warn!(
                    user_id = %user_id,
                    error = %e,
                    "athlete-record: oauth_tokens lookup failed — skipping the \
                     athlete-data layer rather than risk contradicting a true claim"
                );
                return None;
            }
        },
        Err(e) => {
            warn!(
                user_id = %user_id,
                error = %e,
                "athlete-record: provider_connections lookup failed — skipping the \
                 athlete-data layer rather than risk contradicting a true claim"
            );
            return None;
        }
    };

    // The athlete's own zone, so a 21:00 session belongs to the day they
    // trained. A claim about "Tuesday" is checked against their Tuesday, and a
    // claim about "hier" against their yesterday.
    let user = repos.users.get_global(user_id).await.ok().flatten();
    let user_timezone = user.as_ref().and_then(|u| u.timezone.clone());
    let locale = locale.map_or_else(
        || user.map_or_else(default_locale, |u| u.locale),
        str::to_owned,
    );
    let zone = resolve_zone(user_timezone.as_deref());
    let end = Utc::now();
    let today = clock_date(end, zone);

    // Without a provider there is nothing to fetch, and the window query would
    // be a guaranteed-empty round trip on every verified reply.
    if !connected {
        return Some(AthleteRecord::providerless(today, &locale));
    }

    let start = end - ATHLETE_RECORD_WINDOW;
    let (read, cache_read) =
        read_record_window(sources, start, end, |instant| local_date(instant, zone)).await;
    // A claim is checked against what the model could have seen (carnet#723),
    // and over an external transport against what that transport may carry
    // (carnet#724).
    let activities = ai_scope::filter_activities(sources.policies, read);

    let recorded = |a: &Activity| {
        #[allow(clippy::cast_precision_loss)]
        let duration_min = (a.duration_seconds() as f64) / 60.0;
        RecordedActivity {
            date: local_date(a.start_date(), zone),
            sport: a.sport_type().clone(),
            name: a.name().to_owned(),
            distance_km: a.distance_meters().map(|m| m / 1000.0),
            duration_min,
            elevation_m: a.elevation_gain(),
            average_hr_bpm: a.average_heart_rate().map(f64::from),
        }
    };
    let held: Vec<RecordedActivity> = activities.iter().map(recorded).collect();
    let by_source = sessions_by_source(&activities, recorded);
    // A total counts each workout once: the copies two providers hold of one
    // ride are merged the way the activity tools merge what the model reads.
    let (sessions, _) = merge_duplicates(activities, &DedupConfig::from_env());
    let (coverage, block_start) = tokio::join!(
        live_source_coverage(sources),
        current_block_start(sources, today),
    );
    let history = period_history(cache_read, coverage, |read, coverage| HistoryInputs {
        sessions: sessions.iter().map(recorded).collect(),
        by_source,
        read,
        sources: coverage,
        now: end,
        zone,
        block_start,
    });

    Some(AthleteRecord {
        has_provider: true,
        today,
        locale,
        activities: held,
        history,
    })
}

/// The athlete's cached activities from `from` to `to`, and what the read
/// reached, each instant placed on the athlete's calendar by `day`.
///
/// A failed read yields no rows and no [`CacheRead`], never an empty read: the
/// per-activity layer then has nothing to match, which never contradicts a
/// connected athlete, and [`period_history`] has no read to vouch from.
async fn read_record_window(
    sources: &AthleteSources<'_>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    day: impl Fn(DateTime<Utc>) -> NaiveDate,
) -> (Vec<Activity>, Option<CacheRead>) {
    let user_id = sources.user_id;
    let rows = match sources
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &sources.tenant_id,
            None,
            from,
            to,
            ATHLETE_RECORD_LIMIT,
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(
                user_id = %user_id,
                error = %e,
                "athlete-record: activity cache read failed — period totals stay unverifiable"
            );
            return (Vec::new(), None);
        }
    };
    // A read that filled its cap was cut inside its oldest day.
    let read_full = usize::try_from(ATHLETE_RECORD_LIMIT).is_ok_and(|cap| rows.len() >= cap);
    let read = CacheRead {
        from,
        cut_day: rows
            .iter()
            .map(|a| day(a.start_date()))
            .min()
            .filter(|_| read_full),
    };
    (rows, Some(read))
}

/// The history a period total is checked against, or `None` unless both the
/// activity-cache read and every live source's coverage read succeeded.
///
/// The read succeeding is part of what [`PeriodHistory::vouched`] depends on.
/// The sync marks say which days the cache holds in full, and the rows the
/// read returned are then taken as every session of those days. A failed read
/// taken as an empty one would vouch for a week of rest, and contradict every
/// correct total stated over it.
fn period_history<Z: TimeZone>(
    read: Option<CacheRead>,
    sources: Option<Vec<SourceCoverage>>,
    inputs: impl FnOnce(CacheRead, Vec<SourceCoverage>) -> HistoryInputs<Z>,
) -> Option<PeriodHistory> {
    let (read, sources) = read.zip(sources)?;
    PeriodHistory::vouched(inputs(read, sources))
}

/// Each source's own sessions, merged within the source, when more than one
/// source holds any; empty otherwise, since one source's sessions are the
/// merged ones.
///
/// A tool that reads one provider adds up that provider's week, so its total
/// is a reading of a period figure as much as the merged one is.
fn sessions_by_source(
    activities: &[Activity],
    recorded: impl Fn(&Activity) -> RecordedActivity,
) -> Vec<SourceSessions> {
    let mut grouped: BTreeMap<&str, Vec<Activity>> = BTreeMap::new();
    // Cloned because `merge_duplicates` takes its rows by value and the caller
    // still merges the whole list across sources afterwards.
    for activity in activities {
        grouped
            .entry(activity.provider())
            .or_default()
            .push(activity.clone());
    }
    if grouped.len() < 2 {
        return Vec::new();
    }
    grouped
        .into_iter()
        .map(|(source, rows)| {
            let (own, _) = merge_duplicates(rows, &DedupConfig::from_env());
            SourceSessions {
                source: source.to_owned(),
                sessions: own.iter().map(&recorded).collect(),
            }
        })
        .collect()
}

/// What each of the athlete's live activity sources vouches for: when its
/// list head was last read, and how deep its current capture's backfill
/// reached.
///
/// Live means connected, not waiting on a reconnect, and able to deliver
/// activities: the connections a training-history compute stands on
/// ([`resolve_compute_backends`]), under the canonical slug their cached rows,
/// fetch marks and backfill records are keyed by. A lapsed connection is left
/// out because nothing new arrives from it until the athlete reconnects, and
/// the athlete's uploads because they land in the cache the moment they are
/// kept.
///
/// `None` when a read fails: a period total is then never contradicted.
async fn live_source_coverage(sources: &AthleteSources<'_>) -> Option<Vec<SourceCoverage>> {
    let (user_id, tenant_id) = (sources.user_id, sources.tenant_id);
    let backends = resolve_compute_backends(sources.repos, tenant_id, user_id)
        .await
        .inspect_err(|e| {
            warn!(
                user_id = %user_id,
                error = %e,
                "athlete-record: connection read failed — period totals stay unverifiable"
            );
        })
        .ok()?;
    let cache = &sources.repos.activity_cache;
    let mut coverage = Vec::with_capacity(backends.len());
    for backend in backends
        .iter()
        .filter(|backend| backend.capturable && !backend.requires_reauth)
    {
        let (last_sync, backfill) = tokio::join!(
            cache.latest_activity_sync(user_id, &tenant_id, &backend.slug),
            cache.get_backfill_coverage(user_id, &tenant_id, &backend.slug),
        );
        match (last_sync, backfill) {
            (Ok(last_sync), Ok(backfill)) => coverage.push(SourceCoverage {
                last_sync,
                depth: backfill_depth(backfill, &backend.slug),
            }),
            (Err(e), _) | (_, Err(e)) => {
                warn!(
                    user_id = %user_id,
                    provider = %backend.slug,
                    error = %e,
                    "athlete-record: sync coverage read failed — period totals stay unverifiable"
                );
                return None;
            }
        }
    }
    Some(coverage)
}

/// How deep a source's history is held, read with the historical gate's own
/// rule ([`historical_depth_covered`]) so the period check and
/// `get_activities` agree on which windows the cache serves as complete: a
/// record of the source's current capture that exhausted the feed holds every
/// window, one that reached a floor holds the windows opening at or after it,
/// and anything else holds none.
fn backfill_depth(coverage: Option<BackfillCoverage>, slug: &str) -> BackfillDepth {
    let version = current_capture_version(slug);
    let Some(record) = coverage else {
        return BackfillDepth::Unrecorded;
    };
    if historical_depth_covered(coverage, i64::MIN, version) {
        return BackfillDepth::FeedEnd;
    }
    if !historical_depth_covered(coverage, record.oldest_reached_ts, version) {
        return BackfillDepth::Unrecorded;
    }
    Utc.timestamp_opt(record.oldest_reached_ts, 0)
        .single()
        .map_or(BackfillDepth::Unrecorded, BackfillDepth::Reached)
}

/// The first day of the training block the athlete is in: the phase of their
/// active plan that covers `today`, when the plan may be read here.
async fn current_block_start(sources: &AthleteSources<'_>, today: NaiveDate) -> Option<NaiveDate> {
    let user_id = sources.user_id;
    let plan = sources
        .repos
        .training_plans
        .get_active_plan(&sources.tenant_id.to_string(), &user_id.to_string())
        .await
        .inspect_err(|e| {
            warn!(
                user_id = %user_id,
                error = %e,
                "athlete-record: active plan read failed — a block total stays unverifiable"
            );
        })
        .ok()
        .flatten()
        .filter(|plan| ai_scope::admit_derived(plan.transport_policy))?;
    plan.phases
        .iter()
        .find(|phase| phase.covers(today))
        .and_then(PlanPhase::start_date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    /// Wednesday 2026-09-09 at 19:00 UTC.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 9, 19, 0, 0).unwrap()
    }

    fn read() -> CacheRead {
        CacheRead {
            from: now() - ATHLETE_RECORD_WINDOW,
            cut_day: None,
        }
    }

    /// One source read this turn whose feed is exhausted: every day of the
    /// read is vouched for.
    fn synced() -> Vec<SourceCoverage> {
        vec![SourceCoverage {
            last_sync: Some(now() - TimeDelta::minutes(1)),
            depth: BackfillDepth::FeedEnd,
        }]
    }

    /// A read that returned no rows; the inputs of one that failed are never
    /// built.
    fn no_sessions(read: CacheRead, sources: Vec<SourceCoverage>) -> HistoryInputs<Utc> {
        HistoryInputs {
            sessions: Vec::new(),
            by_source: Vec::new(),
            read,
            sources,
            now: now(),
            zone: Utc,
            block_start: None,
        }
    }

    /// A read that succeeded and held nothing vouches for its days: the
    /// athlete rested, and a total stated over them is wrong. The same
    /// coverage beside a read that failed vouches for nothing, so a pool error
    /// never turns a correct total into a contradiction.
    #[test]
    fn a_failed_cache_read_vouches_for_no_period() {
        assert!(
            period_history(Some(read()), Some(synced()), no_sessions).is_some(),
            "an empty read that succeeded vouches for its days"
        );
        assert!(
            period_history(None, Some(synced()), no_sessions).is_none(),
            "a failed read is not an empty one"
        );
    }

    #[test]
    fn a_failed_coverage_read_vouches_for_no_period() {
        assert!(period_history(Some(read()), None, no_sessions).is_none());
    }
}
