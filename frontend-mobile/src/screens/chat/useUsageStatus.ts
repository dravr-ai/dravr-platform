// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for fetching usage quota status from the backend (mobile)
// ABOUTME: Used by chat components to display usage warnings and block at limits

import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useMemo } from 'react';
import { apiClient } from '../../services/api';
import { QUERY_KEYS } from '@pierre/shared-constants';

/** Single counter limit check result from the backend */
export interface LimitCheckResult {
  allowed: boolean;
  current: number;
  limit: number;
  warning: boolean;
  burst_zone: boolean;
  resets_at: string;
}

/** Usage status response */
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

/** Warning level for display */
export type WarningLevel = 'none' | 'warning' | 'burst' | 'blocked';

/** Computed warning state */
export interface UsageWarningState {
  level: WarningLevel;
  sendDisabled: boolean;
  message: string;
  resetsAt: string;
}

const LEVEL_PRIORITY: Record<WarningLevel, number> = {
  none: 0,
  warning: 1,
  burst: 2,
  blocked: 3,
};

function getCounterLevel(counter: LimitCheckResult): WarningLevel {
  if (!counter.allowed) return 'blocked';
  if (counter.burst_zone) return 'burst';
  if (counter.warning) return 'warning';
  return 'none';
}

function formatResetTime(isoString: string): string {
  try {
    const date = new Date(isoString);
    return new Intl.DateTimeFormat(undefined, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(date);
  } catch {
    return 'midnight UTC';
  }
}

export function computeWarningState(data: UsageStatusResponse | undefined): UsageWarningState {
  if (!data) {
    return { level: 'none', sendDisabled: false, message: '', resetsAt: '' };
  }

  const counters: Array<{ counter: LimitCheckResult; label: string }> = [
    { counter: data.daily.messages, label: 'daily messages' },
    { counter: data.daily.tokens, label: 'daily tokens' },
    { counter: data.weekly.messages, label: 'weekly messages' },
  ];

  let worstLevel: WarningLevel = 'none';
  let worstCounter: LimitCheckResult | null = null;
  let worstLabel = '';

  for (const { counter, label } of counters) {
    const level = getCounterLevel(counter);
    if (LEVEL_PRIORITY[level] > LEVEL_PRIORITY[worstLevel]) {
      worstLevel = level;
      worstCounter = counter;
      worstLabel = label;
    }
  }

  if (!worstCounter || worstLevel === 'none') {
    return { level: 'none', sendDisabled: false, message: '', resetsAt: '' };
  }

  const resetTime = formatResetTime(worstCounter.resets_at);
  const pct = worstCounter.limit > 0
    ? Math.round((worstCounter.current / worstCounter.limit) * 100)
    : 0;

  let message: string;
  switch (worstLevel) {
    case 'blocked':
      message = `${worstLabel.charAt(0).toUpperCase() + worstLabel.slice(1)} limit reached. Limits reset at ${resetTime}.`;
      break;
    case 'burst':
      message = `You're in the burst zone for ${worstLabel} (${worstCounter.current}/${worstCounter.limit}). Limits reset at ${resetTime}.`;
      break;
    case 'warning':
      message = `You've used ${pct}% of your ${worstLabel} (${worstCounter.current}/${worstCounter.limit}). Limits reset at ${resetTime}.`;
      break;
    default:
      message = '';
  }

  return {
    level: worstLevel,
    sendDisabled: worstLevel === 'blocked',
    message,
    resetsAt: worstCounter.resets_at,
  };
}

/** Hook to fetch and compute usage warning state for mobile chat */
export function useUsageStatus() {
  const queryClient = useQueryClient();

  const { data, isLoading } = useQuery<UsageStatusResponse>({
    queryKey: QUERY_KEYS.usage.status(),
    queryFn: async () => {
      const response = await apiClient.get('/api/usage/status');
      return response.data;
    },
    refetchInterval: 60_000,
    staleTime: 30_000,
  });

  const warningState = useMemo(() => computeWarningState(data), [data]);

  const invalidate = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.usage.status() });
  }, [queryClient]);

  return {
    data,
    isLoading,
    ...warningState,
    invalidate,
  };
}
