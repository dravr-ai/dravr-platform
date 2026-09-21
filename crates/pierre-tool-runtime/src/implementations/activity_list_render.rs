// ABOUTME: Renders the activity window as the numbered prose list the agent reads
// ABOUTME: Owns the athlete's civil clock for that block — the one surface that quotes dates
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The activity list the agent actually reads.
//!
//! This block is the agent's only view of the athlete's training on a grounded
//! turn: [`super::super::implementations::fitness_support`] hands it the window
//! and the prefetch injects it verbatim. It is also athlete-visible — the
//! messaging envelope ships it as the `ActivityList` reply block — so it is
//! prose for two audiences at once.
//!
//! ## Whose clock this is
//!
//! The athlete's. Every timestamp here renders in their timezone, through
//! [`pierre_core::civil_time::format_local_stamp`] — the same helper
//! `pierre_chat_pipeline::stages::prompt_assembly::format_current_date` uses for
//! `{{CURRENT_DATE}}`, so both carry the same `%Y-%m-%d <weekday> %H:%M` shape.
//! That agreement is the point, and it is load-bearing.
//!
//! Rendering the rows in UTC while the prompt anchor said local put two
//! calendars in one prompt. On 2026-08-28 an athlete in `America/Toronto` was
//! told "today is 2026-08-27 22:59 (America/Toronto)" and handed a row stamped
//! `2026-08-28` for a hike he had started 22:59 the previous evening — Strava had
//! named it "Night Hike". The agent reconciled the contradiction the only way it
//! could: it read the row's date as local and invented a time of day to match,
//! reporting the night hike as "ce matin". Every activity after ~20:00 local was
//! attributed to the following day, so day-counting and rest-day reasoning drifted
//! with it.
//!
//! Both clocks derive from the same stored `users.timezone` and both fall back to
//! UTC together, so an athlete with no timezone on file still sees one coherent
//! frame rather than two.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::BuildHasher;

use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::civil_time::{
    clock_date, format_civil_day, format_local_stamp, local_date, relative_day, relative_day_label,
    resolve_zone,
};
use pierre_core::models::Activity;
use pierre_providers::deduplication::FragmentReport;

use super::sport_labels::localized_sport_name;

/// Format activities as a numbered human-readable list for LLM output.
///
/// This helps smaller models include the list in their response without
/// transforming JSON. Activities render in the order the caller established
/// (`get_activities` sorts by the requested `sort_by` before the display
/// limit).
///
/// `backfill_temps` carries weather-backfilled temperatures keyed by activity id;
/// used when the provider didn't surface ambient temp on the row itself
/// (sciotte / Whoop / Fitbit / Terra all leave it empty).
///
/// `fragment_report` carries fragment-deduplication metadata when overlapping
/// recordings of the same workout were detected; when `Some` and at least one
/// group is present, a header note is prepended so the LLM sees the
/// session-vs-row distinction inline with the list (smaller models that skip
/// the structured `retrieval_context` JSON still get the cue from the prose).
///
/// `now` is the wall clock the list is rendered against. It is a parameter, not
/// a read, so the day boundaries it draws are testable at the instants that
/// matter — the athlete's midnight, which is not the server's.
///
/// ## Today is stated, and so is yesterday
///
/// The list opens with the athlete's current day and the day of the newest row
/// it holds, and every row that falls on today or yesterday says so next to its
/// date — `2026-09-20 dim 16:59 (hier)`. On 2026-09-21 a Telegram athlete asked
/// whether a 30-minute ride *that morning* would be too much; the coach answered
/// about the ride he had done the previous afternoon, called it "ce matin", and
/// dated the rest of his week a day early to match. The prompt's date anchor
/// said 09-21 and the row said 09-20; the model took the newest row as today
/// anyway, which is the one thing the platform contract tells it never to do.
/// A weaker fallback model is exactly where that instruction is weakest, and
/// the fallback is what answered that turn.
///
/// The server already names the weekday for the same reason it now names
/// today: one subtraction is still calendar arithmetic, and the model's record
/// at calendar arithmetic is the whole history of this module.
#[must_use]
pub fn format_activities_as_list<S: BuildHasher>(
    activities: &[Activity],
    backfill_temps: &HashMap<String, f32, S>,
    fragment_report: Option<&FragmentReport>,
    locale: &str,
    user_timezone: Option<&str>,
    now: DateTime<Utc>,
) -> String {
    // Resolved once per call, not per row. An absent or unparseable zone falls
    // back to UTC exactly as `format_current_date` does, so the list and the
    // prompt's date anchor stay in one frame even when the athlete has no
    // timezone on file.
    let zone = resolve_zone(user_timezone);
    // `clock_date`, not `local_date`: a reading of the wall clock is never a
    // date-only sentinel, and the five-minute floor the prompt anchor applies
    // lands on midnight UTC once a day (see `format_clock_stamp`).
    let today = clock_date(now, zone);

    let mut lines = Vec::with_capacity(activities.len() + 6);
    lines.push("Your Activities:".to_owned());
    // The newest row in the list, not in the window: the caller may have sorted
    // by distance and cut to a display limit, and the model reads only what is
    // rendered below.
    let newest = activities
        .iter()
        .map(|a| local_date(a.start_date(), zone))
        .max();
    lines.push(newest.map_or_else(
        || format!("[Today] {}", format_civil_day(today, locale)),
        |day| {
            format!(
                "[Today] {} — newest activity listed: {}{}",
                format_civil_day(today, locale),
                format_civil_day(day, locale),
                relative_day_tag(day, today, locale)
            )
        },
    ));
    lines.push(String::new());
    if let Some(report) = fragment_report {
        if report.has_fragments() {
            lines.push(format!(
                "[Note] {raw} GPS recordings detected, representing ~{sessions} distinct training sessions.",
                raw = report.raw_count,
                sessions = report.session_count,
            ));
            lines.push(
                "[Note] The following appear to be fragments of the same workout (count sessions, not rows):"
                    .to_owned(),
            );
            for group in &report.groups {
                let ids = group.fragment_ids.join(", ");
                lines.push(format!(
                    "       - canonical {canon}; group: [{ids}] ({sport}, {start} → {end})",
                    canon = group.canonical_id,
                    sport = localized_sport_name(&group.sport_type, locale),
                    start = group
                        .window_start
                        .with_timezone(&zone)
                        .format("%Y-%m-%d %H:%M"),
                    end = group
                        .window_end
                        .with_timezone(&zone)
                        .format("%Y-%m-%d %H:%M"),
                ));
            }
            lines.push(String::new());
        }
    }

    // Render in the order the caller already established (get_activities sorts
    // by the requested `sort_by` before the display limit). Re-sorting here
    // would override "longest to shortest" / "oldest first" back to date order.
    for (i, activity) in activities.iter().enumerate() {
        // The weekday is named, not left to the model. Deriving it from a bare
        // date is calendar arithmetic, the same error class the epoch table in
        // `prompt_assembly` exists to remove — and on 2026-09-02 it cost an
        // athlete three rounds of corrections before he left the conversation.
        let date = format_local_stamp(activity.start_date(), zone, locale);
        // Same date the stamp shows (`local_date` keeps a date-only row on the
        // day its provider named), so the tag can never disagree with it.
        let day_tag = relative_day_tag(local_date(activity.start_date(), zone), today, locale);
        // Render the sport with its localized short label (fr "trail"/"rando",
        // not the English "trail run") so the list reads natively in the user's
        // chat language; `Other` keeps its provider-supplied label, unknown
        // locales fall back to English.
        let sport = localized_sport_name(activity.sport_type(), locale);
        let distance_km = activity.distance_meters().unwrap_or(0.0) / 1000.0;
        let duration_secs = activity.duration_seconds();
        let hours = duration_secs / 3600;
        let minutes = (duration_secs % 3600) / 60;
        let seconds = duration_secs % 60;

        let duration_str = if hours > 0 {
            format!("{hours}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes}:{seconds:02}")
        };

        // Append scalar sensor fields when the provider returned them.
        // Small models that skip the JSON tool result still see the
        // enrichment inline — no more "I don't have HR" when the data
        // was on the row all along. `write!` on a String is infallible
        // so the Result is intentionally discarded — matches
        // clippy::format_push_string guidance.
        let mut extras = String::new();
        match (activity.average_heart_rate(), activity.max_heart_rate()) {
            (Some(avg), Some(max)) => {
                let _ = write!(extras, " - HR {avg}/{max}");
            }
            (Some(avg), None) => {
                let _ = write!(extras, " - HR {avg} avg");
            }
            (None, Some(max)) => {
                let _ = write!(extras, " - HR {max} max");
            }
            (None, None) => {}
        }
        if let Some(elevation) = activity.elevation_gain() {
            // Round to whole meters — the agent reasoning doesn't need
            // decimals and the Strava field comes as Option<f32> which
            // sometimes carries spurious fractional noise.
            #[allow(clippy::cast_possible_truncation)]
            let rounded = elevation.round() as i64;
            let _ = write!(extras, " - +{rounded}m");
        }
        if let Some(calories) = activity.calories() {
            let _ = write!(extras, " - {calories} kcal");
        }
        // Prefer the provider-surfaced temperature; fall back to the
        // weather-backfill side-table for activities whose provider
        // didn't capture ambient temp (sciotte / Whoop / Fitbit / Terra).
        let temp = activity
            .temperature()
            .or_else(|| backfill_temps.get(activity.id()).copied());
        if let Some(temp) = temp {
            // Round to whole degrees — sub-degree precision is meaningless to
            // the agent reasoning loop and the providers report 1-decimal at
            // best. The leading sign survives `{:.0}` for sub-zero readings.
            let _ = write!(extras, " - {temp:.0}°C");
        }

        lines.push(format!(
            "{}. [{}] {} - {}{} - {:.2} km - {}{}",
            i + 1,
            sport,
            activity.name(),
            date,
            day_tag,
            distance_km,
            duration_str,
            extras
        ));
    }

    lines.join("\n")
}

/// ` (hier)` / ` (aujourd'hui)` for a day that is yesterday or today on the
/// athlete's calendar, and nothing for any other day — a plain date already
/// carries its weekday, and "two days ago" is not how anyone refers to a
/// session.
fn relative_day_tag(day: NaiveDate, today: NaiveDate, locale: &str) -> String {
    relative_day(day, today).map_or_else(String::new, |rel| {
        format!(" ({})", relative_day_label(rel, locale))
    })
}
