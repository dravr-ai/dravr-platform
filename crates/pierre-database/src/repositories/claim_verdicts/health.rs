// ABOUTME: Folds the claim-verdict health and calibration query rows into their response types
// ABOUTME: Shared by the SQLite and PostgreSQL implementations; re-exported from claim_verdicts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use dravr_colleur::claims::VerdictDisposition;
use pierre_core::constants::oauth_providers::column_excluded_from_ai_evaluation;
use pierre_core::errors::{AppError, AppResult};

use super::{
    VerdictAgentHealth, VerdictCategoryHealth, VerdictHealthDay, VerdictHealthStats,
    VerdictHealthTotals, VerdictLayerHealth, VerdictReasonCount, VerdictStatusBreakdown,
};

/// Fold one `(status, count)` pair into a breakdown.
///
/// An unrecognised status is skipped rather than rejected: it means a verdict
/// enum variant the calibration response has no column for yet, and a count it
/// cannot place is not a reason to fail the whole aggregate.
pub(crate) fn apply_status_count(breakdown: &mut VerdictStatusBreakdown, status: &str, count: i64) {
    match status {
        "supported" => breakdown.supported += count,
        "unsupported" => breakdown.unsupported += count,
        "contradicted" => breakdown.contradicted += count,
        "rhetorical" => breakdown.rhetorical += count,
        "unverifiable" => breakdown.unverifiable += count,
        _ => {}
    }
}

/// `numerator / denominator` as a share, `0.0` when there is nothing to divide by.
pub(super) fn ratio(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        return 0.0;
    }
    numerator as f64 / denominator as f64
}

/// One `(dim, disposition, count)` row of a health breakdown, already decoded.
///
/// Each statement also groups by the verdict's consent sources, so a row
/// standing for verdicts on replies drawn from data barred from AI evaluation
/// can be left out (carnet#1170); several rows may then share a key, which
/// every fold below sums.
pub(crate) struct HealthBreakdownRow {
    /// The grouping key; `None` only for the unattributed agent group.
    pub dim: Option<String>,
    /// The disposition column, parsed; `None` for an undisposed row.
    pub disposition: Option<VerdictDisposition>,
    /// How many rows share the key and disposition.
    pub count: i64,
}

/// Decode one breakdown row, rejecting a disposition the application does
/// not know rather than folding it into the undisposed count.
pub(crate) fn health_breakdown_from_row<R>(row: &R) -> AppResult<HealthBreakdownRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str, e: sqlx::Error| {
        AppError::database(format!("claim_verdicts health {name}: {e}"))
    };
    let dim: Option<String> = row.try_get("dim").map_err(|e| col("dim", e))?;
    let raw: Option<String> = row
        .try_get("disposition")
        .map_err(|e| col("disposition", e))?;
    let disposition = raw
        .as_deref()
        .map(|s| {
            VerdictDisposition::parse(s)
                .ok_or_else(|| AppError::internal(format!("Invalid verdict disposition: {s}")))
        })
        .transpose()?;
    let count: i64 = row.try_get("c").map_err(|e| col("c", e))?;
    Ok(HealthBreakdownRow {
        dim,
        disposition,
        count,
    })
}

/// Whether a grouped row stands for verdicts barred from AI evaluation
/// (carnet#1170): its consent sources name such a key.
fn excluded_row<R>(row: &R) -> AppResult<bool>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let sources: Option<String> = row
        .try_get("consent_sources")
        .map_err(|e| AppError::database(format!("claim_verdicts health consent_sources: {e}")))?;
    Ok(column_excluded_from_ai_evaluation(sources.as_deref()))
}

/// Decode every row of one breakdown statement that may feed a metric.
///
/// A row standing for verdicts barred from AI evaluation is left out
/// (carnet#1170).
pub(crate) fn health_breakdown_rows<R>(rows: &[R]) -> AppResult<Vec<HealthBreakdownRow>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        if !excluded_row(row)? {
            decoded.push(health_breakdown_from_row(row)?);
        }
    }
    Ok(decoded)
}

/// The reason table from its `(reason, consent_sources, count)` rows.
///
/// Each reason once, its count summed over the rows that may feed a metric
/// (carnet#1170), sorted by reason.
pub(crate) fn reason_counts<R>(rows: &[R]) -> AppResult<Vec<VerdictReasonCount>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str, e: sqlx::Error| {
        AppError::database(format!("claim_verdicts health {name}: {e}"))
    };
    let mut by_reason: BTreeMap<String, i64> = BTreeMap::new();
    for row in rows {
        if excluded_row(row)? {
            continue;
        }
        let reason: String = row.try_get("reason").map_err(|e| col("reason", e))?;
        let count: i64 = row.try_get("c").map_err(|e| col("c", e))?;
        *by_reason.entry(reason).or_default() += count;
    }
    Ok(by_reason
        .into_iter()
        .map(|(reason, count)| VerdictReasonCount { reason, count })
        .collect())
}

/// Fold breakdown rows into per-key totals, keyed by the grouping value.
///
/// A `BTreeMap` so every breakdown comes out sorted by key — and, for the
/// agent table, with the unattributed `None` group first.
pub(crate) fn fold_health_breakdown(
    rows: &[HealthBreakdownRow],
) -> BTreeMap<Option<String>, VerdictHealthTotals> {
    let mut out: BTreeMap<Option<String>, VerdictHealthTotals> = BTreeMap::new();
    for row in rows {
        out.entry(row.dim.clone())
            .or_default()
            .add(row.disposition, row.count);
    }
    out
}

/// The layer table from its folded breakdown. A `None` key cannot occur —
/// `layer_fired` is `NOT NULL` — and is skipped rather than invented.
pub(crate) fn layer_health(
    folded: BTreeMap<Option<String>, VerdictHealthTotals>,
) -> Vec<VerdictLayerHealth> {
    folded
        .into_iter()
        .filter_map(|(key, t)| {
            key.map(|layer| VerdictLayerHealth {
                layer,
                flagged: t.flagged,
                disposed: t.disposed,
                false_positives: t.false_positives,
                rate: t.false_positive_rate(),
            })
        })
        .collect()
}

/// The category table from its folded breakdown; `category` is `NOT NULL`.
pub(crate) fn category_health(
    folded: BTreeMap<Option<String>, VerdictHealthTotals>,
) -> Vec<VerdictCategoryHealth> {
    folded
        .into_iter()
        .filter_map(|(key, t)| {
            key.map(|category| VerdictCategoryHealth {
                category,
                flagged: t.flagged,
                disposed: t.disposed,
                false_positives: t.false_positives,
                rate: t.false_positive_rate(),
            })
        })
        .collect()
}

/// The agent table from its folded breakdown, unattributed group included.
pub(crate) fn agent_health(
    folded: BTreeMap<Option<String>, VerdictHealthTotals>,
) -> Vec<VerdictAgentHealth> {
    folded
        .into_iter()
        .map(|(agent_id, t)| VerdictAgentHealth {
            agent_id,
            flagged: t.flagged,
            disposed: t.disposed,
            false_positives: t.false_positives,
            rate: t.false_positive_rate(),
        })
        .collect()
}

/// The daily series from its folded breakdown; the day expression is never null.
pub(crate) fn daily_health(
    folded: BTreeMap<Option<String>, VerdictHealthTotals>,
) -> Vec<VerdictHealthDay> {
    folded
        .into_iter()
        .filter_map(|(key, t)| {
            key.map(|date| VerdictHealthDay {
                date,
                flagged: t.flagged,
                false_positives: t.false_positives,
            })
        })
        .collect()
}

/// Window totals: the layer breakdown summed, since every flagged row has
/// exactly one layer.
pub(crate) fn health_totals(rows: &[HealthBreakdownRow]) -> VerdictHealthTotals {
    let mut totals = VerdictHealthTotals::default();
    for row in rows {
        totals.add(row.disposition, row.count);
    }
    totals
}

/// The health read from its fetched breakdowns: by layer, category, agent and
/// day, in that order.
pub(crate) fn verdict_health_stats(
    window_start: String,
    window_days: i64,
    (layer_rows, category_rows, agent_rows, daily_rows): (
        Vec<HealthBreakdownRow>,
        Vec<HealthBreakdownRow>,
        Vec<HealthBreakdownRow>,
        Vec<HealthBreakdownRow>,
    ),
    by_reason: Vec<VerdictReasonCount>,
) -> VerdictHealthStats {
    let totals = health_totals(&layer_rows);
    let false_positive_rate = totals.false_positive_rate();
    VerdictHealthStats {
        window_start,
        window_days,
        totals,
        false_positive_rate,
        by_layer: layer_health(fold_health_breakdown(&layer_rows)),
        by_category: category_health(fold_health_breakdown(&category_rows)),
        by_agent: agent_health(fold_health_breakdown(&agent_rows)),
        by_reason,
        daily: daily_health(fold_health_breakdown(&daily_rows)),
    }
}
