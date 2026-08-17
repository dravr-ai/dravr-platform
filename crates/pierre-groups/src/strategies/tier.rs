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

/// Strategy composition based on tenant pricing tier.
///
/// Each tier determines which sub-strategies are used and what limits apply.
pub trait GroupTierStrategy: Send + Sync {
    /// Maximum number of groups this tier allows (None = unlimited)
    fn max_groups(&self) -> Option<usize>;

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
/// allowance ([`GroupTierStrategy::max_groups`]).
///
/// REST and `/coach select` are a user asking for a new group, so they spend
/// it. A messaging auto-bind is not a request — the group materializes because
/// somebody added the bot to a chat — and refusing it would leave that chat
/// with no group context and only a log line to say why, so the per-group
/// member cap is the gate there instead. That matters more than the tier
/// numbers suggest: [`crate::GroupService`] is built once with a hardcoded
/// `professional` strategy, so `max_groups` is the same `Some(3)` for every
/// tenant regardless of plan, and an allowance that is not yet a real pricing
/// signal must not silently un-group a live chat.
pub(crate) enum OwnerGroupLimit {
    /// Count the owner's groups against `max_groups` and refuse beyond it.
    Enforced,
    /// Skip the count — the member cap governs this path.
    Exempt,
}

/// Starter tier: small groups, no group features.
///
/// The caps are deliberately generous relative to the feature set: a Starter
/// tenant can hold a handful of people in a chat group, but every group
/// *feature* below (roster, dashboard, peer sharing, digests) stays off. The
/// member cap is what [`crate::GroupService::create_group`] and
/// [`crate::GroupService::create_channel_group`] gate on — a cap of `0` there
/// rejects creation outright, which is why this tier carries a real number
/// rather than zero: adding the bot to a Telegram group is the primary way
/// groups come into existence, and every tenant is created on Starter.
pub struct StarterTierStrategy;

impl GroupTierStrategy for StarterTierStrategy {
    fn max_groups(&self) -> Option<usize> {
        Some(3)
    }

    fn max_members_per_group(&self) -> usize {
        5
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

/// Professional tier: 3 groups, 10 members each
pub struct ProfessionalTierStrategy;

impl GroupTierStrategy for ProfessionalTierStrategy {
    fn max_groups(&self) -> Option<usize> {
        Some(3)
    }

    fn max_members_per_group(&self) -> usize {
        10
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
pub struct EnterpriseTierStrategy;

impl GroupTierStrategy for EnterpriseTierStrategy {
    fn max_groups(&self) -> Option<usize> {
        None
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
