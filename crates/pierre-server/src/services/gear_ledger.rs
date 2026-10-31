// ABOUTME: The athlete's gear ledger over their cached workouts — default pairs given lazily, choices recorded once per workout
// ABOUTME: Copies of one workout merge as the Home list merges them, so a run synced by two providers adds to a pair once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The gear ledger as the athlete's screens read and change it (carnet#711).
//!
//! The pairs and their attributions are stored by
//! [`pierre_database::repositories::AthleteGearRepository`]; this module
//! decides which activity an attribution is recorded on and which pair a new
//! activity takes.
//!
//! **One workout, one attribution.** A run a watch synced to Strava and to
//! intervals.icu is two cached copies of one workout. The copies are merged by
//! the session merger the Home list and the chat turn use
//! ([`merge_duplicates`]), and a workout counts as attributed when any of its
//! copies is. A default is recorded on the merged session's canonical copy;
//! the athlete's own choice is recorded on the copy they chose it on and
//! clears every other copy's, so a pair never counts one workout twice.
//!
//! **A workout's distance follows its merged session.** An attribution
//! records the distance the workout added when it was attributed, so the pair
//! keeps it after the cache lets the activity go. While the workout is still
//! cached, reading the ledger moves that distance to the merged session's
//! when the two differ — which copy stands for a workout follows the one
//! precedence rule, and a change to that rule (Athlete Data Consolidation
//! S3), or its rollback, moves the figure — so the ledger rebuilds itself on
//! its next read.
//!
//! **Defaults are given when the ledger is read.** Reading the ledger, or one
//! activity's pair, first gives every unattributed workout since the oldest
//! pair in use its sport's default ([`default_pair`]), oldest workout first,
//! and records it, so a pair's distance stays what it was when read again. A
//! workout the athlete declared no pair for keeps none.
//!
//! **A deleted activity lets its pair go.** When the athlete deletes an
//! activity — an upload, or a run deleted on Strava — its copy's attribution
//! goes with it ([`release_activity_gear`]): handed to another copy of the
//! same workout when one is left, so the choice stays and the workout still
//! counts once, and cleared otherwise, so the pair no longer counts a workout
//! that is gone and can be deleted once nothing else is attributed to it.
//! The cache's retention window letting an activity go is not a deletion: the
//! pair keeps that distance until the athlete deletes the activity itself.
//!
//! **An imported history brings its gear.** A Strava export names the gear
//! each activity was done in; that name is the athlete's own choice, made on
//! Strava, so an import attributes each run, walk or hike to the pair of that
//! name — declared for it when the ledger holds none — unless its workout
//! already holds an attribution ([`attribute_imported_gear`]).

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    default_pair, shoe_sport_slug, Activity, ActivityGearKey, AthleteGear, AttributionSource,
    GearAttribution, GearDeclaration, GearKind, SportType, TenantId,
    DEFAULT_SHOE_WARN_DISTANCE_METERS, MAX_GEAR_PER_ATHLETE,
};
use pierre_database::repositories::CachedActivityRow;
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::backend_pair_for;
use pierre_providers::deduplication::{is_canonical_copy, merge_duplicates, DedupConfig};
use pierre_tool_runtime::activity_fetch::activity_cache_retention_days;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// Most cached rows one ledger read gives defaults from. The retention
/// window of one athlete's workouts, with a copy per connection, sits far
/// below it; past it, the oldest rows are given theirs on a later read, once
/// newer ones no longer fill the read.
const LEDGER_ROW_LIMIT: i64 = 2_000;

/// How far either side of an activity's start its workout's other copies are
/// looked for. The merger chains copies whose starts lie within an hour or so
/// of each other; a day either side holds every copy of any workout a person
/// records, and every attribution recorded on one of them.
const WORKOUT_WINDOW_HOURS: i64 = 24;

/// How far a workout's merged distance may sit from the distance its
/// attribution recorded before the attribution follows it, in metres: the
/// rounding of a stored `REAL`, never a real change.
const DISTANCE_MOVED_METERS: f64 = 0.5;

/// Most cached rows read around one activity: far more than the copies of
/// every workout one athlete records in two days.
const WORKOUT_ROW_LIMIT: i64 = 200;

/// The pair one activity is attributed to, as its view shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityGear {
    /// The pair, `None` when none is attributed or the athlete declared none.
    pub gear_id: Option<Uuid>,
    /// How it was attributed, `None` when nothing is.
    pub source: Option<AttributionSource>,
}

/// One workout among the athlete's cached copies.
struct LedgerWorkout {
    /// The merged session: its sport, start and distance are the workout's.
    session: Activity,
    /// The copy the merged session was built on, where a default is recorded.
    canonical: ActivityGearKey,
    /// Every copy of the workout, the canonical one included.
    copies: Vec<ActivityGearKey>,
}

/// The ledger key of a cached copy.
fn row_key(row: &CachedActivityRow) -> ActivityGearKey {
    ActivityGearKey {
        provider: row.provider.clone(),
        activity_id: row.activity.id().to_owned(),
    }
}

/// The workouts `rows` hold, each with the keys of its copies.
fn ledger_workouts(rows: &[CachedActivityRow]) -> Vec<LedgerWorkout> {
    // The merger consumes its input and rewrites the canonical copy, while
    // each row's stored key is still needed.
    let recordings: Vec<Activity> = rows.iter().map(|row| row.activity.clone()).collect();
    let (sessions, report) = merge_duplicates(recordings, &DedupConfig::from_env());
    sessions
        .into_iter()
        .filter_map(|session| {
            let canonical = rows
                .iter()
                .find(|row| is_canonical_copy(&row.activity, &session))?;
            let group = report.groups.iter().find(|group| {
                group.canonical_id == session.id()
                    && group.window_start <= session.start_date()
                    && session.start_date() <= group.window_end
            });
            let copies = group.map_or_else(
                || vec![row_key(canonical)],
                |group| {
                    rows.iter()
                        .filter(|row| {
                            group.fragment_ids.iter().any(|id| id == row.activity.id())
                                && group.window_start <= row.activity.start_date()
                                && row.activity.start_date() <= group.window_end
                        })
                        .map(row_key)
                        .collect()
                },
            );
            Some(LedgerWorkout {
                canonical: row_key(canonical),
                copies,
                session,
            })
        })
        .collect()
}

/// The distance a workout adds to its pair, in metres: none when its session
/// recorded none.
fn workout_distance(session: &Activity) -> f64 {
    session
        .distance_meters()
        .filter(|meters| meters.is_finite() && *meters > 0.0)
        .unwrap_or(0.0)
}

/// The cached rows this request may read. Over an external transport, a
/// provider whose terms keep its data inside Dravr's own surfaces is dropped
/// before any merge, so a withheld copy never fills a served one (carnet#724).
fn served_rows(resources: &ServerContext, rows: Vec<CachedActivityRow>) -> Vec<CachedActivityRow> {
    ai_scope::retain_served_here(resources.fitness.provider_registry.as_ref(), rows, |row| {
        (row.provider.as_str(), row.activity.source())
    })
}

/// The athlete's ledger, after every unattributed workout since the oldest
/// pair in use was given its sport's default.
///
/// # Errors
///
/// A database error when the ledger, the attributions or the activity cache
/// cannot be read, or the defaults cannot be recorded.
pub async fn read_ledger(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &TenantId,
    now: DateTime<Utc>,
) -> AppResult<Vec<AthleteGear>> {
    let repos = &resources.common.repos;
    let mut ledger = repos.athlete_gear.list_gear(user_id).await?;
    let Some(oldest) = ledger
        .iter()
        .filter(|pair| !pair.is_retired())
        .map(|pair| pair.added_at)
        .min()
    else {
        return Ok(ledger);
    };
    let since = oldest.max(now - Duration::days(activity_cache_retention_days()));
    if since >= now {
        return Ok(ledger);
    }
    let stored = repos
        .activity_cache
        .get_cached_activity_rows(user_id, tenant_id, since, now, LEDGER_ROW_LIMIT)
        .await?;
    let stored_count = stored.len();
    let rows = served_rows(resources, stored);
    if rows.is_empty() {
        return Ok(ledger);
    }
    let margin = Duration::hours(WORKOUT_WINDOW_HOURS);
    let attributions: HashMap<ActivityGearKey, f64> = repos
        .athlete_gear
        .list_gear_attributions(user_id, since - margin, now + margin)
        .await?
        .into_iter()
        .map(|attribution| (attribution.key, attribution.distance_meters))
        .collect();

    let mut workouts = ledger_workouts(&rows);
    workouts.sort_by_key(|workout| workout.session.start_date());
    let mut defaults = Vec::new();
    let mut moved = Vec::new();
    for workout in workouts {
        if let Some(key) = workout
            .copies
            .iter()
            .find(|key| attributions.contains_key(*key))
        {
            let merged = workout_distance(&workout.session);
            if (attributions[key] - merged).abs() > DISTANCE_MOVED_METERS {
                moved.push((key.clone(), merged));
            }
            continue;
        }
        let start = workout.session.start_date();
        let Some(gear_id) =
            default_pair(&ledger, workout.session.sport_type(), start).map(|pair| pair.id)
        else {
            continue;
        };
        // The pair that took this workout is its sport's last used from here
        // on, for the workouts after it in this same pass.
        if let Some(pair) = ledger.iter_mut().find(|pair| pair.id == gear_id) {
            pair.last_used_at = pair.last_used_at.max(Some(start));
        }
        defaults.push(GearAttribution {
            key: workout.canonical,
            gear_id: Some(gear_id),
            source: AttributionSource::Default,
            start_date: start,
            distance_meters: workout_distance(&workout.session),
        });
    }
    // A distance is refreshed only from every stored copy: a read that had to
    // leave a copy out over this transport merges without it, and must not
    // record that partial figure for the athlete's own ledger.
    let refreshed = if stored_count == rows.len() {
        repos
            .athlete_gear
            .refresh_attribution_distances(user_id, &moved)
            .await?
    } else {
        0
    };
    if defaults.is_empty() && refreshed == 0 {
        return Ok(ledger);
    }
    repos
        .athlete_gear
        .record_default_attributions(user_id, &defaults)
        .await?;
    repos.athlete_gear.list_gear(user_id).await
}

/// The workout holding the caller's own cached activity `activity_id` from
/// `provider` — the provider as the Home row names it — with the key of the
/// copy asked for.
///
/// # Errors
///
/// Not found when the caller holds no such activity in this tenant; refused
/// when its provider's terms keep it off this transport.
async fn owned_workout(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &TenantId,
    provider: &str,
    activity_id: &str,
) -> AppResult<(LedgerWorkout, ActivityGearKey)> {
    let cache = &resources.common.repos.activity_cache;
    let mut owned = None;
    for backend in backend_pair_for(provider) {
        if let Some(activity) = cache
            .get_cached_activity(user_id, tenant_id, &backend, activity_id)
            .await?
        {
            ai_scope::first_party_only_read(
                resources.fitness.provider_registry.as_ref(),
                &backend,
                activity.source(),
            )?;
            owned = Some((backend, activity));
            break;
        }
    }
    let not_found = || AppError::not_found(format!("activity {activity_id} from {provider}"));
    let (backend, activity) = owned.ok_or_else(not_found)?;
    let asked = ActivityGearKey {
        provider: backend,
        activity_id: activity_id.to_owned(),
    };
    let window = Duration::hours(WORKOUT_WINDOW_HOURS);
    let rows = served_rows(
        resources,
        cache
            .get_cached_activity_rows(
                user_id,
                tenant_id,
                activity.start_date() - window,
                activity.start_date() + window,
                WORKOUT_ROW_LIMIT,
            )
            .await?,
    );
    let workout = ledger_workouts(&rows)
        .into_iter()
        .find(|workout| workout.copies.contains(&asked))
        .ok_or_else(not_found)?;
    Ok((workout, asked))
}

/// The pair the caller's activity is attributed to, after the ledger gave
/// every unattributed workout its default.
///
/// # Errors
///
/// Not found when the caller holds no such activity; a database error when
/// the ledger or the cache cannot be read.
pub async fn activity_gear(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &TenantId,
    provider: &str,
    activity_id: &str,
) -> AppResult<ActivityGear> {
    let (workout, _) = owned_workout(resources, user_id, tenant_id, provider, activity_id).await?;
    read_ledger(resources, user_id, tenant_id, Utc::now()).await?;
    let start = workout.session.start_date();
    let margin = Duration::hours(WORKOUT_WINDOW_HOURS);
    let held = resources
        .common
        .repos
        .athlete_gear
        .list_gear_attributions(user_id, start - margin, start + margin)
        .await?
        .into_iter()
        .find(|attribution| workout.copies.contains(&attribution.key));
    Ok(held.map_or(
        ActivityGear {
            gear_id: None,
            source: None,
        },
        |attribution| ActivityGear {
            gear_id: attribution.gear_id,
            source: Some(attribution.source),
        },
    ))
}

/// Record the athlete's choice for their activity: one of their pairs in
/// use, or none. Every other copy of the workout gives up what it held.
///
/// # Errors
///
/// Not found when the caller holds no such activity or no such pair; invalid
/// input when the pair is retired; a database error when the choice cannot
/// be stored.
pub async fn declare_activity_gear(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &TenantId,
    provider: &str,
    activity_id: &str,
    gear_id: Option<Uuid>,
) -> AppResult<ActivityGear> {
    let (workout, asked) =
        owned_workout(resources, user_id, tenant_id, provider, activity_id).await?;
    let gear = &resources.common.repos.athlete_gear;
    if let Some(gear_id) = gear_id {
        let pair = gear
            .get_gear(user_id, gear_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("gear {gear_id}")))?;
        if pair.is_retired() {
            return Err(AppError::invalid_input(
                "A retired pair cannot be chosen for an activity; restore it first",
            ));
        }
    }
    gear.declare_gear_attribution(
        user_id,
        &GearAttribution {
            key: asked,
            gear_id,
            source: AttributionSource::Declared,
            start_date: workout.session.start_date(),
            distance_meters: workout_distance(&workout.session),
        },
        &workout.copies,
    )
    .await?;
    Ok(ActivityGear {
        gear_id,
        source: Some(AttributionSource::Declared),
    })
}

/// An imported activity and the gear its source named for it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImportedGear<'row> {
    /// The activity, as it is cached.
    pub activity: &'row Activity,
    /// The source's name for the gear (a Strava export's `Activity Gear`).
    pub gear_name: &'row str,
}

/// The pair each name an import used was matched to or declared as.
struct NamedPair {
    /// The name as the source spelled it, trimmed.
    name: String,
    /// The start of the earliest activity naming it.
    first_start: DateTime<Utc>,
    /// The sport of that activity: a pair declared for the name is worn for it.
    sport: SportType,
}

/// The ledger's key for a name: the same pair whatever its case.
fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The pair the ledger holds under `key`, or one declared for `named`, added
/// at its first start; `None` when the ledger refuses the name or the athlete
/// holds every pair they may.
async fn pair_for_name(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    ledger: &mut Vec<AthleteGear>,
    key: &str,
    named: &NamedPair,
) -> AppResult<Option<Uuid>> {
    // A retired pair is matched too: an imported workout is history, and a
    // retired pair keeps its history.
    if let Some(pair) = ledger
        .iter()
        .find(|pair| name_key(&pair.declaration.name) == key)
    {
        return Ok(Some(pair.id));
    }
    if ledger.len() >= usize::try_from(MAX_GEAR_PER_ATHLETE).unwrap_or(usize::MAX) {
        warn!(%user_id, "imported gear left unattributed: the athlete holds every pair they may");
        return Ok(None);
    }
    let declaration = GearDeclaration {
        kind: GearKind::Shoe,
        name: named.name.clone(),
        sport: named.sport.clone(),
        initial_distance_meters: 0.0,
        warn_distance_meters: DEFAULT_SHOE_WARN_DISTANCE_METERS,
        pinned: false,
    };
    let declaration = match declaration.validated() {
        Ok(declaration) => declaration,
        Err(e) => {
            warn!(%user_id, error = %e, "imported gear left unattributed: the ledger refuses its name");
            return Ok(None);
        }
    };
    let id = repos
        .athlete_gear
        .create_gear(user_id, &declaration, named.first_start)
        .await?;
    ledger.push(AthleteGear {
        id,
        declaration,
        added_at: named.first_start,
        retired_at: None,
        attributed_distance_meters: 0.0,
        activity_count: 0,
        last_used_at: None,
    });
    Ok(Some(id))
}

/// Attribute the activities an import cached under `provider` to the pairs
/// their source named, each workout once; how many were attributed.
///
/// Only shoe sports are attributed: the ledger holds shoes, so a ride's bike
/// names no pair and is left. A name is matched to the athlete's pair of that
/// name whatever its case, else declared as a pair worn for the sport of the
/// earliest activity naming it, in use from that activity's start. A name the
/// ledger refuses, or an athlete holding every pair they may, leaves its
/// activities unattributed rather than failing the import.
///
/// A workout any of whose copies is already attributed — the athlete's
/// choice, a default, an earlier import — keeps what it holds, so one workout
/// counts once and an athlete's choice is never replaced. Attributions are
/// recorded as declared on the imported copy and only where that copy holds
/// none, so projecting the same chunk again changes nothing.
///
/// # Errors
///
/// A database error when the ledger, the attributions or the activity cache
/// cannot be read, or a pair or an attribution cannot be stored.
pub(crate) async fn attribute_imported_gear(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: &TenantId,
    provider: &str,
    imported: &[ImportedGear<'_>],
) -> AppResult<u64> {
    let window = Duration::hours(WORKOUT_WINDOW_HOURS);
    let mut claimed: HashSet<ActivityGearKey> = HashSet::new();
    let mut unattributed = Vec::new();
    for item in imported {
        if shoe_sport_slug(item.activity.sport_type()).is_none() || item.gear_name.trim().is_empty()
        {
            continue;
        }
        let start = item.activity.start_date();
        let copy = ActivityGearKey {
            provider: provider.to_owned(),
            activity_id: item.activity.id().to_owned(),
        };
        let rows = repos
            .activity_cache
            .get_cached_activity_rows(
                user_id,
                tenant_id,
                start - window,
                start + window,
                WORKOUT_ROW_LIMIT,
            )
            .await?;
        let copies = ledger_workouts(&rows)
            .into_iter()
            .find(|workout| workout.copies.contains(&copy))
            .map_or_else(|| vec![copy.clone()], |workout| workout.copies);
        if copies.iter().any(|key| claimed.contains(key)) {
            continue;
        }
        let held = repos
            .athlete_gear
            .list_gear_attributions(user_id, start - window, start + window)
            .await?
            .iter()
            .any(|attribution| copies.contains(&attribution.key));
        claimed.extend(copies);
        if !held {
            unattributed.push((item, copy));
        }
    }
    if unattributed.is_empty() {
        return Ok(0);
    }

    // Only a name some unattributed workout uses is matched or declared, so
    // an import whose every workout already holds a pair adds none.
    let mut names: BTreeMap<String, NamedPair> = BTreeMap::new();
    for (item, _) in &unattributed {
        let start = item.activity.start_date();
        let pair = names
            .entry(name_key(item.gear_name))
            .or_insert_with(|| NamedPair {
                name: item.gear_name.trim().to_owned(),
                first_start: start,
                sport: item.activity.sport_type().clone(),
            });
        if start < pair.first_start {
            pair.first_start = start;
            pair.sport = item.activity.sport_type().clone();
        }
    }
    let mut ledger = repos.athlete_gear.list_gear(user_id).await?;
    let mut pairs = BTreeMap::new();
    for (key, named) in &names {
        let pair = pair_for_name(repos, user_id, &mut ledger, key, named).await?;
        pairs.insert(key.as_str(), pair);
    }

    let attributions: Vec<GearAttribution> = unattributed
        .into_iter()
        .filter_map(|(item, copy)| {
            let gear_id = (*pairs.get(name_key(item.gear_name).as_str())?)?;
            Some(GearAttribution {
                key: copy,
                gear_id: Some(gear_id),
                source: AttributionSource::Declared,
                start_date: item.activity.start_date(),
                distance_meters: workout_distance(item.activity),
            })
        })
        .collect();
    let recorded = repos
        .athlete_gear
        .record_default_attributions(user_id, &attributions)
        .await?;
    if recorded > 0 {
        info!(%user_id, provider, recorded, "imported activities attributed to their gear");
    }
    Ok(recorded)
}

/// The copy of `released`'s workout that takes its attribution over once
/// `released` is deleted: the workout's canonical copy, or another of its
/// copies when `released` is the canonical one. `None` when `released` is the
/// workout's only copy.
fn successor_copy(
    rows: &[CachedActivityRow],
    released: &ActivityGearKey,
) -> Option<ActivityGearKey> {
    let workout = ledger_workouts(rows)
        .into_iter()
        .find(|workout| workout.copies.contains(released))?;
    if workout.canonical != *released {
        return Some(workout.canonical);
    }
    workout.copies.into_iter().find(|key| key != released)
}

/// Let go of the pair the caller's activity `activity_id` from `provider` —
/// the provider key it is cached under — is attributed to, before that copy
/// is deleted. Another copy of the same workout takes the attribution over
/// when one is cached, so the athlete's choice stays; otherwise it is
/// cleared, and the pair stops counting the workout.
///
/// A copy with no cached row — a run the cache's retention window let go
/// before its provider reported it deleted — has no start to find its
/// workout's other copies by, so its own attribution is cleared.
///
/// Called before the copy's row is deleted, so a write that fails leaves the
/// activity and its attribution both in place for the athlete to retry.
///
/// # Errors
///
/// A database error when the attributions or the activity cache cannot be
/// read, or the attribution cannot be released.
pub(crate) async fn release_activity_gear(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: &TenantId,
    provider: &str,
    activity_id: &str,
) -> AppResult<()> {
    let released = ActivityGearKey {
        provider: provider.to_owned(),
        activity_id: activity_id.to_owned(),
    };
    let Some(activity) = repos
        .activity_cache
        .get_cached_activity(user_id, tenant_id, provider, activity_id)
        .await?
    else {
        return repos
            .athlete_gear
            .release_gear_attribution(user_id, &released, None)
            .await;
    };
    let start = activity.start_date();
    let window = Duration::hours(WORKOUT_WINDOW_HOURS);
    let held = repos
        .athlete_gear
        .list_gear_attributions(user_id, start - window, start + window)
        .await?
        .iter()
        .any(|attribution| attribution.key == released);
    if !held {
        return Ok(());
    }
    let rows = repos
        .activity_cache
        .get_cached_activity_rows(
            user_id,
            tenant_id,
            start - window,
            start + window,
            WORKOUT_ROW_LIMIT,
        )
        .await?;
    let successor = successor_copy(&rows, &released);
    repos
        .athlete_gear
        .release_gear_attribution(user_id, &released, successor.as_ref())
        .await
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use pierre_core::models::{ActivityBuilder, SportType};

    use super::*;

    fn cached(provider: &str, id: &str, minute: u32, meters: f64) -> CachedActivityRow {
        CachedActivityRow {
            provider: provider.to_owned(),
            activity: ActivityBuilder::new(
                id,
                "Morning run",
                SportType::Run,
                Utc.with_ymd_and_hms(2026, 9, 20, 7, minute, 0).unwrap(),
                3_000,
                provider,
            )
            .distance_meters(meters)
            .build(),
            route: None,
            detail_read: false,
        }
    }

    #[test]
    fn two_providers_copies_of_one_run_are_one_workout_with_both_keys() {
        let rows = vec![
            cached("strava", "s1", 0, 10_000.0),
            cached("intervals_icu", "i1", 1, 10_050.0),
            {
                let mut later = cached("strava", "s2", 0, 5_000.0);
                later.activity = ActivityBuilder::new(
                    "s2",
                    "Evening run",
                    SportType::Run,
                    Utc.with_ymd_and_hms(2026, 9, 20, 18, 0, 0).unwrap(),
                    1_500,
                    "strava",
                )
                .distance_meters(5_000.0)
                .build();
                later
            },
        ];
        let mut workouts = ledger_workouts(&rows);
        workouts.sort_by_key(|w| w.session.start_date());
        assert_eq!(workouts.len(), 2);
        let mut morning: Vec<String> = workouts[0]
            .copies
            .iter()
            .map(|key| format!("{}/{}", key.provider, key.activity_id))
            .collect();
        morning.sort();
        assert_eq!(morning, ["intervals_icu/i1", "strava/s1"]);
        assert!(workouts[0].copies.contains(&workouts[0].canonical));
        assert_eq!(
            workouts[1].copies,
            vec![ActivityGearKey {
                provider: "strava".to_owned(),
                activity_id: "s2".to_owned(),
            }]
        );
        assert!((workout_distance(&workouts[1].session) - 5_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_workout_without_a_distance_adds_none() {
        let row = CachedActivityRow {
            provider: "strava".to_owned(),
            activity: ActivityBuilder::new(
                "t1",
                "Treadmill",
                SportType::VirtualRun,
                Utc.with_ymd_and_hms(2026, 9, 20, 7, 0, 0).unwrap(),
                1_800,
                "strava",
            )
            .build(),
            route: None,
            detail_read: false,
        };
        assert!(workout_distance(&row.activity).abs() < f64::EPSILON);
    }

    #[test]
    fn a_deleted_copy_hands_over_to_its_workouts_other_copy_and_a_lone_one_to_none() {
        let key = |provider: &str, id: &str| ActivityGearKey {
            provider: provider.to_owned(),
            activity_id: id.to_owned(),
        };
        let mut evening = cached("strava", "s2", 0, 5_000.0);
        evening.activity = ActivityBuilder::new(
            "s2",
            "Evening run",
            SportType::Run,
            Utc.with_ymd_and_hms(2026, 9, 20, 18, 0, 0).unwrap(),
            1_500,
            "strava",
        )
        .distance_meters(5_000.0)
        .build();
        let rows = vec![
            cached("strava", "s1", 0, 10_000.0),
            cached("intervals_icu", "i1", 1, 10_050.0),
            evening,
        ];

        assert_eq!(
            successor_copy(&rows, &key("strava", "s1")),
            Some(key("intervals_icu", "i1"))
        );
        assert_eq!(
            successor_copy(&rows, &key("intervals_icu", "i1")),
            Some(key("strava", "s1"))
        );
        assert_eq!(successor_copy(&rows, &key("strava", "s2")), None);
        assert_eq!(successor_copy(&rows, &key("strava", "gone")), None);
    }
}
