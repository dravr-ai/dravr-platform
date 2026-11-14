// ABOUTME: A deleted activity lets its gear go: its attribution handed to another copy of the workout, or cleared
// ABOUTME: Also the workout merge the gear ledger reads its copies through, shared by every gear reader and writer

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Gear release, and the workouts the gear ledger reads (carnet#711).
//!
//! A run a watch synced to Strava and to intervals.icu is two cached copies of
//! one workout, merged by the session merger the Home list and the chat turn
//! use
//! ([`merge_duplicates`](pierre_providers::deduplication::merge_duplicates))
//! into a [`LedgerWorkout`](crate::gear_release::LedgerWorkout). When the
//! athlete or a provider deletes one copy, its attribution goes with it
//! ([`release_activity_gear`](crate::gear_release::release_activity_gear)):
//! handed to another copy of the same workout when one is left, so the choice
//! stays and the workout still counts once, and cleared otherwise.
//!
//! It lives here, beside the activity store's projection that calls it on every
//! retraction, so whatever runs the store — the server, or the CLI's seeders —
//! releases gear the one way.

use chrono::Duration;
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, ActivityGearKey, TenantId};
use pierre_database::repositories::{CachedActivityRow, CachedRowsQuery};
use pierre_database::RepositoryRegistry;
use pierre_providers::deduplication::{is_canonical_copy, merge_duplicates, DedupConfig};
use uuid::Uuid;

/// How far either side of an activity's start its workout's other copies are
/// looked for.
///
/// The merger chains copies whose starts lie within an hour or so
/// of each other; a day either side holds every copy of any workout a person
/// records, and every attribution recorded on one of them.
pub const WORKOUT_WINDOW_HOURS: i64 = 24;

/// Most cached rows read around one activity: far more than the copies of
/// every workout one athlete records in two days.
pub const WORKOUT_ROW_LIMIT: i64 = 200;

/// One workout among the athlete's cached copies.
#[derive(Debug, Clone)]
pub struct LedgerWorkout {
    /// The merged session: its sport, start and distance are the workout's.
    pub session: Activity,
    /// The copy the merged session was built on, where a default is recorded.
    pub canonical: ActivityGearKey,
    /// Every copy of the workout, the canonical one included.
    pub copies: Vec<ActivityGearKey>,
}

/// The ledger key of a cached copy.
#[must_use]
pub fn row_key(row: &CachedActivityRow) -> ActivityGearKey {
    ActivityGearKey {
        provider: row.provider.clone(),
        activity_id: row.activity.id().to_owned(),
    }
}

/// The workouts `rows` hold, each with the keys of its copies.
#[must_use]
pub fn ledger_workouts(rows: &[CachedActivityRow]) -> Vec<LedgerWorkout> {
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
#[must_use]
pub fn workout_distance(session: &Activity) -> f64 {
    session
        .distance_meters()
        .filter(|meters| meters.is_finite() && *meters > 0.0)
        .unwrap_or(0.0)
}

/// The other copies of `copy`'s workout among `rows`: the same session
/// recorded by another source, paired as the gear ledger pairs them. Empty
/// when `copy` is its workout's only copy, or not among `rows`.
#[must_use]
pub fn workout_twins(rows: &[CachedActivityRow], copy: &ActivityGearKey) -> Vec<ActivityGearKey> {
    ledger_workouts(rows)
        .into_iter()
        .find(|workout| workout.copies.contains(copy))
        .map(|workout| {
            workout
                .copies
                .into_iter()
                .filter(|key| key != copy)
                .collect()
        })
        .unwrap_or_default()
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

/// Let go of the pair the caller's activity `activity_id` from `provider`
/// (the provider key it is cached under) is attributed to, before that copy
/// is deleted.
///
/// Another copy of the same workout takes the attribution over
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
pub async fn release_activity_gear(
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
            &CachedRowsQuery::window(start - window, start + window, WORKOUT_ROW_LIMIT),
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
    use chrono::{TimeZone, Utc};
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
            segment_read: false,
            stored_at: None,
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
        assert_eq!(workouts.len(), 2, "the rows make two workouts");
        let mut morning: Vec<String> = workouts[0]
            .copies
            .iter()
            .map(|key| format!("{}/{}", key.provider, key.activity_id))
            .collect();
        morning.sort();
        assert_eq!(
            morning,
            ["intervals_icu/i1", "strava/s1"],
            "the morning workout holds both copies"
        );
        assert!(
            workouts[0].copies.contains(&workouts[0].canonical),
            "a workout's canonical copy is one of its copies"
        );
        assert_eq!(
            workouts[1].copies,
            vec![ActivityGearKey {
                provider: "strava".to_owned(),
                activity_id: "s2".to_owned(),
            }],
            "the evening workout holds its one copy"
        );
        assert!(
            (workout_distance(&workouts[1].session) - 5_000.0).abs() < f64::EPSILON,
            "the evening workout keeps its distance"
        );
    }

    #[test]
    fn a_copys_twins_are_its_workouts_other_copies_only() {
        let key = |provider: &str, id: &str| ActivityGearKey {
            provider: provider.to_owned(),
            activity_id: id.to_owned(),
        };
        let rows = vec![
            cached("strava_archive", "4242", 0, 10_000.0),
            cached("strava", "4242", 0, 10_000.0),
            cached("garmin", "g1", 1, 10_050.0),
        ];
        let mut twins = workout_twins(&rows, &key("strava_archive", "4242"));
        twins.sort_by(|a, b| a.provider.cmp(&b.provider));
        assert_eq!(
            twins,
            vec![key("garmin", "g1"), key("strava", "4242")],
            "the twins are the workout's other copies"
        );
        let alone = [cached("strava", "s9", 0, 10_000.0)];
        assert_eq!(
            workout_twins(&alone, &key("strava", "s9")),
            Vec::new(),
            "a lone copy has no twins"
        );
        assert_eq!(
            workout_twins(&rows, &key("strava", "absent")),
            Vec::new(),
            "an absent copy has no twins"
        );
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
            segment_read: false,
            stored_at: None,
        };
        assert!(
            workout_distance(&row.activity).abs() < f64::EPSILON,
            "a workout without a distance adds zero"
        );
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
            Some(key("intervals_icu", "i1")),
            "strava's copy hands over to intervals"
        );
        assert_eq!(
            successor_copy(&rows, &key("intervals_icu", "i1")),
            Some(key("strava", "s1")),
            "intervals' copy hands over to strava"
        );
        assert_eq!(
            successor_copy(&rows, &key("strava", "s2")),
            None,
            "a lone copy hands over to none"
        );
        assert_eq!(
            successor_copy(&rows, &key("strava", "gone")),
            None,
            "an absent copy hands over to none"
        );
    }
}
