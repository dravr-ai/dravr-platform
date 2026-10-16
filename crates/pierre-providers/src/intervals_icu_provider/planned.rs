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

use chrono::{NaiveDate, Utc};
use serde_json::Value;
use tracing::info;

use super::{IntervalsIcuProvider, Target};
use crate::errors::AppResult;
use crate::intervals_icu_planned::{
    completed_by_event, may_hold_done_workouts, planned_categories, planned_events,
    IntervalsIcuActivityPairing, PAIRING_FIELDS,
};
use crate::models::PlannedWorkout;

impl IntervalsIcuProvider {
    /// The workouts and races the calendar plans over `[after, before]`,
    /// each naming the activity that completed it.
    ///
    /// The pairing is a second request, spent only when an event was kept
    /// and the window can hold a workout already done. A refused or
    /// unreadable pairing fails the read: an absent completion means "not
    /// done", never "not known".
    ///
    /// # Errors
    ///
    /// Returns [`AppError`](crate::errors::AppError) when either request is
    /// refused or fails, read on the athlete's path as every athlete-scoped
    /// call is.
    pub(super) async fn planned_workouts(
        &self,
        after: NaiveDate,
        before: NaiveDate,
    ) -> AppResult<Vec<PlannedWorkout>> {
        let entries: Vec<Value> = self
            .get_events(after, before, Some(&planned_categories()))
            .await?;
        let events = planned_events(entries);
        let completed =
            if !events.is_empty() && may_hold_done_workouts(after, Utc::now().date_naive()) {
                completed_by_event(self.list_activity_pairing(after, before).await?)
            } else {
                HashMap::new()
            };
        info!(count = events.len(), %after, %before, "Intervals.icu planned-workout read");
        Ok(events
            .into_iter()
            .map(|event| event.into_workout(&completed))
            .collect())
    }

    /// The activities of `[oldest, newest]`, each with the planned event it
    /// completed. `newest` is read as the start of its day, so the day after
    /// the window's last one goes on the wire.
    async fn list_activity_pairing(
        &self,
        oldest: NaiveDate,
        newest: NaiveDate,
    ) -> AppResult<Vec<IntervalsIcuActivityPairing>> {
        let through = newest.succ_opt().unwrap_or(newest);
        let query = [
            ("oldest", oldest.format("%Y-%m-%d").to_string()),
            ("newest", through.format("%Y-%m-%d").to_string()),
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
