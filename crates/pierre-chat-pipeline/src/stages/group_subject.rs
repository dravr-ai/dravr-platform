// ABOUTME: Whether a group turn's sender is that group's coach rather than an athlete in it
// ABOUTME: A coach's own training is never the group's data, so their turn runs in coach mode
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Who a group turn is about.
//!
//! Every athlete-data stage keys on the sender, which is right in a 1:1 thread
//! and in a group the sender trains in. It is wrong for the group's coach: a
//! coach who finished onboarding owns a group whose athlete invite is still
//! unredeemed, is its only member, and an `@agent` mention there answered from
//! the coach's own CTL, rides and goals as though they were the group's
//! (carnet#741). This stage names that seat once per turn; prompt assembly and
//! the activity prefetch read it to leave the coach's own data out.
//!
//! The seat is a role in THIS group, never a property of the account. A
//! coach-only account invited into someone else's group as an athlete is an
//! athlete there, and its own data reaches its turns as usual.

use pierre_core::models::groups::GroupRole;
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::ConversationRecord;
use pierre_database::RepositoryRegistry;
use pierre_services::intake::athlete_steps_waived;
use tracing::warn;
use uuid::Uuid;

/// The sender holds the coach's seat of the conversation's group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoachSeat {
    /// Members other than the coach, i.e. athletes who have joined. Zero while
    /// every invite is still unredeemed.
    pub athletes: usize,
}

impl CoachSeat {
    /// The prompt block that tells the agent who it is talking to.
    pub fn directive(self) -> String {
        let roster = if self.athletes == 0 {
            "No athlete has joined this group yet: the invite is still pending, so there is \
             no athlete data to read. Say so plainly, and help the coach with what needs none \
             — how the group works, what you will do once athletes join, how they want to \
             approach the season."
                .to_owned()
        } else {
            format!(
                "Athletes in this group: {}. Answer about them from the group context above, \
                 and from data fetched for an athlete the coach names.",
                self.athletes
            )
        };
        format!(
            "\n\n## Who you are talking to\n\n\
             The person writing is this group's coach, not an athlete in it. Their own \
             training, goals and profile are not the group's data: never present their \
             numbers as an athlete's, and never coach them on their own training here. If \
             they ask about their own training, tell them to ask in their own conversation \
             with Dravr, or in a group where they train as an athlete. To read an athlete's \
             data, call get_group_member_activities with that athlete's name. {roster}"
        )
    }
}

/// Whether `user_id` holds the coach's seat of `conv`'s group.
///
/// The seat is the group's recorded human coach, or its owner when the owner is
/// a coach: one holding `manages_roster`, or one who said at onboarding that
/// they coach and do not train. A plain athlete who created a group with
/// friends owns it too, and stays its subject.
///
/// Every lookup failure degrades to `None` — the sender stays the subject,
/// which is what every turn did before this stage existed.
pub async fn coach_seat(
    repos: &RepositoryRegistry,
    conv: &ConversationRecord,
    tenant_id: TenantId,
    user_id: &str,
) -> Option<CoachSeat> {
    let group_id = conv.group_id.as_deref()?;
    let sender = parse_uuid(user_id).ok()?;
    let group = match repos.groups.get_group(group_id, tenant_id).await {
        Ok(group) => group?,
        Err(e) => {
            warn!(group_id, error = %e, "group lookup failed; sender stays the turn's subject");
            return None;
        }
    };
    let members = match repos.groups.list_members(group_id).await {
        Ok(members) => members,
        Err(e) => {
            warn!(group_id, error = %e, "member lookup failed; sender stays the turn's subject");
            return None;
        }
    };
    let seat = CoachSeat {
        athletes: members
            .iter()
            .filter(|m| m.user_id != sender && Some(m.user_id) != group.coach_user_id)
            .count(),
    };

    if group.coach_user_id == Some(sender) {
        return Some(seat);
    }
    let owns = members
        .iter()
        .any(|m| m.user_id == sender && m.role == GroupRole::Owner);
    (owns && is_coach(repos, sender, user_id).await).then_some(seat)
}

/// Whether the account coaches: the operator grant, or the coach-only answer
/// recorded at onboarding.
async fn is_coach(repos: &RepositoryRegistry, sender: Uuid, user_id: &str) -> bool {
    match repos.users.get_global(sender).await {
        Ok(Some(user)) if user.manages_roster => return true,
        Ok(_) => {}
        Err(e) => warn!(error = %e, "user lookup failed; roster grant treated as absent"),
    }
    match repos.user_onboarding.get_onboarding_steps(user_id).await {
        Ok(steps) => athlete_steps_waived(&steps),
        Err(e) => {
            warn!(error = %e, "onboarding step lookup failed; owner treated as an athlete");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_group_says_the_invite_is_pending() {
        let directive = CoachSeat { athletes: 0 }.directive();
        assert!(directive.contains("this group's coach"));
        assert!(directive.contains("invite is still pending"));
    }

    #[test]
    fn a_joined_group_counts_its_athletes() {
        let directive = CoachSeat { athletes: 3 }.directive();
        assert!(directive.contains("Athletes in this group: 3."));
        assert!(!directive.contains("pending"));
    }
}
