// ABOUTME: Times a coach's setup: emits group.first_athlete_joined when a group gains its first non-owner member
// ABOUTME: Measures the validation cohort's 1 h goal from the coach's signup to their first athlete (carnet#739)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach setup timing.
//!
//! The validation cohort aims for a coach to go from a new account to a first
//! athlete in their group within an hour. `onboarding.coach_group_created`
//! marks the group step; this module marks the end of the setup: the first
//! person other than the owner to join the group, timed from the group's
//! creation and from the coach's signup.
//!
//! "First" is the group's earliest non-owner membership row ever, compared by
//! id with the row just inserted, so an athlete who leaves and a second one who
//! joins does not fire it again, and two athletes joining at once fire it
//! exactly once between them.

use chrono::{DateTime, Utc};
use pierre_core::models::groups::{GroupMember, GroupSetupTimeline};
use pierre_database::repositories::CoachingGroupRepository;
use tracing::{info, warn};
use uuid::Uuid;

/// Whole seconds from `start` to `now`, never negative.
fn seconds_between(start: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    now.signed_duration_since(start).num_seconds().max(0)
}

/// Whether `member` is the group's first non-owner membership.
fn is_first_athlete(timeline: &GroupSetupTimeline, member: &GroupMember) -> bool {
    timeline.first_member_id == Some(member.id)
}

/// Emit `group.first_athlete_joined` when `member` is the first person other
/// than the owner to join their group.
///
/// The event is the coach's, so `user_id` is the group owner. A failed lookup
/// is logged and swallowed: the join itself already happened, and timing it is
/// not a reason to fail it.
pub async fn record_member_joined(
    repo: &dyn CoachingGroupRepository,
    member: &GroupMember,
    owner_id: Uuid,
) {
    let group_id = member.group_id.to_string();
    let timeline = match repo.setup_timeline(&group_id, &member.tenant_id).await {
        Ok(Some(timeline)) => timeline,
        Ok(None) => return,
        Err(e) => {
            warn!(group_id = %member.group_id, error = %e, "group setup timeline lookup failed");
            return;
        }
    };
    if !is_first_athlete(&timeline, member) {
        return;
    }
    let now = Utc::now();
    info!(
        target: "notify",
        event = "group.first_athlete_joined",
        user_id = %owner_id,
        tenant_id = %member.tenant_id,
        group_id = %member.group_id,
        seconds_since_group_created = seconds_between(timeline.group_created_at, now),
        seconds_since_coach_signup = seconds_between(timeline.owner_created_at, now),
        "first athlete joined a coach's group"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use pierre_core::models::groups::GroupRole;

    fn member(id: Uuid) -> GroupMember {
        let now = Utc::now();
        GroupMember {
            id,
            group_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            tenant_id: "t".to_owned(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: true,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        }
    }

    fn timeline(first: Option<Uuid>) -> GroupSetupTimeline {
        let now = Utc::now();
        GroupSetupTimeline {
            group_created_at: now,
            owner_created_at: now,
            first_member_id: first,
        }
    }

    #[test]
    fn only_the_earliest_membership_row_is_the_first_athlete() {
        let joined = member(Uuid::new_v4());
        assert!(is_first_athlete(&timeline(Some(joined.id)), &joined));
        assert!(!is_first_athlete(&timeline(Some(Uuid::new_v4())), &joined));
        assert!(!is_first_athlete(&timeline(None), &joined));
    }

    #[test]
    fn elapsed_seconds_are_whole_and_never_negative() {
        let now = Utc::now();
        assert_eq!(seconds_between(now - Duration::minutes(42), now), 2520);
        assert_eq!(seconds_between(now + Duration::seconds(5), now), 0);
    }
}
