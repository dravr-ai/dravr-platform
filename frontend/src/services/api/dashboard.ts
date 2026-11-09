// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Dashboard API methods - usage analytics, admin recent activity
// ABOUTME: Provides data for the main dashboard views and analytics charts

import { axios } from './client';
import type { AdminScope } from '../../hooks/useAdminScope';

export const dashboardApi = {
  async getUsageAnalytics(days: number = 30, tenantId?: string) {
    const params = new URLSearchParams({ days: String(days) });
    if (tenantId) params.append('tenant_id', tenantId);
    const response = await axios.get(`/api/dashboard/analytics?${params}`);
    return response.data;
  },

  /** Fetch recent LLM calls and conversations for the admin activity feed */
  async getRecentActivity(tenantId?: string): Promise<RecentActivityResponse> {
    const response = await axios.get('/api/admin/analytics/recent-activity', {
      params: tenantId ? { tenant_id: tenantId } : undefined,
    });
    return response.data;
  },
};

/** Response shape from the recent-activity admin endpoint */
export interface RecentLlmCall {
  id: string;
  provider: string;
  model: string;
  total_tokens: number;
  /** Prompt tokens served from the provider's context cache */
  cached_tokens: number;
  /** Prompt tokens this call wrote into the provider's context cache */
  cached_write_tokens: number;
  /** The part of `cached_write_tokens` written to a 1-hour cache entry */
  cached_write_1h_tokens: number;
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
  cost_today_usd: number;
}

export interface RecentActivityResponse {
  recent_llm_calls: RecentLlmCall[];
  recent_conversations: RecentConversation[];
  /** How many rows each list above is capped at */
  recent_limit: number;
  /** The tenants the server read */
  scope: AdminScope;
  summary: RecentActivitySummary;
}
