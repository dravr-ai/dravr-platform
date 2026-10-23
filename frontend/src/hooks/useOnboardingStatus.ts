// ABOUTME: The athlete's onboarding status from the server — every web surface that reads it shares this one query
// ABOUTME: The gate, Home's Pick your team sheet and the connect nudge all read provider and role state from it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { userApi } from '../services/api';

/**
 * `GET /api/me/onboarding-status`, under the one query key every reader
 * shares.
 *
 * Kept fresh (5 s) so an OAuth round-trip flips the onboarding gate as soon as
 * the provider connection lands; `enabled` lets a caller wait until the
 * account is one onboarding applies to.
 */
export function useOnboardingStatus(enabled = true) {
  return useQuery({
    queryKey: QUERY_KEYS.user.onboardingStatus(),
    queryFn: () => userApi.getOnboardingStatus(),
    enabled,
    staleTime: 5_000,
  });
}
