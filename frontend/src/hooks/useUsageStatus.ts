// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for fetching usage quota status from the backend
// ABOUTME: Used by chat components to display usage warnings and block at limits

import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useMemo } from 'react';
import { usageApi, type UsageStatusResponse, type LimitCheckResult } from '../services/api/usage';
import { QUERY_KEYS } from '../constants/queryKeys';

/** Warning level for usage status display */
export type WarningLevel = 'none' | 'warning' | 'burst' | 'blocked';

/** Computed usage warning state for UI components */
export interface UsageWarningState {
  /** The most severe warning level across all counters */
  level: WarningLevel;
  /** Whether message sending should be disabled */
  sendDisabled: boolean;
  /** Human-readable message to show in the banner */
  message: string;
  /** ISO 8601 timestamp for when limits reset */
  resetsAt: string;
  /** The most restrictive counter that triggered the warning */
  triggerCounter: LimitCheckResult | null;
}

/** Compute the warning level from a LimitCheckResult */
function getCounterLevel(counter: LimitCheckResult): WarningLevel {
  if (!counter.allowed) return 'blocked';
  if (counter.burst_zone) return 'burst';
  if (counter.warning) return 'warning';
  return 'none';
}

/** Priority order: blocked > burst > warning > none */
const LEVEL_PRIORITY: Record<WarningLevel, number> = {
  none: 0,
  warning: 1,
  burst: 2,
  blocked: 3,
};

/** Format reset time in user's local timezone */
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

/** Compute the warning state from the full usage status response */
export function computeWarningState(data: UsageStatusResponse | undefined): UsageWarningState {
  if (!data) {
    return { level: 'none', sendDisabled: false, message: '', resetsAt: '', triggerCounter: null };
  }

  // Check all daily counters (most relevant for chat usage)
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
    return { level: 'none', sendDisabled: false, message: '', resetsAt: '', triggerCounter: null };
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
    triggerCounter: worstCounter,
  };
}

/** Hook to fetch and compute usage warning state for chat components */
export function useUsageStatus() {
  const queryClient = useQueryClient();

  const { data, isLoading, error } = useQuery<UsageStatusResponse>({
    queryKey: QUERY_KEYS.usage.status(),
    queryFn: () => usageApi.getStatus(),
    refetchInterval: 60_000,
    staleTime: 30_000,
  });

  const warningState = useMemo(() => computeWarningState(data), [data]);

  /** Invalidate the usage query (call after sending a message) */
  const invalidate = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.usage.status() });
  }, [queryClient]);

  return {
    data,
    isLoading,
    error,
    ...warningState,
    invalidate,
  };
}
