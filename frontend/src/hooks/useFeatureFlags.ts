// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook that fetches the calling user's feature flag map
// ABOUTME: Gates surfaces like the API Tokens settings tab and Billing header

import { useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';
import { featureFlagsApi, type FeatureFlagMap, type KnownFeatureFlag } from '../services/api/featureFlags';
import { QUERY_KEYS } from '../constants/queryKeys';

/** Default values used by the hook before the network call resolves and when
 * the request errors. Mirrors the compile-time defaults in
 * `pierre_core::feature_flags::FeatureKey::default_enabled`. */
const FALLBACK_FLAGS: FeatureFlagMap = {
  api_tokens: false,
  billing_header: false,
};

/** Shape returned by `useFeatureFlags`. The `flags` map always covers every
 * known key, populated either from the server (when loaded) or from
 * `FALLBACK_FLAGS` (otherwise). */
export interface UseFeatureFlagsResult {
  flags: FeatureFlagMap;
  known: KnownFeatureFlag[];
  isLoading: boolean;
  isError: boolean;
}

/** Fetch `/api/me/features` once after auth and cache the result for the
 * session. Components consume `flags[FEATURE_KEYS.api_tokens]` directly. */
export function useFeatureFlags(): UseFeatureFlagsResult {
  const { data, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.featureFlags.self(),
    queryFn: () => featureFlagsApi.getMyFeatures(),
    // Flags rarely change in a single session; keep the cache warm.
    staleTime: 5 * 60_000,
  });

  return useMemo(() => {
    const flags = data?.flags ?? FALLBACK_FLAGS;
    // Ensure every known key has a value: server response wins, fall back to
    // FALLBACK_FLAGS for keys the server hasn't returned yet.
    const merged: FeatureFlagMap = { ...FALLBACK_FLAGS, ...flags };
    return {
      flags: merged,
      known: data?.known ?? [],
      isLoading,
      isError,
    };
  }, [data, isLoading, isError]);
}

/** Stable storage-string constants for components that read flags. Matches
 * `FeatureKey::as_str` on the backend. */
export const FEATURE_KEYS = {
  apiTokens: 'api_tokens',
  billingHeader: 'billing_header',
} as const;
