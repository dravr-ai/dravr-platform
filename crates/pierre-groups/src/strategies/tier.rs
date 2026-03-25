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

/// Starter tier: groups disabled
pub struct StarterTierStrategy;

impl GroupTierStrategy for StarterTierStrategy {
    fn max_groups(&self) -> Option<usize> {
        Some(0)
    }

    fn max_members_per_group(&self) -> usize {
        0
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
