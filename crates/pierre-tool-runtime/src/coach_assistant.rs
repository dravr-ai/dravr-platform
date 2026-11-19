// ABOUTME: Which group members the coach tools show the AI assistant a human coach connects: those who allowed it
// ABOUTME: The member half of carnet#1511; the tool half (who is listed the coach tools) is tool_access

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The coach's assistant sees the members who allowed it (carnet#1511)
//!
//! The three coach tools (`get_roster_overview`, the coach side of
//! `get_group_member_activities`, `get_group_member_plan_status`) are listed
//! over MCP, A2A and an API key to a human coach and to an admin
//! ([`crate::tool_access`]). There they show a member only once that member
//! allowed it, per membership, from Dravr's own app
//! ([`CoachConsents::assistant_read`]); everyone else is a count. In Dravr's
//! own app nothing changes.
//!
//! The opt-in governs these tools only. A read of a member by name through the
//! `athlete` argument stays under the coach-sharing consent (JF, Q1), so the
//! words below never say a member left out cannot be read.
//!
//! "Served to the coach's assistant" is read off the transport the call
//! declared ([`MatchAudience::current`]), never off whether a filter applies:
//! member fetches run under `ai_scope::unfiltered`, where that reads false. A
//! call that declared no transport is read as served outside, which fails
//! closed.
//!
//! The check runs before any read of the member is recorded on their read
//! log, so a member left out is never logged as read.
//!
//! [`CoachConsents::assistant_read`]: pierre_core::models::groups::CoachConsents::assistant_read

use pierre_core::models::groups::GroupMember;

use crate::read_subject::MatchAudience;

/// Whether the call running now is served to an assistant outside Dravr's
/// own app: an MCP client, an A2A agent, an API key, or an undeclared caller.
#[must_use]
pub fn serving_coach_assistant() -> bool {
    MatchAudience::current() == MatchAudience::OutsideAssistant
}

/// Whether a coach tool running now may include the member of `membership`:
/// always in Dravr's own app; for the coach's assistant, only while they
/// share with the coach there and allowed the assistant there.
#[must_use]
pub fn includes(membership: &GroupMember) -> bool {
    !serving_coach_assistant()
        || (membership.coach_sharing_consent && membership.coach_consents.assistant_read)
}

/// What the coach's assistant reads beside a roster that left members out:
/// the count, and that a member who still shares is read by name by the
/// other tools (the counsel packet's count wording, 2026-11-15 §5.2).
#[must_use]
pub fn left_out_note(left_out: usize) -> String {
    if left_out == 1 {
        "1 member has not allowed AI tools to see them in the group overview and is not listed \
         here. If that member still shares their training with their human coach, the other \
         tools can read them by name."
            .to_owned()
    } else {
        format!(
            "{left_out} members have not allowed AI tools to see them in the group overview and \
             are not listed here. The other tools can read by name those who still share their \
             training with their human coach."
        )
    }
}

/// What the coach's assistant reads when a member it asked about is not
/// included: the same words whether the member did not allow it or no one
/// matched, so it never learns which.
#[must_use]
pub fn not_included_message(query: &str) -> String {
    format!(
        "No member matching '{query}' is shown to your assistant in a group you coach: a member \
         appears in these roster tools only once they allow it in their own Dravr app, under \
         Group info → What your human coach can do."
    )
}

/// What an admin who leads no active group reads from a coach tool
/// (carnet#1511, Q24): there is no roster of theirs, and impersonating a
/// coach, from Dravr's admin console (support sessions stay on the website,
/// EFVP sign-off 2026-11-18), is how an admin sees a coach's roster as the
/// coach does.
pub const ADMIN_NO_GROUP_NOTE: &str = "You lead no active group in Dravr, so there is no roster \
     to show. To see a coach's roster as they see it, impersonate that coach from Dravr's admin \
     console.";
