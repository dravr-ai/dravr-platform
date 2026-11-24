// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for fetching usage quota status from the backend
// ABOUTME: Wires the query and the translator to the banner decision in @pierre/chat-utils

import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ReplyNotice, UsageStatusResponse } from '@pierre/shared-types';
import { computeWarningState, warningStateFromNotice } from '@pierre/chat-utils';
import { usageApi } from '../services/api';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '@pierre/shared-constants';

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
  const { t, language } = useTranslation();
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
        ? warningStateFromNotice(turnNotice, t('settingsUi.midnightUtc'), language)
        : computeWarningState(data, t('settingsUi.midnightUtc'), language),
    [turnNotice, data, t, language],
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
