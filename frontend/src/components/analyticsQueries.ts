// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The queries behind the admin Analytics panels, defined once for the panels and the tab that holds them
// ABOUTME: The tab starts all three together so the panels appear at once instead of pushing each other down

import { queryOptions } from '@tanstack/react-query';
import { dashboardApi, adminUsageApi } from '../services/api';
import type { AdminTimeRange } from '../hooks/useAdminTimeRange';
import { QUERY_KEYS } from '../constants/queryKeys';
import type { AnalyticsData } from '../types/chart';
import type { LlmConsumptionResponse, ToolUsageResponse } from '../services/api/usage';

/** Request volume, latency and top tools for the Usage Analytics panel. */
export function usageAnalyticsQuery(days: AdminTimeRange) {
  return queryOptions<AnalyticsData>({
    queryKey: QUERY_KEYS.dashboard.usageAnalytics(days),
    queryFn: () => dashboardApi.getUsageAnalytics(days),
  });
}

/** Token and cost consumption for the LLM Consumption panel. */
export function llmConsumptionQuery(days: AdminTimeRange) {
  return queryOptions<LlmConsumptionResponse>({
    queryKey: [...QUERY_KEYS.usage.llmConsumption(days), 'admin-panel'],
    queryFn: () => adminUsageApi.getAdminLlmConsumption(days),
  });
}

/** Per-tool run counts and latency for the Tool Usage panel. */
export function toolUsageQuery(days: AdminTimeRange) {
  return queryOptions<ToolUsageResponse>({
    queryKey: ['admin', 'tool-usage', days],
    queryFn: () => adminUsageApi.getAdminToolUsage(days),
    // Degrade gracefully — never propagate to the error boundary (which would
    // blank the whole Analytics tab if this endpoint is unavailable).
    throwOnError: false,
    retry: false,
  });
}
