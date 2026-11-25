// ABOUTME: Pure helpers of the backfill notice: its transport stamp and one activity's list line
// ABOUTME: No I/O; the notifier in the parent module composes them into the delivered notice
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write as _;

use dravr_tronc::dates::Zone;
use pierre_core::ai_policy::first_party_only;
use pierre_core::civil_time::format_local_day;
use pierre_core::constants::oauth_providers::activity_attribution;
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
///
/// The device that recorded it follows the name, `• name (Garmin Forerunner
/// 965) · …`: a Garmin-recorded entry carries the attribution Garmin's brand
/// guidelines ask of every entry of a list, as the activity list the agent
/// reads does (carnet#521, carnet#1154).
pub(super) fn format_activity_line(activity: &Activity, locale: &str, zone: &Zone) -> String {
    // The day on the athlete's calendar and the sport in their language, from
    // the same helpers the activity list the agent reads uses: the notice is
    // stored in the conversation the model reads back, so a UTC date or an
    // English sport here would contradict that list.
    let date = format_local_day(activity.start_date(), zone, locale);
    let sport = localized_sport_name(activity.sport_type(), locale);
    // The list is persisted in the conversation the model reads back, and the
    // name is whatever the provider account's writers typed.
    let name = display_line(activity.name(), ACTIVITY_NAME_MAX_CHARS);
    let attribution = activity_attribution(activity.source(), activity.device_name())
        .map_or_else(String::new, |label| format!(" ({label})"));
    let mut line = format!("• {name}{attribution} · {date} · {sport}");
    if let Some(meters) = activity.distance_meters() {
        if meters > 0.0 {
            // Infallible write into a String; the trait import is `as _` so the
            // method resolves without exposing `Write` to the rest of the file.
            let _ = write!(line, " · {:.1} km", meters / 1000.0);
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::{Timestamp, Zone};
    use pierre_core::models::{ActivityBuilder, SportType};

    use super::format_activity_line;

    /// A Garmin-recorded entry of the "history ready" list names Garmin and
    /// its device beside its name, as Garmin's brand guidelines ask of each
    /// entry of a list; an entry no device names reads as before.
    #[test]
    fn a_garmin_entry_carries_its_attribution_beside_its_name() {
        let start = Timestamp::parse_from_rfc3339("2026-11-20T07:00:00Z").unwrap_or_default();
        let garmin = ActivityBuilder::new("1", "Tempo", SportType::Run, start, 2_400, "garmin")
            .source("garmin")
            .device_name("Garmin Forerunner 965")
            .distance_meters(10_000.0)
            .build();
        let line = format_activity_line(&garmin, "en", &Zone::utc());
        assert!(
            line.starts_with("• Tempo (Garmin Forerunner 965) · "),
            "{line}"
        );
        assert!(line.ends_with(" · 10.0 km"), "{line}");

        let plain =
            ActivityBuilder::new("2", "Easy", SportType::Run, start, 1_800, "strava").build();
        let line = format_activity_line(&plain, "en", &Zone::utc());
        assert!(line.starts_with("• Easy · "), "{line}");
        assert!(!line.contains('('), "{line}");
    }
}
