// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for the calling user's feature flag map, bound to each client's flags API
// ABOUTME: Gates surfaces like the API Tokens settings row and the Billing header on both clients

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { mergeFeatureFlags } from '@pierre/api-client';
import type { FeatureFlagMap, FeatureFlagsApi, KnownFeatureFlag } from '@pierre/api-client';

/** Flags rarely change within a session, so the cache stays warm this long. */
const FLAGS_STALE_MS = 5 * 60_000;

/**
 * Shape returned by `useFeatureFlags`. The `flags` map always covers every
 * known key, from the server once it has answered and from the shared compile
 * defaults until then.
 */
export interface UseFeatureFlagsResult {
  flags: FeatureFlagMap;
  known: KnownFeatureFlag[];
  isLoading: boolean;
  isError: boolean;
}

/**
 * Build `useFeatureFlags` over one client's feature flags API: fetch
 * `/api/me/features` once after auth and cache it for the session. Components
 * read `flags[FEATURE_KEYS.apiTokens]` directly.
 */
export function createFeatureFlagsHook(featureFlagsApi: FeatureFlagsApi) {
  return function useFeatureFlags(): UseFeatureFlagsResult {
    const { data, isLoading, isError } = useQuery({
      queryKey: QUERY_KEYS.featureFlags.self(),
      queryFn: () => featureFlagsApi.getMyFeatures(),
      staleTime: FLAGS_STALE_MS,
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
  };
}
