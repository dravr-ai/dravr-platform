// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The agent behind an open thread, read off the cached coaches list rather than a second request
// ABOUTME: One query key for every agent reader on the web, so the info panel and the header agree

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { Agent } from '@pierre/shared-types';
import { coachesApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

/** What the agent info panel draws. */
export interface AgentInfoState {
  /** The agent the conversation is bound to, or null while unknown. */
  coach: Agent | null;
  isLoading: boolean;
}

/**
 * The conversation's agent.
 *
 * Reads the same `QUERY_KEYS.coaches.list()` entry the chat header already
 * holds — installed agents plus the system catalogue — so opening the info
 * panel costs no request on a thread whose header has already resolved.
 */
export function useCoachInfo(agentId: string | null | undefined): AgentInfoState {
  const { data, isLoading } = useQuery<{ agents: Agent[] }>({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    staleTime: 5 * 60 * 1000,
    enabled: !!agentId,
  });

  const coach = useMemo(
    () => (agentId ? (data?.agents.find((c) => c.id === agentId) ?? null) : null),
    [data, agentId],
  );

  return { coach, isLoading: isLoading && !!agentId };
}
