// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Resolves the coach bound to a thread out of the athlete's own coach list, cached once
// ABOUTME: The same list the @handle palette reads, so Agent info can never name an agent a mention cannot reach

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { coachesApi } from '../services/api';
import type { Agent } from '../types';

/** What Agent info needs about the agent a thread is bound to. */
export interface UseAgentInfoResult {
  /** The coach row, or null while it loads or when it is not on the list. */
  coach: Agent | null;
  isLoading: boolean;
}

/**
 * The coach behind a thread, read from the athlete's own coach list.
 *
 * That list is already fetched for the `@` palette and shares its query key,
 * so opening Agent info costs no extra request in a session that has typed a
 * mention — and, more importantly, it resolves against exactly the coaches a
 * mention can reach, so the sheet cannot describe a coach `@handle` would
 * miss.
 */
export function useCoachInfo(agentId: string | null): UseAgentInfoResult {
  const { data, isLoading } = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    enabled: agentId !== null,
    staleTime: 5 * 60_000,
  });

  const coach = useMemo(
    () => (agentId === null ? null : (data?.agents ?? []).find((row) => row.id === agentId) ?? null),
    [data, agentId],
  );

  return { coach, isLoading: agentId !== null && isLoading };
}
