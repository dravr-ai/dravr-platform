// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook that fetches the calling user's feature flag map
// ABOUTME: Gates surfaces like the API Tokens settings tab and Billing header

import { useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';
import { mergeFeatureFlags } from '@pierre/api-client';
import type { FeatureFlagMap, KnownFeatureFlag } from '@pierre/api-client';
import { featureFlagsApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

export { FEATURE_KEYS } from '@pierre/api-client';

/** Shape returned by `useFeatureFlags`. The `flags` map always covers every
 * known key, populated either from the server (when loaded) or from the
 * shared compile defaults (otherwise). */
export interface UseFeatureFlagsResult {
  flags: FeatureFlagMap;
  known: KnownFeatureFlag[];
  isLoading: boolean;
  isError: boolean;
}

/** Fetch `/api/me/features` once after auth and cache the result for the
 * session. Components consume `flags[FEATURE_KEYS.apiTokens]` directly. */
export function useFeatureFlags(): UseFeatureFlagsResult {
  const { data, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.featureFlags.self(),
    queryFn: () => featureFlagsApi.getMyFeatures(),
    // Flags rarely change in a single session; keep the cache warm.
    staleTime: 5 * 60_000,
  });

  return useMemo(
    () => ({
      // `mergeFeatureFlags` is the shared answer for a missing response:
      // server values layered over the compile defaults, so a flag the
      // server omits (or a failed request) resolves to off, not on.
      flags: mergeFeatureFlags(data?.flags),
      known: data?.known ?? [],
      isLoading,
      isError,
    }),
    [data, isLoading, isError],
  );
}
