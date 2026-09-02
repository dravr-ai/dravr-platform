// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for fetching usage quota status from the backend
// ABOUTME: Used by chat components to display usage warnings and block at limits

import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ReplyNotice } from '@pierre/shared-types';
import { quotaNoticeBanner } from '@pierre/chat-utils';
import type { TranslatableText } from '@pierre/chat-utils';
import { usageApi, type UsageStatusResponse, type LimitCheckResult } from '../services/api/usage';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '../constants/queryKeys';

/** Warning level for usage status display */
export type WarningLevel = 'none' | 'warning' | 'burst' | 'blocked';

/** Computed usage warning state for UI components */
export interface UsageWarningState {
  /** The most severe warning level across all counters */
  level: WarningLevel;
  /** Whether message sending should be disabled */
  sendDisabled: boolean;
  /** The banner sentence, as a catalogue key plus its params. */
  text: TranslatableText | null;
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
function formatResetTime(isoString: string, fallback: string): string {
  try {
    const date = new Date(isoString);
    return new Intl.DateTimeFormat(undefined, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(date);
  } catch {
    return fallback;
  }
}

/**
 * Compute the warning state from the full usage status response.
 *
 * `resetFallback` is the caller's translated wording for an unparseable reset
 * instant. The sentences themselves come back as catalogue keys: this ran as
 * three hardcoded English templates, and the banner rendered them verbatim
 * under French chrome (carnet#207).
 */
export function computeWarningState(
  data: UsageStatusResponse | undefined,
  resetFallback: string,
): UsageWarningState {
  if (!data) {
    return { level: 'none', sendDisabled: false, text: null, resetsAt: '', triggerCounter: null };
  }

  // Check all daily counters (most relevant for chat usage). The label is a
  // catalogue key; the banner translates it and passes it back into the
  // sentence, so both halves speak one language.
  const counters: Array<{ counter: LimitCheckResult; label: string }> = [
    { counter: data.daily.messages, label: 'usage.dailyMessages' },
    { counter: data.daily.tokens, label: 'usage.dailyTokens' },
    { counter: data.weekly.messages, label: 'usage.weeklyMessages' },
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
    return { level: 'none', sendDisabled: false, text: null, resetsAt: '', triggerCounter: null };
  }

  const time = formatResetTime(worstCounter.resets_at, resetFallback);
  const percent = worstCounter.limit > 0
    ? Math.round((worstCounter.current / worstCounter.limit) * 100)
    : 0;
  const params = {
    label: worstLabel,
    current: worstCounter.current,
    limit: worstCounter.limit,
    time,
  };

  let text: TranslatableText | null;
  switch (worstLevel) {
    case 'blocked':
      text = { key: 'usage.blockedLimitReached', params };
      break;
    case 'burst':
      text = { key: 'usage.burstZone', params };
      break;
    case 'warning':
      text = { key: 'usage.percentUsed', params: { ...params, percent } };
      break;
    default:
      text = null;
  }

  return {
    level: worstLevel,
    sendDisabled: worstLevel === 'blocked',
    text,
    resetsAt: worstCounter.resets_at,
    triggerCounter: worstCounter,
  };
}

/**
 * Compute the warning state from a turn's own `notice` reply block.
 *
 * The wording and the counter reading come from `quotaNoticeBanner`, shared
 * with mobile — a warning the athlete sees on one client must not be a
 * different sentence on the other. A notice never blocks sending: the turn it
 * rode already succeeded.
 */
export function warningStateFromNotice(
  notice: ReplyNotice,
  resetFallback: string,
): UsageWarningState {
  const banner = quotaNoticeBanner(notice, resetFallback);
  return {
    level: banner.level,
    sendDisabled: false,
    text: banner.text,
    resetsAt: banner.resetsAt,
    triggerCounter: null,
  };
}

/**
 * Hook to fetch and compute usage warning state for chat components.
 *
 * Two things measure the same counters: the status endpoint this polls, and
 * the turn itself, whose pre-turn quota check emits a `notice` reply block.
 * They feed ONE banner state — {@link applyNotice} seeds it from the turn's own
 * measurement, and the next poll to land clears it, because by then the
 * endpoint has counted the same turn.
 */
export function useUsageStatus() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();
  const [turnNotice, setTurnNotice] = useState<ReplyNotice | null>(null);

  const { data, isLoading, error } = useQuery<UsageStatusResponse>({
    queryKey: QUERY_KEYS.usage.status(),
    queryFn: () => usageApi.getStatus(),
    refetchInterval: 60_000,
    staleTime: 30_000,
  });

  // A fresh poll has counted whatever the notice reported, so it supersedes it.
  useEffect(() => {
    if (data) setTurnNotice(null);
  }, [data]);

  const warningState = useMemo(
    () =>
      turnNotice
        ? warningStateFromNotice(turnNotice, t('settingsUi.midnightUtc'))
        : computeWarningState(data, t('settingsUi.midnightUtc')),
    [turnNotice, data, t],
  );

  /** Invalidate the usage query (call after sending a message) */
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
    error,
    ...warningState,
    invalidate,
    applyNotice,
  };
}
