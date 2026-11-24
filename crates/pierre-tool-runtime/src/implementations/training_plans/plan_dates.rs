// ABOUTME: The calendar domain a saved plan's dates must fall in, and the reader that enforces it
// ABOUTME: A date past the years the date types hold is refused for its year, not for its shape
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::{CivilDate, DateError};
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::training_plans::parse_plan_date;

/// Calendar domain a plan date must fall in.
///
/// `parse_plan_date` is a format check: it takes any canonical date the date
/// types hold, years -9999 to 9999, so `0999-08-08` round-trips and would
/// persist. A stored plan is re-rendered into the system prompt on every
/// subsequent turn, so the save path bounds every date to the years a plan
/// can mean.
///
/// The last plan year is 9998, one short of the date types' last, because a
/// week renders only when it can be closed: the renderers add six days to its
/// start with checked additions and pass over a week that leaves the calendar.
/// `CivilDate::MAX` is `9999-12-31`, so a week starting after `9999-12-25`
/// would be accepted on save and then silently dropped from every render. The
/// latest week this bound accepts starts `9998-12-31` and ends `9999-01-06`,
/// inside the calendar.
const MIN_PLAN_YEAR: i32 = 1000;
const MAX_PLAN_YEAR: i32 = 9998;

/// Parse a plan date, accepting only a canonical `YYYY-MM-DD` inside the
/// calendar domain plans are rendered in (see [`MIN_PLAN_YEAR`]).
///
/// `field` names the payload field in the rejection so the model's next
/// iteration knows which date to fix.
pub(crate) fn plan_date(field: &str, raw: &str) -> AppResult<CivilDate> {
    let outside = |year: i64| {
        AppError::invalid_input(format!(
            "{field} year {year} is outside {MIN_PLAN_YEAR}-{MAX_PLAN_YEAR}, got '{raw}'"
        ))
    };
    let Some(date) = parse_plan_date(raw) else {
        // A well-formed date past the years the date types hold (-9999 to
        // 9999) is refused for its year, the same answer as any other year
        // outside the plan domain, so the model fixes the year it chose.
        if CivilDate::parse_from_str(raw, "%Y-%m-%d") == Err(DateError::OutOfRange) {
            // Only a year the reader could not hold: a 30 February is out of
            // range too, and stays a shape the model has to fix.
            if let Some(year) = stated_year(raw).filter(|year| {
                !(i64::from(MIN_PLAN_YEAR)..=i64::from(MAX_PLAN_YEAR)).contains(year)
            }) {
                return Err(outside(year));
            }
        }
        return Err(AppError::invalid_input(format!(
            "{field} must be YYYY-MM-DD, got '{raw}'"
        )));
    };
    let year = date.year();
    if !(MIN_PLAN_YEAR..=MAX_PLAN_YEAR).contains(&year) {
        return Err(outside(i64::from(year)));
    }
    Ok(date)
}

/// The signed year a `[+-]YYYY…-MM-DD` text states, read without the
/// calendar's bounds.
fn stated_year(raw: &str) -> Option<i64> {
    let (sign, rest) = match raw.as_bytes().first()? {
        b'+' => (1, &raw[1..]),
        b'-' => (-1, &raw[1..]),
        _ => (1, raw),
    };
    let digits = rest.split('-').next()?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok().map(|year| sign * year)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_past_the_held_years_is_refused_for_its_year_and_a_bad_day_for_its_shape() {
        let past = plan_date("goal_race.date", "+262142-12-31")
            .unwrap_err()
            .to_string();
        assert!(past.contains("year 262142 is outside 1000-9998"), "{past}");
        let before = plan_date("goal_race.date", "-262143-01-01")
            .unwrap_err()
            .to_string();
        assert!(
            before.contains("year -262143 is outside 1000-9998"),
            "{before}"
        );
        let bad_day = plan_date("goal_race.date", "2026-02-30")
            .unwrap_err()
            .to_string();
        assert!(
            bad_day.contains("must be YYYY-MM-DD, got '2026-02-30'"),
            "{bad_day}"
        );
    }

    #[test]
    fn every_accepted_week_start_can_be_closed_inside_the_calendar() {
        let last = plan_date("week_start", "9998-12-31").unwrap();
        assert_eq!(
            last.checked_add_days(6),
            CivilDate::from_ymd(9999, 1, 6).ok(),
            "the latest accepted week must close, or the renderers skip it"
        );
        for raw in ["9999-01-01", "9999-12-26", "9999-12-31"] {
            let refused = plan_date("week_start", raw).unwrap_err().to_string();
            assert!(
                refused.contains("year 9999 is outside 1000-9998"),
                "{raw}: {refused}"
            );
        }
    }
}
