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

use std::sync::Arc;

use chrono::{Duration, NaiveDate};
use dravr_cageux::training_load::{FormBand, TrainingLoad, TrainingLoadCalculator};
use pierre_core::errors::AppResult;
use pierre_core::models::{DailyTrainingState, TenantId};
use pierre_fitness_compute::{ACWR_ACUTE_DAYS, ACWR_CHRONIC_DAYS};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::read_history_from_cache;
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
    // The trend spans one chronic window: the configured one, never a figure
    // of this page's own.
    let trend_days = runtime
        .cageux_config()
        .algorithms
        .params
        .training_load_ctl_days;
    let read = read_history_from_cache(
        runtime,
        tenant_id,
        user_id,
        today - Duration::days(trend_days),
        today,
    )
    .await?;
    Ok(status_from_rows(today, &read.states))
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
