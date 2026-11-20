// ABOUTME: Recomputes each athlete's places from their stored tracks, one workout one start, and names them
// ABOUTME: The places worker's pass, and the service name_place calls to name a place or declare where home is

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete places (carnet#1484, slice 6).
//!
//! A place is where an athlete keeps starting: a trailhead, a club's meeting
//! point. [`places_pass`] recomputes the places of the athletes whose cached
//! rows changed since their last clustering, [`name_place`] lets the
//! athlete name one, and [`declare_home`] lets them say where home is
//! (carnet#1593).
//!
//! **One start per workout.** An athlete's cached rows hold every copy of a
//! workout: a session's and a grant's, an export, a second device. The rows
//! are collapsed by [`one_activity_key`], then by the one row-level merge
//! ([`workouts::workouts`]), and each workout gives one start. It is the
//! trimmed first point of the first of its copies that has a track stored
//! locally (`local_track`), the canonical copy's first: the copy ranking does
//! not prefer a copy with a route, so a canonical Garmin row with no polyline
//! would otherwise hide its Strava twin's track. A workout with no track
//! anywhere gives its recorded raw start, which may only join a place it falls
//! inside ([`StartKind::RawAssignOnly`]). Every copy of a workout takes its place.
//!
//! **Privacy.** A centroid is built from trimmed starts only. Home is
//! excluded ([`cluster`]): the densest cluster of trimmed endpoints, which is
//! never stored, unless the athlete declared home. A declaration stores a
//! state and, for "here", a seed that is a trimmed start or a place
//! centroid, never a raw start; `recompute` is its one reader, and the
//! answer is a state word ([`HomeState`]). No function here returns a
//! centroid or the seed.
//!
//! **Names.** The athlete's own name wins, and only [`name_place`] writes
//! one. Otherwise a place is named for the most frequent city among its
//! workouts, else the most frequent segment they ride, else nothing; a tie
//! goes to the name of the most recent workout.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, CachedRowKey, HomeDeclaration, TenantId};
use pierre_core::untrusted::display_line;
use pierre_database::repositories::{
    AthletePlace, CachedActivityRow, CachedRowCursor, CachedRowsQuery, PlaceNameSource, PlaceRun,
};
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::places::{
    cluster, ClusteredPlace, PlaceAnchor, PlaceParams, StartKind, StartPoint, PLACES_VERSION,
};
use pierre_providers::deduplication::one_activity_key;
use schemars::JsonSchema;
use serde::Serialize;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::activity_track::{local_track, records_outdoor_route};
use crate::workouts;

/// The longest place name, in characters, after defanging.
pub const MAX_PLACE_NAME_CHARS: usize = 60;

/// The radius a place the athlete creates on one visit is given, in metres:
/// the clustering distance, so later starts nearby join it.
const SINGLE_VISIT_RADIUS_METERS: f64 = 300.0;

/// Cached rows read per page of a recompute's read.
const ROWS_PER_PAGE: i64 = 500;

/// Copies of one activity a `name_place` read by activity id expects at most
/// (session, grant, export, mirror); a fuller read takes the first eight.
const COPIES_PER_ACTIVITY: i64 = 8;

/// What one places pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlacesPassReport {
    /// Athletes whose places were recomputed.
    pub users: u32,
    /// Places those athletes now hold.
    pub places: u32,
    /// Athletes whose recompute failed; each is logged and sent to the back
    /// of the queue.
    pub failed: u32,
}

/// Which place `name_place` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceTarget {
    /// A place by its id, as a row's `place.id` serves it.
    Place(String),
    /// The place an activity started at, by the activity's id.
    Activity(String),
}

/// A place as `name_place` answers it: never a centroid or a count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedPlace {
    /// The place id.
    pub id: String,
    /// Its name, if it has one.
    pub name: Option<String>,
    /// Whether `name` is the athlete's own.
    pub athlete_named: bool,
}

/// Why `name_place` named nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceRefusal {
    /// The athlete holds no such place, or the activity has no place to
    /// clear.
    PlaceNotFound,
    /// The athlete holds no activity with that id.
    ActivityNotFound,
    /// The activity has no place and no recorded route to place it by.
    ActivityHasNoRoute,
    /// The name is empty or longer than [`MAX_PLACE_NAME_CHARS`] once
    /// defanged.
    InvalidName,
}

impl PlaceRefusal {
    /// The stable code a client and the refusal strings key on.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlaceNotFound => "place_not_found",
            Self::ActivityNotFound => "activity_not_found",
            Self::ActivityHasNoRoute => "activity_has_no_route",
            Self::InvalidName => "invalid_place_name",
        }
    }
}

/// What the athlete says about home, as [`declare_home`] takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeChoice {
    /// Home is that place, or where that activity started.
    Here(PlaceTarget),
    /// Home is not where the athlete trains: no start is excluded as home.
    Elsewhere,
    /// Back to the automatic guess.
    Forget,
}

/// Where home is, as a word: never a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HomeState {
    /// The athlete said where home is.
    Declared,
    /// The athlete said home is not where they train.
    Elsewhere,
    /// Home is the automatic guess.
    Inferred,
}

/// Recompute the places of up to `max_users` athletes whose places are due.
///
/// One athlete's failure is logged and counted, recorded as their latest
/// try, and the pass moves on. An athlete due for new rows stays due, behind
/// every athlete tried before them, so one whose recompute keeps failing is
/// retried only with the slots no one else is due; one due only for the
/// daily backstop waits another day.
///
/// # Errors
///
/// Returns the repository error when the due athletes cannot be listed.
pub async fn places_pass(
    repos: &RepositoryRegistry,
    now: DateTime<Utc>,
    max_users: u32,
) -> AppResult<PlacesPassReport> {
    let due = repos
        .athlete_places
        .due_users(max_users, PLACES_VERSION, now)
        .await?;
    let mut report = PlacesPassReport::default();
    for (tenant_id, user_id) in due {
        match recompute(repos, &tenant_id, user_id, now).await {
            Ok(places) => {
                report.users += 1;
                report.places += u32::try_from(places.len()).unwrap_or(u32::MAX);
            }
            Err(e) => {
                report.failed += 1;
                warn!(
                    %tenant_id,
                    %user_id,
                    error = %e,
                    "athlete places recompute failed; the athlete goes to the back of the queue"
                );
                if let Err(e) = repos
                    .athlete_places
                    .record_failed_recompute(&tenant_id, user_id, PLACES_VERSION, now)
                    .await
                {
                    warn!(
                        %tenant_id,
                        %user_id,
                        error = %e,
                        "recording a failed athlete places recompute failed; the athlete keeps its turn"
                    );
                }
            }
        }
    }
    Ok(report)
}

/// Name a place as the athlete asked (`Some`), or clear the athlete's name
/// (`None`) so the place goes back to its automatic name.
///
/// By activity: the activity's place is named; an activity with a recorded
/// route and no place (a first visit, or a start inside the home exclusion)
/// becomes the only member of a new place at its trimmed start, named as
/// asked. After a clear or a new place, the athlete's places are recomputed
/// at once, best-effort, so the automatic name comes back and the activity's
/// other copies join the new place without waiting for the worker.
///
/// # Errors
///
/// Returns the repository error when a read or write fails; a refusal is
/// `Ok(Err(_))`.
pub async fn name_place(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    target: PlaceTarget,
    name: Option<String>,
) -> AppResult<Result<NamedPlace, PlaceRefusal>> {
    let name = match name.as_deref().map(clean_name) {
        Some(None) => return Ok(Err(PlaceRefusal::InvalidName)),
        Some(Some(name)) => Some(name),
        None => None,
    };
    let place_id = match target {
        PlaceTarget::Place(id) => id,
        PlaceTarget::Activity(activity_id) => {
            let rows = activity_rows(repos, tenant_id, user_id, &activity_id).await?;
            if rows.is_empty() {
                return Ok(Err(PlaceRefusal::ActivityNotFound));
            }
            let placed = rows
                .iter()
                .find_map(|row| row.place.as_ref().map(|place| place.id.clone()));
            match (placed, &name) {
                (Some(id), _) => id,
                (None, None) => return Ok(Err(PlaceRefusal::PlaceNotFound)),
                (None, Some(name)) => {
                    return create_single_visit_place(repos, tenant_id, user_id, &rows, name).await;
                }
            }
        }
    };
    let cleared = name.is_none();
    let Some(place) = repos
        .athlete_places
        .set_name(tenant_id, user_id, &place_id, name.as_deref())
        .await?
    else {
        return Ok(Err(PlaceRefusal::PlaceNotFound));
    };
    if !cleared {
        return Ok(Ok(named(&place)));
    }
    let Some(places) = recompute_after_change(repos, tenant_id, user_id).await else {
        return Ok(Ok(named(&place)));
    };
    // A place that held only the athlete's single visit is gone once its
    // name is: it answers with its id and no name.
    Ok(Ok(places.iter().find(|p| p.id == place_id).map_or_else(
        || NamedPlace {
            id: place_id.clone(),
            name: None,
            athlete_named: false,
        },
        named,
    )))
}

/// Store what the athlete said about home and recompute their places.
///
/// The recompute runs at once, best-effort, so the next read reflects it;
/// the worker recomputes them otherwise, since a declaration no successful
/// recompute saw keeps them due.
///
/// `Here` by place takes that place's centroid as the seed; by activity, the
/// trimmed first point of the first copy with a recorded route. The seed is
/// never a raw start, never answered, and read by `recompute` alone. A
/// declaration renames nothing: a named place around the seed keeps its
/// name, and `Forget` brings back exactly the automatic result.
///
/// # Errors
///
/// Returns the repository error when a read or write fails; a refusal is
/// `Ok(Err(_))`, and nothing is stored.
pub async fn declare_home(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    choice: HomeChoice,
) -> AppResult<Result<HomeState, PlaceRefusal>> {
    let (home, state) = match choice {
        HomeChoice::Here(target) => {
            let seed = match home_seed(repos, tenant_id, user_id, target).await? {
                Ok(seed) => seed,
                Err(refused) => return Ok(Err(refused)),
            };
            (
                HomeDeclaration::At {
                    lat: seed.0,
                    lon: seed.1,
                },
                HomeState::Declared,
            )
        }
        HomeChoice::Elsewhere => (HomeDeclaration::Elsewhere, HomeState::Elsewhere),
        HomeChoice::Forget => (HomeDeclaration::Inferred, HomeState::Inferred),
    };
    repos
        .athlete_places
        .store_home_declaration(tenant_id, user_id, home, Utc::now())
        .await?;
    recompute_after_change(repos, tenant_id, user_id).await;
    Ok(Ok(state))
}

/// The seed a `Here` declaration stores: the place's centroid, or the
/// activity's trimmed start.
async fn home_seed(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    target: PlaceTarget,
) -> AppResult<Result<(f64, f64), PlaceRefusal>> {
    match target {
        PlaceTarget::Place(id) => Ok(repos
            .athlete_places
            .list(tenant_id, user_id)
            .await?
            .iter()
            .find(|place| place.id == id)
            .map(|place| (place.centroid_lat, place.centroid_lon))
            .ok_or(PlaceRefusal::PlaceNotFound)),
        PlaceTarget::Activity(activity_id) => {
            let rows = activity_rows(repos, tenant_id, user_id, &activity_id).await?;
            if rows.is_empty() {
                return Ok(Err(PlaceRefusal::ActivityNotFound));
            }
            Ok(trimmed_start(repos, tenant_id, user_id, &rows)
                .await?
                .map(|(point, _)| point)
                .ok_or(PlaceRefusal::ActivityHasNoRoute))
        }
    }
}

/// The place as `name_place` answers it.
fn named(place: &AthletePlace) -> NamedPlace {
    NamedPlace {
        id: place.id.clone(),
        name: place.name.clone(),
        athlete_named: place.name_source == Some(PlaceNameSource::Athlete),
    }
}

/// The athlete's name as it is stored: one defanged line of
/// 1..=[`MAX_PLACE_NAME_CHARS`] characters, or `None` when it is not one.
fn clean_name(raw: &str) -> Option<String> {
    let name = display_line(raw, usize::MAX);
    let name = name.trim();
    let chars = name.chars().count();
    (1..=MAX_PLACE_NAME_CHARS)
        .contains(&chars)
        .then(|| name.to_owned())
}

/// Every cached copy of one activity the athlete holds, newest first.
async fn activity_rows(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    activity_id: &str,
) -> AppResult<Vec<CachedActivityRow>> {
    let ids = [activity_id];
    let query = CachedRowsQuery::window(
        DateTime::UNIX_EPOCH,
        Utc::now() + Duration::days(1),
        COPIES_PER_ACTIVITY,
    )
    .with_ids(&ids);
    repos
        .activity_cache
        .get_cached_activity_rows(user_id, tenant_id, &query)
        .await
}

/// The trimmed first point of the first of `rows` with a recorded route,
/// and that row; `None` when no copy has one.
async fn trimmed_start<'rows>(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    rows: &'rows [CachedActivityRow],
) -> AppResult<Option<((f64, f64), &'rows CachedActivityRow)>> {
    for row in rows {
        if let Some(point) = local_track(repos, tenant_id, user_id, row)
            .await?
            .and_then(|track| track.first_point())
        {
            return Ok(Some((point, row)));
        }
    }
    Ok(None)
}

/// Create the athlete's place at the trimmed start of the first copy of the
/// activity with a recorded route, with that copy as its only member.
async fn create_single_visit_place(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    rows: &[CachedActivityRow],
    name: &str,
) -> AppResult<Result<NamedPlace, PlaceRefusal>> {
    let Some(((lat, lon), row)) = trimmed_start(repos, tenant_id, user_id, rows).await? else {
        return Ok(Err(PlaceRefusal::ActivityHasNoRoute));
    };
    let now = Utc::now();
    let place = AthletePlace {
        id: Uuid::new_v4().to_string(),
        centroid_lat: lat,
        centroid_lon: lon,
        radius_m: SINGLE_VISIT_RADIUS_METERS,
        activity_count: 1,
        name: Some(name.to_owned()),
        name_source: Some(PlaceNameSource::Athlete),
        version: PLACES_VERSION,
        computed_at: now,
        created_at: now,
    };
    repos
        .athlete_places
        .create_athlete_place(tenant_id, user_id, &place, &row_key(row))
        .await?;
    let answer = named(&place);
    recompute_after_change(repos, tenant_id, user_id).await;
    Ok(Ok(answer))
}

/// Recompute the athlete's places after a name or a home declaration changed
/// what anchors them. The change is already stored, so a failure is logged,
/// never answered: the athlete stays due, and the worker recomputes them.
async fn recompute_after_change(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
) -> Option<Vec<AthletePlace>> {
    match recompute(repos, tenant_id, user_id, Utc::now()).await {
        Ok(places) => Some(places),
        Err(e) => {
            warn!(
                %tenant_id,
                %user_id,
                error = %e,
                "athlete places recompute after a name or home change failed; the worker recomputes them"
            );
            None
        }
    }
}

/// One workout: the indices of its rows, its canonical copy's rows first.
pub(crate) struct Workout {
    pub(crate) rows: Vec<usize>,
}

/// Recompute one athlete's places from every cached row they hold and store
/// them, with each row's place and what this clustering saw, in one
/// transaction. Returns the places written.
async fn recompute(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> AppResult<Vec<AthletePlace>> {
    // Read before the rows: a row synced, or a track drawn, while this runs
    // makes the athlete due again rather than being taken as seen.
    let stamp = repos
        .athlete_places
        .cached_rows_stamp(tenant_id, user_id)
        .await?;
    // Read once: the run records the declaration this clustering used, so
    // one stored while this runs keeps the athlete due.
    let home = repos
        .athlete_places
        .home_declaration(tenant_id, user_id)
        .await?;
    let previous = repos.athlete_places.list(tenant_id, user_id).await?;
    let rows = all_rows(repos, tenant_id, user_id, now).await?;
    let workouts = workout_rows(&rows);

    let mut points = Vec::new();
    let mut endpoints = Vec::new();
    let mut point_workout: HashMap<CachedRowKey, usize> = HashMap::new();
    for (index, workout) in workouts.iter().enumerate() {
        let Some(point) =
            workout_start(repos, tenant_id, user_id, &rows, workout, &mut endpoints).await?
        else {
            continue;
        };
        point_workout.insert(point.key.clone(), index);
        points.push(point);
    }

    let anchors: Vec<PlaceAnchor> = previous.iter().map(anchor).collect();
    let clustering = cluster(
        &points,
        &endpoints,
        &anchors,
        home.home,
        &PlaceParams::default(),
    );
    debug!(
        %tenant_id,
        %user_id,
        rows = rows.len(),
        workouts = workouts.len(),
        starts = points.len(),
        home = ?home.home,
        places = clustering.places.len(),
        excluded = clustering.excluded.len(),
        "athlete places clustered"
    );

    let mut desired: Vec<Option<&str>> = vec![None; rows.len()];
    let mut places = Vec::with_capacity(clustering.places.len());
    for place in &clustering.places {
        let members: Vec<&Workout> = place
            .members
            .iter()
            .filter_map(|key| point_workout.get(key).map(|&index| &workouts[index]))
            .collect();
        for workout in &members {
            for &row in &workout.rows {
                desired[row] = Some(place.id.as_str());
            }
        }
        let previous = previous.iter().find(|p| p.id == place.id);
        places.push(stored_place(place, previous, &members, &rows, now));
    }

    // Only the rows whose place changes are written; a row whose place is
    // gone is cleared by the replace itself.
    let assignments: Vec<(CachedRowKey, Option<String>)> = rows
        .iter()
        .zip(&desired)
        .filter(|(row, want)| row.place.as_ref().map(|p| p.id.as_str()) != **want)
        .map(|(row, want)| (row_key(row), want.map(str::to_owned)))
        .collect();
    let run = PlaceRun {
        rows_seen: stamp.rows,
        newest_synced_at: stamp.newest_synced_at,
        version: PLACES_VERSION,
        computed_at: now,
        home_declared_at_seen: home.declared_at,
        newest_track_at_seen: stamp.newest_track_at,
    };
    repos
        .athlete_places
        .replace_clustering(tenant_id, user_id, &places, &assignments, &run)
        .await?;
    Ok(places)
}

/// Every cached row the athlete holds in the tenant, newest first.
pub(crate) async fn all_rows(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> AppResult<Vec<CachedActivityRow>> {
    let window =
        CachedRowsQuery::window(DateTime::UNIX_EPOCH, now + Duration::days(1), ROWS_PER_PAGE);
    let mut rows: Vec<CachedActivityRow> = Vec::new();
    loop {
        let query = rows.last().map_or_else(
            || window.clone(),
            |last| window.clone().below(CachedRowCursor::after(last)),
        );
        let page = repos
            .activity_cache
            .get_cached_activity_rows(user_id, tenant_id, &query)
            .await?;
        let full = i64::try_from(page.len()).unwrap_or(i64::MAX) >= ROWS_PER_PAGE;
        rows.extend(page);
        if !full {
            return Ok(rows);
        }
    }
}

/// The athlete's rows grouped into workouts: first every copy under one
/// [`one_activity_key`], then those groups merged by the one row-level merge
/// ([`workouts::workouts`]), the canonical copy's group first.
///
/// The merge names each workout's stored rows, so a group is matched back by
/// its first row's stored key, never by a bare activity id, which is unique
/// only within a provider. A group the merge leaves out is a workout of its
/// own: every row takes exactly one workout.
pub(crate) fn workout_rows(rows: &[CachedActivityRow]) -> Vec<Workout> {
    let mut copies: Vec<Vec<usize>> = Vec::new();
    let mut by_key: HashMap<(String, String), usize> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        let group = *by_key
            .entry(one_activity_key(&row.activity))
            .or_insert_with(|| {
                copies.push(Vec::new());
                copies.len() - 1
            });
        copies[group].push(index);
    }
    let representatives: Vec<CachedActivityRow> =
        copies.iter().map(|group| rows[group[0]].clone()).collect();
    let group_of: HashMap<CachedRowKey, usize> = representatives
        .iter()
        .enumerate()
        .map(|(group, row)| (row_key(row), group))
        .collect();

    let mut taken = vec![false; copies.len()];
    let mut out = Vec::new();
    for merged in workouts::workouts(&representatives) {
        let mut members = Vec::new();
        for row in merged.rows() {
            if let Some(&group) = group_of.get(&row_key(row)) {
                if !taken[group] {
                    taken[group] = true;
                    members.extend_from_slice(&copies[group]);
                }
            }
        }
        if !members.is_empty() {
            out.push(Workout { rows: members });
        }
    }
    for (group, rows) in copies.into_iter().enumerate() {
        if !taken[group] {
            out.push(Workout { rows });
        }
    }
    out
}

/// The workout's start: the trimmed first point of the first of its copies
/// with a local track, whose trimmed first and last points join `endpoints`;
/// else the first recorded raw start of an outdoor copy, which may only join
/// a place; else none.
async fn workout_start(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    rows: &[CachedActivityRow],
    workout: &Workout,
    endpoints: &mut Vec<(f64, f64)>,
) -> AppResult<Option<StartPoint>> {
    for &index in &workout.rows {
        let row = &rows[index];
        let Some(track) = local_track(repos, tenant_id, user_id, row).await? else {
            continue;
        };
        let (Some(first), Some(last)) = (track.first_point(), track.last_point()) else {
            continue;
        };
        endpoints.push(first);
        endpoints.push(last);
        return Ok(Some(StartPoint {
            key: row_key(row),
            lat: first.0,
            lon: first.1,
            kind: StartKind::Trimmed,
        }));
    }
    Ok(workout.rows.iter().find_map(|&index| {
        let row = &rows[index];
        let activity = &row.activity;
        let lat = activity.start_latitude().filter(|v| v.is_finite())?;
        let lon = activity.start_longitude().filter(|v| v.is_finite())?;
        records_outdoor_route(activity).then(|| StartPoint {
            key: row_key(row),
            lat,
            lon,
            kind: StartKind::RawAssignOnly,
        })
    }))
}

/// A stored place as the clustering sees it.
fn anchor(place: &AthletePlace) -> PlaceAnchor {
    PlaceAnchor {
        id: place.id.clone(),
        lat: place.centroid_lat,
        lon: place.centroid_lon,
        radius_m: place.radius_m,
        athlete_named: place.name_source == Some(PlaceNameSource::Athlete),
        activity_count: place.activity_count,
    }
}

/// The row a clustered place is stored as: the athlete's name kept, else its
/// automatic name; its creation kept across re-clustering.
fn stored_place(
    place: &ClusteredPlace,
    previous: Option<&AthletePlace>,
    members: &[&Workout],
    rows: &[CachedActivityRow],
    now: DateTime<Utc>,
) -> AthletePlace {
    let athlete_named = previous.filter(|p| p.name_source == Some(PlaceNameSource::Athlete));
    let (name, name_source) = athlete_named.map_or_else(
        || {
            automatic_name(members, rows)
                .map_or((None, None), |(name, source)| (Some(name), Some(source)))
        },
        |previous| (previous.name.clone(), Some(PlaceNameSource::Athlete)),
    );
    AthletePlace {
        id: place.id.clone(),
        centroid_lat: place.lat,
        centroid_lon: place.lon,
        radius_m: place.radius_m,
        activity_count: u32::try_from(place.members.len()).unwrap_or(u32::MAX),
        name,
        name_source,
        version: PLACES_VERSION,
        computed_at: now,
        created_at: previous.map_or(now, |p| p.created_at),
    }
}

/// The most frequent city among the workouts, else the segment most of them
/// ride; a tie goes to the name of the most recent workout.
fn automatic_name(
    workouts: &[&Workout],
    rows: &[CachedActivityRow],
) -> Option<(String, PlaceNameSource)> {
    let mut cities = Tally::default();
    let mut segments = Tally::default();
    for workout in workouts {
        let copies = || workout.rows.iter().map(|&index| &rows[index].activity);
        let Some(started) = copies().map(Activity::start_date).max() else {
            continue;
        };
        if let Some(city) = copies().find_map(|a| a.city().and_then(non_blank)) {
            cities.count(city, started);
        }
        let efforts = copies().find_map(|a| a.segment_efforts().filter(|e| !e.is_empty()));
        let names: HashSet<String> = efforts
            .into_iter()
            .flatten()
            .filter_map(|effort| non_blank(&effort.name))
            .collect();
        for name in names {
            segments.count(name, started);
        }
    }
    cities
        .best()
        .map(|name| (name, PlaceNameSource::City))
        .or_else(|| segments.best().map(|name| (name, PlaceNameSource::Segment)))
}

/// A name as an automatic name stores it, or `None` when nothing is left.
fn non_blank(raw: &str) -> Option<String> {
    let name = display_line(raw, MAX_PLACE_NAME_CHARS);
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// How many workouts carry each name, and when the latest of them started.
#[derive(Default)]
struct Tally(HashMap<String, (u32, DateTime<Utc>)>);

impl Tally {
    fn count(&mut self, name: String, started: DateTime<Utc>) {
        let entry = self.0.entry(name).or_insert((0, started));
        entry.0 += 1;
        entry.1 = entry.1.max(started);
    }

    /// The most counted name; of equals, the most recent, then the first in
    /// order.
    fn best(self) -> Option<String> {
        self.0
            .into_iter()
            .max_by(|a, b| {
                (a.1)
                    .0
                    .cmp(&(b.1).0)
                    .then((a.1).1.cmp(&(b.1).1))
                    .then_with(|| b.0.cmp(&a.0))
            })
            .map(|(name, _)| name)
    }
}

/// The key a cached row is stored under.
fn row_key(row: &CachedActivityRow) -> CachedRowKey {
    CachedRowKey {
        provider: row.provider.clone(),
        activity_id: row.activity.id().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use pierre_core::models::{ActivityBuilder, SportType};

    use super::*;

    fn row(provider: &str, id: &str, start: DateTime<Utc>) -> CachedActivityRow {
        CachedActivityRow {
            provider: provider.to_owned(),
            activity: ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3_600, provider)
                .distance_meters(30_000.0)
                .build(),
            route: None,
            detail_read: false,
            segment_read: false,
            stored_at: None,
            place: None,
        }
    }

    fn keys(rows: &[CachedActivityRow], workout: &Workout) -> Vec<(String, String)> {
        let mut keys: Vec<(String, String)> = workout
            .rows
            .iter()
            .map(|&i| (rows[i].provider.clone(), rows[i].activity.id().to_owned()))
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn a_merged_workout_never_takes_another_providers_activity_that_shares_its_id() {
        let ride = Utc.with_ymd_and_hms(2025, 6, 1, 8, 0, 0).unwrap();
        let year_later = Utc.with_ymd_and_hms(2026, 6, 1, 8, 0, 0).unwrap();
        // Newest first, as the window read returns them: the unrelated
        // Garmin activity holding id 123 comes before the Strava one.
        let rows = vec![
            row("garmin", "123", year_later),
            row("wahoo", "w-1", ride),
            row("strava", "123", ride),
        ];
        let mut found: Vec<Vec<(String, String)>> = workout_rows(&rows)
            .iter()
            .map(|workout| keys(&rows, workout))
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec![
                vec![("garmin".to_owned(), "123".to_owned())],
                vec![
                    ("strava".to_owned(), "123".to_owned()),
                    ("wahoo".to_owned(), "w-1".to_owned()),
                ],
            ],
            "bug: the Strava ride and its Wahoo twin are one workout; Garmin's 123 a year later is its own"
        );
    }

    #[test]
    fn every_row_takes_exactly_one_workout_its_canonical_copy_first() {
        let ride = Utc.with_ymd_and_hms(2025, 6, 1, 8, 0, 0).unwrap();
        let run = Utc.with_ymd_and_hms(2025, 6, 3, 8, 0, 0).unwrap();
        let rows = vec![
            row("strava", "r-1", run),
            row("wahoo", "w-1", ride),
            row("garmin", "g-1", ride),
            row("strava", "123", ride),
        ];
        let found = workout_rows(&rows);
        let mut seen: Vec<usize> = found.iter().flat_map(|w| w.rows.clone()).collect();
        seen.sort_unstable();
        assert_eq!(
            seen,
            (0..rows.len()).collect::<Vec<_>>(),
            "bug: a row is in no workout, or in two"
        );
        let ride_workout = found
            .iter()
            .find(|w| w.rows.len() == 3)
            .expect("bug: the ride's three copies are not one workout");
        let merged = workouts::workouts(&rows);
        let canonical = merged
            .iter()
            .find(|w| w.copies.len() == 2)
            .expect("bug: the merge does not hold the ride's three copies")
            .canonical;
        assert_eq!(
            row_key(&rows[ride_workout.rows[0]]),
            row_key(canonical),
            "bug: the ride's start is not looked for on its canonical copy first"
        );
    }
}
