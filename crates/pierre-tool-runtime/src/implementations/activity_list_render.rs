// ABOUTME: Renders the activity window as the numbered prose list the coach reads
// ABOUTME: Owns the athlete's civil clock for that block — the one surface that quotes dates
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The activity list the coach actually reads.
//!
//! This block is the coach's only view of the athlete's training on a grounded
//! turn: [`super::super::implementations::fitness_support`] hands it the window
//! and the prefetch injects it verbatim. It is also athlete-visible — the
//! messaging envelope ships it as the `ActivityList` reply block — so it is
//! prose for two audiences at once.
//!
//! ## Whose clock this is
//!
//! The athlete's. Every timestamp here renders in their timezone, in the same
//! `%Y-%m-%d %H:%M` shape that
//! `pierre_chat_pipeline::stages::prompt_assembly::format_current_date` uses for
//! `{{CURRENT_DATE}}`. That agreement is the point, and it is load-bearing.
//!
//! Rendering the rows in UTC while the prompt anchor said local put two
//! calendars in one prompt. On 2026-08-28 an athlete in `America/Toronto` was
//! told "today is 2026-08-27 22:59 (America/Toronto)" and handed a row stamped
//! `2026-08-28` for a hike he had started 22:59 the previous evening — Strava had
//! named it "Night Hike". The coach reconciled the contradiction the only way it
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

use pierre_core::models::Activity;
use pierre_providers::deduplication::FragmentReport;

use super::fitness_support::localized_sport_name;

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
#[must_use]
pub fn format_activities_as_list<S: BuildHasher>(
    activities: &[Activity],
    backfill_temps: &HashMap<String, f32, S>,
    fragment_report: Option<&FragmentReport>,
    locale: &str,
    user_timezone: Option<&str>,
) -> String {
    // Resolved once per call, not per row. An absent or unparseable zone falls
    // back to UTC exactly as `format_current_date` does, so the list and the
    // prompt's date anchor stay in one frame even when the athlete has no
    // timezone on file.
    let zone = user_timezone
        .and_then(|tz| tz.parse::<chrono_tz::Tz>().ok())
        .unwrap_or(chrono_tz::UTC);

    let mut lines = Vec::with_capacity(activities.len() + 6);
    lines.push("Your Activities:".to_owned());
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
        let date = activity
            .start_date()
            .with_timezone(&zone)
            .format("%Y-%m-%d %H:%M")
            .to_string();
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
            // Round to whole meters — the coach reasoning doesn't need
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
            // the coach reasoning loop and the providers report 1-decimal at
            // best. The leading sign survives `{:.0}` for sub-zero readings.
            let _ = write!(extras, " - {temp:.0}°C");
        }

        lines.push(format!(
            "{}. [{}] {} - {} - {:.2} km - {}{}",
            i + 1,
            sport,
            activity.name(),
            date,
            distance_km,
            duration_str,
            extras
        ));
    }

    lines.join("\n")
}
