// ABOUTME: The one aggregator: every count, distance or time over a period, from one read of the store, merged across sources
// ABOUTME: Filters, buckets on the athlete's calendar, and says how far each source's history reaches; never reads a provider

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The activity aggregator (carnet#1485)
//!
//! Counts came from ten per-provider paths and a fixed-shape tool, and two of
//! them contradicted each other with nothing to notice: a window total per
//! sport and a lifetime aggregate, each from its own read. Every count,
//! distance or time over a period now comes from one read of the store
//! ([`visit_stored_workouts`]) and one summing implementation
//! ([`Accumulator`]). The read merges the copies of one workout across
//! sources (a count counts workouts, never rows), the accumulator filters
//! and buckets on the athlete's own calendar, and the answer says how far
//! the history reaches, for each source and over all of them
//! ([`QueryAnswer::coverage`], [`QueryAnswer::coverage_by_source`]).
//!
//! It never reads a provider. A tool's read ([`ReadGate::Model`]) asks a
//! history walk of a source it counts that was never walked, as the
//! training history does; Home's read ([`ReadGate::Display`]) asks nothing.
//!
//! The core is ungated: Home's volume card builds without `tools-data`. The
//! segment half of a `has_segment` count is the tool's: the answer carries
//! the segment population it counted ([`QueryAnswer::segments`]), which the
//! tool settles.

use std::iter;
use std::mem;
use std::sync::Arc;

use chrono::NaiveDate;
use pierre_core::civil_time::resolve_zone;
use pierre_core::constants::oauth::providers::{companion_keys, is_user_provided, STRAVA};
use pierre_core::constants::oauth_providers::{garmin_contribution, GarminContribution};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coverage::{Coverage, CoverageWindow, SourceReach, WindowScope};
use pierre_core::models::TenantId;
use pierre_core::untrusted::{display_line, fence_athlete_text};
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::{resolve_backend, user_facing_name};
use pierre_services::training_history_read::{
    history_store_keys, resolve_compute_backends, user_timezone, ComputeBackend,
};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use uuid::Uuid;

mod accumulator;
mod coverage;
mod parse;
pub(crate) mod predicate;
mod read;
mod served;

pub use accumulator::{
    aggregate, Accumulator, Folded, GroupKey, KeyValue, MetricValues, QueryGroup, QueryMetric,
    UnknownCounts, DEFAULT_METRICS, MAX_GROUPS, UNKNOWN_KEY,
};
#[cfg(feature = "tools-data")]
pub(crate) use parse::askable_tag_names;
pub use parse::{parse_query, AppliedQuery};
pub use predicate::{sport_matches, ActivityPredicate, SegmentMatch};
pub use read::{visit_stored_workouts, CivilWindow, KeyReach, KeySet, ReadReach};
pub use served::{count_served_list, ServedCount, ServedList};

use crate::activity_detail_store::served_place_name;
use crate::runtime::ToolRuntime;

/// Longest unknown provider name a refusal echoes.
const MAX_PROVIDER_ECHO_CHARS: usize = 40;

/// Longest gear label served.
const MAX_GEAR_LABEL_CHARS: usize = 60;

/// The athlete a read is about: the caller, or the coached athlete the
/// dispatch chokepoint resolved.
#[derive(Debug, Clone, Copy)]
pub struct Athlete {
    /// The athlete.
    pub user_id: Uuid,
    /// The tenant their rows are held in.
    pub tenant_id: TenantId,
}

/// How a read gates the stored copies before it merges them, and which
/// coverage rule it answers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadGate {
    /// What a model may read: the item filter every tool applies, the
    /// withheld tallied for the result's note, and each source judged on the
    /// history ladder, a walk asked where none ever was.
    Model,
    /// What the athlete's own surfaces serve, and Home's vouching rule:
    /// nothing asked.
    Display,
}

/// A count asked.
#[derive(Debug, Clone)]
pub struct ActivityQuery {
    /// The first civil day, included; `None` reads the whole stored history.
    pub from: Option<NaiveDate>,
    /// The last civil day, included.
    pub to: NaiveDate,
    /// What the count filters on.
    pub filter: ActivityPredicate,
    /// What it is broken down by: none, one key, or two for a cross-tab.
    pub group_by: Vec<GroupKey>,
    /// What it sums.
    pub metrics: Vec<QueryMetric>,
    /// One source alone: only the copies under its keys ([`source_keys`]).
    pub provider: Option<String>,
}

impl ActivityQuery {
    /// A count of every workout over no window, summing the window totals'
    /// four figures: the count behind a served list ([`count_served_list`]),
    /// which bounds the window itself.
    fn totals_only() -> Self {
        Self {
            from: None,
            to: NaiveDate::MAX,
            filter: ActivityPredicate::default(),
            group_by: Vec::new(),
            metrics: DEFAULT_METRICS.to_vec(),
            provider: None,
        }
    }
}

/// The segment population of a `has_segment` count: the workouts every
/// other filter matched that a detail walk could read (a copy under
/// [`segment_keys`]), which the tool's segment coverage is settled over.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SegmentCounts {
    /// Workouts in the population.
    pub total: usize,
    /// Those whose segments were read.
    pub read: usize,
    /// Those unread that only the athlete's own export holds, which no walk
    /// reads.
    pub export_only: usize,
    /// Whether the gate withheld any of their rows from this reader, or the
    /// read is served over an external transport.
    pub withheld: bool,
}

/// A count's answer.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct QueryAnswer {
    /// The filters, breakdown and metrics the count ran.
    pub applied: AppliedQuery,
    /// The sums over every counted workout.
    pub totals: MetricValues,
    /// The groups, at most [`MAX_GROUPS`], the largest by count.
    pub groups: Vec<QueryGroup>,
    /// Every group there was.
    pub groups_total: usize,
    /// Whether groups were cut.
    pub groups_truncated: bool,
    /// Workouts a filter could not judge, per field: never a miss.
    pub unknown: UnknownCounts,
    /// Whether the count is the whole truth for the window, over every
    /// source.
    pub coverage: Coverage,
    /// Each source's part of it.
    pub coverage_by_source: Vec<SourceReach>,
    /// What Garmin-sourced data contributed to the counted workouts.
    #[schemars(schema_with = "garmin_contribution_schema")]
    pub garmin_contribution: Option<GarminContribution>,
    /// The segment population, when a segment was asked for; the tool
    /// settles its coverage.
    #[serde(skip)]
    #[schemars(skip)]
    pub segments: Option<SegmentCounts>,
}

/// [`GarminContribution`]'s wire shape: `sole`, `part`, or `null`.
fn garmin_contribution_schema(_: &mut SchemaGenerator) -> Schema {
    schemars::json_schema!({
        "description": "Garmin's share of the counted workouts: sole, part, or null when none carries Garmin data.",
        "type": ["string", "null"],
        "enum": ["sole", "part", null],
    })
}

/// The keys one source's activities are stored under: every key its
/// history is held under ([`history_store_keys`]) and the athlete's own
/// imports of it ([`companion_keys`]), named by the provider the athlete
/// sees, so a session's backend reads its provider's imports. A list and a
/// count pinned to one source read this one set (carnet#1485, OQ6).
#[must_use]
pub(crate) fn source_keys(backend: &str) -> Vec<&str> {
    let mut keys = history_store_keys(backend);
    keys.extend(companion_keys(user_facing_name(backend)));
    keys
}

/// The keys the athlete's Strava activities are stored under
/// ([`source_keys`]). Only these are read for segment efforts.
#[must_use]
pub(crate) fn segment_keys() -> Vec<&'static str> {
    source_keys(STRAVA)
}

/// Count, sum and break down the athlete's workouts as `query` asks.
///
/// One read of the store ([`visit_stored_workouts`]) under every key the
/// athlete holds, or `query.provider`'s alone ([`source_keys`]), gated by `gate`,
/// merged, and folded by one [`Accumulator`]. Place groups are labelled
/// with the name this reader may see ([`served_place_name`]), gear groups
/// with the provider's gear name, fenced for a model.
///
/// # Errors
///
/// Refuses a `provider` that names no source Dravr knows; returns the
/// store's error when a read fails.
pub async fn query_activities(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    query: &ActivityQuery,
    gate: ReadGate,
) -> AppResult<QueryAnswer> {
    let athlete = Athlete { user_id, tenant_id };
    let repos = runtime.repos();
    let zone = resolve_zone(user_timezone(repos, user_id).await?.as_deref());
    let source = match &query.provider {
        Some(provider) => Some(known_source(runtime, athlete, provider).await?),
        None => None,
    };
    let pinned_keys: Vec<&str> = source.as_deref().map_or_default(source_keys);
    let keys = if source.is_some() {
        KeySet::Only(&pinned_keys)
    } else {
        KeySet::All
    };
    let window = CivilWindow {
        from: query.from,
        to: query.to,
        zone,
    };

    let mut accumulator = Accumulator::new(query, zone);
    let reach = visit_stored_workouts(runtime, athlete, window, keys, gate, &mut |workouts| {
        for workout in workouts {
            accumulator.add(workout);
        }
    })
    .await?;
    if reach.segment_rows_withheld || ai_scope::serving_external() {
        accumulator.segments_withheld();
    }
    let mut folded = accumulator.finish();

    let count = folded.window_totals.count;
    let span = CoverageWindow {
        scope: WindowScope::Asked,
        asked_from: query.from,
        asked_to: Some(query.to),
        oldest: folded.span.map(|(oldest, _)| oldest),
        newest: folded.span.map(|(_, newest)| newest),
        total: Some(count),
        returned: count,
        totals: Some(folded.window_totals.clone()),
        ..CoverageWindow::default()
    };
    let (coverage, coverage_by_source) = match gate {
        ReadGate::Display => (
            coverage::display_coverage(runtime, athlete, window, &reach, span).await?,
            Vec::new(),
        ),
        ReadGate::Model => {
            let sources = sources_of(runtime, athlete, source.as_deref()).await?;
            coverage::model_coverage(runtime, athlete, &sources, window, &reach, span).await?
        }
    };

    let (garmin, other) = folded.garmin;
    let groups = labelled(mem::take(&mut folded.groups), &folded, gate);
    Ok(QueryAnswer {
        applied: AppliedQuery::of(query),
        totals: folded.totals,
        groups_truncated: folded.groups_total > groups.len(),
        groups_total: folded.groups_total,
        groups,
        unknown: folded.unknown,
        coverage,
        coverage_by_source,
        garmin_contribution: garmin_contribution(
            iter::repeat_n(true, garmin).chain(iter::repeat_n(false, other)),
        ),
        segments: folded.segments,
    })
}

/// The backend that serves `provider` for the athlete
/// ([`resolve_backend`]), or a refusal naming it when it is no source Dravr
/// knows: an unknown name would read no rows and answer a false zero.
async fn known_source(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    provider: &str,
) -> AppResult<String> {
    let slug = resolve_backend(
        runtime.repos(),
        athlete.user_id,
        Some(athlete.tenant_id),
        provider,
    )
    .await;
    let registry = runtime.provider_registry();
    if registry.is_supported(provider) || registry.is_supported(&slug) || is_user_provided(&slug) {
        Ok(slug)
    } else {
        Err(AppError::invalid_input(format!(
            "provider: unknown source '{}'",
            display_line(provider, MAX_PROVIDER_ECHO_CHARS)
        )))
    }
}

/// The sources a model's count is judged over: every source the athlete's
/// history stands on, or the one asked, which is judged as no walk deepens
/// it when the athlete holds no connection to it.
async fn sources_of(
    runtime: &Arc<dyn ToolRuntime>,
    athlete: Athlete,
    asked: Option<&str>,
) -> AppResult<Vec<ComputeBackend>> {
    let backends =
        resolve_compute_backends(runtime.repos(), athlete.tenant_id, athlete.user_id).await?;
    let Some(asked) = asked else {
        return Ok(backends);
    };
    let held = backends.into_iter().find(|backend| backend.slug == asked);
    Ok(vec![held.unwrap_or_else(|| ComputeBackend {
        slug: asked.to_owned(),
        requires_reauth: false,
        capturable: false,
    })])
}

/// `groups` with each place and gear key labelled as this reader may see
/// it: a place's name through [`served_place_name`] (an automatic name is
/// never served externally), a gear name as one display line, fenced as
/// athlete text for a model.
fn labelled(mut groups: Vec<QueryGroup>, folded: &Folded, gate: ReadGate) -> Vec<QueryGroup> {
    for group in &mut groups {
        let mut labels = Vec::new();
        for (key, value) in &group.key {
            let KeyValue::Text(id) = value else {
                continue;
            };
            let label = match key {
                GroupKey::Place => folded.places.get(id).and_then(served_place_name),
                GroupKey::Gear => folded.gear_names.get(id).and_then(|name| match gate {
                    ReadGate::Model => fence_athlete_text(name, MAX_GEAR_LABEL_CHARS),
                    ReadGate::Display => Some(display_line(name, MAX_GEAR_LABEL_CHARS)),
                }),
                _ => None,
            };
            if let Some(label) = label {
                labels.push((*key, label));
            }
        }
        group.label.extend(labels);
    }
    groups
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate, TimeZone, Utc};
    use chrono_tz::Tz;
    use pierre_core::civil_time::resolve_zone;
    use pierre_core::models::{
        Activity, ActivityBuilder, GearRef, PlaceRef, SegmentEffort, SportFamily, SportType,
    };
    use pierre_database::repositories::CachedActivityRow;
    use pierre_services::workouts::workouts;
    use serde_json::json;

    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap_or_default()
    }

    fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or(NaiveDate::MIN)
    }

    fn montreal() -> Tz {
        resolve_zone(Some("America/Montreal"))
    }

    fn session(id: &str, sport: SportType, start: DateTime<Utc>) -> ActivityBuilder {
        ActivityBuilder::new(id, format!("Session {id}"), sport, start, 3_600, "strava")
            .distance_meters(10_000.0)
    }

    fn row(activity: Activity) -> CachedActivityRow {
        CachedActivityRow {
            provider: activity.provider().to_owned(),
            activity,
            route: None,
            detail_read: false,
            segment_read: false,
            stored_at: None,
            place: None,
        }
    }

    fn query(group_by: Vec<GroupKey>) -> ActivityQuery {
        ActivityQuery {
            from: None,
            to: ymd(2026, 12, 31),
            filter: ActivityPredicate::default(),
            group_by,
            metrics: DEFAULT_METRICS.to_vec(),
            provider: None,
        }
    }

    fn fold(rows: &[CachedActivityRow], query: &ActivityQuery) -> Folded {
        let merged = workouts(rows);
        let mut accumulator = Accumulator::new(query, montreal());
        for workout in &merged {
            accumulator.add(workout);
        }
        accumulator.finish()
    }

    fn group_count(folded: &Folded, key: &[(GroupKey, KeyValue)]) -> Option<u64> {
        folded
            .groups
            .iter()
            .find(|group| {
                group
                    .key
                    .iter()
                    .map(|(k, v)| (*k, v.clone()))
                    .eq(key.iter().cloned())
            })
            .and_then(|group| group.values.count)
    }

    #[test]
    fn a_late_ride_on_new_years_eve_counts_in_its_own_year() {
        // 23:30 on Dec 31 in Montreal is 04:30 UTC on Jan 1.
        let rows = vec![
            row(session("eve", SportType::Ride, at(2026, 1, 1, 4, 30)).build()),
            row(session("day", SportType::Ride, at(2026, 1, 1, 18, 0)).build()),
        ];
        let folded = fold(&rows, &query(vec![GroupKey::Year]));
        assert_eq!(
            group_count(&folded, &[(GroupKey::Year, KeyValue::Year(2025))]),
            Some(1),
            "bug: a ride at 23:30 local on Dec 31 is not counted in its own year"
        );
        assert_eq!(
            group_count(&folded, &[(GroupKey::Year, KeyValue::Year(2026))]),
            Some(1),
            "bug: the new year's ride is not counted in the new year"
        );
    }

    #[test]
    fn months_and_weeks_are_civil_and_weeks_start_on_monday() {
        // Sunday 2026-11-15, 22:00 in Montreal: the week of Monday the 9th.
        let rows = vec![row(
            session("sun", SportType::Run, at(2026, 11, 16, 3, 0)).build()
        )];
        let folded = fold(&rows, &query(vec![GroupKey::Month, GroupKey::Week]));
        assert_eq!(
            group_count(
                &folded,
                &[
                    (GroupKey::Month, KeyValue::Text("2026-11".to_owned())),
                    (GroupKey::Week, KeyValue::Text("2026-11-09".to_owned())),
                ]
            ),
            Some(1),
            "bug: a Sunday-night run is not in its civil month and Monday week"
        );
    }

    #[test]
    fn a_cross_tab_sums_to_the_totals() {
        let rows = vec![
            row(session("a", SportType::Ride, at(2024, 5, 1, 12, 0)).build()),
            row(session("b", SportType::Run, at(2024, 6, 1, 12, 0)).build()),
            row(session("c", SportType::Ride, at(2025, 5, 1, 12, 0)).build()),
        ];
        let folded = fold(&rows, &query(vec![GroupKey::Year, GroupKey::SportType]));
        let summed: u64 = folded.groups.iter().filter_map(|g| g.values.count).sum();
        assert_eq!(folded.groups.len(), 3, "bug: the cross-tab lost a cell");
        assert_eq!(
            Some(summed),
            folded.totals.count,
            "bug: a cross-tab's cells do not sum to the totals"
        );
        assert_eq!(
            group_count(
                &folded,
                &[
                    (GroupKey::Year, KeyValue::Year(2024)),
                    (GroupKey::SportType, KeyValue::Text("ride".to_owned())),
                ]
            ),
            Some(1),
            "bug: a cell is not keyed by year and the store's sport spelling"
        );
    }

    /// A ride on June `day` whose Strava copy carries efforts on La Bête only
    /// when `read`.
    fn ride_on_la_bete(id: &str, day: u32, read: bool) -> CachedActivityRow {
        let effort = SegmentEffort {
            id: format!("effort-{id}"),
            segment_id: Some("611413".to_owned()),
            name: "La Bête".to_owned(),
            elapsed_time: 412,
            moving_time: None,
            start_date: at(2026, 6, 1, 12, 0),
            distance: 1_820.5,
            average_heart_rate: None,
            max_heart_rate: None,
            average_cadence: None,
            average_watts: None,
            kom_rank: None,
            pr_rank: None,
            climb_category: None,
            average_grade: None,
            elevation_gain: None,
        };
        let built = session(id, SportType::Ride, at(2026, 6, day, 12, 0));
        let activity = if read {
            built.segment_efforts(vec![effort]).build()
        } else {
            built.build()
        };
        CachedActivityRow {
            segment_read: read,
            ..row(activity)
        }
    }

    #[test]
    fn a_ride_whose_segments_were_never_read_is_unknown_not_a_miss() {
        let rows = vec![
            ride_on_la_bete("read", 1, true),
            ride_on_la_bete("unread", 8, false),
        ];
        let mut asked = query(Vec::new());
        asked.filter.has_segment = Some(SegmentMatch::Name("la bete".to_owned()));
        let folded = fold(&rows, &asked);
        assert_eq!(
            folded.totals.count,
            Some(1),
            "bug: the folded name does not match La Bête, or an unread ride counted"
        );
        assert_eq!(
            folded.unknown.has_segment, 1,
            "bug: a ride whose segments were never read is counted as a miss"
        );
        assert_eq!(
            folded.segments,
            Some(SegmentCounts {
                total: 2,
                read: 1,
                export_only: 0,
                withheld: false,
            }),
            "bug: the segment population is not the two rides a walk can read"
        );
    }

    #[test]
    fn a_ride_with_no_strava_copy_is_outside_the_segment_population() {
        let garmin = session("g", SportType::Ride, at(2026, 6, 2, 12, 0)).build();
        let mut garmin = row(garmin);
        garmin.provider = "garmin".to_owned();
        let mut asked = query(Vec::new());
        asked.filter.has_segment = Some(SegmentMatch::Id("611413".to_owned()));
        let folded = fold(&[garmin], &asked);
        assert_eq!(
            folded.unknown.has_segment, 1,
            "bug: a Garmin-only ride is not unknown for a segment"
        );
        assert_eq!(
            folded.segments.map(|counts| counts.total),
            Some(0),
            "bug: a ride no walk can read widens the segment population"
        );
    }

    #[test]
    fn a_placeless_workout_groups_as_unknown() {
        let mut placed = row(session("p", SportType::Ride, at(2026, 6, 1, 12, 0)).build());
        placed.place = Some(PlaceRef {
            id: "gai-luron".to_owned(),
            name: Some("Gai-Luron".to_owned()),
            athlete_named: true,
        });
        let bare = row(session("b", SportType::Ride, at(2026, 6, 3, 12, 0)).build());
        let folded = fold(&[placed, bare], &query(vec![GroupKey::Place]));
        assert_eq!(
            group_count(
                &folded,
                &[(GroupKey::Place, KeyValue::Text(UNKNOWN_KEY.to_owned()))]
            ),
            Some(1),
            "bug: a workout with no place is not grouped as unknown"
        );
        assert_eq!(
            group_count(
                &folded,
                &[(GroupKey::Place, KeyValue::Text("gai-luron".to_owned()))]
            ),
            Some(1),
            "bug: a placed workout is not grouped by its place id"
        );
    }

    #[test]
    fn a_workout_with_no_trainer_flag_is_unknown_for_an_outdoor_filter() {
        let rows = vec![
            row(session("out", SportType::Ride, at(2026, 6, 1, 12, 0))
                .trainer(false)
                .build()),
            row(session("in", SportType::Ride, at(2026, 6, 2, 12, 0))
                .trainer(true)
                .build()),
            row(session("none", SportType::Ride, at(2026, 6, 3, 12, 0)).build()),
        ];
        let mut asked = query(Vec::new());
        asked.filter.trainer = Some(false);
        let folded = fold(&rows, &asked);
        assert_eq!(
            folded.totals.count,
            Some(1),
            "bug: the outdoor ride is not counted alone"
        );
        assert_eq!(
            folded.unknown.trainer, 1,
            "bug: a ride with no trainer flag is counted as outdoor, or dropped silently"
        );
    }

    #[test]
    fn heart_rate_is_weighted_by_moving_time_and_its_sessions_counted() {
        let mut asked = query(Vec::new());
        asked.metrics = vec![QueryMetric::AvgHr, QueryMetric::MovingTimeS];
        let rows = vec![
            row(session("long", SportType::Run, at(2026, 6, 1, 12, 0))
                .moving_time_seconds(3_000)
                .average_heart_rate(140)
                .build()),
            row(session("short", SportType::Run, at(2026, 6, 2, 12, 0))
                .moving_time_seconds(1_000)
                .average_heart_rate(180)
                .build()),
            row(session("bare", SportType::Run, at(2026, 6, 3, 12, 0)).build()),
        ];
        let folded = fold(&rows, &asked);
        assert_eq!(
            folded.totals.avg_hr,
            Some(150.0),
            "bug: the mean heart rate is not weighted by moving time"
        );
        assert_eq!(
            (
                folded.totals.avg_hr_sessions,
                folded.totals.moving_time_s,
                folded.totals.moving_time_sessions
            ),
            (Some(2), Some(4_000), Some(2)),
            "bug: the sessions behind moving time and heart rate are miscounted"
        );
    }

    #[test]
    fn a_distance_nothing_recorded_sums_to_positive_zero() {
        let bare = ActivityBuilder::new(
            "gym",
            "Gym",
            SportType::Workout,
            at(2026, 6, 1, 12, 0),
            1_800,
            "strava",
        )
        .build();
        let folded = fold(&[row(bare)], &query(Vec::new()));
        let km = folded.totals.distance_km.unwrap_or(f64::NAN);
        assert!(
            km == 0.0 && km.is_sign_positive(),
            "bug: no distance sums to {km}, not +0.0"
        );
    }

    #[test]
    fn groups_past_the_cap_are_cut_and_counted() {
        let rows: Vec<CachedActivityRow> = (0..(MAX_GROUPS + 5))
            .map(|n| {
                let day = ymd(2020, 1, 1) + chrono::Days::new(u64::try_from(n * 7).unwrap_or(0));
                let start = Utc.from_utc_datetime(&day.and_hms_opt(12, 0, 0).unwrap_or_default());
                row(session(&format!("w{n}"), SportType::Run, start).build())
            })
            .collect();
        let folded = fold(&rows, &query(vec![GroupKey::Week]));
        assert_eq!(
            folded.groups.len(),
            MAX_GROUPS,
            "bug: the groups are not cut at the cap"
        );
        assert_eq!(
            folded.groups_total,
            MAX_GROUPS + 5,
            "bug: the groups cut are not counted"
        );
    }

    #[test]
    fn window_totals_give_the_figures_the_page_sum_gave() {
        // Twelve rides with distance and climb, plus one that reports neither:
        // it counts and adds its hour, and adds no distance or climb. The
        // figures a served page's own sum gave, now folded from stored rows.
        let mut rows: Vec<CachedActivityRow> = (1..=12)
            .map(|month| {
                row(ActivityBuilder::new(
                    format!("ride-{month}"),
                    format!("ride-{month}"),
                    SportType::Ride,
                    at(2011, month, 15, 12, 0),
                    3_600,
                    "strava",
                )
                .distance_meters(10_000.0)
                .elevation_gain(125.0)
                .build())
            })
            .collect();
        rows.push(row(ActivityBuilder::new(
            "trainer",
            "trainer",
            SportType::Ride,
            at(2011, 12, 20, 12, 0),
            3_600,
            "strava",
        )
        .build()));
        assert_eq!(
            serde_json::to_value(fold(&rows, &ActivityQuery::totals_only()).window_totals).ok(),
            Some(json!({
                "count": 13,
                "distance_km": 120.0,
                "duration_seconds": 46_800,
                "elevation_gain_m": 1500.0,
            })),
            "bug: the window totals of thirteen rides are not the page sum's figures"
        );
        let two = [
            row(session("a", SportType::Run, at(2011, 3, 1, 12, 0)).build()),
            row(session("b", SportType::Run, at(2011, 9, 1, 12, 0)).build()),
        ];
        let two_totals = fold(&two, &ActivityQuery::totals_only()).window_totals;
        assert_eq!(
            (two_totals.count, two_totals.distance_km),
            (2, 20.0),
            "bug: two runs do not total two and 20 km"
        );
    }

    #[test]
    fn sums_stay_exact_and_only_the_wire_rounds_them() {
        let rows = vec![row(session("odd", SportType::Ride, at(2026, 6, 1, 12, 0))
            .distance_meters(12_345.0)
            .elevation_gain(123.45)
            .build())];
        let folded = fold(&rows, &query(Vec::new()));
        assert_eq!(
            (folded.totals.distance_km, folded.totals.elevation_m),
            (Some(12.345), Some(123.45)),
            "bug: a sum is rounded before a caller converts it"
        );
        assert_eq!(
            serde_json::to_value(folded.totals).ok(),
            Some(json!({
                "count": 1,
                "distance_km": 12.3,
                "elapsed_time_s": 3_600,
                "elevation_m": 123.5,
            })),
            "bug: the wire does not carry one decimal"
        );
    }

    #[test]
    fn a_name_and_gear_are_matched_folded() {
        let named = ActivityBuilder::new(
            "x",
            "Montée de LA BÊTE",
            SportType::Ride,
            at(2026, 6, 1, 12, 0),
            3_600,
            "strava",
        )
        .gear(GearRef {
            id: Some("b123".to_owned()),
            name: "Épic EVO".to_owned(),
        })
        .build();
        let mut asked = query(Vec::new());
        asked.filter.name_contains = Some("la bete".to_owned());
        asked.filter.gear = Some("epic".to_owned());
        let folded = fold(&[row(named)], &asked);
        assert_eq!(
            folded.totals.count,
            Some(1),
            "bug: a folded name or gear fragment does not match its accented original"
        );
    }

    #[test]
    fn a_family_matches_every_type_of_it() {
        let mtb = session("m", SportType::MountainBike, at(2026, 6, 1, 12, 0)).build();
        let virtual_ride = session("v", SportType::VirtualRide, at(2026, 6, 1, 12, 0)).build();
        let run = session("r", SportType::Run, at(2026, 6, 1, 12, 0)).build();
        for ride in [&mtb, &virtual_ride] {
            assert!(
                sport_matches(ride, &[], &[SportFamily::Cycling]),
                "bug: cycling does not match {:?}",
                ride.sport_type()
            );
        }
        assert!(
            !sport_matches(&run, &[], &[SportFamily::Cycling]),
            "bug: cycling matches a run"
        );
        assert!(
            sport_matches(&mtb, &[SportType::Ride], &[]),
            "bug: a ride type no longer heads a mountain bike ride, as the activity list filter does"
        );
    }

    #[test]
    fn arguments_resolve_aliases_and_refuse_by_name() {
        let today = ymd(2026, 11, 16);
        let parsed = parse_query(
            &json!({
                "from": "2020-01-01",
                "sport_type": ["trail", "vtt", "ski de fond"],
                "group_by": ["year", "sport_type"],
                "metrics": ["count"],
            }),
            today,
        );
        let parsed = parsed.unwrap_or_else(|e| panic!("bug: Q3's arguments are refused: {e}"));
        assert_eq!(
            parsed.filter.sport_types,
            vec![
                SportType::TrailRunning,
                SportType::MountainBike,
                SportType::CrossCountrySkiing
            ],
            "bug: sport aliases do not resolve"
        );
        assert_eq!(parsed.to, today, "bug: to does not default to today");
        for refused in [
            json!({ "sport_type": ["curling on mars"] }),
            json!({ "from": "2026-02-01", "to": "2026-01-01" }),
            json!({ "min_distance_km": 20, "max_distance_km": 10 }),
            json!({ "group_by": ["year", "month", "week"] }),
            json!({ "metrics": ["watts"] }),
            json!({ "workout_type": ["bogus"] }),
            json!({ "workout_type": ["unknown"] }),
        ] {
            assert!(
                parse_query(&refused, today).is_err(),
                "bug: {refused} was accepted"
            );
        }
        let tags = parse_query(
            &json!({ "workout_type": ["race", "long_run", "workout", "commute", "recovery"] }),
            today,
        )
        .unwrap_or_else(|e| panic!("bug: the tag vocabulary is refused: {e}"));
        assert_eq!(tags.filter.workout_type.len(), 5, "bug: a tag was dropped");
        let echoed = AppliedQuery::of(
            &parse_query(
                &json!({ "name_contains": "Sortie\n<b>du</b> dimanche" }),
                today,
            )
            .unwrap_or_else(|e| panic!("bug: a name to match is refused: {e}")),
        );
        assert!(
            echoed
                .name_contains
                .is_some_and(|name| !name.contains('\n') && !name.contains("<b>")),
            "bug: the name echoed is not one display line"
        );
    }

    /// Every free-text filter is held to the length `name_contains` is: the
    /// text is athlete-typed, folded per row and echoed back in `applied`.
    #[test]
    fn every_text_filter_is_capped_at_sixty_characters() {
        let today = ymd(2026, 11, 16);
        let sixty = "é".repeat(60);
        let sixty_one = "é".repeat(61);
        let digits = "1".repeat(61);
        for key in ["name_contains", "has_segment", "gear"] {
            assert!(
                parse_query(&json!({ key: sixty }), today).is_ok(),
                "bug: {key} of 60 characters is refused"
            );
            let refused = parse_query(&json!({ key: sixty_one }), today);
            let said = refused.map_or_else(|e| e.to_string(), |_| String::new());
            assert!(
                said.contains(key) && said.contains("60"),
                "bug: {key} of 61 characters is not refused by name: {said:?}"
            );
        }
        assert!(
            parse_query(&json!({ "has_segment": digits }), today).is_err(),
            "bug: a segment id of 61 digits is accepted"
        );
    }
}
