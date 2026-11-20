// ABOUTME: A group tier strategy whose plan includes no group coaching, which no shipping tier is
// ABOUTME: Lets the server suites drive every creation path's refusal of such a plan (feature `groups`)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_groups::strategies::tier::{
    GroupAllowance, GroupFeatureFlags, GroupTierStrategy, StarterTierStrategy,
};
use pierre_groups::strategies::{
    GroupAggregationStrategy, GroupContextStrategy, GroupSummarizationStrategy,
};

/// A plan without group coaching: its allowance is
/// [`GroupAllowance::Disabled`], and everything else is Starter's.
///
/// `tier_strategy_for` never resolves to it, since no shipping plan leaves
/// group coaching out; the suites pass it where a resolved tier is taken.
#[derive(Debug)]
pub struct NoGroupCoachingTier;

impl GroupTierStrategy for NoGroupCoachingTier {
    fn group_allowance(&self) -> GroupAllowance {
        GroupAllowance::Disabled
    }

    fn max_members_per_group(&self) -> usize {
        StarterTierStrategy.max_members_per_group()
    }

    fn allowed_features(&self) -> GroupFeatureFlags {
        StarterTierStrategy.allowed_features()
    }

    fn summarization_strategy(&self) -> Arc<dyn GroupSummarizationStrategy> {
        StarterTierStrategy.summarization_strategy()
    }

    fn aggregation_strategy(&self) -> Arc<dyn GroupAggregationStrategy> {
        StarterTierStrategy.aggregation_strategy()
    }

    fn admin_context_strategy(&self) -> Arc<dyn GroupContextStrategy> {
        StarterTierStrategy.admin_context_strategy()
    }

    fn member_context_strategy(&self) -> Arc<dyn GroupContextStrategy> {
        StarterTierStrategy.member_context_strategy()
    }
}
