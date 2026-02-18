// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Usage status API methods for quota checking and LLM consumption analytics
// ABOUTME: Provides data for usage warning banners and admin consumption dashboards

import { axios } from './client';

/** Single counter limit check result from the backend */
export interface LimitCheckResult {
  /** Whether the request is allowed (under hard limit) */
  allowed: boolean;
  /** Current counter value */
  current: number;
  /** Configured soft limit */
  limit: number;
  /** Whether the user is approaching the limit (at or above warning threshold) */
  warning: boolean;
  /** Whether the user is in the burst zone (between limit and hard limit) */
  burst_zone: boolean;
  /** ISO 8601 timestamp when the counter resets */
  resets_at: string;
}

/** Usage status response from GET /api/usage/status */
export interface UsageStatusResponse {
  daily: {
    messages: LimitCheckResult;
    tokens: LimitCheckResult;
    tool_calls: LimitCheckResult;
  };
  weekly: {
    messages: LimitCheckResult;
    tokens: LimitCheckResult;
    tool_calls: LimitCheckResult;
  };
  resources: {
    conversations: number;
    max_conversations: number;
    coaches: number;
    max_coaches: number;
  };
}

/** LLM consumption breakdown item */
export interface ConsumptionBreakdownItem {
  provider: string;
  model: string;
  call_type: string;
  total_tokens: number;
  calls: number;
  cost_usd: number;
}

/** LLM consumption daily point */
export interface DailyConsumptionPoint {
  date: string;
  tokens: number;
  calls: number;
  cost_usd: number;
}

/** LLM consumption response from GET /api/usage/llm-consumption */
export interface LlmConsumptionResponse {
  summary: {
    total_tokens: number;
    total_calls: number;
    estimated_cost_usd: number;
  };
  breakdown: ConsumptionBreakdownItem[];
  daily_series: DailyConsumptionPoint[];
}

export const usageApi = {
  /** Fetch current usage status (quotas, counters, warnings) */
  async getStatus(): Promise<UsageStatusResponse> {
    const response = await axios.get('/api/usage/status');
    return response.data;
  },

  /** Fetch LLM consumption analytics (user-scoped) */
  async getLlmConsumption(days: number = 30, groupBy?: string): Promise<LlmConsumptionResponse> {
    const params = new URLSearchParams();
    params.append('days', String(days));
    if (groupBy) params.append('group_by', groupBy);
    const response = await axios.get(`/api/usage/llm-consumption?${params}`);
    return response.data;
  },

  /** Fetch LLM consumption analytics (admin-scoped, optional tenant) */
  async getAdminLlmConsumption(days: number = 30, groupBy?: string, tenantId?: string): Promise<LlmConsumptionResponse> {
    const params = new URLSearchParams();
    params.append('days', String(days));
    if (groupBy) params.append('group_by', groupBy);
    if (tenantId) params.append('tenant_id', tenantId);
    const response = await axios.get(`/admin/usage/llm-consumption?${params}`);
    return response.data;
  },
};
