// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Feature flags domain API - the calling user's effective flag map
// ABOUTME: Owns the one request-failure answer both web and mobile use: compile defaults, never "on"

import type { AxiosInstance } from 'axios';
import type { FeatureFlagMap, KnownFeatureFlag, MeFeaturesResponse } from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type { FeatureFlagMap, KnownFeatureFlag, MeFeaturesResponse };

/** Stable storage strings for the flags clients read by name. Matches
 * `FeatureKey::as_str` on the backend. */
export const FEATURE_KEYS = {
  apiTokens: 'api_tokens',
  billingHeader: 'billing_header',
} as const;

/**
 * The values every client shows when the server has not answered — while the
 * request is in flight and when it fails.
 *
 * Both surfaces resolve a failed `GET /api/me/features` to these compile
 * defaults rather than to "flag on", so a network failure can never reveal a
 * gated surface, and web and mobile cannot disagree about what a user sees.
 * Mirrors `pierre_core::feature_flags::FeatureKey::default_enabled`.
 */
export const FALLBACK_FEATURE_FLAGS: FeatureFlagMap = {
  [FEATURE_KEYS.apiTokens]: false,
  [FEATURE_KEYS.billingHeader]: false,
};

/**
 * Resolve the map a client renders from: the server's values layered over the
 * compile defaults, so a key the server omits still has a defined value.
 */
export function mergeFeatureFlags(serverFlags: FeatureFlagMap | undefined): FeatureFlagMap {
  return { ...FALLBACK_FEATURE_FLAGS, ...(serverFlags ?? {}) };
}

/**
 * Creates the feature flags API methods bound to an axios instance.
 */
export function createFeatureFlagsApi(axios: AxiosInstance) {
  return {
    /**
     * Fetch the calling user's effective flag map and the known-flag registry.
     */
    async getMyFeatures(): Promise<MeFeaturesResponse> {
      const response = await axios.get<MeFeaturesResponse>(ENDPOINTS.FEATURE_FLAGS.ME);
      return response.data;
    },
  };
}

export type FeatureFlagsApi = ReturnType<typeof createFeatureFlagsApi>;
