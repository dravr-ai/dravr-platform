// ABOUTME: Statements and row decoder for user_physiology_sport_anchors, an athlete's anchors per sport family
// ABOUTME: Emitted into both backends by the profile repository's macro; every statement scopes by tenant and user
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::NaiveDate;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::{AnchorConfidence, AnchorValue, SportFamily};
use pierre_core::models::{MeasurementKind, SportAnchor, SportAnchorName};
use sqlx::Row;

/// One athlete's per-sport anchors, in sport then anchor order.
pub(crate) const LIST_SPORT_ANCHORS_SQL: &str = r"
            SELECT sport, anchor, value, low, high, kind, origin, as_of, method, confidence
            FROM user_physiology_sport_anchors
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY sport, anchor
            ";

/// Write or replace one per-sport anchor. The profile row must exist: the
/// anchor table's foreign key refuses an anchor with no profile beside it.
pub(crate) const UPSERT_SPORT_ANCHOR_SQL: &str = r"
            INSERT INTO user_physiology_sport_anchors (
                tenant_id, user_id, sport, anchor, value, low, high, kind, origin,
                as_of, method, confidence, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            ON CONFLICT (tenant_id, user_id, sport, anchor) DO UPDATE SET
                value = EXCLUDED.value,
                low = EXCLUDED.low,
                high = EXCLUDED.high,
                kind = EXCLUDED.kind,
                origin = EXCLUDED.origin,
                as_of = EXCLUDED.as_of,
                method = EXCLUDED.method,
                confidence = EXCLUDED.confidence,
                updated_at = CURRENT_TIMESTAMP
            ";

/// Remove one per-sport anchor.
pub(crate) const DELETE_SPORT_ANCHOR_SQL: &str = r"
            DELETE FROM user_physiology_sport_anchors
            WHERE tenant_id = $1 AND user_id = $2 AND sport = $3 AND anchor = $4
            ";

/// Rebuild one stored anchor from a row of [`LIST_SPORT_ANCHORS_SQL`].
///
/// `None` for a sport or anchor name this build does not know — a row a newer
/// revision wrote, read after a rollback — which is skipped rather than
/// failing every read of the athlete's anchors. A confidence that cannot be
/// read is low, so no unreadable value is ever trusted above its evidence.
///
/// # Errors
/// Returns a database error when a column cannot be decoded.
pub(crate) fn sport_anchor_from_row<R>(row: &R) -> AppResult<Option<SportAnchor>>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    NaiveDate: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let read = |e: sqlx::Error, which: &str| AppError::database(format!("read {which}: {e}"));
    let sport: String = row.try_get("sport").map_err(|e| read(e, "sport"))?;
    let anchor: String = row.try_get("anchor").map_err(|e| read(e, "anchor"))?;
    let (Some(sport), Some(anchor)) = (
        SportFamily::ALL
            .iter()
            .copied()
            .find(|family| family.as_str() == sport),
        SportAnchorName::parse(&anchor),
    ) else {
        // Names only: the vocabulary words, never the athlete's values.
        tracing::warn!(
            sport = %sport,
            anchor = %anchor,
            "skipped a per-sport anchor row this build cannot name"
        );
        return Ok(None);
    };
    let value: f64 = row.try_get("value").map_err(|e| read(e, "value"))?;
    let low: Option<f64> = row.try_get("low").map_err(|e| read(e, "low"))?;
    let high: Option<f64> = row.try_get("high").map_err(|e| read(e, "high"))?;
    let kind: String = row.try_get("kind").map_err(|e| read(e, "kind"))?;
    let origin: String = row.try_get("origin").map_err(|e| read(e, "origin"))?;
    let as_of: NaiveDate = row.try_get("as_of").map_err(|e| read(e, "as_of"))?;
    let method: Option<String> = row.try_get("method").map_err(|e| read(e, "method"))?;
    let confidence: String = row
        .try_get("confidence")
        .map_err(|e| read(e, "confidence"))?;
    // Stored from f32 values, so the narrowing restores what was written.
    #[allow(clippy::cast_possible_truncation)]
    let narrow = |v: f64| v as f32;
    Ok(Some(SportAnchor {
        sport,
        anchor,
        kind: MeasurementKind::parse_lenient(&kind),
        value: AnchorValue {
            value: narrow(value),
            low: low.map(narrow),
            high: high.map(narrow),
            origin: Some(origin),
            as_of: Some(as_of),
            method,
            confidence: AnchorConfidence::ALL
                .iter()
                .copied()
                .find(|level| level.as_str() == confidence)
                .unwrap_or_else(|| {
                    tracing::warn!(
                        confidence = %confidence,
                        "read an unknown anchor confidence as low"
                    );
                    AnchorConfidence::Low
                }),
        },
    }))
}

/// The origin and day a stored anchor must carry: the table keeps neither
/// optional, so an anchor without them is refused rather than stored undated.
///
/// # Errors
/// Returns an invalid-input error when either is missing.
pub(crate) fn sport_anchor_stamp(anchor: &SportAnchor) -> AppResult<(&str, NaiveDate)> {
    match (anchor.value.origin.as_deref(), anchor.value.as_of) {
        (Some(origin), Some(as_of)) => Ok((origin, as_of)),
        _ => Err(AppError::invalid_input(format!(
            "the {} {} anchor needs an origin and a day to be stored",
            anchor.sport,
            anchor.anchor.as_str()
        ))),
    }
}
