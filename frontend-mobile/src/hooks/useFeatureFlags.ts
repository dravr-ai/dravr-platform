// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook that fetches the calling user's feature flag map (mobile)
// ABOUTME: Mirrors the web hook so component code can read flags identically across surfaces

import { useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';
import { apiClient } from '../services/api';

export type FeatureFlagMap = Record<string, boolean>;

export interface KnownFeatureFlag {
  key: string;
  description: string;
  default_enabled: boolean;
}

export interface MeFeaturesResponse {
  flags: FeatureFlagMap;
  known: KnownFeatureFlag[];
}

const FALLBACK_FLAGS: FeatureFlagMap = {
  api_tokens: false,
  billing_header: false,
};

export const FEATURE_KEYS = {
  apiTokens: 'api_tokens',
  billingHeader: 'billing_header',
} as const;

export interface UseFeatureFlagsResult {
  flags: FeatureFlagMap;
  known: KnownFeatureFlag[];
  isLoading: boolean;
  isError: boolean;
}

export function useFeatureFlags(): UseFeatureFlagsResult {
  const { data, isLoading, isError } = useQuery({
    queryKey: ['feature-flags', 'me'],
    queryFn: async () => {
      const response = await apiClient.get<MeFeaturesResponse>('/api/me/features');
      return response.data;
    },
    staleTime: 5 * 60_000,
  });

  return useMemo(() => {
    const flags = data?.flags ?? FALLBACK_FLAGS;
    const merged: FeatureFlagMap = { ...FALLBACK_FLAGS, ...flags };
    return {
      flags: merged,
      known: data?.known ?? [],
      isLoading,
      isError,
    };
  }, [data, isLoading, isError]);
}
