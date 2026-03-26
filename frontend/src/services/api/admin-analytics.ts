// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Admin analytics API methods for coaching-centric dashboard metrics
// ABOUTME: Fetches overview, conversation, coach, and engagement analytics

import { axios } from './client';

// ==================== RESPONSE TYPES ====================

/** Overview analytics with active users, conversations, LLM spend, and coach adoption */
export interface AdminAnalyticsOverview {
  active_users_24h: number;
  active_users_7d: number;
  conversations_today: number;
  messages_today: number;
  llm_spend_today_usd: number;
  llm_spend_7d_usd: number;
  coach_adoption_pct: number;
  connected_providers: number;
  total_users: number;
  total_coaches: number;
}

/** A single day in the conversation time series */
export interface ConversationDayPoint {
  date: string;
  conversations: number;
  messages: number;
}

/** Conversation analytics over a time range */
export interface ConversationAnalyticsResponse {
  daily: ConversationDayPoint[];
  peak_hour: number;
  avg_messages_per_conversation: number;
  total_conversations: number;
  total_messages: number;
}

/** A coach entry in the leaderboard */
export interface CoachLeaderboardEntry {
  coach_id: string;
  title: string;
  category: string;
  use_count: number;
  is_system: boolean;
}

/** Category distribution item */
export interface CategoryDistribution {
  category: string;
  count: number;
}

/** Coach analytics response */
export interface CoachAnalyticsResponse {
  top_coaches: CoachLeaderboardEntry[];
  categories: CategoryDistribution[];
  system_coach_count: number;
  user_coach_count: number;
}

/** A single day in the engagement time series */
export interface EngagementDayPoint {
  date: string;
  dau: number;
}

/** Engagement tier distribution */
export interface EngagementTier {
  tier: string;
  count: number;
}

/** Provider distribution item */
export interface ProviderDistribution {
  provider: string;
  connected_count: number;
}

/** Group stats summary */
export interface GroupStats {
  total_groups: number;
  total_members: number;
}

/** Engagement analytics response */
export interface EngagementAnalyticsResponse {
  dau_series: EngagementDayPoint[];
  engagement_tiers: EngagementTier[];
  provider_distribution: ProviderDistribution[];
  group_stats: GroupStats;
}

// ==================== API METHODS ====================

export const adminAnalyticsApi = {
  /** Fetch overview analytics: active users, conversations, LLM spend, coach adoption */
  async getOverview(): Promise<AdminAnalyticsOverview> {
    const response = await axios.get('/api/admin/analytics/overview');
    return response.data;
  },

  /** Fetch conversation analytics for a given number of days */
  async getConversations(days: number = 7): Promise<ConversationAnalyticsResponse> {
    const response = await axios.get(`/api/admin/analytics/conversations?days=${days}`);
    return response.data;
  },

  /** Fetch coach analytics: leaderboard, categories, system vs user counts */
  async getCoaches(): Promise<CoachAnalyticsResponse> {
    const response = await axios.get('/api/admin/analytics/coaches');
    return response.data;
  },

  /** Fetch engagement analytics for a given number of days */
  async getEngagement(days: number = 7): Promise<EngagementAnalyticsResponse> {
    const response = await axios.get(`/api/admin/analytics/engagement?days=${days}`);
    return response.data;
  },
};
