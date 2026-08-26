// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for fetching usage quota status from the backend (mobile)
// ABOUTME: Used by chat components to display usage warnings and block at limits

import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ReplyNotice } from '@pierre/shared-types';
import { quotaNoticeBanner } from '@pierre/chat-utils';
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

/**
 * Compute the warning state from a turn's own `notice` reply block.
 *
 * Shares its wording with web through `quotaNoticeBanner`, so the same cap
 * reads the same way on both clients. A notice never blocks sending: the turn
 * it rode already succeeded.
 */
export function warningStateFromNotice(notice: ReplyNotice): UsageWarningState {
  const banner = quotaNoticeBanner(notice);
  return {
    level: banner.level,
    sendDisabled: false,
    message: banner.message,
    resetsAt: banner.resetsAt,
  };
}

/**
 * Hook to fetch and compute usage warning state for mobile chat.
 *
 * Two things measure the same counters: the status endpoint this polls, and
 * the turn itself, whose pre-turn quota check emits a `notice` reply block.
 * They feed ONE banner state — {@link applyNotice} seeds it from the turn's own
 * measurement, and the next poll to land clears it, because by then the
 * endpoint has counted the same turn.
 */
export function useUsageStatus() {
  const queryClient = useQueryClient();
  const [turnNotice, setTurnNotice] = useState<ReplyNotice | null>(null);

  const { data, isLoading } = useQuery<UsageStatusResponse>({
    queryKey: QUERY_KEYS.usage.status(),
    queryFn: async () => {
      const response = await apiClient.get('/api/usage/status');
      return response.data;
    },
    refetchInterval: 60_000,
    staleTime: 30_000,
  });

  // A fresh poll has counted whatever the notice reported, so it supersedes it.
  useEffect(() => {
    if (data) setTurnNotice(null);
  }, [data]);

  const warningState = useMemo(
    () => (turnNotice ? warningStateFromNotice(turnNotice) : computeWarningState(data)),
    [turnNotice, data],
  );

  const invalidate = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.usage.status() });
  }, [queryClient]);

  /** Show the counters a turn's own `notice` block reported. */
  const applyNotice = useCallback((notice: ReplyNotice) => {
    setTurnNotice(notice);
  }, []);

  return {
    data,
    isLoading,
    ...warningState,
    invalidate,
    applyNotice,
  };
}
