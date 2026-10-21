// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The team proposal query — `GET /api/agents/proposal`, the team picker's input
// ABOUTME: One cache slot shared by onboarding's Your team step and Home's Pick your team sheet

import { useQuery } from '@tanstack/react-query';
import { coachesApi } from '../../services/api';

/** The proposal is a one-time snapshot of the athlete's data; nothing invalidates it. */
const TEAM_PROPOSAL_QUERY_KEY = ['coaches', 'proposal'] as const;

/** `GET /api/agents/proposal`, the picker's input — shared so Home's sheet reads the same cache slot. */
export function useTeamProposal() {
  return useQuery({
    queryKey: TEAM_PROPOSAL_QUERY_KEY,
    queryFn: () => coachesApi.getProposal(),
    staleTime: Infinity,
    retry: 1,
  });
}
