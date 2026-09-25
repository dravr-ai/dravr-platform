// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Admin usage analytics — LLM consumption and per-tool usage across a tenant
// ABOUTME: Web-only; the calling user's own quota counters come from @pierre/api-client's usage domain

import { axios } from './client';

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

/** LLM consumption response from GET /admin/usage/llm-consumption */
export interface LlmConsumptionResponse {
  summary: {
    total_tokens: number;
    total_calls: number;
    estimated_cost_usd: number;
  };
  breakdown: ConsumptionBreakdownItem[];
  daily_series: DailyConsumptionPoint[];
}

/** Per-tool aggregate from GET /admin/tool-usage */
export interface ToolUsageBreakdownItem {
  tool_name: string;
  invocation_count: number;
  turn_count: number;
  avg_latency_ms: number | null;
}

/** Tool-usage analytics response from GET /admin/tool-usage */
export interface ToolUsageResponse {
  summary: {
    total_invocations: number;
    unique_tools: number;
    turns_with_tools: number;
  };
  breakdown: ToolUsageBreakdownItem[];
  days: number;
}

export const adminUsageApi = {
  /** Fetch LLM consumption analytics (admin-scoped, optional tenant) */
  async getAdminLlmConsumption(days: number = 30, groupBy?: string, tenantId?: string): Promise<LlmConsumptionResponse> {
    const params = new URLSearchParams();
    params.append('days', String(days));
    if (groupBy) params.append('group_by', groupBy);
    if (tenantId) params.append('tenant_id', tenantId);
    const response = await axios.get(`/admin/usage/llm-consumption?${params}`);
    return response.data;
  },

  /** Fetch per-tool usage analytics (admin-scoped, optional tenant) */
  async getAdminToolUsage(days: number = 30, tenantId?: string): Promise<ToolUsageResponse> {
    const params = new URLSearchParams();
    params.append('days', String(days));
    if (tenantId) params.append('tenant_id', tenantId);
    const response = await axios.get(`/admin/tool-usage?${params}`);
    return response.data;
  },
};
