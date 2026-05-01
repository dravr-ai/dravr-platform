// ABOUTME: Endurance daily training-state rollup row — CTL/ATL/TSB/ACWR/monotony/strain/ramp_rate/daily_load
// ABOUTME: Backs the training_history table; one row per (tenant_id, user_id, date)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One day of derived training-state metrics for a single user.
///
/// All metrics are computed from the user's activity stream by the
/// `training_history_compute` service. Frameworks:
///
/// - `ctl` / `atl` / `tsb` / `daily_load` — Coggan TSS-based EMA
/// - `acwr` — Gabbett Acute:Chronic Workload Ratio (7d / 28d)
/// - `monotony` — Foster (mean weekly load / std dev weekly load)
/// - `strain` — Foster (weekly load × monotony)
/// - `ramp_rate` — CTL change over the prior 7 days (`CTL_today` - `CTL_7d_ago`)
///
/// Optional metrics are `None` when there is not enough history to compute
/// them — e.g. ACWR needs at least 28 days of activity, monotony needs at
/// least 7 days. `None` here is the deterministic-output rule applied to
/// derived state: never substitute zero for "insufficient history".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyTrainingState {
    /// Calendar date this row covers (UTC).
    pub date: NaiveDate,
    /// Chronic Training Load (42-day EMA of TSS) — proxy for fitness.
    pub ctl: f64,
    /// Acute Training Load (7-day EMA of TSS) — proxy for fatigue.
    pub atl: f64,
    /// Training Stress Balance (`CTL - ATL`) — proxy for form / freshness.
    pub tsb: f64,
    /// Acute:Chronic Workload Ratio (Gabbett). `None` until 28+ days of history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acwr: Option<f64>,
    /// Foster monotony — weekly mean / weekly std dev of daily load.
    /// `None` until 7+ days of non-zero load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monotony: Option<f64>,
    /// Foster strain — weekly sum × monotony. `None` when monotony is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strain: Option<f64>,
    /// CTL change over the previous 7 days (`CTL_today - CTL_7d_ago`).
    /// `None` until 7+ days of CTL exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ramp_rate: Option<f64>,
    /// Sum of TSS for activities on this date.
    pub daily_load: f64,
}

impl DailyTrainingState {
    /// Construct an all-zero (rest-day) state for a given date.
    #[must_use]
    pub const fn zero(date: NaiveDate) -> Self {
        Self {
            date,
            ctl: 0.0,
            atl: 0.0,
            tsb: 0.0,
            acwr: None,
            monotony: None,
            strain: None,
            ramp_rate: None,
            daily_load: 0.0,
        }
    }
}

/// Composite key for the `training_history` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DailyTrainingKey {
    /// Tenant scope.
    pub tenant_id: Uuid,
    /// User this row belongs to.
    pub user_id: Uuid,
    /// Calendar date the row covers.
    pub date: NaiveDate,
}
