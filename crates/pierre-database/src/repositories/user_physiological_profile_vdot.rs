// ABOUTME: How a stated VDOT is stored on user_physiological_profiles: its vdot, vdot_as_of and vdot_method columns
// ABOUTME: Bound together and read back together; a VDOT without its day is refused, since the day is what ages it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::CivilDate;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::ProfileVdot;
use sqlx::Row;

/// The three columns a stated VDOT is stored in: all `None` together when
/// the profile holds none.
pub(crate) struct VdotBinds {
    pub(crate) value: Option<f64>,
    pub(crate) as_of: Option<CivilDate>,
    pub(crate) method: Option<String>,
}

/// The columns of a stated VDOT, all `None` when there is none.
pub(super) fn vdot_binds(vdot: Option<&ProfileVdot>) -> VdotBinds {
    VdotBinds {
        value: vdot.map(|v| v.value),
        as_of: vdot.map(|v| v.as_of),
        method: vdot.and_then(|v| v.method.clone()),
    }
}

/// Read a stated VDOT from `vdot`, `vdot_as_of` and `vdot_method`.
///
/// A VDOT with no day is refused rather than guessed: the day is what ages
/// it, and the binds always store one beside it. A day or method with no VDOT
/// describes nothing and reads as none.
///
/// # Errors
/// Returns a database error when a column cannot be decoded, or a VDOT is
/// stored without its day.
pub(super) fn vdot_from_row<R>(row: &R) -> AppResult<Option<ProfileVdot>>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    CivilDate: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let read = |e: sqlx::Error, which: &str| AppError::database(format!("read {which}: {e}"));
    let Some(value) = row
        .try_get::<Option<f64>, _>("vdot")
        .map_err(|e| read(e, "vdot"))?
    else {
        return Ok(None);
    };
    let as_of: CivilDate = row
        .try_get::<Option<CivilDate>, _>("vdot_as_of")
        .map_err(|e| read(e, "vdot_as_of"))?
        .ok_or_else(|| AppError::database("vdot is stored without its vdot_as_of day"))?;
    let method: Option<String> = row
        .try_get("vdot_method")
        .map_err(|e| read(e, "vdot_method"))?;
    Ok(Some(ProfileVdot {
        value,
        as_of,
        method,
    }))
}
