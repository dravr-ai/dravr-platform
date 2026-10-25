// ABOUTME: Pricing tier-based strategy composition and feature gating
// ABOUTME: Composes summarization, aggregation, and context strategies per tenant plan
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use super::aggregation::{CachedAggregation, GroupAggregationStrategy, LiveAggregation};
use super::context::{GroupContextStrategy, GroupOverviewContext, IndividualFocusContext};
use super::summarization::{
    AdaptiveSummarizer, GroupSummarizationStrategy, RosterCardSummarizer, WeeklyDigestSummarizer,
};

/// Feature flags for group coaching capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupFeatureFlags {
    /// Basic group roster and membership
    pub basic_roster: bool,
    /// Stats dashboard (table view)
    pub stats_dashboard: bool,
    /// Stats charts (enterprise only)
    pub stats_charts: bool,
    /// Peer data sharing opt-in
    pub peer_sharing: bool,
    /// Weekly digest notifications
    pub weekly_digest: bool,
    /// Group management via messaging
    pub messaging_management: bool,
    /// Multiple invite links with expiry
    pub advanced_invites: bool,
}

/// Athletes the Professional plan includes. Each athlete above this many is
/// billed per athlete on the coach's subscription rather than refused.
const PROFESSIONAL_INCLUDED_ATHLETES: usize = 15;

/// Athletes a Starter coach may hold — one group each — before creation is
/// refused.
const STARTER_MAX_ATHLETES: usize = 10;

/// How a tier treats the number of groups one owner holds.
///
/// A coach brings each athlete into a group of their own, so a group is an
/// athlete: the allowance is counted in groups and charged in athletes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupAllowance {
    /// Creation is refused once the owner holds this many groups.
    Capped(usize),
    /// This many groups are included in the plan; each one above it is billed
    /// as one extra athlete on the owner's subscription. Without a
    /// subscription to bill, the included count is a cap.
    BilledAbove(usize),
    /// No limit and no per-athlete charge.
    Unlimited,
}

impl GroupAllowance {
    /// Athletes billed when the owner holds `active_groups` groups: the
    /// groups above the included count under [`Self::BilledAbove`], and zero
    /// for a tier with no per-athlete charge.
    ///
    /// `active_groups` is the count the cap reads, so the charge and the cap
    /// can never disagree about how many athletes a coach has.
    #[must_use]
    pub(crate) fn billed_athletes(self, active_groups: i64) -> u64 {
        match self {
            Self::BilledAbove(included) => {
                let held = u64::try_from(active_groups).unwrap_or(0);
                held.saturating_sub(u64::try_from(included).unwrap_or(u64::MAX))
            }
            Self::Capped(_) | Self::Unlimited => 0,
        }
    }
}

/// Strategy composition based on tenant pricing tier.
///
/// Each tier determines which sub-strategies are used and what limits apply.
pub trait GroupTierStrategy: Send + Sync {
    /// How many groups one owner may hold under this tier, and whether the
    /// ones above the included count are refused or billed.
    fn group_allowance(&self) -> GroupAllowance;

    /// Maximum members per group
    fn max_members_per_group(&self) -> usize;

    /// Which features are enabled for this tier
    fn allowed_features(&self) -> GroupFeatureFlags;

    /// Summarization strategy for this tier
    fn summarization_strategy(&self) -> Arc<dyn GroupSummarizationStrategy>;

    /// Aggregation strategy for this tier
    fn aggregation_strategy(&self) -> Arc<dyn GroupAggregationStrategy>;

    /// Context strategy for admin users
    fn admin_context_strategy(&self) -> Arc<dyn GroupContextStrategy>;

    /// Context strategy for member users
    fn member_context_strategy(&self) -> Arc<dyn GroupContextStrategy>;
}

/// Whether a group-creation path spends one of the owner's tier group
/// allowance ([`GroupTierStrategy::group_allowance`]).
///
/// A REST group creation is a user asking for a new group, so it spends
/// it. A messaging auto-bind is not a request — the group materializes because
/// somebody added the bot to a chat — and refusing it would leave that chat
/// with no group context and only a log line to say why, so the per-group
/// member cap is the gate there instead: a live chat must never be silently
/// un-grouped because its owner reached the plan's allowance.
pub(crate) enum OwnerGroupLimit {
    /// Count the owner's groups against this plan allowance and refuse a
    /// creation it does not cover.
    Enforced(GroupAllowance),
    /// Skip the count — the member cap governs this path.
    Exempt,
}

/// Starter tier: one group per athlete, no group features.
///
/// A Starter group is a coach, one athlete and the agent: the owner (the
/// coach) and the athlete are its two members, and the agent is the group's
/// `agent_id`, never a member row. A coach brings each athlete into their own
/// group, so the group allowance is sized for the validation cohort's 5–10
/// athletes per coach. Every group *feature* below (roster, dashboard, peer
/// sharing, digests) stays off. The member cap is what
/// [`crate::GroupService::create_group`] and
/// [`crate::GroupService::create_channel_group`] gate on — a cap of `0` there
/// rejects creation outright, which is why this tier carries a real number
/// rather than zero: every tenant is created on Starter.
#[derive(Debug)]
pub struct StarterTierStrategy;

impl GroupTierStrategy for StarterTierStrategy {
    fn group_allowance(&self) -> GroupAllowance {
        GroupAllowance::Capped(STARTER_MAX_ATHLETES)
    }

    fn max_members_per_group(&self) -> usize {
        2
    }

    fn allowed_features(&self) -> GroupFeatureFlags {
        GroupFeatureFlags {
            basic_roster: false,
            stats_dashboard: false,
            stats_charts: false,
            peer_sharing: false,
            weekly_digest: false,
            messaging_management: false,
            advanced_invites: false,
        }
    }

    fn summarization_strategy(&self) -> Arc<dyn GroupSummarizationStrategy> {
        Arc::new(RosterCardSummarizer)
    }

    fn aggregation_strategy(&self) -> Arc<dyn GroupAggregationStrategy> {
        Arc::new(LiveAggregation)
    }

    fn admin_context_strategy(&self) -> Arc<dyn GroupContextStrategy> {
        Arc::new(GroupOverviewContext)
    }

    fn member_context_strategy(&self) -> Arc<dyn GroupContextStrategy> {
        Arc::new(IndividualFocusContext)
    }
}

/// Professional tier: one group per athlete, like Starter. The paid coach
/// seat includes 15 athletes; each one above is billed per athlete on the
/// coach's subscription instead of being refused.
#[derive(Debug)]
pub struct ProfessionalTierStrategy;

impl GroupTierStrategy for ProfessionalTierStrategy {
    fn group_allowance(&self) -> GroupAllowance {
        GroupAllowance::BilledAbove(PROFESSIONAL_INCLUDED_ATHLETES)
    }

    fn max_members_per_group(&self) -> usize {
        2
    }

    fn allowed_features(&self) -> GroupFeatureFlags {
        GroupFeatureFlags {
            basic_roster: true,
            stats_dashboard: true,
            stats_charts: false,
            peer_sharing: false,
            weekly_digest: true,
            messaging_management: false,
            advanced_invites: false,
        }
    }

    fn summarization_strategy(&self) -> Arc<dyn GroupSummarizationStrategy> {
        Arc::new(WeeklyDigestSummarizer)
    }

    fn aggregation_strategy(&self) -> Arc<dyn GroupAggregationStrategy> {
        Arc::new(CachedAggregation::new())
    }

    fn admin_context_strategy(&self) -> Arc<dyn GroupContextStrategy> {
        Arc::new(GroupOverviewContext)
    }

    fn member_context_strategy(&self) -> Arc<dyn GroupContextStrategy> {
        Arc::new(IndividualFocusContext)
    }
}

/// Enterprise tier: unlimited groups, 50 members, all features
#[derive(Debug)]
pub struct EnterpriseTierStrategy;

impl GroupTierStrategy for EnterpriseTierStrategy {
    fn group_allowance(&self) -> GroupAllowance {
        GroupAllowance::Unlimited
    }

    fn max_members_per_group(&self) -> usize {
        50
    }

    fn allowed_features(&self) -> GroupFeatureFlags {
        GroupFeatureFlags {
            basic_roster: true,
            stats_dashboard: true,
            stats_charts: true,
            peer_sharing: true,
            weekly_digest: true,
            messaging_management: true,
            advanced_invites: true,
        }
    }

    fn summarization_strategy(&self) -> Arc<dyn GroupSummarizationStrategy> {
        Arc::new(AdaptiveSummarizer::new())
    }

    fn aggregation_strategy(&self) -> Arc<dyn GroupAggregationStrategy> {
        Arc::new(CachedAggregation::new())
    }

    fn admin_context_strategy(&self) -> Arc<dyn GroupContextStrategy> {
        Arc::new(GroupOverviewContext)
    }

    fn member_context_strategy(&self) -> Arc<dyn GroupContextStrategy> {
        Arc::new(IndividualFocusContext)
    }
}

/// Resolve the tier strategy from a tenant plan string
#[must_use]
pub fn tier_strategy_for(plan: &str) -> Arc<dyn GroupTierStrategy> {
    match plan {
        "professional" | "pro" => Arc::new(ProfessionalTierStrategy),
        "enterprise" => Arc::new(EnterpriseTierStrategy),
        _ => Arc::new(StarterTierStrategy),
    }
}

/// Whether a tenant plan's tier enables the `weekly_digest` feature.
///
/// It is the ceiling over every group's digest mode: the digest scheduler
/// sweeps only the tenants it answers `true` for, the permissions endpoint
/// reports it to the clients, and `/group digest` says when a group's plan
/// sends nothing.
#[must_use]
pub fn tier_enables_digest(plan: &str) -> bool {
    tier_strategy_for(plan).allowed_features().weekly_digest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_plan_name_resolves_to_its_tier_allowance() {
        let starter = StarterTierStrategy.group_allowance();
        let professional = ProfessionalTierStrategy.group_allowance();
        assert_eq!(tier_strategy_for("starter").group_allowance(), starter);
        assert_eq!(
            tier_strategy_for("professional").group_allowance(),
            professional
        );
        assert_eq!(tier_strategy_for("pro").group_allowance(), professional);
        assert_eq!(
            tier_strategy_for("enterprise").group_allowance(),
            EnterpriseTierStrategy.group_allowance()
        );
        assert_eq!(
            tier_strategy_for("an-unknown-plan").group_allowance(),
            starter,
            "an unknown plan resolves to Starter"
        );
        assert!(
            matches!(starter, GroupAllowance::Capped(_)),
            "Starter refuses past its allowance"
        );
        assert!(
            matches!(professional, GroupAllowance::BilledAbove(_)),
            "Professional bills past its allowance"
        );
        assert_eq!(
            EnterpriseTierStrategy.group_allowance(),
            GroupAllowance::Unlimited
        );
    }

    #[test]
    fn professional_bills_only_the_athletes_above_fifteen() {
        let allowance = ProfessionalTierStrategy.group_allowance();
        assert_eq!(allowance.billed_athletes(0), 0);
        assert_eq!(allowance.billed_athletes(14), 0);
        assert_eq!(
            allowance.billed_athletes(15),
            0,
            "the 15th athlete is included"
        );
        assert_eq!(
            allowance.billed_athletes(16),
            1,
            "the 16th is the first billed"
        );
        assert_eq!(allowance.billed_athletes(40), 25);
    }

    #[test]
    fn tiers_without_a_per_athlete_charge_bill_nothing() {
        assert_eq!(GroupAllowance::Capped(10).billed_athletes(10), 0);
        assert_eq!(GroupAllowance::Capped(10).billed_athletes(25), 0);
        assert_eq!(GroupAllowance::Unlimited.billed_athletes(500), 0);
    }

    #[test]
    fn a_negative_count_bills_nothing() {
        assert_eq!(GroupAllowance::BilledAbove(15).billed_athletes(-3), 0);
    }
}
