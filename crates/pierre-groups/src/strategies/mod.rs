// ABOUTME: Strategy trait definitions for group coaching behavior
// ABOUTME: Summarization, aggregation, context building, and tier-based feature gating
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// LLM context window management via tiered member summarization
pub mod summarization;

/// Group statistics computation with caching strategies
pub mod aggregation;

/// Coach system prompt extension builders for group context
pub mod context;

/// Pricing tier-based strategy composition and feature gating
pub mod tier;

pub use aggregation::GroupAggregationStrategy;
pub use context::GroupContextStrategy;
pub use summarization::GroupSummarizationStrategy;
pub use tier::GroupTierStrategy;
