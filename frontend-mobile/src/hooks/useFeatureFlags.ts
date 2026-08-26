// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook that fetches the calling user's feature flag map (mobile)
// ABOUTME: Types, keys and the failure answer all come from the shared @pierre/api-client domain

import { useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';
import {
  FEATURE_KEYS,
  mergeFeatureFlags,
  type FeatureFlagMap,
  type KnownFeatureFlag,
} from '@pierre/api-client';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { featureFlagsApi } from '../services/api';

export { FEATURE_KEYS };
export type { FeatureFlagMap, KnownFeatureFlag };

/** Shape returned by `useFeatureFlags`. The `flags` map always covers every
 * known key, populated either from the server (when loaded) or from the
 * shared compile defaults (otherwise). */
export interface UseFeatureFlagsResult {
  flags: FeatureFlagMap;
  known: KnownFeatureFlag[];
  isLoading: boolean;
  isError: boolean;
}

/** Fetch `/api/me/features` once after auth and cache it for the session.
 * Components read `flags[FEATURE_KEYS.apiTokens]` directly. */
export function useFeatureFlags(): UseFeatureFlagsResult {
  const { data, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.featureFlags.self(),
    queryFn: () => featureFlagsApi.getMyFeatures(),
    // Flags rarely change in a single session; keep the cache warm.
    staleTime: 5 * 60_000,
  });

  return useMemo(
    () => ({
      flags: mergeFeatureFlags(data?.flags),
      known: data?.known ?? [],
      isLoading,
      isError,
    }),
    [data, isLoading, isError],
  );
}
