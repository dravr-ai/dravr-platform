// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The feature-flags read behind FeatureFlagsPanel, defined once for the panel and the page that holds it
// ABOUTME: Platform Settings waits on it so the panel cannot land above the other cards after they have drawn

import { queryOptions } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

/** Whose flags a panel shows: the tenant's defaults, or one user's overrides. */
export type FeatureFlagScope =
  | { kind: 'tenant'; id: string }
  | { kind: 'user'; id: string };

export function featureFlagsQuery(scope: FeatureFlagScope) {
  return queryOptions({
    queryKey:
      scope.kind === 'tenant'
        ? QUERY_KEYS.featureFlags.tenantDefaults(scope.id)
        : QUERY_KEYS.featureFlags.userOverrides(scope.id),
    queryFn: () =>
      scope.kind === 'tenant'
        ? adminApi.getTenantFeatureFlags(scope.id)
        : adminApi.getUserFeatureOverrides(scope.id),
    retry: 1,
  });
}
