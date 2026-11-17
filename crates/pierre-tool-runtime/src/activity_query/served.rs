// ABOUTME: A served list's count from the one store read: the window it covers, its filter, merged across sources
// ABOUTME: Also names the served rows the store does not hold, which the count leaves out rather than guessing at

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The count behind a served list (carnet#1485, OQ6).
//!
//! `get_activities` serves a page read from a provider and merged with the
//! athlete's other connections; its window's count and totals come from
//! here, the store's read of the same window under the same filter, so a
//! window listed there and counted by [`query_activities`](super::query_activities)
//! carries one count.
//!
//! A live read the provider vouched for is written to the store before the
//! list answers. A served row the store still does not hold (a capture whose
//! head the provider never saw, a row the store refused, a write that failed)
//! is not counted: it is named [`ServedCount::unstored`], so the count never
//! silently disagrees with the page.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::civil_time::{local_date, resolve_zone};
use pierre_core::errors::AppResult;
use pierre_core::models::coverage::WindowTotals;
use pierre_core::models::{Activity, TenantId};
use pierre_providers::ai_scope;
use pierre_services::training_history_read::user_timezone;
use uuid::Uuid;

use super::{
    source_keys, visit_stored_workouts, Accumulator, ActivityQuery, Athlete, CivilWindow, KeySet,
    ReadGate,
};
use crate::runtime::ToolRuntime;

/// A list a count stands behind.
#[derive(Clone, Copy)]
pub struct ServedList<'a> {
    /// The rows it served, before its display limit.
    pub served: &'a [Activity],
    /// The instant its window opens at, when the ask set one; else its
    /// oldest row opens it.
    pub after: Option<DateTime<Utc>>,
    /// The instant its window closes at, when the ask set one; else now.
    pub before: Option<DateTime<Utc>>,
    /// The one source it was pinned to, by its backend: that source's keys
    /// ([`source_keys`]) are read, as a count pinned to it reads them. `None`
    /// reads every key the athlete holds.
    pub pinned: Option<&'a str>,
    /// The list's own filter, run on each merged workout as on each served
    /// row.
    pub keep: &'a (dyn Fn(&Activity) -> bool + Sync),
}

impl fmt::Debug for ServedList<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServedList")
            .field("served", &self.served.len())
            .field("after", &self.after)
            .field("before", &self.before)
            .field("pinned", &self.pinned)
            .finish_non_exhaustive()
    }
}

/// What the store counts behind a served list.
#[derive(Debug, Clone, PartialEq)]
pub struct ServedCount {
    /// The window's count, distance, duration and climbing.
    pub totals: WindowTotals,
    /// The oldest and newest civil day a counted workout started on.
    pub span: Option<(NaiveDate, NaiveDate)>,
    /// Served rows the store does not hold, left out of the count.
    pub unstored: u64,
}

/// Count `list`'s window from the store, as a model may read it.
///
/// One read ([`visit_stored_workouts`], [`ReadGate::Model`]) of the civil
/// days the window touches, merged; a workout counts when its merged start
/// is inside the window's instants and the list's filter keeps it, summed by
/// the one [`Accumulator`]. The read's withheld tally is dropped: the list's
/// own read tallied the same items. A served row is stored when the read
/// returned a row of its provider and id.
///
/// A list with no `after` and no rows has no window: nothing is counted.
///
/// # Errors
///
/// Returns the store's error when a read fails.
pub async fn count_served_list(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    list: ServedList<'_>,
) -> AppResult<ServedCount> {
    let opens = list
        .after
        .or_else(|| list.served.iter().map(Activity::start_date).min());
    let Some(opens) = opens else {
        return Ok(ServedCount {
            totals: WindowTotals {
                count: 0,
                distance_km: 0.0,
                duration_seconds: 0,
                elevation_gain_m: 0.0,
            },
            span: None,
            unstored: 0,
        });
    };
    let closes = list.before.unwrap_or_else(Utc::now);
    let zone = resolve_zone(user_timezone(runtime.repos(), user_id).await?.as_deref());
    let window = CivilWindow {
        from: Some(local_date(opens, zone)),
        to: local_date(closes, zone),
        zone,
    };
    let pinned_keys: Vec<&str> = list.pinned.map(source_keys).unwrap_or_default();
    let keys = if list.pinned.is_some() {
        KeySet::Only(&pinned_keys)
    } else {
        KeySet::All
    };

    let mut accumulator = Accumulator::new(&ActivityQuery::totals_only(), zone);
    let mut held: HashSet<(String, String)> = HashSet::new();
    let athlete = Athlete { user_id, tenant_id };
    let (read, _withheld) = ai_scope::tallied(visit_stored_workouts(
        runtime,
        athlete,
        window,
        keys,
        ReadGate::Model,
        &mut |workouts| {
            for workout in workouts {
                held.extend(workout.rows().map(|row| {
                    (
                        row.activity.provider().to_owned(),
                        row.activity.id().to_owned(),
                    )
                }));
                let start = workout.session.start_date();
                if (opens..=closes).contains(&start) && (list.keep)(&workout.session) {
                    accumulator.add(workout);
                }
            }
        },
    ))
    .await;
    read?;
    let folded = accumulator.finish();
    let unstored = list
        .served
        .iter()
        .filter(|row| !held.contains(&(row.provider().to_owned(), row.id().to_owned())))
        .count();
    Ok(ServedCount {
        totals: folded.window_totals,
        span: folded.span,
        unstored: u64::try_from(unstored).unwrap_or(u64::MAX),
    })
}
