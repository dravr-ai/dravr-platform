// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Dashboard API methods - overview, analytics, rate limits, request monitoring
// ABOUTME: Provides data for the main dashboard views and analytics charts

import { axios } from './client';

export const dashboardApi = {
  async getDashboardOverview() {
    const response = await axios.get('/api/dashboard/overview');
    return response.data;
  },

  async getUsageAnalytics(days: number = 30) {
    const response = await axios.get(`/api/dashboard/analytics?days=${days}`);
    return response.data;
  },

  async getRateLimitOverview() {
    const response = await axios.get('/api/dashboard/rate-limits');
    return response.data;
  },

  async getRequestLogs(apiKeyId?: string, filter?: {
    timeRange: string;
    status: string;
    tool: string;
  }) {
    const params = new URLSearchParams();
    if (apiKeyId) params.append('api_key_id', apiKeyId);
    if (filter?.timeRange) params.append('time_range', filter.timeRange);
    if (filter?.status && filter.status !== 'all') params.append('status', filter.status);
    if (filter?.tool && filter.tool !== 'all') params.append('tool', filter.tool);

    const response = await axios.get(`/api/dashboard/request-logs?${params}`);
    return response.data;
  },

  async getRequestStats(apiKeyId?: string, timeRange: string = '1h') {
    const params = new URLSearchParams();
    if (apiKeyId) params.append('api_key_id', apiKeyId);
    params.append('time_range', timeRange);

    const response = await axios.get(`/api/dashboard/request-stats?${params}`);
    return response.data;
  },

  async getToolUsageBreakdown(apiKeyId?: string, timeRange: string = '7d') {
    const params = new URLSearchParams();
    if (apiKeyId) params.append('api_key_id', apiKeyId);
    params.append('time_range', timeRange);

    const response = await axios.get(`/api/dashboard/tool-usage?${params}`);
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
