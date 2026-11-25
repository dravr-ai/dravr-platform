// ABOUTME: The copy of the weekly digest posted into a group's chat — over the sharers, never a members-only member
// ABOUTME: The chat app decides who reads the chat, so a member drawn from Garmin data is neither named nor counted (carnet#1170)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The chat copy of the digest: who it shows, and its parameters. The parent
//! module posts it and re-exports both.

use pierre_core::models::groups::{CoachingGroup, GroupMember, MemberFitnessSnapshot};
use pierre_groups::GroupService;
use pierre_services::notification_text::PARAM_SHARED_MEMBERS;
use pierre_tool_runtime::group_fitness::TracedSnapshot;
use serde_json::Value;

use super::covered_digest_params;
#[cfg(doc)]
use super::digest_params;

/// The snapshots the chat copy of the digest may show (carnet#1170).
///
/// Every member's but those drawn from data kept to Dravr's members. A member
/// left out is neither named nor counted in the chat, as a member who does
/// not share is not.
#[must_use]
pub fn room_shown(traced: &[TracedSnapshot]) -> Vec<MemberFitnessSnapshot> {
    traced
        .iter()
        .filter(|traced| !traced.members_only)
        .map(|traced| traced.snapshot.clone())
        .collect()
}

/// The parameters of the digest posted into a group's chat.
///
/// [`digest_params`] over the members who share their training
/// ([`GroupService::peer_sharing_user_ids`]) and nobody else — the summary,
/// the trend, the volume lines, the highlights and the concerns are all
/// computed over that set, so every line of the chat copy, the all-clear
/// included, is true of it. It also carries how many members that is and how
/// many the group holds, which the renderer turns into its scope line.
#[must_use]
pub fn room_digest_params(
    service: &GroupService,
    group: &CoachingGroup,
    members: &[GroupMember],
    snapshots: &[MemberFitnessSnapshot],
) -> Value {
    let sharing = GroupService::peer_sharing_user_ids(group, members);
    covered_digest_params(
        service,
        group,
        members,
        snapshots,
        &sharing,
        PARAM_SHARED_MEMBERS,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use dravr_tronc::dates::Timestamp;
    use pierre_core::models::groups::OvertrainingRiskLevel;
    use uuid::Uuid;

    use super::*;

    fn traced(name: &str, members_only: bool) -> TracedSnapshot {
        TracedSnapshot {
            snapshot: MemberFitnessSnapshot {
                user_id: Uuid::new_v4(),
                display_name: name.to_owned(),
                ctl: None,
                atl: None,
                tsb: None,
                form_ctl: None,
                weekly_volume_km: Some(42.0),
                previous_week_volume_km: None,
                weekly_activity_count: Some(3),
                weekly_duration_seconds: Some(10_800),
                primary_sport: None,
                vdot: None,
                overtraining_risk: OvertrainingRiskLevel::Low,
                days_since_last_activity: Some(1),
                last_activity_per_provider: HashMap::new(),
                recent_activities: Vec::new(),
                needs_reauth_providers: Vec::new(),
                served_stale: false,
                timezone: None,
                deep_fatigue_mornings: Vec::new(),
                garmin_contribution: None,
                computed_at: Timestamp::now(),
            },
            members_only,
        }
    }

    // carnet#1170: the chat copy never shows a member whose figures were drawn
    // from Garmin; everyone else's figures still reach it.
    #[test]
    fn the_chat_copy_leaves_out_a_member_drawn_from_members_only_data() {
        let shown = room_shown(&[traced("Garmin rider", true), traced("Strava runner", false)]);
        let names: Vec<&str> = shown.iter().map(|s| s.display_name.as_str()).collect();
        assert_eq!(names, vec!["Strava runner"]);
    }
}
