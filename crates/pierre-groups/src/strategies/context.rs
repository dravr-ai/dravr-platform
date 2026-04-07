// ABOUTME: Coach system prompt extension builders for group context
// ABOUTME: Produces prompt text blocks tailored to the requester's role (member vs admin)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write;

use pierre_core::models::groups::{
    CoachingGroup, GroupContext, MemberFlag, MemberSummaryCard, SummaryDetailLevel,
};

/// Strategy for building group context text injected into the coach's system prompt.
///
/// Different implementations produce different prompt styles depending on
/// who is asking (admin overview vs individual member focus).
pub trait GroupContextStrategy: Send + Sync {
    /// Build context for group-level interactions (admin viewing full roster)
    fn build_group_context(&self, group: &GroupContext, members: &[MemberSummaryCard]) -> String;

    /// Build context for a specific member's conversation
    fn build_member_context(
        &self,
        member: &MemberSummaryCard,
        group: &GroupContext,
        all_members: &[MemberSummaryCard],
    ) -> String;

    /// Whether to include named peer comparisons (only when `peer_data_sharing` is on)
    fn include_peer_comparison(&self, group: &CoachingGroup) -> bool;
}

/// Context focused on an individual member within the group.
/// Used when a member asks questions about their own training.
pub struct IndividualFocusContext;

impl GroupContextStrategy for IndividualFocusContext {
    fn build_group_context(&self, group: &GroupContext, members: &[MemberSummaryCard]) -> String {
        let mut text = String::with_capacity(512);
        let _ = writeln!(text, "\n--- Group Coaching Context ---");
        let _ = writeln!(
            text,
            "You are coaching a group of {} athletes called \"{}\".",
            group.member_count, group.group.name
        );
        let _ = writeln!(text, "{} members are currently active.", group.active_count);
        let _ = writeln!(text);

        for card in members {
            let _ = writeln!(text, "- {}", card.summary_text.trim());
        }

        let _ = writeln!(text, "--- End Group Context ---");
        text
    }

    fn build_member_context(
        &self,
        member: &MemberSummaryCard,
        group: &GroupContext,
        all_members: &[MemberSummaryCard],
    ) -> String {
        let mut text = String::with_capacity(512);
        let _ = writeln!(text, "\n--- Group Coaching Context ---");
        let _ = writeln!(
            text,
            "You are coaching {} as part of a group of {} athletes called \"{}\".",
            member.display_name, group.member_count, group.group.name
        );
        let _ = writeln!(text);
        let _ = writeln!(text, "{}'s current status:", member.display_name);
        let _ = writeln!(text, "{}", member.summary_text.trim());

        if group.active_count > 1 {
            let _ = writeln!(text);
            let _ = writeln!(
                text,
                "Group has {} active members. Tailor advice to {}'s individual level.",
                group.active_count, member.display_name
            );
            if group.group.peer_data_sharing {
                // Peer sharing is on — include visible peer summaries
                for peer in all_members.iter().filter(|m| m.user_id != member.user_id) {
                    let _ = writeln!(text, "- {}", peer.summary_text.trim());
                }
            } else {
                let _ = writeln!(
                    text,
                    "IMPORTANT: Peer data sharing is DISABLED. You have NO data about \
                     other group members. NEVER fabricate, estimate, or guess other \
                     members' stats. If asked to compare members or plan joint activities \
                     based on fitness data, explain that peer data sharing must be enabled \
                     in the group settings first."
                );
            }
        }

        let _ = writeln!(text, "--- End Group Context ---");
        text
    }

    fn include_peer_comparison(&self, group: &CoachingGroup) -> bool {
        group.peer_data_sharing
    }
}

/// Context providing a full group overview for admin/owner users.
/// Used when the group admin reviews the whole roster.
pub struct GroupOverviewContext;

impl GroupContextStrategy for GroupOverviewContext {
    fn build_group_context(&self, group: &GroupContext, members: &[MemberSummaryCard]) -> String {
        let mut text = String::with_capacity(1024);
        let _ = writeln!(text, "\n--- Group Coaching Context (Admin View) ---");
        let _ = writeln!(
            text,
            "You are the AI coach for \"{}\" ({} members, {} active).",
            group.group.name, group.member_count, group.active_count
        );
        let _ = writeln!(
            text,
            "The admin/owner is reviewing the group. Provide full details."
        );
        let _ = writeln!(text);
        let _ = writeln!(text, "Roster:");
        for card in members {
            let _ = writeln!(text, "- {}", card.summary_text.trim());
        }

        // Flag members needing attention
        let flagged: Vec<&MemberSummaryCard> =
            members.iter().filter(|m| !m.flags.is_empty()).collect();
        if !flagged.is_empty() {
            let _ = writeln!(text);
            let _ = writeln!(text, "Members needing attention:");
            for m in flagged {
                let flag_names: Vec<&str> = m
                    .flags
                    .iter()
                    .map(|f| match f {
                        MemberFlag::Overreaching => "overreaching",
                        MemberFlag::FreshForm => "fresh form",
                        MemberFlag::Inactive => "inactive",
                        MemberFlag::PersonalRecord => "new PR",
                        MemberFlag::InjuryRisk => "injury risk",
                        MemberFlag::VolumeDrop => "volume drop",
                    })
                    .collect();
                let _ = writeln!(text, "  - {}: {}", m.display_name, flag_names.join(", "));
            }
        }

        let _ = writeln!(text, "--- End Group Context ---");
        text
    }

    fn build_member_context(
        &self,
        _member: &MemberSummaryCard,
        group: &GroupContext,
        all_members: &[MemberSummaryCard],
    ) -> String {
        // Admin viewing a specific member still gets full context
        self.build_group_context(group, all_members)
    }

    fn include_peer_comparison(&self, _group: &CoachingGroup) -> bool {
        // Admin always sees all data
        true
    }
}

/// Selects the appropriate context strategy based on the requester's role
#[must_use]
pub fn select_context_strategy(is_admin: bool) -> Box<dyn GroupContextStrategy> {
    if is_admin {
        Box::new(GroupOverviewContext)
    } else {
        Box::new(IndividualFocusContext)
    }
}

/// Build the detail level label for logging
#[must_use]
pub fn detail_level_label(level: SummaryDetailLevel) -> &'static str {
    match level {
        SummaryDetailLevel::Roster => "roster",
        SummaryDetailLevel::Weekly => "weekly",
        SummaryDetailLevel::Detailed => "detailed",
    }
}
