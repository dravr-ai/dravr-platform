// ABOUTME: Prompt stage 7b.1 — the unit system the reader writes in, so the agent's distances match Home's
// ABOUTME: Resolved by pierre_services::units (Settings, then the provider's setting, then the device locale), carnet#835
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The units section of the system prompt.
//!
//! Every tool reports metric — that is what the platform stores and computes
//! in — so without this section the agent writes kilometres to an athlete
//! whose Home, activity view and route marks read miles. The section names
//! the reader's resolved system and, for imperial, tells the agent to convert
//! what the tools report.
//!
//! The reader is the turn's sender. A group-scoped conversation is read by
//! the group, so its replies stay metric — the neutral system, as the digest
//! posted into a group room does; a private conversation follows the sender.

use pierre_core::models::UnitSystem;
use pierre_database::RepositoryRegistry;
use pierre_services::units::resolve_user_units;
use uuid::Uuid;

/// The section for a reader on metric units.
const METRIC_UNITS_CONTEXT: &str = "\n\n## Units\n\n\
The person this reply is for reads metric units: distances in kilometres, \
elevation in metres, pace per kilometre and speed in km/h. Write every \
distance, elevation, pace and speed in those units.";

/// The section for a reader on imperial units.
const IMPERIAL_UNITS_CONTEXT: &str = "\n\n## Units\n\n\
The person this reply is for reads imperial units: distances in miles, \
elevation in feet, pace per mile and speed in mph. Your tools report \
kilometres, metres, pace per kilometre and km/h, so convert before you write \
(1 mi = 1.609 km, 1 ft = 0.3048 m) and write every distance, elevation, pace \
and speed in miles, feet, minutes per mile and mph. A race keeps the name of \
its distance (a 10K, a half marathon).";

/// The section for `system`.
#[must_use]
pub const fn units_context(system: UnitSystem) -> &'static str {
    match system {
        UnitSystem::Metric => METRIC_UNITS_CONTEXT,
        UnitSystem::Imperial => IMPERIAL_UNITS_CONTEXT,
    }
}

/// Append the reader's units section to `base_prompt`: metric when
/// `group_scoped` (several people read the reply), otherwise the sender's.
pub async fn append_units_context(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    group_scoped: bool,
    base_prompt: String,
) -> String {
    let system = if group_scoped {
        UnitSystem::Metric
    } else {
        resolve_user_units(
            repos.users.as_ref(),
            repos.unit_preferences.as_ref(),
            user_id,
            None,
        )
        .await
        .resolved
        .system
    };
    format!("{base_prompt}{}", units_context(system))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imperial_tells_the_agent_to_convert_what_the_tools_report() {
        let section = units_context(UnitSystem::Imperial);
        assert!(section.starts_with("\n\n## Units\n\n"));
        assert!(section.contains("miles"));
        assert!(section.contains("feet"));
        assert!(section.contains("convert"));
    }

    #[test]
    fn metric_names_kilometres_and_needs_no_conversion() {
        let section = units_context(UnitSystem::Metric);
        assert!(section.starts_with("\n\n## Units\n\n"));
        assert!(section.contains("kilometres"));
        assert!(!section.contains("miles"));
        assert!(!section.contains("convert"));
    }
}
