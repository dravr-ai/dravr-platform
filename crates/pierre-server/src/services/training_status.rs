// ABOUTME: The athlete's training status for Home — form today and its band, the form trend, recent load against baseline, recovery days
// ABOUTME: Every figure is read off the cached daily rollup and cageux's own bands; a day the stored history cannot warm is absent, never zero

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Training status, as Home shows it.
//!
//! Nothing here is computed for this page alone. The daily rows are the ones
//! `get_training_history` serves, computed from the stored activities by
//! [`read_history_from_cache`]; form and its band are cageux's
//! ([`FormReading`]), the load ratio is the rollup's `acwr`, and the recovery
//! days are [`TrainingLoadCalculator::recommend_recovery_days`], the figure
//! `generate_recommendations` hands the agent. The page and a conversation
//! about the same day therefore quote the same numbers.
//!
//! Two framing rules hold on this payload as they do on every other surface:
//! form is a share of the athlete's own fitness and is banded on that share,
//! never on the raw balance; and the load ratio is a magnitude against the
//! athlete's own baseline, never a verdict.
//!
//! A day the stored history cannot warm carries no row, so an athlete with too
//! little history gets `form: null` and an empty trend, not a zeroed reading.
//!
//! ## How deep the stored history must be
//!
//! The page reads `status_window`: one chronic window (`ctl_days`, 42 by
//! default) ending on today. Each day of it is computed only with
//! `warmup_days(ctl_days)` (`ctl_days + CTL_WARMUP_MARGIN_DAYS`, 72 by
//! default) of stored activities behind it, so form today needs history
//! reaching 72 days back and the whole trend needs `2 * ctl_days + 30` — 114
//! days. The page itself never reaches a provider; a connect asks the capture
//! rail for exactly that depth through [`request_status_depth`], so what a
//! connect lands is what this page needs (carnet#1043). The recent window a
//! connect reads at once vouches for no day of it.

use std::sync::Arc;

use chrono::{Duration, NaiveDate};
use dravr_cageux::training_load::{FormBand, TrainingLoad, TrainingLoadCalculator};
use pierre_core::errors::AppResult;
use pierre_core::models::{DailyTrainingState, TenantId};
use pierre_fitness_compute::{ACWR_ACUTE_DAYS, ACWR_CHRONIC_DAYS};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::{
    compute_and_persist_history, default_window, read_history_from_cache, TrainingHistoryComputed,
};
use serde::Serialize;
use uuid::Uuid;

/// Body of `GET /api/me/training-status`.
#[derive(Debug, Clone, Serialize)]
pub struct TrainingStatus {
    /// The athlete's civil date the status is read on, `YYYY-MM-DD`.
    pub today: NaiveDate,
    /// Form on `today`; `null` when the stored history cannot stand behind
    /// that day.
    pub form: Option<FormToday>,
    /// Form on each day of the trend the stored history stands behind, oldest
    /// first, ending on `today`. It spans the configured chronic window when
    /// the history is deep enough, fewer days on a thin one, and is empty when
    /// `form` is `null`; its own first and last dates say which.
    pub trend: Vec<FormTrendPoint>,
    /// The recent load against the athlete's own baseline; `null` until the
    /// baseline window holds enough history.
    pub load_ratio: Option<LoadRatio>,
    /// Lighter days cageux recommends from today's form; `null` when form
    /// cannot be judged.
    pub recovery_days: Option<u32>,
}

/// Form on one day, as a share of the athlete's own fitness.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FormToday {
    /// The band the share falls in; `insufficient_history` when there is no
    /// chronic base to scale form against.
    pub band: FormBand,
    /// Form as a whole percentage of fitness; `null` with no chronic base.
    pub pct_of_fitness: Option<f64>,
}

/// One day of the form trend.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FormTrendPoint {
    /// The athlete's civil date, `YYYY-MM-DD`.
    pub date: NaiveDate,
    /// The band form fell in that day.
    pub band: FormBand,
    /// Form as a whole percentage of fitness; `null` with no chronic base.
    pub pct_of_fitness: Option<f64>,
}

/// The recent load as a multiple of the athlete's own baseline.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct LoadRatio {
    /// Mean daily load of the last `acute_days` over that of the last
    /// `chronic_days`. A magnitude, not a verdict.
    pub ratio: f64,
    /// Days in the recent window.
    pub acute_days: i64,
    /// Days in the baseline window.
    pub chronic_days: i64,
}

/// The athlete's training status on `today`, their civil date.
///
/// Reads the stored activities and computes; it writes nothing and reaches no
/// provider, so a Home visit costs a cache read.
///
/// # Errors
///
/// Returns the repository error when the stored activities, the athlete's
/// thresholds or their timezone cannot be read.
pub async fn training_status(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    today: NaiveDate,
) -> AppResult<TrainingStatus> {
    let (from, to) = status_window(today, configured_ctl_days(runtime));
    let read = read_history_from_cache(runtime, tenant_id, user_id, from, to).await?;
    Ok(status_from_rows(today, &read.states))
}

/// The span of the athlete's days the status reads: one chronic window of
/// `ctl_days` ending on `today`, inclusive.
///
/// The one definition of the page's window, read by the page and by the
/// connect that lands its history ([`request_status_depth`]).
#[must_use]
fn status_window(today: NaiveDate, ctl_days: i64) -> (NaiveDate, NaiveDate) {
    (today - Duration::days(ctl_days), today)
}

/// The configured chronic window: the trend spans it, never a figure of this
/// page's own.
fn configured_ctl_days(runtime: &Arc<dyn ToolRuntime>) -> i64 {
    runtime
        .cageux_config()
        .algorithms
        .params
        .training_load_ctl_days
}

/// Make sure the stored history reaches as deep as the status needs, asking
/// the capture rail for what is missing.
///
/// Called when an athlete connects a provider. It computes `status_window`
/// on the athlete's own today through the training-history rail, which asks
/// every capturable connection to page back to the window's warm-up floor
/// when the stored history stops short of it — deduplicated per
/// `(user, provider)` against a capture already in flight, and bounded by
/// the rail's own fetch limit and per-provider page ceiling. With the depth
/// already stored it captures nothing and only refreshes the rollup.
///
/// # Errors
///
/// Returns [`AppError`](pierre_core::errors::AppError) when the athlete's
/// timezone cannot be read, they have no connection, or a repository read or
/// write fails.
pub async fn request_status_depth(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<TrainingHistoryComputed> {
    let (_, today) = default_window(runtime, user_id).await?;
    let (from, to) = status_window(today, configured_ctl_days(runtime));
    compute_and_persist_history(runtime, tenant_id, user_id, from, to).await
}

/// Assemble the status from the daily rows the stored history stands behind.
///
/// Today's figures come from today's row alone: a series that stops short of
/// `today` answers `form: null` rather than passing an older day off as now.
#[must_use]
pub fn status_from_rows(today: NaiveDate, rows: &[DailyTrainingState]) -> TrainingStatus {
    let current = rows.last().filter(|row| row.date == today);
    let Some(current) = current else {
        return TrainingStatus {
            today,
            form: None,
            trend: Vec::new(),
            load_ratio: None,
            recovery_days: None,
        };
    };
    let reading = current.form_reading();
    let recovery_days = reading.form_pct.map(|_| {
        TrainingLoadCalculator::recommend_recovery_days(&TrainingLoad {
            ctl: current.ctl,
            atl: current.atl,
            tsb: current.tsb,
            form_ctl: current.form_ctl,
            tss_history: Vec::new(),
        })
    });
    TrainingStatus {
        today,
        form: Some(FormToday {
            band: reading.band,
            pct_of_fitness: reading.form_pct.map(f64::round),
        }),
        trend: rows
            .iter()
            .map(|row| {
                let reading = row.form_reading();
                FormTrendPoint {
                    date: row.date,
                    band: reading.band,
                    pct_of_fitness: reading.form_pct.map(f64::round),
                }
            })
            .collect(),
        load_ratio: current.acwr.map(|ratio| LoadRatio {
            ratio,
            acute_days: ACWR_ACUTE_DAYS,
            chronic_days: ACWR_CHRONIC_DAYS,
        }),
        recovery_days,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, NaiveDate};
    use pierre_fitness_compute::training_history_compute::warmup_days;
    use pierre_services::training_history_read::first_vouched_day;

    use super::status_window;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    /// The history depth the whole page needs, derived from the same
    /// functions the read uses: its window plus that window's warm-up.
    fn depth_needed(ctl_days: i64) -> i64 {
        let today = day(2026, 10, 15);
        let (from, _) = status_window(today, ctl_days);
        (today - from).num_days() + warmup_days(ctl_days)
    }

    #[test]
    fn the_window_is_one_chronic_window_ending_today() {
        let today = day(2026, 10, 15);
        assert_eq!(status_window(today, 42), (day(2026, 9, 3), today));
    }

    /// History reaching exactly the depth vouches for the whole window, and a
    /// day less leaves its first day unvouched: the depth is the minimum, not
    /// a guess.
    #[test]
    fn the_depth_is_exactly_what_vouches_the_first_day() {
        let today = day(2026, 10, 15);
        let ctl_days = 42;
        let (from, _) = status_window(today, ctl_days);
        let warmup = warmup_days(ctl_days);
        let deep_enough = today - Duration::days(depth_needed(ctl_days));
        assert_eq!(first_vouched_day(from, deep_enough, warmup), from);
        let one_short = deep_enough + Duration::days(1);
        assert!(first_vouched_day(from, one_short, warmup) > from);
    }

    /// Thirty days of history, the recent window a connect reads at once,
    /// vouch for no day of the window: alone, it leaves the page empty.
    #[test]
    fn thirty_days_of_history_vouch_for_nothing() {
        let today = day(2026, 10, 15);
        let (from, to) = status_window(today, 42);
        let oldest = today - Duration::days(30);
        assert!(first_vouched_day(from, oldest, warmup_days(42)) > to);
    }
}
