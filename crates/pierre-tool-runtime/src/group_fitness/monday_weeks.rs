// ABOUTME: A group member's weekly figures: this Monday week and the one before, from the one activity aggregator
// ABOUTME: The weeks the Home volume card counts, in the one tenant the member's Home opens in; a week not vouched for has no figure

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use dravr_tronc::dates::civil_clock::monday_of;
use dravr_tronc::dates::{CivilDate, Elapsed};
use pierre_core::constants::units::METERS_PER_KM;
use pierre_core::models::groups::MemberFitnessSnapshot;
use pierre_core::models::TenantId;
use tracing::warn;
use uuid::Uuid;

use crate::activity_query::{
    query_activities, ActivityPredicate, ActivityQuery, GroupKey, KeyValue, QueryAnswer,
    QueryMetric, ReadGate,
};
use crate::runtime::ToolRuntime;

/// What a member's weekly figures sum: the Home volume card's own count,
/// distance and elapsed time.
const WEEK_METRICS: [QueryMetric; 3] = [
    QueryMetric::Count,
    QueryMetric::DistanceKm,
    QueryMetric::ElapsedTimeS,
];

/// One Monday-to-Sunday week of a member's training, as the aggregator
/// counts it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct WeekFigures {
    /// Workouts.
    sessions: u64,
    /// Metres, over the workouts that recorded a distance.
    metres: f64,
    /// Elapsed seconds.
    seconds: u64,
}

/// A member's current and previous Monday weeks on their own calendar.
///
/// A week the stored history does not vouch for
/// ([`QueryAnswer::first_vouched_week`]) is `None`, never zero figures: a
/// member whose history is not stored yet, or starts after the week, has no
/// number for it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct MondayWeeks {
    this_week: Option<WeekFigures>,
    previous_week: Option<WeekFigures>,
}

impl MondayWeeks {
    /// The two weeks as the answer to the [`week_query`] of `today` counts
    /// them: each week it vouches for carries its figures, and a week it
    /// does not vouch for carries none.
    fn from_answer(today: CivilDate, answer: &QueryAnswer) -> Self {
        let this_monday = monday_of(today);
        let previous_monday = this_monday - Elapsed::weeks(1);
        let first = answer.first_vouched_week(previous_monday);
        let week_of = |monday: CivilDate| {
            if first.is_none_or(|first| first > monday) {
                return None;
            }
            let mut figures = WeekFigures::default();
            let key = KeyValue::Text(monday.to_string());
            let groups = answer
                .groups
                .iter()
                .filter(|group| group.key.get(&GroupKey::Week) == Some(&key));
            for group in groups {
                let values = &group.values;
                figures.sessions = figures.sessions.saturating_add(values.count.unwrap_or(0));
                figures.metres += values.distance_metres().unwrap_or(0.0);
                figures.seconds = figures
                    .seconds
                    .saturating_add(values.elapsed_time_s.unwrap_or(0));
            }
            Some(figures)
        };
        Self {
            this_week: week_of(this_monday),
            previous_week: week_of(previous_monday),
        }
    }

    /// Write the weeks into the snapshot's weekly figures.
    pub(super) fn fill(self, snapshot: &mut MemberFitnessSnapshot) {
        snapshot.weekly_activity_count = self
            .this_week
            .map(|week| i32::try_from(week.sessions).unwrap_or(i32::MAX));
        snapshot.weekly_volume_km = self.this_week.map(|week| week.metres / METERS_PER_KM);
        snapshot.weekly_duration_seconds = self.this_week.map(|week| week.seconds);
        snapshot.previous_week_volume_km =
            self.previous_week.map(|week| week.metres / METERS_PER_KM);
    }
}

/// The count behind a member's weekly figures: their stored workouts from
/// the Monday of the week before `today`'s to `today`, by Monday week.
fn week_query(today: CivilDate) -> ActivityQuery {
    ActivityQuery {
        from: Some(monday_of(today) - Elapsed::weeks(1)),
        to: today,
        filter: ActivityPredicate::default(),
        group_by: vec![GroupKey::Week],
        metrics: WEEK_METRICS.to_vec(),
        provider: None,
    }
}

/// The tenant a member's Home opens in: the first tenant they joined, the
/// one every sign-in names for their session, so the one their Home volume
/// card counts. `fallback` stands for a member who belongs to no tenant and
/// so has no Home. `None` when the memberships cannot be read.
async fn home_tenant(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    fallback: TenantId,
) -> Option<TenantId> {
    match runtime.repos().tenants.first_for_user(user_id).await {
        Ok(home) => Some(home.unwrap_or(fallback)),
        Err(e) => {
            warn!(
                user_id = %user_id,
                error = %e,
                "Snapshot: the member's home tenant could not be read; no week is vouched for"
            );
            None
        }
    }
}

/// A member's current and previous Monday weeks, read from the one
/// aggregator ([`query_activities`]) the Home volume card reads, in the one
/// tenant that card reads ([`home_tenant`]), under that tenant's filter.
///
/// One tenant, never a sum over the tenants the member holds a connection
/// in: a workout stored under two of them would count twice, and the figure
/// would stop being the one the member sees on their own Home.
///
/// Read as [`ReadGate::Roster`]: Home's rows and vouching rule, less what
/// the member's consents withhold from a model; no history walk is asked.
/// A read that fails vouches for no week, so a failure reads as no figure,
/// never as a quiet week.
pub(super) async fn read_monday_weeks(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    fallback_tenant_id: TenantId,
    today: CivilDate,
) -> MondayWeeks {
    let Some(tenant_id) = home_tenant(runtime, user_id, fallback_tenant_id).await else {
        return MondayWeeks::default();
    };
    let query = week_query(today);
    match query_activities(runtime, user_id, tenant_id, &query, ReadGate::Roster).await {
        Ok(answer) => MondayWeeks::from_answer(today, &answer),
        Err(e) => {
            warn!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                error = %e,
                "Snapshot: the weekly volume read failed; no week is vouched for"
            );
            MondayWeeks::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::{Timestamp, Zone};
    use pierre_core::models::coverage::{Completeness, Coverage, CoverageWindow, IncompleteReason};
    use pierre_core::models::{Activity, ActivityBuilder, SportType};
    use pierre_database::repositories::CachedActivityRow;
    use pierre_services::workouts::workouts;

    use super::super::empty_snapshot;
    use super::*;
    use crate::activity_query::{aggregate, AppliedQuery};

    /// Montreal, five hours behind UTC in November.
    const MONTREAL: &str = "America/Montreal";

    /// Wednesday 18 November 2026; its week starts Monday the 16th.
    fn wednesday() -> CivilDate {
        CivilDate::from_ymd(2026, 11, 18).unwrap()
    }

    /// A ride of `km` starting at `instant` (RFC 3339) lasting `seconds`.
    fn ride(id: &str, instant: &str, km: f64, seconds: u64) -> Activity {
        ActivityBuilder::new(
            id,
            "Ride",
            SportType::Ride,
            Timestamp::parse_from_rfc3339(instant).unwrap(),
            seconds,
            "strava",
        )
        .distance_meters(km * METERS_PER_KM)
        .build()
    }

    /// The aggregator's answer to [`week_query`] over `sessions`, stored one
    /// row each, on the calendar of `zone`, standing as `coverage` says.
    fn answer(zone: &str, sessions: &[Activity], coverage: Coverage) -> QueryAnswer {
        let rows: Vec<CachedActivityRow> = sessions
            .iter()
            .map(|activity| CachedActivityRow {
                provider: activity.provider().to_owned(),
                activity: activity.clone(),
                route: None,
                detail_read: false,
                segment_read: false,
                stored_at: None,
                place: None,
            })
            .collect();
        let query = week_query(wednesday());
        let (totals, groups, unknown) =
            aggregate(&workouts(&rows), &query, &zone.parse::<Zone>().unwrap());
        QueryAnswer {
            applied: AppliedQuery::of(&query),
            totals,
            groups_total: groups.len(),
            groups_truncated: false,
            groups,
            unknown,
            coverage,
            coverage_by_source: Vec::new(),
            garmin_contribution: None,
            segments: None,
        }
    }

    /// The member holds activities from before the two weeks.
    fn whole() -> Coverage {
        Coverage::complete(CoverageWindow::default())
    }

    fn weeks_of(answer: &QueryAnswer) -> MondayWeeks {
        MondayWeeks::from_answer(wednesday(), answer)
    }

    #[test]
    fn a_sunday_night_session_is_last_week_and_a_monday_one_this_week() {
        // 23:30 on Sunday the 15th in Montreal is 04:30 UTC on Monday the
        // 16th, and 09:00 on Monday is 14:00 UTC: a week read on UTC's
        // calendar puts both rides in this week.
        let sessions = [
            ride("sunday", "2026-11-16T04:30:00Z", 30.0, 3_600),
            ride("monday", "2026-11-16T14:00:00Z", 12.5, 2_700),
        ];
        let weeks = weeks_of(&answer(MONTREAL, &sessions, whole()));
        assert_eq!(
            weeks.previous_week,
            Some(WeekFigures {
                sessions: 1,
                metres: 30_000.0,
                seconds: 3_600
            }),
            "bug: the Sunday-night ride is not in last week on her calendar"
        );
        assert_eq!(
            weeks.this_week,
            Some(WeekFigures {
                sessions: 1,
                metres: 12_500.0,
                seconds: 2_700
            }),
            "bug: the Monday ride is not in this week on her calendar"
        );

        let mut snapshot = empty_snapshot(Uuid::nil(), "Ada".to_owned(), None, Timestamp::now());
        weeks.fill(&mut snapshot);
        assert_eq!(snapshot.weekly_activity_count, Some(1), "bug: sessions");
        assert_eq!(snapshot.weekly_volume_km, Some(12.5), "bug: this week's km");
        assert_eq!(
            snapshot.weekly_duration_seconds,
            Some(2_700),
            "bug: seconds"
        );
        assert_eq!(
            snapshot.previous_week_volume_km,
            Some(30.0),
            "bug: last week's km"
        );
    }

    #[test]
    fn a_vouched_week_without_training_is_a_real_zero() {
        let weeks = weeks_of(&answer(MONTREAL, &[], whole()));
        assert_eq!(
            weeks.this_week,
            Some(WeekFigures::default()),
            "bug: a vouched quiet week is not zero"
        );
        assert_eq!(
            weeks.previous_week,
            Some(WeekFigures::default()),
            "bug: a vouched quiet week before is not zero"
        );
    }

    #[test]
    fn a_history_not_stored_yet_gives_no_figure() {
        let nothing = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::NoStoredActivities),
            window: CoverageWindow::default(),
            trustworthy_from: None,
            pending: None,
        };
        let weeks = weeks_of(&answer(MONTREAL, &[], nothing));
        assert_eq!(
            weeks,
            MondayWeeks::default(),
            "bug: an unstored history answered a zero"
        );
        let mut snapshot = empty_snapshot(Uuid::nil(), "Ada".to_owned(), None, Timestamp::now());
        weeks.fill(&mut snapshot);
        assert_eq!(
            snapshot.weekly_volume_km, None,
            "bug: an unread week reads as 0 km"
        );
        assert_eq!(
            snapshot.weekly_activity_count, None,
            "bug: an unread week reads as 0 sessions"
        );
    }

    #[test]
    fn a_history_starting_this_week_vouches_for_this_week_alone() {
        // Her oldest stored ride is Tuesday the 17th: last week is not one
        // the store can speak for.
        let sessions = [ride("tuesday", "2026-11-17T17:00:00Z", 20.0, 3_000)];
        let shallow = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::ShallowHistory),
            window: CoverageWindow::default(),
            trustworthy_from: CivilDate::from_ymd(2026, 11, 17).ok(),
            pending: None,
        };
        let weeks = weeks_of(&answer(MONTREAL, &sessions, shallow));
        assert_eq!(
            weeks.previous_week, None,
            "bug: a week before the stored history read as zero"
        );
        assert_eq!(
            weeks.this_week.map(|week| week.sessions),
            Some(1),
            "bug: the week the history starts in is not counted"
        );
    }
}
