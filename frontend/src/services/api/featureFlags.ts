// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: User-facing feature flag API methods
// ABOUTME: Wraps GET /api/me/features for the useFeatureFlags hook

import { axios } from './client';

/** Effective per-flag map returned by `GET /api/me/features`. */
export type FeatureFlagMap = Record<string, boolean>;

/** Registry entry returned alongside the effective map. */
export interface KnownFeatureFlag {
  key: string;
  description: string;
  default_enabled: boolean;
}

/** Response from `GET /api/me/features`. */
export interface MeFeaturesResponse {
  flags: FeatureFlagMap;
  known: KnownFeatureFlag[];
}

export const featureFlagsApi = {
  async getMyFeatures(): Promise<MeFeaturesResponse> {
    const response = await axios.get('/api/me/features');
    return response.data;
  },
};
