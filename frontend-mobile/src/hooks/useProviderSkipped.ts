// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: "Continue without connecting" (mobile), matching the web: advances at once and is saved on the server
// ABOUTME: The connect step's skipped row keeps Tes données out of the wizard at every later launch (carnet#1530)

import { useCallback } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { SESSION_ONLY_QUERY_META } from '../providers/sessionOnlyQuery';
import { useRecordOnboardingStep } from './useRecordOnboardingStep';

/**
 * Whether the user chose, this session, to continue past the provider gate
 * without connecting.
 *
 * Mirrors the web `skipProvider`. The tap is held in the shared query cache,
 * so the routing gate and the connect screen read one value at once, and is
 * written as the connect step's `skipped` row, which the onboarding context
 * reads (`isConnectProviderSkipped`) on every later launch and device: the
 * connect screen never comes back at sign-in, and Home's "Connect your data"
 * nudge takes over. The cache entry itself stays off disk; the server row is
 * the durable answer.
 */
export function useProviderSkipped(userId: string | undefined) {
  const queryClient = useQueryClient();
  const recordStep = useRecordOnboardingStep();
  const queryKey = ['provider-skipped', userId] as const;

  const { data: skipped } = useQuery({
    queryKey,
    // No AsyncStorage read: the server's step row is the durable answer.
    queryFn: () => false,
    staleTime: Infinity,
    // Kept off disk: the persister would otherwise restore it on the next launch.
    meta: SESSION_ONLY_QUERY_META,
  });

  const skip = useCallback(() => {
    queryClient.setQueryData(queryKey, true);
    recordStep('connect_provider', 'skipped');
    // eslint-disable-next-line react-hooks/exhaustive-deps -- queryKey derives from userId
  }, [userId, queryClient, recordStep]);

  return { skipped: skipped ?? false, skip };
}
