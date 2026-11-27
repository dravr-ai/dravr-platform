// ABOUTME: Intervals.icu planned-workout read — the events request, then the pairing request when the window can hold done work
// ABOUTME: Inherent methods on IntervalsIcuProvider; the wire shapes and the mapping are the intervals_icu_planned module's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The two requests behind
//! [`FitnessProvider::list_planned_workouts`](crate::core::FitnessProvider::list_planned_workouts)
//! for Intervals.icu: `GET /events` filtered to the workout and race
//! categories, and `GET /activities?fields=id,paired_event_id` for which
//! activity completed which event. Both go to the athlete's path, so a
//! delegated provider reads the coached athlete's calendar and nobody else's.

use std::collections::HashMap;

use dravr_tronc::dates::{CivilDate, Timestamp};
use serde_json::Value;
use tracing::info;

use super::{IntervalsIcuProvider, Target};
use crate::errors::AppResult;
use crate::intervals_icu_planned::{
    completed_by_event, may_hold_done_workouts, planned_categories, planned_events, pushed_events,
    IntervalsIcuActivityPairing, PAIRING_FIELDS,
};
use crate::models::{PlannedCalendar, PlannedWorkout};

impl IntervalsIcuProvider {
    /// The workouts and races the calendar plans over `[after, before]`.
    ///
    /// Each names the activity that completed it, as the calendar shows the
    /// provider's reader (the athlete, or a coach reading them): the planned
    /// half of [`Self::planned_calendar`], one read serving both.
    ///
    /// # Errors
    ///
    /// Returns the [`Self::planned_calendar`] error.
    pub(super) async fn planned_workouts(
        &self,
        after: CivilDate,
        before: CivilDate,
    ) -> AppResult<Vec<PlannedWorkout>> {
        Ok(self.planned_calendar(after, before).await?.planned)
    }

    /// The calendar over `[after, before]`: its planned workouts and Dravr's pushes.
    ///
    /// Every workout and race the calendar shows the provider's reader, each
    /// naming the activity that completed it, and the events Dravr pushed
    /// with the activity the calendar paired with each (carnet#1792). The
    /// pairing is a second request, spent only when an event was found and
    /// the window can hold a workout already done. A refused or unreadable
    /// pairing fails the read: an absent completion means "not done", never
    /// "not known".
    ///
    /// # Errors
    ///
    /// Returns [`AppError`](crate::errors::AppError) when either request is
    /// refused or fails, read on the athlete's path as every athlete-scoped
    /// call is.
    pub(super) async fn planned_calendar(
        &self,
        after: CivilDate,
        before: CivilDate,
    ) -> AppResult<PlannedCalendar> {
        let entries: Vec<Value> = self
            .get_events(after, before, Some(&planned_categories()))
            .await?;
        let pushed = pushed_events(&entries);
        let events = planned_events(entries, self.planned_reader);
        let holds_any = !events.is_empty() || !pushed.is_empty();
        let completed = if holds_any && may_hold_done_workouts(after, Timestamp::now().utc_date()) {
            completed_by_event(self.list_activity_pairing(after, before).await?)
        } else {
            HashMap::new()
        };
        info!(
            count = events.len(),
            pushed = pushed.len(),
            %after,
            %before,
            "Intervals.icu planned-calendar read"
        );
        Ok(PlannedCalendar {
            planned: events
                .into_iter()
                .map(|event| event.into_workout(&completed))
                .collect(),
            pushed: Some(
                pushed
                    .into_iter()
                    .map(|event| event.into_pairing(&completed))
                    .collect(),
            ),
        })
    }

    /// The activities of `[oldest, newest]`, each with the planned event it
    /// completed. `newest` is read as the start of its day, so the day after
    /// the window's last one goes on the wire.
    async fn list_activity_pairing(
        &self,
        oldest: CivilDate,
        newest: CivilDate,
    ) -> AppResult<Vec<IntervalsIcuActivityPairing>> {
        let through = newest.succ().unwrap_or(newest);
        let query = [
            ("oldest", oldest.to_string()),
            ("newest", through.to_string()),
            ("fields", PAIRING_FIELDS.to_owned()),
        ];
        self.get_athlete_json(
            "list_activity_pairing",
            "/activities",
            &query,
            Target::Athlete,
        )
        .await
    }
}
