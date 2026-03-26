// ABOUTME: Data types for admin analytics dashboard endpoints
// ABOUTME: Row types for database queries and response types for API endpoints
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════
// DATABASE ROW TYPES — deserialized from sqlx query results
// ═══════════════════════════════════════════════════════════════

/// Daily conversation count for time-series charts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyConversationRow {
    /// Date string (YYYY-MM-DD)
    pub date: String,
    /// Number of conversations created on this date
    pub count: i64,
}

/// Top coach by usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopCoachRow {
    /// Coach title
    pub title: String,
    /// Coach category (training, nutrition, recovery, etc.)
    pub category: String,
    /// Number of active assignments for this coach
    pub assignment_count: i64,
}

/// Coach category distribution count
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryCountRow {
    /// Category name
    pub category: String,
    /// Number of coaches in this category
    pub count: i64,
}

/// Daily active users for time-series charts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyActiveUsersRow {
    /// Date string (YYYY-MM-DD)
    pub date: String,
    /// Number of unique users active on this date
    pub count: i64,
}

/// User engagement tier breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementTiers {
    /// Users active in the last 24 hours
    pub daily_active: i64,
    /// Users active in the last 7 days
    pub weekly_active: i64,
    /// Users active in the last 30 days
    pub monthly_active: i64,
    /// Users not active in the last 30 days
    pub dormant: i64,
}

/// Provider connection distribution count
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCountRow {
    /// Provider name (strava, garmin, synthetic, etc.)
    pub provider: String,
    /// Number of connections for this provider
    pub count: i64,
}

/// Coaching group statistics row
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupStatsRow {
    /// Total number of groups
    pub total_groups: i64,
    /// Number of active groups
    pub active_groups: i64,
    /// Total members across all groups
    pub total_members: i64,
}

/// Aggregated LLM usage row for cost calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCostAggregateRow {
    /// LLM provider name
    pub provider: String,
    /// Model identifier
    pub model: String,
    /// Total prompt tokens
    pub prompt_tokens: i64,
    /// Total completion tokens
    pub completion_tokens: i64,
}

// ═══════════════════════════════════════════════════════════════
// API RESPONSE TYPES — serialized to JSON for HTTP responses
// ═══════════════════════════════════════════════════════════════

/// Overview dashboard response with key platform metrics
#[derive(Debug, Serialize)]
pub struct AnalyticsOverviewResponse {
    /// Total active users in the tenant
    pub total_users: i64,
    /// Total conversations in the time window
    pub total_conversations: i64,
    /// Total messages in the time window
    pub total_messages: i64,
    /// Total LLM cost in USD for the time window
    pub total_llm_cost_usd: f64,
    /// Percentage of users with at least one coach assignment
    pub coach_adoption_pct: f64,
    /// Number of connected provider accounts
    pub connected_providers: i64,
    /// Number of active coaching groups
    pub active_groups: i64,
    /// Number of days in the analytics window
    pub days: u32,
}

/// Conversation analytics response with time series and breakdowns
#[derive(Debug, Serialize)]
pub struct ConversationAnalyticsResponse {
    /// Daily conversation counts for charting
    pub daily_series: Vec<DailyConversationRow>,
    /// Peak conversation hour (0-23) based on UTC
    pub peak_hour: Option<i64>,
    /// Average messages per conversation
    pub avg_messages_per_conversation: f64,
    /// Number of days in the analytics window
    pub days: u32,
}

/// Coach analytics response with rankings and distribution
#[derive(Debug, Serialize)]
pub struct CoachAnalyticsResponse {
    /// Top coaches by assignment count
    pub top_coaches: Vec<TopCoachRow>,
    /// Category distribution of coaches
    pub category_distribution: Vec<CategoryCountRow>,
    /// Number of system-created coaches
    pub system_coach_count: i64,
    /// Number of user-created coaches
    pub user_coach_count: i64,
    /// Total store installations
    pub store_installations: i64,
    /// Number of days in the analytics window
    pub days: u32,
}

/// Engagement analytics response with user activity patterns
#[derive(Debug, Serialize)]
pub struct EngagementAnalyticsResponse {
    /// Daily active user counts for charting
    pub daily_active_users: Vec<DailyActiveUsersRow>,
    /// User engagement tier breakdown
    pub engagement_tiers: EngagementTiers,
    /// Provider connection distribution
    pub provider_distribution: Vec<ProviderCountRow>,
    /// Coaching group statistics
    pub group_stats: GroupStatsRow,
    /// Number of days in the analytics window
    pub days: u32,
}
