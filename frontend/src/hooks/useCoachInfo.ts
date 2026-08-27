// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach behind an open thread, read off the cached coaches list rather than a second request
// ABOUTME: One query key for every coach reader on the web, so the info panel and the header agree

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { Coach } from '@pierre/shared-types';
import { coachesApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

/** What the coach info panel draws. */
export interface CoachInfoState {
  /** The coach the conversation is bound to, or null while unknown. */
  coach: Coach | null;
  isLoading: boolean;
}

/**
 * The conversation's coach.
 *
 * Reads the same `QUERY_KEYS.coaches.list()` entry the chat header already
 * holds — installed coaches plus the system catalogue — so opening the info
 * panel costs no request on a thread whose header has already resolved.
 */
export function useCoachInfo(coachId: string | null | undefined): CoachInfoState {
  const { data, isLoading } = useQuery<{ coaches: Coach[] }>({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    staleTime: 5 * 60 * 1000,
    enabled: !!coachId,
  });

  const coach = useMemo(
    () => (coachId ? (data?.coaches.find((c) => c.id === coachId) ?? null) : null),
    [data, coachId],
  );

  return { coach, isLoading: isLoading && !!coachId };
}
