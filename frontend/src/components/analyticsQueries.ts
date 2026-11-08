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

/** Request volume and latency for the Usage Analytics panel. */
export function usageAnalyticsQuery(days: AdminTimeRange, tenantId?: string) {
  return queryOptions<AnalyticsData>({
    queryKey: [...QUERY_KEYS.dashboard.usageAnalytics(days), 'scoped', tenantId],
    queryFn: () => dashboardApi.getUsageAnalytics(days, tenantId),
  });
}

/** Token and cost consumption for the LLM Consumption panel. */
export function llmConsumptionQuery(days: AdminTimeRange, tenantId?: string) {
  return queryOptions<LlmConsumptionResponse>({
    queryKey: [...QUERY_KEYS.usage.llmConsumption(days), 'admin-panel', tenantId],
    queryFn: () => adminUsageApi.getAdminLlmConsumption(days, undefined, tenantId),
  });
}

/** Per-tool run counts and latency for the Tool Usage panel. */
export function toolUsageQuery(days: AdminTimeRange, tenantId?: string) {
  return queryOptions<ToolUsageResponse>({
    queryKey: ['admin', 'tool-usage', days, tenantId],
    queryFn: () => adminUsageApi.getAdminToolUsage(days, tenantId),
    // Degrade gracefully — never propagate to the error boundary (which would
    // blank the whole Analytics tab if this endpoint is unavailable).
    throwOnError: false,
    retry: false,
  });
}
