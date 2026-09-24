// ABOUTME: Shared statements, row decoder and body for the dravr-riviere TimeSeriesStore over data_point_series
// ABOUTME: One SQL text per operation; each backend shell supplies only its type
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The riviere time-series store, written once.
//!
//! `data_point_series` keys on text ids on both backends; `recorded_at`
//! binds as a [`DateTime<Utc>`], which sqlx-sqlite encodes to the bytes
//! `to_rfc3339()` writes, so the half-open range comparisons order as they
//! always did on the `SQLite` TEXT column while Postgres gets its
//! `TIMESTAMPTZ`. Nothing on this table is spelled differently per engine.

use chrono::{DateTime, Utc};
use dravr_riviere::{DataPoint, RiviereError};

// ============================================================================
// data_point_series (riviere TimeSeriesStore)
// ============================================================================

/// Raw points for a `(source, series)` inside a half-open `[start, end)`
/// range, ascending, excluding soft-deleted rows.
pub(crate) const TS_FETCH_RANGE_SQL: &str = r"
            SELECT recorded_at, value
            FROM data_point_series
            WHERE data_source_id = $1
              AND series_type_id = $2
              AND recorded_at >= $3
              AND recorded_at < $4
              AND deleted_at IS NULL
            ORDER BY recorded_at ASC
            ";

/// Insert one point; a second insert at the same timestamp replaces the
/// first (riviere's last-writer-wins on the unique key).
pub(crate) const TS_INSERT_POINT_SQL: &str = r"
                INSERT INTO data_point_series
                    (id, data_source_id, series_type_id, recorded_at, zone_offset, value)
                VALUES ($1, $2, $3, $4, NULL, $5)
                ON CONFLICT(data_source_id, series_type_id, recorded_at) DO UPDATE SET
                    value = EXCLUDED.value
                ";

/// The most recent live point of a series.
pub(crate) const TS_LATEST_SQL: &str = r"
            SELECT recorded_at, value
            FROM data_point_series
            WHERE data_source_id = $1
              AND series_type_id = $2
              AND deleted_at IS NULL
            ORDER BY recorded_at DESC
            LIMIT 1
            ";

/// Soft-delete every live point inside a half-open range: reads filter on
/// `deleted_at` while the row stays, matching the other health tables.
pub(crate) const TS_DELETE_RANGE_SQL: &str = r"
            UPDATE data_point_series
            SET deleted_at = $1
            WHERE data_source_id = $2
              AND series_type_id = $3
              AND recorded_at >= $4
              AND recorded_at < $5
              AND deleted_at IS NULL
            ";

/// Decode one `data_point_series` row into a riviere [`DataPoint`].
///
/// # Errors
/// Returns a storage error when `recorded_at` or `value` cannot be decoded.
pub(crate) fn data_point_from_row<R>(row: &R) -> Result<DataPoint, RiviereError>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    f64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let timestamp: DateTime<Utc> =
        row.try_get("recorded_at")
            .map_err(|e| RiviereError::Storage {
                message: format!("invalid recorded_at: {e}"),
            })?;
    let value: f64 = row.try_get("value").map_err(|e| RiviereError::Storage {
        message: format!("invalid value: {e}"),
    })?;
    Ok(DataPoint::new(timestamp, value))
}

/// Emit the [`TimeSeriesStore`] implementation for one backend type, with
/// the range fetch its `query` and `aggregate` paths share.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
///
/// [`TimeSeriesStore`]: dravr_riviere::TimeSeriesStore
macro_rules! impl_time_series_store {
    ($ty:ty) => {
        impl $ty {
            /// Fetch raw points for a `(source, series)` within a half-open `[start, end)`
            /// range, ascending by `recorded_at`, excluding soft-deleted rows. Shared by
            /// the `query` and `aggregate` paths of the `TimeSeriesStore` impl.
            async fn ts_fetch_range(
                &self,
                source_id: &str,
                series_type: u32,
                range: &TimeRange,
            ) -> Result<Vec<DataPoint>, RiviereError> {
                let rows = sqlx::query(TS_FETCH_RANGE_SQL)
                    .bind(source_id)
                    .bind(i64::from(series_type))
                    .bind(range.start)
                    .bind(range.end)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| RiviereError::Storage {
                        message: format!("query data_point_series: {e}"),
                    })?;

                rows.iter().map(data_point_from_row).collect()
            }
        }

        #[async_trait::async_trait]
        impl TimeSeriesStore for $ty {
            async fn insert(
                &self,
                source_id: &str,
                series_type: u32,
                point: DataPoint,
            ) -> Result<(), RiviereError> {
                self.insert_batch(source_id, series_type, vec![point]).await
            }

            async fn insert_batch(
                &self,
                source_id: &str,
                series_type: u32,
                points: Vec<DataPoint>,
            ) -> Result<(), RiviereError> {
                if points.is_empty() {
                    return Ok(());
                }

                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| RiviereError::Storage {
                        message: format!("begin tx for time-series insert: {e}"),
                    })?;

                for point in &points {
                    let id = Uuid::new_v4().to_string();
                    sqlx::query(TS_INSERT_POINT_SQL)
                        .bind(&id)
                        .bind(source_id)
                        .bind(i64::from(series_type))
                        .bind(point.timestamp)
                        .bind(point.value)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| RiviereError::Storage {
                            message: format!("insert data_point_series row: {e}"),
                        })?;
                }

                tx.commit().await.map_err(|e| RiviereError::Storage {
                    message: format!("commit time-series insert: {e}"),
                })?;

                Ok(())
            }

            async fn query(
                &self,
                source_id: &str,
                series_type: u32,
                range: &TimeRange,
            ) -> Result<QueryResult, RiviereError> {
                let points = self.ts_fetch_range(source_id, series_type, range).await?;
                Ok(QueryResult::from_points(points))
            }

            async fn aggregate(
                &self,
                source_id: &str,
                series_type: u32,
                range: &TimeRange,
                window_secs: i64,
                aggregation: Aggregation,
            ) -> Result<Vec<AggregatedPoint>, RiviereError> {
                let points = self.ts_fetch_range(source_id, series_type, range).await?;
                Ok(aggregate_windows(
                    &points,
                    range.start,
                    range.end,
                    window_secs,
                    aggregation,
                ))
            }

            async fn latest(
                &self,
                source_id: &str,
                series_type: u32,
            ) -> Result<Option<DataPoint>, RiviereError> {
                let row = sqlx::query(TS_LATEST_SQL)
                    .bind(source_id)
                    .bind(i64::from(series_type))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| RiviereError::Storage {
                        message: format!("latest data_point_series: {e}"),
                    })?;

                row.as_ref().map(data_point_from_row).transpose()
            }

            async fn delete_range(
                &self,
                source_id: &str,
                series_type: u32,
                range: &TimeRange,
            ) -> Result<u64, RiviereError> {
                let res = sqlx::query(TS_DELETE_RANGE_SQL)
                    .bind(Utc::now())
                    .bind(source_id)
                    .bind(i64::from(series_type))
                    .bind(range.start)
                    .bind(range.end)
                    .execute(self.pool())
                    .await
                    .map_err(|e| RiviereError::Storage {
                        message: format!("delete_range data_point_series: {e}"),
                    })?;

                Ok(res.rows_affected())
            }
        }
    };
}
pub(crate) use impl_time_series_store;
