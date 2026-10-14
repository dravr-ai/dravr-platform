// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Dashboard API methods - usage analytics, admin recent activity
// ABOUTME: Provides data for the main dashboard views and analytics charts

import { axios } from './client';

export const dashboardApi = {
  async getUsageAnalytics(days: number = 30) {
    const response = await axios.get(`/api/dashboard/analytics?days=${days}`);
    return response.data;
  },

  /** Fetch recent LLM calls and conversations for the admin activity feed */
  async getRecentActivity(): Promise<RecentActivityResponse> {
    const response = await axios.get('/api/admin/analytics/recent-activity');
    return response.data;
  },
};

/** Response shape from the recent-activity admin endpoint */
export interface RecentLlmCall {
  id: string;
  provider: string;
  model: string;
  total_tokens: number;
  cost_usd: number;
  call_type: string;
  execution_time_ms: number | null;
  created_at: string;
}

export interface RecentConversation {
  id: string;
  title: string;
  updated_at: string;
  user_email: string;
}

export interface RecentActivitySummary {
  active_conversations: number;
  llm_calls_today: number;
  total_tokens_today: number;
  estimated_cost_today: number;
}

export interface RecentActivityResponse {
  recent_llm_calls: RecentLlmCall[];
  recent_conversations: RecentConversation[];
  summary: RecentActivitySummary;
}
