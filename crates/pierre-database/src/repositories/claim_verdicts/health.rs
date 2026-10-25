// ABOUTME: Folds the claim-verdict health and calibration query rows into their response types
// ABOUTME: Shared by the SQLite and PostgreSQL implementations; re-exported from claim_verdicts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use pierre_core::errors::{AppError, AppResult};
use pierre_memory::VerdictDisposition;

use super::{
    VerdictAgentHealth, VerdictCategoryHealth, VerdictHealthDay, VerdictHealthTotals,
    VerdictLayerHealth, VerdictReasonCount, VerdictStatusBreakdown,
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

/// Decode every row of one breakdown statement.
pub(crate) fn health_breakdown_rows<R>(rows: &[R]) -> AppResult<Vec<HealthBreakdownRow>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    rows.iter().map(health_breakdown_from_row).collect()
}

/// Decode one `(reason, count)` row of the reason table.
pub(crate) fn reason_count_from_row<R>(row: &R) -> AppResult<VerdictReasonCount>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str, e: sqlx::Error| {
        AppError::database(format!("claim_verdicts health {name}: {e}"))
    };
    Ok(VerdictReasonCount {
        reason: row.try_get("reason").map_err(|e| col("reason", e))?,
        count: row.try_get("c").map_err(|e| col("c", e))?,
    })
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
