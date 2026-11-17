// ABOUTME: The place a cached row's start belongs to, as every read of whole cached rows returns it
// ABOUTME: Joined from athlete_places on the row's own tenant, user and place_id; a gone place reads as none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What slice 6 (carnet#1484) derives beside a cached row.
//!
//! A row's place is written only by the places service
//! (`AthletePlaceRepository::replace_clustering` and `create_athlete_place`).
//! A list sync never writes it. Every read of whole cached rows returns it
//! as the row's `place`.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::PlaceRef;

/// The `athlete_places.name_source` of a name the athlete gave.
const ATHLETE_NAME_SOURCE: &str = "athlete";

/// Read the place a row's start belongs to out of one row of the window read
/// (`sql::GET_CACHED_ACTIVITY_ROWS_SQL`): `None` when the row has none, or
/// its `place_id` names a place that no longer exists.
///
/// # Errors
/// Returns a database error naming the column that cannot be decoded.
pub(crate) fn row_place_from_row<R>(row: &R) -> AppResult<Option<PlaceRef>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let text = |column: &str| -> AppResult<Option<String>> {
        row.try_get(column)
            .map_err(|e| AppError::database(format!("activity col {column}: {e}")))
    };
    let name = text("place_name")?;
    let name_source = text("place_name_source")?;
    Ok(text("place_id")?.map(|id| PlaceRef {
        id,
        name,
        athlete_named: name_source.as_deref() == Some(ATHLETE_NAME_SOURCE),
    }))
}
