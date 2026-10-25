// ABOUTME: Pure helpers of the backfill notice: its transport stamp and one activity's list line
// ABOUTME: No I/O; the notifier in the parent module composes them into the delivered notice
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write as _;

use chrono_tz::Tz;
use pierre_core::ai_policy::first_party_only;
use pierre_core::civil_time::format_local_day;
use pierre_core::models::Activity;
use pierre_core::transport::TransportPolicy;
use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};
use pierre_providers::registry::global_registry;
use pierre_tool_runtime::implementations::sport_labels::localized_sport_name;

/// The stamp of a backfill notice over `provider`'s warmed window: the
/// provider's own terms, and those of any upstream source its rows relay
/// (carnet#769).
pub(super) fn warmed_window_policy(provider: &str, warmed: &[Activity]) -> TransportPolicy {
    let terms = global_registry();
    TransportPolicy::from_first_party_only(
        first_party_only(terms.as_ref(), provider, None)
            || warmed
                .iter()
                .any(|a| first_party_only(terms.as_ref(), a.provider(), a.source())),
    )
}

/// Render one activity as a compact `• name · day · sport · 10.0 km` line.
/// Distance is shown in kilometres (one decimal) only when present. Pure
/// formatting over the cheaply-available `Activity` accessors — no allocation
/// beyond the returned line.
pub(super) fn format_activity_line(activity: &Activity, locale: &str, zone: Tz) -> String {
    // The day on the athlete's calendar and the sport in their language, from
    // the same helpers the activity list the agent reads uses: the notice is
    // stored in the conversation the model reads back, so a UTC date or an
    // English sport here would contradict that list.
    let date = format_local_day(activity.start_date(), zone, locale);
    let sport = localized_sport_name(activity.sport_type(), locale);
    // The list is persisted in the conversation the model reads back, and the
    // name is whatever the provider account's writers typed.
    let name = display_line(activity.name(), ACTIVITY_NAME_MAX_CHARS);
    let mut line = format!("• {name} · {date} · {sport}");
    if let Some(meters) = activity.distance_meters() {
        if meters > 0.0 {
            // Infallible write into a String; the trait import is `as _` so the
            // method resolves without exposing `Write` to the rest of the file.
            let _ = write!(line, " · {:.1} km", meters / 1000.0);
        }
    }
    line
}
